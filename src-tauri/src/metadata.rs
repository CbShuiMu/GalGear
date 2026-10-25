//! Metadata lookup, association and background enrichment.
use crate::{
    data_sources, state::AppState, state::find_game, state::persist_game, state::snapshot,
};
use galgear_core::{Game, catalog, providers};
use galgear_i18n::Message;
use tauri::{Emitter, State};

#[tauri::command]
pub(crate) async fn search_metadata(
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
pub(crate) async fn select_metadata(
    id: String,
    url: String,
    expected_url: Option<String>,
    target: Option<galgear_core::game_details::MetadataTarget>,
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
    let cover = if target.is_none() && game.cover.is_none() && metadata.cover_url.is_some() {
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
    if !matches!(
        target,
        Some(galgear_core::game_details::MetadataTarget::Tags)
    ) {
        game.metadata_warning = match &cover {
            Some(Err(error)) => Some(error.clone()),
            Some(Ok((_, _, warning))) => warning.clone(),
            None => None,
        };
    }
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        let edit = galgear_core::game_details::associate(
            &game,
            next.game_edits.get(&id),
            source,
            metadata,
            target.unwrap_or(galgear_core::game_details::MetadataTarget::Details),
        );
        next.game_edits.insert(id, edit);
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
pub(crate) async fn enrich_library(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), Message> {
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
            g.metadata.as_ref().is_some_and(|m| {
                !m.url.is_empty() && m.schema_version < providers::METADATA_SCHEMA_VERSION
            }) && !settings
                .game_edits
                .get(&g.id)
                .is_some_and(|edit| edit.setup_pending)
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
                None,
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
    }
    Ok(())
}
