use galgear_i18n::Message;
use reqwest::{Url, blocking::Client};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::time::Duration;

/// Version of the complete credits and local-tag cache format.
pub const METADATA_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Metadata {
    pub url: String,
    pub title: String,
    pub description: String,
    pub developer: String,
    pub release_date: String,
    pub cover_url: Option<String>,
    pub publisher: String,
    pub brand: String,
    pub artists: String,
    pub voices: String,
    pub writers: String,
    pub music: String,
    pub singers: String,
    pub tags: Vec<String>,
    pub links: Vec<MetadataLink>,
    pub schema_version: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MetadataLink {
    pub field: String,
    pub name: String,
    pub url: String,
}

impl Metadata {
    /// All imported categories, people and organizations as local tag names.
    pub fn tag_names(&self) -> Vec<String> {
        crate::tags::normalize(self.tags.iter().cloned().chain(
            crate::tags::CREDIT_FIELDS.iter().flat_map(|field| {
                crate::tags::credit_names(
                    self.field(field).unwrap_or_default(),
                    self.links
                        .iter()
                        .filter(|link| link.field == *field)
                        .map(|link| link.name.as_str()),
                )
            }),
        ))
    }

    /// Return the cached value of an editable metadata field.
    pub fn field(&self, field: &str) -> Option<&str> {
        Some(match field {
            "title" => &self.title,
            "description" => &self.description,
            "developer" => &self.developer,
            "publisher" => &self.publisher,
            "release_date" => &self.release_date,
            "brand" => &self.brand,
            "artists" => &self.artists,
            "voices" => &self.voices,
            "writers" => &self.writers,
            "music" => &self.music,
            "singers" => &self.singers,
            _ => return None,
        })
    }
    /// Apply a single manually protected field, including deliberate empty values.
    pub fn set_field(&mut self, field: &str, value: &str) -> bool {
        let target = match field {
            "title" => &mut self.title,
            "description" => &mut self.description,
            "developer" => &mut self.developer,
            "publisher" => &mut self.publisher,
            "release_date" => &mut self.release_date,
            "brand" => &mut self.brand,
            "artists" => &mut self.artists,
            "voices" => &mut self.voices,
            "writers" => &mut self.writers,
            "music" => &mut self.music,
            "singers" => &mut self.singers,
            _ => return false,
        };
        value.clone_into(target);
        // Links supplied by the website no longer describe an edited field.
        self.links.retain(|link| link.field != field);
        true
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Resource {
    pub title: String,
    pub url: String,
    pub source: String,
    pub description: String,
    pub game_title: String,
    pub tags: Vec<String>,
    pub author: String,
    pub uploaded_at: String,
    pub size: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceSearch {
    pub resources: Vec<Resource>,
    pub warnings: Vec<Message>,
}

pub fn client() -> Result<Client, Message> {
    Client::builder()
        .user_agent("GalGear/0.1 (+local game library)")
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(network)
}
fn network(e: impl ToString) -> Message {
    Message::detail("error.network", e)
}
pub fn fan_base(domain: &str) -> Result<Url, Message> {
    let url = Url::parse(domain).map_err(network)?;
    if url.scheme() != "https"
        || ![
            "2dfan.com",
            "2dfmax.top",
            "2dfdf.de",
            "fan2d.top",
            "acgfan.top",
            "t.vndb.org",
            "s.vndb.org",
            "lain.bgm.tv",
            "store.ymgal.games",
        ]
        .contains(&url.host_str().unwrap_or_default())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Message::new("error.site_url"));
    }
    Ok(url)
}
fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static selector")
}
pub fn page(client: &Client, url: Url) -> Result<String, Message> {
    use std::io::Read;
    let response = client
        .get(url)
        .send()
        .map_err(network)?
        .error_for_status()
        .map_err(network)?;
    let mut bytes = Vec::new();
    response
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(network)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Message::new("error.response_size"));
    }
    String::from_utf8(bytes).map_err(network)
}

