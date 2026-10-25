//! Source covers are selected independently from the game's metadata association.
use crate::{
    detail, models::Game, preferences, requests::Pending, requests::issue, state::App,
    state::show_toast, view::f, view::t, widgets::button, widgets::esc,
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State {
    source: String,
    query: String,
    search_query: Option<String>,
    request: Option<u32>,
    covers: Vec<Cover>,
    error: Option<Message>,
}

#[derive(Deserialize)]
struct Cover {
    title: String,
    url: String,
    image_url: String,
    thumbnail_url: String,
}

fn load(app: &mut App) {
    let Some(id) = app.detail.selected.clone() else {
        return;
    };
    app.cover_picker.covers.clear();
    app.cover_picker.error = None;
    app.cover_picker.request = Some(app.requests.next_id());
    issue(
        app,
        Pending::CoverOptions,
        "source_covers",
        json!({"id":id,"source":app.cover_picker.source,"query":app.cover_picker.search_query}),
    );
}

fn search(app: &mut App) {
    let query = app.cover_picker.query.trim();
    app.cover_picker.search_query = (!query.is_empty()).then(|| query.to_owned());
    load(app);
}

pub(crate) fn input(app: &mut App, field: &str, value: &str) -> bool {
    if field != "cover-query" {
        return false;
    }
    app.cover_picker.query = value.into();
    true
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    match action {
        "cover-picker" => {
            app.detail.editor = Some(detail::Editor::Cover);
            app.cover_picker = State {
                source: "vndb".into(),
                query: app
                    .detail
                    .selected
                    .as_ref()
                    .and_then(|id| app.catalog.games.iter().find(|game| &game.id == id))
                    .map(|game| game.title.clone())
                    .unwrap_or_default(),
                ..Default::default()
            };
            load(app);
        }
        "cover-provider" if preferences::SOURCES.iter().any(|(id, _)| *id == value) => {
            if app.cover_picker.source != value {
                app.cover_picker.source = value.into();
                search(app);
            }
        }
        "cover-search" => {
            if !app.cover_picker.query.trim().is_empty() {
                search(app);
            }
        }
        "cover-retry" => load(app),
        "cover-select" => {
            if let Some(cover) = app
                .cover_picker
                .covers
                .iter()
                .find(|cover| cover.image_url == value)
                && let Some(id) = app.detail.selected.clone()
            {
                issue(
                    app,
                    Pending::Cover,
                    "select_source_cover",
                    json!({"id":id,"url":cover.url,"imageUrl":cover.image_url}),
                );
            }
        }
        _ => return false,
    }
    true
}

pub(crate) fn resolve(app: &mut App, request: u32, success: bool, payload: &str) {
    if app.cover_picker.request != Some(request) {
        return;
    }
    app.cover_picker.request = None;
    let result = if success {
        serde_json::from_str(payload).map_err(|error| Message::detail("error.read_data", error))
    } else {
        Err(Message::from_error(payload))
    };
    match result {
        Ok(covers) => app.cover_picker.covers = covers,
        Err(error) => {
            show_toast(app, error.clone(), true);
            app.cover_picker.error = Some(error);
        }
    }
}

pub(crate) fn render(html: &mut String, app: &App, game: &Game) {
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog cover-picker-dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title"><header><h2 id="editor-title">{}</h2>{}</header><div class="association-search cover-search"><label class="edit-field" for="cover-query"><span>{}</span><input id="cover-query" data-input="cover-query" value="{}" autocomplete="off"/></label>{}</div><div class="cover-provider-picker">"#,t(app,"detail.cover"),button("editor-close","",t(app,"common.close"),"close","quiet",app.busy()),t(app,"metadata.query"),esc(&app.cover_picker.query),button("cover-search","",t(app,"cover.search"),"search","secondary",app.busy()||app.cover_picker.request.is_some()||app.cover_picker.query.trim().is_empty())).unwrap();
    preferences::source_picker(
        html,
        app,
        "cover-provider",
        &app.cover_picker.source,
        app.busy(),
    );
    write!(html,r#"</div><div class="editor-body cover-results" id="cover-results" data-render-key="cover-results-{}" aria-busy="{}">"#,esc(&app.cover_picker.source),app.cover_picker.request.is_some()).unwrap();
    if app.cover_picker.request.is_some() {
        write!(
            html,
            r#"<p class="cover-empty" role="status">{}</p>"#,
            t(app, "cover.loading")
        )
        .unwrap();
    } else if let Some(error) = &app.cover_picker.error {
        write!(
            html,
            r#"<div class="cover-empty"><p>{}</p>{}</div>"#,
            esc(&error.render(app.settings.language)),
            button(
                "cover-retry",
                "",
                t(app, "cover.retry"),
                "refresh",
                "quiet",
                app.busy()
            )
        )
        .unwrap();
    } else if app.cover_picker.covers.is_empty() {
        write!(
            html,
            r#"<p class="cover-empty">{}</p>"#,
            t(app, "cover.empty")
        )
        .unwrap();
    } else {
        html.push_str("<div class=\"cover-options\">");
        for cover in &app.cover_picker.covers {
            write!(html,r#"<button type="button" class="cover-option" data-action="cover-select" data-value="{}" title="{}" aria-label="{}" {}><img src="{}" alt="{}" loading="lazy" referrerpolicy="no-referrer"/><span>{}</span></button>"#,esc(&cover.image_url),esc(&cover.title),f(app,"cover.select",&[("name",&cover.title)]),if app.busy(){"disabled"}else{""},esc(&cover.thumbnail_url),esc(&cover.title),esc(&cover.title)).unwrap();
        }
        html.push_str("</div>");
    }
    write!(
        html,
        r#"</div><footer class="cover-picker-footer">{}</footer></section></div>"#,
        button(
            "choose-cover",
            &game.id,
            t(app, "cover.local"),
            "folder",
            "quiet",
            app.busy()
        )
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_source_discards_an_outdated_cover_response() {
        let mut app = App::default();
        app.cover_picker.source = "bangumi".into();
        app.cover_picker.request = Some(2);
        resolve(
            &mut app,
            1,
            true,
            r#"[{"title":"Old","url":"https://vndb.org/v17","image_url":"old.jpg","thumbnail_url":"old.jpg"}]"#,
        );
        assert!(app.cover_picker.covers.is_empty() && app.cover_picker.request == Some(2));
    }

    #[test]
    fn custom_cover_search_survives_source_changes_without_changing_game_metadata() {
        let mut app = App::default();
        app.catalog.games.push(
            serde_json::from_value(json!({
                "id":"game", "title":"Original title", "folder":"F:/Game",
                "executable":"F:/Game/game.exe", "engine":"unknown", "cover":null,
                "modified":0, "metadata":{"url":"https://vndb.org/v1"}
            }))
            .unwrap(),
        );
        app.detail.selected = Some("game".into());
        dispatch(&mut app, "cover-picker", "game");
        let initial = crate::bridge::take_requests();
        let (request_id, command, payload) = &initial[0];
        assert_eq!(command, "source_covers");
        assert!(serde_json::from_str::<serde_json::Value>(payload).unwrap()["query"].is_null());
        assert_eq!(app.cover_picker.query, "Original title");
        app.requests.complete(*request_id);
        resolve(&mut app, *request_id, true, "[]");

        input(&mut app, "cover-query", "  Another title  ");
        dispatch(&mut app, "cover-search", "");
        let search = crate::bridge::take_requests();
        let payload: serde_json::Value = serde_json::from_str(&search[0].2).unwrap();
        assert_eq!(payload["query"], "Another title");
        assert_eq!(payload["source"], "vndb");
        dispatch(&mut app, "cover-provider", "bangumi");
        let switched = crate::bridge::take_requests();
        let payload: serde_json::Value = serde_json::from_str(&switched[0].2).unwrap();
        assert_eq!(payload["query"], "Another title");
        assert_eq!(payload["source"], "bangumi");
        resolve(
            &mut app,
            search[0].0,
            true,
            r#"[{"title":"Old","url":"https://vndb.org/v1","image_url":"old.jpg","thumbnail_url":"old.jpg"}]"#,
        );
        assert!(app.cover_picker.covers.is_empty());
        resolve(
            &mut app,
            switched[0].0,
            true,
            r#"[{"title":"Another title","url":"https://bgm.tv/subject/2","image_url":"new.jpg","thumbnail_url":"new.jpg"}]"#,
        );
        assert_eq!(app.cover_picker.covers[0].title, "Another title");
        assert_eq!(
            app.catalog.games[0].metadata.as_ref().unwrap().url,
            "https://vndb.org/v1"
        );
    }

    #[test]
    fn empty_cover_search_does_not_start_a_request() {
        let mut app = App::default();
        input(&mut app, "cover-query", "   ");
        dispatch(&mut app, "cover-search", "");
        assert!(crate::bridge::take_requests().is_empty());
    }
}
