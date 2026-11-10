//! Data source settings expose status, never stored secrets, to the UI.
use crate::{preferences, state::AppState};
use galgear_core::{
    Settings, credentials,
    data_sources::{self, Credentials, SourceStatus},
};
use galgear_i18n::Message;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::State;

#[derive(Default)]
pub(crate) struct OAuthState {
    active: Mutex<Option<Arc<AtomicBool>>>,
}
impl OAuthState {
    fn require_idle(&self) -> Result<(), Message> {
        if self
            .active
            .lock()
            .map_err(|_| Message::new("error.operation"))?
            .is_some()
        {
            return Err(Message::new("sources.oauth_pending"));
        }
        Ok(())
    }
}

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
    patch_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SourceStatus>, Message> {
    state.source_oauth.require_idle()?;
    data_sources::validate_source(&source)?;
    if source == "2dfan" {
        return Err(Message::new("auth.invalid"));
    }
    let path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        data_sources::oauth::transaction(|| {
            let mut value = data_sources::load(&path, &source)?;
            if !token.trim().is_empty() {
                value.token = token.trim().into();
                value.refresh_token.clear();
                value.expires_at = 0;
            }
            if !client_id.trim().is_empty() {
                value.client_id = client_id.trim().into();
            }
            if !client_secret.trim().is_empty() {
                value.client_secret = client_secret.trim().into();
            }
            if source == "nextmoe" {
                if value.token.starts_with("nmk_") {
                    return Err(Message::new("sources.user_token_required"));
                }
                if let Some(key) = patch_key.filter(|key| !key.trim().is_empty()) {
                    value.patch_key = key.trim().into();
                    data_sources::moyu::check(&value)?;
                }
            }
            data_sources::check(&source, &value)?;
            data_sources::save(&path, &source, &value)?;
            data_sources::statuses(&path)
        })
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}
#[tauri::command]
pub async fn clear_data_source(
    source: String,
    state: State<'_, AppState>,
) -> Result<Vec<SourceStatus>, Message> {
    state.source_oauth.require_idle()?;
    if source == "2dfan" {
        return Err(Message::new("auth.invalid"));
    }
    let path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        data_sources::oauth::transaction(|| {
            credentials::clear(&data_sources::credential_target(&path, &source)?)?;
            data_sources::statuses(&path)
        })
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}
/// 保存全局默认来源，或指定入口的手动偏好；旧调用省略 context 时仍更新全局默认。
#[tauri::command]
pub fn set_data_source(
    source: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if context.as_deref() != Some("patches") || source != "all" {
        data_sources::validate_source(&source)?;
    }
    preferences::update(&state, |settings| {
        if let Some(context) = context {
            if !matches!(context.as_str(), "metadata" | "tags" | "cover" | "patches") {
                return Err(Message::new("error.operation"));
            }
            settings.source_preferences.insert(context, source);
        } else {
            settings.metadata_provider = source;
        }
        Ok(())
    })
}
pub async fn credentials_for(state: &AppState, source: &str) -> Result<Credentials, Message> {
    if source == "erogamescape" {
        return Ok(Credentials::default());
    }
    let path = state.store.path.clone();
    let source = source.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        if source == "2dfan" {
            data_sources::load(&path, &source)
        } else {
            data_sources::oauth::load_fresh(&path, &source)
        }
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[tauri::command]
pub async fn login_data_source(
    source: String,
    state: State<'_, AppState>,
) -> Result<Vec<SourceStatus>, Message> {
    if !matches!(source.as_str(), "nextmoe" | "bangumi") {
        return Err(Message::new("sources.oauth_unsupported"));
    }
    let reply = galgear_i18n::tr(
        crate::state::snapshot(&state)?.language,
        "sources.oauth_return",
    )
    .to_owned();
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = state
            .source_oauth
            .active
            .lock()
            .map_err(|_| Message::new("error.operation"))?;
        if active.is_some() {
            return Err(Message::new("sources.oauth_pending"));
        }
        *active = Some(cancelled.clone());
    }
    let path = state.store.path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let value = data_sources::load(&path, &source)?;
        let value = data_sources::oauth::login(
            &source,
            value,
            &cancelled,
            |url| open::that_detached(url).map_err(|_| Message::new("sources.oauth_error")),
            &reply,
        )?;
        data_sources::oauth::transaction(|| {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Message::new("sources.oauth_cancelled"));
            }
            data_sources::check(&source, &value)?;
            if cancelled.load(Ordering::Relaxed) {
                return Err(Message::new("sources.oauth_cancelled"));
            }
            data_sources::save(&path, &source, &value)?;
            data_sources::statuses(&path)
        })
    })
    .await
    .map_err(|_| Message::new("error.operation"));
    *state
        .source_oauth
        .active
        .lock()
        .map_err(|_| Message::new("error.operation"))? = None;
    result?
}

#[tauri::command]
pub fn cancel_data_source_login(state: State<'_, AppState>) -> Result<(), Message> {
    if let Some(cancelled) = state
        .source_oauth
        .active
        .lock()
        .map_err(|_| Message::new("error.operation"))?
        .as_ref()
    {
        cancelled.store(true, Ordering::Relaxed);
    }
    Ok(())
}
