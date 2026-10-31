//! Library migration preview and explicit commit; parsing stays on the desktop side.
use crate::{
    models::{Catalog, Settings},
    requests::{Pending, issue},
    state::{App, apply_settings, show_toast},
    view::{f, t},
    widgets::{button, esc},
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    preview: Option<Preview>,
    error: Option<Message>,
}
#[derive(Deserialize)]
struct Preview {
    token: u64,
    source: String,
    added: usize,
    skipped: usize,
    rows: Vec<Row>,
}
#[derive(Deserialize)]
struct Row {
    title: String,
    path: String,
    reason: Option<Message>,
}
#[derive(Deserialize)]
struct Imported {
    settings: Settings,
    catalog: Catalog,
    added: usize,
    skipped: usize,
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> Option<bool> {
    match action {
        "import-open" if !app.busy() => {
            app.library_import = State {
                open: true,
                ..Default::default()
            };
        }
        "library-close" if app.library_import.open => {
            if !app.busy() {
                app.library_import = State::default();
            }
        }
        "import-select"
            if app.library_import.open
                && !app.busy()
                && matches!(value, "potatovn" | "reinamanager" | "lunabox") =>
        {
            app.library_import.preview = None;
            app.library_import.error = None;
            issue(
                app,
                Pending::ImportPreview,
                "prepare_library_import",
                json!({"source":value}),
            );
        }
        "import-confirm" if app.library_import.open && !app.busy() => {
            if let Some(preview) = &app.library_import.preview
                && preview.added > 0
            {
                issue(
                    app,
                    Pending::ImportCommit,
                    "confirm_library_import",
                    json!({"token":preview.token}),
                );
            }
        }
        _ => return None,
    }
    Some(true)
}
pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    id: u32,
    success: bool,
    payload: &str,
) -> bool {
    if !matches!(kind, Pending::ImportPreview | Pending::ImportCommit) {
        return false;
    }
    if !success {
        app.library_import.error = Some(Message::from_error(payload));
        return true;
    }
    let result: Result<(), serde_json::Error> = if matches!(kind, Pending::ImportPreview) {
        serde_json::from_str::<Option<Preview>>(payload).map(|preview| {
            app.library_import.preview = preview;
        })
    } else {
        serde_json::from_str::<Imported>(payload).map(|result| {
            app.catalog = result.catalog;
            apply_settings(app, result.settings, id);
            app.library_import = State::default();
            show_toast(
                app,
                Message::new("import.done")
                    .arg("added", result.added)
                    .arg("skipped", result.skipped),
                false,
            );
        })
    };
    if let Err(e) = result {
        app.library_import.error = Some(Message::detail("error.read_data", e));
    }
    true
}
pub(crate) fn render(html: &mut String, app: &App) {
    if !app.library_import.open {
        return;
    }
    write!(html,r#"<div class="library-overlay"><section class="library-dialog import-dialog" role="dialog" aria-modal="true" aria-labelledby="import-title"><header><h2 id="import-title">{}</h2><button id="library-dialog-close" class="icon-button" data-action="library-close" aria-label="{}" {}>×</button></header><div class="library-dialog-body"><p>{}</p><div class="import-sources">"#,t(app,"import.title"),t(app,"common.close"),if app.busy(){"disabled"}else{""},t(app,"import.help")).unwrap();
    for (key, name, help) in [
        ("potatovn", "PotatoVN", "import.potato_help"),
        ("reinamanager", "ReinaManager", "import.reina_help"),
        ("lunabox", "LunaBox", "import.luna_help"),
    ] {
        write!(
            html,
            "<div>{}<small>{}</small></div>",
            button(
                "import-select",
                key,
                name,
                "database",
                "secondary",
                app.busy()
            ),
            t(app, help)
        )
        .unwrap();
    }
    html.push_str("</div>");
    if app.busy() {
        write!(html, "<p role=\"status\">{}</p>", t(app, "import.working")).unwrap();
    }
    if let Some(error) = &app.library_import.error {
        write!(
            html,
            "<p class=\"error-text\" role=\"alert\">{}</p>",
            esc(&error.render(app.settings.language))
        )
        .unwrap();
    }
    if let Some(preview) = &app.library_import.preview {
        write!(
            html,
            "<p class=\"import-summary\">{} · {}</p><div class=\"import-rows\">",
            esc(&preview.source),
            f(
                app,
                "import.summary",
                &[
                    ("added", &preview.added.to_string()),
                    ("skipped", &preview.skipped.to_string())
                ]
            )
        )
        .unwrap();
        for row in &preview.rows {
            write!(html,"<div class=\"import-row\"><strong>{}</strong><small>{}</small><span class=\"{}\">{}</span></div>",esc(&row.title),esc(&row.path),if row.reason.is_some(){"muted"}else{"import-ready"},esc(&row.reason.as_ref().map(|r|r.render(app.settings.language)).unwrap_or_else(||t(app,"import.ready").into()))).unwrap();
        }
        html.push_str("</div>");
    }
    write!(
        html,
        "</div><footer class=\"library-dialog-footer\">{}{}</footer></section></div>",
        button(
            "library-close",
            "",
            t(app, "common.cancel"),
            "",
            "quiet",
            app.busy()
        ),
        button(
            "import-confirm",
            "",
            t(app, "import.confirm"),
            "download",
            "primary",
            app.busy()
                || app
                    .library_import
                    .preview
                    .as_ref()
                    .is_none_or(|p| p.added == 0)
        )
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_preview_do_not_change_the_library() {
        let mut app = App::default();
        dispatch(&mut app, "import-open", "");
        resolve(&mut app, &Pending::ImportPreview, 1, true, "null");
        assert!(app.library_import.preview.is_none());
        assert!(app.catalog.games.is_empty());
        resolve(
            &mut app,
            &Pending::ImportPreview,
            2,
            true,
            r#"{"token":7,"source":"potatovn","added":0,"skipped":1,"rows":[]}"#,
        );
        crate::bridge::take_requests();
        dispatch(&mut app, "import-confirm", "");
        assert!(crate::bridge::take_requests().is_empty());
        dispatch(&mut app, "library-close", "");
        assert!(!app.library_import.open);
    }
}
