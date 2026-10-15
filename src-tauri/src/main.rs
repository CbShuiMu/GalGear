#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod collections;
mod data_sources;
mod dialogs;
mod fan_cache;
mod features;

use galgear_core::{
    Bootstrap, Game, GameEdit, ScanResult, Settings, catalog,
    config::{ConfigStore, normalize_folder},
    credentials::{self, FanCredentials},
    fan_account::{Account, DownloadPlan, DownloadPreview, Session, Transfer},
    patches, providers, runtime, saves, scanner, snapshots, tags,
};
use galgear_i18n::{Language, Message, tr};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{Emitter, Manager, State};

struct AppState {
    store: ConfigStore,
    settings: Mutex<Settings>,
    catalog: Mutex<ScanResult>,
    scanning: AtomicBool,
    cancel: Arc<AtomicBool>,
    running: Mutex<std::collections::BTreeMap<String, std::process::Child>>,
    patch_plan: Mutex<Option<patches::Plan>>,
    fan_plan: Mutex<Option<(DownloadPlan, FanCredentials)>>,
    downloading: AtomicBool,
    download_cancel: Arc<AtomicBool>,
    downloaded: Mutex<Option<String>>,
    fan_refresh_running: AtomicBool,
    fan_cache: Mutex<fan_cache::Cache>,
    fan_refresh_lock: tokio::sync::Mutex<()>,
    save_plan: Mutex<Option<snapshots::Preview>>,
    writing: AtomicBool,
}

struct ScanGuard<'a>(&'a AtomicBool);
impl Drop for ScanGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn snapshot(state: &AppState) -> Result<Settings, Message> {
    Ok(state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?
        .clone())
}

