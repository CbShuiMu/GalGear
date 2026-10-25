//! Patch previews, installation and recovery.
use crate::{
    dialogs, downloads::download_directory, state, state::AppState, state::find_game,
    state::snapshot,
};
use galgear_core::{Game, ScanResult, Settings, patches, providers, runtime, saves, snapshots};
use galgear_i18n::Message;
use std::{path::PathBuf, sync::Mutex};
use tauri::State;

#[derive(Default)]
pub(crate) struct PatchState {
    plan: Mutex<Option<patches::Plan>>,
}

pub(crate) fn recover(data: &std::path::Path, settings: &Settings, cached: &mut ScanResult) {
    match patches::list(data) {
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
                        snapshots::require_stopped(runtime::detect(game.executable.as_deref()))?;
                        validate_patch_target(game, &record.preview.target, settings)?;
                        patches::restore(data, &record.id).map(|_| ())
                    });
                if let Err(error) = result {
                    cached.warnings.push(error);
                }
            }
        }
        Err(error) => cached.warnings.push(error),
    }
}

pub(crate) fn patch_data(state: &AppState) -> PathBuf {
    state.store.path.with_file_name("patches")
}

#[tauri::command]
pub(crate) async fn preview_downloaded_patch(
    id: String,
    target: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<patches::Preview>, Message> {
    let game = find_game(&state, &id)?;
    if !game.available || game.executable.is_none() {
        return Err(Message::new("error.exe_missing"));
    }
    let path = state
        .downloads
        .downloaded_path()?
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
        .patches
        .plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some(plan);
    Ok(Some(preview))
}

#[tauri::command]
pub(crate) async fn choose_patch_target(
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
pub(crate) async fn preview_patch(
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
    let recent = state.downloads.downloaded_path()?;
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
        .patches
        .plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some(plan);
    Ok(Some(preview))
}

#[tauri::command]
pub(crate) async fn install_patch(
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    let _guard = state::begin_write(&state)?;
    let plan = state
        .patches
        .plan
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
pub(crate) async fn restore_patch(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    let _guard = state::begin_write(&state)?;
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

pub(crate) fn validate_patch_target(
    game: &Game,
    target: &str,
    settings: &Settings,
) -> Result<(), Message> {
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
pub(crate) async fn list_patches(
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    let data = patch_data(&state);
    tauri::async_runtime::spawn_blocking(move || patches::list(&data))
        .await
        .map_err(|e| Message::detail("error.patch", e))?
}

#[tauri::command]
pub(crate) fn run_patch_installer(
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    let plan = state
        .patches
        .plan
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
pub(crate) fn set_exe_status(
    id: String,
    installed: bool,
    state: State<'_, AppState>,
) -> Result<Vec<patches::Installed>, Message> {
    patches::set_exe_status(&patch_data(&state), &id, installed)
}

#[tauri::command]
pub(crate) async fn search_patches(
    query: String,
    state: State<'_, AppState>,
) -> Result<providers::ResourceSearch, Message> {
    let domain = snapshot(&state)?.fan_domain;
    tauri::async_runtime::spawn_blocking(move || providers::search_patch_sites(&domain, &query))
        .await
        .map_err(|e| Message::detail("error.network", e))?
}
