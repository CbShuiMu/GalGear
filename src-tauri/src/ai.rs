//! AI settings return provider status without exposing stored API keys.
use crate::{preferences, state::AppState, state::snapshot};
use galgear_core::{Settings, ai, credentials};
use galgear_i18n::Message;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ProviderStatus {
    provider: String,
    configured: bool,
    model: String,
}

fn status(state: &AppState, provider: &str) -> Result<ProviderStatus, Message> {
    let credentials = credentials::load(&ai::credential_target(&state.store.path, provider)?)?;
    Ok(ProviderStatus {
        provider: provider.into(),
        configured: !credentials.token.is_empty(),
        model: snapshot(state)?
            .ai_models
            .get(provider)
            .cloned()
            .unwrap_or_default(),
    })
}

#[tauri::command]
pub fn get_ai_providers(state: State<'_, AppState>) -> Result<Vec<ProviderStatus>, Message> {
    ai::PROVIDERS
        .iter()
        .map(|provider| status(&state, provider))
        .collect()
}

#[tauri::command]
pub fn set_active_ai_provider(
    provider: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    preferences::update(&state, |settings| ai::select_provider(settings, &provider))
}

#[tauri::command]
pub async fn list_ai_models(
    provider: String,
    token: String,
    state: State<'_, AppState>,
) -> Result<Vec<ai::Model>, Message> {
    ai::models_url(&provider)?;
    let path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let token = ai::token_for(&path, &provider, &token)?;
        ai::list_models(&provider, &token)
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[tauri::command]
pub async fn save_ai_provider(
    provider: String,
    token: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<ProviderStatus, Message> {
    ai::models_url(&provider)?;
    if model.trim().is_empty() {
        return Err(Message::new("ai.model_unavailable"));
    }
    let path = state.store.path.clone();
    let source = provider.clone();
    let selected = model.clone();
    // Validate the chosen model with this key before replacing a working configuration.
    tauri::async_runtime::spawn_blocking(move || {
        let token = ai::token_for(&path, &source, &token)?;
        let models = ai::list_models(&source, &token)?;
        if !models.iter().any(|model| model.id == selected) {
            return Err(Message::new("ai.model_unavailable"));
        }
        ai::save_token(&path, &source, token)
    })
    .await
    .map_err(|_| Message::new("error.operation"))??;
    preferences::update(&state, |settings| {
        settings.ai_models.insert(provider.clone(), model);
        Ok(())
    })?;
    status(&state, &provider)
}

#[tauri::command]
pub fn clear_ai_provider(
    provider: String,
    state: State<'_, AppState>,
) -> Result<ProviderStatus, Message> {
    credentials::clear(&ai::credential_target(&state.store.path, &provider)?)?;
    preferences::update(&state, |settings| {
        settings.ai_models.remove(&provider);
        Ok(())
    })?;
    status(&state, &provider)
}
