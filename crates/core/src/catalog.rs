use crate::{Game, ScanResult, Settings, config::display_path};
use galgear_i18n::Message;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) mod identity;
pub mod title;
pub use identity::empty_scan_duplicates;

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
        aliases: Vec::new(),
        tags: Vec::new(),
        available: path.is_file(),
        metadata: None,
        scan_stamp: String::new(),
        metadata_warning: None,
    })
}

pub fn apply_edits(game: &mut Game, settings: &Settings) {
    apply_saved_edits(game, settings);
    game.available = game
        .executable
        .as_ref()
        .is_some_and(|p| Path::new(p).is_file());
}

/// 仅应用已保存的资料与标签，不访问游戏目录，保留缓存中的可用状态。
/// 用于启动首屏，路径可用性由后续后台检查更新。
pub fn apply_saved_edits(game: &mut Game, settings: &Settings) {
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
}

pub fn is_excluded(game: &Game, settings: &Settings) -> bool {
    settings.excluded_games.contains(&game.id)
        || settings
            .excluded_paths
            .iter()
            .any(|path| path.eq_ignore_ascii_case(&game.folder))
        || settings.excluded_entries.values().any(|excluded| {
            settings.excluded_games.contains(&excluded.id)
                && (excluded.folder.eq_ignore_ascii_case(&game.folder)
                    || excluded.executable.as_ref().is_some_and(|path| {
                        game.executable
                            .as_ref()
                            .is_some_and(|other| path.eq_ignore_ascii_case(other))
                    }))
        })
}

/// Register a normalized library directory and reconsider games removed from it.
/// Re-adding an existing directory leaves its exclusions unchanged.
pub fn add_library_folder(settings: &mut Settings, path: String) {
    add_library_folders(settings, vec![path]);
}

/// 批量注册已规范化的库目录，保留选择顺序，返回实际新增数。
/// 重复目录不改变排除记录，父子目录分别保留为独立的库。
pub fn add_library_folders(settings: &mut Settings, paths: Vec<String>) -> usize {
    let mut known: std::collections::HashSet<_> = settings
        .library_folders
        .iter()
        .map(|path| path.to_ascii_lowercase())
        .collect();
    let mut roots = std::collections::HashSet::new();
    let previous_len = settings.library_folders.len();
    for path in paths {
        if known.insert(path.to_ascii_lowercase()) {
            roots.insert(std::path::PathBuf::from(path.to_lowercase()));
            settings.library_folders.push(path);
        }
    }
    let added = settings.library_folders.len() - previous_len;
    if added == 0 {
        return 0;
    }
    // 沿路径祖先查找整批新增根目录，避免每个库都遍历所有排除项。
    let belongs_to_added = |folder: &str| {
        Path::new(&folder.to_lowercase())
            .ancestors()
            .any(|ancestor| roots.contains(ancestor))
    };
    settings
        .excluded_paths
        .retain(|excluded| !belongs_to_added(excluded));
    let excluded: std::collections::HashSet<_> = settings.excluded_games.iter().collect();
    let restored: std::collections::BTreeSet<_> = settings
        .excluded_entries
        .values()
        .filter(|game| excluded.contains(&game.id) && belongs_to_added(&game.folder))
        .map(|game| game.id.clone())
        .collect();
    settings.excluded_games.retain(|id| !restored.contains(id));
    for id in restored {
        let edit = settings.game_edits.entry(id).or_default();
        edit.setup_confirmed = false;
        edit.setup_pending = true;
    }
    added
}

/// 移除仅属于已移除库的目录条目，返回去重后的待清理游戏目录。
/// `settings` 必须是移除该库后的配置。手动条目及其他库覆盖的目录继续保留。
pub fn detach_library_games(
    list: &mut ScanResult,
    settings: &Settings,
    removed: &str,
) -> BTreeSet<PathBuf> {
    let path_key = |path: &str| {
        #[cfg(windows)]
        let path = path.to_lowercase();
        PathBuf::from(path)
    };
    let removed = path_key(removed);
    let roots: HashMap<_, _> = settings
        .library_folders
        .iter()
        .map(|path| (path_key(path), path))
        .collect();
    let manual_folders: HashSet<_> = settings
        .manual_games
        .iter()
        .map(|game| path_key(&game.folder))
        .collect();
    let manual_ids: HashSet<_> = settings
        .manual_games
        .iter()
        .map(|game| game.id.as_str())
        .collect();
    let mut detached = BTreeSet::new();
    list.games.retain_mut(|game| {
        let folder = path_key(&game.folder);
        if !folder.starts_with(&removed) {
            return true;
        }
        if let Some(root) = folder.ancestors().find_map(|ancestor| roots.get(ancestor)) {
            if path_key(&game.library) == removed {
                game.library.clone_from(root);
            }
            return true;
        }
        if manual_ids.contains(game.id.as_str()) {
            return true;
        }
        if !manual_folders.contains(&folder) {
            detached.insert(PathBuf::from(&game.folder));
        }
        false
    });
    detached
}