async fn refresh(
    state: &AppState,
    app: &tauri::AppHandle,
    full: bool,
) -> Result<ScanResult, Message> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err(Message::new("error.scan_running"));
    }
    let _guard = ScanGuard(&state.scanning);
    state.cancel.store(false, Ordering::SeqCst);
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
    let cancel = state.cancel.clone();
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
    let settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut current = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let catalog = catalog::merge(fresh, &current, &settings);
    catalog::save(&state.store.path.with_file_name("catalog.json"), &catalog)?;
    *current = catalog.clone();
    Ok(catalog)
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, Message> {
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
async fn scan_library(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    full: Option<bool>,
) -> Result<ScanResult, Message> {
    refresh(&state, &app, full.unwrap_or(false)).await
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn choose_library_folder(state: State<'_, AppState>) -> Result<Option<String>, Message> {
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
async fn add_library_folder(
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
async fn remove_library_folder(
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
fn save_preferences(
    card_size: u32,
    sort_by: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !matches!(sort_by.as_str(), "title" | "recent") {
        return Err(Message::new("error.sort"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.card_size = card_size.clamp(160, 280);
    next.sort_by = sort_by;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
fn set_language(language: String, state: State<'_, AppState>) -> Result<Settings, Message> {
    let language = Language::from_code(&language).ok_or_else(|| Message::new("error.language"))?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.language = language;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

fn find_game(state: &AppState, id: &str) -> Result<Game, Message> {
    state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .games
        .iter()
        .find(|game| game.id == id)
        .cloned()
        .ok_or_else(|| Message::new("error.game_missing"))
}

#[tauri::command]
async fn choose_cover(id: String, state: State<'_, AppState>) -> Result<Option<Game>, Message> {
    let mut game = find_game(&state, &id)?;
    let language = snapshot(&state)?.language;
    let role = format!("cover:{id}");
    let settings = snapshot(&state)?;
    let context = settings
        .dialog_directories
        .get(&role)
        .map(PathBuf::from)
        .into_iter()
        .chain([PathBuf::from(&game.folder)])
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, &role, &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .set_title(tr(language, "dialog.cover"))
            .add_filter(
                tr(language, "dialog.images"),
                &["png", "jpg", "jpeg", "webp", "bmp"],
            )
            .pick_file()
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))?;
    let Some(path) = path else {
        return Ok(None);
    };
    dialogs::remember(&state, &role, &path)?;
    let cache_path = state
        .store
        .path
        .parent()
        .ok_or_else(|| Message::new("error.config_path"))?
        .join("covers")
        .join(format!("{id}.jpg"));
    let image_path = path.clone();
    let destination = cache_path.clone();
    let data = tauri::async_runtime::spawn_blocking(move || {
        scanner::cache_poster(&image_path, &destination)
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))??;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        next.cover_overrides
            .insert(id.clone(), cache_path.to_string_lossy().into_owned());
        state.store.save(&next)?;
        *settings = next;
    }
    game.cover = Some(data);
    persist_game(&state, game).map(Some)
}

#[tauri::command]
fn open_game_folder(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    if !PathBuf::from(&game.folder).is_dir() {
        return Err(Message::new("error.game_folder"));
    }
    std::process::Command::new("explorer.exe")
        .arg(&game.folder)
        .spawn()
        .map_err(|e| Message::detail("error.open_folder", e))?;
    Ok(())
}

#[tauri::command]
fn launch_game(
    id: String,
    mode: Option<String>,
    confirmed_unknown: Option<bool>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), Message> {
    let _guard = features::begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let status = runtime::detect(game.executable.as_deref());
    if status == runtime::Status::Running {
        return Err(Message::new("error.already_running"));
    }
    if status == runtime::Status::Unknown && !confirmed_unknown.unwrap_or(false) {
        return Err(Message::new("runtime.confirm"));
    }
    let executable = game
        .executable
        .ok_or_else(|| Message::new("error.exe_missing"))?;
    if !PathBuf::from(&executable).is_file() {
        return Err(Message::new("error.exe_missing"));
    }
    let settings = snapshot(&state)?;
    let edit = settings.game_edits.get(&id).cloned().unwrap_or_default();
    if edit.upscale {
        if !PathBuf::from(&settings.magpie).is_file() {
            return Err(Message::new("tools.magpie_missing"));
        }
        if runtime::detect(Some(&settings.magpie)) != runtime::Status::Running {
            return Err(Message::new("tools.magpie_start"));
        }
    }
    let mut running = state
        .running
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    if let Some(child) = running.get_mut(&id)
        && child
            .try_wait()
            .map_err(|e| Message::detail("error.launch", e))?
            .is_none()
    {
        return Err(Message::new("error.already_running"));
    }
    let locale = mode.as_deref().unwrap_or(&edit.launch_mode) == "locale";
    let mut command = if locale {
        if !PathBuf::from(&settings.locale_emulator).is_file() {
            return Err(Message::new("tools.locale_missing"));
        }
        let mut command = std::process::Command::new(&settings.locale_emulator);
        let profiles = runtime::locale_profiles(std::path::Path::new(&settings.locale_emulator))
            .map_err(|e| Message::detail("tools.error", e))?;
        let profile = profiles
            .iter()
            .find(|p| {
                if edit.locale_profile.is_empty() {
                    p.locale == "ja-JP" && !p.admin
                } else {
                    p.guid == edit.locale_profile
                }
            })
            .ok_or_else(|| Message::new("tools.locale_profile"))?;
        command.args(["-runas", &profile.guid, &executable]);
        command
    } else {
        std::process::Command::new(edit.launcher.as_deref().unwrap_or(&executable))
    };
    if !locale && edit.launcher.is_some() {
        command.arg(&executable);
    }
    let child = command
        .args(&edit.arguments)
        .current_dir(edit.working_directory.as_deref().unwrap_or(&game.folder))
        .spawn()
        .map_err(|e| Message::detail("error.launch", e))?;
    running.insert(id, child);
    if edit.upscale {
        features::schedule_upscale(
            app.clone(),
            executable,
            settings.magpie_hotkey.clone(),
            settings.launch_behavior == "exit",
        );
    }
    if settings.launch_behavior == "minimize"
        && let Some(window) = app.get_webview_window("main")
    {
        let _ = window.minimize();
    }
    if settings.launch_behavior == "exit" && !edit.upscale {
        app.exit(0);
    }
    Ok(())
}

fn prepare_cover(game: &mut Game, cache: &std::path::Path) -> Result<(), Message> {
    use base64::Engine;
    use sha2::Digest;
    use std::io::Write;
    if let Some(value) = &game.cover
        && let Some(encoded) = value.strip_prefix("data:image/jpeg;base64,")
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Message::detail("error.image", e))?;
        std::fs::create_dir_all(cache).map_err(|e| Message::detail("error.image", e))?;
        let key = format!("{:x}", sha2::Sha256::digest(&bytes));
        let path = cache.join(format!("{key}.jpg"));
        if !path.exists() {
            let mut file = tempfile::NamedTempFile::new_in(cache)
                .map_err(|e| Message::detail("error.image", e))?;
            file.write_all(&bytes)
                .map_err(|e| Message::detail("error.image", e))?;
            file.as_file()
                .sync_all()
                .map_err(|e| Message::detail("error.image", e))?;
            match file.persist_noclobber(&path) {
                Ok(_) => {}
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Message::detail("error.image", e)),
            }
        }
        game.cover = Some(format!("http://galgear-cover.localhost/{key}"));
    }
    Ok(())
}

fn persist_game(state: &AppState, mut game: Game) -> Result<Game, Message> {
    prepare_cover(
        &mut game,
        &state.store.path.with_file_name("covers").join("rendered"),
    )?;
    let mut list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    if let Some(entry) = list.games.iter_mut().find(|g| g.id == game.id) {
        *entry = game.clone();
    }
    catalog::save(&state.store.path.with_file_name("catalog.json"), &list)?;
    Ok(game)
}

#[tauri::command]
async fn detect_saves(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, Message> {
    let game = find_game(&state, &id)?;
    let edit = snapshot(&state)?.game_edits.get(&id).cloned();
    let report =
        tauri::async_runtime::spawn_blocking(move || saves::discover(&game, edit.as_ref()))
            .await
            .map_err(|e| Message::detail("error.operation", e))?;
    Ok(serde_json::json!({ "id": id, "report": report }))
}

#[tauri::command]
async fn choose_save_folder(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, Message> {
    let mut game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let role = format!("save:{id}");
    let save_game = game.clone();
    let save_edit = settings.game_edits.get(&id).cloned();
    let report = tauri::async_runtime::spawn_blocking(move || {
        saves::discover(&save_game, save_edit.as_ref())
    })
    .await
    .map_err(|_| Message::new("error.operation"))?;
    let context = settings
        .game_edits
        .get(&id)
        .and_then(|e| e.save_path.as_ref())
        .map(PathBuf::from)
        .into_iter()
        .chain(settings.dialog_directories.get(&role).map(PathBuf::from))
        .chain(
            report
                .locations
                .iter()
                .filter(|l| l.exists)
                .map(|l| PathBuf::from(&l.path)),
        )
        .chain([PathBuf::from(&game.folder)])
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, &role, &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || dialogs::dialog(start).pick_folder())
        .await
        .map_err(|e| Message::detail("error.operation", e))?;
    let Some(path) = path else {
        return Ok(None);
    };
    dialogs::remember(&state, &role, &path)?;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        let edit = next.game_edits.entry(id.clone()).or_default();
        let path = galgear_core::config::display_path(&path);
        if !edit.save_paths.contains(&path) {
            edit.save_paths.push(path);
        }
        state.store.save(&next)?;
        catalog::apply_edits(&mut game, &next);
        *settings = next;
    }
    persist_game(&state, game)?;
    detect_saves(id, state).await.map(Some)
}

#[tauri::command]
async fn open_save_folder(
    id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    let edit = snapshot(&state)?.game_edits.get(&id).cloned();
    let check = path.clone();
    let allowed = tauri::async_runtime::spawn_blocking(move || {
        saves::discover(&game, edit.as_ref())
            .locations
            .iter()
            .any(|l| l.exists && l.path == check)
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))?;
    if !allowed {
        return Err(Message::new("error.game_folder"));
    }
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map_err(|e| Message::detail("error.open_folder", e))?;
    Ok(())
}

#[tauri::command]
fn edit_game(
    id: String,
    mut edit: GameEdit,
    locale_profile: Option<String>,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    edit.tags = tags::normalize(edit.tags);
    if edit.title.as_ref().is_some_and(|t| t.trim().is_empty()) {
        return Err(Message::new("error.title_required"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    // Metadata and chosen saves are changed only through their own controls.
    edit.tags.clone_from(&game.tags);
    if edit.title.as_ref() == Some(&game.title) {
        edit.title = next.game_edits.get(&id).and_then(|e| e.title.clone());
    }
    if let Some(previous) = next.game_edits.get(&id) {
        edit.metadata.clone_from(&previous.metadata);
        edit.save_path.clone_from(&previous.save_path);
        edit.save_paths.clone_from(&previous.save_paths);
        edit.metadata_overrides
            .clone_from(&previous.metadata_overrides);
        edit.tag_rules.clone_from(&previous.tag_rules);
        edit.launch_mode.clone_from(&previous.launch_mode);
        edit.locale_profile.clone_from(&previous.locale_profile);
        edit.play_status = previous.play_status;
        edit.auto_backup = previous.auto_backup;
        edit.description.clone_from(&previous.description);
    }
    if let Some(profile) = locale_profile {
        edit.locale_profile = profile;
    }
    tags::remember(&game.tags, &edit.tags, &mut edit.tag_rules);
    next.game_edits.insert(id, edit);
    state.store.save(&next)?;
    catalog::apply_edits(&mut game, &next);
    *settings = next;
    drop(settings);
    persist_game(&state, game)
}

#[tauri::command]
async fn add_manual_game(state: State<'_, AppState>) -> Result<Option<Game>, Message> {
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
async fn relocate_game(id: String, state: State<'_, AppState>) -> Result<Option<Game>, Message> {
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
fn exclude_game(id: String, state: State<'_, AppState>) -> Result<(), Message> {
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

#[tauri::command]
fn set_launch_behavior(behavior: String, state: State<'_, AppState>) -> Result<Settings, Message> {
    if !["keep", "minimize", "exit"].contains(&behavior.as_str()) {
        return Err(Message::new("error.operation"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.launch_behavior = behavior;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
async fn search_metadata(
    query: String,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<providers::Resource>, Message> {
    let settings = snapshot(&state)?;
    let source = source.unwrap_or(settings.metadata_provider);
    let credentials = data_sources::credentials_for(&state, &source)?;
    tauri::async_runtime::spawn_blocking(move || {
        galgear_core::data_sources::search(&source, &settings.fan_domain, &query, &credentials)
    })
    .await
    .map_err(|e| Message::detail("error.network", e))?
}

#[tauri::command]
async fn select_metadata(
    id: String,
    url: String,
    expected_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    find_game(&state, &id)?;
    let domain = snapshot(&state)?.fan_domain;
    let (source, _) = galgear_core::data_sources::source_for_url(&url)?;
    let credentials = data_sources::credentials_for(&state, source)?;
    let metadata = tauri::async_runtime::spawn_blocking(move || {
        galgear_core::data_sources::metadata(&domain, &url, &credentials)
    })
    .await
    .map_err(|e| Message::detail("error.network", e))??;
    let game = find_game(&state, &id)?;
    let cover = if game.cover.is_none() && metadata.cover_url.is_some() {
        let game = game.clone();
        let metadata = metadata.clone();
        let cache = state.store.path.with_file_name("covers");
        Some(
            tauri::async_runtime::spawn_blocking(move || {
                providers::save_cover(&game, &metadata, &cache)
            })
            .await
            .map_err(|e| Message::detail("error.network", e))?,
        )
    } else {
        None
    };
    let mut game = find_game(&state, &id)?;
    if expected_url.as_deref().is_some_and(|expected| {
        game.metadata
            .as_ref()
            .map(|m| m.url.as_str())
            .unwrap_or_default()
            != expected
    }) {
        return Ok(game);
    }
    game.metadata_warning = match &cover {
        Some(Err(error)) => Some(error.clone()),
        Some(Ok((_, _, warning))) => warning.clone(),
        None => None,
    };
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        let edit = next.game_edits.entry(id).or_default();
        edit.tags = tags::merge(&game.tags, &metadata.tags, &edit.tag_rules);
        edit.metadata = Some(metadata);
        if game.cover.is_none()
            && let Some(Ok((data, path, _warning))) = &cover
        {
            game.cover = Some(data.clone());
            next.cover_overrides.insert(game.id.clone(), path.clone());
        }
        state.store.save(&next)?;
        catalog::apply_edits(&mut game, &next);
        *settings = next;
    }
    persist_game(&state, game)
}

#[tauri::command]
async fn enrich_library(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), Message> {
    let games = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .games
        .clone();
    let settings = snapshot(&state)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Message::detail("error.operation", e))?
        .as_secs();
    let games: Vec<_> = games
        .into_iter()
        .filter(|g| {
            g.metadata
                .as_ref()
                .is_none_or(|m| m.schema_version < providers::METADATA_SCHEMA_VERSION)
                && (settings
                    .metadata_attempt_versions
                    .get(&g.id)
                    .copied()
                    .unwrap_or_default()
                    < providers::METADATA_SCHEMA_VERSION
                    || settings
                        .metadata_checks
                        .get(&g.id)
                        .is_none_or(|previous| now.saturating_sub(*previous) > 86400))
        })
        .take(1000)
        .collect();
    let total = games.len();
    for (index, game) in games.into_iter().enumerate() {
        let _ = app.emit(
            "enrichment-progress",
            serde_json::json!({"title":game.title,"current":index+1,"total":total}),
        );
        {
            let mut current = state
                .settings
                .lock()
                .map_err(|_| Message::new("error.config_state"))?;
            let mut next = current.clone();
            next.metadata_checks.insert(game.id.clone(), now);
            next.metadata_attempt_versions
                .insert(game.id.clone(), providers::METADATA_SCHEMA_VERSION);
            state.store.save(&next)?;
            *current = next;
        }
        if let Some(metadata) = &game.metadata
            && !metadata.url.is_empty()
        {
            match select_metadata(
                game.id.clone(),
                metadata.url.clone(),
                Some(metadata.url.clone()),
                state.clone(),
            )
            .await
            {
                Ok(updated) => {
                    let _ = app.emit("game-enriched", serde_json::json!({"game":updated}));
                }
                Err(error) => {
                    let _ = app.emit(
                        "game-enriched",
                        serde_json::json!({"id":game.id,"warning":error}),
                    );
                }
            }
            continue;
        }
        let query = PathBuf::from(&game.folder)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let domain = settings.fan_domain.clone();
        let source = settings.metadata_provider.clone();
        let credentials = data_sources::credentials_for(&state, &source)?;
        let results = tauri::async_runtime::spawn_blocking(move || {
            galgear_core::data_sources::search(&source, &domain, &query, &credentials)
        })
        .await
        .map_err(|e| Message::detail("error.network", e))?;
        {
            let mut current = state
                .settings
                .lock()
                .map_err(|_| Message::new("error.config_state"))?;
            let mut next = current.clone();
            next.metadata_checks.insert(game.id.clone(), now);
            state.store.save(&next)?;
            *current = next;
        }
        match results {
            Ok(candidates) => {
                if candidates.len() == 1 {
                    match select_metadata(
                        game.id.clone(),
                        candidates[0].url.clone(),
                        Some(String::new()),
                        state.clone(),
                    )
                    .await
                    {
                        Ok(updated) => {
                            let _ = app.emit("game-enriched", serde_json::json!({"game":updated}));
                        }
                        Err(e) => {
                            let _ = app.emit(
                                "game-enriched",
                                serde_json::json!({"id":game.id,"warning":e}),
                            );
                        }
                    }
                } else {
                    let _ = app.emit(
                        "game-enriched",
                        serde_json::json!({"id":game.id,"candidates":candidates}),
                    );
                }
            }
            Err(e) => {
                let _ = app.emit(
                    "game-enriched",
                    serde_json::json!({"id":game.id,"warning":e}),
                );
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn open_resource(url: String) -> Result<(), Message> {
    let parsed = reqwest::Url::parse(&url).map_err(|e| Message::detail("error.site_url", e))?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Message::new("error.site_url"));
    }
    open::that_detached(parsed.as_str()).map_err(|e| Message::detail("error.network", e))
}

fn patch_data(state: &AppState) -> PathBuf {
    state.store.path.with_file_name("patches")
}

#[tauri::command]
fn load_fan_credentials(state: State<'_, AppState>) -> Result<FanCredentials, Message> {
    credentials::load(&credentials::target(&state.store.path))
}

#[tauri::command]
fn save_fan_credentials(
    value: FanCredentials,
    state: State<'_, AppState>,
) -> Result<FanCredentials, Message> {
    if state.downloading.load(Ordering::SeqCst) {
        return Err(Message::new("download.busy"));
    }
    if state.fan_refresh_running.load(Ordering::SeqCst) {
        return Err(Message::new("auth.busy"));
    }
    credentials::save(&credentials::target(&state.store.path), &value)?;
    let scope = fan_cache::scope(&snapshot(&state)?.fan_domain, &value);
    state
        .fan_cache
        .lock()
        .map_err(|_| Message::new("error.config_state"))?
        .reset(scope);
    *state
        .fan_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = None;
    Ok(value)
}

#[tauri::command]
fn clear_fan_credentials(state: State<'_, AppState>) -> Result<FanCredentials, Message> {
    if state.downloading.load(Ordering::SeqCst) {
        return Err(Message::new("download.busy"));
    }
    if state.fan_refresh_running.load(Ordering::SeqCst) {
        return Err(Message::new("auth.busy"));
    }
    credentials::clear(&credentials::target(&state.store.path))?;
    let empty = FanCredentials {
        mode: "csrf".into(),
        ..Default::default()
    };
    let scope = fan_cache::scope(&snapshot(&state)?.fan_domain, &empty);
    state
        .fan_cache
        .lock()
        .map_err(|_| Message::new("error.config_state"))?
        .reset(scope);
    *state
        .fan_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = None;
    Ok(FanCredentials {
        mode: "csrf".into(),
        ..Default::default()
    })
}

#[tauri::command]
fn get_fan_status_cache(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<fan_cache::Snapshot, Message> {
    let settings = snapshot(&state)?;
    let value = load_fan_credentials(state.clone())?;
    let mut cache = state
        .fan_cache
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    cache.ensure_scope(fan_cache::scope(&settings.fan_domain, &value));
    let initial = cache.initial();
    let result = cache.snapshot();
    drop(cache);
    if initial {
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let _ = refresh_fan_status_inner(&state, &app, false).await;
        });
    }
    Ok(result)
}

async fn refresh_fan_status_inner(
    state: &AppState,
    app: &tauri::AppHandle,
    force: bool,
) -> Result<fan_cache::Snapshot, Message> {
    let _serial = state.fan_refresh_lock.lock().await;
    if state.downloading.load(Ordering::SeqCst) {
        return Ok(state
            .fan_cache
            .lock()
            .map_err(|_| Message::new("error.config_state"))?
            .snapshot());
    }
    let settings = snapshot(state)?;
    let value = credentials::load(&credentials::target(&state.store.path))?;
    let (ticket, previous) = {
        let mut cache = state
            .fan_cache
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        cache.ensure_scope(fan_cache::scope(&settings.fan_domain, &value));
        let Some(ticket) = cache.begin(fan_cache::now(), force) else {
            return Ok(cache.snapshot());
        };
        (ticket, cache.snapshot())
    };
    state.fan_refresh_running.store(true, Ordering::SeqCst);
    let _ = app.emit("fan-status-updated", &previous);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let now = fan_cache::now();
        if value.cookie.is_empty() && value.token.is_empty() {
            return Ok(fan_cache::Snapshot {
                account: Account {
                    status: "unauthenticated".into(),
                    reason: Some(Message::new("auth.need_cookie")),
                    checked_at: now,
                    ..Default::default()
                },
                updated_at: now,
                ..Default::default()
            });
        }
        let session = Session::new(&settings.fan_domain, value)?;
        let account = session.check()?;
        Ok(fan_cache::Snapshot {
            account,
            updated_at: fan_cache::now(),
            ..Default::default()
        })
    })
    .await
    .unwrap_or_else(|_| Err(Message::new("auth.network")));
    state.fan_refresh_running.store(false, Ordering::SeqCst);
    let (committed, status) = {
        let mut cache = state
            .fan_cache
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let committed = cache.finish(ticket, result);
        (committed, cache.snapshot())
    };
    if committed {
        let _ = app.emit("fan-status-updated", &status);
    }
    Ok(status)
}

#[tauri::command]
async fn refresh_fan_status(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<fan_cache::Snapshot, Message> {
    refresh_fan_status_inner(&state, &app, true).await
}

fn start_fan_status_scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let state = app.state::<AppState>();
                let now = fan_cache::now();
                let due = state
                    .fan_cache
                    .lock()
                    .map(|cache| cache.due(now))
                    .unwrap_or(false);
                if due
                    && !state.downloading.load(Ordering::SeqCst)
                    && !state.fan_refresh_running.load(Ordering::SeqCst)
                {
                    let _ = refresh_fan_status_inner(&state, &app, false).await;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(fan_cache::next_minute(
                fan_cache::now(),
            )))
            .await;
        }
    });
}

fn download_directory(state: &AppState, settings: &Settings) -> PathBuf {
    if settings.download_folder.is_empty() {
        state.store.path.with_file_name("downloads")
    } else {
        PathBuf::from(&settings.download_folder)
    }
}

#[tauri::command]
async fn load_game_icons(
    state: State<'_, AppState>,
) -> Result<std::collections::BTreeMap<String, String>, Message> {
    let games = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .games
        .clone();
    let cache = state.store.path.with_file_name("icons");
    tauri::async_runtime::spawn_blocking(move || {
        games
            .into_iter()
            .filter(|g| g.available)
            .filter_map(|g| {
                let path = galgear_core::launcher_icons::cached_icon(
                    std::path::Path::new(g.executable.as_deref()?),
                    &cache,
                )?;
                Some((
                    g.id,
                    format!(
                        "http://galgear-icon.localhost/{}",
                        path.file_stem()?.to_string_lossy()
                    ),
                ))
            })
            .collect()
    })
    .await
    .map_err(|_| Message::new("error.image"))
}

#[tauri::command]
async fn choose_download_folder(state: State<'_, AppState>) -> Result<Settings, Message> {
    let directory = download_directory(&state, &snapshot(&state)?);
    let start = dialogs::start(&state, "downloads", &[directory])?;
    let path = tauri::async_runtime::spawn_blocking(move || dialogs::dialog(start).pick_folder())
        .await
        .map_err(|_| Message::new("download.write"))?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    if let Some(path) = path {
        let mut next = settings.clone();
        next.download_folder = galgear_core::config::display_path(&path);
        state.store.save(&next)?;
        *settings = next;
    }
    Ok(settings.clone())
}

#[tauri::command]
fn open_download_folder(state: State<'_, AppState>) -> Result<(), Message> {
    let path = download_directory(&state, &snapshot(&state)?);
    std::fs::create_dir_all(&path).map_err(|_| Message::new("download.write"))?;
    open::that_detached(path).map_err(|_| Message::new("download.write"))
}

#[tauri::command]
async fn inspect_fan_resource(
    url: String,
    state: State<'_, AppState>,
) -> Result<DownloadPreview, Message> {
    let domain = snapshot(&state)?.fan_domain;
    let value = credentials::load(&credentials::target(&state.store.path))?;
    let bound = value.clone();
    let mut plan =
        tauri::async_runtime::spawn_blocking(move || Session::new(&domain, value)?.inspect(&url))
            .await
            .map_err(|_| Message::new("auth.network"))??;
    plan.preview.ticket = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();
    let preview = plan.preview.clone();
    *state
        .fan_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some((plan, bound));
    Ok(preview)
}

#[tauri::command]
async fn start_fan_download(
    ticket: String,
    confirm_price: Option<u64>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Transfer, Message> {
    if state.downloading.swap(true, Ordering::SeqCst) {
        return Err(Message::new("download.busy"));
    }
    state.download_cancel.store(false, Ordering::SeqCst);
    let result = async {
        let (plan, bound) = state
            .fan_plan
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?
            .take()
            .ok_or_else(|| Message::new("download.expired"))?;
        if plan.preview.ticket != ticket {
            return Err(Message::new("download.expired"));
        }
        let settings = snapshot(&state)?;
        let credentials = credentials::load(&credentials::target(&state.store.path))?;
        if bound != credentials {
            return Err(Message::new("download.expired"));
        }
        if plan.preview.requires_purchase
            && (!plan.preview.can_purchase
                || confirm_price.is_none()
                || confirm_price != plan.preview.price)
        {
            return Err(Message::new("download.confirm_required"));
        }
        let directory = download_directory(&state, &settings);
        let audit_directory = state.store.path.with_file_name("purchases");
        let cancel = state.download_cancel.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use sha2::{Digest, Sha256};
            let session = Session::new(&settings.fan_domain, credentials)?;
            let mut fresh = session.inspect(&plan.preview.source_url)?;
            if fresh.preview.requires_purchase {
                if !plan.preview.requires_purchase || fresh.preview.price != confirm_price {
                    return Err(Message::new("download.price_changed"));
                }
                if cancel.load(Ordering::Relaxed) {
                    return Err(Message::new("download.cancelled"));
                }
                // A pending marker survives crashes/timeouts; never retry an uncertain charge automatically.
                std::fs::create_dir_all(&audit_directory)
                    .map_err(|_| Message::new("download.write"))?;
                let marker = audit_directory.join(format!(
                    "{:x}.pending",
                    Sha256::digest(fresh.preview.source_url.as_bytes())
                ));
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)
                    .map_err(|e| {
                        Message::new(if e.kind() == std::io::ErrorKind::AlreadyExists {
                            "download.purchase_uncertain"
                        } else {
                            "download.write"
                        })
                    })?;
                file.write_all(
                    format!(
                        "{}\n{}\n",
                        fresh.preview.source_url,
                        confirm_price.unwrap_or_default()
                    )
                    .as_bytes(),
                )
                .and_then(|_| file.sync_all())
                .map_err(|_| Message::new("download.write"))?;
                let _ = app.emit(
                    "fan-download-progress",
                    Transfer {
                        phase: "purchasing".into(),
                        title: fresh.preview.title.clone(),
                        ..Default::default()
                    },
                );
                fresh = match session.purchase(&fresh) {
                    Ok(plan) => plan,
                    Err(error) => {
                        drop(file);
                        if error.key != "download.purchase_uncertain" {
                            let _ = std::fs::remove_file(&marker);
                        }
                        return Err(error);
                    }
                };
                drop(file);
                let _ = std::fs::remove_file(marker);
            }
            if fresh.preview.free && fresh.preview.can_download {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Message::new("download.cancelled"));
                }
                fresh = session.acquire_free(&fresh)?;
            }
            let mut final_transfer = Transfer::default();
            session.download(&fresh, &directory, &cancel, &mut |progress| {
                final_transfer = progress.clone();
                let _ = app.emit("fan-download-progress", progress);
            })?;
            Ok(final_transfer)
        })
        .await
        .map_err(|_| Message::new("download.network"))?
    }
    .await;
    state.downloading.store(false, Ordering::SeqCst);
    if let Ok(transfer) = &result {
        *state
            .downloaded
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))? = transfer.path.clone();
    }
    result
}

#[tauri::command]
fn cancel_fan_download(state: State<'_, AppState>) {
    state.download_cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn preview_downloaded_patch(
    id: String,
    target: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<patches::Preview>, Message> {
    let game = find_game(&state, &id)?;
    if !game.available || game.executable.is_none() {
        return Err(Message::new("error.exe_missing"));
    }
    let path = state
        .downloaded
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .clone()
        .ok_or_else(|| Message::new("download.expired"))?;
    let folder = PathBuf::from(
        target
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(game.folder),
    );
    let plan = tauri::async_runtime::spawn_blocking(move || {
        patches::preview(&PathBuf::from(path), &folder, id)
    })
    .await
    .map_err(|_| Message::new("error.patch"))??;
    let preview = plan.preview.clone();
    *state
        .patch_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some(plan);
    Ok(Some(preview))
}

#[tauri::command]
async fn choose_patch_target(
    id: String,
    target: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<String>, Message> {
    let game = find_game(&state, &id)?;
    let role = format!("patch-target:{id}");
    let context = target
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .into_iter()
        .chain([PathBuf::from(game.folder)])
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, &role, &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || dialogs::dialog(start).pick_folder())
        .await
        .map_err(|_| Message::new("error.patch"))?;
    if let Some(path) = &path {
        dialogs::remember(&state, &role, path)?;
    }
    Ok(path.map(|p| galgear_core::config::display_path(&p)))
}

#[tauri::command]
async fn preview_patch(
    id: String,
    target: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<patches::Preview>, Message> {
    let game = find_game(&state, &id)?;
    if game.executable.is_none() || !game.available {
        return Err(Message::new("error.exe_missing"));
    }
    let settings = snapshot(&state)?;
    let directory = download_directory(&state, &settings);
    if settings.download_folder.is_empty() {
        std::fs::create_dir_all(&directory).map_err(|_| Message::new("download.write"))?;
    }
    let recent = state
        .downloaded
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .clone();
    let context = recent
        .map(PathBuf::from)
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .into_iter()
        .chain([directory])
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, "patch-file", &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .add_filter("Patch", &["zip", "rar", "7z", "exe"])
            .pick_file()
    })
    .await
    .map_err(|e| Message::detail("error.patch", e))?;
    let Some(path) = path else {
        return Ok(None);
    };
    dialogs::remember(&state, "patch-file", &path)?;
    let folder = PathBuf::from(
        target
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(game.folder),
    );
    let plan = tauri::async_runtime::spawn_blocking(move || patches::preview(&path, &folder, id))
        .await
        .map_err(|e| Message::detail("error.patch", e))??;
    let preview = plan.preview.clone();
    *state
        .patch_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some(plan);
    Ok(Some(preview))
}

#[tauri::command]
async fn install_patch(state: State<'_, AppState>) -> Result<Vec<patches::Installed>, Message> {
    let _guard = features::begin_write(&state)?;
    let plan = state
        .patch_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .take()
        .ok_or_else(|| Message::detail("error.patch", "Select a patch and preview it first"))?;
    let data = patch_data(&state);
    let game = find_game(&state, &plan.preview.game_id)?;
    snapshots::require_stopped(runtime::detect(game.executable.as_deref()))?;
    validate_patch_target(&game, &plan.preview.target, &snapshot(&state)?)?;
    tauri::async_runtime::spawn_blocking(move || {
        patches::install(plan, &data)?;
        patches::list(&data)
    })
    .await
    .map_err(|e| Message::detail("error.patch", e))?
}

#[tauri::command]
async fn restore_patch(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    let _guard = features::begin_write(&state)?;
    let data = patch_data(&state);
    let record = patches::list(&data)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| Message::new("error.game_missing"))?;
    let game = find_game(&state, &record.preview.game_id)?;
    snapshots::require_stopped(runtime::detect(game.executable.as_deref()))?;
    validate_patch_target(&game, &record.preview.target, &snapshot(&state)?)?;
    tauri::async_runtime::spawn_blocking(move || {
        patches::restore(&data, &id)?;
        patches::list(&data)
    })
    .await
    .map_err(|e| Message::detail("error.patch", e))?
}

fn validate_patch_target(game: &Game, target: &str, settings: &Settings) -> Result<(), Message> {
    let root =
        std::fs::canonicalize(&game.folder).map_err(|e| Message::detail("error.patch", e))?;
    let target = std::fs::canonicalize(target).map_err(|e| Message::detail("error.patch", e))?;
    // A relocated game's old absolute patch target must never be written silently.
    let saves = saves::discover(game, settings.game_edits.get(&game.id));
    let current_save = saves
        .locations
        .iter()
        .filter(|l| l.confidence == "confirmed" && l.exists)
        .filter_map(|l| std::fs::canonicalize(&l.path).ok())
        .any(|path| target.starts_with(path));
    if !target.starts_with(root) && !current_save {
        return Err(Message::new("patch.target_reconfirm"));
    }
    Ok(())
}

#[tauri::command]
async fn list_patches(state: State<'_, AppState>) -> Result<Vec<patches::Installed>, Message> {
    let data = patch_data(&state);
    tauri::async_runtime::spawn_blocking(move || patches::list(&data))
        .await
        .map_err(|e| Message::detail("error.patch", e))?
}

#[tauri::command]
fn run_patch_installer(state: State<'_, AppState>) -> Result<Vec<patches::Installed>, Message> {
    let plan = state
        .patch_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let preview = &plan
        .as_ref()
        .ok_or_else(|| Message::detail("error.patch", "Select a patch first"))?
        .preview;
    let executable = preview
        .executable
        .as_ref()
        .ok_or_else(|| Message::detail("error.patch", "Not an EXE patch"))?;
    std::process::Command::new(executable)
        .current_dir(&preview.target)
        .spawn()
        .map_err(|e| Message::detail("error.patch", e))?;
    patches::record_exe(preview.clone(), &patch_data(&state))?;
    patches::list(&patch_data(&state))
}

#[tauri::command]
fn set_exe_status(
    id: String,
    installed: bool,
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    patches::set_exe_status(&patch_data(&state), &id, installed)
}

#[tauri::command]
async fn search_patches(
    query: String,
    state: State<'_, AppState>,
) -> Result<providers::ResourceSearch, Message> {
    let domain = snapshot(&state)?.fan_domain;
    tauri::async_runtime::spawn_blocking(move || providers::search_patch_sites(&domain, &query))
        .await
        .map_err(|e| Message::detail("error.network", e))?
}

#[tauri::command]
fn set_close_behavior(behavior: String, state: State<'_, AppState>) -> Result<Settings, Message> {
    if !["exit", "tray"].contains(&behavior.as_str()) {
        return Err(Message::new("error.operation"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.close_behavior = behavior;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
fn set_release_page(url: String, state: State<'_, AppState>) -> Result<Settings, Message> {
    if !url.trim().is_empty() {
        let parsed = reqwest::Url::parse(url.trim()).map_err(|_| Message::new("error.site_url"))?;
        let parts: Vec<_> = parsed.path_segments().into_iter().flatten().collect();
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("github.com")
            || parts.len() != 3
            || parts[2] != "releases"
        {
            return Err(Message::new("error.site_url"));
        }
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.release_page = url.trim().into();
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
async fn check_update(state: State<'_, AppState>) -> Result<serde_json::Value, Message> {
    let url = snapshot(&state)?.release_page;
    if url.is_empty() {
        return Err(Message::new("error.release_page"));
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| Message::new("error.site_url"))?;
    let parts: Vec<_> = parsed.path_segments().into_iter().flatten().collect();
    if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") || parts.len() != 3 {
        return Err(Message::new("error.site_url"));
    }
    let api = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        parts[0], parts[1]
    );
    let release =
        tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, Message> {
            providers::client()?
                .get(api)
                .send()
                .map_err(|e| Message::detail("error.network", e))?
                .error_for_status()
                .map_err(|e| Message::detail("error.network", e))?
                .json()
                .map_err(|e| Message::detail("error.network", e))
        })
        .await
        .map_err(|e| Message::detail("error.network", e))??;
    Ok(serde_json::json!({"version":release["tag_name"],"url":url}))
}

fn main() {
    tauri::Builder::default()
        .register_uri_scheme_protocol("galgear-icon", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let bytes = if id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()) {
                std::fs::read(
                    context
                        .app_handle()
                        .state::<AppState>()
                        .store
                        .path
                        .with_file_name("icons")
                        .join(format!("{id}.png")),
                )
                .ok()
            } else {
                None
            };
            tauri::http::Response::builder()
                .status(if bytes.is_some() { 200 } else { 404 })
                .header("Content-Type", "image/png")
                .header("Access-Control-Allow-Origin", "*")
                .body(bytes.unwrap_or_default())
                .expect("Static icon response")
        })
        .register_uri_scheme_protocol("galgear-cover", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let bytes = if id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()) {
                let state = context.app_handle().state::<AppState>();
                std::fs::read(
                    state
                        .store
                        .path
                        .with_file_name("covers")
                        .join("rendered")
                        .join(format!("{id}.jpg")),
                )
                .ok()
            } else {
                None
            };
            tauri::http::Response::builder()
                .status(if bytes.is_some() { 200 } else { 404 })
                .header("Content-Type", "image/jpeg")
                .header("Access-Control-Allow-Origin", "*")
                .body(bytes.unwrap_or_default())
                .expect("Static image response")
        })
        .setup(|app| {
            let exe_dir = std::env::current_exe()?
                .parent()
                .ok_or_else(|| std::io::Error::other("Executable directory"))?
                .to_owned();
            let data_dir = if exe_dir.join("portable.flag").exists() {
                exe_dir.join("data")
            } else {
                app.path().app_config_dir()?
            };
            let store = ConfigStore::new(data_dir.join("settings.json"));
            let settings = store.load().map_err(std::io::Error::other)?;
            app.set_theme(Some(if settings.theme == "dark" {
                tauri::Theme::Dark
            } else {
                tauri::Theme::Light
            }));
            let mut cached = catalog::load(&store.path.with_file_name("catalog.json"))
                .unwrap_or_else(|error| ScanResult {
                    warnings: vec![error],
                    ..Default::default()
                });
            let patch_dir = store.path.with_file_name("patches");
            match patches::list(&patch_dir) {
                Ok(records) => {
                    for record in records
                        .iter()
                        .filter(|r| ["installing", "restoring"].contains(&r.status.as_str()))
                    {
                        let result = cached
                            .games
                            .iter()
                            .find(|g| g.id == record.preview.game_id)
                            .ok_or_else(|| Message::new("patch.target_reconfirm"))
                            .and_then(|game| {
                                snapshots::require_stopped(runtime::detect(
                                    game.executable.as_deref(),
                                ))?;
                                validate_patch_target(game, &record.preview.target, &settings)?;
                                patches::restore(&patch_dir, &record.id).map(|_| ())
                            });
                        if let Err(error) = result {
                            cached.warnings.push(error);
                        }
                    }
                }
                Err(error) => cached.warnings.push(error),
            }
            for game in &mut cached.games {
                catalog::apply_edits(game, &settings);
            }
            let language = settings.language;
            let value = credentials::load(&credentials::target(&store.path)).unwrap_or_default();
            let status_cache = fan_cache::Cache::load(
                store.path.with_file_name("fan-status-cache.json"),
                fan_cache::scope(&settings.fan_domain, &value),
            );
            app.manage(AppState {
                store,
                settings: Mutex::new(settings),
                catalog: Mutex::new(cached),
                scanning: AtomicBool::new(false),
                cancel: Arc::new(AtomicBool::new(false)),
                running: Mutex::new(Default::default()),
                patch_plan: Mutex::new(None),
                fan_plan: Mutex::new(None),
                downloading: AtomicBool::new(false),
                download_cancel: Arc::new(AtomicBool::new(false)),
                downloaded: Mutex::new(None),
                fan_refresh_running: AtomicBool::new(false),
                fan_cache: Mutex::new(status_cache),
                fan_refresh_lock: tokio::sync::Mutex::new(()),
                save_plan: Mutex::new(None),
                writing: AtomicBool::new(false),
            });
            let show = tauri::menu::MenuItem::with_id(
                app,
                "show",
                tr(language, "tray.show"),
                true,
                None::<&str>,
            )?;
            start_fan_status_scheduler(app.handle().clone());
            features::scheduler(app.handle().clone());
            let quit = tauri::menu::MenuItem::with_id(
                app,
                "quit",
                tr(language, "settings.exit"),
                true,
                None::<&str>,
            )?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            tauri::tray::TrayIconBuilder::with_id("main")
                .icon(
                    app.default_window_icon()
                        .ok_or_else(|| std::io::Error::other("Missing window icon"))?
                        .clone(),
                )
                .menu(&menu)
                .tooltip("GalGear")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event
                        && let Some(window) = tray.app_handle().get_webview_window("main")
                    {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if snapshot(&state).is_ok_and(|s| s.close_behavior == "tray") {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            load_game_icons,
            load_fan_credentials,
            save_fan_credentials,
            clear_fan_credentials,
            get_fan_status_cache,
            refresh_fan_status,
            choose_download_folder,
            open_download_folder,
            inspect_fan_resource,
            start_fan_download,
            cancel_fan_download,
            preview_downloaded_patch,
            scan_library,
            cancel_scan,
            detect_saves,
            choose_save_folder,
            open_save_folder,
            edit_game,
            add_manual_game,
            relocate_game,
            exclude_game,
            set_launch_behavior,
            set_close_behavior,
            set_release_page,
            check_update,
            search_metadata,
            data_sources::get_data_sources,
            data_sources::save_data_source,
            data_sources::clear_data_source,
            data_sources::set_data_source,
            select_metadata,
            open_resource,
            enrich_library,
            preview_patch,
            choose_patch_target,
            install_patch,
            restore_patch,
            list_patches,
            run_patch_installer,
            set_exe_status,
            search_patches,
            choose_library_folder,
            add_library_folder,
            remove_library_folder,
            save_preferences,
            set_language,
            choose_cover,
            open_game_folder,
            launch_game,
            features::get_preferences,
            features::set_theme,
            features::set_display,
            features::edit_title,
            features::save_game_details,
            features::set_play_status,
            features::set_patch_tab,
            features::edit_metadata,
            features::edit_tags,
            features::reset_cover,
            features::reset_game_tags,
            collections::edit_collection,
            collections::delete_collections,
            collections::batch_games,
            collections::excluded_entries,
            collections::set_library_bounds,
            features::confirm_save,
            features::save_history,
            features::backup_saves,
            features::preview_save_restore,
            features::restore_saves,
            features::update_snapshot,
            features::configure_tools,
            features::set_game_tools,
            features::game_runtime,
            features::upscale_game,
        ])
        .run(tauri::generate_context!())
        .expect("GalGear startup failed");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_a_cover_changes_its_url_and_preserves_previous_image() {
        use base64::Engine;
        let temp = tempfile::tempdir().unwrap();
        let mut game = catalog::manual_game(&temp.path().join("game.exe"), "Game".into()).unwrap();
        let data = |bytes: &[u8]| {
            format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        };
        game.cover = Some(data(b"first image"));
        prepare_cover(&mut game, temp.path()).unwrap();
        let first = game.cover.clone().unwrap();
        game.cover = Some(data(b"second image"));
        prepare_cover(&mut game, temp.path()).unwrap();
        let second = game.cover.clone().unwrap();
        assert_ne!(first, second);
        let file = temp
            .path()
            .join(format!("{}.jpg", first.rsplit('/').next().unwrap()));
        assert_eq!(std::fs::read(file).unwrap(), b"first image");
    }
}
