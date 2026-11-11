//! 批評空間公开 SQL 表单的只读适配；只发送固定查询，不接受外部 SQL。
use crate::providers::{METADATA_SCHEMA_VERSION, Metadata, MetadataLink, Resource};
use galgear_i18n::Message;
use reqwest::{Url, blocking::Client};
use scraper::{Html, Selector};
use std::{collections::BTreeMap, fmt::Write, io::Read};

const BASE: &str = "https://erogamescape.dyndns.org/~ap2/ero/toukei_kaiseki";
const FIELDS: &str =
    "g.id, g.gamename, g.sellday, b.brandname, g.genre, g.shoukai, b.id AS brand_id";
const FROM: &str = "FROM gamelist g LEFT JOIN brandlist b ON g.brandname = b.id";
type Row = BTreeMap<String, String>;

fn error(detail: impl std::fmt::Display) -> Message {
    Message::detail("sources.erogamescape_error", detail)
}

pub(super) fn url_id(url: &Url) -> Result<String, Message> {
    let pairs: Vec<_> = url.query_pairs().collect();
    if url.path() != "/~ap2/ero/toukei_kaiseki/game.php"
        || pairs.len() != 1
        || pairs[0].0 != "game"
        || pairs[0].1.is_empty()
        || !pairs[0].1.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Message::new("error.site_url"));
    }
    let id: u32 = pairs[0]
        .1
        .parse()
        .map_err(|_| Message::new("error.site_url"))?;
    if id == 0 {
        return Err(Message::new("error.site_url"));
    }
    Ok(id.to_string())
}

fn game_url(id: &str) -> String {
    format!("{BASE}/game.php?game={id}")
}

fn search_sql(query: &str) -> String {
    let escaped = query
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    let mut hex = String::with_capacity(escaped.len() * 2);
    for byte in escaped.bytes() {
        let _ = write!(hex, "{byte:02x}");
    }
    // 2026 年站点更新后对非 ASCII SQL 的转义不兼容。以 UTF-8 十六进制
    // 构造文字，既保留日文，也避免引号和 LIKE 通配符改变查询含义。
    format!(
        "SELECT {FIELDS} {FROM} WHERE g.gamename ILIKE '%' || convert_from(decode('{hex}','hex'),'UTF8') || '%' ESCAPE '!' ORDER BY g.count2 DESC NULLS LAST, g.id LIMIT 30"
    )
}

fn query(client: &Client, sql: &str) -> Result<Vec<Row>, Message> {
    let response = client
        .post(format!("{BASE}/sql_for_erogamer_form.php"))
        .form(&[("sql", sql)])
        .send()
        .map_err(error)?
        .error_for_status()
        .map_err(error)?;
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(error("Response too large"));
    }
    parse_table(std::str::from_utf8(&bytes).map_err(error)?)
}

fn parse_table(html: &str) -> Result<Vec<Row>, Message> {
    let document = Html::parse_document(html);
    let table_selector = Selector::parse("table").expect("static selector");
    let row_selector = Selector::parse("tr").expect("static selector");
    let cell_selector = Selector::parse("th, td").expect("static selector");
    for table in document.select(&table_selector) {
        let mut rows = table.select(&row_selector);
        let Some(header) = rows.next() else {
            continue;
        };
        let headers: Vec<_> = header
            .select(&cell_selector)
            .map(|cell| cell.text().collect::<String>().trim().to_owned())
            .collect();
        if ![
            "id",
            "gamename",
            "brandname",
            "sellday",
            "genre",
            "shoukai",
            "brand_id",
        ]
        .iter()
        .all(|key| headers.iter().any(|h| h == key))
        {
            continue;
        }
        return rows
            .map(|row| {
                let values: Vec<_> = row
                    .select(&cell_selector)
                    .map(|cell| cell.text().collect::<String>().trim().to_owned())
                    .collect();
                if values.len() != headers.len() {
                    return Err(error("Invalid result row"));
                }
                Ok(headers.iter().cloned().zip(values).collect())
            })
            .collect();
    }
    // 站点错误页与布局变更必须可见，不能作为“没有搜索结果”。
    Err(error("Missing SQL result table"))
}