/// 永久清除应用条目，并记录最小扫描排除信息；游戏文件和备份独立保留。
pub fn remove_entry(
    settings: &mut Settings,
    list: &mut ScanResult,
    id: &str,
) -> Result<(), Message> {
    let game = list
        .games
        .iter()
        .chain(settings.excluded_entries.values())
        .find(|game| game.id == id)
        .ok_or_else(|| Message::new("error.game_missing"))?;
    let folder = game.folder.clone();
    forget_entry(settings, id);
    if !settings
        .excluded_paths
        .iter()
        .any(|path| path.eq_ignore_ascii_case(&folder))
    {
        settings.excluded_paths.push(folder);
    }
    list.games.retain(|game| game.id != id);
    Ok(())
}

/// 显式重新导入允许重新发现该位置，旧条目资料不会恢复。
pub fn allow_import(settings: &mut Settings, game: &Game) {
    settings.excluded_games.retain(|id| id != &game.id);
    settings
        .excluded_paths
        .retain(|path| !path.eq_ignore_ascii_case(&game.folder));
}

/// Forget application records without touching game files or save backups.
pub fn forget_entry(settings: &mut Settings, id: &str) {
    settings.excluded_entries.remove(id);
    settings.excluded_games.retain(|excluded| excluded != id);
    settings.manual_games.retain(|game| game.id != id);
    settings.game_edits.remove(id);
    settings.game_fonts.remove(id);
    settings.cover_overrides.remove(id);
    settings.metadata_checks.remove(id);
    settings.metadata_attempt_versions.remove(id);
    settings
        .dialog_directories
        .retain(|key, _| !key.ends_with(&format!(":{id}")));
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

/// 重新检查路径可用性，离线目录仍保留全部游戏资料。
pub fn refresh_availability(settings: &Settings, list: &mut ScanResult) {
    for game in &mut list.games {
        apply_edits(game, settings);
    }
}

/// Merge scanned candidates with retained records, including unavailable games.
pub fn merge(mut fresh: ScanResult, old: &ScanResult, settings: &Settings) -> ScanResult {
    // 与原先 find 的语义一致，重复 ID 始终取第一次出现的条目。
    let mut old_by_id = HashMap::new();
    for game in &old.games {
        old_by_id.entry(game.id.as_str()).or_insert(game);
    }
    let mut positions = HashMap::new();
    for (index, game) in fresh.games.iter().enumerate() {
        positions.entry(game.id.clone()).or_insert(index);
    }
    let manual_ids: HashSet<_> = settings
        .manual_games
        .iter()
        .map(|game| game.id.as_str())
        .collect();
    let libraries: HashSet<_> = settings
        .library_folders
        .iter()
        .map(|path| path.to_ascii_lowercase())
        .collect();
    for game in old
        .games
        .iter()
        .chain(settings.manual_games.iter())
        .chain(settings.excluded_entries.values())
    {
        if !positions.contains_key(&game.id)
            && (libraries.contains(&game.library.to_ascii_lowercase())
                || manual_ids.contains(game.id.as_str())
                || settings.excluded_entries.contains_key(&game.id))
        {
            positions.insert(game.id.clone(), fresh.games.len());
            fresh.games.push(game.clone());
        }
    }
    for manual in &settings.manual_games {
        let mut pinned = manual.clone();
        if let Some(cached) = old_by_id.get(manual.id.as_str()) {
            pinned.cover.clone_from(&cached.cover);
            pinned.cover_artwork.clone_from(&cached.cover_artwork);
        }
        if let Some(&index) = positions.get(&manual.id) {
            fresh.games[index] = pinned;
        } else {
            positions.insert(pinned.id.clone(), fresh.games.len());
            fresh.games.push(pinned);
        }
    }
    for game in &mut fresh.games {
        if let Some(cached) = old_by_id.get(game.id.as_str()) {
            game.aliases.clone_from(&cached.aliases);
            game.playtime_seconds = cached.playtime_seconds;
            game.last_played = cached.last_played;
            if game.metadata.is_none() {
                game.metadata.clone_from(&cached.metadata);
            }
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
    fn indexed_merge_preserves_first_cached_record_manual_precedence_and_saved_edits() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("game.exe");
        fs::write(&exe, b"fixture").unwrap();
        let mut cached = manual_game(&exe, "Cached".into()).unwrap();
        cached.library = display_path(temp.path());
        cached.playtime_seconds = 120;
        cached.aliases = vec!["Alias".into()];
        cached.cover = Some("cached-cover".into());
        let mut duplicate = cached.clone();
        duplicate.playtime_seconds = 999;
        duplicate.cover = Some("wrong-cover".into());
        let old = ScanResult {
            games: vec![cached.clone(), duplicate],
            ..Default::default()
        };
        let mut manual = cached.clone();
        manual.title = "Manual".into();
        manual.cover = None;
        let mut settings = Settings {
            library_folders: vec![cached.library.to_ascii_uppercase()],
            manual_games: vec![manual],
            ..Default::default()
        };
        settings
            .game_edits
            .entry(cached.id.clone())
            .or_default()
            .title = Some("Saved".into());
        let merged = merge(ScanResult::default(), &old, &settings);
        assert_eq!(merged.games.len(), 1);
        assert_eq!(merged.games[0].title, "Saved");
        assert_eq!(merged.games[0].playtime_seconds, 120);
        assert_eq!(merged.games[0].aliases, ["Alias"]);
        assert_eq!(merged.games[0].cover.as_deref(), Some("cached-cover"));
    }

    #[test]
    fn batch_library_restore_preserves_order_boundaries_and_existing_exclusions() {
        let mut settings = Settings {
            library_folders: vec!["C:/Existing".into()],
            excluded_paths: vec![
                "C:/Existing/Game".into(),
                "C:/New/Game".into(),
                "C:/New-other/Game".into(),
            ],
            ..Default::default()
        };
        for folder in ["C:/New", "C:/New/Child", "C:/Existing", "C:/New-other"] {
            let game = manual_game(&Path::new(folder).join("game.exe"), folder.into()).unwrap();
            settings.excluded_games.push(game.id.clone());
            settings.excluded_entries.insert(game.id.clone(), game);
        }
        let added = add_library_folders(
            &mut settings,
            ["c:/existing", "C:/New/Child", "C:/New", "c:/new"]
                .map(String::from)
                .to_vec(),
        );
        assert_eq!(added, 2);
        assert_eq!(
            settings.library_folders,
            ["C:/Existing", "C:/New/Child", "C:/New"]
        );
        assert_eq!(
            settings.excluded_paths,
            ["C:/Existing/Game", "C:/New-other/Game"]
        );
        assert_eq!(settings.excluded_games.len(), 2);
        assert!(settings.excluded_games.iter().all(|id| {
            matches!(
                settings.excluded_entries[id].folder.as_str(),
                "C:/Existing" | "C:/New-other"
            )
        }));
        assert_eq!(settings.game_edits.len(), 2);
        assert!(
            settings
                .game_edits
                .values()
                .all(|edit| edit.setup_pending && !edit.setup_confirmed)
        );
        let previous = settings.clone();
        add_library_folders(&mut settings, previous.library_folders.clone());
        assert_eq!(settings, previous);
    }

    #[test]
    #[ignore = "批量目录性能测试，使用 --release --ignored --nocapture"]
    fn batch_library_registration_scaling() {
        for count in [1_000, 10_000, 50_000] {
            let paths: Vec<_> = (0..count).map(|i| format!("C:/Library/{i}")).collect();
            let mut settings = Settings {
                excluded_paths: paths.iter().map(|path| format!("{path}/Game")).collect(),
                ..Default::default()
            };
            for path in &paths {
                let game = manual_game(&Path::new(path).join("game.exe"), path.clone()).unwrap();
                settings.excluded_games.push(game.id.clone());
                settings.excluded_entries.insert(game.id.clone(), game);
            }
            settings.excluded_paths.push("C:/Other/Game".into());
            let selected = paths.iter().chain(&paths).cloned().collect();
            let start = std::time::Instant::now();
            assert_eq!(add_library_folders(&mut settings, selected), count);
            eprintln!(
                "{count} roots + {count} legacy exclusions + duplicates: {:?}",
                start.elapsed()
            );
            assert_eq!(settings.library_folders, paths);
            assert_eq!(settings.excluded_paths, ["C:/Other/Game"]);
            assert!(settings.excluded_games.is_empty());
            assert_eq!(settings.game_edits.len(), count);
        }
    }

    #[test]
    fn readding_removed_library_after_restart_requires_setup_for_its_excluded_games() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Library");
        let other_root = temp.path().join("Library-other");
        for root in [&root, &other_root] {
            let folder = root.join("Game");
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("game.exe"), b"game").unwrap();
            fs::write(folder.join("data.xp3"), b"data").unwrap();
        }
        let root = display_path(&root);
        let mut settings = Settings {
            library_folders: vec![root.clone(), display_path(&other_root)],
            ..Default::default()
        };
        let mut list = crate::scanner::scan(&settings);
        let id = list
            .games
            .iter()
            .find(|game| game.library == root)
            .unwrap()
            .id
            .clone();
        let other_id = list
            .games
            .iter()
            .find(|game| game.id != id)
            .unwrap()
            .id
            .clone();
        let game = list.games.iter_mut().find(|game| game.id == id).unwrap();
        game.playtime_seconds = 3600;
        let edit = settings.game_edits.entry(id.clone()).or_default();
        edit.setup_confirmed = true;
        edit.arguments = vec!["--custom".into()];
        edit.metadata = Some(crate::providers::Metadata {
            url: "https://vndb.org/v1".into(),
            ..Default::default()
        });
        remove_entry(&mut settings, &mut list, &id).unwrap();
        remove_entry(&mut settings, &mut list, &other_id).unwrap();
        // Restart with the persisted exclusions, then remove and re-add the root.
        let mut settings: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert!(
            merge(crate::scanner::scan(&settings), &list, &settings)
                .games
                .is_empty()
        );
        settings.library_folders.retain(|path| path != &root);
        add_library_folder(&mut settings, root);
        let mut settings: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let fresh = crate::scanner::scan(&settings);
        crate::game_setup::mark_new(&mut settings, &fresh.games, &Default::default());
        let restored = merge(fresh, &list, &settings);

        assert_eq!(
            restored
                .games
                .iter()
                .map(|game| &game.id)
                .collect::<Vec<_>>(),
            vec![&id]
        );
        assert!(settings.game_edits[&id].setup_pending);
        assert!(settings.game_edits[&id].arguments.is_empty());
        assert!(settings.excluded_entries.is_empty());
        assert!(!restored.games.iter().any(|game| game.id == other_id));
        assert_eq!(restored.games[0].playtime_seconds, 0);
    }

    #[test]
    fn adding_an_already_registered_library_preserves_exclusions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("game.exe");
        fs::write(&path, b"game").unwrap();
        let game = manual_game(&path, "Game".into()).unwrap();
        let root = display_path(temp.path());
        let mut settings = Settings {
            library_folders: vec![root.clone()],
            ..Default::default()
        };
        let id = game.id.clone();
        let mut list = ScanResult {
            games: vec![game],
            ..Default::default()
        };
        remove_entry(&mut settings, &mut list, &id).unwrap();
        let previous = settings.clone();

        add_library_folder(&mut settings, root.to_uppercase());

        assert_eq!(settings, previous);
    }

    #[test]
    fn permanent_removal_keeps_files_and_requires_explicit_import() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        fs::write(folder.join("data.xp3"), b"assets").unwrap();
        let mut settings = Settings {
            library_folders: vec![display_path(temp.path())],
            ..Default::default()
        };
        let mut list = crate::scanner::scan(&settings);
        let game = list.games[0].clone();
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .setup_confirmed = true;
        remove_entry(&mut settings, &mut list, &game.id).unwrap();
        assert!(settings.game_edits.is_empty());
        assert!(settings.excluded_entries.is_empty());
        assert!(folder.join("game.exe").is_file());
        assert!(
            merge(crate::scanner::scan(&settings), &list, &settings)
                .games
                .is_empty()
        );
        allow_import(&mut settings, &game);
        let fresh = crate::scanner::scan(&settings);
        assert!(crate::game_setup::mark_new(
            &mut settings,
            &fresh.games,
            &Default::default()
        ));
        assert!(settings.game_edits[&game.id].setup_pending);
    }

    #[test]
    fn offline_scan_preserves_manual_records_and_returns_without_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        let game = manual_game(&folder.join("game.exe"), "Game".into()).unwrap();
        let id = game.id.clone();
        let mut settings = Settings {
            manual_games: vec![game.clone()],
            ..Default::default()
        };
        settings
            .game_edits
            .entry(id.clone())
            .or_default()
            .setup_confirmed = true;
        settings.cover_overrides.insert(id.clone(), "cover".into());
        crate::collections::edit(&mut settings, "create", "", "Favorites", None).unwrap();
        settings.collections[0].games.insert(id.clone());
        let mut list = ScanResult {
            games: vec![game],
            ..Default::default()
        };
        let detached = temp.path().join("detached");
        fs::rename(&folder, &detached).unwrap();
        refresh_availability(&settings, &mut list);
        assert!(!list.games[0].available);
        assert!(settings.game_edits[&id].setup_confirmed);
        assert!(settings.collections[0].games.contains(&id));
        fs::rename(&detached, &folder).unwrap();
        let reloaded = merge(ScanResult::default(), &list, &settings);
        assert_eq!(reloaded.games.len(), 1);
        assert_eq!(reloaded.games[0].id, id);
        assert!(reloaded.games[0].available);
    }

    #[test]
    fn existing_removal_stays_excluded_across_scans_and_restart() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("Game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("game.exe"), b"game").unwrap();
        fs::write(folder.join("data.xp3"), b"assets").unwrap();
        let mut settings = Settings {
            library_folders: vec![display_path(temp.path())],
            ..Default::default()
        };
        let mut list = crate::scanner::scan(&settings);
        let id = list.games[0].id.clone();
        list.games[0].available = false; // Disk presence, not a stale UI flag, decides removal.
        settings
            .game_edits
            .entry(id.clone())
            .or_default()
            .setup_confirmed = true;
        remove_entry(&mut settings, &mut list, &id).unwrap();
        assert!(folder.join("game.exe").is_file());
        assert!(settings.excluded_paths.contains(&display_path(&folder)));
        let mut reloaded: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let fresh = crate::scanner::scan(&reloaded);
        assert!(!crate::game_setup::mark_new(
            &mut reloaded,
            &fresh.games,
            &Default::default()
        ));
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
        let mut settings = Settings {
            library_folders: vec![display_path(&root)],
            ..Default::default()
        };
        let mut list = crate::scanner::scan(&settings);
        let game = list.games[0].clone();
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .setup_confirmed = true;
        settings.manual_games.push(game.clone());
        settings
            .cover_overrides
            .insert(game.id.clone(), "old-cover".into());
        settings.metadata_checks.insert(game.id.clone(), 1);
        settings
            .metadata_attempt_versions
            .insert(game.id.clone(), 1);
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
        allow_import(&mut settings, &game);
        for incremental in [false, true] {
            let mut reloaded: Settings =
                serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
            let fresh = if incremental {
                crate::scanner::scan_incremental(&reloaded, &list, &Default::default(), &mut |_| {})
            } else {
                crate::scanner::scan(&reloaded)
            };
            assert_eq!(fresh.games.len(), 1);
            assert_eq!(fresh.games[0].id, game.id);
            assert!(crate::game_setup::mark_new(
                &mut reloaded,
                &fresh.games,
                &Default::default()
            ));
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
        game.aliases = vec!["星之终途".into()];
        game.playtime_seconds = 4500;
        game.last_played = Some(1730000000);
        let old = ScanResult {
            games: vec![game.clone()],
            ..Default::default()
        };
        let merged = merge(fresh, &old, &settings);
        assert_eq!(merged.games[0].aliases, game.aliases);
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
        assert_eq!(game.tags, vec!["LocalTag"]);
    }
    #[test]
    fn cached_metadata_imports_only_tags_and_respects_local_tag_rules() {
        let mut game = manual_game(Path::new("F:/game.exe"), "Game".into()).unwrap();
        game.metadata = Some(crate::providers::Metadata {
            tags: vec!["ADV".into(), "School".into(), "Romance".into()],
            brand: "Studio".into(),
            developer: "studio".into(),
            writers: "Alice Smith、Bob".into(),
            music: "Composer".into(),
            ..Default::default()
        });
        apply_edits(&mut game, &Settings::default());
        assert_eq!(game.tags, vec!["ADV", "School", "Romance"]);
        let settings = Settings {
            game_edits: std::collections::BTreeMap::from([(
                game.id.clone(),
                crate::GameEdit {
                    tags: vec!["Local".into()],
                    tag_rules: std::collections::BTreeMap::from([
                        ("school".into(), None),
                        ("romance".into(), Some("恋爱".into())),
                        ("studio".into(), Some("Renamed studio".into())),
                    ]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        apply_edits(&mut game, &settings);
        apply_edits(&mut game, &settings);
        assert_eq!(game.tags, vec!["Local", "ADV", "恋爱"]);
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
