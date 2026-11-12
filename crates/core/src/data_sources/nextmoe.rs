//! The v2 catalog is public; user-supplied credentials only change its quota.
use super::{Cover, Credentials, add_cover, add_credit, api_error, auth, request_json, text};
use crate::providers::{self, Metadata, Resource};
use galgear_i18n::Message;
use reqwest::blocking::Client;
use serde_json::Value;

const API: &str = "https://api.nextmoe.dev/v2/catalog/works";

fn work_url(id: &str) -> String {
    format!("https://developer.nextmoe.dev/explore/work/{id}?nsfw=1")
}

fn is_game(work: &Value) -> bool {
    matches!(work["medium"].as_str(), Some("galgame" | "doujin_game"))
}

fn work_id(work: &Value) -> Result<&str, Message> {
    work["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| api_error("Invalid NextMoe work ID"))
}

fn localized_name(value: &Value) -> String {
    for lang in ["zh-Hans", "zh-CN", "zh", "zh-Hant", "zh-TW"] {
        if let Some(name) = value["localized"][lang]["value"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
        {
            return name.into();
        }
    }
    text(value, "display_name")
}

fn description(work: &Value) -> String {
    let intros: Vec<_> = work["intros"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|intro| {
            intro["value"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
        })
        .collect();
    // The API returns every language, so explicitly choose rather than trusting array order.
    for lang in ["zh-Hans", "zh-CN", "zh", "zh-Hant", "zh-TW", "ja", "en"] {
        if let Some(intro) = intros
            .iter()
            .filter(|intro| intro["lang"] == lang)
            .min_by_key(|intro| intro["is_machine"].as_bool().unwrap_or(false))
        {
            return text(intro, "value");
        }
    }
    intros
        .first()
        .map_or_else(String::new, |intro| text(intro, "value"))
}

fn parse_search(value: &Value) -> Result<Vec<Resource>, Message> {
    if value["object"] != "list" {
        return Err(api_error("Invalid NextMoe search response"));
    }
    value["items"]
        .as_array()
        .ok_or_else(|| api_error("Missing NextMoe search results"))?
        .iter()
        .filter(|work| is_game(work))
        .map(|work| {
            let id = work_id(work)?;
            let title = localized_name(work);
            if work["object"] != "work" || title.trim().is_empty() {
                return Err(api_error("Invalid NextMoe work"));
            }
            let preview = parse_metadata(work, &work_url(id))?;
            Ok(Resource {
                aliases: names(work),
                cover_url: preview.cover_url,
                brand: preview.brand,
                release_date: preview.release_date,
                title,
                url: work_url(id),
                source: "nextmoe".into(),
                ..Default::default()
            })
        })
        .collect()
}

pub(super) fn check(client: &Client, credentials: &Credentials) -> Result<(), Message> {
    let value = request_json(auth(client.get(API), "nextmoe", credentials).query(&[
        ("limit", "1"),
        ("fields", "id"),
        ("nsfw", "true"),
    ]))?;
    if value["object"] != "list" || !value["items"].is_array() {
        return Err(api_error("Invalid NextMoe catalog response"));
    }
    Ok(())
}

pub(super) fn search(
    client: &Client,
    query: &str,
    credentials: &Credentials,
) -> Result<Vec<Resource>, Message> {
    let value = request_json(auth(client.get(API), "nextmoe", credentials).query(&[
        ("q", query),
        ("limit", "20"),
        ("nsfw", "true"),
        ("include", "titles,companies"),
    ]))?;
    parse_search(&value)
}

fn names(work: &Value) -> Vec<String> {
    let mut names = vec![text(work, "display_name"), text(work, "latin")];
    if let Some(localized) = work["localized"].as_object() {
        names.extend(localized.values().map(|name| text(name, "value")));
    }
    for title in work["titles"].as_array().into_iter().flatten() {
        names.push(text(title, "title"));
        names.push(text(title, "latin"));
    }
    galgear_tags::normalize(names)
}

fn matching_work<'a>(value: &'a Value, query: &str) -> Result<Option<&'a Value>, Message> {
    // Validate shape even when no candidate matches: malformed responses must not be cached as success.
    parse_search(value)?;
    let key = galgear_tags::key(query);
    let mut matches = value["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|work| {
            is_game(work)
                && names(work)
                    .iter()
                    .any(|name| galgear_tags::key(name) == key)
        });
    let first = matches.next();
    Ok(if matches.next().is_none() {
        first
    } else {
        None
    })
}

