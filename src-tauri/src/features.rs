//! Commands for local preferences, metadata edits, save history and external tools.
use super::{AppState, find_game, persist_game, snapshot};
use galgear_core::{
    Game, Settings, catalog,
    runtime::{self, Status},
    saves, snapshots, tags,
};
use galgear_i18n::Message;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tauri::{Emitter, Manager, State};

pub(super) fn update(
    state: &AppState,
    change: impl FnOnce(&mut Settings) -> Result<(), Message>,
) -> Result<Settings, Message> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    change(&mut next)?;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub fn get_preferences(state: State<'_, AppState>) -> Result<Settings, Message> {
    snapshot(&state)
}

#[tauri::command]
pub fn set_patch_tab(visible: bool, state: State<'_, AppState>) -> Result<Settings, Message> {
    update(&state, |settings| {
        settings.show_patches = visible;
        Ok(())
    })
}

#[tauri::command]
pub fn set_play_status(
    id: String,
    status: galgear_core::PlayStatus,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    let settings = update(&state, |settings| {
        let edit =
            galgear_core::game_details::with_status(&game, settings.game_edits.get(&id), status);
        settings.game_edits.insert(id, edit);
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub fn save_game_details(
    id: String,
    draft: galgear_core::game_details::Draft,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    let settings = update(&state, |settings| {
        let edit = galgear_core::game_details::prepare(&game, settings.game_edits.get(&id), draft)?;
        settings.game_edits.insert(id, edit);
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub fn set_theme(
    theme: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Settings, Message> {
    if !matches!(theme.as_str(), "light" | "dark") {
        return Err(Message::new("error.operation"));
    }
    let settings = update(&state, |settings| {
        settings.theme = theme;
        Ok(())
    })?;
    app.set_theme(Some(if settings.theme == "dark" {
        tauri::Theme::Dark
    } else {
        tauri::Theme::Light
    }));
    Ok(settings)
}

#[tauri::command]
pub fn set_color_theme(
    color_theme: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !matches!(color_theme.as_str(), "pink" | "white-pink") {
        return Err(Message::new("error.operation"));
    }
    update(&state, |settings| {
        settings.color_theme = color_theme;
        Ok(())
    })
}

#[tauri::command]
pub fn set_display(
    cover_scale: u32,
    font_size: u32,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !(50..=200).contains(&cover_scale) || ![14, 16, 18].contains(&font_size) {
        return Err(Message::new("error.operation"));
    }
    update(&state, |s| {
        s.cover_scale = cover_scale;
        s.font_size = font_size;
        Ok(())
    })
}

#[tauri::command]
pub fn edit_title(id: String, title: String, state: State<'_, AppState>) -> Result<Game, Message> {
    if title.trim().is_empty() {
        return Err(Message::new("error.title_required"));
    }
    let mut game = find_game(&state, &id)?;
    let settings = update(&state, |s| {
        s.game_edits.entry(id).or_default().title = Some(title.trim().into());
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub fn edit_tags(
    id: String,
    names: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    let names = tags::normalize(names);
    let settings = update(&state, |s| {
        let edit = s.game_edits.entry(id).or_default();
        tags::remember(&game.tags, &names, &mut edit.tag_rules);
        edit.tags = names;
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub async fn reset_cover(id: String, state: State<'_, AppState>) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    update(&state, |s| {
        s.cover_overrides.remove(&id);
        Ok(())
    })?;
    game.cover = None;
    let url = game
        .metadata
        .as_ref()
        .map(|m| m.url.clone())
        .filter(|u| !u.is_empty());
    let game = persist_game(&state, game)?;
    if let Some(url) = url {
        super::select_metadata(id, url, None, state).await
    } else {
        Ok(game)
    }
}

#[tauri::command]
pub fn edit_metadata(
    id: String,
    field: String,
    value: Option<String>,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    if !galgear_core::providers::Metadata::default().set_field(&field, "") {
        return Err(Message::new("error.operation"));
    }
    if field == "title" {
        return Err(Message::new("error.operation"));
    }
    let settings = update(&state, |s| {
        let edit = s.game_edits.entry(id).or_default();
        if let Some(value) = value {
            edit.metadata_overrides.insert(field.clone(), value);
        } else {
            edit.metadata_overrides.remove(&field);
            if field == "description" {
                edit.description = None;
            }
        }
        if edit.metadata.is_none() {
            edit.metadata = Some(galgear_core::providers::Metadata::default());
        }
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub fn reset_game_tags(id: String, state: State<'_, AppState>) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    let settings = update(&state, |s| {
        let edit = s.game_edits.entry(id).or_default();
        if edit.tags.is_empty() {
            edit.tags.clone_from(&game.tags);
        }
        edit.tag_rules.clear();
        if let Some(metadata) = &game.metadata {
            edit.tags = tags::merge(&edit.tags, &metadata.tags, &edit.tag_rules);
        }
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub fn confirm_save(id: String, path: String, state: State<'_, AppState>) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    if !saves::discover(&game, settings.game_edits.get(&id))
        .locations
        .iter()
        .any(|l| l.path == path && l.exists)
    {
        return Err(Message::new("error.game_folder"));
    }
    update(&state, |s| {
        let edit = s.game_edits.entry(id).or_default();
        if !edit.save_paths.contains(&path) {
            edit.save_paths.push(path);
        }
        Ok(())
    })?;
    Ok(())
}

pub fn backup_data(state: &AppState, settings: &Settings) -> PathBuf {
    if settings.backup_folder.is_empty() {
        state.store.path.with_file_name("save-backups")
    } else {
        PathBuf::from(&settings.backup_folder)
    }
}
fn confirmed_roots(
    state: &AppState,
    game: &Game,
    settings: &Settings,
) -> Result<Vec<String>, Message> {
    let roots = saves::discover(game, settings.game_edits.get(&game.id))
        .locations
        .into_iter()
        .filter(|l| l.confidence == "confirmed" && l.exists)
        .map(|l| l.path)
        .collect::<Vec<_>>();
    snapshots::validate_roots(
        &roots,
        Path::new(&game.folder),
        &backup_data(state, settings),
    )
}

pub(super) struct WriteGuard<'a>(&'a AppState);
impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.0.writing.store(false, Ordering::SeqCst);
    }
}
pub(super) fn begin_write(state: &AppState) -> Result<WriteGuard<'_>, Message> {
    if state.writing.swap(true, Ordering::SeqCst) {
        return Err(Message::new("backup.busy"));
    }
    Ok(WriteGuard(state))
}

#[tauri::command]
pub async fn save_history(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let data = backup_data(&state, &snapshot(&state)?);
    tauri::async_runtime::spawn_blocking(move || snapshots::list(&data, &id))
        .await
        .map_err(|e| Message::detail("backup.error", e))?
}
pub async fn backup(id: String, note: String, state: &AppState) -> Result<(), Message> {
    let _guard = begin_write(state)?;
    let game = find_game(state, &id)?;
    let settings = snapshot(state)?;
    let roots = confirmed_roots(state, &game, &settings)?;
    let data = backup_data(state, &settings);
    tauri::async_runtime::spawn_blocking(move || {
        snapshots::recover(
            &data,
            &id,
            &roots,
            runtime::detect(game.executable.as_deref()),
        )?;
        snapshots::create_named_checked(
            &data,
            (&id, &game.title),
            &roots,
            &note,
            settings.backup_retention,
            || runtime::detect(game.executable.as_deref()),
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))?
}
#[tauri::command]
pub async fn backup_saves(
    id: String,
    note: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    backup(id.clone(), note, &state).await?;
    save_history(id, state).await
}

#[tauri::command]
pub async fn preview_save_restore(
    id: String,
    snapshot_id: String,
    state: State<'_, AppState>,
) -> Result<snapshots::Preview, Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let roots = confirmed_roots(&state, &game, &settings)?;
    let data = backup_data(&state, &settings);
    let preview = tauri::async_runtime::spawn_blocking(move || {
        snapshots::preview(
            &data,
            &id,
            &snapshot_id,
            &roots,
            runtime::detect(game.executable.as_deref()),
        )
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))??;
    *state
        .save_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some(preview.clone());
    Ok(preview)
}

#[tauri::command]
pub async fn restore_saves(
    id: String,
    snapshot_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let plan = state
        .save_plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .take()
        .ok_or_else(|| Message::new("backup.preview_required"))?;
    if plan.snapshot.id != snapshot_id || plan.snapshot.game_id != id {
        return Err(Message::new("backup.preview_required"));
    }
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    if confirmed_roots(&state, &game, &settings)? != plan.targets {
        return Err(Message::new("backup.preview_required"));
    }
    let data = backup_data(&state, &settings);
    tauri::async_runtime::spawn_blocking(move || {
        snapshots::restore_checked(&data, &plan, settings.backup_retention, || {
            runtime::detect(game.executable.as_deref())
        })
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))??;
    save_history(id, state.clone()).await
}

#[tauri::command]
pub async fn update_snapshot(
    id: String,
    snapshot_id: String,
    locked: bool,
    note: String,
    delete: bool,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let data = backup_data(&state, &snapshot(&state)?);
    let game_id = id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if delete {
            snapshots::delete(&data, &game_id, &snapshot_id)
        } else {
            snapshots::update(&data, &game_id, &snapshot_id, locked, &note)
        }
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))??;
    save_history(id, state.clone()).await
}

#[tauri::command]
pub fn configure_tools(
    locale_emulator: String,
    magpie: String,
    magpie_hotkey: String,
    backup_folder: String,
    backup_retention: usize,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    runtime::hotkey_keys(&magpie_hotkey).map_err(|e| Message::detail("tools.error", e))?;
    if !(1..=1000).contains(&backup_retention) {
        return Err(Message::new("error.operation"));
    }
    update(&state, |s| {
        s.locale_emulator = locale_emulator;
        s.magpie = magpie;
        s.magpie_hotkey = magpie_hotkey;
        s.backup_folder = backup_folder;
        s.backup_retention = backup_retention;
        Ok(())
    })
}
#[tauri::command]
pub fn set_game_tools(
    id: String,
    launch_mode: String,
    locale_profile: String,
    auto_backup: bool,
    upscale: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    find_game(&state, &id)?;
    if !["normal", "locale"].contains(&launch_mode.as_str()) {
        return Err(Message::new("error.operation"));
    }
    update(&state, |s| {
        let edit = s.game_edits.entry(id).or_default();
        edit.launch_mode = launch_mode;
        edit.locale_profile = locale_profile;
        edit.auto_backup = auto_backup;
        if let Some(upscale) = upscale {
            edit.upscale = upscale;
        }
        Ok(())
    })
}
#[tauri::command]
pub fn game_runtime(id: String, state: State<'_, AppState>) -> Result<Status, Message> {
    Ok(runtime::detect(
        find_game(&state, &id)?.executable.as_deref(),
    ))
}

#[tauri::command]
pub fn upscale_game(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    if !Path::new(&settings.magpie).is_file() {
        return Err(Message::new("tools.magpie_missing"));
    }
    // An existing Magpie instance with a user-selected scaling profile is required.
    if runtime::detect(Some(&settings.magpie)) != Status::Running {
        return Err(Message::new("tools.magpie_start"));
    }
    if runtime::detect(game.executable.as_deref()) != Status::Running {
        return Err(Message::new("tools.game_stopped"));
    }
    let executable = game
        .executable
        .ok_or_else(|| Message::new("error.exe_missing"))?;
    runtime::trigger_magpie(Path::new(&executable), &settings.magpie_hotkey)
        .map_err(|e| Message::detail("tools.error", e))
}

pub fn schedule_upscale(
    app: tauri::AppHandle,
    executable: String,
    hotkey: String,
    exit_after: bool,
) {
    tauri::async_runtime::spawn(async move {
        let mut last_error = String::new();
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            match runtime::trigger_magpie(Path::new(&executable), &hotkey) {
                Ok(()) => {
                    if exit_after {
                        app.exit(0);
                    }
                    return;
                }
                Err(error) => last_error = error,
            }
        }
        let _ = app.emit("tool-failed", Message::detail("tools.error", last_error));
    });
}

pub fn scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous = BTreeMap::new();
        let mut was_running = std::collections::BTreeSet::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let state = app.state::<AppState>();
            let games = match state.catalog.lock() {
                Ok(c) => c
                    .games
                    .iter()
                    .map(|g| (g.id.clone(), g.executable.clone()))
                    .collect::<Vec<_>>(),
                Err(_) => continue,
            };
            let current =
                match tauri::async_runtime::spawn_blocking(move || runtime::detect_many(&games))
                    .await
                {
                    Ok(c) => c,
                    Err(_) => continue,
                };
            if current != previous {
                let _ = app.emit("runtime-updated", &current);
            }
            if let Ok(settings) = snapshot(&state) {
                // Recovery remains pending when runtime or current save targets cannot be confirmed.
                let data = backup_data(&state, &settings);
                let recovery_games = state
                    .catalog
                    .lock()
                    .map(|c| c.games.clone())
                    .unwrap_or_default();
                let recovery_settings = settings.clone();
                if let Ok(_guard) = begin_write(&state) {
                    let handle = app.clone();
                    let _ = tauri::async_runtime::spawn_blocking(move || {
                        for game in recovery_games {
                            if !data.join(&game.id).join("restore-journal.json").exists() {
                                continue;
                            }
                            let roots =
                                saves::discover(&game, recovery_settings.game_edits.get(&game.id))
                                    .locations
                                    .into_iter()
                                    .filter(|l| l.confidence == "confirmed" || l.source == "config")
                                    .map(|l| l.path)
                                    .collect::<Vec<_>>();
                            if let Err(error) = snapshots::recover(
                                &data,
                                &game.id,
                                &roots,
                                runtime::detect(game.executable.as_deref()),
                            ) {
                                let _ = handle.emit(
                                    "backup-completed",
                                    serde_json::json!({"id":game.id,"error":error}),
                                );
                            }
                        }
                    })
                    .await;
                }
                for (id, status) in &current {
                    if *status == Status::Running {
                        was_running.insert(id.clone());
                    }
                    if *status == Status::Stopped
                        && was_running.remove(id)
                        && settings.game_edits.get(id).is_some_and(|e| e.auto_backup)
                    {
                        let result = backup(id.clone(), "Automatic backup".into(), &state).await;
                        let _ = app.emit(
                            "backup-completed",
                            serde_json::json!({"id":id,"error":result.err()}),
                        );
                    }
                }
            }
            previous = current;
        }
    });
}
