//! 导入阶段的目录命名规则；只选择名称，不改写名称或访问文件系统。
use std::path::Path;

/// 从目录本身开始向上寻找非通用名称，原样保留编号、空格及标点。
/// 允许越过所选游戏库根目录；直到路径根仍无有效名称时返回空串。
pub fn from_directory(folder: &Path) -> String {
    folder
        .ancestors()
        .filter_map(Path::file_name)
        .map(|name| name.to_string_lossy())
        .find(|name| !is_generic(name))
        .map_or_else(String::new, |name| name.into_owned())
}

pub(crate) fn is_generic(value: &str) -> bool {
    let value = value.trim();
    let suffix = value.trim_start_matches(|ch: char| ch.is_ascii_digit());
    // 编号和分隔符只参与通用名判断，不修改最终导入名称。
    let value = if suffix.len() < value.len()
        && (suffix.starts_with("  ") || suffix.starts_with(". ") || suffix.starts_with('、'))
    {
        suffix.trim_start_matches([' ', '.', '、']).trim()
    } else {
        value
    };
    let key: String = value
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
        .collect();
    !key.chars().any(char::is_alphabetic)
        || matches!(
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

#[cfg(test)]
mod tests {
    use super::from_directory;
    use std::path::Path;

    #[test]
    fn import_uses_nearest_meaningful_ancestor_verbatim() {
        let name = "8  援女交際 ～亜梨紗の情事～";
        let title = Path::new("library").join(name);
        for folder in [title.clone(), title.join("game"), title.join("8  game/bin")] {
            assert_eq!(from_directory(&folder), name);
        }
        assert_eq!(from_directory(Path::new("人妻俱乐部/game")), "人妻俱乐部");
        assert_eq!(from_directory(Path::new("12 Riven/game")), "12 Riven");
        assert_eq!(from_directory(Path::new("  Title  /game")), "  Title  ");
        assert_eq!(from_directory(Path::new("Title.v2/game")), "Title.v2");
    }

    #[test]
    fn generic_or_empty_paths_do_not_become_search_titles() {
        for folder in ["", "game/data/bin", "GAME/ｗｗｗ", "123/本体", "/"] {
            assert!(from_directory(Path::new(folder)).is_empty(), "{folder}");
        }
    }
}
