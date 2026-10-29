//! Game launch, runtime observation and external tools.
use crate::{
    preferences::update, save_commands::backup, save_commands::backup_data, state, state::AppState,
    state::begin_write, state::find_game, state::snapshot,
};
use galgear_core::{Settings, playtime, runtime, runtime::Status, saves, snapshots};
use galgear_i18n::Message;
use std::{collections::BTreeMap, path::Path, path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager, State};

#[derive(Default)]
pub(crate) struct RuntimeState {
    running: Mutex<BTreeMap<String, std::process::Child>>,
}

fn normalize_tool_path(path: &str) -> &str {
    let path = path.trim();
    path.strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .unwrap_or(path)
        .trim()
}

fn tool_path<'a>(path: &'a str, missing_key: &str) -> Result<&'a str, Message> {
    let path = normalize_tool_path(path);
    if path.is_empty() {
        return Err(Message::new(missing_key));
    }
    if !Path::new(path).is_file() {
        return Err(Message::new("tools.path_invalid").arg("path", path));
    }
    Ok(path)
}

fn select_locale_profile<'a>(
    profiles: &'a [runtime::LocaleProfile],
    guid: &str,
) -> Result<&'a runtime::LocaleProfile, Message> {
    profiles
        .iter()
        .find(|profile| {
            if guid.is_empty() {
                profile.locale == "ja-JP" && profile.admin
            } else {
                profile.guid == guid
            }
        })
        .ok_or_else(|| Message::new("tools.locale_profile"))
}