pub fn search_fan(domain: &str, query: &str) -> Result<Vec<Resource>, Message> {
    let base = fan_base(domain)?;
    let mut url = base.join("/search").map_err(network)?;
    url.query_pairs_mut().append_pair("keyword", query.trim());
    let body = page(&client()?, url)?;
    Ok(parse_links(&body, &base, "/subjects/", "2DFan"))
}
pub fn parse_links(body: &str, base: &Url, prefix: &str, source: &str) -> Vec<Resource> {
    let doc = Html::parse_document(body);
    let mut results = Vec::new();
    let selection = match prefix {
        "/subjects/" => ".subject-card-title a[href]",
        "/downloads/" => ".download-card-title a[href]",
        _ => "a[href]",
    };
    for a in doc.select(&selector(selection)) {
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        let Ok(url) = base.join(href) else {
            continue;
        };
        let tail = url.path().strip_prefix(prefix).unwrap_or_default();
        if url.host_str() != base.host_str()
            || tail.is_empty()
            || !tail.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        let title = a.text().collect::<Vec<_>>().join(" ").trim().to_owned();
        if title.is_empty() || results.iter().any(|r: &Resource| r.url == url.as_str()) {
            continue;
        }
        let card = a
            .ancestors()
            .filter_map(scraper::ElementRef::wrap)
            .find(|e| {
                e.value()
                    .attr("class")
                    .is_some_and(|s| s.split_whitespace().any(|c| c == "download-card"))
            });
        let text = |css: &str| {
            card.as_ref()
                .and_then(|e| e.select(&selector(css)).next())
                .map(|e| e.text().collect::<String>().trim().to_owned())
                .unwrap_or_default()
        };
        results.push(Resource {
            title,
            url: url.into(),
            source: source.into(),
            description: String::new(),
            tags: card
                .as_ref()
                .map(|e| {
                    e.select(&selector(".download-card-kind"))
                        .map(|tag| tag.text().collect::<String>().trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            author: text(".uploader-name"),
            uploaded_at: card
                .as_ref()
                .and_then(|e| e.select(&selector("time.download-card-date")).next())
                .and_then(|e| e.value().attr("datetime"))
                .map(str::to_owned)
                .unwrap_or_else(|| text(".download-card-date")),
            size: text(".download-card-size"),
            ..Default::default()
        });
    }
    results
}
pub fn fan_metadata(domain: &str, url: &str) -> Result<Metadata, Message> {
    let base = fan_base(domain)?;
    let url = base.join(url).map_err(network)?;
    if url.host_str() != base.host_str() || !url.path().starts_with("/subjects/") {
        return Err(Message::new("error.site_url"));
    }
    let body = page(&client()?, url.clone())?;
    parse_metadata(&body, &url)
}

/// Parse only this subject's story and metadata; SEO summaries are not stories.
pub fn parse_metadata(body: &str, url: &Url) -> Result<Metadata, Message> {
    let base = url;
    let doc = Html::parse_document(body);
    let content = |name: &str| {
        doc.select(&selector(name))
            .next()
            .and_then(|e| e.value().attr("content"))
            .unwrap_or_default()
            .to_owned()
    };
    let title = doc
        .select(&selector("h1"))
        .next()
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| content("meta[property='og:title']"));
    let mut cover = content("meta[property='og:image']");
    if cover.is_empty() {
        cover = doc
            .select(&selector("img[src]"))
            .filter_map(|e| e.value().attr("src"))
            .find(|s| s.contains("/subjects/") || s.contains("/covers/"))
            .unwrap_or_default()
            .into();
    }
    let description = doc
        .select(&selector(".intro-content, .subject-description"))
        .next()
        .map(story_text)
        .unwrap_or_default();
    let field = |labels: &[&str]| -> String {
        for row in doc.select(&selector(".subject-meta-list .meta-row")) {
            let label = row
                .select(&selector("dt"))
                .next()
                .map(|e| e.text().collect::<String>())
                .unwrap_or_default();
            if labels.contains(&label.trim().trim_end_matches([':', '：']).trim()) {
                return row
                    .select(&selector("dd"))
                    .next()
                    .map(metadata_text)
                    .unwrap_or_default();
            }
        }
        String::new()
    };
    if title.is_empty() {
        return Err(Message::new("error.metadata"));
    }
    let mut metadata = Metadata {
        url: url.to_string(),
        title,
        description,
        developer: field(&["开发商"]),
        publisher: field(&["发行商", "发行", "发行品牌"]),
        brand: field(&["品牌"]),
        artists: field(&["原画"]),
        voices: field(&["声优", "配音"]),
        writers: field(&["剧本", "脚本", "剧本／脚本", "剧本/脚本"]),
        music: field(&["音乐"]),
        singers: field(&["歌手"]),
        release_date: field(&["发售日期", "发行日期"]),
        cover_url: if cover.is_empty() {
            None
        } else {
            base.join(&cover).ok().map(Into::into)
        },
        tags: crate::tags::normalize(
            doc.select(&selector(".subject-tags-inline > a.nexus-tag"))
                .map(|e| e.text().collect::<String>()),
        ),
        links: Vec::new(),
        schema_version: METADATA_SCHEMA_VERSION,
    };
    for row in doc.select(&selector(".subject-meta-list .meta-row")) {
        let label = row
            .select(&selector("dt"))
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let field = match label.trim().trim_end_matches([':', '：']).trim() {
            "品牌" => "brand",
            "原画" => "artists",
            "声优" | "配音" => "voices",
            "剧本" | "脚本" | "剧本／脚本" | "剧本/脚本" => "writers",
            "音乐" => "music",
            "歌手" => "singers",
            "开发商" => "developer",
            "发行商" | "发行" | "发行品牌" => "publisher",
            _ => continue,
        };
        for a in row.select(&selector("dd a.nexus-inline-link[href]")) {
            if let Some(href) = a.value().attr("href")
                && let Ok(link) = base.join(href)
                && matches!(link.scheme(), "http" | "https")
            {
                metadata.links.push(MetadataLink {
                    field: field.into(),
                    name: a.text().collect::<String>().trim().into(),
                    url: link.into(),
                });
            }
        }
    }
    Ok(metadata)
}

fn metadata_text(element: scraper::ElementRef<'_>) -> String {
    let mut output = String::new();
    let separator = |c: char| {
        matches!(
            c,
            '、' | '，' | ',' | '；' | ';' | '/' | '／' | '・' | ':' | '：'
        )
    };
    for child in element.children() {
        let text = if let scraper::Node::Text(text) = child.value() {
            text.to_string()
        } else if let Some(child) = scraper::ElementRef::wrap(child) {
            child.text().collect::<String>()
        } else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if !output.is_empty()
            && !text.chars().next().is_some_and(separator)
            && !output.chars().last().is_some_and(separator)
        {
            output.push(' ');
        }
        output.push_str(text);
    }
    output
}

fn story_text(element: scraper::ElementRef<'_>) -> String {
    fn append(element: scraper::ElementRef<'_>, text: &mut String) {
        for child in element.children() {
            if let scraper::Node::Text(value) = child.value() {
                text.push_str(value);
            } else if let Some(child) = scraper::ElementRef::wrap(child) {
                let block = matches!(child.value().name(), "p" | "div" | "br");
                if block {
                    text.push('\n');
                }
                append(child, text);
                if block {
                    text.push('\n');
                }
            }
        }
    }
    let mut text = String::new();
    append(element, &mut text);
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn search_fan_patches(domain: &str, query: &str) -> Result<Vec<Resource>, Message> {
    let base = fan_base(domain)?;
    let mut url = base.join("/search").map_err(network)?;
    url.query_pairs_mut().append_pair("keyword", query.trim());
    let body = page(&client()?, url)?;
    let mut results = Vec::new();
    // Search pages identify games; patches are fetched from that game's downloads page.
    for subject in parse_links(&body, &base, "/subjects/", "2DFan")
        .into_iter()
        .take(5)
    {
        let url = Url::parse(&format!("{}/downloads", subject.url)).map_err(network)?;
        match page(&client()?, url) {
            Ok(body) => {
                let mut patches = parse_links(&body, &base, "/downloads/", "2DFan");
                for p in &mut patches {
                    p.game_title = subject.title.clone();
                }
                results.extend(patches);
            }
            Err(_) => results.push(Resource {
                title: subject.title,
                url: format!("{}/downloads", subject.url),
                source: "2DFan".into(),
                description: String::new(),
                ..Default::default()
            }),
        }
    }
    Ok(results)
}

pub fn search_patch_sites(domain: &str, query: &str) -> Result<ResourceSearch, Message> {
    if query.trim().is_empty() {
        return Err(Message::new("error.title_required"));
    }
    let (fan, kun) = std::thread::scope(|scope| {
        let fan = scope.spawn(|| search_fan_patches(domain, query));
        let kun = search_kun(query);
        (
            fan.join()
                .unwrap_or_else(|_| Err(network("2DFan request worker failed"))),
            kun,
        )
    });
    let mut output = ResourceSearch::default();
    for (source, result) in [("2DFan", fan), ("鲲 Gal", kun)] {
        match result {
            Ok(mut resources) => output.resources.append(&mut resources),
            Err(_) => output
                .warnings
                .push(Message::new("patches.source_error").arg("source", source)),
        }
    }
    Ok(output)
}

pub fn search_kun(query: &str) -> Result<Vec<Resource>, Message> {
    let mut url = Url::parse("https://www.moyu.moe/api/v1/search/quick").map_err(network)?;
    url.query_pairs_mut()
        .append_pair("keywords", query.trim())
        .append_pair("content_limit", "sfw");
    read_kun(&client()?, url)
}
fn read_kun(client: &Client, url: Url) -> Result<Vec<Resource>, Message> {
    use std::io::Read;
    // Public GET only. Follow the system network configuration; retry a transient failure once.
    for attempt in 0..2 {
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::REFERER, "https://www.moyu.moe/")
            .send();
        let response = match response {
            Err(e) if attempt == 0 && (e.is_connect() || e.is_timeout()) => continue,
            result => result.map_err(network)?,
        };
        if attempt == 0 && [429, 502, 503, 504].contains(&response.status().as_u16()) {
            continue;
        }
        let response = response.error_for_status().map_err(network)?;
        let mut bytes = Vec::new();
        if let Err(e) = response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes) {
            if attempt == 0 {
                continue;
            }
            return Err(network(e));
        }
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Message::new("error.response_size"));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(network)?;
        return validate_kun(&value);
    }
    Err(Message::new("patches.source_error").arg("source", "鲲 Gal"))
}
fn validate_kun(value: &serde_json::Value) -> Result<Vec<Resource>, Message> {
    if !value["data"]["resources"].is_array() {
        return Err(Message::new("patches.source_error").arg("source", "鲲 Gal"));
    }
    Ok(parse_kun(value))
}
fn parse_kun(value: &serde_json::Value) -> Vec<Resource> {
    let mut results = Vec::new();
    if let Some(items) = value["data"]["resources"].as_array() {
        for item in items {
            let Some(id) = item["id"].as_u64() else {
                continue;
            };
            let title = item["name"].as_str().unwrap_or_default().to_owned();
            let game = item["patch"]["name"]["display_name"]
                .as_str()
                .unwrap_or_default();
            let tags: Vec<String> = ["type", "language", "platform"]
                .iter()
                .flat_map(|key| {
                    item[*key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str())
                })
                .map(str::to_owned)
                .collect();
            results.push(Resource {
                title,
                url: format!("https://www.moyu.moe/resource/{id}"),
                source: "鲲 Gal".into(),
                game_title: game.into(),
                tags,
                author: item["user"]["name"].as_str().unwrap_or_default().into(),
                uploaded_at: item["created"].as_str().unwrap_or_default().into(),
                size: item["size"].as_str().unwrap_or_default().into(),
                ..Default::default()
            });
        }
    }
    results
}

