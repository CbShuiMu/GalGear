//! 游戏库显示偏好的提交入口，保存成功后才切换视图。
use crate::{
    requests::{Pending, issue},
    state::App,
};
use serde_json::json;

pub(crate) const OPTIONS: [(&str, &str); 2] = [
    ("poster", "display.mode_poster"),
    ("caption", "display.mode_caption"),
];

pub(crate) fn current(value: &str) -> &str {
    if value == "caption" {
        "caption"
    } else {
        "poster"
    }
}

pub(crate) fn pending(app: &App) -> bool {
    app.requests
        .values()
        .any(|p| matches!(p, Pending::LibraryDisplay))
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if action != "library-display" {
        return false;
    }
    if app.busy()
        || pending(app)
        || !OPTIONS.iter().any(|(mode, _)| *mode == value)
        || current(&app.settings.library_display_mode) == value
    {
        return true;
    }
    app.library.display_request = app.requests.next_id();
    issue(
        app,
        Pending::LibraryDisplay,
        "set_library_display_mode",
        json!({"mode": value}),
    );
    true
}
