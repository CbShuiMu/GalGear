//! Library scanning and membership commands.
use crate::{
    covers::prepare_cover, dialogs, state::AppState, state::find_game, state::persist_game,
    state::snapshot,
};
use galgear_core::{Bootstrap, Game, ScanResult, catalog, config::normalize_folder, scanner};
use galgear_i18n::{Message, tr};
use std::{path::PathBuf, sync::Arc, sync::atomic::AtomicBool, sync::atomic::Ordering};
use tauri::{Emitter, State};

#[derive(Default)]
pub(crate) struct ScanState {
    scanning: AtomicBool,
    cancel: Arc<AtomicBool>,
    forgotten: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl ScanState {
    pub(crate) fn is_scanning(&self) -> bool {
        self.scanning.load(Ordering::SeqCst)
    }
    pub(crate) fn forget(&self, ids: &std::collections::BTreeSet<String>) {
        if let Ok(mut forgotten) = self.forgotten.lock() {
            forgotten.extend(ids.iter().cloned());
        }
    }
    pub(crate) fn retain_current(&self, games: &mut Vec<Game>) {
        if let Ok(forgotten) = self.forgotten.lock() {
            games.retain(|game| !forgotten.contains(&game.id));
        }
    }
}

struct ScanGuard<'a>(&'a AtomicBool);
impl Drop for ScanGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub(crate) fn reorder_games(ids: Vec<String>, before: Option<String>, state: State<'_, AppState>) -> Result<galgear_core::Settings, Message> {
    let catalog = state.catalog.lock().map_err(|_| Message::new("error.catalog_state"))?.clone();
    crate::preferences::update(&state, |settings| {
        if settings.sort_by != "custom" { return Err(Message::new("error.sort")); }
        galgear_core::library_order::reorder(settings, &catalog.games, &ids, before.as_deref());
        Ok(())
    })
}

pub(crate) async fn refresh(
    state: &AppState,
    app: &tauri::AppHandle,
    full: bool,
) -> Result<ScanResult, Message> {
    if state.scan.scanning.swap(true, Ordering::SeqCst) {
        return Err(Message::new("error.scan_running"));
    }
    let _guard = ScanGuard(&state.scan.scanning);
    state
        .scan
        .forgotten
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .clear();
    state.scan.cancel.store(false, Ordering::SeqCst);
    let settings = snapshot(state)?;
    let previous = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .clone();
    let previous_covers: std::collections::BTreeMap<_, _> = previous
        .games
        .iter()
        .map(|g| (g.id.clone(), g.cover.clone()))
        .collect();
    let known = previous.games.iter().map(|game| game.id.clone()).collect();
    let cancel = state.scan.cancel.clone();
    let handle = app.clone();
    let cover_dir = state.store.path.with_file_name("covers").join("rendered");
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut progress = |data: &ScanResult| {
            if last.elapsed() < std::time::Duration::from_millis(120) && data.units_completed != data.units_total { return; }
            let _ = handle.emit("scan-progress", serde_json::json!({"folders_scanned":data.folders_scanned,"completed":data.units_completed,"total":data.units_total}));
            last = std::time::Instant::now();
        };
        let mut result = if full {
            scanner::scan_refresh(&settings, &previous, &cancel, &mut progress)
        } else {
            scanner::scan_incremental(&settings, &previous, &cancel, &mut progress)
        };
        for game in &mut result.games {
            if let Err(e) = prepare_cover(game, &cover_dir) {
                result.warnings.push(e);
            }
        }
        result
    })
    .await;
    let mut fresh = result.map_err(|e| Message::detail("error.scan", e))?;
    {
        let current = state
            .catalog
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?;
        for game in &mut fresh.games {
            if let Some(updated) = current.games.iter().find(|g| g.id == game.id)
                && previous_covers
                    .get(&game.id)
                    .is_none_or(|before| before != &updated.cover)
            {
                game.cover.clone_from(&updated.cover);
                game.cover_artwork.clone_from(&updated.cover_artwork);
            }
        }
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut current = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let mut next = settings.clone();
    state.scan.retain_current(&mut fresh.games);
    state.scan.retain_current(&mut current.games);
    let retained = current.clone();
    galgear_core::game_setup::mark_new(&mut next, &fresh.games, &known);
    let catalog = catalog::merge(fresh, &retained, &next);
    if next != *settings {
        state.store.save(&next)?;
        *settings = next;
    }
    catalog::save(&state.store.path.with_file_name("catalog.json"), &catalog)?;
    *current = catalog.clone();
    Ok(catalog)
}

