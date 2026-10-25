//! Download plans, purchase confirmation and transfers.
use crate::{dialogs, state::AppState, state::snapshot};
use galgear_core::{
    Settings, credentials, credentials::FanCredentials, fan_account::DownloadPlan,
    fan_account::DownloadPreview, fan_account::Session, fan_account::Transfer,
};
use galgear_i18n::Message;
use std::{
    path::PathBuf, sync::Arc, sync::Mutex, sync::atomic::AtomicBool, sync::atomic::Ordering,
};
use tauri::{Emitter, State};

#[derive(Default)]
pub(crate) struct DownloadState {
    active: AtomicBool,
    cancel: Arc<AtomicBool>,
    plan: Mutex<Option<(DownloadPlan, FanCredentials)>>,
    downloaded: Mutex<Option<String>>,
}

impl DownloadState {
    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub(crate) fn invalidate_plan(&self) -> Result<(), Message> {
        *self
            .plan
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))? = None;
        Ok(())
    }

    pub(crate) fn downloaded_path(&self) -> Result<Option<String>, Message> {
        Ok(self
            .downloaded
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?
            .clone())
    }
}

pub(crate) fn download_directory(state: &AppState, settings: &Settings) -> PathBuf {
    if settings.download_folder.is_empty() {
        state.store.path.with_file_name("downloads")
    } else {
        PathBuf::from(&settings.download_folder)
    }
}

#[tauri::command]
pub(crate) async fn choose_download_folder(
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    let directory = download_directory(&state, &snapshot(&state)?);
    let start = dialogs::start(&state, "downloads", &[directory])?;
    let path = tauri::async_runtime::spawn_blocking(move || dialogs::dialog(start).pick_folder())
        .await
        .map_err(|_| Message::new("download.write"))?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    if let Some(path) = path {
        let mut next = settings.clone();
        next.download_folder = galgear_core::config::display_path(&path);
        state.store.save(&next)?;
        *settings = next;
    }
    Ok(settings.clone())
}

#[tauri::command]
pub(crate) fn open_download_folder(state: State<'_, AppState>) -> Result<(), Message> {
    let path = download_directory(&state, &snapshot(&state)?);
    std::fs::create_dir_all(&path).map_err(|_| Message::new("download.write"))?;
    open::that_detached(path).map_err(|_| Message::new("download.write"))
}

#[tauri::command]
pub(crate) async fn inspect_fan_resource(
    url: String,
    state: State<'_, AppState>,
) -> Result<DownloadPreview, Message> {
    let domain = snapshot(&state)?.fan_domain;
    let value = credentials::load(&credentials::target(&state.store.path))?;
    let bound = value.clone();
    let mut plan =
        tauri::async_runtime::spawn_blocking(move || Session::new(&domain, value)?.inspect(&url))
            .await
            .map_err(|_| Message::new("auth.network"))??;
    plan.preview.ticket = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();
    let preview = plan.preview.clone();
    *state
        .downloads
        .plan
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))? = Some((plan, bound));
    Ok(preview)
}

#[tauri::command]
pub(crate) async fn start_fan_download(
    ticket: String,
    confirm_price: Option<u64>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Transfer, Message> {
    if state.downloads.active.swap(true, Ordering::SeqCst) {
        return Err(Message::new("download.busy"));
    }
    state.downloads.cancel.store(false, Ordering::SeqCst);
    let result = async {
        let (plan, bound) = state
            .downloads
            .plan
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?
            .take()
            .ok_or_else(|| Message::new("download.expired"))?;
        if plan.preview.ticket != ticket {
            return Err(Message::new("download.expired"));
        }
        let settings = snapshot(&state)?;
        let credentials = credentials::load(&credentials::target(&state.store.path))?;
        if bound != credentials {
            return Err(Message::new("download.expired"));
        }
        if plan.preview.requires_purchase
            && (!plan.preview.can_purchase
                || confirm_price.is_none()
                || confirm_price != plan.preview.price)
        {
            return Err(Message::new("download.confirm_required"));
        }
        let directory = download_directory(&state, &settings);
        let audit_directory = state.store.path.with_file_name("purchases");
        let cancel = state.downloads.cancel.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use sha2::{Digest, Sha256};
            let session = Session::new(&settings.fan_domain, credentials)?;
            let mut fresh = session.inspect(&plan.preview.source_url)?;
            if fresh.preview.requires_purchase {
                if !plan.preview.requires_purchase || fresh.preview.price != confirm_price {
                    return Err(Message::new("download.price_changed"));
                }
                if cancel.load(Ordering::Relaxed) {
                    return Err(Message::new("download.cancelled"));
                }
                // A pending marker survives crashes/timeouts; never retry an uncertain charge automatically.
                std::fs::create_dir_all(&audit_directory)
                    .map_err(|_| Message::new("download.write"))?;
                let marker = audit_directory.join(format!(
                    "{:x}.pending",
                    Sha256::digest(fresh.preview.source_url.as_bytes())
                ));
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)
                    .map_err(|e| {
                        Message::new(if e.kind() == std::io::ErrorKind::AlreadyExists {
                            "download.purchase_uncertain"
                        } else {
                            "download.write"
                        })
                    })?;
                file.write_all(
                    format!(
                        "{}\n{}\n",
                        fresh.preview.source_url,
                        confirm_price.unwrap_or_default()
                    )
                    .as_bytes(),
                )
                .and_then(|_| file.sync_all())
                .map_err(|_| Message::new("download.write"))?;
                let _ = app.emit(
                    "fan-download-progress",
                    Transfer {
                        phase: "purchasing".into(),
                        title: fresh.preview.title.clone(),
                        ..Default::default()
                    },
                );
                fresh = match session.purchase(&fresh) {
                    Ok(plan) => plan,
                    Err(error) => {
                        drop(file);
                        if error.key != "download.purchase_uncertain" {
                            let _ = std::fs::remove_file(&marker);
                        }
                        return Err(error);
                    }
                };
                drop(file);
                let _ = std::fs::remove_file(marker);
            }
            if fresh.preview.free && fresh.preview.can_download {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Message::new("download.cancelled"));
                }
                fresh = session.acquire_free(&fresh)?;
            }
            let mut final_transfer = Transfer::default();
            session.download(&fresh, &directory, &cancel, &mut |progress| {
                final_transfer = progress.clone();
                let _ = app.emit("fan-download-progress", progress);
            })?;
            Ok(final_transfer)
        })
        .await
        .map_err(|_| Message::new("download.network"))?
    }
    .await;
    state.downloads.active.store(false, Ordering::SeqCst);
    if let Ok(transfer) = &result {
        *state
            .downloads
            .downloaded
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))? = transfer.path.clone();
    }
    result
}

#[tauri::command]
pub(crate) fn cancel_fan_download(state: State<'_, AppState>) {
    state.downloads.cancel.store(true, Ordering::SeqCst);
}
