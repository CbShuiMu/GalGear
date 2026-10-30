//! Game launch, runtime observation and external tools.
use crate::{
    preferences::update, save_commands::automatic_backup, save_commands::backup_data, state, state::AppState,
    state::begin_write, state::find_game, state::snapshot,
};
use galgear_core::{Settings, playtime, runtime, runtime::Status, saves, snapshots};
use galgear_i18n::Message;
use std::{collections::BTreeMap, path::Path, path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager, State};

#[cfg(windows)]
use crate::bundled_tools::{MagpieProcess, start_magpie};
#[cfg(not(windows))]
type MagpieProcess = std::process::Child;

#[derive(Default)]
pub(crate) struct RuntimeState {
    running: Mutex<BTreeMap<String, std::process::Child>>,
    magpie: Mutex<Option<MagpieProcess>>,
    shutting_down: std::sync::atomic::AtomicBool,
}

pub(crate) fn shutdown_magpie(state: &AppState) -> Result<(), Message> {
    state.tools.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut owned = state.tools.magpie.lock().map_err(|_| Message::new("error.operation"))?;
    if let Some(child) = owned.as_mut() {
        #[cfg(windows)]
        child.stop()?;
        #[cfg(not(windows))]
        if child.try_wait().map_err(|e| Message::detail("tools.error", e))?.is_none() {
            child.kill().map_err(|e| Message::detail("tools.error", e))?;
            child.wait().map_err(|e| Message::detail("tools.error", e))?;
        }
    } else {
        #[cfg(windows)]
        if let Ok(settings) = snapshot(state)
            && let Ok(executable) = tool_path(&settings.magpie, "tools.magpie_missing")
        {
            crate::bundled_tools::stop_magpie(&executable)?;
        }
    }
    *owned = None;
    Ok(())
}

fn normalize_tool_path(path: &str) -> &str {
    let path = path.trim();
    path.strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .unwrap_or(path)
        .trim()
}

fn tool_path(path: &str, missing_key: &str) -> Result<PathBuf, Message> {
    let path = normalize_tool_path(path);
    if path.is_empty() {
        return Err(Message::new(missing_key));
    }
    let executable = std::env::current_exe().map_err(|e| Message::detail("tools.error", e))?;
    let resolved = crate::bundled_tools::resolve(executable.parent().unwrap_or(Path::new(".")), path);
    if !resolved.is_file() {
        return Err(Message::new("tools.path_invalid").arg("path", path));
    }
    Ok(resolved)
}

fn ensure_magpie(state: &AppState, settings: &Settings) -> Result<String, Message> {
    let executable = tool_path(&settings.magpie, "tools.magpie_missing")?;
    let hotkey = crate::bundled_tools::magpie_hotkey(&executable, &settings.magpie_hotkey)?;
    let mut owned = state.tools.magpie.lock().map_err(|_| Message::new("error.operation"))?;
    if state.tools.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Message::new("error.operation"));
    }
    if runtime::detect(executable.to_str()) == Status::Running {
        #[cfg(windows)]
        crate::bundled_tools::wait_magpie(&executable)?;
        if owned.as_mut().is_none_or(|child| child.try_wait().is_ok_and(|status| status.is_some())) {
            *owned = Some(start_magpie(&executable)?);
        }
        return Ok(hotkey);
    }
    if let Some(child) = owned.as_mut() {
        let _ = child.try_wait();
    }
    *owned = Some(start_magpie(&executable)?);
    Ok(hotkey)
}

#[cfg(not(windows))]
fn start_magpie(executable: &Path) -> Result<MagpieProcess, Message> {
    let mut command = std::process::Command::new(executable);
    command.arg("-t").current_dir(executable.parent().unwrap_or(Path::new(".")));
    command.spawn().map_err(|e| Message::detail("tools.error", e))
}

fn save_magpie_hotkey(state: &AppState, executable: &Path, hotkey: &str) -> Result<String, Message> {
    crate::bundled_tools::validate_magpie_hotkey(executable, hotkey)?;
    let current = crate::bundled_tools::magpie_hotkey(executable, "")?;
    // A redundant Save must not interrupt scaling or restart the application.
    if current == crate::bundled_tools::canonical_magpie_hotkey(hotkey)? { return Ok(current); }
    let mut owned = state.tools.magpie.lock().map_err(|_| Message::new("error.operation"))?;
    if state.tools.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Message::new("error.operation"));
    }
    #[cfg(windows)]
    let running = if let Some(child) = owned.as_mut() {
        let running = child.try_wait().map_err(|e| Message::detail("tools.error", e))?.is_none();
        child.stop()?;
        *owned = None;
        crate::bundled_tools::stop_magpie(executable)? || running
    } else {
        crate::bundled_tools::stop_magpie(executable)?
    };
    #[cfg(not(windows))]
    let running = false;
    // Read again after exit to include any pending Magpie settings save.
    let saved = crate::bundled_tools::write_magpie_hotkey(executable, hotkey);
    let restarted = if running {
        start_magpie(executable).map(|child| *owned = Some(child))
    } else { Ok(()) };
    let hotkey = saved?;
    restarted?;
    Ok(hotkey)
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
    let upscale_hotkey = if edit.upscale { Some(ensure_magpie(&state, &settings)?) } else { None };
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
        let mut command = std::process::Command::new(&locale_emulator);
        let profiles = runtime::locale_profiles(&locale_emulator)
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
    if let Some(hotkey) = upscale_hotkey {
        schedule_upscale(
            app.clone(),
            executable,
            hotkey,
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
pub(crate) async fn configure_tools(
    locale_emulator: String,
    magpie: String,
    magpie_hotkey: String,
    backup_retention: usize,
    app: tauri::AppHandle,
) -> Result<Settings, Message> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        configure_tools_at(&handle.state::<AppState>(), &locale_emulator, &magpie, &magpie_hotkey, backup_retention)
    }).await.map_err(|e| Message::detail("tools.error", e))?
}