/// 启动时修复旧版空白扫描副本。此时尚无运行任务，备份与目录不会并发写入。
pub(crate) fn repair_scan_duplicates(
    config_path: &std::path::Path,
    settings: &galgear_core::Settings,
    catalog: &mut ScanResult,
) -> Result<(), Message> {
    let candidates = catalog::empty_scan_duplicates(&catalog.games, settings);
    let mut removed = std::collections::BTreeSet::new();
    for id in candidates {
        let Some(game) = catalog.games.iter().find(|game| game.id == id) else {
            continue;
        };
        match duplicate_has_backups(config_path, settings, game) {
            Ok(false) => {
                removed.insert(id);
            }
            Ok(true) => {}
            Err(error) => catalog.warnings.push(error),
        }
    }
    if !removed.is_empty() {
        catalog.games.retain(|game| !removed.contains(&game.id));
        // 仅更新原子目录缓存，旧配置和备份索引保留，避免跨文件迁移半途失败。
        catalog::save(&config_path.with_file_name("catalog.json"), catalog)?;
    }
    Ok(())
}

fn duplicate_has_backups(
    config_path: &std::path::Path,
    settings: &galgear_core::Settings,
    game: &Game,
) -> Result<bool, Message> {
    let mut roots = vec![
        config_path.with_file_name("save-backups"),
        galgear_core::snapshots::game_data(std::path::Path::new(&game.folder)),
    ];
    for root in std::iter::once(&settings.backup_folder)
        .chain(&settings.backup_legacy_folders)
        .filter(|root| !root.is_empty())
    {
        let path = PathBuf::from(root);
        // 配置过的外部备份盘离线时，无法证明副本没有历史。
        if !path.is_dir() {
            return Ok(true);
        }
        roots.push(path);
    }
    for root in roots {
        if galgear_core::snapshots::has_game_data(&root, &game.id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[tauri::command]
pub(crate) async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, Message> {
    Ok(Bootstrap {
        update_complete: std::env::args().any(|arg| arg == "--galgear-update-complete"),
        ai_available: crate::skill_package::available(&state),
        settings: snapshot(&state)?,
        catalog: state
            .catalog
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?
            .clone(),
        config_path: state.store.path.to_string_lossy().into_owned(),
        version: env!("CARGO_PKG_VERSION").into(),
        cg_engines: Vec::new(),
    })
}

#[tauri::command]
pub(crate) async fn scan_library(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    full: Option<bool>,
) -> Result<ScanResult, Message> {
    refresh(&state, &app, full.unwrap_or(false)).await
}

#[tauri::command]
pub(crate) fn cancel_scan(state: State<'_, AppState>) {
    state.scan.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub(crate) async fn choose_library_folder(
    state: State<'_, AppState>,
) -> Result<Option<String>, Message> {
    let language = snapshot(&state)?.language;
    let start = dialogs::start(
        &state,
        "library",
        &snapshot(&state)?
            .library_folders
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
    )?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .set_title(tr(language, "dialog.library"))
            .pick_folder()
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))?;
    if let Some(path) = &path {
        dialogs::remember(&state, "library", path)?;
    }
    Ok(path.map(|p| galgear_core::config::display_path(&p)))
}

#[tauri::command]
pub(crate) async fn add_library_folder(
    path: String,
    state: State<'_, AppState>,
) -> Result<Bootstrap, Message> {
    let path = tauri::async_runtime::spawn_blocking(move || normalize_folder(&path))
        .await
        .map_err(|e| Message::detail("error.operation", e))??;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        catalog::add_library_folder(&mut next, path);
        state.store.save(&next)?;
        *settings = next;
    }
    bootstrap(state).await
}

#[tauri::command]
pub(crate) async fn remove_library_folder(
    path: String,
    state: State<'_, AppState>,
) -> Result<Bootstrap, Message> {
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        next.library_folders.retain(|p| p != &path);
        state.store.save(&next)?;
        *settings = next;
    }
    bootstrap(state).await
}

#[tauri::command]
pub(crate) async fn add_manual_game(state: State<'_, AppState>) -> Result<Option<Game>, Message> {
    let settings = snapshot(&state)?;
    let context = settings
        .dialog_directories
        .get("manual-game")
        .map(PathBuf::from)
        .into_iter()
        .chain(settings.library_folders.iter().map(PathBuf::from))
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, "manual-game", &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .add_filter("Game", &["exe"])
            .pick_file()
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))?;
    let Some(path) = path else {
        return Ok(None);
    };
    dialogs::remember(&state, "manual-game", &path)?;
    let title = (if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        path.parent().unwrap_or(&path)
    } else {
        &path
    })
    .file_stem()
    .unwrap_or_default()
    .to_string_lossy()
    .into_owned();
    let game = catalog::manual_game(&path, title)?;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        catalog::allow_import(&mut next, &game);
        if !next.manual_games.iter().any(|g| g.id == game.id) {
            next.manual_games.push(game.clone());
        }
        galgear_core::game_setup::mark_new(
            &mut next,
            std::slice::from_ref(&game),
            &Default::default(),
        );
        state.store.save(&next)?;
        *settings = next;
    }
    let mut list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    if !list.games.iter().any(|g| g.id == game.id) {
        list.games.push(game.clone());
    }
    catalog::save(&state.store.path.with_file_name("catalog.json"), &list)?;
    Ok(Some(game))
}

