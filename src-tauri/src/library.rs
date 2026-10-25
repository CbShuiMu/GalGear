//! Library scanning and membership commands.
use crate::{
    covers::prepare_cover, dialogs, state::AppState, state::find_game, state::persist_game,
    state::snapshot,
};
use galgear_core::{Bootstrap, Game, ScanResult, catalog, config::normalize_folder, scanner};
use galgear_i18n::{Message, tr};
use std::{path::PathBuf, sync::Arc, sync::atomic::AtomicBool, sync::atomic::Ordering};
use tauri::{Emitter, Manager, State};

#[derive(Default)]
pub(crate) struct ScanState {
    scanning: AtomicBool,
    cancel: Arc<AtomicBool>,
}

struct ScanGuard<'a>(&'a AtomicBool);
impl Drop for ScanGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
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
    let cache_dir = cover_dir.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut sent = 0;
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut progress = |data: &ScanResult| {
            if last.elapsed() < std::time::Duration::from_millis(120) {
                return;
            }
            let mut batch = data.games[sent..].to_vec();
            for game in &mut batch {
                let _ = prepare_cover(game, &cache_dir);
            }
            if let Ok(settings) = snapshot(&handle.state::<AppState>()) {
                batch.retain(|g| !settings.excluded_games.contains(&g.id));
                for game in &mut batch {
                    catalog::apply_edits(game, &settings);
                }
                if let Ok(mut current) = handle.state::<AppState>().catalog.lock() {
                    for game in &batch {
                        if !current.games.iter().any(|g| g.id == game.id) {
                            current.games.push(game.clone());
                        }
                    }
                }
            }
            let _ = handle.emit(
                "scan-progress",
                serde_json::json!({"games":batch,"folders_scanned":data.folders_scanned}),
            );
            sent = data.games.len();
            last = std::time::Instant::now();
        };
        let mut result = if full {
            scanner::scan_with(&settings, &cancel, &mut progress)
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
    if galgear_core::game_setup::mark_new(&mut next, &fresh.games, &known) {
        state.store.save(&next)?;
        *settings = next;
    }
    let catalog = catalog::merge(fresh, &current, &settings);
    catalog::save(&state.store.path.with_file_name("catalog.json"), &catalog)?;
    *current = catalog.clone();
    Ok(catalog)
}

#[tauri::command]
pub(crate) async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, Message> {
    Ok(Bootstrap {
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
        if !next
            .library_folders
            .iter()
            .any(|p| p.to_lowercase() == path.to_lowercase())
        {
            next.library_folders.push(path);
        }
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
        next.excluded_games.retain(|id| id != &game.id);
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
pub(crate) fn exclude_game(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.excluded_entries.insert(id.clone(), game);
    if !next.excluded_games.contains(&id) {
        next.excluded_games.push(id.clone());
    }
    state.store.save(&next)?;
    *settings = next;
    drop(settings);
    let mut list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    list.games.retain(|g| g.id != id);
    catalog::save(&state.store.path.with_file_name("catalog.json"), &list)
}