fn metadata_row(row: &Row) -> Result<Metadata, Message> {
    let get = |key: &str| row.get(key).map(String::as_str).unwrap_or_default();
    let id: u32 = get("id").parse().map_err(|_| error("Invalid game ID"))?;
    if id == 0 || get("gamename").is_empty() {
        return Err(error("Missing game identity"));
    }
    let brand = get("brandname");
    let mut metadata = Metadata {
        url: game_url(&id.to_string()),
        title: get("gamename").into(),
        brand: brand.into(),
        developer: brand.into(),
        publisher: brand.into(),
        release_date: get("sellday").into(),
        // shoukai 也可能是站外介绍链接，不把链接当剧情，也不自动访问未知域名。
        description: if get("shoukai").starts_with("http") {
            String::new()
        } else {
            get("shoukai").into()
        },
        tags: crate::tags::normalize([get("genre").to_owned()]),
        schema_version: METADATA_SCHEMA_VERSION,
        ..Default::default()
    };
    if let Ok(brand_id) = get("brand_id").parse::<u32>()
        && brand_id > 0
        && !brand.is_empty()
    {
        metadata.links.push(MetadataLink {
            field: "brand".into(),
            name: brand.into(),
            url: format!("{BASE}/brand.php?brand={brand_id}"),
        });
    }
    Ok(metadata)
}

pub(super) fn search(client: &Client, title: &str) -> Result<Vec<Resource>, Message> {
    query(client, &search_sql(title))?
        .iter()
        .map(|row| {
            let metadata = metadata_row(row)?;
            Ok(Resource {
                title: metadata.title,
                url: metadata.url,
                source: "ErogameScape".into(),
                brand: metadata.brand,
                release_date: metadata.release_date,
                ..Default::default()
            })
        })
        .collect()
}

pub(super) fn metadata(client: &Client, id: &str) -> Result<Metadata, Message> {
    let id: u32 = id.parse().map_err(|_| Message::new("error.site_url"))?;
    let rows = query(
        client,
        &format!("SELECT {FIELDS} {FROM} WHERE g.id = {id} LIMIT 1"),
    )?;
    let metadata = metadata_row(rows.first().ok_or_else(|| error("Game not found"))?)?;
    if metadata.url != game_url(&id.to_string()) {
        return Err(error("Unexpected game identity"));
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::{metadata_row, parse_table, search_sql};
    const HEADER: &str = "<tr><th>id</th><th>gamename</th><th>sellday</th><th>brandname</th><th>genre</th><th>shoukai</th><th>brand_id</th></tr>";

    #[test]
    fn only_canonical_game_urls_are_accepted() {
        for host in ["erogamescape.org", "erogamescape.dyndns.org"] {
            let url = format!("https://{host}/~ap2/ero/toukei_kaiseki/game.php?game=123");
            assert_eq!(
                crate::data_sources::source_for_url(&url).unwrap(),
                ("erogamescape", "123".into())
            );
            for suffix in [
                "",
                "game=0",
                "game=1&game=2",
                "game=1&sql=SELECT",
                "game=abc",
                "game=1%20OR%201=1",
            ] {
                let url = format!("https://{host}/~ap2/ero/toukei_kaiseki/game.php?{suffix}");
                assert!(crate::data_sources::source_for_url(&url).is_err(), "{url}");
            }
        }
        assert!(
            crate::data_sources::source_for_url("https://erogamescape.org/other?game=1").is_err()
        );
        assert!(
            crate::data_sources::source_for_url(
                "https://erogamescape.org:8443/~ap2/ero/toukei_kaiseki/game.php?game=1"
            )
            .is_err()
        );
    }

    #[test]
    fn sql_encodes_unicode_quotes_and_literal_wildcards() {
        let sql = search_sql("夏'_%!");
        assert!(sql.is_ascii());
        assert!(sql.contains("e5a48f27215f21252121"));
        assert!(sql.ends_with("LIMIT 30"));
    }

    #[test]
    fn table_decodes_text_and_metadata_without_treating_errors_as_empty_results() {
        let html = format!(
            "<table><tr><td>navigation</td></tr></table><table>{HEADER}<tr><td>123</td><td>夏 &amp; 空</td><td>2024-01-26</td><td>制作社</td><td>ADV</td><td>物語の紹介</td><td>12</td></tr></table>"
        );
        let rows = parse_table(&html).unwrap();
        let metadata = metadata_row(&rows[0]).unwrap();
        assert_eq!(metadata.title, "夏 & 空");
        assert_eq!(metadata.brand, "制作社");
        assert_eq!(metadata.release_date, "2024-01-26");
        assert_eq!(metadata.tags, ["ADV"]);
        assert_eq!(metadata.description, "物語の紹介");
        assert!(metadata.url.ends_with("game.php?game=123"));
        assert!(
            parse_table(&format!("<table>{HEADER}</table>"))
                .unwrap()
                .is_empty()
        );
        let error = parse_table("<html>SQL Error</html>").unwrap_err();
        assert_eq!(
            error.render(galgear_i18n::Language::Chinese),
            "数据源请求失败：批评空间：Missing SQL result table"
        );
        assert!(parse_table(&format!("<table>{HEADER}<tr><td>123</td></tr></table>")).is_err());
    }
}
