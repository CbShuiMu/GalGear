//! 2DFan 只读请求与账号操作共用鉴权、Cookie、超时和同源重定向规则。
use crate::credentials::FanCredentials;
use galgear_i18n::Message;
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder},
    cookie::Jar,
    header::{AUTHORIZATION, HeaderValue},
};
use std::{io::Read, sync::Arc, time::Duration};

fn auth_error() -> Message {
    Message::new("auth.network")
}

fn cookie_jar(base: &Url, credentials: &FanCredentials) -> Result<Arc<Jar>, Message> {
    credentials.validate()?;
    let jar = Arc::new(Jar::default());
    for pair in credentials
        .cookie
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let Some((name, value)) = pair.split_once('=') else {
            return Err(Message::new("auth.invalid"));
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            || HeaderValue::from_str(value).is_err()
        {
            return Err(Message::new("auth.invalid"));
        }
        jar.add_cookie_str(&format!("{name}={value}; Path=/; Secure"), base);
    }
    Ok(jar)
}

fn builder(base: &Url, jar: Arc<Jar>) -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .user_agent("GalGear/0.1 (+local game library)")
        .cookie_provider(jar)
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::custom({
            let origin = base.origin();
            move |attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("Too many redirects")
                } else if attempt.url().origin() == origin {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }
        }))
}

pub(crate) fn clients(
    base: &Url,
    credentials: &FanCredentials,
) -> Result<(Client, Client), Message> {
    let jar = cookie_jar(base, credentials)?;
    let client = builder(base, jar.clone())
        .build()
        .map_err(|_| auth_error())?;
    let download_client = builder(base, jar)
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(1800))
        .build()
        .map_err(|_| Message::new("download.network"))?;
    Ok((client, download_client))
}

fn request(
    client: &Client,
    base: &Url,
    credentials: &FanCredentials,
    url: Url,
) -> Result<RequestBuilder, Message> {
    let mut request = client.get(url.clone());
    if url.origin() == base.origin() && !credentials.token.is_empty() {
        let (key, value) = if credentials.mode == "bearer" {
            (AUTHORIZATION, format!("Bearer {}", credentials.token))
        } else {
            (
                reqwest::header::HeaderName::from_static("x-csrf-token"),
                credentials.token.clone(),
            )
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| Message::new("auth.invalid"))?;
        value.set_sensitive(true);
        request = request.header(key, value);
    }
    Ok(request)
}

pub(crate) fn read(
    client: &Client,
    base: &Url,
    credentials: &FanCredentials,
    url: Url,
) -> Result<String, Message> {
    // Cookie 仅按主机隔离，读取前还需拒绝端口、协议或用户信息发生变化的 URL。
    if url.origin() != base.origin() || !url.username().is_empty() || url.password().is_some() {
        return Err(Message::new("error.site_url"));
    }
    let response = request(client, base, credentials, url)?
        .send()
        .map_err(|_| auth_error())?;
    crate::rate_limit::check(&response)?;
    let response = response
        .error_for_status()
        .map_err(|_| auth_error())?;
    if response.status().is_redirection() {
        return Err(auth_error());
    }
    let mut data = Vec::new();
    response
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|_| auth_error())?;
    if data.len() > 4 * 1024 * 1024 {
        return Err(Message::new("error.response_size"));
    }
    String::from_utf8(data).map_err(|_| auth_error())
}

