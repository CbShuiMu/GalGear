use crate::{Game, ScanResult, Settings, config::display_path};
use galgear_i18n::Message;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};

pub fn manual_game(path: &Path, title: String) -> Result<Game, Message> {
    let folder = path
        .parent()
        .ok_or_else(|| Message::new("error.game_folder"))?;
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Err(Message::new("error.executable_required"));
    }
    Ok(Game {
        id: format!(
            "{:x}",
            Sha256::digest(display_path(path).to_lowercase().as_bytes())
        ),
        title,
        folder: display_path(folder),
        library: display_path(folder),
        executable: Some(display_path(path)),
        engine: "unknown".into(),
        play_status: Default::default(),
        cover: None,
        cover_artwork: None,
        modified: 0,
        playtime_seconds: 0,
        last_played: None,
        cg_unlock_status: "unsupported".into(),
        auto_unlock: Default::default(),
        tags: Vec::new(),
        available: path.is_file(),
        metadata: None,
        scan_stamp: String::new(),
        metadata_warning: None,
    })
}

pub fn apply_edits(game: &mut Game, settings: &Settings) {
    if let Some(edit) = settings.game_edits.get(&game.id) {
        if let Some(title) = &edit.title {
            game.title.clone_from(title);
        }
        game.tags.clone_from(&edit.tags);
        game.play_status = edit.play_status;
        if let Some(exe) = &edit.executable {
            game.executable = Some(exe.clone());
        }
        if let Some(metadata) = &edit.metadata {
            game.metadata = Some(metadata.clone());
        }
        if let Some(metadata) = &mut game.metadata {
            for (field, value) in &edit.metadata_overrides {
                metadata.set_field(field, value);
            }
            if let Some(description) = &edit.description {
                metadata.description.clone_from(description);
            }
        }
        if edit.title.is_none()
            && let Some(metadata) = &game.metadata
            && !metadata.title.is_empty()
        {
            game.title.clone_from(&metadata.title);
        }
    }
    let edit = settings.game_edits.get(&game.id);
    let imported = edit
        .and_then(|edit| edit.source_tags.clone())
        .unwrap_or_else(|| {
            game.metadata
                .as_ref()
                .map(|meta| meta.tag_names())
                .unwrap_or_default()
        });
    if !imported.is_empty() {
        let rules = edit.map(|edit| &edit.tag_rules);
        game.tags = crate::tags::merge(&game.tags, &imported, rules.unwrap_or(&Default::default()));
    }
    game.available = game
        .executable
        .as_ref()
        .is_some_and(|p| Path::new(p).is_file());
}

pub fn is_excluded(game: &Game, settings: &Settings) -> bool {
    settings.excluded_games.contains(&game.id)
        || settings.excluded_entries.values().any(|excluded| {
            settings.excluded_games.contains(&excluded.id)
                && (excluded.folder.eq_ignore_ascii_case(&game.folder)
                    || excluded.executable.as_ref().is_some_and(|path| {
                        game.executable.as_ref().is_some_and(|other| path.eq_ignore_ascii_case(other))
                    }))
        })
}

