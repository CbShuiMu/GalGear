//! 存档路径中的变量只在根位置展开；游戏包中的同名占位目录不是用户目录。
use std::path::{Path, PathBuf};

pub(super) fn has_placeholder(path: &Path) -> bool {
    path.to_string_lossy().split(['/', '\\']).any(|part| {
        (part.starts_with("$(") && part.ends_with(')'))
            || (part.starts_with('%') && part.ends_with('%') && part.len() > 2)
    })
}

pub(super) fn expand(
    value: &str,
    mut variable: impl FnMut(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let normalized = value.replace('\\', "/");
    let (first, suffix) = normalized.split_once('/').unwrap_or((&normalized, ""));
    let name = first
        .strip_prefix("$(")
        .and_then(|s| s.strip_suffix(')'))
        .or_else(|| first.strip_prefix('%').and_then(|s| s.strip_suffix('%')));
    let path = if let Some(name) = name {
        let name = name.to_ascii_uppercase();
        if !["APPDATA", "LOCALAPPDATA", "USERPROFILE", "CURRENT"].contains(&name.as_str())
            || suffix
                .split('/')
                .any(|part| part == ".." || part.contains(':'))
            || suffix.starts_with('/')
        {
            return None;
        }
        variable(&name)?.join(suffix)
    } else {
        PathBuf::from(normalized)
    };
    (!has_placeholder(&path)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::expand;
    use std::path::{Path, PathBuf};

    #[test]
    fn variables_expand_only_at_the_root_without_traversal_or_literal_fallback() {
        let root = Path::new("User/Roaming");
        for input in ["%APPDATA%/SYRUP/GameFHD", "$(appdata)\\SYRUP\\GameFHD"] {
            assert_eq!(
                expand(input, |_| Some(root.into())),
                Some(root.join("SYRUP/GameFHD"))
            );
        }
        for input in [
            "Game/$(APPDATA)/SYRUP/GameFHD",
            "%UNKNOWN%/save",
            "%APPDATA%/../save",
            "$(APPDATA)//save",
            "$(APPDATA)/C:/save",
            "$(APPDATA)/%UNKNOWN%/save",
        ] {
            assert_eq!(expand(input, |_| Some(root.into())), None, "{input}");
        }
        assert_eq!(expand("$(APPDATA)/save", |_| None), None);
        assert_eq!(
            expand("savedata", |_| None),
            Some(PathBuf::from("savedata"))
        );
    }
}
