//! AI provider discovery; API keys stay in the Windows credential vault.
use crate::credentials::{self, FanCredentials};
use galgear_i18n::Message;
use reqwest::{blocking::Client, header, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, time::Duration};

pub mod runtime;
pub mod skills;

pub const PROVIDERS: [&str; 4] = ["openai", "deepseek", "qwen", "kimi"];

pub fn select_provider(settings: &mut crate::Settings, provider: &str) -> Result<(), Message> {
    models_url(provider)?;
    settings.ai_provider = provider.into();
    Ok(())
}

pub fn selected_model(settings: &crate::Settings) -> Result<(&str, Option<&str>), Message> {
    models_url(&settings.ai_provider)?;
    Ok((
        &settings.ai_provider,
        settings
            .ai_models
            .get(&settings.ai_provider)
            .map(String::as_str)
            .filter(|model| !model.trim().is_empty()),
    ))
}

pub fn models_url(provider: &str) -> Result<&'static str, Message> {
    match provider {
        "openai" => Ok("https://api.openai.com/v1/models"),
        "deepseek" => Ok("https://api.deepseek.com/models"),
        "qwen" => Ok("https://dashscope.aliyuncs.com/compatible-mode/v1/models"),
        "kimi" => Ok("https://api.moonshot.cn/v1/models"),
        _ => Err(Message::new("ai.invalid_provider")),
    }
}

pub fn credential_target(config: &Path, provider: &str) -> Result<String, Message> {
    models_url(provider)?;
    let normalized = config.to_string_lossy().replace('/', "\\").to_lowercase();
    Ok(format!(
        "GalGear/AI/{provider}/{:x}",
        Sha256::digest(normalized.as_bytes())
    ))
}

pub fn validate_token(token: &str) -> Result<(), Message> {
    if token.is_empty() || token.len() > 1024 || !token.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Message::new("ai.invalid_token"));
    }
    Ok(())
}

pub fn token_for(config: &Path, provider: &str, draft: &str) -> Result<String, Message> {
    let token = if draft.trim().is_empty() {
        credentials::load(&credential_target(config, provider)?)?.token
    } else {
        draft.trim().to_owned()
    };
    validate_token(&token)?;
    Ok(token)
}

pub fn save_token(config: &Path, provider: &str, token: String) -> Result<(), Message> {
    validate_token(&token)?;
    credentials::save(
        &credential_target(config, provider)?,
        &FanCredentials {
            token,
            mode: "bearer".into(),
            ..Default::default()
        },
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, alias = "context_length")]
    pub context_window: Option<u64>,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<Model>,
}

fn parse_models(bytes: &[u8]) -> Result<Vec<Model>, Message> {
    let mut models: Vec<Model> = serde_json::from_slice::<ModelList>(bytes)
        .map_err(|_| Message::new("ai.invalid_response"))?
        .data;
    models.retain(|model| !model.id.trim().is_empty());
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

fn fetch_models(client: &Client, url: &str, token: &str) -> Result<Vec<Model>, Message> {
    validate_token(token)?;
    let mut authorization = header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| Message::new("ai.invalid_token"))?;
    authorization.set_sensitive(true);
    let response = client
        .get(url)
        .header(header::AUTHORIZATION, authorization)
        .send()
        .map_err(|_| Message::new("ai.network_error"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 => Message::new("ai.unauthorized"),
            403 => Message::new("ai.forbidden"),
            429 => Message::new("ai.rate_limited"),
            _ => Message::new("ai.http_error").arg("status", status.as_u16()),
        });
    }
    // Bound provider responses and never surface response bodies containing account data.
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Message::new("ai.network_error"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Message::new("ai.invalid_response"));
    }
    parse_models(&bytes)
}

fn client_builder(provider: &str) -> Result<reqwest::blocking::ClientBuilder, Message> {
    models_url(provider)?;
    let builder = Client::builder();
    // OpenAI follows the configured proxy; domestic providers always connect directly.
    Ok(if provider == "openai" {
        builder
    } else {
        builder.no_proxy()
    })
}

