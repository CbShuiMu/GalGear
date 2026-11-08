//! 全局备份库中的游戏目录映射，标题只作目录名称，身份仍使用稳定游戏 ID。
use super::{error, safe_id};
use galgear_i18n::Message;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn index(data: &Path) -> Result<BTreeMap<String, String>, Message> {
    super::migration::reject_links(&data.join("games.json"))?;
    match fs::read(data.join("games.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(error),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(err) => Err(error(err)),
    }
}

pub(super) fn directory(data: &Path, game: &str) -> Result<PathBuf, Message> {
    safe_id(game)?;
    let entries = index(data)?;
    let name = entries.get(game).map(String::as_str).unwrap_or(game);
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || name.ends_with([' ', '.'])
    {
        return Err(error("Invalid game backup directory"));
    }
    let path = data.join(name);
    super::migration::reject_links(&path)?;
    Ok(path)
}

/// 为游戏分配短且可读的目录，同名作品分开保存，后续改名保留原目录关联。
/// 调用者必须持有备份写入协调锁。已有索引损坏时返回错误。
pub fn register_game(data: &Path, game: &str, title: &str) -> Result<(), Message> {
    safe_id(game)?;
    super::migration::reject_links(data)?;
    let mut entries = index(data)?;
    if entries.contains_key(game) {
        directory(data, game)?;
        return Ok(());
    }
    let name: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(64)
        .collect();
    let name = name.trim().trim_end_matches([' ', '.']);
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        });
    let base = if name.is_empty() {
        "Game".into()
    } else if reserved {
        format!("_{name}")
    } else {
        name.to_owned()
    };
    let mut name = base.clone();
    let mut number = 2;
    while data.join(&name).exists()
        || entries
            .values()
            .any(|entry| entry.eq_ignore_ascii_case(&name))
    {
        name = format!("{base}-{number}");
        number += 1;
    }
    entries.insert(game.into(), name);
    fs::create_dir_all(data).map_err(error)?;
    let mut staged = tempfile::NamedTempFile::new_in(data).map_err(error)?;
    serde_json::to_writer_pretty(&mut staged, &entries).map_err(error)?;
    staged.flush().map_err(error)?;
    staged.as_file().sync_all().map_err(error)?;
    staged.persist(data.join("games.json")).map_err(error)?;
    Ok(())
}

pub(super) fn timestamp_name(created: u64) -> Result<String, Message> {
    let date = time::OffsetDateTime::from_unix_timestamp(i64::try_from(created).map_err(error)?)
        .map_err(error)?;
    #[cfg(windows)]
    let date = date.to_offset(time::UtcOffset::local_offset_at(date).map_err(error)?);
    Ok(format!(
        "{:04}-{:02}-{:02}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        date.hour(),
        date.minute()
    ))
}

pub(super) fn snapshot_id(parent: &Path, created: u64) -> Result<String, Message> {
    let base = timestamp_name(created)?;
    let mut id = base.clone();
    let mut number = 2;
    while parent.join(&id).exists() {
        id = format!("{base}-{number}");
        number += 1;
    }
    Ok(id)
}

/// 已登记的游戏 ID 和目录名称，包括已经从游戏库删除的条目所留下的备份。
pub fn registered_games(data: &Path) -> Result<BTreeMap<String, String>, Message> {
    index(data)
}

#[cfg(test)]
mod tests {
    use super::{directory, register_game};
    #[test]
    fn named_directories_keep_distinct_identities_and_survive_title_edits() {
        let temp = tempfile::tempdir().unwrap();
        register_game(temp.path(), "one", "ATRI").unwrap();
        register_game(temp.path(), "two", "ATRI").unwrap();
        register_game(temp.path(), "one", "New title").unwrap();
        assert_eq!(
            directory(temp.path(), "one").unwrap(),
            temp.path().join("ATRI")
        );
        assert_eq!(
            directory(temp.path(), "two").unwrap(),
            temp.path().join("ATRI-2")
        );
        register_game(temp.path(), "three", "../../CON:<a>").unwrap();
        assert_eq!(
            directory(temp.path(), "three").unwrap().parent(),
            Some(temp.path())
        );
        register_game(temp.path(), "four", "CON.txt").unwrap();
        assert_eq!(
            directory(temp.path(), "four").unwrap(),
            temp.path().join("_CON.txt")
        );
        register_game(temp.path(), "trailing", "Title . . ").unwrap();
        assert_eq!(
            directory(temp.path(), "trailing").unwrap(),
            temp.path().join("Title")
        );
        std::fs::write(temp.path().join("games.json"), b"broken").unwrap();
        assert!(register_game(temp.path(), "five", "New").is_err());
    }
}
