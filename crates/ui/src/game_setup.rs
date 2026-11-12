//! Batch association and executable confirmation, with independent row drafts.
use crate::{
    choices, models::Game, models::Resource, models::Settings, models::SetupSearchResult, preferences, requests::Pending,
    requests::issue, state::{App, show_toast}, view::f, view::t, widgets::button, widgets::esc, widgets::icon,
};
use galgear_i18n::{Message, format_text};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, collections::BTreeSet, fmt::Write};

#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    rematch: Option<String>,
    page: usize,
    rows: Vec<Draft>,
    deferred: BTreeSet<String>,
    progress: Option<ScrapeProgress>,
}

struct ScrapeProgress {
    total: usize,
    completed: usize,
    failed: usize,
}

struct Draft {
    id: String,
    source: String,
    source_manual: bool,
    query: String,
    executable: String,
    confirmed: bool,
    url: String,
    candidates: Vec<Resource>,
    searched: bool,
    search_request: Option<u32>,
    automatic: bool,
    picking: bool,
    queued: bool,
    saving: bool,
    error: Option<Message>,
}

impl Draft {
    fn locked(&self) -> bool {
        self.automatic || self.queued || self.saving || self.picking
    }
    fn can_save(&self) -> bool {
        !self.locked()
            && !self.source.is_empty()
            && !self.url.is_empty()
            && self.confirmed

    }
}

impl State {
    fn scraping(&self) -> bool {
        self.rows.iter().any(|row| row.automatic)
    }
    fn busy(&self) -> bool {
        self.rows
            .iter()
            .any(Draft::locked)
    }
    fn queue_ready(&mut self) {
        for row in self.rows.iter_mut().filter(|row| row.can_save()) {
            row.queued = true;
            row.error = None;
        }
    }
}

pub(crate) fn visible(settings: &Settings, game: &Game) -> bool {
    !settings.excluded_games.contains(&game.id)
        && settings.game_edits.get(&game.id)
            .is_some_and(|edit| edit.setup_confirmed && !edit.setup_pending)
}

fn needed(app: &App, game: &Game) -> bool {
    !app.settings.excluded_games.contains(&game.id)
        && !app.setup.deferred.contains(&game.id)
        && !visible(&app.settings, game)
}

/// An explicit refresh reopens games deferred earlier in this run.
pub(crate) fn refreshed(app: &mut App) {
    app.setup.deferred.clear();
}

/// 尚未手动选择的配置行跟随默认来源，已有资料关联和手动选择保持不变。
pub(crate) fn sync_default_source(app: &mut App) {
    let source = preferences::preferred_source(app, "metadata").to_owned();
    for row in &mut app.setup.rows {
        if !row.source_manual && !row.locked() && row.url.is_empty() {
            change_source(row, &source);
        }
    }
}

fn draft(app: &App, game: &Game) -> Draft {
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
                preferences::preferred_source(app, "metadata").into()
            } else {
                preferences::source_for_url(&url).0.into()
            }
        });
    let available = preferences::available_source(app, &source).to_owned();
    let url = if source == available {
        url
    } else {
        String::new()
    };
    let source = available;
    Draft {
        id: game.id.clone(),
        source_manual: !url.is_empty()
            || app
                .settings
                .game_edits
                .get(&game.id)
                .is_some_and(|edit| !edit.metadata_provider.is_empty()),
        source,
        query: game.title.clone(),
        executable: game.executable.clone().unwrap_or_default(),
        confirmed: false,
        url,
        candidates: Vec::new(),
        searched: false,
        search_request: None,
        automatic: false,
        picking: false,
        queued: false,
        saving: false,
        error: None,
    }
}

/// 复用资料确认窗口；取消时保留详情草稿，成功保存后才丢弃草稿。
pub(crate) fn open_rematch(app: &mut App) {
    if !app.detail.editing
        || app.detail.editing_configuration
        || app.busy()
        || app.setup.open
        || !app.setup.rows.is_empty()
    {
        return;
    }
    let Some(game) = app
        .detail
        .selected
        .as_ref()
        .and_then(|id| app.catalog.games.iter().find(|game| &game.id == id))
    else {
        return;
    };
    let mut row = draft(app, game);
    row.query = crate::features::form(app, "draft-title");
    app.setup.rematch = Some(game.id.clone());
    app.setup.rows.push(row);
    app.setup.page = 0;
    app.setup.open = true;
    app.choices.open = None;
    app.context_menu = None;
}

