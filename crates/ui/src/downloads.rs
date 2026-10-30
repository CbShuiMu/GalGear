//! Download presentation state and request status.
use crate::{
    models::DownloadPreview, models::Settings, models::Transfer, requests::Pending,
    requests::RequestQueue, requests::issue, state::App,
};
use serde_json::json;

#[derive(Default)]
pub(crate) struct State {
    pub(crate) preview: Option<DownloadPreview>,
    pub(crate) preview_folder: String,
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
        "open-download-result" => issue(
            app,
            Pending::Open,
            "open_download_folder",
            json!({"lastDownload":true}),
        ),
        "inspect-download" if !busy(&app.requests) => {
            app.downloads.preview = None;
            let game_id = app
                .patches
                .resources
                .iter()
                .any(|resource| resource.url == value)
                .then(|| app.patches.resources_game.clone())
                .flatten();
            app.downloads.preview_folder = game_id
                .as_ref()
                .and_then(|id| app.catalog.games.iter().find(|game| &game.id == id))
                .map(|game| format!("{}/patch", game.folder.trim_end_matches(['/', '\\'])))
                .unwrap_or_else(|| folder(&app.config_path, &app.settings));
            issue(
                app,
                Pending::DownloadPreview,
                "inspect_fan_resource",
                json!({"url":value,"gameId":game_id}),
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

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let mut app = App::default();
        app.settings.download_folder = "E:/Downloads".into();
        app.catalog.games.push(serde_json::from_value(json!({"id":"game","title":"Game","folder":"F:/Game","engine":"unknown","modified":0})).unwrap());
        app.patches.resources = serde_json::from_value(
            json!([{"source":"2DFan","title":"Patch","url":"https://2dfan.com/downloads/1"}]),
        )
        .unwrap();
        app
    }

    #[test]
    fn inspection_uses_result_origin_even_after_the_search_input_changes() {
        let mut app = app();
        app.patches.resources_game = Some("game".into());
        app.patches.patch_search_game.clear();
        app.form
            .insert("patch-query".into(), "Unrelated new input".into());
        crate::bridge::take_requests();
        dispatch(
            &mut app,
            "inspect-download",
            "https://2dfan.com/downloads/1",
        );
        let requests = crate::bridge::take_requests();
        let payload: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
        assert_eq!(payload["gameId"], "game");
        assert_eq!(app.downloads.preview_folder, "F:/Game/patch");
        app.patches.resources_game = None;
        assert_eq!(app.downloads.preview_folder, "F:/Game/patch");
    }

    #[test]
    fn manually_searched_resources_use_default_downloads_even_when_game_is_selected() {
        let mut app = app();
        app.patches.patch_search_game = "game".into();
        crate::bridge::take_requests();
        dispatch(
            &mut app,
            "inspect-download",
            "https://2dfan.com/downloads/1",
        );
        let requests = crate::bridge::take_requests();
        let payload: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
        assert!(payload["gameId"].is_null());
        assert_eq!(app.downloads.preview_folder, "E:/Downloads");
    }
}
