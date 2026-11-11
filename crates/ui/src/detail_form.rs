//! One detail draft shared by the in-place editor and its save action.
use crate::{
    features,
    models::{Game, Metadata},
    requests::Pending,
    requests::issue,
    state::{App, show_toast},
};
use galgear_i18n::Message;
use serde_json::json;
use std::{collections::BTreeMap, fmt::Write};
use wasm_bindgen::prelude::wasm_bindgen;

pub(crate) fn begin(app: &mut App) {
    app.form.retain(|key, _| !key.starts_with("draft-"));
    let Some(game) = app
        .detail
        .selected
        .as_ref()
        .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
    else {
        return;
    };
    if !game.available && matches!(app.detail.editor, Some(crate::detail::Editor::Tools)) {
        return;
    }
    let edit = app
        .settings
        .game_edits
        .get(&game.id)
        .cloned()
        .unwrap_or_default();
    for (key, value) in [
        ("draft-title", game.title.clone()),
        ("draft-tags", game.tags.join(", ")),
        (
            "draft-executable",
            game.executable.clone().unwrap_or_default(),
        ),
        ("draft-arguments", edit.arguments.join("\n")),
        ("draft-workdir", edit.working_directory.unwrap_or_default()),
        ("draft-launcher", edit.launcher.unwrap_or_default()),
        ("draft-locale", edit.locale_profile),
    ] {
        app.form.insert(key.into(), value);
    }
    let empty = Metadata::default();
    let metadata = game.metadata.as_ref().unwrap_or(&empty);
    for (field, _) in features::METADATA_FIELDS {
        app.form.insert(
            format!("draft-meta-{field}"),
            features::meta_value(metadata, field).into(),
        );
    }
    app.detail.tag_draft = None;
    app.detail.editing = true;
    app.detail.editing_configuration =
        matches!(app.detail.editor, Some(crate::detail::Editor::Tools));
    app.choices.open = None;
}

pub(crate) fn cancel(app: &mut App) {
    app.detail.editing = false;
    app.detail.editing_configuration = false;
    app.form.retain(|key, _| !key.starts_with("draft-"));
}

pub(crate) fn save(app: &mut App) {
    let Some(id) = app.detail.selected.clone() else {
        return;
    };
    let value = |key: &str| features::form(app, key);
    let optional = |key: &str| {
        let text = value(key);
        (!text.trim().is_empty()).then_some(text)
    };
    let metadata: BTreeMap<_, _> = features::METADATA_FIELDS
        .iter()
        .map(|(field, _)| (field.to_string(), value(&format!("draft-meta-{field}"))))
        .collect();
    let resets: Vec<_> = features::METADATA_FIELDS
        .iter()
        .filter(|(field, _)| value(&format!("draft-reset-{field}")) == "true")
        .map(|(field, _)| field)
        .collect();
    let payload = json!({"id":id,"draft":{"title":value("draft-title"),"tags":value("draft-tags").split([',','，']).map(str::trim).collect::<Vec<_>>(),"executable":value("draft-executable"),"arguments":value("draft-arguments").lines().collect::<Vec<_>>(),"working_directory":optional("draft-workdir"),"launcher":optional("draft-launcher"),"locale_profile":value("draft-locale"),"metadata":metadata,"metadata_resets":resets,"reset_tag_rules":value("draft-reset-tags")=="true"}});
    issue(app, Pending::DetailSaved, "save_game_details", payload);
}

pub(crate) fn refresh_changed_title(app: &mut App, game: &Game) {
    if !app
        .catalog
        .games
        .iter()
        .any(|old| old.id == game.id && old.title != game.title)
    {
        return;
    }
    let request_id = app.requests.next_id();
    issue(
        app,
        Pending::TitleRefresh(game.id.clone(), game.title.clone()),
        "refresh_title_metadata",
        json!({"id":game.id,"title":game.title,"requestId":request_id}),
    );
}

fn current_refresh(app: &App, request_id: u32) -> bool {
    let Some(Pending::TitleRefresh(id, title)) = app.requests.get(request_id) else {
        return false;
    };
    app.catalog
        .games
        .iter()
        .any(|game| &game.id == id && &game.title == title)
}

/// 接收已找到新词条的通知；忽略已完成或已被新标题取代的请求。
#[wasm_bindgen]
pub fn title_refresh_started(request_id: u32) {
    crate::state::APP.with(|state| {
        let mut app = state.borrow_mut();
        if current_refresh(&app, request_id) && app.detail.title_refreshing.insert(request_id) {
            crate::view::paint(&app);
        }
    });
}

