use super::{
    Data, Entry, MAX_DATA, MAX_ROWS, archive::local_cover, error, number, source_url, strings,
    text, truth,
};
use crate::PlayStatus;
use galgear_i18n::Message;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, path::Path};

// Only these literal table names reach SQL. No SQL from the backup is executed.
fn table(db: &Connection, name: &str, required: bool) -> Result<Vec<Value>, Message> {
    let found: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |r| r.get(0),
        )
        .map_err(error)?;
    if !found {
        return if required {
            Err(Message::new("import.format"))
        } else {
            Ok(Vec::new())
        };
    }
    let mut query = db
        .prepare(&format!(
            "SELECT * FROM \"{name}\" LIMIT {}",
            MAX_ROWS * 10 + 1
        ))
        .map_err(error)?;
    let columns = query.column_names().map(str::to_owned).collect::<Vec<_>>();
    let mut rows = query.query([]).map_err(error)?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(error)? {
        if result.len() >= MAX_ROWS * 10 {
            return Err(Message::new("import.limit"));
        }
        let mut object = Map::new();
        for (i, key) in columns.iter().enumerate() {
            let value = match row.get_ref(i).map_err(error)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::from(n),
                ValueRef::Real(n) => Value::from(n),
                ValueRef::Text(s) => {
                    bytes += s.len();
                    if bytes as u64 > MAX_DATA {
                        return Err(Message::new("import.limit"));
                    }
                    Value::String(std::str::from_utf8(s).map_err(error)?.into())
                }
                ValueRef::Blob(_) => Value::Null,
            };
            object.insert(key.clone(), value);
        }
        result.push(Value::Object(object));
    }
    Ok(result)
}
fn json(v: &Value, key: &str) -> Result<Value, Message> {
    if let Some(s) = v[key].as_str().filter(|s| !s.trim().is_empty()) {
        serde_json::from_str(s).map_err(error)
    } else {
        Ok(v[key].clone())
    }
}
pub(super) fn read(path: &Path) -> Result<Data, Message> {
    if std::fs::metadata(path).map_err(error)?.len() > 512 * 1024 * 1024 {
        return Err(Message::new("import.limit"));
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(error)?;
    db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        MAX_DATA as i32,
    )
    .map_err(error)?;
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; BEGIN;")
        .map_err(error)?;
    let games = table(&db, "games", true)?;
    db.prepare("SELECT id,id_type,localpath FROM games LIMIT 0")
        .map_err(error)?;
    if games.len() > MAX_ROWS {
        return Err(Message::new("import.limit"));
    }
    let sources = table(&db, "game_sources", false)?;
    let stats: BTreeMap<_, _> = table(&db, "game_statistics", false)?
        .into_iter()
        .map(|v| (text(&v, "game_id"), v))
        .collect();
    let mut source_map: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in sources {
        source_map
            .entry(text(&row, "game_id"))
            .or_default()
            .push(row);
    }
    let migrations = table(&db, "seaql_migrations", false)?;
    let new_status = migrations
        .iter()
        .any(|r| text(r, "version").contains("m20260131_000006"));
    let mut data = Data::default();
    for v in games {
        let id = text(&v, "id");
        if id.is_empty() || v.get("id_type").is_none() {
            return Err(Message::new("import.format"));
        }
        let preferred = text(&v, "id_type").to_lowercase();
        let custom = json(&v, "custom_data")?;
        let rows = source_map.remove(&id).unwrap_or_default();
        let mut layers = vec![custom];
        let mut url = String::new();
        for source in [&preferred, "vndb", "bgm", "ymgal"] {
            if let Some(row) = rows.iter().find(|r| text(r, "source") == *source) {
                layers.push(json(row, "data")?);
                if url.is_empty() {
                    url = source_url(source, &text(row, "external_id"));
                }
            } else {
                layers.push(json(&v, &format!("{source}_data"))?);
                if url.is_empty() {
                    url = source_url(source, &text(&v, &format!("{source}_id")));
                }
            }
        }
        layers.push(v.clone());
        let field = |keys: &[&str]| -> String {
            layers
                .iter()
                .find_map(|layer| keys.iter().map(|k| text(layer, k)).find(|s| !s.is_empty()))
                .unwrap_or_default()
        };
        let local = text(&v, "localpath");
        let exe = text(&v, "executable");
        let local_exe = Path::new(&local)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        let folder = if local_exe {
            Path::new(&local)
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .into_owned()
        } else {
            local.clone()
        };
        let executable = if exe.is_empty() {
            if local_exe { local } else { String::new() }
        } else if Path::new(&exe).is_absolute() {
            exe
        } else {
            Path::new(&folder).join(exe).to_string_lossy().into_owned()
        };
        let mut entry = Entry {
            id: id.clone(),
            title: field(&["custom_name", "name_cn", "name"]),
            folder,
            executable,
            save_path: text(&v, "savepath"),
            locale: truth(&v, "le_launch"),
            upscale: truth(&v, "magpie"),
            ..Default::default()
        };
        let status = number(&v, "clear");
        entry.status = if new_status {
            match status {
                1 => PlayStatus::Wanted,
                2 => PlayStatus::Playing,
                3 => PlayStatus::Finished,
                4 => PlayStatus::Shelved,
                5 => PlayStatus::Abandoned,
                _ => PlayStatus::Unmarked,
            }
        } else if status == 1 {
            PlayStatus::Finished
        } else {
            PlayStatus::Unmarked
        };
        if let Some(stat) = stats.get(&id) {
            // ReinaManager stores session/statistic durations in minutes.
            entry.seconds = number(stat, "total_time").saturating_mul(60);
            entry.last_played = Some(number(stat, "last_played")).filter(|s| *s > 0);
        }
        entry.metadata.title = entry.title.clone();
        entry.metadata.description = field(&["summary"]);
        entry.metadata.developer = field(&["developer"]);
        entry.metadata.brand = entry.metadata.developer.clone();
        entry.metadata.release_date = field(&["date"]);
        entry.metadata.url = url;
        entry.tags = layers
            .iter()
            .map(|v| strings(&v["tags"]))
            .find(|tags| !tags.is_empty())
            .unwrap_or_default();
        entry.cover = local_cover(
            path.parent().unwrap_or(Path::new(".")),
            &field(&["custom_cover", "image"]),
        )?;
        data.push(entry)?;
    }
    let links = table(&db, "game_collection_link", false)?;
    for c in table(&db, "collections", false)? {
        let id = text(&c, "id");
        data.groups.push((
            text(&c, "name"),
            links
                .iter()
                .filter(|r| text(r, "collection_id") == id)
                .map(|r| text(r, "game_id"))
                .collect(),
        ));
    }
    Ok(data)
}
