//! Batch association and executable confirmation, with independent row drafts.
use crate::{
    choices, models::Game, models::Resource, models::Settings, preferences, requests::Pending,
    requests::issue, state::App, view::f, view::t, widgets::button, widgets::esc, widgets::icon,
};
use galgear_i18n::{Message, format_text};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, collections::BTreeSet, fmt::Write};

#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    rows: Vec<Draft>,
    deferred: BTreeSet<String>,
}

struct Draft {
    id: String,
    source: String,
    query: String,
    executable: String,
    confirmed: bool,
    url: String,
    candidates: Vec<Resource>,
    searched: bool,
    search_request: Option<u32>,
    picking: bool,
    queued: bool,
    saving: bool,
    error: Option<Message>,
}

impl Draft {
    fn locked(&self) -> bool {
        self.queued || self.saving || self.picking
    }
    fn can_save(&self) -> bool {
        !self.locked()
            && !self.source.is_empty()
            && !self.url.is_empty()
            && self.confirmed
            && self
                .executable
                .trim()
                .to_ascii_lowercase()
                .ends_with(".exe")
    }
}

impl State {
    fn busy(&self) -> bool {
        self.rows
            .iter()
            .any(|row| row.queued || row.saving || row.picking)
    }
    fn queue_ready(&mut self) {
        for row in self.rows.iter_mut().filter(|row| row.can_save()) {
            row.queued = true;
            row.error = None;
        }
    }
}

fn needed(app: &App, game: &Game) -> bool {
    !app.settings.excluded_games.contains(&game.id)
        && !app.setup.deferred.contains(&game.id)
        && (game
            .metadata
            .as_ref()
            .is_none_or(|meta| meta.url.trim().is_empty())
            || app
                .settings
                .game_edits
                .get(&game.id)
                .is_some_and(|edit| edit.setup_pending))
}

/// An explicit refresh reopens games deferred earlier in this run.
pub(crate) fn refreshed(app: &mut App) {
    app.setup.deferred.clear();
}

pub(crate) fn sync(app: &mut App) {
    app.setup
        .rows
        .retain(|row| app.catalog.games.iter().any(|game| game.id == row.id));
    if !app.initialized || app.busy() {
        return;
    }
    let additions: Vec<_> = app
        .catalog
        .games
        .iter()
        .filter(|game| needed(app, game) && !app.setup.rows.iter().any(|row| row.id == game.id))
        .map(|game| {
            let url = game
                .metadata
                .as_ref()
                .map(|meta| meta.url.clone())
                .unwrap_or_default();
            let source = app
                .settings
                .game_edits
                .get(&game.id)
                .map(|edit| edit.metadata_provider.clone())
                .filter(|source| !source.is_empty())
                .unwrap_or_else(|| {
                    if url.is_empty() {
                        crate::models::default_source()
                    } else {
                        preferences::source_for_url(&url).0.into()
                    }
                });
            Draft {
                id: game.id.clone(),
                source,
                query: game.title.clone(),
                executable: game.executable.clone().unwrap_or_default(),
                confirmed: false,
                url,
                candidates: Vec::new(),
                searched: false,
                search_request: None,
                picking: false,
                queued: false,
                saving: false,
                error: None,
            }
        })
        .collect();
    if !additions.is_empty() {
        app.setup.rows.extend(additions);
        app.setup.open = true;
        app.choices.open = None;
        app.context_menu = None;
    }
    if app.setup.rows.is_empty() {
        app.setup.open = false;
    }
}

pub(crate) fn input(app: &mut App, field: &str, value: &str) -> bool {
    let (id, query) = if let Some(id) = field.strip_prefix("setup-query:") {
        (id, true)
    } else if let Some(id) = field.strip_prefix("setup-executable:") {
        (id, false)
    } else {
        return false;
    };
    let Some(row) = app.setup.rows.iter_mut().find(|row| row.id == id) else {
        return true;
    };
    if row.locked() {
        return true;
    }
    if query && row.query != value {
        row.query = value.into();
        row.candidates.clear();
        row.searched = false;
        row.search_request = None;
        row.url.clear();
    } else if !query && row.executable != value {
        row.executable = value.into();
        row.confirmed = false;
    }
    row.error = None;
    true
}

