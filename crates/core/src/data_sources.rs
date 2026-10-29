//! Metadata providers share the existing local cache and Windows credential vault.
use crate::{
    credentials::{self, FanCredentials},
    providers::{self, Metadata, MetadataLink, Resource},
};
use galgear_i18n::Message;
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

mod nextmoe;

pub const SOURCES: [&str; 5] = ["2dfan", "vndb", "bangumi", "ymgal", "nextmoe"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cover {
    pub title: String,
    pub url: String,
    pub image_url: String,
    pub thumbnail_url: String,
}

fn add_cover(covers: &mut Vec<Cover>, title: &str, url: &str, image: &Value) {
    let Some(image_url) = image["url"].as_str().filter(|url| !url.is_empty()) else {
        return;
    };
    if covers.iter().any(|cover| cover.image_url == image_url) {
        return;
    }
    covers.push(Cover {
        title: title.into(),
        url: url.into(),
        image_url: image_url.into(),
        thumbnail_url: image["thumbnail"]
            .as_str()
            .filter(|url| !url.is_empty())
            .unwrap_or(image_url)
            .into(),
    });
}

fn add_release_covers(covers: &mut Vec<Cover>, value: &Value, url: &str, vn_id: &str) {
    for release in value["results"].as_array().into_iter().flatten() {
        for image in release["images"].as_array().into_iter().flatten() {
            if matches!(image["type"].as_str(), Some("pkgfront" | "dig"))
                && image["vn"].as_str().is_none_or(|id| id == vn_id)
            {
                add_cover(covers, &text(release, "title"), url, image);
            }
        }
    }
}

/// Available front and digital covers for one associated work, without changing its metadata.
pub fn covers(domain: &str, url: &str, credentials: &Credentials) -> Result<Vec<Cover>, Message> {
    let (source, id) = source_for_url(url)?;
    if source == "nextmoe" {
        return nextmoe::covers(&providers::client()?, &id, url, credentials);
    }
    if source != "vndb" {
        let metadata = metadata(domain, url, credentials)?;
        let mut covers = Vec::new();
        if let Some(image_url) = metadata.cover_url {
            add_cover(&mut covers, &metadata.title, url, &json!({"url":image_url}));
        }
        return Ok(covers);
    }
    let client = providers::client()?;
    let vn_id = format!("v{id}");
    let value = request_json(auth(client.post("https://api.vndb.org/kana/vn"), source, credentials)
        .json(&json!({"filters":["id","=",vn_id],"fields":"title,image.url,image.thumbnail","results":1})))?;
    let mut covers = Vec::new();
    if let Some(vn) = value["results"].as_array().and_then(|rows| rows.first()) {
        add_cover(&mut covers, &text(vn, "title"), url, &vn["image"]);
    }
    // Release images include packaging backs and discs; offer only usable front covers.
    for page in 1..=3 {
        let value = request_json(auth(client.post("https://api.vndb.org/kana/release"), source, credentials)
            .json(&json!({"filters":["vn","=",["id","=",vn_id]],"fields":"title,images.url,images.thumbnail,images.type,images.vn","results":100,"page":page})))?;
        add_release_covers(&mut covers, &value, url, &vn_id);
        if value["more"] != true {
            break;
        }
    }
    Ok(covers)
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    pub token: String,
    pub client_id: String,
    pub client_secret: String,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataSourceCredentials([redacted])")
    }
}
#[derive(Serialize)]
pub struct SourceStatus {
    pub source: String,
    pub configured: bool,
}
pub fn validate_source(source: &str) -> Result<(), Message> {
    if SOURCES.contains(&source) {
        Ok(())
    } else {
        Err(Message::new("error.site_url"))
    }
}
pub fn credential_target(config: &Path, source: &str) -> Result<String, Message> {
    validate_source(source)?;
    #[cfg(windows)]
    let normalized = config.to_string_lossy().replace('/', "\\");
    #[cfg(windows)]
    let config = Path::new(&normalized);
    Ok(credentials::target(config).replace("/2DFan/", &format!("/{source}/")))
}
pub fn load(config: &Path, source: &str) -> Result<Credentials, Message> {
    let saved = credentials::load(&credential_target(config, source)?)?;
    if saved.cookie.is_empty() {
        Ok(Credentials {
            token: saved.token,
            ..Default::default()
        })
    } else {
        let mut value: Credentials =
            serde_json::from_str(&saved.cookie).map_err(|_| Message::new("auth.store_error"))?;
        value.token = saved.token;
        Ok(value)
    }
}
pub fn save(config: &Path, source: &str, value: &Credentials) -> Result<(), Message> {
    let mut rest = value.clone();
    rest.token.clear();
    let saved = FanCredentials {
        token: value.token.clone(),
        cookie: serde_json::to_string(&rest).map_err(|_| Message::new("auth.store_error"))?,
        mode: "bearer".into(),
    };
    credentials::save(&credential_target(config, source)?, &saved)
}
pub fn statuses(config: &Path) -> Result<Vec<SourceStatus>, Message> {
    ["vndb", "bangumi", "ymgal", "nextmoe"]
        .into_iter()
        .map(|source| {
            let value = load(config, source)?;
            Ok(SourceStatus {
                source: source.into(),
                configured: !value.token.is_empty() || !value.client_id.is_empty(),
            })
        })
        .collect()
}
fn api_error(detail: impl std::fmt::Display) -> Message {
    Message::detail("sources.api_error", detail)
}
fn response_json(response: Response) -> Result<Value, Message> {
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(Message::new("sources.auth_error"));
    }
    if !status.is_success() {
        return Err(api_error(format!("HTTP {status}")));
    }
    let mut bytes = Vec::new();
    response
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| api_error("Response read failed"))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(api_error("Response too large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| api_error("Invalid JSON response"))
}
fn request_json(request: RequestBuilder) -> Result<Value, Message> {
    response_json(request.send().map_err(|e| api_error(e.without_url()))?)
}
fn auth(request: RequestBuilder, source: &str, credentials: &Credentials) -> RequestBuilder {
    if credentials.token.is_empty() {
        request
    } else if source == "vndb" {
        request.header("Authorization", format!("Token {}", credentials.token))
    } else {
        request.bearer_auth(&credentials.token)
    }
}
struct YmToken {
    key: String,
    token: String,
    expires: Instant,
}
static YM_TOKEN: Mutex<Option<YmToken>> = Mutex::new(None);
fn ym_token(client: &Client, credentials: &Credentials) -> Result<String, Message> {
    let client_id = if credentials.client_id.is_empty() {
        "ymgal"
    } else {
        &credentials.client_id
    };
    let client_secret = if credentials.client_id.is_empty() {
        "luna0327"
    } else {
        &credentials.client_secret
    };
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{client_id}\0{client_secret}"))
    );
    let mut cache = YM_TOKEN
        .lock()
        .map_err(|_| Message::new("error.operation"))?;
    if let Some(saved) = cache
        .as_ref()
        .filter(|saved| saved.key == key && saved.expires > Instant::now())
    {
        return Ok(saved.token.clone());
    }
    let value = request_json(client.get("https://www.ymgal.games/oauth/token").query(&[
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("scope", "public"),
    ]))?;
    let token = value["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| Message::new("sources.auth_error"))?
        .to_owned();
    let seconds = value["expires_in"]
        .as_u64()
        .unwrap_or(3600)
        .saturating_sub(30);
    *cache = Some(YmToken {
        key,
        token: token.clone(),
        expires: Instant::now() + Duration::from_secs(seconds),
    });
    Ok(token)
}
fn ym_request(
    client: &Client,
    credentials: &Credentials,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Value, Message> {
    for attempt in 0..2 {
        let token = ym_token(client, credentials)?;
        let response = client
            .get(format!("https://www.ymgal.games{path}"))
            .query(query)
            .header("Accept", "application/json;charset=utf-8")
            .header("version", "1")
            .bearer_auth(token)
            .send()
            .map_err(|e| api_error(e.without_url()))?;
        if response.status().as_u16() == 401 && attempt == 0 {
            *YM_TOKEN
                .lock()
                .map_err(|_| Message::new("error.operation"))? = None;
            continue;
        }
        let value = response_json(response)?;
        if value["success"] != true {
            return Err(api_error(format!("YMGal code {}", value["code"])));
        }
        return Ok(value["data"].clone());
    }
    Err(Message::new("sources.auth_error"))
}
pub fn check(source: &str, credentials: &Credentials) -> Result<(), Message> {
    validate_source(source)?;
    for text in [
        &credentials.token,
        &credentials.client_id,
        &credentials.client_secret,
    ] {
        if text.contains(['\r', '\n']) || text.len() > 1024 {
            return Err(Message::new("auth.invalid"));
        }
    }
    if credentials.client_id.is_empty() != credentials.client_secret.is_empty() {
        return Err(Message::new("auth.invalid"));
    }
    let client = providers::client()?;
    match source {
        "vndb" if !credentials.token.is_empty() => {
            request_json(auth(
                client.get("https://api.vndb.org/kana/authinfo"),
                source,
                credentials,
            ))?;
        }
        "bangumi" if !credentials.token.is_empty() => {
            request_json(auth(
                client.get("https://api.bgm.tv/v0/me"),
                source,
                credentials,
            ))?;
        }
        "ymgal" => {
            ym_token(&client, credentials)?;
        }
        "nextmoe" => nextmoe::check(&client, credentials)?,
        _ => {}
    }
    Ok(())
}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
fn preferred_title(value: &Value, original: &str, chinese: &str) -> String {
    let translated = text(value, chinese);
    if translated.is_empty() {
        text(value, original)
    } else {
        translated
    }
}
fn numeric_id(value: &Value) -> Option<String> {
    value.as_u64().map(|id| id.to_string()).or_else(|| {
        value
            .as_str()
            .filter(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()))
            .map(str::to_owned)
    })
}
pub fn source_for_url(url: &str) -> Result<(&'static str, String), Message> {
    let url = Url::parse(url).map_err(|_| Message::new("error.site_url"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || (url.query().is_some()
            && !(url.host_str() == Some("developer.nextmoe.dev") && url.query() == Some("nsfw=1")))
        || url.fragment().is_some()
    {
        return Err(Message::new("error.site_url"));
    }
    let path = url.path().trim_end_matches('/');
    let (source, id) = match url.host_str() {
        Some("vndb.org") => ("vndb", path.strip_prefix("/v").map(str::to_owned)),
        Some("bgm.tv" | "bangumi.tv" | "chii.in") => {
            ("bangumi", path.strip_prefix("/subject/").map(str::to_owned))
        }
        Some("www.ymgal.games") => (
            "ymgal",
            path.to_ascii_lowercase()
                .strip_prefix("/ga")
                .map(str::to_owned),
        ),
        Some("api.nextmoe.dev") => (
            "nextmoe",
            path.strip_prefix("/v2/catalog/works/").map(str::to_owned),
        ),
        Some("developer.nextmoe.dev") => (
            "nextmoe",
            path.strip_prefix("/explore/work/").map(str::to_owned),
        ),
        _ => return Ok(("2dfan", String::new())),
    };
    let id = id
        .filter(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()))
        .ok_or_else(|| Message::new("error.site_url"))?;
    Ok((source, id.to_owned()))
}
pub fn search(
    source: &str,
    domain: &str,
    query: &str,
    credentials: &Credentials,
) -> Result<Vec<Resource>, Message> {
    validate_source(source)?;
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 95 {
        return Err(Message::new("error.metadata"));
    }
    if source == "2dfan" {
        return providers::search_fan(domain, query);
    }
    let client = providers::client()?;
    if source == "nextmoe" {
        return nextmoe::search(&client, query, credentials);
    }
    let value = match source {
        "vndb" => request_json(
            auth(
                client.post("https://api.vndb.org/kana/vn"),
                source,
                credentials,
            )
            .json(&json!({"filters":["search","=",query],"fields":"title,alttitle","results":20})),
        )?,
        "bangumi" => request_json(
            auth(
                client
                    .post("https://api.bgm.tv/v0/search/subjects")
                    .query(&[("limit", 20)]),
                source,
                credentials,
            )
            .json(&json!({"keyword":query,"filter":{"type":[4]},"sort":"match"})),
        )?,
        "ymgal" => ym_request(
            &client,
            credentials,
            "/open/archive/search-game",
            &[
                ("mode", "list"),
                ("keyword", query),
                ("pageNum", "1"),
                ("pageSize", "20"),
            ],
        )?,
        _ => return Err(Message::new("error.site_url")),
    };
    let list = match source {
        "vndb" | "ymgal" => &value["results"],
        _ => &value["data"],
    };
    let list = if source == "ymgal" {
        &value["result"]
    } else {
        list
    };
    let entries = list
        .as_array()
        .ok_or_else(|| api_error("Missing search results"))?;
    entries
        .iter()
        .map(|entry| {
            let (title, url) = match source {
                "vndb" => {
                    let id = text(entry, "id");
                    if !id.starts_with('v') || !id[1..].bytes().all(|c| c.is_ascii_digit()) {
                        return Err(api_error("Invalid VNDB ID"));
                    }
                    (
                        preferred_title(entry, "title", "alttitle"),
                        format!("https://vndb.org/{id}"),
                    )
                }
                "bangumi" => (
                    preferred_title(entry, "name", "name_cn"),
                    format!(
                        "https://bgm.tv/subject/{}",
                        numeric_id(&entry["id"]).ok_or_else(|| api_error("Invalid subject ID"))?
                    ),
                ),
                _ => (
                    preferred_title(entry, "name", "chineseName"),
                    format!(
                        "https://www.ymgal.games/ga{}",
                        numeric_id(&entry["id"]).ok_or_else(|| api_error("Invalid archive ID"))?
                    ),
                ),
            };
            Ok(Resource {
                title,
                url,
                source: source.into(),
                ..Default::default()
            })
        })
        .collect()
}
fn join_names(value: &Value) -> String {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["name"].as_str())
        .collect::<Vec<_>>()
        .join("、")
}
fn add_credit(meta: &mut Metadata, role: &str, name: &str, url: &str) {
    let role = role.to_lowercase();
    let field = if role.contains("原画")
        || role.contains("角色设计")
        || role.contains("キャラクター")
        || ["art", "chardesign"].contains(&role.as_str())
    {
        "artists"
    } else if role.contains("剧本")
        || role.contains("脚本")
        || role.contains("シナリオ")
        || role == "scenario"
    {
        "writers"
    } else if role.contains("音乐")
        || role.contains("音楽")
        || role.contains("作曲")
        || role == "music"
    {
        "music"
    } else if role.contains("歌") || role == "songs" {
        "singers"
    } else if role.contains("声优") || role.contains("声優") || role.contains("配音") {
        "voices"
    } else if role.contains("开发") || role.contains("开发商") || role.contains("游戏开发商")
    {
        "developer"
    } else if role.contains("发行") {
        "publisher"
    } else {
        return;
    };
    let mut names = meta.field(field).unwrap_or_default().to_owned();
    if names.split('、').any(|existing| existing == name) || name.is_empty() {
        return;
    }
    if !names.is_empty() {
        names.push('、');
    }
    names.push_str(name);
    let links = std::mem::take(&mut meta.links);
    meta.set_field(field, &names);
    meta.links = links;
    if !url.is_empty() {
        meta.links.push(MetadataLink {
            field: field.into(),
            name: name.into(),
            url: url.into(),
        });
    }
}
pub fn parse_vndb(value: &Value, url: &str) -> Result<Metadata, Message> {
    let vn = value["results"]
        .as_array()
        .and_then(|entries| entries.first())
        .ok_or_else(|| Message::new("error.metadata"))?;
    let mut meta = Metadata {
        url: url.into(),
        title: preferred_title(vn, "title", "alttitle"),
        description: text(vn, "description"),
        release_date: text(vn, "released"),
        developer: join_names(&vn["developers"]),
        cover_url: vn["image"]["url"].as_str().map(str::to_owned),
        schema_version: providers::METADATA_SCHEMA_VERSION,
        ..Default::default()
    };
    meta.brand.clone_from(&meta.developer);
    meta.tags = vn["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tag| tag["spoiler"].as_u64().unwrap_or(0) == 0)
        .filter_map(|tag| tag["name"].as_str().map(str::to_owned))
        .collect();
    for staff in vn["staff"].as_array().into_iter().flatten() {
        meta.tags.push(text(staff, "name"));
        add_credit(
            &mut meta,
            &text(staff, "role"),
            &text(staff, "name"),
            &format!("https://vndb.org/{}", text(staff, "id")),
        );
    }
    Ok(meta)
}
fn infobox_value(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        text.into()
    } else {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["v"].as_str())
            .collect::<Vec<_>>()
            .join("、")
    }
}
pub fn parse_bangumi(value: &Value, url: &str) -> Result<Metadata, Message> {
    if value["type"].as_u64() != Some(4) {
        return Err(Message::new("error.metadata"));
    }
    let mut meta = Metadata {
        url: url.into(),
        title: preferred_title(value, "name", "name_cn"),
        description: text(value, "summary"),
        release_date: text(value, "date"),
        cover_url: value["images"]["large"].as_str().map(str::to_owned),
        schema_version: providers::METADATA_SCHEMA_VERSION,
        ..Default::default()
    };
    meta.tags = value["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["name"].as_str().map(str::to_owned))
        .collect();
    for item in value["infobox"].as_array().into_iter().flatten() {
        add_credit(
            &mut meta,
            &text(item, "key"),
            &infobox_value(&item["value"]),
            "",
        );
    }
    meta.brand.clone_from(&meta.developer);
    Ok(meta)
}
pub fn parse_ymgal(value: &Value, url: &str) -> Result<Metadata, Message> {
    let game = &value["game"];
    if game["gid"].is_null() {
        return Err(Message::new("error.metadata"));
    }
    let mut meta = Metadata {
        url: url.into(),
        title: preferred_title(game, "name", "chineseName"),
        description: text(game, "introduction"),
        release_date: text(game, "releaseDate"),
        cover_url: game["mainImg"].as_str().map(str::to_owned),
        schema_version: providers::METADATA_SCHEMA_VERSION,
        ..Default::default()
    };
    for staff in game["staff"].as_array().into_iter().flatten() {
        meta.tags.push(text(staff, "emp_name"));
        let link = numeric_id(&staff["pid"])
            .map(|id| format!("https://www.ymgal.games/pa{id}"))
            .unwrap_or_default();
        add_credit(
            &mut meta,
            &text(staff, "job_name"),
            &text(staff, "emp_name"),
            &link,
        );
    }
    for character in game["characters"].as_array().into_iter().flatten() {
        if let Some(id) = numeric_id(&character["cvId"]) {
            add_credit(
                &mut meta,
                "声优",
                &text(&value["pidMapping"][&id], "name"),
                &format!("https://www.ymgal.games/pa{id}"),
            );
        }
    }
    Ok(meta)
}
pub fn metadata(domain: &str, url: &str, credentials: &Credentials) -> Result<Metadata, Message> {
    let (source, id) = source_for_url(url)?;
    if source == "2dfan" {
        return providers::fan_metadata(domain, url);
    }
    let client = providers::client()?;
    match source {
        "vndb" => {
            let value = request_json(auth(client.post("https://api.vndb.org/kana/vn"),source,credentials).json(&json!({"filters":["id","=",format!("v{id}")],"fields":"title,alttitle,description,released,image.url,developers.name,tags.name,tags.spoiler,staff.name,staff.role","results":1})))?;
            parse_vndb(&value, url)
        }
        "bangumi" => {
            let value = request_json(auth(
                client.get(format!("https://api.bgm.tv/v0/subjects/{id}")),
                source,
                credentials,
            ))?;
            let mut meta = parse_bangumi(&value, url)?;
            let people = request_json(auth(
                client.get(format!("https://api.bgm.tv/v0/subjects/{id}/persons")),
                source,
                credentials,
            ))?;
            for person in people.as_array().into_iter().flatten() {
                meta.tags.push(text(person, "name"));
                add_credit(
                    &mut meta,
                    &text(person, "relation"),
                    &text(person, "name"),
                    &format!("https://bgm.tv/person/{}", person["id"]),
                );
            }
            Ok(meta)
        }
        "ymgal" => {
            let value = ym_request(&client, credentials, "/open/archive", &[("gid", &id)])?;
            let mut meta = parse_ymgal(&value, url)?;
            if let Some(org_id) = numeric_id(&value["game"]["developerId"]).filter(|id| id != "0") {
                let org = ym_request(&client, credentials, "/open/archive", &[("orgId", &org_id)])?;
                meta.developer = text(&org["org"], "name");
                meta.brand.clone_from(&meta.developer);
                if !meta.brand.is_empty() {
                    meta.links.push(MetadataLink {
                        field: "brand".into(),
                        name: meta.brand.clone(),
                        url: format!("https://www.ymgal.games/oa{org_id}"),
                    });
                }
            }
            Ok(meta)
        }
        "nextmoe" => nextmoe::metadata(&client, &id, url, credentials),
        _ => Err(Message::new("error.site_url")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_covers_deduplicate_images_and_exclude_backs_and_other_bundle_games() {
        let value = json!({"results":[{"title":"Release", "images":[
            {"url":"front.jpg","thumbnail":"thumb.jpg","type":"pkgfront","vn":null},
            {"url":"front.jpg","type":"dig","vn":"v17"},
            {"url":"back.jpg","type":"pkgback","vn":null},
            {"url":"other.jpg","type":"pkgfront","vn":"v18"},
            {"url":"digital.jpg","type":"dig","vn":"v17"}
        ]}]});
        let mut covers = Vec::new();
        add_release_covers(&mut covers, &value, "https://vndb.org/v17", "v17");
        assert_eq!(
            covers
                .iter()
                .map(|cover| (cover.image_url.as_str(), cover.thumbnail_url.as_str()))
                .collect::<Vec<_>>(),
            vec![("front.jpg", "thumb.jpg"), ("digital.jpg", "digital.jpg")]
        );
    }

    #[test]
    fn provider_urls_reject_confusable_hosts_paths_and_credentials() {
        for url in [
            "https://vndb.org.evil.test/v1",
            "https://vndb.org/v1/other",
            "https://bgm.tv/subject/no",
            "https://user@www.ymgal.games/ga1",
        ] {
            assert!(source_for_url(url).is_err() || source_for_url(url).unwrap().0 == "2dfan");
        }
        assert_eq!(
            source_for_url("https://www.ymgal.games/GA31147").unwrap(),
            ("ymgal", "31147".into())
        );
    }
    #[test]
    fn nextmoe_work_urls_accept_only_catalog_paths_and_the_nsfw_switch() {
        assert_eq!(
            source_for_url("https://developer.nextmoe.dev/explore/work/15?nsfw=1").unwrap(),
            ("nextmoe", "15".into())
        );
        assert_eq!(
            source_for_url("https://developer.nextmoe.dev/explore/work/15").unwrap(),
            ("nextmoe", "15".into())
        );
        for url in [
            "https://developer.nextmoe.dev/explore/work/15?nsfw=1&other=1",
            "https://developer.nextmoe.dev/explore/work/15?nsfw=true",
            "https://developer.nextmoe.dev/explore/work/15/other",
            "https://developer.nextmoe.dev/explore/work/not-a-number",
            "https://user@developer.nextmoe.dev/explore/work/15?nsfw=1",
            "https://api.nextmoe.dev/v2/catalog/works/15?nsfw=1",
        ] {
            assert!(source_for_url(url).is_err(), "accepted {url}");
        }
    }
    #[test]
    fn source_metadata_preserves_story_credits_and_cover() {
        let value = json!({"type":4,"name":"Game","summary":"First\n\nSecond","images":{"large":"https://lain.bgm.tv/cover.jpg"},"infobox":[{"key":"开发","value":"Brand"},{"key":"原画","value":[{"v":"Artist"}]}]});
        let meta = parse_bangumi(&value, "https://bgm.tv/subject/1").unwrap();
        assert_eq!(meta.brand, "Brand");
        assert_eq!(meta.artists, "Artist");
        assert_eq!(meta.description, "First\n\nSecond");
        assert!(meta.cover_url.is_some());
    }
    #[test]
    fn ymgal_maps_staff_and_character_voice_actors() {
        let value = json!({"game":{"gid":"1","name":"Game","staff":[{"emp_name":"Writer","job_name":"剧本","pid":"8"}],"characters":[{"cvId":"9"}]},"pidMapping":{"9":{"name":"Voice"}}});
        let meta = parse_ymgal(&value, "https://www.ymgal.games/ga1").unwrap();
        assert_eq!(meta.writers, "Writer");
        assert_eq!(meta.voices, "Voice");
    }
    #[test]
    fn staff_with_other_roles_are_still_imported_as_tags() {
        let value = json!({"results":[{"title":"Game","developers":[{"name":"Studio"}],"tags":[{"name":"ADV","spoiler":0}],"staff":[{"name":"Director","role":"director"},{"name":"Alice Smith","role":"scenario"}]}]});
        let meta = parse_vndb(&value, "https://vndb.org/v1").unwrap();
        assert_eq!(
            meta.tag_names(),
            vec!["ADV", "Director", "Alice Smith", "Studio"]
        );
        let value = json!({"game":{"gid":"1","name":"Game","staff":[{"emp_name":"Producer","job_name":"制作总指挥"}]}});
        let meta = parse_ymgal(&value, "https://www.ymgal.games/ga1").unwrap();
        assert_eq!(meta.tag_names(), vec!["Producer"]);
    }
}