pub fn list_models(provider: &str, token: &str) -> Result<Vec<Model>, Message> {
    let url = models_url(provider)?;
    let client = client_builder(provider)?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .user_agent("GalGear/0.1")
        .build()
        .map_err(|_| Message::new("ai.network_error"))?;
    fetch_models(&client, url, token)
}

/// Bounded chat completion shared by setup and progress summaries. Never return API bodies.
pub fn complete(
    provider: &str,
    token: &str,
    model: &str,
    system: &str,
    input: &str,
    json: bool,
) -> Result<String, Message> {
    validate_token(token)?;
    let (url, body) = completion_request(provider, model, system, input, json)?;
    let client = client_builder(provider)?
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(Policy::none())
        .build()
        .map_err(|_| Message::new("ai.network_error"))?;
    complete_with_client(&client, url, token, &body, Duration::from_millis(500))
}

fn completion_request(
    provider: &str,
    model: &str,
    system: &str,
    input: &str,
    json: bool,
) -> Result<(&'static str, serde_json::Value), Message> {
    let url = match provider {
        "deepseek" => "https://api.deepseek.com/chat/completions",
        "openai" => "https://api.openai.com/v1/chat/completions",
        "qwen" => "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
        "kimi" => "https://api.moonshot.cn/v1/chat/completions",
        _ => return Err(Message::new("ai.invalid_provider")),
    };
    let mut body = serde_json::json!({"model":model,"messages":[{"role":"system","content":system},{"role":"user","content":input}],"max_tokens":8192});
    if provider == "openai" {
        body.as_object_mut().map(|body| body.remove("max_tokens"));
        body["max_completion_tokens"] = serde_json::json!(8192);
    }
    if provider == "deepseek" {
        body["thinking"] = serde_json::json!({"type":"disabled"});
    }
    // DashScope's hybrid models require thinking disabled for JSON/non-streaming calls.
    if provider == "qwen" {
        body["enable_thinking"] = serde_json::json!(false);
    }
    // Only hybrid Kimi models accept disabled thinking; reasoning-only models do not.
    if provider == "kimi"
        && ["kimi-k2.5", "kimi-k2.6"].iter().any(|id| {
            model == *id
                || model
                    .strip_prefix(id)
                    .is_some_and(|suffix| suffix.starts_with('-'))
        })
    {
        body["thinking"] = serde_json::json!({"type":"disabled"});
    }
    if json {
        body["response_format"] = serde_json::json!({"type":"json_object"});
    }
    Ok((url, body))
}

struct CompletionFailure {
    message: Message,
    retryable: bool,
}

fn complete_with_client(
    client: &Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
    retry_delay: Duration,
) -> Result<String, Message> {
    // Reuse exactly the same payload; malformed or truncated completions need a
    // caller decision, while temporary transport/provider failures may recover.
    let mut attempt = 0;
    loop {
        match complete_attempt(client, url, token, body) {
            Ok(content) => return Ok(content),
            Err(failure) if failure.retryable && attempt < 2 => {
                attempt += 1;
                std::thread::sleep(retry_delay * attempt);
            }
            Err(failure) => return Err(failure.message),
        }
    }
}

fn transient_network_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_body()
}

fn transient_body_error(error: &std::io::Error) -> bool {
    if let Some(error) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<reqwest::Error>())
    {
        return transient_network_error(error);
    }
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
    )
}