pub(crate) fn sync(app: &mut App) {
    let game_ids: BTreeSet<_> = app
        .catalog
        .games
        .iter()
        .map(|game| game.id.as_str())
        .collect();
    app.setup
        .rows
        .retain(|row| game_ids.contains(row.id.as_str()));
    // 重新匹配期间不混入扫描发现的其他待确认游戏。
    if app.setup.rematch.is_some() {
        if app.setup.rows.is_empty() {
            app.setup.rematch = None;
            app.setup.open = false;
        }
        return;
    }
    if !app.initialized
        || app.busy()
        || app.library.scanning
        || matches!(
            app.library.dialog,
            Some(crate::library::Dialog::FolderFailures { .. })
        )
    {
        return;
    }
    let row_ids: BTreeSet<_> = app.setup.rows.iter().map(|row| row.id.as_str()).collect();
    let additions: Vec<_> = app
        .catalog
        .games
        .iter()
        .filter(|game| needed(app, game) && !row_ids.contains(game.id.as_str()))
        .map(|game| draft(app, game))
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
        row.confirmed = false;
    } else if !query && row.executable != value {
        row.executable = value.into();
        row.confirmed = false;
    }
    row.error = None;
    true
}

// Each save submits only ready drafts; bound concurrent network work to three games.
fn pump(app: &mut App) {
    // 已停止或换源的旧请求仍占用名额，收到响应后才发出下一项。
    let active = app.requests.values().filter(|request| {
        matches!(request, Pending::SetupSearch(_) | Pending::SetupSave(_))
    }).count();
    let slots = 3usize.saturating_sub(active);
    for _ in 0..slots {
        if let Some(row) = app.setup.rows.iter_mut().find(|row| row.automatic && row.search_request.is_none()) {
            row.search_request = Some(app.requests.next_id());
            let (id, source) = (row.id.clone(), row.source.clone());
            issue(app, Pending::SetupSearch(id.clone()), "search_game_metadata", json!({"id":id,"source":source}));
            continue;
        }
        let Some(row) = app.setup.rows.iter_mut().find(|row| row.queued) else {
            break;
        };
        row.queued = false;
        row.saving = true;
        let payload = json!({"id":row.id,"source":row.source,"url":row.url,"executable":"","confirmed":row.confirmed});
        let id = row.id.clone();
        let command = if app.setup.rematch.is_some() {
            "rematch_game_metadata"
        } else {
            "complete_game_setup"
        };
        issue(app, Pending::SetupSave(id), command, payload);
    }
    if !app.setup.scraping() {
        finish_progress(app, false);
    }
}

fn finish_progress(app: &mut App, stopped: bool) {
    let Some(progress) = app.setup.progress.take() else {
        return;
    };
    if progress.total == 0 {
        return;
    }
    let key = if stopped { "setup.auto_stopped" } else { "setup.auto_complete" };
    let message = Message::new(key)
        .arg("done", progress.completed)
        .arg("total", progress.total)
        .arg("failed", progress.failed);
    show_toast(app, message, false);
}

