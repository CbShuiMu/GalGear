//! Save locations, snapshots and restore transactions.
use crate::{
    dialogs, preferences::update, state::AppState, state::begin_write, state::find_game,
    state::persist_game, state::snapshot,
};
use galgear_core::{Game, Settings, catalog, runtime, saves, snapshots};
use galgear_i18n::Message;
use std::{path::Path, path::PathBuf, sync::Mutex};
use tauri::State;

#[derive(Default)]
pub(crate) struct SaveState {
    plan: Mutex<Option<snapshots::Preview>>,
}

#[tauri::command]
pub(crate) async fn detect_saves(
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
pub(crate) async fn choose_save_folder(
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
        edit.save_path = Some(path.clone());
        if !edit.save_paths.contains(&path) {
            edit.save_paths.push(path);
        }
        state.store.save(&next)?;
        catalog::apply_edits(&mut game, &next);
        *settings = next;
    }
    persist_game(&state, game)?;
    let settings = snapshot(&state)?;
    let mut result = detect_saves(id, state).await?;
    result["settings"] =
        serde_json::to_value(settings).map_err(|e| Message::detail("error.operation", e))?;
    Ok(Some(result))
}

#[tauri::command]
pub(crate) async fn open_save_folder(
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
pub(crate) fn confirm_save(
    id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
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
        edit.save_path = Some(path.clone());
        if !edit.save_paths.contains(&path) {
            edit.save_paths.push(path);
        }
        Ok(())
    })
}

pub(crate) fn backup_data(game: &Game) -> PathBuf {
    snapshots::game_data(Path::new(&game.folder))
}

pub(crate) fn prepare_backup_data(
    config_path: &Path,
    settings: &Settings,
    game: &Game,
) -> Result<PathBuf, Message> {
    if !Path::new(&game.folder).is_dir() {
        return Err(Message::detail("error.game_folder", &game.folder));
    }
    let data = backup_data(game);
    snapshots::migrate(&config_path.with_file_name("save-backups"), &data, &game.id)?;
    // Keep the retired setting until all games (including offline drives) migrate.
    if !settings.backup_folder.is_empty() {
        snapshots::migrate(Path::new(&settings.backup_folder), &data, &game.id)?;
    }
    Ok(data)
}

pub(crate) fn migrate_known_backups(
    config_path: &Path,
    settings: &Settings,
    games: &[Game],
) -> Vec<Message> {
    let mut seen = std::collections::BTreeSet::new();
    let mut warnings = Vec::new();
    for game in games
        .iter()
        .chain(&settings.manual_games)
        .chain(settings.excluded_entries.values())
    {
        if !seen.insert(&game.id) {
            continue;
        }
        let default = config_path.with_file_name("save-backups");
        let has_legacy = default.join(&game.id).exists()
            || (!settings.backup_folder.is_empty()
                && Path::new(&settings.backup_folder).join(&game.id).exists());
        if has_legacy && let Err(error) = prepare_backup_data(config_path, settings, game) {
            warnings.push(error);
        }
    }
    warnings
}

pub(crate) fn confirmed_roots(
    _state: &AppState,
    game: &Game,
    settings: &Settings,
) -> Result<Vec<String>, Message> {
    let roots = saves::discover(game, settings.game_edits.get(&game.id))
        .locations
        .into_iter()
        .filter(|l| l.confidence == "confirmed" && l.exists)
        .map(|l| l.path)
        .collect::<Vec<_>>();
    snapshots::validate_roots(&roots, Path::new(&game.folder), &backup_data(game))
}

#[tauri::command]
pub(crate) async fn save_history(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let config_path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        snapshots::list(&data, &id)
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))?
}

pub(crate) async fn backup(id: String, note: String, state: &AppState) -> Result<(), Message> {
    let _guard = begin_write(state)?;
    let game = find_game(state, &id)?;
    let settings = snapshot(state)?;
    let roots = confirmed_roots(state, &game, &settings)?;
    let config_path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
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
pub(crate) async fn backup_saves(
    id: String,
    note: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    backup(id.clone(), note, &state).await?;
    save_history(id, state).await
}

#[tauri::command]
pub(crate) async fn preview_save_restore(
    id: String,
    snapshot_id: String,
    state: State<'_, AppState>,
) -> Result<snapshots::Preview, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let roots = confirmed_roots(&state, &game, &settings)?;
    let config_path = state.store.path.clone();
    let preview = tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
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
        .saves
        .plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some(preview.clone());
    Ok(preview)
}

