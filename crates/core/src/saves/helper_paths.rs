//! 只读解析游戏附带的“打开存档目录”批处理中的字面路径；不执行命令或展开脚本变量。
use super::{SaveReport, add, paths, save_files, validate_file};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn discover(folder: &Path, roots: &[(String, PathBuf)], report: &mut SaveReport) {
    // 这里只检查游戏目录直属的小文本文件，不递归扫描资源包或执行辅助程序。
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.take(512).filter_map(Result::ok) {
        let path = entry.path();
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
            || !entry.metadata().is_ok_and(|m| m.len() <= 64 * 1024)
            || validate_file(&path).is_err()
        {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let text = if bytes.starts_with(&[0xff, 0xfe]) {
            encoding_rs::UTF_16LE.decode(&bytes[2..]).0
        } else if bytes.starts_with(&[0xfe, 0xff]) {
            encoding_rs::UTF_16BE.decode(&bytes[2..]).0
        } else if let Ok(text) = std::str::from_utf8(&bytes) {
            std::borrow::Cow::Borrowed(text.trim_start_matches('\u{feff}'))
        } else {
            encoding_rs::SHIFT_JIS.decode(&bytes).0
        };
        for line in text.lines() {
            let Some(value) = explorer_path(line) else {
                continue;
            };
            let Some(resolved) = paths::expand(value, |name| {
                let source = match name {
                    "APPDATA" => "roaming",
                    "LOCALAPPDATA" => "local",
                    _ => return None,
                };
                roots
                    .iter()
                    .find(|(kind, _)| kind == source)
                    .map(|(_, path)| path.clone())
            }) else {
                continue;
            };
            if super::validate_components(&resolved).is_ok() && save_files(&resolved) {
                add(report, resolved, "config", "configured");
            }
        }
    }
}

fn explorer_path(line: &str) -> Option<&str> {
    let line = line.trim().trim_start_matches('@');
    let (command, value) = line.split_once(char::is_whitespace)?;
    if !command.eq_ignore_ascii_case("explorer") && !command.eq_ignore_ascii_case("explorer.exe") {
        return None;
    }
    let value = value.trim();
    let value = if value.starts_with('"') {
        value.strip_prefix('"')?.strip_suffix('"')?
    } else {
        if value.chars().any(char::is_whitespace) {
            return None;
        }
        value
    };
    let (variable, suffix) = value.split_once(['/', '\\'])?;
    if !(variable.eq_ignore_ascii_case("%APPDATA%")
        || variable.eq_ignore_ascii_case("%LOCALAPPDATA%"))
        || suffix.is_empty()
        || suffix
            .split(['/', '\\'])
            .any(|part| matches!(part, "" | "." | ".."))
        || suffix.contains(['%', '!', '&', '|', '<', '>', '^', '"', '*', '?'])
    {
        return None;
    }
    Some(value)
}
