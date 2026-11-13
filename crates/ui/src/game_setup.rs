//! Batch association and executable confirmation, with independent row drafts.
use crate::{
    choices, models::Game, models::Resource, models::Settings, preferences, requests::Pending,
    requests::issue,
    state::{App, show_toast},
    view::{f, t},
    widgets::{button, esc, icon, progress_toast},
};
use galgear_i18n::{Message, format_text};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, collections::BTreeSet, fmt::Write};

#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    rematch: Option<String>,
    viewport: Viewport,
    rows: Vec<Draft>,
    deferred: BTreeSet<String>,
    progress: Option<ScrapeProgress>,
    save_progress: Option<SaveProgress>,
    batch_progress: Option<(u32, usize)>,
}

#[derive(PartialEq)]
struct Viewport {
    start: usize,
    count: usize,
    pinned: String,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            start: 0,
            count: 32,
            pinned: String::new(),
        }
    }
}

/// DOM 只上报可见范围和焦点所在行；完整草稿与刮削队列始终由 Rust 持有。
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn setup_viewport(start: usize, count: usize, pinned: &str) {
    crate::state::APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.setup.open {
            return;
        }
        let viewport = Viewport {
            start: start.min(app.setup.rows.len().saturating_sub(1)),
            count: count.clamp(16, 80),
            pinned: pinned.to_owned(),
        };
        if app.setup.viewport != viewport {
            app.setup.viewport = viewport;
            crate::view::paint(&app);
        }
    });
}

struct ScrapeProgress {
    total: usize,
    completed: usize,
    failed: usize,
}