#[tauri::command]
pub(crate) async fn restore_saves(
    id: String,
    snapshot_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let plan = state
        .saves
        .plan
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
    let config_path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        snapshots::restore_checked(&data, &plan, settings.backup_retention, || {
            runtime::detect(game.executable.as_deref())
        })
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))??;
    drop(_guard);
    save_history(id, state.clone()).await
}

#[tauri::command]
pub(crate) async fn update_snapshot(
    id: String,
    snapshot_id: String,
    locked: bool,
    note: String,
    delete: bool,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let config_path = state.store.path.clone();
    let game_id = id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        if delete {
            snapshots::delete(&data, &game_id, &snapshot_id)
        } else {
            snapshots::update(&data, &game_id, &snapshot_id, locked, &note)
        }
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))??;
    drop(_guard);
    save_history(id, state.clone()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn game_with_legacy_backup(root: &Path, name: &str, legacy: &Path) -> Game {
        let folder = root.join(name);
        fs::create_dir(&folder).unwrap();
        let executable = folder.join("game.exe");
        fs::write(&executable, []).unwrap();
        let game = catalog::manual_game(&executable, name.into()).unwrap();
        let save = folder.join("savedata");
        fs::create_dir(&save).unwrap();
        fs::write(save.join("slot.sav"), name.as_bytes()).unwrap();
        snapshots::create_named_checked(
            legacy,
            (&game.id, &game.title),
            &[galgear_core::config::display_path(&save)],
            "keep note",
            10,
            || runtime::Status::Stopped,
        )
        .unwrap();
        game
    }

    #[test]
    fn preparation_merges_default_and_custom_history_into_the_game_folder() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("settings.json");
        let legacy = temp.path().join("save-backups");
        let custom = temp.path().join("custom-backups");
        let game = game_with_legacy_backup(temp.path(), "game", &legacy);
        let roots = vec![galgear_core::config::display_path(
            &Path::new(&game.folder).join("savedata"),
        )];
        snapshots::create_named_checked(
            &custom,
            (&game.id, &game.title),
            &roots,
            "custom note",
            10,
            || runtime::Status::Stopped,
        )
        .unwrap();
        let settings = Settings {
            backup_folder: galgear_core::config::display_path(&custom),
            ..Default::default()
        };

        let data = prepare_backup_data(&config, &settings, &game).unwrap();

        assert!(data.starts_with(&game.folder));
        assert_eq!(snapshots::list(&data, &game.id).unwrap().len(), 2);
        assert!(!legacy.join(&game.id).exists());
        assert!(!custom.join(&game.id).exists());
        assert_eq!(
            fs::read(Path::new(&roots[0]).join("slot.sav")).unwrap(),
            b"game"
        );
    }

    #[test]
    fn startup_migration_includes_manual_and_excluded_games() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("settings.json");
        let legacy = temp.path().join("save-backups");
        let manual = game_with_legacy_backup(temp.path(), "manual", &legacy);
        let excluded = game_with_legacy_backup(temp.path(), "excluded", &legacy);
        let mut settings = Settings::default();
        settings.manual_games.push(manual.clone());
        settings
            .excluded_entries
            .insert(excluded.id.clone(), excluded.clone());

        assert!(migrate_known_backups(&config, &settings, &[]).is_empty());
        for game in [&manual, &excluded] {
            assert_eq!(
                snapshots::list(&backup_data(game), &game.id).unwrap().len(),
                1
            );
            assert!(!legacy.join(&game.id).exists());
        }
    }

    #[test]
    fn unavailable_game_keeps_legacy_backups_and_reports_a_warning() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("settings.json");
        let legacy = temp.path().join("save-backups");
        let mut game = game_with_legacy_backup(temp.path(), "game", &legacy);
        game.folder = galgear_core::config::display_path(&temp.path().join("offline"));

        assert_eq!(
            migrate_known_backups(&config, &Settings::default(), &[game.clone()]).len(),
            1
        );
        assert!(legacy.join(&game.id).is_dir());
        assert!(!Path::new(&game.folder).exists());
    }
}
