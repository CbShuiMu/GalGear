//! Game actions and persistent play states exposed by a card context menu.
use crate::{state::App, view::t, widgets::button, widgets::esc, widgets::icon};
use std::fmt::Write;

pub(crate) struct Menu {
    pub id: String,
    pub x: f64,
    pub y: f64,
}
pub(crate) const STATUSES: [(&str, &str, &str); 6] = [
    ("wanted", "status.wanted", "☆"),
    ("playing", "status.playing", "▶"),
    ("finished", "status.finished", "✓"),
    ("shelved", "status.shelved", "Ⅱ"),
    ("abandoned", "status.abandoned", "×"),
    ("unmarked", "status.unmarked", "−"),
];
pub(crate) fn label<'a>(app: &'a App, status: &str) -> &'a str {
    t(
        app,
        STATUSES
            .iter()
            .find(|(value, _, _)| *value == status)
            .map_or("status.unmarked", |(_, key, _)| key),
    )
}
pub(crate) fn render(html: &mut String, app: &App) {
    let Some(menu) = &app.context_menu else {
        return;
    };
    let Some(game) = app.catalog.games.iter().find(|g| g.id == menu.id) else {
        return;
    };
    write!(html,r#"<section class="game-context-menu" role="menu" aria-label="{}" data-context-x="{}" data-context-y="{}">"#,esc(&game.title),menu.x,menu.y).unwrap();
    for (action, label, icon, disabled) in [
        (
            "launch-normal",
            "detail.launch",
            "play",
            app.busy() || !game.available,
        ),
        ("game-detail", "context.detail", "about", app.busy()),
        ("context-delete", "context.delete", "close", app.busy()),
        ("open-folder", "detail.open_directory", "folder", app.busy()),
    ] {
        html.push_str(&button(
            action,
            &game.id,
            t(app, label),
            icon,
            "quiet",
            disabled,
        ));
    }
    write!(html,r#"<div class="context-state"><button type="button" class="button quiet" aria-haspopup="menu">{}<span>{}</span>{}</button><div class="context-submenu" role="menu">"#,icon("check"),t(app,"context.status"),icon("arrow")).unwrap();
    for (status, key, symbol) in STATUSES {
        write!(html,r#"<button type="button" class="status-option {}" role="menuitemradio" aria-checked="{}" data-action="play-status" data-value="{}"><span class="status-symbol {}">{}</span><span>{}</span></button>"#,if game.play_status==status{"selected"}else{""},game.play_status==status,status,status,symbol,t(app,key)).unwrap();
    }
    html.push_str("</div></div></section>");
}
