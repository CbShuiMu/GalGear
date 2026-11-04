//! 用户授权、环回回调和刷新令牌轮换。网络工作必须由调用方放在后台线程。
use super::{Credentials, request_json};
use crate::providers;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use galgear_i18n::Message;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// 刷新令牌是一次性凭据；读、刷新、持久化必须串行，与保存和清除使用同一事务。
static STORE: Mutex<()> = Mutex::new(());

const BANGUMI_TOKEN: &str = "https://galgear.cbshuimu.net/oauth/bangumi/token";
const BANGUMI_SESSION: &str = "https://galgear.cbshuimu.net/oauth/bangumi/session";

/// 保存令牌交换方式，以便旧版自建应用凭据继续通过原应用刷新。
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exchange {
    #[default]
    Direct,
    BangumiBroker,
}

struct Application<'a> {
    client_id: &'a str,
    authorize: &'a str,
    token: &'a str,
    redirect: &'a str,
    session: Option<&'a str>,
}

fn application(source: &str) -> Result<Application<'static>, Message> {
    let (authorize, token) = endpoints(source)?;
    Ok(match source {
        "nextmoe" => Application {
            client_id: "1588bff36b26df9d9e56d60f16fb0330",
            authorize,
            token,
            redirect: "https://galgear.cbshuimu.net/oauth/nextmoe/callback",
            session: None,
        },
        "bangumi" => Application {
            client_id: "bgm74746aead5bc3bbb8",
            authorize,
            token: BANGUMI_TOKEN,
            redirect: "https://galgear.cbshuimu.net/oauth/bangumi/callback",
            session: Some(BANGUMI_SESSION),
        },
        _ => return Err(Message::new("sources.oauth_unsupported")),
    })
}

/// Serialize credential changes, including rotating a refresh token.
pub fn transaction<T>(operation: impl FnOnce() -> Result<T, Message>) -> Result<T, Message> {
    let _guard = STORE.lock().map_err(|_| Message::new("auth.store_error"))?;
    operation()
}

