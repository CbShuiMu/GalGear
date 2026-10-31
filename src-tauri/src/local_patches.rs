//! Commands for the patch directories belonging to one game.
use crate::{patch_commands::patch_data, state, state::AppState, state::find_game};
use galgear_core::{
    patches::{self, folders},
    runtime, saves, snapshots,
};
use galgear_i18n::Message;
use std::path::Path;
use tauri::State;

#[tauri::command]
pub async fn list_game_patches(
    id: String,
    state: State<'_, AppState>,
) -> Result<folders::Report, Message> {
    let game = find_game(&state, &id)?;
    if !game.available {
        return Err(Message::new("error.game_folder"));
    }
    tauri::async_runtime::spawn_blocking(move || folders::list(Path::new(&game.folder)))
        .await
        .map_err(|e| Message::detail("error.patch", e))?
}

#[tauri::command]
pub async fn apply_game_patch(
    id: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<folders::Report, Message> {
    let _guard = state::begin_write(&state)?;
    let game = find_game(&state, &id)?;
    if !game.available {
        return Err(Message::new("error.game_folder"));
    }
    let data = patch_data(&state);
    let edit = state::snapshot(&state)?.game_edits.get(&id).cloned();
    tauri::async_runtime::spawn_blocking(move || {
        snapshots::require_stopped(runtime::detect(game.executable.as_deref()))?;
        let folder = Path::new(&game.folder);
        let report = saves::discover(&game, edit.as_ref());
        let confirmed: Vec<_> = report.locations.iter().filter(|location| location.exists && location.confidence == "confirmed").collect();
        let save_path = if confirmed.len() == 1 { Some(Path::new(&confirmed[0].path)) } else { None };
        let plan = folders::preview_to(folder, &name, id, save_path)?;
        snapshots::require_stopped(runtime::detect(game.executable.as_deref()))?;
        patches::install(plan, &data)?;
        folders::list(folder)
    })
    .await
    .map_err(|e| Message::detail("error.patch", e))?
}

#[tauri::command]
pub async fn open_game_patch_folder(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    if !game.available {
        return Err(Message::new("error.game_folder"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let folder = folders::directory(Path::new(&game.folder))?;
        std::process::Command::new("explorer.exe")
            .arg(folder)
            .spawn()
            .map_err(|e| Message::detail("error.open_folder", e))?;
        Ok(())
    })
    .await
    .map_err(|e| Message::detail("error.patch", e))?
}
