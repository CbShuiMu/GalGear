//! 截图设置、通知和最近截图；采集与文件写入由桌面端负责。
use crate::{
    models::{ScreenshotPreview, Settings},
    requests::{Pending, issue},
    state::{APP, App, show_toast},
    view::{paint, t},
    widgets::{button, esc},
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;
use wasm_bindgen::prelude::wasm_bindgen;

#[derive(Default)]
pub(crate) struct State {
    game: Option<String>,
    generation: u32,
    result: Option<Result<Option<ScreenshotPreview>, Message>>,
    saved_settings_request: u32,
}

impl State {
    pub(crate) fn saved(&mut self, request: u32) {
        self.saved_settings_request = request;
    }
    /// 后台读取的旧设置不能撤销已成功保存的快捷键或开关。
    pub(crate) fn preserve_settings(
        &self,
        current: &Settings,
        incoming: &mut Settings,
        request: u32,
    ) {
        if request < self.saved_settings_request {
            incoming.screenshot_enabled = current.screenshot_enabled;
            incoming
                .screenshot_hotkey
                .clone_from(&current.screenshot_hotkey);
        }
    }
}

pub(crate) fn load(app: &mut App, game: &str) {
    app.screenshots.game = Some(game.into());
    app.screenshots.generation = app.screenshots.generation.wrapping_add(1);
    app.screenshots.result = None;
    issue(
        app,
        Pending::Screenshot(game.into(), app.screenshots.generation),
        "latest_screenshot",
        json!({"id":game}),
    );
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if action == "screenshot-refresh" {
        load(app, value);
        return true;
    }
    let (kind, command, payload) = match action {
        "screenshot-open" => (Pending::Open, "open_screenshot_folder", json!({"id":value})),
        _ => return false,
    };
    if !app.busy() {
        issue(app, kind, command, payload);
    }
    true
}

pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    _id: u32,
    success: bool,
    payload: &str,
) -> bool {
    if let Pending::Screenshot(game, generation) = kind {
        if app.screenshots.game.as_ref() == Some(game) && app.screenshots.generation == *generation
        {
            app.screenshots.result = Some(if success {
                serde_json::from_str::<Option<ScreenshotPreview>>(payload)
                    .map_err(|e| Message::detail("error.read_data", e))
            } else {
                Err(Message::from_error(payload))
            });
        }
        return true;
    }
    false
}

#[derive(Deserialize)]
struct Notice {
    notice: Message,
    error: bool,
}

#[wasm_bindgen]
pub fn screenshot_completed(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        match serde_json::from_str::<Notice>(payload) {
            Ok(notice) => show_toast(&mut app, notice.notice, notice.error),
            Err(error) => show_toast(&mut app, Message::detail("error.read_data", error), true),
        }
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn screenshot_updated(game: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.detail.selected.as_deref() == Some(game) {
            load(&mut app, game);
            paint(&app);
        }
    });
}

pub(crate) fn render(html: &mut String, app: &App) {
    crate::shortcut_picker::field(html, app, "screenshot-hotkey");
}

pub(crate) fn render_card(html: &mut String, app: &App, game: &str) {
    write!(html,r#"<section class="detail-card screenshot-card" data-render-key="recent-screenshot"><div class="utility-heading"><h3>{}</h3>{}</div>"#,
        t(app,"screenshot.recent"),button("screenshot-open",game,t(app,"screenshot.open_folder"),"folder_open","quiet",app.busy())).unwrap();
    let result = if app.screenshots.game.as_deref() == Some(game) {
        app.screenshots.result.as_ref()
    } else {
        None
    };
    match result {
        Some(Ok(Some(preview))) if preview.image.starts_with("data:image/png;base64,") => {
            write!(html,r#"<figure class="screenshot-preview"><img src="{}" alt="{}"/><figcaption>{}</figcaption></figure>"#,
                esc(&preview.image),t(app,"screenshot.recent"),esc(&preview.name)).unwrap();
        }
        Some(Ok(None)) => {
            write!(
                html,
                r#"<div class="screenshot-empty"><p>{}</p></div>"#,
                t(app, "screenshot.empty")
            )
            .unwrap();
        }
        None => {
            write!(
                html,
                r#"<p class="muted" role="status">{}</p>"#,
                t(app, "screenshot.loading")
            )
            .unwrap();
        }
        other => {
            let error = match other {
                Some(Err(error)) => error.clone(),
                _ => Message::new("screenshot.invalid_frame"),
            };
            write!(
                html,
                r#"<p class="screenshot-error" role="status">{}</p>{}"#,
                esc(&error.render(app.settings.language)),
                button(
                    "screenshot-refresh",
                    game,
                    t(app, "screenshot.retry"),
                    "refresh",
                    "quiet",
                    false
                )
            )
            .unwrap();
        }
    }
    html.push_str("</section>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_preferences_cannot_undo_a_saved_shortcut() {
        let mut app = App::default();
        let old = app.settings.clone();
        app.settings.screenshot_hotkey = "Alt+F9".into();
        app.settings.screenshot_enabled = false;
        app.screenshots.saved(10);
        crate::state::apply_settings(&mut app, old, 9);
        assert!(!app.settings.screenshot_enabled);
        assert_eq!(app.settings.screenshot_hotkey, "Alt+F9");
    }
    #[test]
    fn late_preview_cannot_replace_newer_game_or_refresh() {
        let mut app = App {
            screenshots: State {
                game: Some("second".into()),
                generation: 3,
                result: None,
                ..Default::default()
            },
            ..Default::default()
        };
        resolve(
            &mut app,
            &Pending::Screenshot("first".into(), 2),
            1,
            true,
            "null",
        );
        resolve(
            &mut app,
            &Pending::Screenshot("second".into(), 2),
            2,
            true,
            "null",
        );
        assert!(app.screenshots.result.is_none());
        resolve(
            &mut app,
            &Pending::Screenshot("second".into(), 3),
            3,
            true,
            "null",
        );
        let mut html = String::new();
        render_card(&mut html, &app, "second");
        assert!(html.contains("screenshot-empty"));
        assert!(html.contains("screenshot-open"));
        assert!(!html.contains("<img"));
    }
}
