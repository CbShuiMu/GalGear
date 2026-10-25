//! Download presentation state and request status.
use crate::{
    models::DownloadPreview, models::Settings, models::Transfer, requests::Pending,
    requests::RequestQueue, requests::issue, state::App,
};
use serde_json::json;

#[derive(Default)]
pub(crate) struct State {
    pub(crate) preview: Option<DownloadPreview>,
    pub(crate) transfer: Option<Transfer>,
    pub(crate) notice: Option<Transfer>,
    pub(crate) notice_generation: u32,
}

pub(crate) fn busy(requests: &RequestQueue) -> bool {
    requests
        .values()
        .any(|p| matches!(p, Pending::DownloadPreview | Pending::Download))
}

pub(crate) fn folder(config_path: &str, settings: &Settings) -> String {
    if !settings.download_folder.is_empty() {
        return settings.download_folder.clone();
    }
    config_path
        .rsplit_once(['\\', '/'])
        .map(|(dir, _)| format!("{dir}\\downloads"))
        .unwrap_or_default()
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> Option<bool> {
    match action {
        "dismiss-download-notice" => app.downloads.notice = None,
        "download-folder" => issue(
            app,
            Pending::Preferences,
            "choose_download_folder",
            json!({}),
        ),
        "open-downloads" => issue(app, Pending::Open, "open_download_folder", json!({})),
        "inspect-download" if !busy(&app.requests) => {
            app.downloads.preview = None;
            issue(
                app,
                Pending::DownloadPreview,
                "inspect_fan_resource",
                json!({"url":value}),
            );
        }
        "close-download" if !busy(&app.requests) => app.downloads.preview = None,
        "download-free" | "purchase-download" if !busy(&app.requests) => {
            if let Some(preview) = app.downloads.preview.clone()
                && ((action == "download-free" && preview.can_download)
                    || (action == "purchase-download" && preview.can_purchase))
            {
                app.downloads.transfer = Some(Transfer {
                    phase: "preparing".into(),
                    title: preview.title.clone(),
                    ..Default::default()
                });
                let price = if action == "purchase-download" {
                    preview.price
                } else {
                    None
                };
                issue(
                    app,
                    Pending::Download,
                    "start_fan_download",
                    json!({"ticket":preview.ticket,"confirmPrice":price}),
                );
            }
        }
        "cancel-download" => issue(app, Pending::Open, "cancel_fan_download", json!({})),
        _ => return None,
    }
    Some(true)
}