// Each save submits only ready drafts; bound concurrent network work to three games.
fn pump(app: &mut App) {
    let slots = 3usize.saturating_sub(app.setup.rows.iter().filter(|row| row.saving).count());
    for _ in 0..slots {
        let Some(row) = app.setup.rows.iter_mut().find(|row| row.queued) else {
            break;
        };
        row.queued = false;
        row.saving = true;
        let payload = json!({"id":row.id,"source":row.source,"url":row.url,"executable":row.executable,"confirmed":row.confirmed});
        let id = row.id.clone();
        issue(app, Pending::SetupSave(id), "complete_game_setup", payload);
    }
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !action.starts_with("setup-") {
        return false;
    }
    if !app.setup.open {
        return true;
    }
    if action == "setup-defer-all" {
        if !app.setup.busy() {
            app.setup
                .deferred
                .extend(app.setup.rows.iter().map(|row| row.id.clone()));
            app.setup.rows.clear();
            app.setup.open = false;
        }
        return true;
    }
    if action == "setup-save" {
        if !app.setup.busy() {
            app.setup.queue_ready();
            pump(app);
        }
        return true;
    }
    let pair = serde_json::from_str::<[String; 2]>(value).ok();
    let (id, selected) = pair.as_ref().map_or((value, ""), |[id, selected]| {
        (id.as_str(), selected.as_str())
    });
    let Some(row) = app.setup.rows.iter_mut().find(|row| row.id == id) else {
        return true;
    };
    if row.locked() {
        return true;
    }
    match action {
        "setup-source"
            if preferences::SOURCES
                .iter()
                .any(|(source, _)| *source == selected) =>
        {
            if row.source != selected {
                row.source = selected.into();
                row.url.clear();
                row.candidates.clear();
                row.search_request = None;
                row.searched = false;
                row.error = None;
            }
        }
        "setup-search" if !row.source.is_empty() && !row.query.trim().is_empty() => {
            row.search_request = Some(app.requests.next_id());
            row.candidates.clear();
            row.url.clear();
            row.searched = false;
            row.error = None;
            let (id, source, query) = (row.id.clone(), row.source.clone(), row.query.clone());
            issue(
                app,
                Pending::SetupSearch(id),
                "search_metadata",
                json!({"query":query,"source":source}),
            );
        }
        "setup-candidate" => {
            if row
                .candidates
                .iter()
                .any(|candidate| candidate.url == selected)
            {
                row.url = selected.into();
                row.error = None;
            }
        }
        "setup-browse" => {
            // Native pickers share a window, so only open one at a time.
            if app.setup.rows.iter().any(|row| row.picking) {
                return true;
            }
            let row = app.setup.rows.iter_mut().find(|row| row.id == id);
            if let Some(row) = row {
                row.picking = true;
                issue(
                    app,
                    Pending::SetupExecutable(id.into()),
                    "choose_setup_executable",
                    json!({"id":id}),
                );
            }
        }
        "setup-confirm-launcher" => row.confirmed = !row.confirmed,
        _ => {}
    }
    true
}

pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    request_id: u32,
    success: bool,
    payload: &str,
) -> bool {
    let (Pending::SetupSearch(id) | Pending::SetupExecutable(id) | Pending::SetupSave(id)) = kind
    else {
        return false;
    };
    let Some(index) = app.setup.rows.iter().position(|row| row.id == *id) else {
        return true;
    };
    let row = &mut app.setup.rows[index];
    if matches!(kind, Pending::SetupSearch(_)) && row.search_request != Some(request_id) {
        return true;
    }
    match kind {
        Pending::SetupSearch(_) => row.search_request = None,
        Pending::SetupExecutable(_) => row.picking = false,
        Pending::SetupSave(_) => row.saving = false,
        _ => {}
    }
    let result: Result<(), Message> = if !success {
        Err(Message::from_error(payload))
    } else {
        (|| -> Result<(), String> {
            match kind {
                Pending::SetupSearch(_) => {
                    row.candidates =
                        serde_json::from_str(payload).map_err(|error| error.to_string())?;
                    row.searched = true;
                }
                Pending::SetupExecutable(_) => {
                    let path: Option<String> =
                        serde_json::from_str(payload).map_err(|error| error.to_string())?;
                    if let Some(path) = path {
                        row.executable = path;
                        row.confirmed = false;
                    }
                }
                Pending::SetupSave(_) => {
                    #[derive(Deserialize)]
                    struct ResultData {
                        game: Game,
                        settings: Settings,
                    }
                    let mut data: ResultData =
                        serde_json::from_str(payload).map_err(|error| error.to_string())?;
                    if data.game.id != *id {
                        return Err("Unexpected game in setup response".into());
                    }
                    if let Some(game) = app.catalog.games.iter_mut().find(|game| game.id == *id) {
                        *game = data.game;
                    }
                    // Responses may finish out of order; merge only this game's edit.
                    if let Some(edit) = data.settings.game_edits.remove(id) {
                        app.settings.game_edits.insert(id.clone(), edit);
                    }
                    app.setup.deferred.insert(id.clone());
                }
                _ => {}
            }
            Ok(())
        })()
        .map_err(|error| Message::detail("error.read_data", error))
    };
    match result {
        Err(error) => app.setup.rows[index].error = Some(error),
        Ok(()) if matches!(kind, Pending::SetupSave(_)) => {
            app.setup.rows.remove(index);
        }
        _ => {}
    }
    if matches!(kind, Pending::SetupSave(_)) {
        pump(app);
        if app.setup.rows.is_empty() {
            app.setup.open = false;
        }
    }
    true
}

fn choice_row<'a>(app: &'a App, field: &str) -> Option<(&'a Draft, bool)> {
    let (kind, id) = field.split_once(':')?;
    let source = match kind {
        "setup-source" => true,
        "setup-candidate" => false,
        _ => return None,
    };
    app.setup
        .rows
        .iter()
        .find(|row| row.id == id)
        .map(|row| (row, source))
}

pub(crate) fn choice_enabled(app: &App, field: &str) -> bool {
    app.setup.open
        && choice_row(app, field).is_some_and(|(row, source)| {
            !row.locked() && (source || !row.candidates.is_empty() || !row.url.is_empty())
        })
}

pub(crate) fn choice_current<'a>(app: &'a App, field: &str) -> &'a str {
    choice_row(app, field).map_or(
        "",
        |(row, source)| {
            if source { &row.source } else { &row.url }
        },
    )
}

pub(crate) fn choice_placeholder(app: &App, field: &str) -> Option<&'static str> {
    choice_row(app, field).map(|(row, source)| {
        t(
            app,
            if source {
                "setup.choose_source"
            } else if row.search_request.is_some() {
                "setup.searching"
            } else {
                "setup.choose_entry"
            },
        )
    })
}

pub(crate) fn choice_options(app: &App, field: &str) -> Vec<choices::OptionItem> {
    let Some((row, source)) = choice_row(app, field) else {
        return Vec::new();
    };
    if source {
        return preferences::SOURCES
            .iter()
            .map(|(source, name)| choices::item(source, name))
            .collect();
    }
    let mut options: Vec<_> = row
        .candidates
        .iter()
        .map(|candidate| choices::item(&candidate.url, &candidate.title))
        .collect();
    if !row.url.is_empty()
        && !options.iter().any(|option| option.value == row.url)
        && let Some(game) = app.catalog.games.iter().find(|game| game.id == row.id)
    {
        options.insert(0, choices::item(&row.url, &game.title));
    }
    options
}