struct SaveProgress {
    total: usize,
    saved: usize,
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
        let mut total = 0;
        for row in self.rows.iter_mut().filter(|row| row.can_save()) {
            row.queued = true;
            row.error = None;
            total += 1;
        }
        self.save_progress = (total > 0).then_some(SaveProgress { total, saved: 0 });
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
    app.setup.viewport = Viewport::default();
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
        if !app.setup.open {
            app.setup.viewport = Viewport::default();
        }
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

// 批量保存交给后端调度，前端限制自动搜索和单条保存入口的在途数量。
fn pump(app: &mut App) {
    if app.setup.rematch.is_none() && app.setup.rows.iter().filter(|row| row.queued).count() > 1 {
        let mut ids = Vec::new();
        let mut inputs = Vec::new();
        for row in app.setup.rows.iter_mut().filter(|row| row.queued) {
            row.queued = false;
            row.saving = true;
            ids.push(row.id.clone());
            inputs.push(json!({"id":row.id,"source":row.source,"url":row.url,"executable":"","confirmed":row.confirmed}));
        }
        let request_id = app.requests.next_id();
        app.setup.batch_progress = Some((request_id, 0));
        issue(
            app,
            Pending::SetupBatch(ids),
            "complete_game_setup_batch",
            json!({"inputs":inputs,"requestId":request_id}),
        );
    }
    // 已停止或换源的旧请求仍占用名额，收到响应后才发出下一项。
    let active = app.requests.values().filter(|request| {
        matches!(request, Pending::SetupSearch(_) | Pending::SetupSave(_))
    }).count();
    let slots = 3usize.saturating_sub(active);
    for _ in 0..slots {
        if let Some(row) = app.setup.rows.iter_mut().find(|row| row.automatic && row.search_request.is_none()) {
            row.search_request = Some(app.requests.next_id());
            let (id, source, query) = (row.id.clone(), row.source.clone(), row.query.clone());
            issue(app, Pending::SetupSearch(id), "search_metadata", json!({"query":query,"source":source}));
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
    if !app.setup.rows.iter().any(|row| row.queued || row.saving)
        && !app
            .requests
            .values()
            .any(|request| matches!(request, Pending::SetupSave(_)))
        && let Some(progress) = app.setup.save_progress.take()
    {
        let failed = progress.total - progress.saved;
        let key = if failed == 0 {
            "setup.save_complete"
        } else {
            "setup.save_failed"
        };
        show_toast(
            app,
            Message::new(key)
                .arg("saved", progress.saved)
                .arg("total", progress.total)
                .arg("failed", failed),
            failed > 0,
        );
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

/// 进度事件按请求编号隔离，提交响应到达前不把准备完成计为保存成功。
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn setup_save_progress(payload: &str) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Progress {
        request_id: u32,
        done: usize,
        total: usize,
    }
    let Ok(progress) = serde_json::from_str::<Progress>(payload) else {
        return;
    };
    crate::state::APP.with(|state| {
        let mut app = state.borrow_mut();
        let total = app.setup.save_progress.as_ref().map(|value| value.total);
        if let Some((id, done)) = &mut app.setup.batch_progress
            && *id == progress.request_id
            && total == Some(progress.total)
        {
            *done = (*done).max(progress.done.min(progress.total));
            crate::view::paint(&app);
        }
    });
}

pub(crate) fn render_progress(html: &mut String, app: &App) {
    if let Some(progress) = &app.setup.progress {
        progress_toast(
            html,
            "setup-scrape-toast",
            &f(
                app,
                "setup.auto_progress",
                &[
                    ("done", &progress.completed.to_string()),
                    ("total", &progress.total.to_string()),
                    ("failed", &progress.failed.to_string()),
                ],
            ),
        );
    }
    if let Some(progress) = &app.setup.save_progress {
        let (key, done) = match app.setup.batch_progress {
            Some((_, done)) => ("setup.fetch_progress", done),
            None => ("setup.save_progress", 0),
        };
        progress_toast(
            html,
            "setup-save-toast",
            &f(
                app,
                key,
                &[
                    ("done", &done.to_string()),
                    ("saved", &progress.saved.to_string()),
                    ("total", &progress.total.to_string()),
                ],
            ),
        );
    }
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !action.starts_with("setup-") {
        return false;
    }
    if !app.setup.open {
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
            app.setup.save_progress = None;
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
            let mut empty = 0;
            for row in app.setup.rows.iter_mut().filter(|row| !row.confirmed || row.url.is_empty()) {
                row.candidates.clear();
                row.url.clear();
                row.searched = row.query.trim().is_empty();
                row.search_request = None;
                row.confirmed = false;
                row.error = None;
                row.automatic = !row.searched;
                if row.searched { empty += 1; }
                total += 1;
            }
            app.setup.progress = Some(ScrapeProgress { total, completed: empty, failed: empty });
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
        "setup-open-folder" => issue(app, Pending::Open, "open_game_folder", json!({"id":id})),
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
    if let Pending::SetupBatch(ids) = kind {
        app.setup.batch_progress = None;
        #[derive(Deserialize)]
        struct Batch {
            items: Vec<Item>,
            warning: Option<Message>,
        }
        #[derive(Deserialize)]
        struct Item {
            id: String,
            result: Result<serde_json::Value, Message>,
        }
        let batch = if success {
            serde_json::from_str::<Batch>(payload)
                .map_err(|error| Message::detail("error.read_data", error))
        } else {
            Err(Message::from_error(payload))
        };
        let (mut results, warning) = match batch {
            Ok(batch) => (
                batch
                    .items
                    .into_iter()
                    .map(|item| (item.id, item.result))
                    .collect::<BTreeMap<_, _>>(),
                batch.warning,
            ),
            Err(error) => (
                ids.iter()
                    .map(|id| (id.clone(), Err(error.clone())))
                    .collect(),
                None,
            ),
        };
        for id in ids {
            let result = results
                .remove(id)
                .unwrap_or_else(|| Err(Message::new("error.read_data")));
            let (success, payload) = match result {
                Ok(value) => (true, value.to_string()),
                Err(error) => (false, serde_json::to_string(&error).unwrap_or_default()),
            };
            resolve(
                app,
                &Pending::SetupSave(id.clone()),
                request_id,
                success,
                &payload,
            );
        }
        if let Some(warning) = warning {
            show_toast(app, warning, true);
        }
        return true;
    }
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
                    row.candidates = serde_json::from_str(payload)
                        .map_err(|error| error.to_string())?;
                    if automatic && let Some(first) = row.candidates.first() {
                        row.url.clone_from(&first.url);
                        row.confirmed = !row.url.is_empty()
                            && (row.candidates.len() == 1 || first.title == row.query);
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
            if let Some(progress) = &mut app.setup.save_progress {
                progress.saved += 1;
            }
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
    let row_count = app.setup.rows.len() + 1;
    let busy = app.setup.busy();
    let ready = app.setup.rows.iter().filter(|row| row.can_save()).count();
    write!(html,r#"<div class="setup-overlay" data-render-key="setup-overlay"><section class="library-dialog setup-dialog" role="dialog" aria-modal="true" aria-labelledby="setup-title" data-render-key="setup-batch"><header><div><h2 id="setup-title">{}</h2><p class="muted">{}</p></div><button id="setup-close" class="icon-button" data-action="setup-defer-all" aria-label="{}" {}>{}</button></header><div class="setup-help">{}</div><div class="setup-table-scroll" id="setup-table-scroll"><table class="setup-table" aria-rowcount="{row_count}">"#,t(app,if app.setup.rematch.is_some() { "detail.rematch" } else { "setup.title" }),if app.setup.rematch.is_some() { esc(t(app,"detail.rematch_hint")) } else { f(app,"setup.remaining",&[("count",&app.setup.rows.len().to_string())]) },t(app,"common.close"),if busy{"disabled"}else{""},icon("close"),format_args!("<div class=\"setup-actions\">{}{}</div>",button("choice-toggle","setup-batch-source",t(app,"setup.batch_source"),"database","quiet choice-trigger",busy).replacen("<button", "<button id=\"setup-batch-source\"", 1),button(if app.setup.scraping() { "setup-auto-stop" } else { "setup-auto" },"",t(app,if app.setup.scraping() { "setup.auto_stop" } else { "setup.auto" }),if app.setup.scraping() { "close" } else { "search" },"quiet",busy && !app.setup.scraping()))).unwrap();
    html.push_str("<thead><tr>");
    for key in [
        "setup.game",
        "setup.source",
        "setup.name",
        "setup.entry",
    ] {
        write!(html, "<th scope=\"col\">{}</th>", t(app, key)).unwrap();
    }
    let start = app.setup.viewport.start.min(app.setup.rows.len().saturating_sub(app.setup.viewport.count));
    write!(html, r#"</tr></thead><tbody data-setup-total="{}" data-setup-start="{start}" data-setup-count="{}" data-setup-pinned="{}">"#, app.setup.rows.len(), app.setup.viewport.count, esc(&app.setup.viewport.pinned)).unwrap();
    let games: BTreeMap<_, _> = app
        .catalog
        .games
        .iter()
        .map(|game| (game.id.as_str(), game))
        .collect();
    let end = (start + app.setup.viewport.count).min(app.setup.rows.len());
    let mut previous = 0;
    for (index, row) in app.setup.rows.iter().enumerate().filter(|(index, row)| {
        (start..end).contains(index) || row.id == app.setup.viewport.pinned
    }) {
        render_spacer(html, previous, index);
        previous = index + 1;
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
        write!(html,r#"<tr data-render-key="setup-row-{}" data-setup-game="{}" data-setup-index="{index}" aria-rowindex="{}"><td class="setup-game"><strong title="{}">{}</strong><span class="setup-path" role="link" tabindex="{}" aria-disabled="{}" data-action="setup-open-folder" data-value="{}" title="{}" aria-label="{}: {}">{}</span><span class="setup-row-status" role="status">{}</span></td><td>"#,esc(&row.id),esc(&row.id),index + 2,esc(&game.title),esc(&game.title),if row.locked() { -1 } else { 0 },row.locked(),esc(&row.id),esc(&game.folder),t(app,"detail.open_directory"),esc(&game.folder),esc(&game.folder),t(app,status)).unwrap();
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
        if let Some(error) = &row.error {
            write!(html, r#"<div class="setup-notice" role="alert">{}<span>{}</span></div>"#, icon("error"), esc(&error.render(app.settings.language))).unwrap();
        }
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
    }
    render_spacer(html, previous, app.setup.rows.len());
    html.push_str("</tbody></table></div>");
    write!(html,r#"<footer class="library-dialog-footer setup-footer"><span class="muted">{}</span><div>{}{}</div></footer></section></div>"#,f(app,"setup.ready",&[("count",&ready.to_string())]),button("setup-defer-all","",t(app,if app.setup.rematch.is_some() { "common.cancel" } else { "setup.later" }),"","quiet",busy),button("setup-save","",t(app,if app.setup.rows.iter().any(|row| row.saving || row.queued) {"setup.saving"}else{"setup.save"}),"check","primary",busy||ready==0)).unwrap();
}

// 仅传递行数，由 DOM 桥接通过 CSSOM 设置高度，避免桌面 CSP 拦截 style 属性更新。
fn render_spacer(html: &mut String, start: usize, end: usize) {
    if start < end {
        write!(html, r#"<tr class="setup-spacer" data-render-key="setup-spacer-{start}" aria-hidden="true"><td colspan="4" data-setup-spacer-rows="{}"></td></tr>"#, end - start).unwrap();
    }
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
    fn automatic_scraping_searches_current_text_and_confirms_single_result_before_save() {
        let mut app = app();
        let query = "8  援女交際 ～亜梨紗の情事～";
        for row in &mut app.setup.rows { row.query = query.into(); }
        crate::bridge::take_requests();
        dispatch(&mut app, "setup-auto", "");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 2);
        for (request_id, command, payload) in &requests {
            assert_eq!(command, "search_metadata");
            assert_eq!(serde_json::from_str::<serde_json::Value>(payload).unwrap()["query"], query);
            let kind = app.requests.complete(*request_id).unwrap();
            resolve(
                &mut app,
                &kind,
                *request_id,
                true,
                r#"[{"title":"中文译名","url":"https://vndb.org/v2"}]"#,
            );
        }
        assert!(app.setup.rows.iter().all(|row| row.query == query));
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
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].1, "complete_game_setup_batch");
        let payload: serde_json::Value = serde_json::from_str(&saves[0].2).unwrap();
        let inputs = payload["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 2);
        for input in inputs {
            assert_eq!(input["url"], "https://vndb.org/v2");
            assert_eq!(input["confirmed"], true);
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
            resolve(&mut app, &kind, *request_id, true, if index == 0 { r#"[{"title":"First","url":"https://vndb.org/v1"}]"# } else { "[]" });
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
    fn automatic_scraping_skips_empty_queries_and_preselects_multiple_results() {
        let mut app = app();
        app.setup.rows[0].query = "人妻俱乐部".into();
        app.setup.rows[1].query.clear();
        crate::bridge::take_requests();
        dispatch(&mut app, "setup-auto", "");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        let (request_id, _, payload) = &requests[0];
        assert_eq!(serde_json::from_str::<serde_json::Value>(payload).unwrap()["query"], "人妻俱乐部");
        let kind = app.requests.complete(*request_id).unwrap();
        resolve(&mut app, &kind, *request_id, true,
            r#"[{"title":"人妻游泳俱乐部","url":"https://vndb.org/v1"},{"title":"人妻俱乐部","url":"https://vndb.org/v2"}]"#);
        assert_eq!(app.setup.rows[0].url, "https://vndb.org/v1");
        assert!(!app.setup.rows[0].confirmed);
        assert!(!app.setup.rows[0].can_save());
        assert!(app.setup.rows[1].searched);
        assert!(!app.setup.rows[1].confirmed);
        assert!(!app.setup.busy());
        dispatch(&mut app, "setup-save", "");
        assert!(crate::bridge::take_requests().is_empty());
        dispatch(&mut app, "setup-confirm-launcher", "new");
        dispatch(&mut app, "setup-save", "");
        assert_eq!(crate::bridge::take_requests().len(), 1);
    }

    #[test]
    fn automatic_scraping_confirms_multiple_results_only_when_first_title_equals_query() {
        for (query, first_title, url, confirmed) in [
            ("白色相簿", "白色相簿", "https://vndb.org/v1", true),
            ("白色相簿2", "白色相簿2", "https://vndb.org/v1", true),
            ("白色相簿2", "白色相簿", "https://vndb.org/v1", false),
            ("8  白色相簿", "白色相簿", "https://vndb.org/v1", false),
            ("白色相簿 2", "白色相簿2", "https://vndb.org/v1", false),
            ("白色相簿", "白色相簿", "", false),
        ] {
            let mut app = app();
            app.setup.rows.truncate(1);
            input(&mut app, "setup-query:new", query);
            crate::bridge::take_requests();
            dispatch(&mut app, "setup-auto", "");
            let requests = crate::bridge::take_requests();
            let request_id = requests[0].0;
            let kind = app.requests.complete(request_id).unwrap();
            resolve(
                &mut app,
                &kind,
                request_id,
                true,
                &json!([
                    {"title": first_title, "url": url},
                    {"title": query, "url": "https://vndb.org/v2"},
                    {"title": "Other", "url": "https://vndb.org/v3"}
                ])
                .to_string(),
            );
            let row = &app.setup.rows[0];
            assert_eq!(row.query, query);
            assert_eq!(row.url, url);
            assert_eq!(row.candidates.len(), 3);
            assert_eq!(row.confirmed, confirmed, "query={query}, first={first_title}");
            assert_eq!(row.can_save(), confirmed);
            assert!(crate::bridge::take_requests().is_empty());
        }
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
        assert_eq!(requests.len(), 1);
        let request_id = requests[0].0;
        let kind = app.requests.complete(request_id).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&saved_response("imported")).unwrap();
        let response = json!({"items":[
            {"id":"imported", "result":{"Ok":saved}},
            {"id":"new", "result":{"Err":{"key":"error.network","args":{}}}}
        ], "warning":null});
        resolve(&mut app, &kind, request_id, true, &response.to_string());
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
    fn batch_save_submits_every_row_and_accepts_out_of_order_results() {
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
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        let (request_id, command, payload) = &requests[0];
        assert_eq!(command, "complete_game_setup_batch");
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
        let inputs = payload["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 5);
        assert!(app.setup.rows.iter().all(|row| row.saving));
        let items: Vec<_> = inputs.iter().rev().map(|input| {
            let id = input["id"].as_str().unwrap();
            let saved: serde_json::Value = serde_json::from_str(&saved_response(id)).unwrap();
            json!({"id":id,"result":{"Ok":saved}})
        }).collect();
        let kind = app.requests.complete(*request_id).unwrap();
        resolve(&mut app, &kind, *request_id, true, &json!({"items":items,"warning":null}).to_string());
        sync(&mut app);
        assert!(app.setup.rows.is_empty());
        assert!(crate::bridge::take_requests().is_empty());
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