fn complete_attempt(
    client: &Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<String, CompletionFailure> {
    let invalid_response = || CompletionFailure {
        message: Message::new("ai.invalid_response"),
        retryable: false,
    };
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .map_err(|error| CompletionFailure {
            message: Message::new("ai.network_error"),
            retryable: transient_network_error(&error),
        })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(CompletionFailure {
            message: match status {
                401 => Message::new("ai.unauthorized"),
                403 => Message::new("ai.forbidden"),
                429 => Message::new("ai.rate_limited"),
                code => Message::new("ai.http_error").arg("status", code),
            },
            retryable: matches!(status, 429 | 500 | 502 | 503 | 504),
        });
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CompletionFailure {
            message: Message::new("ai.network_error"),
            retryable: transient_body_error(&error),
        })?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid_response());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid_response())?;
    let choice = &value["choices"][0];
    if choice["finish_reason"] != "stop" {
        let reason = choice["finish_reason"]
            .as_str()
            .filter(|reason| {
                matches!(
                    *reason,
                    "length"
                        | "content_filter"
                        | "tool_calls"
                        | "function_call"
                        | "insufficient_system_resource"
                )
            })
            .unwrap_or("unknown");
        return Err(CompletionFailure {
            message: Message::new("summary.incomplete_response").arg("reason", reason),
            retryable: false,
        });
    }
    choice["message"]["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(invalid_response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, sync::mpsc, thread};

    #[test]
    fn active_provider_persists_and_never_selects_another_providers_model() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::new(temp.path().join("settings.json"));
        let mut settings = store.load().unwrap();
        settings
            .ai_models
            .insert("openai".into(), "openai-selected-model".into());
        settings
            .ai_models
            .insert("deepseek".into(), "deepseek-selected-model".into());
        select_provider(&mut settings, "openai").unwrap();
        store.save(&settings).unwrap();
        let mut restored = store.load().unwrap();
        assert_eq!(
            selected_model(&restored).unwrap(),
            ("openai", Some("openai-selected-model"))
        );
        restored.ai_models.remove("openai");
        assert_eq!(selected_model(&restored).unwrap(), ("openai", None));
        assert!(select_provider(&mut restored, "unknown-provider").is_err());
        assert_eq!(restored.ai_provider, "openai");
    }

    #[test]
    fn discovery_accepts_provider_formats_and_new_model_ids() {
        let models = parse_models(br#"{"object":"list","data":[{"id":"future-model","name":"Future Model","context_window":100},{"id":"alpha"},{"id":"future-model"},{"id":""}]}"#).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "future-model"]
        );
        assert_eq!(models[1].name, "Future Model");
        assert!(parse_models(br#"{"error":{"message":"secret"}}"#).is_err());
    }

    #[test]
    fn domestic_providers_persist_independent_models_and_credential_targets() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::new(temp.path().join("settings.json"));
        let mut settings = store.load().unwrap();
        let mut targets = std::collections::BTreeSet::new();
        for provider in PROVIDERS {
            settings
                .ai_models
                .insert(provider.into(), format!("{provider}-model"));
            select_provider(&mut settings, provider).unwrap();
            store.save(&settings).unwrap();
            let restored = store.load().unwrap();
            assert_eq!(
                selected_model(&restored).unwrap(),
                (provider, Some(format!("{provider}-model").as_str()))
            );
            assert!(targets.insert(credential_target(&store.path, provider).unwrap()));
        }
        assert_eq!(store.load().unwrap().ai_models.len(), 4);
    }

    #[test]
    fn kimi_discovery_reads_context_length() {
        let models =
            parse_models(br#"{"data":[{"id":"kimi-k2.6","context_length":262144}]}"#).unwrap();
        assert_eq!(models[0].context_window, Some(262144));
    }

    #[test]
    fn tokens_reject_header_injection_and_provider_targets_are_isolated() {
        for token in ["", "sk-token\r\nInjected: yes", "sk-token secret", "密钥"] {
            assert!(validate_token(token).is_err());
        }
        assert!(models_url("unknown").is_err());
        assert_ne!(
            credential_target(Path::new("settings.json"), "openai").unwrap(),
            credential_target(Path::new("settings.json"), "deepseek").unwrap()
        );
    }

    #[test]
    #[cfg(windows)]
    fn tokens_and_selected_models_survive_reopening_without_serializing_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::new(temp.path().join("settings.json"));
        let mut settings = store.load().unwrap();
        settings
            .ai_models
            .insert("openai".into(), "future-model".into());
        settings
            .ai_models
            .insert("deepseek".into(), "other-model".into());
        store.save(&settings).unwrap();
        let target = credential_target(&store.path, "openai").unwrap();
        save_token(&store.path, "openai", "test-only-private-token".into()).unwrap();
        let restored = credentials::load(&target);
        credentials::clear(&target).unwrap();
        assert_eq!(restored.unwrap().token, "test-only-private-token");
        assert!(credentials::load(&target).unwrap().token.is_empty());
        assert_eq!(store.load().unwrap().ai_models, settings.ai_models);
        assert!(
            !std::fs::read_to_string(&store.path)
                .unwrap()
                .contains("private-token")
        );
    }

    fn serve(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/models", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, handle)
    }

    #[test]
    fn discovery_sends_bearer_authentication_and_redacts_remote_errors() {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (url, server) = serve("200 OK", r#"{"data":[{"id":"new-model"}]}"#);
        assert_eq!(
            fetch_models(&client, &url, "test-token").unwrap()[0].id,
            "new-model"
        );
        assert!(
            server
                .join()
                .unwrap()
                .to_lowercase()
                .contains("authorization: bearer test-token\r\n")
        );
        let (url, server) = serve("401 Unauthorized", r#"{"error":{"message":"test-token"}}"#);
        let error = fetch_models(&client, &url, "test-token").unwrap_err();
        server.join().unwrap();
        assert!(
            error
                .render(galgear_i18n::Language::English)
                .contains("Token")
        );
        assert!(!format!("{error:?}").contains("test-token"));
    }

    struct CompletionServer {
        url: String,
        stop: mpsc::Sender<()>,
        server: Option<thread::JoinHandle<Vec<String>>>,
    }

    impl CompletionServer {
        fn start(responses: Vec<String>) -> Self {
            assert!(!responses.is_empty());
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/chat/completions", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let (stop, shutdown) = mpsc::channel();
            let server = thread::spawn(move || {
                let mut requests = Vec::new();
                while shutdown.try_recv().is_err() {
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                            continue;
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    };
                    // Windows may inherit the listener's nonblocking mode.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                    }
                    let headers = std::str::from_utf8(&request).unwrap();
                    let length = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    request.extend(body);
                    let response = &responses[requests.len().min(responses.len() - 1)];
                    requests.push(String::from_utf8(request).unwrap());
                    stream.write_all(response.as_bytes()).unwrap();
                }
                requests
            });
            Self {
                url,
                stop,
                server: Some(server),
            }
        }

        fn requests(mut self) -> Vec<String> {
            self.stop.send(()).unwrap();
            self.server.take().unwrap().join().unwrap()
        }
    }

    impl Drop for CompletionServer {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
        }
    }

    fn completion_client() -> Client {
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    fn completion_reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    const COMPLETED: &str = r#"{"choices":[{"finish_reason":"stop","message":{"content":"complete recap and progress"}}]}"#;

    #[test]
    fn domestic_completion_requests_use_provider_parameters_and_parse_final_content() {
        for (provider, model) in [("qwen", "qwen-plus"), ("kimi", "kimi-k2.6")] {
            for json in [false, true] {
                let (url, body) =
                    completion_request(provider, model, "Return JSON", "verified script", json)
                        .unwrap();
                assert!(url.ends_with("/chat/completions"));
                assert_eq!(
                    url.strip_suffix("chat/completions").unwrap(),
                    models_url(provider)
                        .unwrap()
                        .strip_suffix("models")
                        .unwrap()
                );
                assert_eq!(body["max_tokens"], 8192);
                assert!(body.get("max_completion_tokens").is_none());
                assert_eq!(body.get("response_format").is_some(), json);
                if provider == "qwen" {
                    assert_eq!(body["enable_thinking"], false);
                    assert!(body.get("thinking").is_none());
                } else {
                    assert_eq!(body["thinking"]["type"], "disabled");
                    assert!(body.get("enable_thinking").is_none());
                }
                let server = CompletionServer::start(vec![completion_reply(
                    "200 OK",
                    r#"{"choices":[{"finish_reason":"stop","message":{"reasoning_content":"internal reasoning","content":"{\"recap\":\"verified\"}"}}]}"#,
                )]);
                let result = complete_with_client(
                    &completion_client(),
                    &server.url,
                    "test-token",
                    &body,
                    Duration::ZERO,
                )
                .unwrap();
                assert_eq!(result, r#"{"recap":"verified"}"#);
                let requests = server.requests();
                assert_eq!(requests.len(), 1);
                let sent: serde_json::Value =
                    serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(sent, body);
                assert_eq!(sent["model"], model);
                assert_eq!(sent["messages"][1]["content"], "verified script");
            }
        }
    }

    #[test]
    fn kimi_reasoning_only_and_legacy_models_keep_default_thinking() {
        for model in [
            "kimi-k2-thinking",
            "kimi-k2.7-code",
            "kimi-k3.1-pro",
            "moonshot-v1-32k",
        ] {
            let (_, body) = completion_request("kimi", model, "system", "input", true).unwrap();
            assert!(body.get("thinking").is_none(), "{model}");
            assert!(body.get("temperature").is_none(), "{model}");
        }
    }

    #[test]
    fn completion_retries_rate_limit_with_the_same_request_payload() {
        let server = CompletionServer::start(vec![
            completion_reply(
                "429 Too Many Requests",
                r#"{"error":{"message":"private-token"}}"#,
            ),
            completion_reply("200 OK", COMPLETED),
        ]);
        let body = serde_json::json!({"model":"selected-model", "messages":[{"role":"user", "content":"verified script"}], "response_format":{"type":"json_object"}});
        let result = complete_with_client(
            &completion_client(),
            &server.url,
            "private-token",
            &body,
            Duration::ZERO,
        )
        .unwrap();
        let requests = server.requests();
        assert_eq!(result, "complete recap and progress");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
    }

    #[test]
    fn completion_does_not_retry_authentication_or_billing_failures() {
        for (status, key) in [
            ("401 Unauthorized", "ai.unauthorized"),
            ("402 Payment Required", "ai.http_error"),
            ("403 Forbidden", "ai.forbidden"),
        ] {
            let server = CompletionServer::start(vec![completion_reply(
                status,
                r#"{"error":{"message":"private-token"}}"#,
            )]);
            let error = complete_with_client(
                &completion_client(),
                &server.url,
                "private-token",
                &serde_json::json!({}),
                Duration::ZERO,
            )
            .unwrap_err();
            assert_eq!((error.key.as_str(), server.requests().len()), (key, 1));
            assert!(!format!("{error:?}").contains("private-token"));
        }
    }

    #[test]
    fn completion_stops_after_three_temporary_provider_failures() {
        let server = CompletionServer::start(vec![completion_reply(
            "503 Service Unavailable",
            r#"{"error":{"message":"private-token"}}"#,
        )]);
        let error = complete_with_client(
            &completion_client(),
            &server.url,
            "private-token",
            &serde_json::json!({}),
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(
            (error.key.as_str(), server.requests().len()),
            ("ai.http_error", 3)
        );
    }

    #[test]
    fn completion_retries_an_interrupted_response_body() {
        let partial = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 500\r\nConnection: close\r\n\r\n{\"choices\":";
        let server =
            CompletionServer::start(vec![partial.into(), completion_reply("200 OK", COMPLETED)]);
        let result = complete_with_client(
            &completion_client(),
            &server.url,
            "private-token",
            &serde_json::json!({}),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(
            (result.as_str(), server.requests().len()),
            ("complete recap and progress", 2)
        );
    }

    #[test]
    fn completion_rejects_truncated_generation_without_replaying_paid_request() {
        let server = CompletionServer::start(vec![completion_reply(
            "200 OK",
            r#"{"choices":[{"finish_reason":"length","message":{"content":"partial recap"}}]}"#,
        )]);
        let error = complete_with_client(
            &completion_client(),
            &server.url,
            "private-token",
            &serde_json::json!({}),
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(
            (error.key.as_str(), server.requests().len()),
            ("summary.incomplete_response", 1)
        );
    }

    #[test]
    fn completion_does_not_surface_unknown_remote_finish_reason() {
        let server = CompletionServer::start(vec![completion_reply(
            "200 OK",
            r#"{"choices":[{"finish_reason":"private-token","message":{"content":"complete"}}]}"#,
        )]);
        let error = complete_with_client(
            &completion_client(),
            &server.url,
            "private-token",
            &serde_json::json!({}),
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(
            error.args.get("reason").map(String::as_str),
            Some("unknown")
        );
        assert!(!format!("{error:?}").contains("private-token"));
        assert_eq!(server.requests().len(), 1);
    }
}
