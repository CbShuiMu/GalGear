//! 第三方备份的只读解析、预览与增量合并；不运行备份中的脚本或写入游戏目录。
mod archive;
mod luna;
mod potato;
mod reina;
#[cfg(test)]
mod tests;

use crate::{
    Game, GameEdit, PlayStatus, Settings, catalog, collections::Collection, providers::Metadata,
};
use galgear_i18n::Message;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) const MAX_ROWS: usize = 30_000;
pub(super) const MAX_DATA: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Potato,
    Reina,
    Luna,
}
impl Source {
    pub fn parse(value: &str) -> Result<Self, Message> {
        match value {
            "potatovn" => Ok(Self::Potato),
            "reinamanager" => Ok(Self::Reina),
            "lunabox" => Ok(Self::Luna),
            _ => Err(Message::new("import.format")),
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Potato => "potatovn",
            Self::Reina => "reinamanager",
            Self::Luna => "lunabox",
        }
    }
}
#[derive(Default)]
pub(super) struct Entry {
    id: String,
    title: String,
    folder: String,
    executable: String,
    metadata: Metadata,
    cover: Option<String>,
    tags: Vec<String>,
    status: PlayStatus,
    seconds: u64,
    last_played: Option<u64>,
    save_path: String,
    locale: bool,
    upscale: bool,
}
#[derive(Default)]
pub(super) struct Data {
    entries: Vec<Entry>,
    groups: Vec<(String, Vec<String>)>,
    cover_bytes: usize,
}
impl Data {
    fn push(&mut self, entry: Entry) -> Result<(), Message> {
        self.cover_bytes = self
            .cover_bytes
            .saturating_add(entry.cover.as_ref().map_or(0, String::len));
        if self.cover_bytes > 128 * 1024 * 1024 || self.entries.len() >= MAX_ROWS {
            return Err(Message::new("import.limit"));
        }
        self.entries.push(entry);
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub struct Row {
    pub title: String,
    pub path: String,
    pub reason: Option<Message>,
}
#[derive(Clone, Default, Serialize)]
pub struct Preview {
    pub source: String,
    pub added: usize,
    pub skipped: usize,
    pub rows: Vec<Row>,
}
#[derive(Clone)]
pub struct Plan {
    pub preview: Preview,
    pub games: Vec<(Game, GameEdit)>,
    pub collections: Vec<Collection>,
}

fn error(value: impl std::fmt::Display) -> Message {
    Message::detail("import.error", value)
}
fn text(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
fn number(value: &Value, key: &str) -> u64 {
    value[key]
        .as_u64()
        .or_else(|| text(value, key).parse().ok())
        .unwrap_or(0)
}
fn truth(value: &Value, key: &str) -> bool {
    value[key]
        .as_bool()
        .unwrap_or_else(|| matches!(text(value, key).as_str(), "1" | "true"))
}
fn strings(value: &Value) -> Vec<String> {
    if let Some(a) = value.as_array() {
        a.iter()
            .filter_map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.get("name").and_then(Value::as_str).map(str::to_owned))
            })
            .collect()
    } else if let Some(s) = value.as_str() {
        serde_json::from_str::<Value>(s)
            .ok()
            .filter(Value::is_array)
            .map(|v| strings(&v))
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}
fn source_url(source: &str, id: &str) -> String {
    let id = id.trim().trim_start_matches(['v', 'V']);
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) {
        return String::new();
    }
    match source.to_ascii_lowercase().as_str() {
        "vndb" => format!("https://vndb.org/v{id}"),
        "bgm" | "bangumi" => format!("https://bgm.tv/subject/{id}"),
        "ymgal" => format!("https://www.ymgal.games/ga{id}"),
        "2dfan" => format!("https://2dfan.com/subjects/{id}"),
        _ => String::new(),
    }
}
fn timestamp(value: &str) -> Option<u64> {
    // Both formats use ISO calendar dates; DuckDB may use a space and an offset without minutes.
    let s = value.trim();
    if s.len() < 19 {
        return None;
    }
    let nums: Vec<i32> = s
        .get(..19)?
        .split(['-', 'T', ' ', ':'])
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if nums.len() != 6 || nums[0] < 1970 {
        return None;
    }
    let date = time::Date::from_calendar_date(
        nums[0],
        time::Month::try_from(u8::try_from(nums[1]).ok()?).ok()?,
        u8::try_from(nums[2]).ok()?,
    )
    .ok()?;
    let local = date
        .with_hms(
            u8::try_from(nums[3]).ok()?,
            u8::try_from(nums[4]).ok()?,
            u8::try_from(nums[5]).ok()?,
        )
        .ok()?;
    let suffix = &s[19..];
    let offset = if let Some(i) = suffix.find(['+', '-']) {
        let sign = if suffix.as_bytes()[i] == b'-' { -1 } else { 1 };
        let parts: Vec<_> = suffix[i + 1..].split(':').collect();
        let hours: i8 = parts.first()?.parse().ok()?;
        let minutes: i8 = parts.get(1).unwrap_or(&"0").parse().ok()?;
        if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
            return None;
        }
        time::UtcOffset::from_hms(hours * sign, minutes * sign, 0).ok()?
    } else {
        time::UtcOffset::UTC
    };
    u64::try_from(local.assume_offset(offset).unix_timestamp()).ok()
}
fn path_key(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
fn duplicate(game: &Game, settings: &Settings, games: &[Game]) -> bool {
    games
        .iter()
        .chain(&settings.manual_games)
        .chain(settings.excluded_entries.values())
        .any(|g| {
            g.id == game.id
                || path_key(&g.folder) == path_key(&game.folder)
                || g.executable
                    .as_ref()
                    .zip(game.executable.as_ref())
                    .is_some_and(|(a, b)| path_key(a) == path_key(b))
        })
}

/// 仅预览可在本机启动的条目；已有及已排除游戏不会被覆盖。
pub fn read(
    source: Source,
    path: &Path,
    settings: &Settings,
    existing: &[Game],
) -> Result<Plan, Message> {
    let data = match source {
        Source::Potato => potato::read(path)?,
        Source::Reina => reina::read(path)?,
        Source::Luna => luna::read(path)?,
    };
    let mut plan = Plan {
        preview: Preview {
            source: source.key().into(),
            ..Default::default()
        },
        games: Vec::new(),
        collections: Vec::new(),
    };
    let mut ids = BTreeMap::new();
    let mut seen = existing.to_vec();
    for entry in data.entries {
        let exe = Path::new(&entry.executable);
        let folder = Path::new(&entry.folder);
        let mut reason = None;
        if entry.title.is_empty()
            || !exe.is_absolute()
            || !exe
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
        {
            reason = Some(Message::new("import.no_launcher"));
        } else if !exe.is_file() || !folder.is_dir() {
            reason = Some(Message::new("import.missing_path"));
        }
        if reason.is_none() {
            let mut game = catalog::manual_game(exe, entry.title.clone())?;
            // Use the scanner's directory identity so a later folder scan cannot duplicate it.
            let identity = folder.canonicalize().map_err(error)?;
            game.id = format!(
                "{:x}",
                Sha256::digest(
                    crate::config::display_path(&identity)
                        .to_lowercase()
                        .as_bytes()
                )
            );
            game.folder = crate::config::display_path(folder);
            game.library = game.folder.clone();
            if duplicate(&game, settings, &seen) {
                reason = Some(Message::new("import.duplicate"));
            } else {
                game.metadata = Some(entry.metadata);
                game.cover = entry.cover;
                game.tags = crate::tags::normalize(entry.tags);
                game.play_status = entry.status;
                game.playtime_seconds = entry.seconds;
                game.last_played = entry.last_played;
                let save = Path::new(&entry.save_path);
                let edit = GameEdit {
                    title: Some(game.title.clone()),
                    metadata: game.metadata.clone(),
                    tags: game.tags.clone(),
                    executable: game.executable.clone(),
                    play_status: game.play_status,
                    save_path: (save.is_absolute() && save.is_dir())
                        .then(|| entry.save_path.clone()),
                    launch_mode: if entry.locale { "locale" } else { "normal" }.into(),
                    upscale: entry.upscale,
                    setup_confirmed: true,
                    ..Default::default()
                };
                ids.insert(entry.id, game.id.clone());
                seen.push(game.clone());
                plan.games.push((game, edit));
            }
        }
        if reason.is_some() {
            plan.preview.skipped += 1;
        } else {
            plan.preview.added += 1;
        }
        plan.preview.rows.push(Row {
            title: entry.title,
            path: entry.executable,
            reason,
        });
    }
    for (name, members) in data.groups {
        let games: BTreeSet<_> = members
            .iter()
            .filter_map(|id| ids.get(id).cloned())
            .collect();
        if name.trim().is_empty() || games.is_empty() {
            continue;
        }
        let id = format!(
            "import-{:x}",
            Sha256::digest(format!("{}:{name}", source.key()))
        );
        plan.collections.push(Collection {
            id,
            name,
            parent: None,
            games,
        });
    }
    Ok(plan)
}

/// 确认时再次检查路径和重复项，返回实际新增的游戏；调用方负责原子保存配置。
pub fn merge(plan: &Plan, settings: &mut Settings, existing: &[Game]) -> Vec<Game> {
    let mut added = Vec::new();
    for (game, edit) in &plan.games {
        if duplicate(game, settings, existing)
            || !Path::new(&game.folder).is_dir()
            || !game
                .executable
                .as_ref()
                .is_some_and(|p| Path::new(p).is_file())
        {
            continue;
        }
        settings.manual_games.push(game.clone());
        settings.game_edits.insert(game.id.clone(), edit.clone());
        added.push(game.clone());
    }
    let ids: BTreeSet<_> = added.iter().map(|g| g.id.as_str()).collect();
    for collection in &plan.collections {
        let mut c = collection.clone();
        c.games.retain(|id| ids.contains(id.as_str()));
        if c.games.is_empty() {
            continue;
        }
        if let Some(old) = settings.collections.iter_mut().find(|old| old.id == c.id) {
            old.games.extend(c.games);
        } else {
            settings.collections.push(c);
        }
    }
    added
}