pub(crate) fn render(html: &mut String, app: &App) {
    if !app.setup.open {
        return;
    }
    let busy = app.setup.busy();
    let ready = app.setup.rows.iter().filter(|row| row.can_save()).count();
    write!(html,r#"<div class="setup-overlay" data-render-key="setup-overlay"><section class="library-dialog setup-dialog" role="dialog" aria-modal="true" aria-labelledby="setup-title" data-render-key="setup-batch"><header><div><h2 id="setup-title">{}</h2><p class="muted">{}</p></div><button id="setup-close" class="icon-button" data-action="setup-defer-all" aria-label="{}" {}>{}</button></header><p class="setup-help">{}</p><div class="setup-table-scroll" id="setup-table-scroll"><table class="setup-table"><thead><tr>"#,t(app,"setup.title"),f(app,"setup.remaining",&[("count",&app.setup.rows.len().to_string())]),t(app,"common.close"),if busy{"disabled"}else{""},icon("close"),t(app,"setup.batch_help")).unwrap();
    for key in [
        "setup.game",
        "setup.source",
        "setup.entry",
        "setup.executable",
    ] {
        write!(html, "<th scope=\"col\">{}</th>", t(app, key)).unwrap();
    }
    html.push_str("</tr></thead><tbody>");
    let games: BTreeMap<_, _> = app
        .catalog
        .games
        .iter()
        .map(|game| (game.id.as_str(), game))
        .collect();
    for row in &app.setup.rows {
        let Some(game) = games.get(row.id.as_str()) else {
            continue;
        };
        let disabled = if row.locked() { "disabled" } else { "" };
        let status = if row.saving {
            "setup.saving"
        } else if row.queued {
            "setup.queued"
        } else if row.confirmed {
            "setup.confirmed"
        } else {
            "setup.pending"
        };
        write!(html,r#"<tr data-render-key="setup-row-{}" data-setup-game="{}"><td class="setup-game"><strong title="{}">{}</strong><small title="{}">{}</small><span class="setup-row-status" role="status">{}</span></td><td>"#,esc(&row.id),esc(&row.id),esc(&game.title),esc(&game.title),esc(&game.folder),esc(&game.folder),t(app,status)).unwrap();
        choices::control(
            html,
            app,
            &format!("setup-source:{}", row.id),
            &format_text(
                app.settings.language,
                "setup.source_for",
                &[("name", &game.title)],
            ),
            row.locked(),
        );
        write!(html,r#"</td><td><div class="setup-row-field"><input id="setup-query-{}" data-input="setup-query:{}" value="{}" aria-label="{}" autocomplete="off" {}/><button class="icon-button" data-action="setup-search" data-value="{}" aria-label="{}" title="{}" {}>{}</button></div>"#,esc(&row.id),esc(&row.id),esc(&row.query),f(app,"setup.search_for",&[("name",&game.title)]),disabled,esc(&row.id),t(app,"metadata.search"),t(app,"metadata.search"),if row.locked()||row.source.is_empty()||row.query.trim().is_empty()||row.search_request.is_some(){"disabled"}else{""},icon("search")).unwrap();
        choices::control(
            html,
            app,
            &format!("setup-candidate:{}", row.id),
            &format_text(
                app.settings.language,
                "setup.entry_for",
                &[("name", &game.title)],
            ),
            row.locked() || row.candidates.is_empty() && row.url.is_empty(),
        );
        if row.searched && row.candidates.is_empty() {
            write!(
                html,
                "<small class=\"muted\">{}</small>",
                t(app, "setup.no_results")
            )
            .unwrap();
        }
        write!(html,r#"</td><td><div class="setup-row-field"><input id="setup-executable-{}" data-input="setup-executable:{}" value="{}" title="{}" aria-label="{}" spellcheck="false" {}/><button class="icon-button" data-action="setup-browse" data-value="{}" title="{}" aria-label="{}" {}>{}</button><button type="button" class="icon-button setup-launcher-check" data-action="setup-confirm-launcher" data-value="{}" aria-pressed="{}" aria-label="{}" title="{}" {}>{}</button></div></td></tr>"#,esc(&row.id),esc(&row.id),esc(&row.executable),esc(&row.executable),f(app,"setup.executable_for",&[("name",&game.title)]),disabled,esc(&row.id),t(app,"setup.browse"),f(app,"setup.executable_for",&[("name",&game.title)]),if row.locked()||app.setup.rows.iter().any(|row| row.picking){"disabled"}else{""},icon("folder"),esc(&row.id),row.confirmed,f(app,"setup.confirm_for",&[("name",&game.title)]),t(app,"setup.confirm_launcher"),disabled,icon("check")).unwrap();
        if let Some(error) = &row.error {
            write!(html,r#"<tr data-render-key="setup-error-{}"><td colspan="4" class="setup-error" role="alert">{}</td></tr>"#,esc(&row.id),esc(&error.render(app.settings.language))).unwrap();
        }
    }
    write!(html,r#"</tbody></table></div><footer class="library-dialog-footer setup-footer"><span class="muted">{}</span><div>{}{}</div></footer></section></div>"#,f(app,"setup.ready",&[("count",&ready.to_string())]),button("setup-defer-all","",t(app,"setup.later"),"","quiet",busy),button("setup-save","",t(app,if busy{"setup.saving"}else{"setup.save"}),"check","primary",busy||ready==0)).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GameEdit;
    fn game(id: &str, url: &str) -> serde_json::Value {
        json!({"id":id,"title":id,"folder":"F:/Games/Test","executable":"F:/Games/Test/game.exe","engine":"unknown","cover":null,"modified":0,"metadata":if url.is_empty(){serde_json::Value::Null}else{json!({"url":url})}})
    }
    fn app() -> App {
        let mut app = App {
            initialized: true,
            ..Default::default()
        };
        app.catalog.games = [
            game("cached", "https://vndb.org/v1"),
            game("new", ""),
            game("imported", "https://bgm.tv/subject/2"),
        ]
        .into_iter()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect();
        app.settings.game_edits.insert(
            "imported".into(),
            GameEdit {
                setup_pending: true,
                metadata_provider: "bangumi".into(),
                ..Default::default()
            },
        );
        sync(&mut app);
        app
    }
    fn ready(app: &mut App, id: &str) {
        let row = app.setup.rows.iter_mut().find(|row| row.id == id).unwrap();
        row.source = "vndb".into();
        row.url = "https://vndb.org/v3".into();
        row.confirmed = true;
    }
    fn saved_response(id: &str) -> String {
        json!({
            "game": game(id, "https://vndb.org/v3"),
            "settings": {
                "library_folders": [], "card_size": 200, "sort_by": "title",
                "game_edits": {(id): {"metadata_provider": "vndb", "setup_confirmed": true}}
            }
        })
        .to_string()
    }
    #[test]
    fn unmatched_games_can_search_2dfan_without_selecting_a_source() {
        let mut app = app();
        assert_eq!(choice_current(&app, "setup-source:new"), "2dfan");
        dispatch(&mut app, "setup-search", "new");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "search_metadata");
        let payload: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
        assert_eq!(payload, json!({"query":"new", "source":"2dfan"}));
        assert!(!app.setup.rows[0].can_save());
        assert_eq!(app.setup.rows[1].source, "bangumi");
    }
    #[test]
    fn pending_games_keep_their_existing_metadata_source() {
        let mut app = app();
        app.settings.game_edits.insert(
            "cached".into(),
            GameEdit {
                setup_pending: true,
                ..Default::default()
            },
        );
        sync(&mut app);
        assert_eq!(choice_current(&app, "setup-source:cached"), "vndb");
        assert_eq!(
            choice_current(&app, "setup-candidate:cached"),
            "https://vndb.org/v1"
        );
    }
    #[test]
    fn all_pending_games_render_as_independent_rows() {
        let mut app = app();
        assert_eq!(
            app.setup
                .rows
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "imported"]
        );
        dispatch(
            &mut app,
            "setup-source",
            &json!(["new", "vndb"]).to_string(),
        );
        dispatch(&mut app, "setup-confirm-launcher", "new");
        input(&mut app, "setup-executable:new", "F:/other.exe");
        assert_eq!(app.setup.rows[0].source, "vndb");
        assert!(!app.setup.rows[0].confirmed);
        assert_eq!(app.setup.rows[1].source, "bangumi");
        let mut html = String::new();
        render(&mut html, &app);
        assert_eq!(html.matches("data-setup-game=").count(), 2);
        assert!(html.contains("setup-table-scroll"));
        assert_eq!(html.matches("data-action=\"setup-save\"").count(), 1);
    }
    #[test]
    fn explicit_refresh_reopens_deferred_unmatched_games() {
        let mut app = app();
        dispatch(&mut app, "setup-defer-all", "");
        sync(&mut app);
        assert!(!app.setup.open);
        refreshed(&mut app);
        sync(&mut app);
        assert!(app.setup.open);
        assert_eq!(app.setup.rows.len(), 2);
    }
    #[test]
    fn source_changes_reject_old_searches_without_changing_other_rows() {
        let mut app = app();
        app.setup.rows[0].search_request = Some(12);
        dispatch(
            &mut app,
            "setup-source",
            &json!(["new", "bangumi"]).to_string(),
        );
        resolve(
            &mut app,
            &Pending::SetupSearch("new".into()),
            12,
            true,
            r#"[{"title":"Old","url":"https://vndb.org/v1"}]"#,
        );
        assert!(app.setup.rows[0].candidates.is_empty());
        assert_eq!(app.setup.rows[1].url, "https://bgm.tv/subject/2");
    }
    #[test]
    fn one_confirmation_queues_all_ready_rows() {
        let mut app = app();
        ready(&mut app, "new");
        ready(&mut app, "imported");
        app.setup.queue_ready();
        assert!(app.setup.rows.iter().all(|row| row.queued));
        assert!(app.setup.busy());
    }
    #[test]
    fn successive_partial_saves_keep_the_dialog_open_until_all_games_are_saved() {
        let mut app = app();
        input(&mut app, "setup-query:imported", "Unfinished search");
        for (index, id) in ["new", "imported"].into_iter().enumerate() {
            ready(&mut app, id);
            let mut html = String::new();
            render(&mut html, &app);
            let save_button = html
                .split("data-action=\"setup-save\"")
                .nth(1)
                .unwrap()
                .split('>')
                .next()
                .unwrap();
            assert!(!save_button.contains("disabled"));
            dispatch(&mut app, "setup-save", "");
            let requests = crate::bridge::take_requests();
            assert_eq!(requests.len(), 1);
            let (request_id, command, payload) = &requests[0];
            assert_eq!(command, "complete_game_setup");
            let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
            assert_eq!(payload["id"], id);
            let kind = app.requests.complete(*request_id).unwrap();
            resolve(&mut app, &kind, *request_id, true, &saved_response(id));
            sync(&mut app);
            assert!(!app.setup.busy());
            if index == 0 {
                assert!(app.setup.open);
                assert_eq!(app.setup.rows.len(), 1);
                assert_eq!(app.setup.rows[0].id, "imported");
                assert_eq!(app.setup.rows[0].query, "Unfinished search");
                assert!(!app.setup.rows[0].locked());
                assert!(!app.setup.rows[0].confirmed);
                assert_eq!(app.setup.rows[0].source, "bangumi");
                assert!(app.setup.rows[0].url.is_empty());
                let mut html = String::new();
                render(&mut html, &app);
                assert_eq!(html.matches("data-setup-game=").count(), 1);
                assert!(!html.contains("data-setup-game=\"new\""));
                assert!(html.contains(&f(&app, "setup.remaining", &[("count", "1")])));
                dispatch(&mut app, "setup-save", "");
                assert!(crate::bridge::take_requests().is_empty());
            }
        }
        assert!(!app.setup.open);
        assert!(app.setup.rows.is_empty());
        assert!(app.settings.game_edits["new"].setup_confirmed);
        assert!(app.settings.game_edits["imported"].setup_confirmed);
        dispatch(&mut app, "setup-save", "");
        assert!(crate::bridge::take_requests().is_empty());
        sync(&mut app);
        assert!(!app.setup.open);
    }
    #[test]
    fn failed_final_save_keeps_the_dialog_open_until_retry_succeeds() {
        let mut app = app();
        ready(&mut app, "new");
        ready(&mut app, "imported");
        dispatch(&mut app, "setup-save", "");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 2);
        let (first_id, _, _) = &requests[0];
        let (second_id, _, _) = &requests[1];
        let kind = app.requests.complete(*second_id).unwrap();
        resolve(
            &mut app,
            &kind,
            *second_id,
            true,
            &saved_response("imported"),
        );
        sync(&mut app);
        assert!(app.setup.open);
        assert!(app.setup.busy());
        assert_eq!(app.setup.rows.len(), 1);
        assert_eq!(app.setup.rows[0].id, "new");
        let kind = app.requests.complete(*first_id).unwrap();
        resolve(
            &mut app,
            &kind,
            *first_id,
            false,
            r#"{"key":"error.network","args":{}}"#,
        );
        sync(&mut app);
        assert!(app.setup.open);
        assert!(!app.setup.busy());
        assert!(app.setup.rows[0].error.is_some());
        assert!(app.setup.rows[0].confirmed);
        assert_eq!(app.setup.rows[0].url, "https://vndb.org/v3");
        assert_eq!(app.setup.rows.len(), 1);
        dispatch(&mut app, "setup-save", "");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        let (request_id, _, payload) = &requests[0];
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(payload["id"], "new");
        let kind = app.requests.complete(*request_id).unwrap();
        resolve(&mut app, &kind, *request_id, true, &saved_response("new"));
        sync(&mut app);
        assert!(!app.setup.open);
        assert!(app.setup.rows.is_empty());
    }
    #[test]
    fn failed_rows_keep_their_draft_and_out_of_order_saves_preserve_other_edits() {
        let mut app = app();
        ready(&mut app, "new");
        resolve(
            &mut app,
            &Pending::SetupSave("new".into()),
            15,
            false,
            r#"{"key":"error.network","args":{}}"#,
        );
        assert!(app.setup.rows[0].error.is_some());
        assert_eq!(app.setup.rows[0].url, "https://vndb.org/v3");
        let payload=json!({"game":game("new","https://vndb.org/v3"),"settings":{"library_folders":[],"card_size":200,"sort_by":"title","game_edits":{"new":{"metadata_provider":"vndb","setup_confirmed":true},"imported":{"metadata_provider":"outdated"}}}}).to_string();
        resolve(
            &mut app,
            &Pending::SetupSave("new".into()),
            16,
            true,
            &payload,
        );
        assert!(app.setup.open);
        assert_eq!(app.setup.rows.len(), 1);
        assert_eq!(app.setup.rows[0].id, "imported");
        assert_eq!(
            app.settings.game_edits["imported"].metadata_provider,
            "bangumi"
        );
        assert_eq!(app.settings.game_edits["new"].metadata_provider, "vndb");
    }

    #[test]
    fn removing_saved_rows_keeps_concurrent_saves_and_queued_games_running() {
        let mut app = app();
        for id in ["third", "fourth", "fifth"] {
            app.catalog
                .games
                .push(serde_json::from_value(game(id, "")).unwrap());
        }
        sync(&mut app);
        for id in ["new", "imported", "third", "fourth", "fifth"] {
            ready(&mut app, id);
        }
        dispatch(&mut app, "setup-save", "");
        let mut requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 3);
        let mut completed = BTreeSet::new();
        while let Some((request_id, command, payload)) = requests.pop() {
            assert_eq!(command, "complete_game_setup");
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
            let id = payload["id"].as_str().unwrap();
            assert!(completed.insert(id.to_owned()));
            let kind = app.requests.complete(request_id).unwrap();
            resolve(&mut app, &kind, request_id, true, &saved_response(id));
            sync(&mut app);
            assert!(app.setup.rows.iter().all(|row| row.id != id));
            assert_eq!(app.setup.rows.len(), 5 - completed.len());
            assert_eq!(app.setup.open, !app.setup.rows.is_empty());
            requests.extend(crate::bridge::take_requests());
            assert!(requests.len() <= 3);
        }
        assert_eq!(completed.len(), 5);
        assert!(!app.setup.open);
        assert!(!app.setup.busy());
    }

    #[test]
    fn invalid_save_response_keeps_the_row_in_the_dialog() {
        let mut app = app();
        ready(&mut app, "new");
        resolve(
            &mut app,
            &Pending::SetupSave("new".into()),
            15,
            true,
            &saved_response("unexpected"),
        );
        assert!(app.setup.open);
        assert_eq!(app.setup.rows.len(), 2);
        assert!(app.setup.rows[0].error.is_some());
        assert!(app.setup.rows[0].can_save());
    }
}