#[tauri::command]
pub(crate) fn open_game_folder(id: String, state: State<'_, AppState>) -> Result<(), Message> {
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
pub(crate) fn launch_game(
    id: String,
    mode: Option<String>,
    confirmed_unknown: Option<bool>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), Message> {
    let _guard = state::begin_write(&state)?;
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
        let magpie = tool_path(&settings.magpie, "tools.magpie_missing")?;
        if runtime::detect(Some(magpie)) != runtime::Status::Running {
            return Err(Message::new("tools.magpie_start"));
        }
    }
    let mut running = state
        .tools
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
        let locale_emulator = tool_path(&settings.locale_emulator, "tools.locale_missing")?;
        let mut command = std::process::Command::new(locale_emulator);
        let profiles = runtime::locale_profiles(Path::new(locale_emulator))
            .map_err(|e| Message::detail("tools.error", e))?;
        let profile = select_locale_profile(&profiles, &edit.locale_profile)?;
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
        schedule_upscale(
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

#[tauri::command]
pub(crate) fn configure_tools(
    locale_emulator: String,
    magpie: String,
    magpie_hotkey: String,
    backup_retention: usize,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    runtime::hotkey_keys(&magpie_hotkey).map_err(|e| Message::detail("tools.error", e))?;
    if !(1..=1000).contains(&backup_retention) {
        return Err(Message::new("error.operation"));
    }
    let locale_emulator = normalize_tool_path(&locale_emulator);
    let magpie = normalize_tool_path(&magpie);
    if !locale_emulator.is_empty() {
        tool_path(locale_emulator, "tools.locale_missing")?;
    }
    if !magpie.is_empty() {
        tool_path(magpie, "tools.magpie_missing")?;
    }
    update(&state, |s| {
        s.locale_emulator = locale_emulator.to_owned();
        s.magpie = magpie.to_owned();
        s.magpie_hotkey = magpie_hotkey;
        s.backup_retention = backup_retention;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn set_game_tools(
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
pub(crate) fn game_runtime(id: String, state: State<'_, AppState>) -> Result<Status, Message> {
    Ok(runtime::detect(
        find_game(&state, &id)?.executable.as_deref(),
    ))
}

#[tauri::command]
pub(crate) fn upscale_game(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let magpie = tool_path(&settings.magpie, "tools.magpie_missing")?;
    // An existing Magpie instance with a user-selected scaling profile is required.
    if runtime::detect(Some(magpie)) != Status::Running {
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

pub(crate) fn schedule_upscale(
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

pub(crate) fn scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous = BTreeMap::new();
        let mut sessions = galgear_core::play_sessions::Tracker::default();
        let mut tracker = playtime::Tracker::default();
        let mut playtime_dirty = false;
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
            let handle = app.clone();
            let observed = current.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let state = handle.state::<AppState>();
                let result = record_playtime(&state, &mut tracker, &observed, &mut playtime_dirty);
                if let Ok(records) = &result
                    && !records.is_empty()
                {
                    let _ = handle.emit("playtime-updated", records);
                }
                (tracker, playtime_dirty, result.err())
            })
            .await;
            match result {
                Ok((next_tracker, dirty, error)) => {
                    tracker = next_tracker;
                    playtime_dirty = dirty;
                    if let Some(error) = error {
                        let _ = app.emit("tool-failed", error);
                    }
                }
                Err(_) => {
                    tracker = playtime::Tracker::default();
                    playtime_dirty = true;
                }
            }
            if let Ok(settings) = snapshot(&state) {
                // Recovery remains pending when runtime or current save targets cannot be confirmed.
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
                            let data = backup_data(&game);
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
                for id in sessions.observe(&current) {
                    if settings.game_edits.get(&id).is_some_and(|e| e.auto_backup) {
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

fn record_playtime(
    state: &AppState,
    tracker: &mut playtime::Tracker,
    statuses: &BTreeMap<String, Status>,
    dirty: &mut bool,
) -> Result<playtime::Records, Message> {
    let mut records = state
        .playtime
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let mut catalog = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let changed = tracker.observe(
        &mut catalog.games,
        statuses,
        &mut records,
        std::time::Instant::now(),
        timestamp,
    );
    *dirty |= !changed.is_empty();
    if *dirty {
        playtime::save(&state.store.path.with_file_name("playtime.json"), &records)?;
        *dirty = false;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale_profiles() -> Vec<runtime::LocaleProfile> {
        vec![
            runtime::LocaleProfile {
                guid: "english-admin".into(),
                name: "English (Admin)".into(),
                locale: "en-US".into(),
                admin: true,
            },
            runtime::LocaleProfile {
                guid: "japanese".into(),
                name: "Run in Japanese".into(),
                locale: "ja-JP".into(),
                admin: false,
            },
            runtime::LocaleProfile {
                guid: "japanese-admin".into(),
                name: "Run in Japanese (Admin)".into(),
                locale: "ja-JP".into(),
                admin: true,
            },
        ]
    }

    #[test]
    fn default_locale_launch_selects_japanese_admin_after_other_profiles() {
        let profiles = locale_profiles();
        assert_eq!(
            select_locale_profile(&profiles, "").unwrap().guid,
            "japanese-admin"
        );
    }

    #[test]
    fn explicit_locale_guid_keeps_the_users_non_admin_choice() {
        let profiles = locale_profiles();
        assert_eq!(
            select_locale_profile(&profiles, "japanese").unwrap().guid,
            "japanese"
        );
    }

    #[test]
    fn missing_japanese_admin_profile_reports_an_error_instead_of_falling_back() {
        let mut profiles = locale_profiles();
        profiles.retain(|profile| profile.guid != "japanese-admin");
        assert_eq!(
            select_locale_profile(&profiles, "").unwrap_err().key,
            "tools.locale_profile"
        );
    }

    #[test]
    fn unknown_explicit_locale_guid_does_not_use_the_default_admin_profile() {
        let profiles = locale_profiles();
        assert_eq!(
            select_locale_profile(&profiles, "missing").unwrap_err().key,
            "tools.locale_profile"
        );
    }

    #[test]
    fn copied_tool_path_accepts_quotes_spaces_and_unicode() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("转区工具 Locale Emulator");
        std::fs::create_dir(&folder).unwrap();
        let executable = folder.join("LEProc.exe");
        std::fs::write(&executable, []).unwrap();
        let copied = format!(" \"{}\"\r\n", executable.display());

        assert_eq!(
            tool_path(&copied, "tools.locale_missing").unwrap(),
            executable.to_str().unwrap()
        );
    }

    #[test]
    fn unavailable_tool_reports_the_configured_path() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("LEProc.exe");
        let path = executable.to_str().unwrap();

        assert_eq!(
            tool_path(path, "tools.locale_missing").unwrap_err(),
            Message::new("tools.path_invalid").arg("path", path)
        );
    }

    #[test]
    fn tool_directory_is_rejected_as_an_executable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_str().unwrap();

        assert_eq!(
            tool_path(path, "tools.locale_missing").unwrap_err().key,
            "tools.path_invalid"
        );
    }

    #[test]
    fn blank_tool_path_reports_the_missing_setting() {
        assert_eq!(
            tool_path(" \"\"\r\n", "tools.locale_missing")
                .unwrap_err()
                .key,
            "tools.locale_missing"
        );
    }
}