pub(crate) fn render_progress(html: &mut String, app: &App) {
    let Some(progress) = &app.setup.progress else {
        return;
    };
    write!(html, r#"<div id="setup-scrape-toast" class="toast save-ai-toast" role="status" aria-atomic="true"><span>{}</span></div>"#,
        f(app, "setup.auto_progress", &[
            ("done", &progress.completed.to_string()),
            ("total", &progress.total.to_string()),
            ("failed", &progress.failed.to_string()),
        ])).unwrap();
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !action.starts_with("setup-") {
        return false;
    }
    if !app.setup.open {
        return true;
    }
    if action == "setup-page" {
        if let Ok(page) = value.parse::<usize>() { app.setup.page = page.min(app.setup.rows.len().saturating_sub(1) / 40); app.choices.open = None; }
        return true;
    }
    if action == "setup-auto-stop" {
        for row in &mut app.setup.rows {
            if row.automatic {
                row.automatic = false;
                row.search_request = None;
            }
        }
        finish_progress(app, true);
        return true;
    }
    if action == "setup-defer-all" {
        if !app.setup.busy() {
            if app.setup.rematch.take().is_none() {
                app.setup
                    .deferred
                    .extend(app.setup.rows.iter().map(|row| row.id.clone()));
            }
            app.choices.open = None;
            app.setup.rows.clear();
            app.setup.progress = None;
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
    if action == "setup-auto" {
        if !app.setup.busy() {
            let mut total = 0;
            for row in app.setup.rows.iter_mut().filter(|row| !row.confirmed || row.url.is_empty()) {
                let Some(game) = app.catalog.games.iter().find(|game| game.id == row.id) else { continue };
                row.query.clone_from(&game.title);
                row.candidates.clear();
                row.url.clear();
                row.searched = false;
                row.search_request = None;
                row.confirmed = false;
                row.error = None;
                row.automatic = true;
                total += 1;
            }
            app.setup.progress = Some(ScrapeProgress { total, completed: 0, failed: 0 });
            pump(app);
        }
        return true;
    }
    if action == "setup-batch-source" {
        if !app.setup.busy() && preferences::source_available(app, value) {
            for row in &mut app.setup.rows {
                row.source_manual = true;
                change_source(row, value);
            }
        }
        return true;
    }
    let pair = serde_json::from_str::<[String; 2]>(value).ok();
    let (id, selected) = pair.as_ref().map_or((value, ""), |[id, selected]| {
        (id.as_str(), selected.as_str())
    });
    if action == "setup-source" && !preferences::source_available(app, selected) { return true; }
    let Some(row) = app.setup.rows.iter_mut().find(|row| row.id == id) else {
        return true;
    };
    if row.locked() { return true; }
    match action {
        "setup-source" => {
            row.source_manual = true;
            change_source(row, selected);
        }
        "setup-search" if !row.source.is_empty() && !row.query.trim().is_empty() => {
            row.search_request = Some(app.requests.next_id());
            row.candidates.clear();
            row.url.clear();
        row.confirmed = false;
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
                row.confirmed = false;
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
        pump(app);
        return true;
    };
    let row = &mut app.setup.rows[index];
    if matches!(kind, Pending::SetupSearch(_)) && row.search_request != Some(request_id) {
        pump(app);
        return true;
    }
    let automatic = row.automatic && matches!(kind, Pending::SetupSearch(_));
    if automatic { row.automatic = false; }
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
                    if automatic {
                        let result: SetupSearchResult = serde_json::from_str(payload)
                            .map_err(|error| error.to_string())?;
                        row.query = result.query;
                        row.candidates = result.candidates;
                        if let [first, _, ..] = row.candidates.as_slice() {
                            row.url.clone_from(&first.url);
                        } else if let ([candidate], Some(url)) =
                            (row.candidates.as_slice(), result.matched_url)
                            && !url.is_empty()
                            && candidate.url == url
                        {
                            row.url = url;
                            row.confirmed = true;
                        }
                    } else {
                        row.candidates = serde_json::from_str(payload)
                            .map_err(|error| error.to_string())?;
                    }
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
                        settings: Option<Settings>,
                        edit: Option<crate::models::GameEdit>,
                    }
                    let mut data: ResultData =
                        serde_json::from_str(payload).map_err(|error| error.to_string())?;
                    if data.game.id != *id {
                        return Err("Unexpected game in setup response".into());
                    }
                    let edit = data.edit.take().or_else(|| {
                        data.settings.as_mut().and_then(|settings| settings.game_edits.remove(id))
                    }).ok_or_else(|| "Missing game edit in setup response".to_owned())?;
                    if let Some(game) = app.catalog.games.iter_mut().find(|game| game.id == *id) {
                        crate::playtime::replace_game(game, data.game);
                    }
                    // Responses may finish out of order; merge only this game's edit.
                    app.settings.game_edits.insert(id.clone(), edit);
                    app.setup.deferred.insert(id.clone());
                }
                _ => {}
            }
            Ok(())
        })()
        .map_err(|error| Message::detail("error.read_data", error))
    };
    if automatic && let Some(progress) = &mut app.setup.progress {
        progress.completed += 1;
        if result.is_err() || !app.setup.rows[index].confirmed {
            progress.failed += 1;
        }
    }
    match result {
        Err(error) => app.setup.rows[index].error = Some(error),
        Ok(()) if matches!(kind, Pending::SetupSave(_)) => {
            app.setup.rows.remove(index);
            if app.setup.rematch.as_deref() == Some(id) {
                app.setup.rematch = None;
                app.detail.metadata_warnings.remove(id);
                if app.detail.selected.as_ref() == Some(id) {
                    crate::detail_form::cancel(app);
                }
            }
            app.choices.open = None;
        }
        _ => {}
    }
    if matches!(kind, Pending::SetupSearch(_) | Pending::SetupSave(_)) {
        pump(app);
        if app.setup.rows.is_empty() {
            app.setup.open = false;
        }
    }
    true
}