pub(super) fn aliases(
    client: &Client,
    query: &str,
    id: Option<&str>,
    credentials: &Credentials,
) -> Result<Vec<String>, Message> {
    let id = if let Some(id) = id {
        id.to_owned()
    } else {
        let results = request_json(auth(client.get(API), "nextmoe", credentials).query(&[
            ("q", query),
            ("limit", "20"),
            ("nsfw", "true"),
            ("include", "titles,companies"),
        ]))?;
        let Some(work) = matching_work(&results, query)? else {
            return Ok(Vec::new());
        };
        work_id(work)?.to_owned()
    };
    let work = request_json(
        auth(client.get(format!("{API}/{id}")), "nextmoe", credentials)
            .query(&[("nsfw", "true"), ("include", "titles")]),
    )?;
    if work["object"] != "work" || !is_game(&work) || work_id(&work)? != id {
        return Err(api_error("Invalid NextMoe alias response"));
    }
    Ok(names(&work))
}

fn credit_role(group: &Value) -> &str {
    match group["role_key"].as_str().unwrap_or_default() {
        "developer" => "开发",
        "publisher" => "发行",
        "character-design" | "illustration" | "original-art" => "原画",
        "scenario" => "scenario",
        "composer" | "arrange" | "music" | "theme-song-composition" => "music",
        "voice-actor" => "声优",
        "vocal" | "theme-song-performance" => "songs",
        _ => group["role_name"].as_str().unwrap_or_default(),
    }
}

fn parse_metadata(work: &Value, url: &str) -> Result<Metadata, Message> {
    work_id(work)?;
    let title = localized_name(work);
    if work["object"] != "work" || !is_game(work) || title.trim().is_empty() {
        return Err(Message::new("error.metadata"));
    }
    let mut meta = Metadata {
        url: url.into(),
        title,
        description: description(work),
        release_date: {
            let date = work["release_date"].as_str().unwrap_or_default();
            match work["release_date_precision"].as_str() {
                Some("year") => date.get(..4).unwrap_or_default(),
                Some("month") => date.get(..7).unwrap_or_default(),
                _ => date,
            }
            .into()
        },
        cover_url: work["cover"]["url"]
            .as_str()
            .filter(|url| !url.is_empty())
            .map(str::to_owned),
        schema_version: providers::METADATA_SCHEMA_VERSION,
        ..Default::default()
    };
    let mut brands = Vec::new();
    for company in work["companies"].as_array().into_iter().flatten() {
        let name = localized_name(company);
        let role = match company["attribution_role"].as_str() {
            Some("developer") => "开发",
            Some("publisher") => "发行",
            Some("brand" | "circle") => {
                if !name.is_empty() && !brands.contains(&name) {
                    brands.push(name);
                }
                continue;
            }
            _ => continue,
        };
        add_credit(&mut meta, role, &name, "");
    }
    let has_developer_company = !meta.developer.is_empty();
    let has_publisher_company = !meta.publisher.is_empty();
    for group in work["credits"].as_array().into_iter().flatten() {
        let role = credit_role(group);
        // Company attributions already carry elected localized names; credit-name aliases
        // may refer to the same company without the corresponding translation.
        let use_company_name =
            (role == "开发" && has_developer_company) || (role == "发行" && has_publisher_company);
        for credit in group["credits"].as_array().into_iter().flatten() {
            let name = localized_name(credit);
            if !name.is_empty() && !use_company_name {
                add_credit(&mut meta, role, &name, "");
            }
        }
    }
    for tag in work["tags"].as_array().into_iter().flatten() {
        // Also guard locally in case a future response includes a higher spoiler tier.
        if tag["spoiler"]
            .as_str()
            .is_some_and(|spoiler| spoiler != "none")
        {
            continue;
        }
        let name = localized_name(tag);
        if !name.is_empty() {
            meta.tags.push(name);
        }
    }
    meta.tags = crate::tags::normalize(meta.tags);
    if brands.is_empty() {
        meta.brand.clone_from(&meta.developer);
    } else {
        meta.brand = brands.join("、");
        if meta.developer.is_empty() {
            meta.developer.clone_from(&meta.brand);
        }
    }
    Ok(meta)
}

pub(super) fn metadata(
    client: &Client,
    id: &str,
    url: &str,
    credentials: &Credentials,
) -> Result<Metadata, Message> {
    // Request only the blocks used by the library; credits are not part of view=full.
    let work = request_json(
        auth(client.get(format!("{API}/{id}")), "nextmoe", credentials).query(&[
            ("nsfw", "true"),
            ("include", "titles,intros,companies,tags,credits"),
            ("spoiler", "none"),
        ]),
    )?;
    parse_metadata(&work, url)
}

