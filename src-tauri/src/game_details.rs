//! Local game details and metadata edits.
use crate::{preferences::update, state::AppState, state::find_game, state::persist_game};
use galgear_core::{Game, GameEdit, catalog, tags};
use galgear_i18n::Message;
use tauri::State;

#[tauri::command]
pub(crate) fn set_play_status(
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
pub(crate) fn save_game_details(
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
pub(crate) fn edit_title(
    id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
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
pub(crate) fn edit_tags(
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
pub(crate) fn edit_metadata(
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
pub(crate) fn reset_game_tags(id: String, state: State<'_, AppState>) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    let settings = update(&state, |s| {
        let edit = s.game_edits.entry(id).or_default();
        if edit.tags.is_empty() {
            edit.tags.clone_from(&game.tags);
        }
        edit.tag_rules.clear();
        if let Some(metadata) = &game.metadata {
            edit.tags = tags::merge(&edit.tags, &metadata.tag_names(), &edit.tag_rules);
        }
        Ok(())
    })?;
    catalog::apply_edits(&mut game, &settings);
    persist_game(&state, game)
}

#[tauri::command]
pub(crate) fn edit_game(
    id: String,
    mut edit: GameEdit,
    locale_profile: Option<String>,
    state: State<'_, AppState>,
) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    edit.tags = tags::normalize(edit.tags);
    if edit.title.as_ref().is_some_and(|t| t.trim().is_empty()) {
        return Err(Message::new("error.title_required"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    // Metadata and chosen saves are changed only through their own controls.
    edit.tags.clone_from(&game.tags);
    if edit.title.as_ref() == Some(&game.title) {
        edit.title = next.game_edits.get(&id).and_then(|e| e.title.clone());
    }
    if let Some(previous) = next.game_edits.get(&id) {
        edit.metadata.clone_from(&previous.metadata);
        edit.save_path.clone_from(&previous.save_path);
        edit.save_paths.clone_from(&previous.save_paths);
        edit.metadata_overrides
            .clone_from(&previous.metadata_overrides);
        edit.tag_rules.clone_from(&previous.tag_rules);
        edit.source_tags.clone_from(&previous.source_tags);
        edit.summary_brand.clone_from(&previous.summary_brand);
        edit.summary_release_date
            .clone_from(&previous.summary_release_date);
        edit.launch_mode.clone_from(&previous.launch_mode);
        edit.locale_profile.clone_from(&previous.locale_profile);
        edit.play_status = previous.play_status;
        edit.auto_backup = previous.auto_backup;
        edit.description.clone_from(&previous.description);
        edit.metadata_provider
            .clone_from(&previous.metadata_provider);
        edit.setup_pending = previous.setup_pending;
        edit.setup_confirmed = previous.setup_confirmed;
    }
    if let Some(profile) = locale_profile {
        edit.locale_profile = profile;
    }
    tags::remember(&game.tags, &edit.tags, &mut edit.tag_rules);
    next.game_edits.insert(id, edit);
    state.store.save(&next)?;
    catalog::apply_edits(&mut game, &next);
    *settings = next;
    drop(settings);
    persist_game(&state, game)
}
