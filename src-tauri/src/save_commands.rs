//! Save locations, snapshots and restore transactions.
use crate::{
    dialogs, preferences::update, state::AppState, state::begin_write, state::find_game,
    state::persist_game, state::snapshot,
};
use galgear_core::{Game, Settings, catalog, saves, snapshots};
use galgear_i18n::Message;
use std::{path::Path, path::PathBuf, sync::Mutex};
use tauri::State;

#[derive(Clone)]
struct SaveSuggestion {
    folder: String,
    report: saves::SaveReport,
}

#[derive(Default)]
pub(crate) struct SaveState {
    plan: Mutex<Option<snapshots::Preview>>,
    observations:
        Mutex<std::collections::BTreeMap<String, std::sync::Arc<saves::observation::Observation>>>,
    suggestions: Mutex<std::collections::BTreeMap<String, SaveSuggestion>>,
}

#[derive(serde::Serialize)]
pub(crate) struct SaveHistory {
    snapshots: Vec<snapshots::Snapshot>,
    current_snapshot: Option<String>,
    notice: Option<Message>,
}

#[tauri::command]
pub(crate) async fn detect_saves(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, Message> {
    let game = find_game(&state, &id)?;
    let pending = state
        .saves
        .suggestions
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .get(&id)
        .filter(|suggestion| suggestion.folder == game.folder)
        .map(|suggestion| suggestion.report.clone());
    if let Some(report) = pending {
        return Ok(serde_json::json!({"id": id, "report": report}));
    }
    let edit = snapshot(&state)?.game_edits.get(&id).cloned();
    let report =
        tauri::async_runtime::spawn_blocking(move || saves::discover(&game, edit.as_ref()))
            .await
            .map_err(|e| Message::detail("error.operation", e))?;
    Ok(serde_json::json!({ "id": id, "report": report }))
}

#[tauri::command]
pub(crate) async fn find_save_with_ai(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, Message> {
    let game = find_game(&state, &id)?;
    let original_folder = game.folder.clone();
    let settings = snapshot(&state)?;
    let original_edit = settings.game_edits.get(&id).cloned().unwrap_or_default();
    let config = state.store.path.clone();
    let data = backup_data(&config, &settings);
    let report = tauri::async_runtime::spawn_blocking(move || {
        let report = saves::ai_find::find(&config, &settings, &game)?;
        let roots: Vec<_> = report
            .locations
            .iter()
            .flat_map(saves::location_roots)
            .collect();
        snapshots::validate_roots(&roots, Path::new(&game.folder), &data)?;
        Ok::<_, Message>(report)
    })
    .await
    .map_err(|_| Message::new("error.operation"))??;
    let _guard = begin_write(&state)?;
    if find_game(&state, &id)?.folder != original_folder {
        return Err(Message::new("save.ai_not_found"));
    }
    let settings = update(&state, |settings| {
        let edit = settings.game_edits.entry(id.clone()).or_default();
        // 网络请求期间的手动修改优先，迟到的 AI 回答不能覆盖它。
        if edit.save_files != original_edit.save_files
            || edit.save_paths != original_edit.save_paths
            || edit.save_path != original_edit.save_path
            || edit.save_path_auto != original_edit.save_path_auto
        {
            return Err(Message::new("error.operation"));
        }
        apply_selection(edit, &report);
        Ok(())
    })?;
    state
        .saves
        .suggestions
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .remove(&id);
    Ok(serde_json::json!({"id": id, "report": report, "settings": settings}))
}

fn apply_selection(edit: &mut galgear_core::GameEdit, report: &saves::SaveReport) {
    edit.save_files = report
        .locations
        .iter()
        .flat_map(|location| location.files.iter().cloned())
        .collect();
    edit.save_paths = report
        .locations
        .iter()
        .filter(|location| location.files.is_empty())
        .map(|location| location.path.clone())
        .collect();
    edit.save_path = (edit.save_files.is_empty() && edit.save_paths.len() == 1)
        .then(|| edit.save_paths[0].clone());
    // 已验证的 AI 范围固定保存，后续扫描不能按时间戳更换文件或目录。
    edit.save_path_auto = false;
}

fn manual_selection(game: &Game, path: &Path) -> Result<saves::SaveReport, Message> {
    let path = path
        .canonicalize()
        .map_err(|error| Message::detail("error.game_folder", error))?;
    let mut report = saves::discover(game, None);
    report.locations.retain(|location| {
        !location.files.is_empty()
            && Path::new(&location.path).canonicalize().ok().as_ref() == Some(&path)
    });
    if report.locations.is_empty() {
        report.locations.push(saves::SaveLocation {
            path: galgear_core::config::display_path(&path),
            files: Vec::new(),
            source: "manual".into(),
            confidence: "confirmed".into(),
            exists: true,
            readable: true,
        });
    }
    Ok(report)
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
    if report
        .locations
        .iter()
        .any(|location| !location.files.is_empty())
        && !report
            .locations
            .iter()
            .any(|location| location.files.is_empty() && location.confidence == "confirmed")
    {
        return choose_save_files(id, state).await;
    }
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
    let _guard = begin_write(&state)?;
    if find_game(&state, &id)?.folder != game.folder {
        return Err(Message::new("error.game_folder"));
    }
    let selection_game = game.clone();
    let selection_path = path.clone();
    let data = backup_data(&state.store.path, &snapshot(&state)?);
    let selected = tauri::async_runtime::spawn_blocking(move || {
        let selected = manual_selection(&selection_game, &selection_path)?;
        let roots: Vec<_> = selected
            .locations
            .iter()
            .flat_map(saves::location_roots)
            .collect();
        snapshots::validate_roots(&roots, Path::new(&selection_game.folder), &data)?;
        Ok::<_, Message>(selected)
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))??;
    dialogs::remember(&state, &role, &path)?;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        let edit = next.game_edits.entry(id.clone()).or_default();
        apply_selection(edit, &selected);
        state.store.save(&next)?;
        catalog::apply_edits(&mut game, &next);
        *settings = next;
    }
    persist_game(&state, game)?;
    state
        .saves
        .suggestions
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .remove(&id);
    let settings = snapshot(&state)?;
    let mut result = detect_saves(id, state.clone()).await?;
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
    tauri::async_runtime::spawn_blocking(move || {
        let report = saves::discover(&game, edit.as_ref());
        let has_confirmed = report
            .locations
            .iter()
            .any(|location| location.confidence == "confirmed");
        let mut folders = std::collections::BTreeSet::new();
        for location in report.locations {
            if (path.is_empty() && (!has_confirmed || location.confidence == "confirmed"))
                || location.path == path
            {
                let folder = PathBuf::from(location.path);
                if folder.is_dir() {
                    folders.insert(folder);
                }
            }
        }
        if folders.is_empty() {
            return Err(Message::new("error.game_folder"));
        }
        for folder in folders {
            crate::folders::open(&folder)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| Message::detail("error.open_folder", e))?
}

#[tauri::command]
pub(crate) async fn confirm_save(
    id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let search_game = game.clone();
    let edit = settings.game_edits.get(&id).cloned();
    let local =
        tauri::async_runtime::spawn_blocking(move || saves::discover(&search_game, edit.as_ref()))
            .await
            .map_err(|error| Message::detail("error.operation", error))?;
    let suggestion = state
        .saves
        .suggestions
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .get(&id)
        .filter(|suggestion| suggestion.folder == game.folder)
        .cloned();
    let suggested = suggestion
        .as_ref()
        .map(|suggestion| &suggestion.report)
        .filter(|report| {
            report
                .locations
                .iter()
                .any(|location| location.path == path)
        });
    let selected: Vec<_> = if let Some(report) = suggested {
        report
            .locations
            .iter()
            .filter(|location| location.source == "ai" || location.path == path)
            .collect()
    } else {
        local
            .locations
            .iter()
            .filter(|location| location.path == path)
            .collect()
    };
    if selected.is_empty() {
        return Err(Message::new("error.game_folder"));
    }
    let roots: Vec<_> = selected
        .iter()
        .flat_map(|location| saves::location_roots(location))
        .collect();
    snapshots::validate_roots(
        &roots,
        Path::new(&game.folder),
        &backup_data(&state.store.path, &settings),
    )?;
    let result = update(&state, |settings| {
        let edit = settings.game_edits.entry(id.clone()).or_default();
        edit.save_files = selected
            .iter()
            .flat_map(|location| location.files.iter().cloned())
            .collect();
        edit.save_paths = selected
            .iter()
            .filter(|location| location.files.is_empty())
            .map(|location| location.path.clone())
            .collect();
        edit.save_path = (edit.save_files.is_empty() && edit.save_paths.len() == 1)
            .then(|| edit.save_paths[0].clone());
        edit.save_path_auto = false;
        Ok(())
    })?;
    state
        .saves
        .suggestions
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .remove(&id);
    Ok(result)
}

/// 保存前后诊断只记录元数据，比较完成后仍由用户确认文件范围。
#[tauri::command]
pub(crate) async fn observe_saves(
    id: String,
    finish: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, Message> {
    let game = find_game(&state, &id)?;
    let original_folder = game.folder.clone();
    let observer = state.tools.observer();
    let baseline = state
        .saves
        .observations
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .get(&id)
        .cloned();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        snapshots::require_stopped(observer.detect(game.executable.as_deref()))?;
        if finish {
            let baseline = baseline.ok_or_else(|| Message::new("save.observation_expired"))?;
            Ok((None, baseline.finish(&game)?))
        } else {
            Ok((
                Some(std::sync::Arc::new(saves::observation::Observation::begin(
                    &game,
                )?)),
                saves::SaveReport::default(),
            ))
        }
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))?;
    let (baseline, report): (
        Option<std::sync::Arc<saves::observation::Observation>>,
        saves::SaveReport,
    ) = outcome?;
    if find_game(&state, &id)?.folder != original_folder {
        return Err(Message::new("save.observation_expired"));
    }
    if let Some(baseline) = baseline {
        state
            .saves
            .observations
            .lock()
            .map_err(|_| Message::new("error.operation"))?
            .insert(id.clone(), baseline);
    } else {
        state
            .saves
            .observations
            .lock()
            .map_err(|_| Message::new("error.operation"))?
            .remove(&id);
        state
            .saves
            .suggestions
            .lock()
            .map_err(|_| Message::new("error.operation"))?
            .insert(
                id.clone(),
                SaveSuggestion {
                    folder: find_game(&state, &id)?.folder,
                    report: report.clone(),
                },
            );
    }
    Ok(serde_json::json!({"id":id,"report":report,"observing":!finish}))
}

/// 手动多选散落存档，复用文件范围校验及配置持久化。
#[tauri::command]
pub(crate) async fn choose_save_files(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, Message> {
    let game = find_game(&state, &id)?;
    let folder = PathBuf::from(&game.folder);
    let chosen =
        tauri::async_runtime::spawn_blocking(move || dialogs::dialog(Some(folder)).pick_files())
            .await
            .map_err(|error| Message::detail("error.operation", error))?;
    let Some(files) = chosen else { return Ok(None) };
    let _guard = begin_write(&state)?;
    if find_game(&state, &id)?.folder != game.folder {
        return Err(Message::new("error.game_folder"));
    }
    let settings = snapshot(&state)?;
    let paths: Vec<_> = files
        .iter()
        .map(|file| galgear_core::config::display_path(file))
        .collect();
    for file in &files {
        saves::validate_file(file)?;
    }
    let paths = snapshots::validate_roots(
        &paths,
        Path::new(&game.folder),
        &backup_data(&state.store.path, &settings),
    )?;
    update(&state, |settings| {
        let edit = settings.game_edits.entry(id.clone()).or_default();
        edit.save_files = paths;
        edit.save_path = None;
        edit.save_paths.clear();
        edit.save_path_auto = false;
        Ok(())
    })?;
    state
        .saves
        .suggestions
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .remove(&id);
    let mut result = detect_saves(id, state.clone()).await?;
    result["settings"] = serde_json::to_value(snapshot(&state)?)
        .map_err(|error| Message::detail("error.operation", error))?;
    Ok(Some(result))
}

#[tauri::command]
pub(crate) async fn choose_backup_folder(
    state: State<'_, AppState>,
) -> Result<galgear_core::Settings, Message> {
    let current = backup_data(&state.store.path, &snapshot(&state)?);
    let start = crate::dialogs::start(&state, "backup", std::slice::from_ref(&current))?;
    let selected =
        tauri::async_runtime::spawn_blocking(move || crate::dialogs::dialog(start).pick_folder())
            .await
            .map_err(|error| Message::detail("backup.error", error))?;
    let Some(selected) = selected else {
        return snapshot(&state);
    };
    let _guard = begin_write(&state)?;
    let settings = snapshot(&state)?;
    let current = backup_data(&state.store.path, &settings);
    if let (Ok(current), Ok(selected)) = (
        std::fs::canonicalize(&current),
        std::fs::canonicalize(&selected),
    ) && current == selected
    {
        return Ok(settings);
    }
    let games = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .games
        .clone();
    let old = current.clone();
    let destination = selected.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut entries = snapshots::registered_games(&old)?;
        for game in games {
            entries.entry(game.id).or_insert(game.title);
        }
        for (id, title) in entries {
            snapshots::register_game(&destination, &id, &title)?;
            snapshots::copy_history(&old, &destination, &id)?;
        }
        Ok::<_, Message>(())
    })
    .await
    .map_err(|error| Message::detail("backup.error", error))??;
    crate::preferences::update(&state, |settings| {
        let source = galgear_core::config::display_path(&current);
        if !settings.backup_legacy_folders.contains(&source) {
            settings.backup_legacy_folders.push(source);
        }
        settings.backup_folder = galgear_core::config::display_path(&selected);
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn open_backup_folder(state: State<'_, AppState>) -> Result<(), Message> {
    let directory = backup_data(&state.store.path, &snapshot(&state)?);
    std::fs::create_dir_all(&directory).map_err(|error| Message::detail("backup.error", error))?;
    crate::folders::open(&directory)
}

pub(crate) fn backup_data(config_path: &Path, settings: &galgear_core::Settings) -> PathBuf {
    if settings.backup_folder.is_empty() {
        config_path.with_file_name("save-backups")
    } else {
        PathBuf::from(&settings.backup_folder)
    }
}

pub(crate) fn prepare_backup_data(
    config_path: &Path,
    settings: &galgear_core::Settings,
    game: &Game,
) -> Result<PathBuf, Message> {
    let data = backup_data(config_path, settings);
    snapshots::register_game(&data, &game.id, &game.title)?;
    // 索引建立后，同一根目录下旧 ID 目录仍由 migrate 优先读取。
    snapshots::migrate(&data, &data, &game.id)?;
    snapshots::migrate(&config_path.with_file_name("save-backups"), &data, &game.id)?;
    for source in &settings.backup_legacy_folders {
        snapshots::migrate(Path::new(source), &data, &game.id)?;
    }
    snapshots::migrate(
        &snapshots::game_data(Path::new(&game.folder)),
        &data,
        &game.id,
    )?;
    Ok(data)
}

pub(crate) fn migrate_known_backups(
    config_path: &Path,
    settings: &galgear_core::Settings,
    games: &[Game],
) -> Vec<Message> {
    let mut seen = std::collections::BTreeSet::new();
    let mut warnings = Vec::new();
    for game in games
        .iter()
        .chain(&settings.manual_games)
        .chain(settings.excluded_entries.values())
    {
        if seen.insert(&game.id)
            && let Err(error) = prepare_backup_data(config_path, settings, game)
        {
            warnings.push(error);
        }
    }
    warnings
}

pub(crate) fn confirmed_roots(
    state: &AppState,
    game: &Game,
    settings: &Settings,
) -> Result<Vec<String>, Message> {
    let roots = saves::discover(game, settings.game_edits.get(&game.id))
        .locations
        .into_iter()
        .filter(|l| l.confidence == "confirmed" && (l.exists || l.source == "manual"))
        .flat_map(|l| saves::location_roots(&l))
        .collect::<Vec<_>>();
    snapshots::validate_roots(
        &roots,
        Path::new(&game.folder),
        &backup_data(&state.store.path, settings),
    )
}

#[tauri::command]
pub(crate) async fn save_history(
    id: String,
    state: State<'_, AppState>,
) -> Result<SaveHistory, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let config_path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        let history = snapshots::list(&data, &id)?;
        let roots = saves::discover(&game, settings.game_edits.get(&id))
            .locations
            .into_iter()
            .filter(|location| {
                location.confidence == "confirmed"
                    && (location.exists || location.source == "manual")
            })
            .flat_map(|location| saves::location_roots(&location))
            .collect::<Vec<_>>();
        let current_snapshot = snapshots::validate_roots(&roots, Path::new(&game.folder), &data)
            .and_then(|roots| snapshots::matching_current(&data, &history, &roots))
            .unwrap_or(None);
        Ok(SaveHistory {
            snapshots: history,
            current_snapshot,
            notice: None,
        })
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))?
}

pub(crate) async fn backup(id: String, note: String, state: &AppState) -> Result<bool, Message> {
    backup_with_mode(id, note, state, false).await
}

pub(crate) async fn automatic_backup(id: String, state: &AppState) -> Result<bool, Message> {
    backup_with_mode(id, "Automatic backup".into(), state, true).await
}

async fn backup_with_mode(
    id: String,
    note: String,
    state: &AppState,
    automatic: bool,
) -> Result<bool, Message> {
    let _guard = begin_write(state)?;
    let game = find_game(state, &id)?;
    let settings = snapshot(state)?;
    let roots = confirmed_roots(state, &game, &settings)?;
    let config_path = state.store.path.clone();
    let observer = state.tools.observer();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        snapshots::recover(
            &data,
            &id,
            &roots,
            observer.detect(game.executable.as_deref()),
        )?;
        let create = if automatic {
            snapshots::create_automatic_named_checked
        } else {
            snapshots::create_unique_named_checked
        };
        create(
            &data,
            (&id, &game.title),
            &roots,
            &note,
            settings.backup_retention,
            || observer.detect(game.executable.as_deref()),
        )
        .map(|result| result.created)
    })
    .await
    .map_err(|e| Message::detail("backup.error", e))?
}