/// Existing games stay excluded until explicitly restored. Missing games are
/// forgotten so that a later scan can discover them with a fresh setup record.
/// Only application records are changed; game files and backups are untouched.
pub fn remove_entry(settings: &mut Settings, list: &mut ScanResult, id: &str) -> Result<(), Message> {
    let mut game = list.games.iter().find(|game| game.id == id).cloned()
        .ok_or_else(|| Message::new("error.game_missing"))?;
    apply_edits(&mut game, settings);
    let present = match fs::metadata(&game.folder) {
        Ok(metadata) => metadata.is_dir(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(Message::detail("error.operation", error)),
    };
    if present {
        let edit = settings.game_edits.entry(id.into()).or_default();
        edit.setup_confirmed = true;
        edit.setup_pending = false;
        settings.excluded_entries.insert(id.into(), game);
        if !settings.excluded_games.iter().any(|excluded| excluded == id) {
            settings.excluded_games.push(id.into());
        }
    } else {
        forget_entry(settings, id);
    }
    list.games.retain(|game| game.id != id);
    Ok(())
}

/// Forget application records without touching game files or save backups.
pub fn forget_entry(settings: &mut Settings, id: &str) {
    settings.excluded_entries.remove(id);
    settings.excluded_games.retain(|excluded| excluded != id);
    settings.manual_games.retain(|game| game.id != id);
    settings.game_edits.remove(id);
    settings.cover_overrides.remove(id);
    settings.metadata_checks.remove(id);
    settings.metadata_attempt_versions.remove(id);
    settings.dialog_directories.retain(|key, _| !key.ends_with(&format!(":{id}")));
    for folder in &mut settings.collections {
        folder.games.remove(id);
    }
}

pub fn directory_missing(game: &Game) -> bool {
    match fs::metadata(&game.folder) {
        Ok(metadata) => !metadata.is_dir(),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Only a confirmed missing directory removes a record; access errors do not.
pub fn purge_missing(settings: &mut Settings, list: &mut ScanResult) -> std::collections::BTreeSet<String> {
    let missing: std::collections::BTreeSet<_> = list.games.iter()
        .chain(settings.manual_games.iter())
        .chain(settings.excluded_entries.values())
        .filter(|game| directory_missing(game))
        .map(|game| game.id.clone()).collect();
    for id in &missing { forget_entry(settings, id); }
    list.games.retain(|game| !missing.contains(&game.id));
    missing
}

/// Merge scanned candidates and retained records after missing directories are purged.
pub fn merge(mut fresh: ScanResult, old: &ScanResult, settings: &Settings) -> ScanResult {
    for game in old
        .games
        .iter()
        .chain(settings.manual_games.iter())
        .chain(settings.excluded_entries.values())
    {
        if !fresh.games.iter().any(|g| g.id == game.id)
            && (settings
                .library_folders
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&game.library))
                || settings.manual_games.iter().any(|g| g.id == game.id)
                || settings.excluded_entries.contains_key(&game.id))
        {
            fresh.games.push(game.clone());
        }
    }
    for manual in &settings.manual_games {
        let mut pinned = manual.clone();
        if let Some(cached) = old.games.iter().find(|g| g.id == manual.id) {
            pinned.cover.clone_from(&cached.cover);
            pinned.cover_artwork.clone_from(&cached.cover_artwork);
        }
        if let Some(entry) = fresh.games.iter_mut().find(|g| g.id == manual.id) {
            *entry = pinned;
        } else {
            fresh.games.push(pinned);
        }
    }
    for game in &mut fresh.games {
        if let Some(cached) = old.games.iter().find(|cached| cached.id == game.id) {
            game.playtime_seconds = cached.playtime_seconds;
            game.last_played = cached.last_played;
        }
        apply_edits(game, settings);
    }
    fresh.games.retain(|g| {
        g.executable.as_ref().is_some_and(|path| {
            Path::new(path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        }) && !is_excluded(g, settings)
    });
    fresh.games.sort_by_key(|g| g.title.to_lowercase());
    fresh
}

pub fn load(path: &Path) -> Result<ScanResult, Message> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut catalog: ScanResult =
                serde_json::from_slice(&bytes).map_err(|e| Message::detail("error.cache", e))?;
            catalog.games.retain(|game| {
                game.executable.as_ref().is_some_and(|path| {
                    Path::new(path)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                })
            });
            Ok(catalog)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ScanResult::default()),
        Err(e) => Err(Message::detail("error.cache", e)),
    }
}
pub fn save(path: &Path, catalog: &ScanResult) -> Result<(), Message> {
    let parent = path
        .parent()
        .ok_or_else(|| Message::new("error.config_path"))?;
    fs::create_dir_all(parent).map_err(|e| Message::detail("error.cache", e))?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| Message::detail("error.cache", e))?;
    serde_json::to_writer(&mut temp, catalog).map_err(|e| Message::detail("error.cache", e))?;
    temp.flush()
        .map_err(|e| Message::detail("error.cache", e))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| Message::detail("error.cache", e))?;
    temp.persist(path)
        .map_err(|e| Message::detail("error.cache", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_game_is_forgotten_when_its_directory_disappears_and_is_new_if_recreated() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        fs::write(folder.join("data.xp3"), b"data").unwrap();
        let mut settings = Settings { library_folders: vec![display_path(temp.path())], ..Default::default() };
        let mut list = crate::scanner::scan(&settings);
        let id = list.games[0].id.clone();
        settings.game_edits.entry(id.clone()).or_default().setup_confirmed = true;
        remove_entry(&mut settings, &mut list, &id).unwrap();
        assert!(purge_missing(&mut settings, &mut list).is_empty());
        assert!(settings.game_edits[&id].setup_confirmed);
        assert!(merge(crate::scanner::scan(&settings), &list, &settings).games.is_empty());
        fs::remove_dir_all(&folder).unwrap();
        assert!(purge_missing(&mut settings, &mut list).contains(&id));
        assert!(settings.excluded_games.is_empty());
        assert!(settings.excluded_entries.is_empty());
        assert!(!settings.game_edits.contains_key(&id));
        let mut settings: Settings = serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        fs::write(folder.join("data.xp3"), b"data").unwrap();
        let fresh = crate::scanner::scan(&settings);
        crate::game_setup::mark_new(&mut settings, &fresh.games, &Default::default());
        assert!(settings.game_edits[&id].setup_pending);
        assert!(!settings.game_edits[&id].setup_confirmed);
    }

    #[test]
    fn scan_cleanup_forgets_visible_manual_and_collection_records() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        let game = manual_game(&folder.join("game.exe"), "Game".into()).unwrap();
        let id = game.id.clone();
        let mut settings = Settings { manual_games: vec![game.clone()], ..Default::default() };
        settings.game_edits.entry(id.clone()).or_default().setup_confirmed = true;
        settings.cover_overrides.insert(id.clone(), "cover".into());
        crate::collections::edit(&mut settings, "create", "", "Favorites", None).unwrap();
        settings.collections[0].games.insert(id.clone());
        let mut list = ScanResult { games: vec![game], ..Default::default() };
        fs::remove_dir_all(&folder).unwrap();
        assert!(purge_missing(&mut settings, &mut list).contains(&id));
        assert!(list.games.is_empty());
        assert!(settings.manual_games.is_empty());
        assert!(settings.game_edits.is_empty());
        assert!(settings.cover_overrides.is_empty());
        assert!(settings.collections[0].games.is_empty());
        assert!(merge(ScanResult::default(), &list, &settings).games.is_empty());
    }

    #[test]
    fn missing_executable_does_not_forget_an_existing_game_directory() {
        let temp = tempfile::tempdir().unwrap();
        let game = manual_game(&temp.path().join("removed.exe"), "Game".into()).unwrap();
        let id = game.id.clone();
        let mut settings = Settings::default();
        let mut list = ScanResult { games: vec![game], ..Default::default() };
        assert!(purge_missing(&mut settings, &mut list).is_empty());
        remove_entry(&mut settings, &mut list, &id).unwrap();
        assert!(settings.excluded_entries.contains_key(&id));
    }

    #[test]
    fn existing_removal_stays_excluded_across_scans_and_restart() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        fs::write(folder.join("data.xp3"), b"assets").unwrap();
        let mut settings = Settings { library_folders: vec![display_path(temp.path())], ..Default::default() };
        let mut list = crate::scanner::scan(&settings);
        let id = list.games[0].id.clone();
        list.games[0].available = false; // Disk presence, not a stale UI flag, decides removal.
        settings.game_edits.entry(id.clone()).or_default().setup_confirmed = true;
        remove_entry(&mut settings, &mut list, &id).unwrap();
        assert!(folder.join("game.exe").is_file());
        assert!(settings.excluded_games.contains(&id));
        let mut reloaded: Settings = serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let fresh = crate::scanner::scan(&reloaded);
        assert!(!crate::game_setup::mark_new(&mut reloaded, &fresh.games, &Default::default()));
        assert!(merge(fresh, &list, &reloaded).games.is_empty());
        let manual_alias = manual_game(&folder.join("game.exe"), "Alias".into()).unwrap();
        assert!(is_excluded(&manual_alias, &reloaded));
    }

    #[test]
    fn forgotten_missing_game_is_new_on_full_and_startup_incremental_scans() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Library");
        let folder = root.join("Game");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        fs::write(folder.join("data.xp3"), b"assets").unwrap();
        let mut settings = Settings { library_folders: vec![display_path(&root)], ..Default::default() };
        let mut list = crate::scanner::scan(&settings);
        let game = list.games[0].clone();
        settings.game_edits.entry(game.id.clone()).or_default().setup_confirmed = true;
        settings.manual_games.push(game.clone());
        settings.cover_overrides.insert(game.id.clone(), "old-cover".into());
        settings.metadata_checks.insert(game.id.clone(), 1);
        settings.metadata_attempt_versions.insert(game.id.clone(), 1);
        crate::collections::edit(&mut settings, "create", "", "Favorites", None).unwrap();
        settings.collections[0].games.insert(game.id.clone());
        let moved = temp.path().join("Moved");
        fs::rename(&folder, &moved).unwrap();
        remove_entry(&mut settings, &mut list, &game.id).unwrap();
        assert!(list.games.is_empty());
        assert!(settings.excluded_games.is_empty());
        assert!(settings.excluded_entries.is_empty());
        assert!(settings.manual_games.is_empty());
        assert!(settings.game_edits.is_empty());
        assert!(settings.cover_overrides.is_empty());
        assert!(settings.metadata_checks.is_empty());
        assert!(settings.metadata_attempt_versions.is_empty());
        assert!(settings.collections[0].games.is_empty());
        fs::rename(&moved, &folder).unwrap();
        for incremental in [false, true] {
            let mut reloaded: Settings = serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
            let fresh = if incremental {
                crate::scanner::scan_incremental(&reloaded, &list, &Default::default(), &mut |_| {})
            } else { crate::scanner::scan(&reloaded) };
            assert_eq!(fresh.games.len(), 1);
            assert_eq!(fresh.games[0].id, game.id);
            assert!(crate::game_setup::mark_new(&mut reloaded, &fresh.games, &Default::default()));
            let restored = merge(fresh, &list, &reloaded);
            assert_eq!(restored.games.len(), 1);
            assert!(restored.games[0].metadata.is_none());
            assert!(reloaded.game_edits[&game.id].setup_pending);
            assert!(!reloaded.game_edits[&game.id].setup_confirmed);
        }
    }
    #[test]
    fn fresh_scan_and_manual_settings_preserve_the_latest_cached_play_record() {
        let mut game = manual_game(Path::new("F:/games/game.exe"), "Game".into()).unwrap();
        let settings = Settings {
            manual_games: vec![game.clone()],
            ..Default::default()
        };
        let fresh = ScanResult {
            games: vec![game.clone()],
            ..Default::default()
        };
        game.playtime_seconds = 4500;
        game.last_played = Some(1730000000);
        let old = ScanResult {
            games: vec![game.clone()],
            ..Default::default()
        };
        let merged = merge(fresh, &old, &settings);
        assert_eq!(
            crate::playtime::Record::from_game(&merged.games[0]),
            crate::playtime::Record::from_game(&game)
        );
    }

    #[test]
    fn legacy_catalog_without_play_records_loads_with_zero_hours_and_no_date() {
        let game = manual_game(Path::new("F:/games/game.exe"), "Game".into()).unwrap();
        let mut value = serde_json::to_value(game).unwrap();
        value.as_object_mut().unwrap().remove("playtime_seconds");
        value.as_object_mut().unwrap().remove("last_played");
        let restored: Game = serde_json::from_value(value).unwrap();
        assert_eq!(
            crate::playtime::Record::from_game(&restored),
            crate::playtime::Record::default()
        );
    }

    #[test]
    fn image_files_cannot_be_manually_added_or_restored_from_old_cache() {
        let directory = tempfile::tempdir().unwrap();
        assert!(manual_game(&directory.path().join("game.iso"), "Image".into()).is_err());
        let mut game = manual_game(&directory.path().join("game.exe"), "Game".into()).unwrap();
        game.executable = None;
        let cache = ScanResult {
            games: vec![game],
            ..Default::default()
        };
        let path = directory.path().join("catalog.json");
        save(&path, &cache).unwrap();
        assert!(load(&path).unwrap().games.is_empty());
    }
    #[test]
    fn manual_empty_fields_and_local_tags_survive_metadata_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let mut game = manual_game(&temp.path().join("game.exe"), "Local".into()).unwrap();
        let mut edit = crate::GameEdit {
            tags: vec!["LocalTag".into()],
            metadata: Some(crate::providers::Metadata {
                title: "Site title".into(),
                description: "Site story".into(),
                brand: "Site brand".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        edit.metadata_overrides
            .insert("description".into(), String::new());
        edit.metadata_overrides
            .insert("brand".into(), "Edited brand".into());
        let mut settings = Settings::default();
        settings.game_edits.insert(game.id.clone(), edit);
        apply_edits(&mut game, &settings);
        assert_eq!(game.metadata.as_ref().unwrap().description, "");
        assert_eq!(game.metadata.as_ref().unwrap().brand, "Edited brand");
        settings
            .game_edits
            .get_mut(&game.id)
            .unwrap()
            .metadata
            .as_mut()
            .unwrap()
            .description = "New story".into();
        apply_edits(&mut game, &settings);
        assert_eq!(game.metadata.as_ref().unwrap().description, "");
        assert_eq!(game.tags, vec!["LocalTag", "Edited brand"]);
    }
    #[test]
    fn cached_metadata_adds_all_credits_and_respects_local_tag_rules() {
        let mut game = manual_game(Path::new("F:/game.exe"), "Game".into()).unwrap();
        game.metadata = Some(crate::providers::Metadata {
            tags: vec!["ADV".into()],
            brand: "Studio".into(),
            developer: "studio".into(),
            writers: "Alice Smith、Bob".into(),
            music: "Composer".into(),
            ..Default::default()
        });
        apply_edits(&mut game, &Settings::default());
        assert_eq!(
            game.tags,
            vec!["ADV", "studio", "Alice Smith", "Bob", "Composer"]
        );
        let settings = Settings {
            game_edits: std::collections::BTreeMap::from([(
                game.id.clone(),
                crate::GameEdit {
                    tags: vec!["Local".into()],
                    tag_rules: std::collections::BTreeMap::from([
                        ("bob".into(), None),
                        ("studio".into(), Some("Renamed studio".into())),
                    ]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        apply_edits(&mut game, &settings);
        apply_edits(&mut game, &settings);
        assert_eq!(
            game.tags,
            vec!["Local", "ADV", "Renamed studio", "Alice Smith", "Composer"]
        );
    }
    #[test]
    fn edits_and_missing_entries_survive_scan_and_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut game = manual_game(&temp.path().join("game.exe"), "Original".into()).unwrap();
        game.library = "library".into();
        let old = ScanResult {
            games: vec![game.clone()],
            ..Default::default()
        };
        let mut settings = Settings::default();
        settings.library_folders.push("library".into());
        settings.game_edits.insert(
            game.id.clone(),
            crate::GameEdit {
                title: Some("Edited".into()),
                tags: vec!["Completed".into()],
                ..Default::default()
            },
        );
        let merged = merge(ScanResult::default(), &old, &settings);
        assert_eq!(merged.games[0].title, "Edited");
        assert!(!merged.games[0].available);
        let path = temp.path().join("catalog.json");
        save(&path, &merged).unwrap();
        assert_eq!(load(&path).unwrap().games, merged.games);
        settings.excluded_games.push(game.id);
        assert!(merge(merged, &old, &settings).games.is_empty());
    }
}