fn configure_tools_at(
    state: &AppState, locale_emulator: &str, magpie: &str, magpie_hotkey: &str, backup_retention: usize,
) -> Result<Settings, Message> {
    runtime::hotkey_keys(magpie_hotkey).map_err(|e| Message::detail("tools.error", e))?;
    if !(1..=1000).contains(&backup_retention) { return Err(Message::new("error.operation")); }
    let locale_emulator = normalize_tool_path(locale_emulator);
    let magpie = normalize_tool_path(magpie);
    if !locale_emulator.is_empty() { tool_path(locale_emulator, "tools.locale_missing")?; }
    let executable = if magpie.is_empty() { None } else { Some(tool_path(magpie, "tools.magpie_missing")?) };
    update(state, |settings| {
        let hotkey = if let Some(executable) = &executable {
            save_magpie_hotkey(state, executable, magpie_hotkey)?
        } else { magpie_hotkey.trim().into() };
        settings.locale_emulator = locale_emulator.into();
        settings.magpie = magpie.into();
        settings.magpie_hotkey = hotkey;
        settings.backup_retention = backup_retention;
        Ok(())
    })
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct MagpieHotkey { magpie: String, hotkey: String }

pub(crate) fn sync_magpie_hotkey(state: &AppState) -> Result<Option<MagpieHotkey>, Message> {
    let mut settings = state.settings.lock().map_err(|_| Message::new("error.config_state"))?;
    if settings.magpie.is_empty() { return Ok(None); }
    let executable = tool_path(&settings.magpie, "tools.magpie_missing")?;
    let hotkey = crate::bundled_tools::magpie_hotkey(&executable, &settings.magpie_hotkey)?;
    if hotkey == settings.magpie_hotkey { return Ok(None); }
    let mut next = settings.clone();
    next.magpie_hotkey = hotkey.clone();
    state.store.save(&next)?;
    *settings = next;
    Ok(Some(MagpieHotkey { magpie: settings.magpie.clone(), hotkey }))
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
    let game = find_game(&state, &id)?;
    if !["normal", "locale"].contains(&launch_mode.as_str()) {
        return Err(Message::new("error.operation"));
    }
    update(&state, |s| {
        let backup_available = !auto_backup || galgear_core::saves::discover(&game, s.game_edits.get(&id))
            .locations.iter().any(|location| location.exists && location.confidence == "confirmed");
        let edit = s.game_edits.entry(id).or_default();
        edit.launch_mode = launch_mode;
        edit.locale_profile = locale_profile;
        edit.auto_backup = auto_backup && backup_available;
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
    if runtime::detect(game.executable.as_deref()) != Status::Running {
        return Err(Message::new("tools.game_stopped"));
    }
    let executable = game
        .executable
        .ok_or_else(|| Message::new("error.exe_missing"))?;
    let hotkey = ensure_magpie(&state, &settings)?;
    runtime::trigger_magpie(Path::new(&executable), &hotkey)
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
    let hotkey_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let handle = hotkey_app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Ok(Some(update)) = sync_magpie_hotkey(&handle.state::<AppState>()) {
                    let _ = handle.emit("magpie-hotkey-updated", update);
                }
            }).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
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
                        let result = automatic_backup(id.clone(), &state).await;
                        let _ = app.emit(
                            "backup-completed",
                            serde_json::json!({"id":id,"created":result.as_ref().ok(),"error":result.as_ref().err()}),
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
    fn saving_and_observing_magpie_hotkeys_updates_both_settings_files() {
        let data = tempfile::tempdir().unwrap();
        let state = AppState::load(data.path().join("data")).unwrap();
        let executable = data.path().join("Magpie.exe");
        std::fs::write(&executable, []).unwrap();
        std::fs::create_dir(data.path().join("config")).unwrap();
        let config = data.path().join("config/config.json");
        std::fs::write(&config, r#"{"shortcuts":{"scale":3137}}"#).unwrap();
        let path = executable.to_str().unwrap();
        let settings = configure_tools_at(&state, "", path, "Alt+Shift+B", 10).unwrap();
        assert_eq!(settings.magpie_hotkey, "Alt+Shift+B");
        assert_eq!(crate::bundled_tools::magpie_hotkey(&executable, "").unwrap(), settings.magpie_hotkey);
        assert!(sync_magpie_hotkey(&state).unwrap().is_none());
        // Simulate Magpie saving its own new shortcut, then a transient partial save.
        std::fs::write(&config, r#"{"shortcuts":{"scale":629}}"#).unwrap();
        assert_eq!(sync_magpie_hotkey(&state).unwrap().unwrap().hotkey, "Ctrl+F6");
        assert_eq!(state.store.load().unwrap().magpie_hotkey, "Ctrl+F6");
        std::fs::write(&config, "{").unwrap();
        assert!(sync_magpie_hotkey(&state).is_err());
        assert_eq!(snapshot(&state).unwrap().magpie_hotkey, "Ctrl+F6");
        std::fs::write(&config, r#"{"shortcuts":{"scale":3139}}"#).unwrap();
        assert_eq!(sync_magpie_hotkey(&state).unwrap().unwrap().hotkey, "Alt+Shift+C");
    }

    #[test]
    fn failed_hotkey_save_preserves_galgear_preferences() {
        let data = tempfile::tempdir().unwrap();
        let state = AppState::load(data.path().join("data")).unwrap();
        let executable = data.path().join("Magpie.exe");
        std::fs::write(&executable, []).unwrap();
        std::fs::create_dir(data.path().join("config")).unwrap();
        let config = data.path().join("config/config.json");
        std::fs::write(&config, r#"{"shortcuts":{"scale":3137,"windowedModeScale":3138}}"#).unwrap();
        let previous = serde_json::to_value(snapshot(&state).unwrap()).unwrap();
        assert!(configure_tools_at(&state, "", executable.to_str().unwrap(), "Alt+Shift+B", 20).is_err());
        assert_eq!(serde_json::to_value(snapshot(&state).unwrap()).unwrap(), previous);
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
            executable
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

    #[test]
    #[ignore = "Starts an isolated Magpie; set GALGEAR_BUNDLED_SMOKE_ROOT to a disposable copy"]
    fn saving_hotkey_restarts_running_magpie_with_the_new_configuration() {
        let root = PathBuf::from(std::env::var("GALGEAR_BUNDLED_SMOKE_ROOT").unwrap());
        let executable = root.join(crate::bundled_tools::MAGPIE);
        assert_eq!(runtime::detect(executable.to_str()), Status::Stopped);
        let data = tempfile::tempdir().unwrap();
        let state = AppState::load(data.path().to_owned()).unwrap();
        struct Cleanup<'a>(&'a AppState);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = shutdown_magpie(self.0);
            }
        }
        let _cleanup = Cleanup(&state);
        let path = executable.to_str().unwrap();
        let original = crate::bundled_tools::magpie_hotkey(&executable, "").unwrap();
        let settings = configure_tools_at(&state, "", path, &original, 10).unwrap();
        ensure_magpie(&state, &settings).unwrap();
        let previous = state.tools.magpie.lock().unwrap().as_ref().unwrap().id();
        let saved = configure_tools_at(&state, "", path, "Alt+Shift+B", 10).unwrap();
        assert_eq!(saved.magpie_hotkey, "Alt+Shift+B");
        assert_eq!(runtime::detect(executable.to_str()), Status::Running);
        let current = state.tools.magpie.lock().unwrap().as_ref().unwrap().id();
        assert_ne!(current, previous);
        assert_eq!(crate::bundled_tools::magpie_hotkey(&executable, "").unwrap(), "Alt+Shift+B");
        // A repeated save leaves the same live process in place.
        configure_tools_at(&state, "", path, "Shift+Alt+B", 10).unwrap();
        assert_eq!(state.tools.magpie.lock().unwrap().as_ref().unwrap().id(), current);
    }

    #[test]
    #[ignore = "Starts the bundled Magpie; set GALGEAR_BUNDLED_SMOKE_ROOT to the app folder"]
    fn bundled_magpie_starts_with_its_portable_configuration() {
        let root = PathBuf::from(std::env::var("GALGEAR_BUNDLED_SMOKE_ROOT").unwrap());
        let executable = root.join(crate::bundled_tools::MAGPIE);
        assert_eq!(runtime::detect(executable.to_str()), Status::Stopped);
        let data = tempfile::tempdir().unwrap();
        let state = AppState::load(data.path().to_owned()).unwrap();
        let settings = Settings { magpie: executable.to_string_lossy().into_owned(), ..Default::default() };
        let result = ensure_magpie(&state, &settings);
        let running = runtime::detect(executable.to_str());
        // Only stop the child started by this test, never a pre-existing instance.
        shutdown_magpie(&state).unwrap();
        assert_eq!(result.unwrap(), "Alt+Shift+A");
        assert_eq!(running, Status::Running);
    }
}