#[tauri::command]
pub(crate) async fn relocate_game(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Game>, Message> {
    let mut game = find_game(&state, &id)?;
    let role = format!("relocate:{id}");
    let settings = snapshot(&state)?;
    let context = std::iter::once(PathBuf::from(&game.folder))
        .chain(settings.library_folders.iter().map(PathBuf::from))
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, &role, &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .add_filter("Game", &["exe"])
            .pick_file()
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))?;
    let Some(path) = path else {
        return Ok(None);
    };
    dialogs::remember(&state, &role, &path)?;
    let mut replacement = catalog::manual_game(&path, game.title.clone())?;
    replacement.id.clone_from(&game.id);
    replacement.cover.clone_from(&game.cover);
    replacement.tags.clone_from(&game.tags);
    replacement.play_status = game.play_status;
    replacement.metadata.clone_from(&game.metadata);
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.manual_games.retain(|g| g.id != id);
    next.manual_games.push(replacement.clone());
    let edit = next.game_edits.entry(id).or_default();
    edit.executable.clone_from(&replacement.executable);
    edit.working_directory = Some(replacement.folder.clone());
    state.store.save(&next)?;
    *settings = next;
    drop(settings);
    game = replacement;
    persist_game(&state, game).map(Some)
}

#[tauri::command]
pub(crate) fn exclude_game(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::collections::BatchResult, Message> {
    crate::collections::change_games(&state, "exclude", vec![id], None)
}

#[cfg(test)]
mod tests {
    use crate::state::AppState;
    use galgear_core::{
        GameEdit, Settings, catalog,
        config::{ConfigStore, display_path},
        cover_art, playtime, scanner, snapshots,
    };
    use std::fs;

    #[test]
    fn startup_repairs_only_empty_scan_duplicates_and_keeps_original_backup_identity() {
        for protected in [
            "empty",
            "cover_cache",
            "cover_override",
            "playtime",
            "backup",
            "offline",
            "configured",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let folder = temp.path().join("Game");
            let data = temp.path().join("config");
            fs::create_dir(&folder).unwrap();
            fs::write(folder.join("AdvHD.exe"), b"fixture").unwrap();
            fs::write(folder.join("data.arc"), b"fixture").unwrap();
            let mut settings = Settings {
                library_folders: vec![display_path(temp.path())],
                ..Default::default()
            };
            let mut catalog = scanner::scan(&settings);
            let duplicate = catalog.games[0].id.clone();
            let manual = catalog::manual_game(&folder.join("AdvHD.exe"), "Game".into()).unwrap();
            settings.manual_games.push(manual.clone());
            settings.game_edits.insert(
                duplicate.clone(),
                GameEdit {
                    setup_pending: true,
                    ..Default::default()
                },
            );
            if protected == "configured" {
                settings.game_edits.get_mut(&duplicate).unwrap().title = Some("My title".into());
            }
            if matches!(protected, "cover_cache" | "cover_override") {
                let cache = data.join("covers").join("rendered");
                fs::create_dir_all(&cache).unwrap();
                let artwork = cover_art::Artwork {
                    pipeline: cover_art::PIPELINE.into(),
                    source: "a".repeat(64),
                    cover: "b".repeat(64),
                    background: "c".repeat(64),
                };
                for key in [&artwork.source, &artwork.cover, &artwork.background] {
                    fs::write(cover_art::image_path(&cache, key), b"cached image").unwrap();
                }
                artwork.apply(&mut catalog.games[0]);
                if protected == "cover_override" {
                    settings
                        .cover_overrides
                        .insert(duplicate.clone(), display_path(&folder.join("cover.jpg")));
                }
            }
            if protected == "offline" {
                settings
                    .backup_legacy_folders
                    .push(display_path(&temp.path().join("offline")));
            }
            catalog.games.push(manual.clone());
            ConfigStore::new(data.join("settings.json"))
                .save(&settings)
                .unwrap();
            catalog::save(&data.join("catalog.json"), &catalog).unwrap();
            if protected == "playtime" {
                let records = [(
                    duplicate.clone(),
                    playtime::Record {
                        playtime_seconds: 7,
                        last_played: Some(10),
                    },
                )]
                .into_iter()
                .collect();
                playtime::save(&data.join("playtime.json"), &records).unwrap();
            }
            let backup = data.join("save-backups");
            snapshots::register_game(&backup, &manual.id, "Game").unwrap();
            snapshots::register_game(&backup, &duplicate, "Game").unwrap();
            // 未完成的备份也必须保护其身份。
            if protected == "backup" {
                fs::create_dir_all(backup.join(&duplicate)).unwrap();
                fs::write(backup.join(&duplicate).join("partial.zip"), b"backup").unwrap();
            }
            let expected_count = if matches!(protected, "empty" | "cover_cache") {
                1
            } else {
                2
            };
            for _ in 0..2 {
                let state = AppState::load(data.clone()).unwrap();
                let current = state.catalog.lock().unwrap();
                assert_eq!(current.games.len(), expected_count, "{protected}");
                assert!(current.games.iter().any(|game| game.id == manual.id));
                assert_eq!(
                    snapshots::registered_games(&backup).unwrap()[&manual.id],
                    "Game"
                );
            }
            assert!(folder.join("AdvHD.exe").is_file());
        }
    }
}
