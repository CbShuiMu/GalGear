//! 游戏库尺寸偏好与快捷键共用提交入口；布局测量由 DOM 层负责。
use crate::{
    requests::{Pending, issue},
    state::App,
};
use serde_json::json;

pub(crate) const OPTIONS: [(&str, &str); 3] = [
    ("normal", "display.cards_normal"),
    ("small", "display.cards_small"),
    ("tiny", "display.cards_tiny"),
];

pub(crate) fn current(value: &str) -> &str {
    if OPTIONS.iter().any(|(size, _)| *size == value) {
        value
    } else {
        "normal"
    }
}

pub(crate) fn pending(app: &App) -> bool {
    app.requests
        .values()
        .any(|p| matches!(p, Pending::LibrarySize))
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !matches!(action, "library-size" | "library-size-step") {
        return false;
    }
    if app.busy() || pending(app) {
        return true;
    }
    let old = current(&app.settings.library_card_size);
    let next = if action == "library-size-step" {
        if !matches!(app.page.as_str(), "games" | "collections")
            || app.detail.selected.is_some()
            || app.library.dialog.is_some()
            || app.choices.open.is_some()
            || app.context_menu.is_some()
        {
            return true;
        }
        match (old, value) {
            ("normal", "smaller") | ("tiny", "larger") => "small",
            ("small", "smaller") => "tiny",
            ("small", "larger") => "normal",
            _ => return true,
        }
    } else if OPTIONS.iter().any(|(size, _)| *size == value) {
        value
    } else {
        return true;
    };
    if old != next {
        app.library.size_request = app.requests.next_id();
        issue(
            app,
            Pending::LibrarySize,
            "set_library_card_size",
            json!({"size":next}),
        );
    }
    true
}