/// Download and normalize a source image with the same limits as local posters.
pub fn cover_bytes(image_url: &str, referrer: &str) -> Result<Vec<u8>, Message> {
    use std::io::Read;
    let url = Url::parse(image_url).map_err(network)?;
    if url.scheme() != "https"
        || ![
            "img.achost.top",
            "2dfmax.top",
            "2dfdf.de",
            "2dfan.com",
            "fan2d.top",
            "acgfan.top",
            "t.vndb.org",
            "s.vndb.org",
            "lain.bgm.tv",
            "store.ymgal.games",
            "image.kungal.iloveren.link",
        ]
        .contains(&url.host_str().unwrap_or_default())
    {
        return Err(Message::new("error.site_url"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Message::new("error.site_url"));
    }
    let response = Client::builder()
        .user_agent("GalGear/0.1 (+local game library)")
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(network)?
        .get(url)
        .header(reqwest::header::REFERER, referrer)
        .send()
        .map_err(network)?
        .error_for_status()
        .map_err(network)?;
    let mut bytes = Vec::new();
    response
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(network)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Message::new("error.image_size"));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(network)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20000);
    limits.max_image_height = Some(20000);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(network)?.thumbnail(540, 720);
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Jpeg)
        .map_err(network)?;
    Ok(output.into_inner())
}