pub(crate) fn render_refresh_progress(html: &mut String, app: &App) {
    if app
        .detail
        .title_refreshing
        .iter()
        .any(|id| current_refresh(app, *id))
    {
        write!(html, r#"<div id="title-refresh-toast" class="toast save-ai-toast" role="status" aria-live="polite" aria-atomic="true"><span class="network-spinner" aria-hidden="true"></span><span>{}</span></div>"#, crate::view::t(app, "detail.title_refreshing")).unwrap();
    }
}

pub(crate) fn resolve_refresh(
    app: &mut App,
    pending: &Pending,
    request_id: u32,
    success: bool,
    payload: &str,
) -> bool {
    let Pending::TitleRefresh(id, title) = pending else {
        return false;
    };
    // 先结束此请求的提示，再处理失败或迟到结果；不会关闭其他游戏仍在进行的同步。
    app.detail.title_refreshing.remove(&request_id);
    if !app
        .catalog
        .games
        .iter()
        .any(|game| &game.id == id && &game.title == title)
    {
        return true;
    }
    let result = if success {
        serde_json::from_str::<Option<Game>>(payload)
            .map_err(|error| Message::detail("error.read_data", error))
    } else {
        Err(Message::from_error(payload))
    };
    let warning = match result {
        Ok(Some(game)) if &game.id == id && &game.title == title => {
            let warning = game.metadata_warning.clone();
            if let Some(entry) = app.catalog.games.iter_mut().find(|entry| &entry.id == id) {
                crate::playtime::replace_game(entry, game);
            }
            app.detail.metadata_warnings.remove(id);
            issue(app, Pending::Preferences, "get_preferences", json!({}));
            warning
        }
        Ok(_) => None,
        Err(error) => Some(error),
    };
    if let Some(warning) = warning {
        show_toast(
            app,
            Message::detail(
                "detail.title_refresh_failed",
                warning.render(app.settings.language),
            ),
            true,
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bridge::take_requests, state::APP};

    fn game_json(title: &str) -> serde_json::Value {
        json!({"id":"fixture", "title":title, "folder":"F:/Fixture", "engine":"unknown", "modified":0, "tags":["Old tag"]})
    }

    fn game(title: &str) -> Game {
        serde_json::from_value(game_json(title)).unwrap()
    }

    #[test]
    fn successful_changed_title_save_starts_nonblocking_refresh_and_applies_result() {
        take_requests();
        APP.with(|state| {
            let mut app = state.borrow_mut();
            *app = App::default();
            app.catalog.games.push(game("Old title"));
            issue(
                &mut app,
                Pending::DetailSaved,
                "save_game_details",
                json!({}),
            );
        });
        let (request, _, _) = take_requests().pop().unwrap();
        crate::responses::resolve(request, true, &game_json("New title").to_string());
        let requests = take_requests();
        let (refresh, _, payload) = requests
            .iter()
            .find(|(_, command, _)| command == "refresh_title_metadata")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(payload).unwrap(),
            json!({"id":"fixture","title":"New title","requestId":refresh})
        );
        for (id, command, _) in &requests {
            if command == "get_preferences" {
                crate::responses::resolve(*id, true, "{}");
            }
        }
        APP.with(|state| {
            let app = state.borrow();
            assert!(!app.busy());
            assert_eq!(app.catalog.games[0].title, "New title");
        });
        let mut result = game_json("New title");
        result["tags"] = json!(["New tag"]);
        crate::responses::resolve(*refresh, true, &result.to_string());
        APP.with(|state| assert_eq!(state.borrow().catalog.games[0].tags, ["New tag"]));
    }

    #[test]
    fn unchanged_title_or_failed_save_never_starts_refresh() {
        for success in [true, false] {
            take_requests();
            APP.with(|state| {
                let mut app = state.borrow_mut();
                *app = App::default();
                app.catalog.games.push(game("Same title"));
                issue(
                    &mut app,
                    Pending::DetailSaved,
                    "save_game_details",
                    json!({}),
                );
            });
            let (id, _, _) = take_requests().pop().unwrap();
            let payload = if success {
                game_json("Same title").to_string()
            } else {
                r#"{"key":"error.config_write","args":{}}"#.into()
            };
            crate::responses::resolve(id, success, &payload);
            assert!(
                !take_requests()
                    .iter()
                    .any(|(_, command, _)| command == "refresh_title_metadata")
            );
        }
    }

    #[test]
    fn empty_failed_and_late_results_preserve_saved_game() {
        let mut app = App::default();
        app.catalog.games.push(game("New title"));
        let request = Pending::TitleRefresh("fixture".into(), "New title".into());
        resolve_refresh(&mut app, &request, 1, true, "null");
        assert_eq!(app.catalog.games[0].tags, ["Old tag"]);
        resolve_refresh(
            &mut app,
            &request,
            1,
            false,
            r#"{"key":"error.network","args":{}}"#,
        );
        assert_eq!(app.catalog.games[0].title, "New title");
        assert_eq!(
            app.toast.as_ref().unwrap().0.key,
            "detail.title_refresh_failed"
        );
        app.catalog.games[0].title = "Newest title".into();
        app.toast = None;
        resolve_refresh(
            &mut app,
            &request,
            1,
            true,
            &game_json("New title").to_string(),
        );
        assert_eq!(app.catalog.games[0].title, "Newest title");
        assert!(app.toast.is_none());
    }
}
