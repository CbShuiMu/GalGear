//! 导入阶段原样读取目录名，不访问文件系统。
use std::path::Path;

/// 返回 EXE 所在目录自身的名称，保留编号、空格及标点。
/// 路径没有目录名（例如盘符根目录）时返回空串。
pub fn from_directory(folder: &Path) -> String {
    folder
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::from_directory;
    use std::path::Path;

    #[test]
    fn directory_name_is_preserved_without_keyword_filtering() {
        for name in [
            "game",
            "data",
            "bin",
            "www",
            "x64",
            "游戏",
            "本体",
            "123",
            "8  援女交際 ～亜梨紗の情事～",
            "  Title  ",
            "Title.v2",
        ] {
            assert_eq!(from_directory(&Path::new("Parent Title").join(name)), name);
        }
        assert_eq!(
            from_directory(Path::new("Parent Title/game/data/bin")),
            "bin"
        );
    }

    #[test]
    fn nameless_directory_returns_empty_text() {
        for folder in ["", "/"] {
            assert!(from_directory(Path::new(folder)).is_empty());
        }
    }
}
