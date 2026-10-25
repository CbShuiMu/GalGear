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
) -> Result<ResultData, Message> {
    let game = find_game(&state, &id)?;
    game_setup::validate(&source, &url, &executable, confirmed)?;
    let domain = snapshot(&state)?.fan_domain;
    let credentials = data_sources::credentials_for(&state, &source)?;
    let metadata = tauri::async_runtime::spawn_blocking(move || {
        galgear_core::data_sources::metadata(&domain, &url, &credentials)
    })
    .await
    .map_err(|error| Message::detail("error.network", error))??;
    let cover = if game.cover.is_none() && metadata.cover_url.is_some() {
        let metadata = metadata.clone();
        let cache = state.store.path.with_file_name("covers");
        Some(
            tauri::async_runtime::spawn_blocking(move || {
                providers::save_cover(&game, &metadata, &cache)
            })
            .await
            .map_err(|error| Message::detail("error.network", error))?,
        )
    } else {
        None
    };
    let mut game = find_game(&state, &id)?;
    let settings = preferences::update(&state, |settings| {
        let edit = game_setup::prepare(
            &game,
            settings.game_edits.get(&id),
            &source,
            &executable,
            metadata,
        )?;
        settings.game_edits.insert(id.clone(), edit);
        if let Some(Ok((_, path, _))) = &cover {
            settings.cover_overrides.insert(id.clone(), path.clone());
        }
        Ok(())
    })?;
    if let Some(Ok((data, _, warning))) = cover {
        game.cover = Some(data);
        game.metadata_warning = warning;
    } else if let Some(Err(error)) = cover {
        game.metadata_warning = Some(error);
    }
    catalog::apply_edits(&mut game, &settings);
    Ok(ResultData {
        game: persist_game(&state, game)?,
        settings,
    })
}