fn change_source(row: &mut Draft, source: &str) {
    if row.source == source { return; }
    row.source = source.into();
    row.url.clear();
    row.confirmed = false;
    row.candidates.clear();
    row.search_request = None;
    row.searched = false;
    row.error = None;
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
    if field == "setup-batch-source" { return app.setup.open && !app.setup.busy(); }
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

fn source_options(app: &App) -> Vec<choices::OptionItem> {
    preferences::available_sources(app)
        .map(|(source, name)| choices::item(source, name))
        .collect()
}

pub(crate) fn choice_options(app: &App, field: &str) -> Vec<choices::OptionItem> {
    if field == "setup-batch-source" {
        return source_options(app);
    }
    let Some((row, source)) = choice_row(app, field) else {
        return Vec::new();
    };
    if source {
        return source_options(app);
    }
    let mut options: Vec<_> = row
        .candidates
        .iter()
        .map(|candidate| choices::OptionItem {
            value: candidate.url.clone(), label: candidate.title.clone(),
            subtitle: if candidate.brand.is_empty() { t(app, "common.unknown") } else { &candidate.brand }.into(),
            detail: if candidate.release_date.is_empty() { t(app, "common.unknown") } else { &candidate.release_date }.into(),
            image: candidate.cover_url.clone().filter(|url| url.starts_with("https://") || url.starts_with("http://")),
        })
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
    write!(html,r#"<div class="setup-overlay" data-render-key="setup-overlay"><section class="library-dialog setup-dialog" role="dialog" aria-modal="true" aria-labelledby="setup-title" data-render-key="setup-batch"><header><div><h2 id="setup-title">{}</h2><p class="muted">{}</p></div><button id="setup-close" class="icon-button" data-action="setup-defer-all" aria-label="{}" {}>{}</button></header><div class="setup-help">{}</div><div class="setup-table-scroll" id="setup-table-scroll"><table class="setup-table">"#,t(app,if app.setup.rematch.is_some() { "detail.rematch" } else { "setup.title" }),if app.setup.rematch.is_some() { esc(t(app,"detail.rematch_hint")) } else { f(app,"setup.remaining",&[("count",&app.setup.rows.len().to_string())]) },t(app,"common.close"),if busy{"disabled"}else{""},icon("close"),format_args!("<div class=\"setup-actions\">{}{}</div>",button("choice-toggle","setup-batch-source",t(app,"setup.batch_source"),"database","quiet choice-trigger",busy).replacen("<button", "<button id=\"setup-batch-source\"", 1),button(if app.setup.scraping() { "setup-auto-stop" } else { "setup-auto" },"",t(app,if app.setup.scraping() { "setup.auto_stop" } else { "setup.auto" }),if app.setup.scraping() { "close" } else { "search" },"quiet",busy && !app.setup.scraping()))).unwrap();
    html.push_str("<thead><tr>");
    for key in [
        "setup.game",
        "setup.source",
        "setup.name",
        "setup.entry",
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
    let page = app.setup.page.min(app.setup.rows.len().saturating_sub(1) / 40);
    for row in app.setup.rows.iter().skip(page * 40).take(40) {
        let Some(game) = games.get(row.id.as_str()) else {
            continue;
        };
        let disabled = if row.locked() { "disabled" } else { "" };
        let status = if row.saving {
            "setup.saving"
        } else if row.queued {
            "setup.queued"
        } else if row.search_request.is_some() {
            "setup.searching"
        } else if row.automatic {
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
        html.push_str("</td><td><div class=\"setup-entry-field\">");
        if row.searched && row.candidates.is_empty() && row.url.is_empty() && row.error.is_none() {
            write!(
                html,
                r#"<div class="setup-notice" role="status">{}<span>{}</span></div>"#,
                icon("search"),
                t(app, "setup.no_results")
            )
            .unwrap();
        } else {
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
        }
        write!(html,r#"<button type="button" class="icon-button setup-launcher-check" data-action="setup-confirm-launcher" data-value="{}" aria-pressed="{}" aria-label="{}" title="{}" {}>{}</button></div></td></tr>"#,esc(&row.id),row.confirmed,f(app,"setup.confirm_for",&[("name",&game.title)]),t(app,"common.confirm"),if row.locked() || row.url.is_empty() {"disabled"} else {""},icon("check")).unwrap();
        if let Some(error) = &row.error {
            write!(html,r#"<tr data-render-key="setup-error-{}"><td colspan="4" class="setup-error"><div class="setup-notice" role="alert">{}<span>{}</span></div></td></tr>"#,esc(&row.id),icon("error"),esc(&error.render(app.settings.language))).unwrap();
        }
    }
    html.push_str("</tbody></table></div>");
    if app.setup.rows.len() > 40 {
        write!(html, "<div class=\"setup-pagination\">{}<span>{}</span>{}</div>", button("setup-page", &page.saturating_sub(1).to_string(), t(app,"common.previous"), "chevron_left", "quiet icon-only", page == 0), f(app,"setup.page", &[("page", &(page+1).to_string()), ("total", &app.setup.rows.len().div_ceil(40).to_string())]), button("setup-page", &(page+1).to_string(), t(app,"common.next"), "chevron", "quiet icon-only", (page+1)*40 >= app.setup.rows.len())).unwrap();
    }
    write!(html,r#"<footer class="library-dialog-footer setup-footer"><span class="muted">{}</span><div>{}{}</div></footer></section></div>"#,f(app,"setup.ready",&[("count",&ready.to_string())]),button("setup-defer-all","",t(app,if app.setup.rematch.is_some() { "common.cancel" } else { "setup.later" }),"","quiet",busy),button("setup-save","",t(app,if app.setup.rows.iter().any(|row| row.saving || row.queued) {"setup.saving"}else{"setup.save"}),"check","primary",busy||ready==0)).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GameEdit;

    fn rematch_app() -> App {
        let mut app = app();
        dispatch(&mut app, "setup-defer-all", "");
        app.detail.selected = Some("cached".into());
        crate::detail_form::begin(&mut app);
        app.form
            .insert("draft-title".into(), "Unsaved title".into());
        open_rematch(&mut app);
        app
    }

    #[test]
    fn rematch_isolated_from_imports_cancel_preserves_draft_and_ignores_late_search() {
        let mut app = rematch_app();
        refreshed(&mut app);
        sync(&mut app);
        assert_eq!(app.setup.rows.len(), 1);
        assert_eq!(app.setup.rows[0].id, "cached");
        assert_eq!(app.setup.rows[0].query, "Unsaved title");
        dispatch(&mut app, "setup-search", "cached");
        let request_id = app.requests.last_id();
        dispatch(&mut app, "setup-defer-all", "");
        open_rematch(&mut app);
        resolve(
            &mut app,
            &Pending::SetupSearch("cached".into()),
            request_id,
            true,
            r#"[{"title":"Late","url":"https://vndb.org/v99"}]"#,
        );
        assert!(app.setup.rows[0].candidates.is_empty());
        assert!(app.detail.editing);
        assert_eq!(crate::features::form(&app, "draft-title"), "Unsaved title");
    }

    #[test]
    fn rematch_failure_allows_retry_and_only_success_exits_editor() {
        let mut app = rematch_app();
        crate::bridge::take_requests();
        ready(&mut app, "cached");
        dispatch(&mut app, "setup-save", "");
        let (id, command, _) = crate::bridge::take_requests().pop().unwrap();
        assert_eq!(command, "rematch_game_metadata");
        let kind = app.requests.complete(id).unwrap();
        resolve(&mut app, &kind, id, false, r#"{"key":"error.network"}"#);
        assert!(app.setup.open && app.detail.editing);
        assert!(app.setup.rows[0].error.is_some());
        assert_eq!(
            app.catalog.games[0].metadata.as_ref().unwrap().url,
            "https://vndb.org/v1"
        );
        dispatch(&mut app, "setup-save", "");
        let (id, _, _) = crate::bridge::take_requests().pop().unwrap();
        dispatch(&mut app, "setup-defer-all", "");
        assert!(app.setup.open, "saving cannot be dismissed");
        let kind = app.requests.complete(id).unwrap();
        resolve(&mut app, &kind, id, true, &saved_response("cached"));
        assert!(!app.setup.open && !app.detail.editing);
        assert!(!app.form.contains_key("draft-title"));
        assert_eq!(
            app.catalog.games[0].metadata.as_ref().unwrap().url,
            "https://vndb.org/v3"
        );
    }

    #[test]
    fn default_source_changes_only_untouched_setup_rows() {
        let mut app = app();
        app.settings.metadata_provider = "bangumi".into();
        sync_default_source(&mut app);
        assert_eq!(app.setup.rows[0].source, "bangumi");
        dispatch(&mut app, "setup-source", r#"["new","vndb"]"#);
        app.settings.metadata_provider = "2dfan".into();
        sync_default_source(&mut app);
        assert_eq!(app.setup.rows[0].source, "vndb");
    }

    #[test]
    fn batch_source_uses_only_verified_sources_and_discards_late_search() {
        let mut app = app();
        assert_eq!(
            source_options(&app)
                .iter()
                .map(|o| o.value.as_str())
                .collect::<Vec<_>>(),
            vec!["nextmoe", "2dfan", "bangumi", "vndb", "erogamescape"]
        );
        let before = app.setup.rows[0].source.clone();
        dispatch(&mut app, "setup-batch-source", "ymgal");
        assert_eq!(app.setup.rows[0].source, before);
        dispatch(&mut app, "setup-search", "new");
        let old = app.requests.last_id();
        dispatch(&mut app, "setup-batch-source", "vndb");
        resolve(
            &mut app,
            &Pending::SetupSearch("new".into()),
            old,
            true,
            r#"[{"title":"old","url":"https://2dfmax.top/subjects/1"}]"#,
        );
        assert!(
            app.setup
                .rows
                .iter()
                .all(|r| r.source == "vndb" && r.url.is_empty() && r.candidates.is_empty())
        );
    }
    fn game(id: &str, url: &str) -> serde_json::Value {
        json!({"id":id,"title":id,"folder":"F:/Games/Test","executable":"F:/Games/Test/game.exe","engine":"unknown","cover":null,"modified":0,"metadata":if url.is_empty(){serde_json::Value::Null}else{json!({"url":url})}})
    }
    fn app() -> App {
        let mut app = App {
            initialized: true,
            ..Default::default()
        };
        app.data_sources = ["vndb", "bangumi"]
            .into_iter()
            .map(|source| preferences::SourceStatus {
                source: source.into(),
                configured: true,
                ..Default::default()
            })
            .collect();
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
        app.settings.game_edits.entry("cached".into()).or_default().setup_confirmed = true;
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
    fn only_confirmed_games_appear_on_home_and_in_library_across_defer_refresh_and_restore() {
        let mut app = app();
        // Imported metadata alone does not establish player confirmation.
        let visible_ids = |app: &App| crate::library::matching(&app.catalog, &app.settings, &app.library, "games")
            .iter().map(|game| game.id.clone()).collect::<Vec<_>>();
        assert_eq!(visible_ids(&app), vec!["cached"]);
        let mut html = String::new();
        crate::home::render(&mut html, &app);
        assert!(html.contains("home-game-cached"));
        assert!(!html.contains("home-game-new"));
        assert!(!html.contains("home-game-imported"));
        dispatch(&mut app, "setup-defer-all", "");
        assert_eq!(visible_ids(&app), vec!["cached"]);
        refreshed(&mut app);
        sync(&mut app);
        assert_eq!(app.setup.rows.len(), 2);
        ready(&mut app, "new");
        dispatch(&mut app, "setup-save", "");
        let requests = crate::bridge::take_requests();
        let id = requests[0].0;
        let kind = app.requests.complete(id).unwrap();
        resolve(&mut app, &kind, id, true, &saved_response("new"));
        assert_eq!(visible_ids(&app), vec!["cached", "new"]);
        app.settings.excluded_games.push("new".into());
        refreshed(&mut app);
        sync(&mut app);
        assert_eq!(visible_ids(&app), vec!["cached"]);
        assert!(!app.setup.rows.iter().any(|row| row.id == "new"));
        app.settings.excluded_games.clear();
        assert_eq!(visible_ids(&app), vec!["cached", "new"]);
    }

    #[test]
    fn automatic_scraping_confirms_backend_match_but_waits_for_user_to_save() {
        let mut app = app();
        crate::bridge::take_requests();
        dispatch(&mut app, "setup-auto", "");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 2);
        for (request_id, command, payload) in &requests {
            assert_eq!(command, "search_game_metadata");
            assert!(serde_json::from_str::<serde_json::Value>(payload).unwrap()["id"].is_string());
            let kind = app.requests.complete(*request_id).unwrap();
            resolve(
                &mut app,
                &kind,
                *request_id,
                true,
                r#"{"query":"Second","candidates":[{"title":"Second","url":"https://vndb.org/v2"}],"matched_url":"https://vndb.org/v2"}"#,
            );
        }
        assert!(crate::bridge::take_requests().is_empty());
        assert!(app.setup.open);
        assert!(!app.setup.busy());
        assert!(app.setup.rows.iter().all(|row| row.confirmed && row.can_save()));
        assert!(!app.settings.game_edits.get("new").is_some_and(|edit| edit.setup_confirmed));
        let mut html = String::new();
        render(&mut html, &app);
        assert_eq!(html.matches("aria-pressed=\"true\"").count(), 2);

        dispatch(&mut app, "setup-save", "");
        let saves = crate::bridge::take_requests();
        assert_eq!(saves.len(), 2);
        for (_, command, payload) in &saves {
            assert_eq!(command, "complete_game_setup");
            let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
            assert_eq!(payload["url"], "https://vndb.org/v2");
            assert_eq!(payload["confirmed"], true);
        }
    }

    #[test]
    fn automatic_scraping_allows_deferred_launcher_and_retains_empty_failed_rows_for_retry() {
        let mut app = app();
        crate::bridge::take_requests();
        app.setup.rows[0].executable.clear();
        dispatch(&mut app, "setup-auto", "");
        let requests = crate::bridge::take_requests();
        for (index, (request_id, _, _)) in requests.iter().enumerate() {
            let kind = app.requests.complete(*request_id).unwrap();
            resolve(&mut app, &kind, *request_id, true, if index == 0 { r#"{"query":"First","candidates":[{"title":"First","url":"https://vndb.org/v1"}],"matched_url":"https://vndb.org/v1"}"# } else { r#"{"query":"Other","candidates":[],"matched_url":null}"# });
        }
        assert!(crate::bridge::take_requests().is_empty());
        assert!(app.setup.rows[0].error.is_none());
        assert!(app.setup.rows[0].can_save());
        assert!(app.setup.rows[1].searched && app.setup.rows[1].candidates.is_empty());
        assert!(!app.setup.busy());
        assert!(app.setup.open);
        dispatch(&mut app, "setup-auto", "");
        for (request_id, _, _) in crate::bridge::take_requests() {
            let kind = app.requests.complete(request_id).unwrap();
            resolve(&mut app, &kind, request_id, false, r#"{"key":"ai.network_error"}"#);
        }
        assert!(!app.setup.busy());
        assert!(app.setup.rows[0].can_save());
        assert!(app.setup.rows[1].error.is_some());
    }

    #[test]
    fn unmatched_games_can_search_nextmoe_without_selecting_a_source() {
        let mut app = app();
        assert_eq!(choice_current(&app, "setup-source:new"), "nextmoe");
        dispatch(&mut app, "setup-search", "new");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "search_metadata");
        let payload: serde_json::Value = serde_json::from_str(&requests[0].2).unwrap();
        assert_eq!(payload, json!({"query":"new", "source":"nextmoe"}));
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
        assert!(app.setup.rows[0].can_save());
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
        assert!(app.setup.rows[0].can_save());
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