#[tauri::command]
pub(crate) async fn backup_saves(
    id: String,
    note: String,
    state: State<'_, AppState>,
) -> Result<SaveHistory, Message> {
    let created = backup(id.clone(), note, &state).await?;
    let mut history = save_history(id, state).await?;
    history.notice = Some(Message::new(if created {
        "backup.success"
    } else {
        "backup.duplicate"
    }));
    Ok(history)
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
    let observer = state.tools.observer();
    let preview = tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        snapshots::preview(
            &data,
            &id,
            &snapshot_id,
            &roots,
            observer.detect(game.executable.as_deref()),
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
) -> Result<SaveHistory, Message> {
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
    let observer = state.tools.observer();
    tauri::async_runtime::spawn_blocking(move || {
        let data = prepare_backup_data(&config_path, &settings, &game)?;
        snapshots::restore_checked(&data, &plan, settings.backup_retention, || {
            observer.detect(game.executable.as_deref())
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
) -> Result<SaveHistory, Message> {
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
    use galgear_core::runtime;
    use std::fs;

    #[cfg(windows)]
    #[test]
    fn air_manual_selection_and_ai_configuration_back_up_only_selected_files() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("AIR");
        std::fs::create_dir(&folder).unwrap();
        for name in ["air.exe", "SAVE.SAV", "READ.SAV", "resource.dat"] {
            std::fs::write(folder.join(name), name.as_bytes()).unwrap();
        }
        let game = catalog::manual_game(&folder.join("air.exe"), "AIR".into()).unwrap();
        let report = manual_selection(&game, &folder).unwrap();
        let mut edit = galgear_core::GameEdit::default();
        apply_selection(&mut edit, &report);
        assert_eq!(edit.save_files.len(), 2);
        assert!(edit.save_path.is_none() && edit.save_paths.is_empty());
        let persisted: galgear_core::GameEdit =
            serde_json::from_str(&serde_json::to_string(&edit).unwrap()).unwrap();
        let selected = saves::discover(&game, Some(&persisted));
        assert!(
            selected
                .locations
                .iter()
                .all(|location| location.confidence == "confirmed")
        );
        let roots: Vec<_> = selected
            .locations
            .iter()
            .flat_map(saves::location_roots)
            .collect();
        let data = temp.path().join("backup");
        let roots = snapshots::validate_roots(&roots, &folder, &data).unwrap();
        let saved = snapshots::create(
            &data,
            &game.id,
            &roots,
            "",
            10,
            galgear_core::runtime::Status::Stopped,
        )
        .unwrap();
        assert_eq!(saved.files.len(), 2);
        assert!(
            saved
                .files
                .iter()
                .all(|file| ["READ.SAV", "SAVE.SAV"].contains(&file.path.as_str()))
        );
    }

    #[test]
    fn automatic_backup_reuses_matching_manual_history_after_game_stops() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("game");
        let saves = folder.join("savedata");
        fs::create_dir_all(&saves).unwrap();
        let executable = folder.join("galgear-backup-test.exe");
        fs::write(&executable, []).unwrap();
        let game = catalog::manual_game(&executable, "Backup test".into()).unwrap();
        let state = AppState::load(temp.path().join("config")).unwrap();
        state.catalog.lock().unwrap().games.push(game.clone());
        state.settings.lock().unwrap().game_edits.insert(
            game.id.clone(),
            galgear_core::GameEdit {
                save_path: Some(galgear_core::config::display_path(&saves)),
                auto_backup: true,
                ..Default::default()
            },
        );
        assert_eq!(
            runtime::detect(game.executable.as_deref()),
            runtime::Status::Stopped
        );
        let data = backup_data(&state.store.path, &crate::state::snapshot(&state).unwrap());
        tauri::async_runtime::block_on(async {
            fs::write(saves.join("slot.sav"), b"manual save").unwrap();
            fs::write(
                saves.join("system.ksd"),
                br#"(const) %["trail_custom_exit_end" => 9, "cg_scene" => 1]"#,
            )
            .unwrap();
            assert!(
                backup(game.id.clone(), "Manual backup".into(), &state)
                    .await
                    .unwrap()
            );
            let original = snapshots::list(&data, &game.id).unwrap().remove(0);
            fs::write(saves.join("slot.sav"), b"changed save").unwrap();
            assert!(automatic_backup(game.id.clone(), &state).await.unwrap());
            // An older match must be reused even when the latest backup differs.
            fs::write(saves.join("slot.sav"), b"manual save").unwrap();
            fs::write(
                saves.join("system.ksd"),
                br#"(const) %["cg_scene" => 1, "trail_custom_exit_end" => 10]"#,
            )
            .unwrap();
            assert!(!automatic_backup(game.id.clone(), &state).await.unwrap());
            let history = snapshots::list(&data, &game.id).unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(
                snapshots::matching_current(
                    &data,
                    &history,
                    &[galgear_core::config::display_path(&saves)],
                )
                .unwrap(),
                Some(original.id)
            );
        });
    }

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
    fn preparation_merges_default_and_custom_history_into_named_global_folder() {
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

        assert_eq!(data, custom);
        assert!(data.join("game").is_dir());
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
                snapshots::list(&backup_data(&config, &settings), &game.id)
                    .unwrap()
                    .len(),
                1
            );
            assert!(!legacy.join(&game.id).exists());
        }
    }

    #[test]
    fn unavailable_game_can_access_global_history_without_creating_game_directory() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("settings.json");
        let legacy = temp.path().join("save-backups");
        let mut game = game_with_legacy_backup(temp.path(), "game", &legacy);
        game.folder = galgear_core::config::display_path(&temp.path().join("offline"));

        assert_eq!(
            migrate_known_backups(&config, &Settings::default(), &[game.clone()]).len(),
            0
        );
        assert_eq!(snapshots::list(&legacy, &game.id).unwrap().len(), 1);
        assert!(!Path::new(&game.folder).exists());
    }
}
