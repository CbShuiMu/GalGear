//! 自动刮削的名称提取与唯一匹配规则，不执行网络请求或写入游戏资料。
use crate::providers::Resource;
use serde::Serialize;
use std::path::Path;

/// 自动查询的候选及唯一匹配；没有明确匹配时保留候选供用户手选。
#[derive(Serialize)]
pub struct SearchResult {
    pub query: String,
    pub candidates: Vec<Resource>,
    pub matched_url: Option<String>,
}

fn name_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(ch as u32 - 0xfee0).unwrap_or(ch),
            _ => ch,
        })
        .filter(|ch| {
            !ch.is_whitespace()
                && !matches!(
                    ch,
                    '-' | '‐'
                        | '–'
                        | '—'
                        | '~'
                        | '～'
                        | '〜'
                        | '_'
                        | ':'
                        | '.'
                        | ','
                        | '・'
                        | '·'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '「'
                        | '」'
                        | '『'
                        | '』'
                )
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn informative(value: &str) -> bool {
    let key = name_key(without_ordinal(value));
    key.chars().any(char::is_alphabetic)
        && !matches!(
            key.as_str(),
            "game"
                | "games"
                | "data"
                | "bin"
                | "www"
                | "gamedata"
                | "x86"
                | "x64"
                | "win32"
                | "win64"
                | "unknown"
                | "游戏"
                | "本体"
        )
}

fn without_ordinal(value: &str) -> &str {
    let value = value.trim();
    let suffix = value.trim_start_matches(|ch: char| ch.is_ascii_digit());
    // 只移除明确的目录编号格式，保留「12 Riven」等标题里的数字。
    if suffix.len() < value.len()
        && (suffix.starts_with("  ") || suffix.starts_with(". ") || suffix.starts_with('、'))
    {
        suffix.trim_start_matches([' ', '.', '、']).trim()
    } else {
        value
    }
}

/// 原样返回已有标题或库内最近的有效目录名，保留编号和空白。
/// 没有可用名称时返回空串，由调用方跳过网络查询并保留待确认状态。
pub fn query(title: &str, folder: &Path, library: &Path) -> String {
    if informative(title) {
        return title.into();
    }
    let Ok(relative) = folder.strip_prefix(library) else {
        return String::new();
    };
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return String::new();
    }
    relative
        .ancestors()
        .filter_map(Path::file_name)
        .filter_map(|name| name.to_str())
        .find(|name| informative(name))
        .unwrap_or_default()
        .into()
}

/// 原名、译名或别名规范化后完全一致且仅有一个候选时允许自动确认。
/// 仅在比较时允许忽略明确的目录编号，保留标题数字、副标题和版本文字。
pub fn resolve(query: String, candidates: Vec<Resource>) -> SearchResult {
    let key = name_key(&query);
    let unnumbered_key = name_key(without_ordinal(&query));
    let meaningful = informative(&query);
    let mut matches = candidates.iter().filter(|candidate| {
        meaningful
            && !candidate.url.is_empty()
            && std::iter::once(&candidate.title)
                .chain(&candidate.aliases)
                .any(|name| {
                    let name = name_key(name);
                    name == key || name == unnumbered_key
                })
    });
    let first = matches.next();
    let matched_url = first
        .filter(|_| matches.next().is_none())
        .map(|candidate| candidate.url.clone());
    SearchResult {
        query,
        candidates,
        matched_url,
    }
}

#[cfg(test)]
mod tests {
    use super::{query, resolve};
    use crate::providers::Resource;
    use std::path::Path;

    fn candidate(title: &str, id: &str, aliases: &[&str]) -> Resource {
        Resource {
            title: title.into(),
            url: format!("https://developer.nextmoe.dev/explore/work/{id}?nsfw=1"),
            aliases: aliases.iter().map(|name| (*name).into()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn nested_installation_uses_title_or_nearest_meaningful_parent() {
        let root = Path::new("library");
        let directory_name = "8  援女交際 ～亜梨紗の情事～";
        let folder = root.join(directory_name).join("8  game/bin");
        for title in ["game", "", directory_name] {
            assert_eq!(query(title, &folder, root), directory_name);
        }
        assert_eq!(query("  Manual Title  ", &folder, root), "  Manual Title  ");
        assert_eq!(query("12 Riven", &folder, root), "12 Riven");
    }

    #[test]
    fn numbered_query_matches_unnumbered_alias_without_changing_search_text() {
        let search = "8  援女交際 ～亜梨紗の情事～";
        let correct = candidate("译名", "982", &["援女交際 ～亜梨紗の情事～"]);
        let result = resolve(search.into(), vec![correct.clone()]);
        assert_eq!(result.query, search);
        assert_eq!(result.matched_url.as_deref(), Some(correct.url.as_str()));

        let numbered = candidate(search, "983", &[]);
        assert!(
            resolve(search.into(), vec![correct, numbered])
                .matched_url
                .is_none()
        );
    }

    #[test]
    fn generic_names_never_escape_library_boundary() {
        let root = Path::new("Some Game/library");
        for folder in [
            root.to_owned(),
            root.join("game/data/bin"),
            root.join("../Other/game"),
            Path::new("Elsewhere/game").to_owned(),
        ] {
            assert!(query("game", &folder, root).is_empty());
        }
        assert!(
            resolve("game".into(), vec![candidate("Game", "1", &[])])
                .matched_url
                .is_none()
        );
    }

    #[test]
    fn unrelated_first_result_is_not_confirmed_and_original_name_matches_translation() {
        let wrong = candidate("性治疗师", "226643", &["セックスセラピスト"]);
        let title = "援女交際 ～亜梨紗の情事～";
        assert!(
            resolve(title.into(), vec![wrong.clone()])
                .matched_url
                .is_none()
        );
        let correct = candidate("译名", "982", &[title]);
        assert_eq!(
            resolve(title.into(), vec![wrong, correct.clone()])
                .matched_url
                .as_deref(),
            Some(correct.url.as_str())
        );
    }

    #[test]
    fn normalized_aliases_match_but_ambiguity_sequels_and_editions_do_not() {
        let original = candidate("中文标题", "1", &["Ｓｏｍｅ　Ｔｉｔｌｅ ～副标题～"]);
        assert!(
            resolve("some title~副标题~".into(), vec![original.clone()])
                .matched_url
                .is_some()
        );
        for query in [
            "Some Title",
            "Some Title 2 ～副标题～",
            "Some Title ～副标题～ PLUS",
            "",
            "123",
        ] {
            assert!(
                resolve(query.into(), vec![original.clone()])
                    .matched_url
                    .is_none()
            );
        }
        let other = candidate("同名译名", "2", &["Some Title ～副标题～"]);
        assert!(
            resolve("Some Title ～副标题～".into(), vec![original, other])
                .matched_url
                .is_none()
        );
        let legacy: Resource =
            serde_json::from_str(r#"{"title":"Legacy","url":"https://vndb.org/v1"}"#).unwrap();
        assert!(resolve("Legacy".into(), vec![legacy]).matched_url.is_some());
    }
}
