//! Magpie configuration selection and portable library export requests.
use crate::{
    choices,
    features::setting,
    requests::{Pending, issue},
    state::{App, show_toast},
    view::t,
    widgets::{button, esc},
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

#[derive(Default, Deserialize)]
pub(crate) struct State {
    #[serde(default)]
    options: Vec<Mode>,
    #[serde(default)]
    pub(crate) selected: String,
    #[serde(skip)]
    path: String,
    #[serde(skip)]
    request: Option<u32>,
    #[serde(skip)]
    error: Option<Message>,
}
#[derive(Deserialize)]
struct Mode {
    value: String,
    label: String,
}

pub(crate) fn load(app: &mut App) {
    let path = setting(app, "tool-magpie", &app.settings.magpie);
    if app.tool_preferences.request.is_some() {
        return;
    }
    app.tool_preferences = State {
        path: path.clone(),
        request: Some(app.requests.next_id()),
        ..Default::default()
    };
    issue(
        app,
        Pending::MagpieModes,
        "magpie_modes",
        json!({"magpie":path}),
    );
}
pub(crate) fn selection(app: &App) -> Option<&str> {
    (app.tool_preferences.path == setting(app, "tool-magpie", &app.settings.magpie)
        && app
            .tool_preferences
            .options
            .iter()
            .any(|m| m.value == app.tool_preferences.selected))
    .then_some(app.tool_preferences.selected.as_str())
}
pub(crate) fn enabled(app: &App, field: &str) -> bool {
    field == "magpie-mode"
        && !app.busy()
        && app.tool_preferences.request.is_none()
        && !app.tool_preferences.options.is_empty()
        && app.tool_preferences.path == setting(app, "tool-magpie", &app.settings.magpie)
}
pub(crate) fn options(app: &App) -> Vec<choices::OptionItem> {
    if app.tool_preferences.path != setting(app, "tool-magpie", &app.settings.magpie)
        || app.tool_preferences.options.is_empty()
    {
        return vec![choices::item("", t(app, "tools.magpie_mode_load"))];
    }
    let mut modes: Vec<_> = app
        .tool_preferences
        .options
        .iter()
        .map(|m| choices::item(&m.value, &m.label))
        .collect();
    if selection(app).is_none() {
        modes.insert(0, choices::item("", t(app, "tools.magpie_mode_select")));
    }
    modes
}
pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    match action {
        "magpie-reload" => load(app),
        "magpie-mode"
            if enabled(app, action)
                && app
                    .tool_preferences
                    .options
                    .iter()
                    .any(|m| m.value == value) =>
        {
            app.tool_preferences.selected = value.into();
        }
        "export-potatovn" if !app.busy() => {
            issue(app, Pending::ExportPotato, "export_potatovn", json!({}))
        }
        _ => return false,
    }
    true
}
pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    id: u32,
    success: bool,
    payload: &str,
) -> bool {
    match kind {
        Pending::MagpieModes => {
            if app.tool_preferences.request != Some(id) {
                return true;
            }
            app.tool_preferences.request = None;
            let result = if success {
                serde_json::from_str::<State>(payload)
                    .map_err(|e| Message::detail("error.read_data", e))
            } else {
                Err(Message::from_error(payload))
            };
            match result {
                Ok(mut state) => {
                    state.path = app.tool_preferences.path.clone();
                    app.tool_preferences = state;
                }
                Err(error) => app.tool_preferences.error = Some(error),
            }
        }
        Pending::ExportPotato => {
            if !success {
                show_toast(app, Message::from_error(payload), true);
            } else {
                match serde_json::from_str::<Option<String>>(payload) {
                    Ok(Some(path)) => {
                        show_toast(app, Message::new("export.saved").arg("path", path), false)
                    }
                    Ok(None) => {}
                    Err(error) => show_toast(app, Message::detail("error.read_data", error), true),
                }
            }
        }
        _ => return false,
    }
    true
}
pub(crate) fn render(html: &mut String, app: &App) {
    write!(html, r#"<div class="edit-field magpie-mode-field"><div class="magpie-mode-label"><span>{}</span>{}</div>"#,
        t(app,"tools.magpie_mode"), button("magpie-reload","",t(app,"tools.magpie_mode_load"),"refresh","quiet",app.busy() || app.tool_preferences.request.is_some())).unwrap();
    choices::control(
        html,
        app,
        "magpie-mode",
        t(app, "tools.magpie_mode"),
        !enabled(app, "magpie-mode"),
    );
    if let Some(error) = &app.tool_preferences.error {
        write!(
            html,
            "<small class=\"error-text\">{}</small>",
            esc(&error.render(app.settings.language))
        )
        .unwrap();
    }
    html.push_str("</div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_changes_cannot_save_a_profile_from_a_different_magpie() {
        let mut app = App::default();
        app.settings.magpie = "first/Magpie.exe".into();
        load(&mut app);
        let id = app.requests.last_id();
        app.requests.complete(id);
        resolve(
            &mut app,
            &Pending::MagpieModes,
            id,
            true,
            r#"{"options":[{"value":"mode-a","label":"My profile"}],"selected":"mode-a"}"#,
        );
        assert_eq!(selection(&app), Some("mode-a"));
        app.form
            .insert("tool-magpie".into(), "second/Magpie.exe".into());
        assert!(!enabled(&app, "magpie-mode"));
        assert_eq!(selection(&app), None);
        load(&mut app);
        let next = app.requests.last_id();
        resolve(
            &mut app,
            &Pending::MagpieModes,
            id,
            true,
            r#"{"options":[],"selected":"old"}"#,
        );
        assert_eq!(app.tool_preferences.request, Some(next));
    }
    #[test]
    fn cancelled_export_is_silent_and_failure_is_reported() {
        let mut app = App::default();
        resolve(&mut app, &Pending::ExportPotato, 1, true, "null");
        assert!(app.toast.is_none());
        resolve(
            &mut app,
            &Pending::ExportPotato,
            2,
            false,
            r#"{"key":"export.error","args":{"detail":"write failed"}}"#,
        );
        assert!(app.toast.as_ref().unwrap().1);
    }
}
