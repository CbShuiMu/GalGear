//! NextMoe 联邦补丁搜索与托管文件下载。文件接口须具备运营授予的 moyu:files 权限。
use super::{Credentials, api_error, nextmoe, request_json, text};
use crate::providers::{self, Resource};
use galgear_i18n::Message;
use reqwest::{Url, blocking::Client};
use serde_json::Value;
use std::collections::BTreeSet;

const API: &str = "https://api.nextmoe.dev/v2/moyu";

fn resource_id(id: &str) -> Result<&str, Message> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) {
        return Err(api_error("Invalid NextMoe resource ID"));
    }
    Ok(id)
}

fn file_response(response: reqwest::blocking::Response) -> Result<Value, Message> {
    match response.status().as_u16() {
        401 | 403 => Err(Message::new("patches.nextmoe_download_key")),
        404 => Err(Message::new("patches.nextmoe_file_missing")),
        _ => super::response_json(response),
    }
}

struct File {
    url: Url,
    filename: String,
    bytes: u64,
    password: String,
}

fn file(value: &Value, id: &str) -> Result<File, Message> {
    let invalid = || api_error("Invalid NextMoe file response");
    let url =
        Url::parse(value["download_url"].as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let filename = value["filename"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)?;
    let bytes = value["bytes"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(invalid)?;
    if value["resource_id"].as_str() != Some(id)
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    if bytes > 8 * 1024 * 1024 * 1024 {
        return Err(Message::new("download.too_large"));
    }
    Ok(File {
        url,
        filename: filename.into(),
        bytes,
        password: text(value, "password"),
    })
}

/// 用户点击下载后才请求文件直链；不缓存直链、不向文件主机发送应用密钥。
pub fn download(
    id: &str,
    credentials: &Credentials,
    directory: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(crate::downloads::Transfer),
) -> Result<crate::downloads::Transfer, Message> {
    use std::sync::atomic::Ordering;
    let id = resource_id(id)?;
    let key = key(credentials).map_err(|_| Message::new("patches.nextmoe_download_key"))?;
    let client = providers::client()?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Message::new("download.cancelled"));
    }
    let metadata = file_response(
        client
            .get(format!("{API}/resources/{id}"))
            .bearer_auth(key)
            .send()
            .map_err(|_| Message::new("download.network"))?,
    )?;
    if metadata["storage"] != "s3" {
        return Err(Message::new("patches.nextmoe_file_missing"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Message::new("download.cancelled"));
    }
    let value = file_response(
        client
            .get(format!("{API}/resources/{id}/file"))
            .bearer_auth(key)
            .send()
            .map_err(|_| Message::new("download.network"))?,
    )?;
    let file = file(&value, id)?;
    let resource = resource(&metadata, "")?;
    if resource.download_id.as_deref() != Some(id) {
        return Err(api_error("Resource identity mismatch"));
    }
    let title = resource.title;
    let mut transfer = crate::downloads::Transfer::default();
    let path = crate::downloads::download(
        &crate::downloads::FileInfo {
            url: &file.url,
            referer: &resource.url,
            title: &title,
            filename: Some(&file.filename),
            bytes: Some(file.bytes),
        },
        directory,
        cancel,
        &mut |update| {
            transfer = update.clone();
            if update.phase != "complete" {
                progress(update);
            }
        },
    )?;
    transfer.path = Some(path);
    transfer.archive_password = (!file.password.is_empty()).then_some(file.password);
    Ok(transfer)
}

fn key(credentials: &Credentials) -> Result<&str, Message> {
    let key = credentials.patch_key.trim();
    if key.starts_with("nmk_") {
        Ok(key)
    } else {
        Err(Message::new("patches.nextmoe_key"))
    }
}

fn items(value: &Value) -> Result<&Vec<Value>, Message> {
    if value["object"] != "list" {
        return Err(api_error("Invalid NextMoe patch list"));
    }
    value["items"]
        .as_array()
        .ok_or_else(|| api_error("Missing NextMoe patch items"))
}

/// Validate a user-supplied application key without requesting file permissions.
pub fn check(credentials: &Credentials) -> Result<(), Message> {
    let value = request_json(
        providers::client()?
            .get(format!("{API}/patches"))
            .bearer_auth(key(credentials)?)
            .query(&[("limit", "1")]),
    )?;
    items(&value)?;
    Ok(())
}

fn resource(row: &Value, title: &str) -> Result<Resource, Message> {
    let url = text(row, "web_url");
    let parsed = Url::parse(&url).map_err(|_| api_error("Invalid NextMoe resource URL"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("www.moyu.moe")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(api_error("Invalid NextMoe resource URL"));
    }
    let name = text(row, "name");
    if row["object"] != "patch_resource" || name.trim().is_empty() {
        return Err(api_error("Invalid NextMoe resource"));
    }
    Ok(Resource {
        cover_url: None, brand: String::new(), release_date: String::new(),
        download_id: Some(resource_id(row["id"].as_str().unwrap_or_default())?.into()),
        title: name,
        url,
        source: "NextMoe".into(),
        game_title: title.into(),
        description: text(row, "note"),
        size: text(row, "size"),
        author: text(row, "localization_group_name"),
        uploaded_at: text(row, "created_at"),
        tags: ["type", "language", "platform"]
            .iter()
            .flat_map(|key| row[*key].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    })
}

fn resources(
    client: &Client,
    patch: &Value,
    title: &str,
    key: &str,
) -> Result<Vec<Resource>, Message> {
    let id = patch["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| api_error("Invalid NextMoe patch ID"))?;
    read_resources(client, &format!("{API}/patches/{id}/resources"), title, key)
}

fn read_resources(
    client: &Client,
    url: &str,
    title: &str,
    key: &str,
) -> Result<Vec<Resource>, Message> {
    let mut output = Vec::new();
    let mut cursor = String::new();
    let mut seen = BTreeSet::new();
    loop {
        let mut request = client.get(url).bearer_auth(key).query(&[("limit", "100")]);
        if !cursor.is_empty() {
            request = request.query(&[("cursor", &cursor)]);
        }
        let value = request_json(request)?;
        for row in items(&value)? {
            output.push(resource(row, title)?);
        }
        match value["next_cursor"].as_str().filter(|s| !s.is_empty()) {
            None => break,
            Some(next) if seen.len() < 100 && seen.insert(next.to_owned()) => cursor = next.into(),
            _ => return Err(api_error("Invalid or excessive NextMoe pagination")),
        }
    }
    Ok(output)
}

/// Resolve game names through catalog, then fetch every resource of the matched patch pages.
/// Requires the user's application key; catalog user tokens cannot access the federation API.
pub fn search(query: &str, credentials: &Credentials) -> Result<Vec<Resource>, Message> {
    let key = key(credentials)?;
    let client = providers::client()?;
    // 补丁密钥未必有 catalog:read；作品解析使用公开目录，不扩大密钥权限。
    let catalog_credentials = Credentials::default();
    let works = nextmoe::search(&client, query, &catalog_credentials)?;
    if works.is_empty() {
        return Ok(Vec::new());
    }
    let mut titles = std::collections::BTreeMap::new();
    for work in works {
        let (_, id) = super::source_for_url(&work.url)?;
        titles.insert(id, work.title);
    }
    let refs = titles
        .keys()
        .map(|id| format!("catalog:{id}"))
        .collect::<Vec<_>>()
        .join(",");
    let value = request_json(
        client
            .get(format!("{API}/patches"))
            .bearer_auth(key)
            .query(&[("refs", refs.as_str()), ("nsfw", "true")]),
    )?;
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    for patch in items(&value)? {
        let title = patch["catalog_work_id"]
            .as_str()
            .and_then(|id| titles.get(id))
            .map(String::as_str)
            .unwrap_or(query);
        for resource in resources(&client, patch, title, key)? {
            if seen.insert(resource.url.clone()) {
                output.push(resource);
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_access_failures_explain_credentials_or_missing_hosted_file() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::Duration,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in [401, 403, 404] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
                write!(
                    stream,
                    "HTTP/1.1 {status} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for expected in [
            "patches.nextmoe_download_key",
            "patches.nextmoe_download_key",
            "patches.nextmoe_file_missing",
        ] {
            let response = client.get(&url).send().unwrap();
            assert_eq!(file_response(response).unwrap_err().key, expected);
        }
        server.join().unwrap();
    }
    #[test]
    fn file_metadata_requires_matching_identity_https_and_size() {
        let mut data = json!({"resource_id":"7","download_url":"https://cdn.example/file?signature=private","filename":"patch.ZIP","bytes":42,"password":"archive-pass"});
        let parsed = file(&data, "7").unwrap();
        assert_eq!(parsed.bytes, 42);
        assert_eq!(parsed.password, "archive-pass");
        assert!(file(&data, "8").is_err());
        data["download_url"] = json!("file:///C:/private");
        assert!(file(&data, "7").is_err());
        for id in [
            "",
            "https://evil.example/resource/7",
            "../7",
            "7/file",
            "7?other=1",
        ] {
            assert!(resource_id(id).is_err());
        }
        assert_eq!(resource_id("7").unwrap(), "7");
        let result = download(
            "7",
            &Credentials::default(),
            std::path::Path::new("unused"),
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(result.unwrap_err().key, "patches.nextmoe_download_key");
    }
    #[test]
    fn resource_pagination_uses_opaque_cursor_and_bearer_key() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/patches/11617/resources",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            for page in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..count]);
                assert!(request.starts_with("GET /patches/11617/resources?limit=100"));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer nmk_test_fixture")
                );
                if page == 1 {
                    assert!(request.contains("cursor=cur_%2B%2F%3D"));
                }
                let body = json!({"object":"list","items":[{"object":"patch_resource","id":(10463+page).to_string(),"name":format!("Patch {page}"),"web_url":format!("https://www.moyu.moe/resource/{}",10463+page)}],"next_cursor":if page == 0 {json!("cur_+/=")} else {Value::Null}}).to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let result = read_resources(&client, &url, "Game", "nmk_test_fixture").unwrap();
        server.join().unwrap();
        assert_eq!(
            result.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Patch 0", "Patch 1"]
        );
    }
    #[test]
    fn maps_federated_resources_and_rejects_untrusted_urls() {
        let mut row = json!({"object":"patch_resource", "id":"10463", "patch_id":"11617", "name":"Patch", "web_url":"https://www.moyu.moe/resource/10463", "type":["manual"], "language":["zh-Hans"], "localization_group_name":"Group", "created_at":"2026-10-01T00:00:00Z", "size":"1 MB"});
        let parsed = resource(&row, "Game").unwrap();
        assert_eq!(
            (
                parsed.game_title.as_str(),
                parsed.author.as_str(),
                parsed.source.as_str()
            ),
            ("Game", "Group", "NextMoe")
        );
        assert_eq!(parsed.tags, ["manual", "zh-Hans"]);
        row["web_url"] = json!("https://evil.example/resource/10463");
        assert!(resource(&row, "Game").is_err());
        assert!(items(&json!({"code":"RATE_LIMITED"})).is_err());
        assert!(
            items(&json!({"object":"list","items":[]}))
                .unwrap()
                .is_empty()
        );
        assert!(
            key(&Credentials {
                token: "user-token".into(),
                ..Default::default()
            })
            .is_err()
        );
    }
}
