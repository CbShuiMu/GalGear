use super::{
    Data, Entry, MAX_ROWS, archive::Archive, error, number, source_url, strings, text, timestamp,
    truth,
};
use crate::PlayStatus;
use galgear_i18n::Message;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

fn property<'a>(v: &'a Value, key: &str) -> &'a Value {
    v[key].get("Value").unwrap_or(&v[key])
}
fn label(v: &Value, key: &str) -> String {
    property(v, key).as_str().unwrap_or_default().to_owned()
}
pub(super) fn read(path: &Path) -> Result<Data, Message> {
    let mut archive = Archive::open(path)?;
    let value = archive.json("data.galgames.json")?;
    let games = value
        .as_array()
        .ok_or_else(|| Message::new("import.format"))?;
    if games.len() > MAX_ROWS {
        return Err(Message::new("import.limit"));
    }
    let sources = archive.json("data.galgameSources.json")?;
    let mut installs: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for source in sources.as_array().into_iter().flatten() {
        for entry in source["Galgames"].as_array().into_iter().flatten() {
            let id = entry["Galgame"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| text(&entry["Galgame"], "Uuid"));
            installs.entry(id).or_default().push(entry.clone());
        }
    }
    let mut data = Data::default();
    for v in games {
        let id = text(v, "Uuid");
        if id.is_empty() {
            return Err(error("PotatoVN: missing Uuid"));
        }
        let candidates = installs.get(&id);
        let install = candidates.and_then(|a| {
            a.iter()
                .find(|i| text(i, "EntryId") == text(v, "PreferredInstallationId"))
                .or_else(|| a.first())
        });
        let config = install
            .map(|i| &i["LocalConfig"])
            .filter(|c| c.is_object())
            .unwrap_or(v);
        let folder = install
            .map(|i| text(i, "Path"))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| text(v, "Path"));
        let exe = text(config, "ExePath");
        let executable = if !exe.is_empty() && !Path::new(&exe).is_absolute() {
            Path::new(&folder).join(exe).to_string_lossy().into_owned()
        } else {
            exe
        };
        let mut entry = Entry {
            id,
            title: label(v, "Name"),
            folder,
            executable,
            tags: strings(property(v, "Tags")),
            seconds: number(v, "TotalPlayTime").saturating_mul(60),
            last_played: timestamp(&text(v, "LastPlayTime")),
            save_path: text(config, "SavePath"),
            locale: truth(config, "RunInLocaleEmulator"),
            upscale: truth(v, "EnableMagpie"),
            ..Default::default()
        };
        entry.status = match number(v, "PlayType") {
            1 => PlayStatus::Playing,
            2 => PlayStatus::Finished,
            3 => PlayStatus::Shelved,
            4 => PlayStatus::Abandoned,
            5 => PlayStatus::Wanted,
            _ => PlayStatus::Unmarked,
        };
        entry.metadata.title = entry.title.clone();
        entry.metadata.description = label(v, "Description");
        entry.metadata.developer = label(v, "Developer");
        entry.metadata.brand = entry.metadata.developer.clone();
        entry.metadata.release_date = label(v, "ReleaseDate").chars().take(10).collect();
        let preferred = number(v, "RssType") as usize;
        for index in std::iter::once(preferred).chain([1, 0, 5]) {
            let source = match index {
                0 => "vndb",
                1 => "bgm",
                5 => "ymgal",
                _ => continue,
            };
            let id = v["Ids"]
                .get(index)
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = source_url(source, id);
            if !url.is_empty() {
                entry.metadata.url = url;
                break;
            }
        }
        entry.cover = archive.cover(&label(v, "ImagePath"))?;
        data.push(entry)?;
    }
    let groups = archive.json("data.categoryGroups.json")?;
    for group in groups.as_array().into_iter().flatten() {
        // Built-in status/developer categories are reconstructed from each game's metadata.
        if number(group, "Type") != 2 {
            continue;
        }
        for c in group["Categories"].as_array().into_iter().flatten() {
            data.groups.push((
                text(c, "Name"),
                strings(c.get("GalgamesX").unwrap_or(&c["Galgames"])),
            ));
        }
    }
    Ok(data)
}