fn parse_covers(work: &Value, url: &str) -> Result<Vec<Cover>, Message> {
    work_id(work)?;
    if work["object"] != "work" || !is_game(work) {
        return Err(Message::new("error.metadata"));
    }
    let title = localized_name(work);
    let mut covers = Vec::new();
    add_cover(&mut covers, &title, url, &work["cover"]);
    for image in work["covers"].as_array().into_iter().flatten() {
        // Covers also contain discs, booklet pages and packaging backs.
        if matches!(
            image["cover_kind"].as_str(),
            Some("main" | "pkgfront" | "dig")
        ) {
            add_cover(&mut covers, &title, url, image);
        }
    }
    Ok(covers)
}

pub(super) fn covers(
    client: &Client,
    id: &str,
    url: &str,
    credentials: &Credentials,
) -> Result<Vec<Cover>, Message> {
    let work = request_json(
        auth(client.get(format!("{API}/{id}")), "nextmoe", credentials)
            .query(&[("nsfw", "true"), ("include", "titles,covers")]),
    )?;
    parse_covers(&work, url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aliases_match_unique_multilingual_work_and_reject_sequels() {
        let work = serde_json::json!({"object":"work","id":"41","medium":"galgame","display_name":"終のステラ",
            "localized":{"en":{"value":"Stella of The End"},"zh-Hans":{"value":"星之终途"}},
            "titles":[{"title":"Tsui no Stella","latin":null},{"title":"星之终途"}]});
        let sequel = serde_json::json!({"object":"work","id":"440","medium":"galgame","display_name":"Stella of The End: Diary"});
        let result = serde_json::json!({"object":"list","items":[sequel,work.clone()]});
        let found = matching_work(&result, "stella of the end")
            .unwrap()
            .unwrap();
        assert_eq!(work_id(found).unwrap(), "41");
        assert!(names(found).contains(&"星之终途".into()));
        assert!(names(found).contains(&"Tsui no Stella".into()));
        assert!(matching_work(&result, "Stella").unwrap().is_none());
        let ambiguous = serde_json::json!({"object":"list","items":[work.clone(),work]});
        assert!(matching_work(&ambiguous, "星之终途").unwrap().is_none());
        assert!(matching_work(&serde_json::json!({}), "Game").is_err());
    }

    #[test]
    fn search_uses_localized_names_string_ids_and_only_games() {
        let results = parse_search(&json!({"object":"list","items":[
            {"object":"work","id":"15","medium":"galgame","display_name":"Original",
             "localized":{"zh-Hans":{"value":"中文标题","is_machine":false}}},
            {"object":"work","id":"9007199254740993","medium":"doujin_game","display_name":"Doujin"},
            {"object":"work","id":"17","medium":"music","display_name":"Soundtrack"}
        ]})).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "中文标题");
        assert!(results[0].aliases.contains(&"Original".into()));
        assert!(results[0].aliases.contains(&"中文标题".into()));
        assert_eq!(results[0].url, work_url("15"));
        assert_eq!(results[1].url, work_url("9007199254740993"));
    }

    #[test]
    fn search_preserves_catalog_order_original_names_and_aliases() {
        let results = parse_search(&json!({"object":"list","items":[
            {"object":"work","id":"226643","medium":"galgame","display_name":"セックスセラピスト",
             "localized":{"zh-Hans":{"value":"性治疗师"}}},
            {"object":"work","id":"982","medium":"galgame","display_name":"援女交際 ～亜梨紗の情事～",
             "localized":{"zh-Hans":{"value":"中文译名"}},"titles":[{"title":"別名","latin":"Alternate Title"}]}
        ]})).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, work_url("226643"));
        assert_eq!(results[1].url, work_url("982"));
        for name in [
            "援女交際 ～亜梨紗の情事～",
            "中文译名",
            "別名",
            "Alternate Title",
        ] {
            assert!(results[1].aliases.iter().any(|alias| alias == name));
        }
    }

    #[test]
    fn malformed_ids_and_missing_list_are_errors() {
        for value in [
            json!({"object":"list","items":[{"object":"work","id":"15/other","medium":"galgame","display_name":"Game"}]}),
            json!({"object":"list","items":[{"object":"work","id":15,"medium":"galgame","display_name":"Game"}]}),
            json!({"object":"list"}),
        ] {
            assert!(parse_search(&value).is_err());
        }
    }

    #[test]
    fn metadata_maps_multilingual_intros_companies_credits_and_safe_tags() {
        let work = json!({"object":"work","id":"15","medium":"galgame","display_name":"Game",
            "release_date":"2016-07-29","cover":{"url":"https://image.kungal.iloveren.link/front.webp"},
            "intros":[{"lang":"en","value":"English"},{"lang":"zh-Hans","value":"机器翻译","is_machine":true},
                      {"lang":"zh-Hans","value":"第一段\n\n第二段","is_machine":false}],
            "companies":[{"display_name":"Studio","attribution_role":"developer"},{"display_name":"Publisher","attribution_role":"publisher"}],
            "credits":[
                {"role_key":"scenario","role_name":"脚本","credits":[{"display_name":"Writer"}]},
                {"role_key":"character-design","credits":[{"display_name":"Artist"}]},
                {"role_key":"voice-actor","credits":[{"display_name":"Voice"},{"display_name":"Voice"}]},
                {"role_key":"vocal","credits":[{"display_name":"Singer"}]},
                {"role_key":"composer","credits":[{"display_name":"Composer"}]},
                {"role_key":"director","role_name":"导演","credits":[{"display_name":"Director"}]}
            ],
            "tags":[{"display_name":"Romance","spoiler":"none"},{"display_name":"Spoiler","spoiler":"major"}]
        });
        let meta = parse_metadata(&work, &format!("{API}/15")).unwrap();
        assert_eq!(meta.description, "第一段\n\n第二段");
        assert_eq!(
            (
                meta.developer.as_str(),
                meta.brand.as_str(),
                meta.publisher.as_str()
            ),
            ("Studio", "Studio", "Publisher")
        );
        assert_eq!(
            (
                meta.writers.as_str(),
                meta.artists.as_str(),
                meta.voices.as_str()
            ),
            ("Writer", "Artist", "Voice")
        );
        assert_eq!(
            (meta.singers.as_str(), meta.music.as_str()),
            ("Singer", "Composer")
        );
        assert_eq!(meta.release_date, "2016-07-29");
        assert_eq!(meta.tags, ["Romance"]);
        assert_eq!(meta.tag_names(), ["Romance"]);
        assert!(meta.cover_url.is_some());
    }

    #[test]
    fn covers_exclude_backs_discs_and_duplicate_primary_image() {
        let work = json!({"object":"work","id":"15","medium":"galgame","display_name":"Game",
        "cover":{"url":"front.webp"},"covers":[
            {"url":"front.webp","cover_kind":"main"},
            {"url":"pack.webp","cover_kind":"pkgfront"},
            {"url":"back.webp","cover_kind":"pkgback"},
            {"url":"disc.webp","cover_kind":"media"}
        ]});
        let covers = parse_covers(&work, &format!("{API}/15")).unwrap();
        assert_eq!(
            covers
                .iter()
                .map(|cover| cover.image_url.as_str())
                .collect::<Vec<_>>(),
            vec!["front.webp", "pack.webp"]
        );
    }

    #[test]
    fn circles_supply_the_brand_and_partial_dates_keep_their_precision() {
        let mut work = json!({"object":"work","id":"15","medium":"doujin_game","display_name":"Game",
            "release_date":"2016-01-01","release_date_precision":"year",
            "companies":[{"display_name":"Circle","attribution_role":"circle"}]});
        let meta = parse_metadata(&work, &work_url("15")).unwrap();
        assert_eq!(
            (meta.developer.as_str(), meta.brand.as_str()),
            ("Circle", "Circle")
        );
        assert_eq!(meta.release_date, "2016");
        work["release_date_precision"] = json!("month");
        assert_eq!(
            parse_metadata(&work, &work_url("15")).unwrap().release_date,
            "2016-01"
        );
    }

    #[test]
    fn company_attributions_prevent_untranslated_credit_aliases_in_company_fields() {
        let mut work = json!({"object":"work","id":"15","medium":"galgame","display_name":"Game",
        "companies":[
            {"display_name":"ゆずソフト","localized":{"zh-Hans":{"value":"柚子软件"}},"attribution_role":"developer"},
            {"display_name":"HIKARI FIELD","localized":{"zh-Hans":{"value":"光之领域"}},"attribution_role":"publisher"}
        ],
        "credits":[
            {"role_key":"developer","credits":[{"display_name":"ゆずソフト"}]},
            {"role_key":"publisher","credits":[{"display_name":"HIKARI FIELD"}]}
        ]});
        let meta = parse_metadata(&work, &work_url("15")).unwrap();
        assert_eq!(
            (meta.developer.as_str(), meta.brand.as_str()),
            ("柚子软件", "柚子软件")
        );
        assert_eq!(meta.publisher, "光之领域");

        work["companies"] = json!([]);
        let fallback = parse_metadata(&work, &work_url("15")).unwrap();
        assert_eq!(fallback.developer, "ゆずソフト");
        assert_eq!(fallback.publisher, "HIKARI FIELD");
    }
}
