//! 独立路线图窗口仅加载打包页面，游戏目录中的内容始终作为数据传递。
use crate::state::{AppState, find_game, snapshot};
use galgear_core::ai_summary::route_view;
use galgear_i18n::{Language, Message, tr};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

fn window_label(id: &str) -> String {
    format!("route-{:x}", Sha256::digest(id.as_bytes()))
}

fn open_error(error: impl std::fmt::Display) -> Message {
    Message::detail("route.open_failed_detail", error)
}

#[tauri::command]
pub(crate) async fn open_ai_route_map(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), Message> {
    let label = window_label(&id);
    if let Some(window) = app.get_webview_window(&label) {
        window.unminimize().map_err(open_error)?;
        window.show().map_err(open_error)?;
        return window.set_focus().map_err(open_error);
    }
    let guard = state.summaries.begin(&id)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let lang = settings.language;
    let title = format!("{} · {}", game.title, tr(lang, "route.title"));
    let config = state.store.path.clone();
    let document =
        tauri::async_runtime::spawn_blocking(move || route_view::load(&game, &settings, &config))
            .await
            .map_err(open_error)??;
    let result = build_window(&app, &label, &title, lang, document);
    drop(guard);
    result.map(|_| ())
}

fn build_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    lang: Language,
    document: route_view::Document,
) -> Result<WebviewWindow, Message> {
    let payload = serde_json::json!({
        "document": document, "title": title, "language": lang.code(),
        "exportError": tr(lang, "route.export_failed"),
    });
    let script = format!("window.GalGearRoute = {payload};");
    let mut builder =
        WebviewWindowBuilder::new(app, label, WebviewUrl::App("route-map.html".into()))
            .initialization_script(script)
            .title(title)
            .inner_size(1080.0, 760.0)
            .min_inner_size(360.0, 360.0)
            .center();
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(open_error)?;
    }
    builder.build().map_err(open_error)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Export {
    Json,
    Draft,
}

#[tauri::command]
pub(crate) async fn export_route(
    kind: Export,
    contents: String,
    window: WebviewWindow,
) -> Result<(), String> {
    if !window.label().starts_with("route-") || contents.len() > 4 * 1024 * 1024 {
        return Err("Invalid route export".into());
    }
    let (name, extension) = match kind {
        Export::Json => ("route-map-data.json", "json"),
        Export::Draft => ("route-guide.txt", "txt"),
    };
    tauri::async_runtime::spawn_blocking(move || -> std::io::Result<()> {
        let Some(path) = rfd::FileDialog::new()
            .set_parent(&window)
            .set_file_name(name)
            .add_filter(extension, &[extension])
            .save_file()
        else {
            return Ok(());
        };
        let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(contents.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[cfg(all(test, windows))]
mod tests;