fn endpoints(source: &str) -> Result<(&'static str, &'static str), Message> {
    match source {
        "nextmoe" => Ok((
            "https://account.nextmoe.com/api/v1/oauth/authorize",
            "https://account.nextmoe.com/api/v1/oauth/token",
        )),
        "bangumi" => Ok((
            "https://bgm.tv/oauth/authorize",
            "https://bgm.tv/oauth/access_token",
        )),
        _ => Err(Message::new("sources.oauth_unsupported")),
    }
}
fn failure() -> Message {
    Message::new("sources.oauth_error")
}
fn random() -> Result<String, Message> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| failure())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn tokens(value: &Value, credentials: &mut Credentials) -> Result<(), Message> {
    let token = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(failure)?;
    let refresh = value["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(failure)?;
    let seconds = value["expires_in"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(failure)?;
    credentials.token = token.into();
    credentials.refresh_token = refresh.into();
    credentials.expires_at = now().saturating_add(seconds);
    Ok(())
}

/// Read credentials, refreshing expiring OAuth tokens once under the credential transaction.
pub fn load_fresh(config: &Path, source: &str) -> Result<Credentials, Message> {
    transaction(|| {
        let mut credentials = super::load(config, source)?;
        if !credentials.refresh_token.is_empty()
            && credentials.expires_at <= now().saturating_add(60)
        {
            // 不重试会轮换令牌的 POST，也不在失败时静默降为匿名。
            let value = request_json(refresh_request(source, &credentials)?)?;
            tokens(&value, &mut credentials)?;
            super::save(config, source, &credentials)?;
        }
        Ok(credentials)
    })
}

fn refresh_request(
    source: &str,
    credentials: &Credentials,
) -> Result<reqwest::blocking::RequestBuilder, Message> {
    let client = providers::client()?;
    if matches!(credentials.oauth_exchange, Exchange::BangumiBroker) {
        if source != "bangumi" {
            return Err(failure());
        }
        return Ok(client.post(BANGUMI_TOKEN).json(&serde_json::json!({
            "grant_type": "refresh_token", "refresh_token": credentials.refresh_token,
        })));
    }
    let (_, endpoint) = endpoints(source)?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", credentials.refresh_token.as_str()),
        ("client_id", credentials.client_id.as_str()),
    ];
    if source == "bangumi" {
        let redirect = if credentials.oauth_redirect_uri.is_empty() {
            "http://127.0.0.1:38947/callback"
        } else {
            &credentials.oauth_redirect_uri
        };
        form.extend([
            ("client_secret", credentials.client_secret.as_str()),
            ("redirect_uri", redirect),
        ]);
    }
    Ok(client.post(endpoint).form(&form))
}

fn callback(target: &str, redirect: &Url, state: &str) -> Result<Option<String>, Message> {
    let url = redirect.join(target).map_err(|_| failure())?;
    if url.origin() != redirect.origin() || url.path() != redirect.path() {
        return Ok(None);
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    let unique = |key: &str| {
        let mut matches = pairs.iter().filter(|(name, _)| name == key);
        let first = matches.next().map(|(_, value)| value.as_ref());
        if matches.next().is_some() {
            None
        } else {
            first
        }
    };
    if unique("state") != Some(state) {
        return Ok(None);
    }
    if unique("error").is_some() {
        return Err(failure());
    }
    Ok(unique("code").filter(|s| !s.is_empty()).map(str::to_owned))
}

/// 使用内置公开应用打开系统浏览器，等待有期限的本机回调；只有匹配的 state 可完成登录。
/// NextMoe 本机 PKCE 换码；Bangumi 通过保管应用密钥的服务端换码。
pub fn login(
    source: &str,
    credentials: Credentials,
    cancelled: &AtomicBool,
    open: impl FnOnce(&str) -> Result<(), Message>,
    reply: &str,
) -> Result<Credentials, Message> {
    login_with(
        source,
        credentials,
        cancelled,
        open,
        reply,
        &application(source)?,
    )
}

fn login_with(
    source: &str,
    mut credentials: Credentials,
    cancelled: &AtomicBool,
    open: impl FnOnce(&str) -> Result<(), Message>,
    reply: &str,
    application: &Application<'_>,
) -> Result<Credentials, Message> {
    use sha2::{Digest, Sha256};
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|_| Message::new("sources.oauth_port"))?;
    listener.set_nonblocking(true).map_err(|_| failure())?;
    let redirect = Url::parse(&format!(
        "http://{}/callback",
        listener.local_addr().map_err(|_| failure())?
    ))
    .map_err(|_| failure())?;
    let port = listener.local_addr().map_err(|_| failure())?.port();
    let nonce = random()?;
    let verifier = random()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state =
        if let Some(endpoint) = application.session {
            let value = request_json(providers::client()?.post(endpoint).json(
                &serde_json::json!({
                    "port": port, "nonce": nonce, "code_challenge": challenge,
                }),
            ))?;
            let state = value["state"].as_str().ok_or_else(failure)?;
            // 服务端签名不可在客户端验证，但必须绑定本次端口、随机值和 PKCE challenge。
            let parts: Vec<_> = state.split('.').collect();
            if parts.len() != 7
                || parts[0] != "gg2"
                || parts[1] != source
                || parts[2] != port.to_string()
                || parts[3] != nonce
                || parts[5] != challenge
                || parts[4].parse::<u64>().is_err()
                || parts[6].len() != 43
            {
                return Err(failure());
            }
            state.to_owned()
        } else {
            format!("gg1.{source}.{port}.{nonce}")
        };
    credentials.client_id = application.client_id.into();
    credentials.client_secret.clear();
    credentials.oauth_redirect_uri = application.redirect.into();
    credentials.oauth_exchange = if application.session.is_some() {
        Exchange::BangumiBroker
    } else {
        Exchange::Direct
    };
    let mut url = Url::parse(application.authorize).map_err(|_| failure())?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", credentials.client_id.as_str()),
        ("redirect_uri", application.redirect),
        ("response_type", "code"),
        ("state", state.as_str()),
    ]);
    if source == "nextmoe" {
        url.query_pairs_mut().extend_pairs([
            ("scope", "catalog:read"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(Message::new("sources.oauth_cancelled"));
    }
    open(url.as_str())?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let code = loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Message::new("sources.oauth_cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(Message::new("sources.oauth_timeout"));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|_| failure())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .map_err(|_| failure())?;
                let mut bytes = Vec::new();
                let mut byte = [0];
                while bytes.len() < 8192
                    && Instant::now() < deadline
                    && !cancelled.load(Ordering::Relaxed)
                {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes);
                let mut line = request
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split_whitespace();
                let result = if bytes.ends_with(b"\r\n\r\n") && line.next() == Some("GET") {
                    callback(line.next().unwrap_or_default(), &redirect, &state)
                } else {
                    Ok(None)
                };
                let valid = !matches!(result, Ok(None));
                let body = if valid { reply } else { "Invalid callback" };
                // 页面可能已关闭，浏览器回执失败不影响已经校验的授权结果。
                let _ = write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    if valid { "200 OK" } else { "400 Bad Request" },
                    body.len(),
                    body
                );
                if let Some(code) = result? {
                    break code;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(_) => return Err(failure()),
        }
    };
    if cancelled.load(Ordering::Relaxed) {
        return Err(Message::new("sources.oauth_cancelled"));
    }
    let client = providers::client()?;
    let request = if application.session.is_some() {
        client.post(application.token).json(&serde_json::json!({
            "grant_type": "authorization_code", "code": code,
            "state": state, "code_verifier": verifier,
        }))
    } else {
        client.post(application.token).form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", application.client_id),
            ("redirect_uri", application.redirect),
            ("code_verifier", verifier.as_str()),
        ])
    };
    let value = request_json(request)?;
    tokens(&value, &mut credentials)?;
    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_login_uses_registered_https_pkce_and_cancellation_releases_listener() {
        use std::cell::Cell;
        let cancelled = AtomicBool::new(false);
        let port = Cell::new(0);
        let result = login(
            "nextmoe",
            Credentials {
                client_id: "fixture".into(),
                ..Default::default()
            },
            &cancelled,
            |url| {
                let url = Url::parse(url).unwrap();
                let params: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
                assert_eq!(url.host_str(), Some("account.nextmoe.com"));
                assert_eq!(params["code_challenge_method"], "S256");
                assert_eq!(params["code_challenge"].len(), 43);
                assert_eq!(params["scope"], "catalog:read");
                assert_eq!(
                    params["client_id"],
                    application("nextmoe").unwrap().client_id
                );
                assert!(!params.contains_key("client_secret"));
                assert_eq!(
                    params["redirect_uri"],
                    "https://galgear.cbshuimu.net/oauth/nextmoe/callback"
                );
                let state: Vec<_> = params["state"].split('.').collect();
                assert_eq!(&state[..2], &["gg1", "nextmoe"]);
                assert_eq!(state[3].len(), 43);
                port.set(state[2].parse().unwrap());
                cancelled.store(true, Ordering::Relaxed);
                Ok(())
            },
            "Return to GalGear",
        );
        assert!(result.is_err());
        assert!(TcpListener::bind(("127.0.0.1", port.get())).is_ok());
    }
    #[test]
    fn refresh_preserves_legacy_application_and_uses_broker_only_when_recorded() {
        let mut credentials: Credentials = serde_json::from_str(r#"{"client_id":"old-id","client_secret":"fixture-secret","refresh_token":"old-refresh"}"#).unwrap();
        let request = refresh_request("bangumi", &credentials)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), "https://bgm.tv/oauth/access_token");
        let body: std::collections::BTreeMap<_, _> = reqwest::Url::parse(&format!(
            "https://fixture/?{}",
            std::str::from_utf8(request.body().unwrap().as_bytes().unwrap()).unwrap()
        ))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
        assert_eq!(body["redirect_uri"], "http://127.0.0.1:38947/callback");
        assert_eq!(body["client_id"], "old-id");
        credentials.oauth_exchange = Exchange::BangumiBroker;
        let request = refresh_request("bangumi", &credentials)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), BANGUMI_TOKEN);
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"grant_type":"refresh_token","refresh_token":"old-refresh"})
        );
        assert!(refresh_request("nextmoe", &credentials).is_err());
    }

    #[test]
    fn both_builtin_logins_exchange_callbacks_and_preserve_patch_key() {
        use sha2::{Digest, Sha256};
        use std::net::TcpStream;
        for source in ["nextmoe", "bangumi"] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/token", server.local_addr().unwrap());
            let app = Application {
                token: &endpoint,
                session: (source == "bangumi").then_some(endpoint.as_str()),
                ..application(source).unwrap()
            };
            let worker = std::thread::spawn(move || {
                let mut state = String::new();
                let mut challenge = String::new();
                for stage in 0..if source == "bangumi" { 2 } else { 1 } {
                    let (mut stream, _) = server.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    while !bytes.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        bytes.push(byte[0]);
                    }
                    let head = String::from_utf8(bytes).unwrap();
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    let value = if source == "bangumi" {
                        serde_json::from_slice::<Value>(&body).unwrap()
                    } else {
                        let url = Url::parse(&format!(
                            "https://fixture/?{}",
                            String::from_utf8(body).unwrap()
                        ))
                        .unwrap();
                        serde_json::to_value(
                            url.query_pairs()
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect::<std::collections::BTreeMap<_, _>>(),
                        )
                        .unwrap()
                    };
                    assert!(value.get("client_secret").is_none());
                    let reply = if source == "bangumi" && stage == 0 {
                        challenge = value["code_challenge"].as_str().unwrap().into();
                        state = format!("gg2.bangumi.{}.{}.{}.{}.{}",value["port"],value["nonce"].as_str().unwrap(),now()+180,challenge,"a".repeat(43));
                        serde_json::json!({"state":state})
                    } else {
                        assert_eq!(value["grant_type"], "authorization_code");
                        assert_eq!(value["code"], "fixture-code");
                        if source == "bangumi" {
                            assert_eq!(value["state"], state);
                            assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(value["code_verifier"].as_str().unwrap().as_bytes())),challenge);
                        } else {
                            assert_eq!(value["redirect_uri"], application(source).unwrap().redirect);
                            assert_eq!(value["client_id"], application(source).unwrap().client_id);
                            assert_eq!(value["code_verifier"].as_str().unwrap().len(),43);
                        }
                        serde_json::json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600})
                    }.to_string();
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",reply.len(),reply).unwrap();
                }
            });
            let value = login_with(source, Credentials { patch_key:"nmk_fixture".into(),client_secret:"old-secret".into(), ..Default::default() }, &AtomicBool::new(false), |url| {
                let url=Url::parse(url).unwrap();
                let state=url.query_pairs().find(|(k,_)| k=="state").unwrap().1.into_owned();
                let port: u16=state.split('.').nth(2).unwrap().parse().unwrap();
                let mut stream=TcpStream::connect(("127.0.0.1",port)).unwrap();
                write!(stream,"GET /callback?state={state}&code=fixture-code HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
                Ok(())
            }, "Received", &app).unwrap();
            worker.join().unwrap();
            assert_eq!(value.token, "fixture-access");
            assert_eq!(value.refresh_token, "fixture-refresh");
            assert_eq!(value.patch_key, "nmk_fixture");
            assert!(value.client_secret.is_empty());
            assert_eq!(value.oauth_redirect_uri, app.redirect);
        }
    }
    #[test]
    fn callback_requires_origin_path_unique_state_and_code() {
        let redirect = Url::parse("http://127.0.0.1:38947/callback").unwrap();
        assert_eq!(
            callback("/callback?state=s&code=a%2Bb", &redirect, "s")
                .unwrap()
                .as_deref(),
            Some("a+b")
        );
        for target in [
            "/favicon.ico",
            "/callback?state=x&code=a",
            "/callback?state=s&state=s&code=a",
            "https://evil.example/callback?state=s&code=a",
        ] {
            assert!(callback(target, &redirect, "s").unwrap().is_none());
        }
        assert!(callback("/callback?state=s&error=access_denied", &redirect, "s").is_err());
    }
    #[test]
    fn malformed_token_response_does_not_replace_existing_credentials() {
        let mut credentials = Credentials {
            token: "old".into(),
            ..Default::default()
        };
        assert!(tokens(&serde_json::json!({"access_token":"new"}), &mut credentials).is_err());
        assert_eq!(credentials.token, "old");
        let random = random().unwrap();
        assert_eq!(random.len(), 43);
    }
}
