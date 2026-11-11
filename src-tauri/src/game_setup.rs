//! Interactive setup keeps each game's source and launcher independent.
use crate::{
    data_sources, dialogs, preferences, state::AppState, state::find_game, state::persist_game,
    state::snapshot,
};
use galgear_core::{Game, Settings, catalog, config::display_path, game_setup, providers};
use galgear_i18n::Message;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

#[derive(Serialize)]
pub struct ResultData {
    game: Game,
    settings: Settings,
}

#[derive(Serialize)]
pub struct SetupResult {
    game: Game,
    edit: galgear_core::GameEdit,
}

#[tauri::command]
pub(crate) async fn search_game_metadata(
    id: String,
    source: String,
    state: State<'_, AppState>,
) -> Result<game_setup::matching::SearchResult, Message> {
    let game = find_game(&state, &id)?;
    let query = game_setup::matching::query(
        &game.title,
        std::path::Path::new(&game.folder),
        std::path::Path::new(&game.library),
    );
    if query.is_empty() {
        return Ok(game_setup::matching::resolve(query, Vec::new()));
    }
    let candidates = state
        .metadata_search
        .search(
            state.store.path.clone(),
            source,
            snapshot(&state)?.fan_domain,
            query.clone(),
        )
        .await?;
    Ok(game_setup::matching::resolve(query, candidates))
}

#[derive(Serialize)]
pub(crate) struct Executable {
    path: String,
    name: String,
    icon: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ConfirmedExecutable {
    #[serde(flatten)]
    saved: ResultData,
    runtime: galgear_core::runtime::Status,
}

#[tauri::command]
pub(crate) async fn list_game_executables(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Executable>, Message> {
    let game = find_game(&state, &id)?;
    let cache = state.store.path.with_file_name("icons");
    tauri::async_runtime::spawn_blocking(move || {
        game_setup::executables(std::path::Path::new(&game.folder)).map(|paths| {
            paths
                .into_iter()
                .map(|path| {
                    let icon =
                        galgear_core::launcher_icons::cached_icon(&path, &cache).and_then(|path| {
                            path.file_stem().map(|stem| {
                                format!("http://galgear-icon.localhost/{}", stem.to_string_lossy())
                            })
                        });
                    Executable {
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path: display_path(&path),
                        icon,
                    }
                })
                .collect()
        })
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))?
}

#[tauri::command]
pub(crate) async fn confirm_game_executable(
    id: String,
    executable: String,
    app: tauri::AppHandle,
) -> Result<ConfirmedExecutable, Message> {
    use tauri::Manager;
    // 路径验证及磁盘写入在后台执行，与启动、补丁和存档共用写入互斥。
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let saved = save_executable(&state, &id, &executable)?;
        // UI 的缓存状态可能仍对应旧启动路径，继续启动前检查新路径。
        let runtime = state
            .tools
            .observer()
            .detect(saved.game.executable.as_deref());
        Ok(ConfirmedExecutable { saved, runtime })
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))?
}

fn save_executable(state: &AppState, id: &str, executable: &str) -> Result<ResultData, Message> {
    let _guard = crate::state::begin_write(state)?;
    let mut game = find_game(state, id)?;
    if state.tools.observer().detect(game.executable.as_deref())
        == galgear_core::runtime::Status::Running
    {
        return Err(Message::new("error.already_running"));
    }
    let selected = game_setup::executables(std::path::Path::new(&game.folder))?
        .into_iter()
        .find(|path| display_path(path) == executable)
        .ok_or_else(|| Message::new("setup.executable_required"))?;
    let settings = preferences::update(state, |settings| {
        let edit = settings.game_edits.entry(id.to_owned()).or_default();
        edit.executable = Some(display_path(&selected));
        edit.working_directory = selected.parent().map(display_path);
        edit.launcher_pending = false;
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    Ok(ResultData {
        game: persist_game(state, game)?,
        settings,
    })
}

#[tauri::command]
pub async fn choose_setup_executable(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, Message> {
    let game = find_game(&state, &id)?;
    let role = format!("setup-executable:{id}");
    let context = game
        .executable
        .iter()
        .map(PathBuf::from)
        .chain([PathBuf::from(&game.folder)])
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, &role, &context)?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .add_filter("Game", &["exe"])
            .pick_file()
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))?;
    let Some(path) = selected else {
        return Ok(None);
    };
    dialogs::remember(&state, &role, &path)?;
    Ok(Some(display_path(&path)))
}

