//! Account credentials and cached status refresh.
use crate::{fan_cache, state::AppState, state::snapshot};
use galgear_core::{
    Settings, config::ConfigStore, credentials, credentials::FanCredentials, fan_account::Account,
    fan_account::Session,
};
use galgear_i18n::Message;
use std::{sync::Mutex, sync::atomic::AtomicBool, sync::atomic::Ordering};
use tauri::{Emitter, Manager, State};

pub(crate) struct AccountState {
    cache: Mutex<fan_cache::Cache>,
    refreshing: AtomicBool,
    refresh_lock: tokio::sync::Mutex<()>,
}

impl AccountState {
    pub(crate) fn load(store: &ConfigStore, settings: &Settings) -> Self {
        let value = credentials::load(&credentials::target(&store.path)).unwrap_or_default();
        Self {
            cache: Mutex::new(fan_cache::Cache::load(
                store.path.with_file_name("fan-status-cache.json"),
                fan_cache::scope(&settings.fan_domain, &value),
            )),
            refreshing: AtomicBool::new(false),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }
}

#[tauri::command]
pub(crate) fn load_fan_credentials(state: State<'_, AppState>) -> Result<FanCredentials, Message> {
    credentials::load(&credentials::target(&state.store.path))
}

#[tauri::command]
pub(crate) fn save_fan_credentials(
    value: FanCredentials,
    state: State<'_, AppState>,
) -> Result<FanCredentials, Message> {
    if state.downloads.is_active() {
        return Err(Message::new("download.busy"));
    }
    if state.account.refreshing.load(Ordering::SeqCst) {
        return Err(Message::new("auth.busy"));
    }
    credentials::save(&credentials::target(&state.store.path), &value)?;
    let scope = fan_cache::scope(&snapshot(&state)?.fan_domain, &value);
    state
        .account
        .cache
        .lock()
        .map_err(|_| Message::new("error.config_state"))?
        .reset(scope);
    state.downloads.invalidate_plan()?;
    Ok(value)
}

#[tauri::command]
pub(crate) fn clear_fan_credentials(state: State<'_, AppState>) -> Result<FanCredentials, Message> {
    if state.downloads.is_active() {
        return Err(Message::new("download.busy"));
    }
    if state.account.refreshing.load(Ordering::SeqCst) {
        return Err(Message::new("auth.busy"));
    }
    credentials::clear(&credentials::target(&state.store.path))?;
    let empty = FanCredentials {
        mode: "csrf".into(),
        ..Default::default()
    };
    let scope = fan_cache::scope(&snapshot(&state)?.fan_domain, &empty);
    state
        .account
        .cache
        .lock()
        .map_err(|_| Message::new("error.config_state"))?
        .reset(scope);
    state.downloads.invalidate_plan()?;
    Ok(FanCredentials {
        mode: "csrf".into(),
        ..Default::default()
    })
}

#[tauri::command]
pub(crate) fn get_fan_status_cache(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<fan_cache::Snapshot, Message> {
    let settings = snapshot(&state)?;
    let value = load_fan_credentials(state.clone())?;
    let mut cache = state
        .account
        .cache
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    cache.ensure_scope(fan_cache::scope(&settings.fan_domain, &value));
    let initial = cache.initial();
    let result = cache.snapshot();
    drop(cache);
    if initial {
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let _ = refresh_fan_status_inner(&state, &app, false).await;
        });
    }
    Ok(result)
}

pub(crate) async fn refresh_fan_status_inner(
    state: &AppState,
    app: &tauri::AppHandle,
    force: bool,
) -> Result<fan_cache::Snapshot, Message> {
    let _serial = state.account.refresh_lock.lock().await;
    if state.downloads.is_active() {
        return Ok(state
            .account
            .cache
            .lock()
            .map_err(|_| Message::new("error.config_state"))?
            .snapshot());
    }
    let settings = snapshot(state)?;
    let value = credentials::load(&credentials::target(&state.store.path))?;
    let (ticket, previous) = {
        let mut cache = state
            .account
            .cache
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        cache.ensure_scope(fan_cache::scope(&settings.fan_domain, &value));
        let Some(ticket) = cache.begin(fan_cache::now(), force) else {
            return Ok(cache.snapshot());
        };
        (ticket, cache.snapshot())
    };
    state.account.refreshing.store(true, Ordering::SeqCst);
    let _ = app.emit("fan-status-updated", &previous);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let now = fan_cache::now();
        if value.cookie.is_empty() && value.token.is_empty() {
            return Ok(fan_cache::Snapshot {
                account: Account {
                    status: "unauthenticated".into(),
                    reason: Some(Message::new("auth.need_cookie")),
                    checked_at: now,
                    ..Default::default()
                },
                updated_at: now,
                ..Default::default()
            });
        }
        let session = Session::new(&settings.fan_domain, value)?;
        let account = session.check()?;
        Ok(fan_cache::Snapshot {
            account,
            updated_at: fan_cache::now(),
            ..Default::default()
        })
    })
    .await
    .unwrap_or_else(|_| Err(Message::new("auth.network")));
    state.account.refreshing.store(false, Ordering::SeqCst);
    let (committed, status) = {
        let mut cache = state
            .account
            .cache
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let committed = cache.finish(ticket, result);
        (committed, cache.snapshot())
    };
    if committed {
        let _ = app.emit("fan-status-updated", &status);
    }
    Ok(status)
}

#[tauri::command]
pub(crate) async fn refresh_fan_status(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<fan_cache::Snapshot, Message> {
    refresh_fan_status_inner(&state, &app, true).await
}

pub(crate) fn start_fan_status_scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let state = app.state::<AppState>();
                let now = fan_cache::now();
                let due = state
                    .account
                    .cache
                    .lock()
                    .map(|cache| cache.due(now))
                    .unwrap_or(false);
                if due
                    && !state.downloads.is_active()
                    && !state.account.refreshing.load(Ordering::SeqCst)
                {
                    let _ = refresh_fan_status_inner(&state, &app, false).await;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(fan_cache::next_minute(
                fan_cache::now(),
            )))
            .await;
        }
    });
}
