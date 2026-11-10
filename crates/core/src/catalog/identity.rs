//! 扫描结果与已有安装记录的身份匹配，以及旧版空白扫描副本的识别。
#[cfg(test)]
mod tests;
use crate::{Game, GameEdit, Settings, config::display_path};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

fn path_key(path: &str) -> String {
    let path = Path::new(path);
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    display_path(&canonical)
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

pub(crate) struct InstalledGames<'a> {
    folders: BTreeMap<String, Vec<&'a Game>>,
    settings: &'a Settings,
}

impl<'a> InstalledGames<'a> {
    pub(crate) fn new(games: &'a [Game], settings: &'a Settings) -> Self {
        let mut folders: BTreeMap<String, Vec<&Game>> = BTreeMap::new();
        // 目录缓存包含最新资料，manual_games 补齐尚未写入缓存的手动条目。
        for game in games.iter().chain(&settings.manual_games) {
            let entries = folders.entry(path_key(&game.folder)).or_default();
            if !entries.iter().any(|entry| entry.id == game.id) {
                entries.push(game);
            }
        }
        Self { folders, settings }
    }

    pub(crate) fn find(&self, folder: &str, executable: Option<&str>) -> Option<&'a Game> {
        let entries = self.folders.get(&path_key(folder))?;
        if let Some(executable) = executable {
            let executable = path_key(executable);
            let mut exact = entries.iter().filter(|game| {
                self.settings
                    .game_edits
                    .get(&game.id)
                    .and_then(|edit| edit.executable.as_deref())
                    .or(game.executable.as_deref())
                    .is_some_and(|path| path_key(path) == executable)
            });
            if let Some(game) = exact.next() {
                return exact.next().is_none().then_some(*game);
            }
        }
        // 同目录多个手动版本不能通过目录兜底猜测其身份。
        if let [game] = entries.as_slice() {
            Some(*game)
        } else {
            None
        }
    }
}

/// 返回可清理的旧版空白扫描副本。调用者还需检查独立游玩记录及备份数据。
/// 仅识别同目录唯一手动条目旁的目录哈希副本，不合并有用户配置的记录。
pub fn empty_scan_duplicates(games: &[Game], settings: &Settings) -> Vec<String> {
    let manual = InstalledGames::new(&settings.manual_games, settings);
    games
        .iter()
        .filter(|game| {
            let Some(entries) = manual.folders.get(&path_key(&game.folder)) else {
                return false;
            };
            let [original] = entries.as_slice() else {
                return false;
            };
            let untouched = GameEdit {
                setup_pending: true,
                ..Default::default()
            };
            let folder = Path::new(&game.folder);
            let identity = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_owned());
            // cover_artwork 来自按内容共享的缓存。用户选定的封面由 cover_overrides 保护。
            game.id != original.id
                && game.id
                    == format!(
                        "{:x}",
                        Sha256::digest(display_path(&identity).to_lowercase().as_bytes())
                    )
                && !game.scan_stamp.is_empty()
                && settings.game_edits.get(&game.id) == Some(&untouched)
                && game.metadata.is_none()
                && game.tags.is_empty()
                && game.play_status == Default::default()
                && game.playtime_seconds == 0
                && game.last_played.is_none()
                && game.cg_unlock_status == "unsupported"
                && game.auto_unlock == Default::default()
                && !settings.cover_overrides.contains_key(&game.id)
                && !settings.game_fonts.contains_key(&game.id)
                && !settings.excluded_entries.contains_key(&game.id)
                && !super::is_excluded(game, settings)
                && !settings
                    .collections
                    .iter()
                    .any(|collection| collection.games.contains(&game.id))
                && !settings
                    .dialog_directories
                    .keys()
                    .any(|key| key.ends_with(&format!(":{}", game.id)))
        })
        .map(|game| game.id.clone())
        .collect()
}
