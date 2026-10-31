use super::{
    Data, Entry, MAX_DATA, MAX_ROWS, archive::Archive, error, number, source_url, text, timestamp,
    truth,
};
use crate::PlayStatus;
use galgear_i18n::Message;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, path::Path};

fn table(archive: &mut Archive, prefix: &str, name: &str) -> Result<Vec<Value>, Message> {
    let Some(bytes) = archive.bytes(&format!("{prefix}{name}.csv"), MAX_DATA)? else {
        return Ok(Vec::new());
    };
    let mut csv = csv::Reader::from_reader(bytes.as_slice());
    let columns = csv.headers().map_err(error)?.clone();
    if name == "games"
        && ["id", "name", "path", "source_type"]
            .iter()
            .any(|key| !columns.iter().any(|column| column == *key))
    {
        return Err(Message::new("import.format"));
    }
    let mut rows = Vec::new();
    for row in csv.records() {
        if rows.len() >= MAX_ROWS * 10 {
            return Err(Message::new("import.limit"));
        }
        let row = row.map_err(error)?;
        let object: Map<_, _> = columns
            .iter()
            .zip(row.iter())
            .map(|(k, v)| (k.into(), Value::String(v.into())))
            .collect();
        rows.push(Value::Object(object));
    }
    Ok(rows)
}
pub(super) fn read(path: &Path) -> Result<Data, Message> {
    let mut archive = Archive::open(path)?;
    let prefix = if archive.contains("database/games.csv") {
        "database/"
    } else {
        ""
    };
    if !archive.contains(&format!("{prefix}games.csv")) {
        return Err(Message::new("import.luna_format"));
    }
    let games = table(&mut archive, prefix, "games")?;
    if games.len() > MAX_ROWS {
        return Err(Message::new("import.limit"));
    }
    let mut times: BTreeMap<String, (u64, Option<u64>)> = BTreeMap::new();
    for row in table(&mut archive, prefix, "play_sessions")? {
        let time = times.entry(text(&row, "game_id")).or_default();
        time.0 = time.0.saturating_add(number(&row, "duration"));
        time.1 = time.1.max(timestamp(&text(&row, "start_time")));
    }
    let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in table(&mut archive, prefix, "game_tags")? {
        tags.entry(text(&row, "game_id"))
            .or_default()
            .push(text(&row, "name"));
    }
    let mut data = Data::default();
    for v in games {
        let id = text(&v, "id");
        if id.is_empty() || v.get("name").is_none() || v.get("source_type").is_none() {
            return Err(Message::new("import.format"));
        }
        let exe = text(&v, "path");
        let folder = text(&v, "game_directory");
        let mut entry = Entry {
            id: id.clone(),
            title: text(&v, "name"),
            executable: exe.clone(),
            folder: if folder.is_empty() {
                Path::new(&exe)
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .into_owned()
            } else {
                folder
            },
            tags: tags.remove(&id).unwrap_or_default(),
            save_path: text(&v, "save_path"),
            locale: truth(&v, "use_locale_emulator"),
            upscale: truth(&v, "use_magpie"),
            ..Default::default()
        };
        entry.status = match text(&v, "status").as_str() {
            "want_to_play" => PlayStatus::Wanted,
            "playing" => PlayStatus::Playing,
            "completed" => PlayStatus::Finished,
            "on_hold" => PlayStatus::Shelved,
            "dropped" => PlayStatus::Abandoned,
            _ => PlayStatus::Unmarked,
        };
        if let Some((seconds, last)) = times.get(&id) {
            entry.seconds = *seconds;
            entry.last_played = *last;
        }
        entry.metadata.title = entry.title.clone();
        entry.metadata.description = text(&v, "summary");
        entry.metadata.developer = text(&v, "company");
        entry.metadata.brand = entry.metadata.developer.clone();
        entry.metadata.release_date = text(&v, "release_date");
        entry.metadata.url = source_url(&text(&v, "source_type"), &text(&v, "source_id"));
        let cover = text(&v, "cover_url").replace('\\', "/");
        let key = cover.find("covers/").map(|i| &cover[i..]).unwrap_or(&cover);
        entry.cover = archive
            .cover(key)?
            .or_else(|| super::archive::remote_cover(&text(&v, "cover_source_url")))
            .or_else(|| super::archive::remote_cover(&cover));
        data.push(entry)?;
    }
    let links = table(&mut archive, prefix, "game_categories")?;
    for v in table(&mut archive, prefix, "categories")? {
        if truth(&v, "is_system") {
            continue;
        }
        let id = text(&v, "id");
        data.groups.push((
            text(&v, "name"),
            links
                .iter()
                .filter(|r| text(r, "category_id") == id)
                .map(|r| text(r, "game_id"))
                .collect(),
        ));
    }
    Ok(data)
}
