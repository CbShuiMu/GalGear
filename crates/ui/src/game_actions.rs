//! Shared favorite and completion controls for shelves and detail pages.
use crate::{
    models::Game,
    state::App,
    view::t,
    widgets::{esc, icon},
};
use std::fmt::Write;

pub(crate) fn render(html: &mut String, app: &App, game: &Game) {
    let collected = app
        .settings
        .collections
        .iter()
        .any(|folder| folder.games.contains(&game.id));
    let finished = game.play_status == "finished";
    for (action, key, symbol, kind, active) in [
        (
            "collection-picker",
            "card.favorite",
            "heart",
            "favorite",
            collected,
        ),
        (
            "card-finished",
            "card.finished",
            if finished { "trophy-filled" } else { "trophy" },
            "finished",
            finished,
        ),
    ] {
        let _ = write!(
            html,
            r#"<button type="button" class="card-action {} {}" data-action="{}" data-value="{}" aria-label="{}" title="{}" {} {}>{}</button>"#,
            kind,
            if active { "is-active" } else { "" },
            action,
            esc(&game.id),
            t(app, key),
            t(app, key),
            if action == "collection-picker" {
                "aria-haspopup=\"dialog\"".to_owned()
            } else {
                format!("aria-pressed=\"{finished}\"")
            },
            if app.busy() { "disabled" } else { "" },
            icon(symbol)
        );
    }
}