pub fn save_cover(
    game: &crate::Game,
    metadata: &Metadata,
    cache: &std::path::Path,
) -> Result<(String, String, Option<Message>), Message> {
    use std::io::Write;
    let bytes = cover_bytes(
        metadata
            .cover_url
            .as_deref()
            .ok_or_else(|| Message::new("error.metadata"))?,
        &metadata.url,
    )?;
    let local = std::path::PathBuf::from(&game.folder).join("cover.jpg");
    let write = |path: &std::path::Path| -> Result<(), Message> {
        let parent = path
            .parent()
            .ok_or_else(|| Message::new("error.cover_path"))?;
        std::fs::create_dir_all(parent).map_err(network)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(network)?;
        temp.write_all(&bytes).map_err(network)?;
        temp.as_file().sync_all().map_err(network)?;
        temp.persist_noclobber(path).map_err(network)?;
        Ok(())
    };
    if local.is_file() {
        return Ok((
            crate::scanner::poster_data(&local)?,
            crate::config::display_path(&local),
            None,
        ));
    }
    let (destination, warning) = match write(&local) {
        Ok(()) => (local, None),
        Err(e) => {
            std::fs::create_dir_all(cache).map_err(network)?;
            let fallback = cache.join(format!(
                "{}-{:x}.jpg",
                game.id,
                sha2::Sha256::digest(&bytes)
            ));
            if !fallback.exists() {
                write(&fallback)?;
            }
            (fallback, Some(Message::detail("cover.fallback", e)))
        }
    };
    Ok((
        crate::scanner::poster_data(&destination)?,
        crate::config::display_path(&destination),
        warning,
    ))
}
pub fn fan_patches(domain: &str, subject_url: &str) -> Result<Vec<Resource>, Message> {
    let base = fan_base(domain)?;
    let url = base.join(subject_url).map_err(network)?;
    if url.host_str() != base.host_str() || !url.path().starts_with("/subjects/") {
        return Err(Message::new("error.site_url"));
    }
    let body = page(&client()?, url)?;
    Ok(parse_links(&body, &base, "/downloads/", "2DFan"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kun_error_envelope_is_not_an_empty_success_and_transient_get_retries_once() {
        use std::io::{Read, Write};
        assert!(validate_kun(&serde_json::json!({"code":429,"message":"retry later"})).is_err());
        assert!(
            validate_kun(&serde_json::json!({"data":{"resources":[]}}))
                .unwrap()
                .is_empty()
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/search", listener.local_addr().unwrap())).unwrap();
        let server = std::thread::spawn(move || {
            for (status, body) in [
                ("503 Service Unavailable", "temporary"),
                (
                    "200 OK",
                    r#"{"data":{"resources":[{"id":1,"name":"Recovered"}]}}"#,
                ),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..n]);
                assert!(request.starts_with("GET /search "));
                assert!(request.to_lowercase().contains("accept: application/json"));
                write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert_eq!(read_kun(&client, url).unwrap()[0].title, "Recovered");
        server.join().unwrap();
    }
    #[test]
    fn resource_metadata_stays_with_its_own_card() {
        let body = r#"<div class='download-card'><h4 class='download-card-title'><a href='/downloads/1'>AI patch</a></h4><span class='download-card-kind'>AI翻译</span><span class='uploader-name'>Author A</span><time class='download-card-date' datetime='2024-01-06T12:00:00+08:00'>2024-01-06</time><span class='download-card-size'>3.4 MB</span></div><div class='download-card'><h4 class='download-card-title'><a href='/downloads/2'>CG save</a></h4><span class='download-card-kind'>全CG存档</span><span class='uploader-name'>Author B</span><time class='download-card-date'>2023-12-22</time></div>"#;
        let resources = parse_links(
            body,
            &fan_base("https://2dfmax.top").unwrap(),
            "/downloads/",
            "2DFan",
        );
        assert_eq!(resources[0].tags, vec!["AI翻译"]);
        assert_eq!(resources[0].author, "Author A");
        assert_eq!(resources[0].size, "3.4 MB");
        assert_eq!(resources[1].author, "Author B");
        assert_eq!(resources[1].uploaded_at, "2023-12-22");
        assert!(resources[1].size.is_empty());
    }
    #[test]
    fn kun_resource_keeps_game_author_created_time_and_tags() {
        let resources = parse_kun(
            &serde_json::json!({"data":{"resources":[{"id":9,"name":"Patch","patch":{"name":{"display_name":"Game"}},"type":["ai"],"language":["zh-Hans"],"size":"22.6MB","user":{"name":"Author"},"created":"2024-01-06T10:00:00Z"}]}}),
        );
        assert_eq!(resources[0].game_title, "Game");
        assert_eq!(resources[0].author, "Author");
        assert_eq!(resources[0].uploaded_at, "2024-01-06T10:00:00Z");
        assert_eq!(resources[0].tags, vec!["ai", "zh-Hans"]);
    }
    #[test]
    fn adjacent_people_links_remain_separate_from_subject_tags_and_release_fields() {
        let body = r#"<h1>Subject</h1><dl class='subject-meta-list'><div class='meta-row'><dt>声优：</dt><dd><a class='nexus-inline-link' href='/tags/one'>One</a><a class='nexus-inline-link' href='/tags/two'>Two</a><a class='nexus-inline-link' href='/tags/three'>Three</a></dd></div><div class='meta-row'><dt>剧本</dt><dd><a class='nexus-inline-link' href='/tags/writer1'>Writer One</a><a class='nexus-inline-link' href='/tags/writer2'>Writer Two</a></dd></div><div class='meta-row'><dt>发行</dt><dd><a class='nexus-inline-link' href='/tags/publisher'>Publisher</a></dd></div><div class='meta-row'><dt>发售日期</dt><dd>2014-01-31</dd></div></dl><div class='subject-tags-inline'><a class='nexus-tag' href='/tags/school'>学园</a><a class='nexus-tag' href='/tags/adv'>ADV</a></div>"#;
        let metadata = parse_metadata(
            body,
            &Url::parse("https://2dfmax.top/subjects/2111").unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.voices, "One Two Three");
        assert_eq!(metadata.writers, "Writer One Writer Two");
        assert_eq!(metadata.publisher, "Publisher");
        assert_eq!(metadata.release_date, "2014-01-31");
        assert_eq!(metadata.tags, vec!["学园", "ADV"]);
        assert_eq!(
            metadata
                .links
                .iter()
                .filter(|link| link.field == "voices")
                .count(),
            3
        );
    }
    #[test]
    fn metadata_keeps_paragraphs_people_links_and_only_subject_tags() {
        let body = r#"<h1>Title</h1><meta name='description' content='SEO summary'><div class='intro-content'><p>First</p><p>Second<br>Third</p></div><dl class='subject-meta-list'><div class='meta-row'><dt>品牌</dt><dd><a class='nexus-inline-link' href='/tags/1'>Brand</a></dd></div><div class='meta-row'><dt>声优</dt><dd><a class='nexus-inline-link' href='/tags/2'>One</a>、<a class='nexus-inline-link' href='/tags/3'>Two</a></dd></div></dl><div class='subject-tags-inline'><a class='nexus-tag'> School </a><a class='nexus-tag'>school</a></div><a class='nexus-tag'>Unrelated</a>"#;
        let metadata =
            parse_metadata(body, &Url::parse("https://2dfmax.top/subjects/1").unwrap()).unwrap();
        assert_eq!(metadata.description, "First\n\nSecond\n\nThird");
        assert_eq!(metadata.developer, "");
        assert_eq!(metadata.brand, "Brand");
        assert_eq!(metadata.voices, "One、Two");
        assert_eq!(metadata.links.len(), 3);
        assert_eq!(metadata.tags, vec!["School"]);
    }
    #[test]
    fn missing_story_does_not_use_seo_and_empty_manual_field_is_protected() {
        let mut metadata = parse_metadata(
            "<h1>Title</h1><meta name='description' content='SEO summary'>",
            &Url::parse("https://2dfmax.top/subjects/1").unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.description, "");
        metadata.brand = "Automatic".into();
        assert!(metadata.set_field("brand", ""));
        assert_eq!(metadata.brand, "");
    }
    #[test]
    fn site_links_are_deduplicated_and_external_links_are_excluded() {
        let base = fan_base("https://2dfmax.top").unwrap();
        let results = parse_links(
            r#"<h3 class='subject-card-title'><a href='/subjects/42'>游戏 A</a><a href='/subjects/42'>重复</a><a href='https://evil.test/subjects/9'>Bad</a><a href='/subjects/search'>Search</a></h3><aside><a href='/subjects/99'>Unrelated recommendation</a></aside>"#,
            &base,
            "/subjects/",
            "2DFan",
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "游戏 A");
        assert!(fan_base("https://2dfmax.top.evil.test").is_err());
    }
    #[test]
    fn download_category_badges_cannot_replace_resource_titles() {
        let base = fan_base("https://2dfmax.top").unwrap();
        let body = r#"<a href='/downloads/7'>汉化</a><h3 class='download-card-title'><a href='/downloads/7'>测试汉化补丁 v1.1</a></h3><aside><a href='/downloads/9'>Other</a></aside>"#;
        let resources = parse_links(body, &base, "/downloads/", "2DFan");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].title, "测试汉化补丁 v1.1");
        assert!(
            parse_links(
                "<a href='/downloads/7'>汉化</a>",
                &base,
                "/downloads/",
                "2DFan"
            )
            .is_empty()
        );
    }
}
