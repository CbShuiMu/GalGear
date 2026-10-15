//! Data source settings expose status, never stored secrets, to the UI.
use super::{AppState, features};
use galgear_core::{
    Settings, credentials,
    data_sources::{self, Credentials, SourceStatus},
};
use galgear_i18n::Message;
use tauri::State;

#[tauri::command]
pub fn get_data_sources(state: State<'_, AppState>) -> Result<Vec<SourceStatus>, Message> {
    data_sources::statuses(&state.store.path)
}
#[tauri::command]
pub async fn save_data_source(
    source: String,
    token: String,
    client_id: String,
    client_secret: String,
    state: State<'_, AppState>,
) -> Result<Vec<SourceStatus>, Message> {
    data_sources::validate_source(&source)?;
    if source == "2dfan" {
        return Err(Message::new("auth.invalid"));
    }
    let path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut value = data_sources::load(&path, &source)?;
        if !token.trim().is_empty() {
            value.token = token.trim().into();
        }
        if !client_id.trim().is_empty() {
            value.client_id = client_id.trim().into();
        }
        if !client_secret.trim().is_empty() {
            value.client_secret = client_secret.trim().into();
        }
        data_sources::check(&source, &value)?;
        data_sources::save(&path, &source, &value)?;
        data_sources::statuses(&path)
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}
#[tauri::command]
pub fn clear_data_source(
    source: String,
    state: State<'_, AppState>,
) -> Result<Vec<SourceStatus>, Message> {
    if source == "2dfan" {
        return Err(Message::new("auth.invalid"));
    }
    credentials::clear(&data_sources::credential_target(
        &state.store.path,
        &source,
    )?)?;
    data_sources::statuses(&state.store.path)
}
#[tauri::command]
pub fn set_data_source(source: String, state: State<'_, AppState>) -> Result<Settings, Message> {
    data_sources::validate_source(&source)?;
    features::update(&state, |settings| {
        settings.metadata_provider = source;
        Ok(())
    })
}
pub fn credentials_for(state: &AppState, source: &str) -> Result<Credentials, Message> {
    if source == "2dfan" {
        Ok(Credentials::default())
    } else {
        data_sources::load(&state.store.path, source)
    }
}