#[tauri::command]
pub async fn complete_game_setup(
    id: String,
    source: String,
    url: String,
    executable: String,
    confirmed: bool,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<SetupResult, Message> {
    complete(
        SetupInput {
            id,
            source,
            url,
            executable,
            confirmed,
            rematch: false,
        },
        state,
        app,
    )
    .await
}

/// 在后台刮削并覆盖当前词条，沿用导入确认的校验及持久化流程。
#[tauri::command]
pub(crate) async fn rematch_game_metadata(
    id: String,
    source: String,
    url: String,
    confirmed: bool,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<SetupResult, Message> {
    complete(
        SetupInput {
            id,
            source,
            url,
            executable: String::new(),
            confirmed,
            rematch: true,
        },
        state,
        app,
    )
    .await
}

struct SetupInput {
    id: String,
    source: String,
    url: String,
    executable: String,
    confirmed: bool,
    rematch: bool,
}

async fn complete(
    input: SetupInput,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<SetupResult, Message> {
    let SetupInput {
        id,
        source,
        url,
        executable,
        confirmed,
        rematch,
    } = input;
    let game = find_game(&state, &id)?;
    game_setup::validate(&source, &url, &executable, confirmed)?;
    let domain = snapshot(&state)?.fan_domain;
    let credentials = data_sources::credentials_for(&state, &source).await?;
    let cache = state.store.path.with_file_name("covers");
    let (metadata, cover) = tauri::async_runtime::spawn_blocking(move || {
        let metadata = galgear_core::data_sources::metadata(&domain, &url, &credentials)?;
        let cover = if rematch {
            // 内容寻址缓存不覆盖旧封面文件；无图或下载失败时仍保留原图。
            match galgear_core::data_sources::covers(&domain, &url, &credentials) {
                Ok(covers) => covers.first().map(|cover| {
                    crate::covers::download_source_cover(&cover.image_url, &url, &cache)
                        .map(|cover| (cover.data, cover.path, None))
                }),
                Err(error) => Some(Err(error)),
            }
        } else if game.cover.is_none() && metadata.cover_url.is_some() {
            Some(providers::save_cover(&game, &metadata, &cache))
        } else {
            None
        };
        Ok::<_, Message>((metadata, cover))
    })
    .await
    .map_err(|error| Message::detail("error.network", error))??;
    // 资料配置由 preferences 序列化写入，磁盘持久化不占用异步执行器。
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let mut game = find_game(&state, &id)?;
        let settings = preferences::update(&state, |settings| {
            let edit = if rematch {
                game_setup::prepare_rematch(&game, settings.game_edits.get(&id), &source, metadata)?
            } else {
                game_setup::prepare(
                    &game,
                    settings.game_edits.get(&id),
                    &source,
                    &executable,
                    metadata,
                )?
            };
            settings.game_edits.insert(id.clone(), edit);
            if let Some(Ok((_, path, _))) = &cover {
                settings.cover_overrides.insert(id.clone(), path.clone());
            }
            Ok(())
        })?;
        if let Some(Ok((data, _, warning))) = cover {
            game.cover = Some(data);
            game.cover_artwork = None;
            game.metadata_warning = warning;
        } else if let Some(Err(error)) = cover {
            game.metadata_warning = Some(error);
        }
        catalog::apply_edits(&mut game, &settings);
        // 批量确认只返回本游戏的配置，避免每次传输不断增长的整个游戏库设置。
        let edit = settings.game_edits.get(&id).cloned()
            .ok_or_else(|| Message::new("error.config_state"))?;
        Ok(SetupResult {
            game: persist_game(&state, game)?,
            edit,
        })
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))?
}

#[cfg(all(test, windows))]
mod tests {
    use super::save_executable;
    use crate::state::AppState;
    use galgear_core::{catalog, config::display_path, game_setup, launcher_icons};
    use std::{fs, path::PathBuf};

    #[test]
    fn native_executable_choice_extracts_icon_and_persists_only_valid_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("game");
        fs::create_dir(&folder).unwrap();
        let executable = folder.join("汉化启动器.exe");
        let windows = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        fs::copy(windows.join("System32/notepad.exe"), &executable).unwrap();
        let state = AppState::load(temp.path().join("config")).unwrap();
        let game = catalog::manual_game(&executable, "Fixture".into()).unwrap();
        state.catalog.lock().unwrap().games.push(game.clone());
        state
            .settings
            .lock()
            .unwrap()
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .launcher_pending = true;
        let candidates = game_setup::executables(&folder).unwrap();
        assert_eq!(candidates.len(), 1);
        let icon = launcher_icons::cached_icon(&candidates[0], &temp.path().join("icons")).unwrap();
        assert!(fs::metadata(icon).unwrap().len() > 0);
        assert!(
            save_executable(
                &state,
                &game.id,
                &display_path(&windows.join("System32/notepad.exe"))
            )
            .is_err()
        );
        assert!(state.settings.lock().unwrap().game_edits[&game.id].launcher_pending);
        let result = save_executable(&state, &game.id, &display_path(&candidates[0])).unwrap();
        let saved = state.store.load().unwrap();
        assert_eq!(
            saved.game_edits[&game.id].executable,
            result.game.executable
        );
        assert!(!saved.game_edits[&game.id].launcher_pending);
        assert_eq!(
            saved.game_edits[&game.id].working_directory.as_deref(),
            Some(display_path(&folder).as_str())
        );
    }
}