pub(crate) fn page(
    base: &Url,
    url: Url,
    credentials: Option<&FanCredentials>,
) -> Result<String, Message> {
    let anonymous = FanCredentials {
        mode: "csrf".into(),
        ..Default::default()
    };
    let credentials = credentials.unwrap_or(&anonymous);
    let client = builder(base, cookie_jar(base, credentials)?)
        .build()
        .map_err(|_| auth_error())?;
    read(&client, base, credentials, url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::cookie::CookieStore;

    #[test]
    fn authenticated_reads_stop_cross_origin_redirects() {
        use std::io::Write;
        let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        other.set_nonblocking(true).unwrap();
        let base = Url::parse(&format!("http://{}/", origin.local_addr().unwrap())).unwrap();
        let destination = format!("http://{}/private", other.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = origin.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0 && bytes.len() < 8192);
                bytes.extend_from_slice(&buffer[..n]);
            }
            assert!(
                String::from_utf8(bytes)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("x-csrf-token: fixture-token")
            );
            write!(socket, "HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let credentials = FanCredentials {
            mode: "csrf".into(),
            token: "fixture-token".into(),
            ..Default::default()
        };
        let client = builder(&base, cookie_jar(&base, &credentials).unwrap())
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert_eq!(
            read(
                &client,
                &base,
                &credentials,
                base.join("/subjects/1").unwrap()
            )
            .unwrap_err()
            .key,
            "auth.network"
        );
        server.join().unwrap();
        assert_eq!(
            other.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn token_modes_and_cookie_share_the_configured_origin() {
        let base = Url::parse("https://2dfmax.top/").unwrap();
        for (mode, header, value) in [
            ("csrf", "x-csrf-token", "fixture-token"),
            ("bearer", "authorization", "Bearer fixture-token"),
        ] {
            let credentials = FanCredentials {
                mode: mode.into(),
                token: "fixture-token".into(),
                cookie: "session=fixture-session; level=fixture-level".into(),
            };
            let jar = cookie_jar(&base, &credentials).unwrap();
            let client = builder(&base, jar.clone()).build().unwrap();
            let url = base.join("/subjects/123").unwrap();
            let request = request(&client, &base, &credentials, url.clone())
                .unwrap()
                .build()
                .unwrap();
            assert_eq!(request.headers()[header], value);
            assert!(request.headers()[header].is_sensitive());
            let cookies = jar.cookies(&url).unwrap();
            let cookies = cookies.to_str().unwrap();
            assert!(cookies.contains("session=fixture-session"));
            assert!(cookies.contains("level=fixture-level"));
            for foreign in ["https://api.nextmoe.dev/", "https://2dfmax.top.evil.test/"] {
                let url = Url::parse(foreign).unwrap();
                assert!(jar.cookies(&url).is_none());
                assert_eq!(
                    read(&client, &base, &credentials, url).unwrap_err().key,
                    "error.site_url"
                );
            }
            assert!(
                jar.cookies(&Url::parse("http://2dfmax.top/").unwrap())
                    .is_none()
            );
            assert_eq!(
                read(
                    &client,
                    &base,
                    &credentials,
                    Url::parse("https://2dfmax.top:8443/subjects/1").unwrap()
                )
                .unwrap_err()
                .key,
                "error.site_url"
            );
        }
    }

    #[test]
    fn absent_or_partial_credentials_keep_available_authentication() {
        let base = Url::parse("https://2dfmax.top/").unwrap();
        for (token, cookie) in [
            ("", ""),
            ("fixture-token", ""),
            ("", "session=fixture-session"),
        ] {
            let credentials = FanCredentials {
                mode: "csrf".into(),
                token: token.into(),
                cookie: cookie.into(),
            };
            let jar = cookie_jar(&base, &credentials).unwrap();
            let client = builder(&base, jar.clone()).build().unwrap();
            let request = request(&client, &base, &credentials, base.clone())
                .unwrap()
                .build()
                .unwrap();
            assert_eq!(
                request.headers().contains_key("x-csrf-token"),
                !token.is_empty()
            );
            assert_eq!(jar.cookies(&base).is_some(), !cookie.is_empty());
        }
        let invalid = FanCredentials {
            mode: "csrf".into(),
            cookie: "malformed-cookie".into(),
            ..Default::default()
        };
        assert_eq!(
            page(&base, base.clone(), Some(&invalid)).unwrap_err().key,
            "auth.invalid"
        );
    }
}
