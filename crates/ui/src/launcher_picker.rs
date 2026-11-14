//! 共用启动文件选择器；首次启动确认后继续启动，配置修改仅保存。
use crate::{
    features,
    models::{Game, Settings},
    requests::{Pending, issue},
    state::App,
    view::t,
    widgets::{button, esc, icon},
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State {
    id: Option<String>,
    launch_action: Option<String>,
    candidates: Vec<Candidate>,
    selected: String,
    request: Option<u32>,
    error: Option<Message>,
}

#[derive(Deserialize)]
struct Candidate {
    path: String,
    name: String,
    icon: Option<String>,
}

impl State {
    fn can_save(&self) -> bool {
        self.request.is_none()
            && !self.selected.is_empty()
            && self
                .candidates
                .iter()
                .any(|item| item.path == self.selected)
    }
}

pub(crate) fn open(app: &mut App, id: &str, action: &str) -> bool {
    if !app
        .settings
        .game_edits
        .get(id)
        .is_some_and(|edit| edit.launcher_pending)
        && app
            .catalog
            .games
            .iter()
            .find(|game| game.id == id)
            .is_none_or(|game| game.executable.is_some())
    {
        return false;
    }
    begin(app, id, Some(action));
    true
}

fn begin(app: &mut App, id: &str, launch_action: Option<&str>) {
    let Some(game) = app.catalog.games.iter().find(|game| game.id == id) else {
        return;
    };
    if !game.available || app.launcher_picker.id.is_some() {
        return;
    }
    app.launcher_picker = State {
        id: Some(id.into()),
        launch_action: launch_action.map(str::to_owned),
        selected: game.executable.clone().unwrap_or_default(),
        request: Some(app.requests.next_id()),
        ..Default::default()
    };
    issue(
        app,
        Pending::LauncherList,
        "list_game_executables",
        json!({"id":id}),
    );
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !action.starts_with("launcher-") {
        return false;
    }
    if action == "launcher-edit" {
        if !app.detail.editing && app.detail.selected.as_deref() == Some(value) {
            begin(app, value, None);
        }
        return true;
    }
    let picker = &mut app.launcher_picker;
    match action {
        "launcher-close" if picker.request.is_none() => *picker = State::default(),
        "launcher-select"
            if picker.request.is_none()
                && picker.candidates.iter().any(|item| item.path == value) =>
        {
            picker.selected = value.into()
        }
        "launcher-save" => save(app),
        _ => {}
    }
    true
}

fn save(app: &mut App) {
    let picker = &mut app.launcher_picker;
    if !picker.can_save() {
        return;
    }
    let payload = json!({"id":picker.id,"executable":picker.selected});
    picker.error = None;
    picker.request = Some(app.requests.next_id());
    issue(
        app,
        Pending::LauncherSave,
        "confirm_game_executable",
        payload,
    );
}

pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    request: u32,
    success: bool,
    payload: &str,
) -> bool {
    if !matches!(kind, Pending::LauncherList | Pending::LauncherSave) {
        return false;
    }
    if app.launcher_picker.request != Some(request) {
        return true;
    }
    app.launcher_picker.request = None;
    if !success {
        if matches!(kind, Pending::LauncherList) {
            app.launcher_picker.selected.clear();
        }
        app.launcher_picker.error = Some(Message::from_error(payload));
        return true;
    }
    let result = (|| -> Result<(), serde_json::Error> {
        if matches!(kind, Pending::LauncherList) {
            app.launcher_picker.candidates = serde_json::from_str(payload)?;
            if !app
                .launcher_picker
                .candidates
                .iter()
                .any(|candidate| candidate.path == app.launcher_picker.selected)
            {
                app.launcher_picker.selected.clear();
            }
            if let [candidate] = app.launcher_picker.candidates.as_slice() {
                app.launcher_picker.selected = candidate.path.clone();
                if app.launcher_picker.launch_action.is_some() {
                    save(app);
                }
            }
        } else {
            #[derive(Deserialize)]
            struct Saved {
                game: Game,
                settings: Settings,
                runtime: Option<String>,
            }
            let mut saved: Saved = serde_json::from_str(payload)?;
            if app.launcher_picker.id.as_deref() != Some(&saved.game.id) {
                return Ok(());
            }
            let id = saved.game.id.clone();
            if let Some(game) = app.catalog.games.iter_mut().find(|game| game.id == id) {
                *game = saved.game;
            }
            if let Some(edit) = saved.settings.game_edits.remove(&id) {
                app.settings.game_edits.insert(id.clone(), edit);
            }
            if let Some(runtime) = saved.runtime {
                app.runtime.insert(id.clone(), runtime);
            }
            let action = app.launcher_picker.launch_action.take();
            app.launcher_picker = State::default();
            if let Some(action) = action {
                features::dispatch(app, &action, &id);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        app.launcher_picker.error = Some(Message::detail("error.read_data", error));
    }
    true
}

pub(crate) fn render(html: &mut String, app: &App) {
    let picker = &app.launcher_picker;
    let busy = picker.request.is_some();
    // 首次启动的单候选仍自动确认；主动修改时始终显示弹窗并等待确认。
    if picker.id.is_none()
        || (picker.launch_action.is_some() && busy && picker.candidates.len() <= 1)
    {
        return;
    }
    write!(html, r#"<div class="library-overlay"><section class="library-dialog launcher-dialog" role="dialog" aria-modal="true" aria-labelledby="launcher-title" tabindex="-1"><header><h2 id="launcher-title">{}</h2></header><div class="launcher-list">"#, t(app, "launcher.title")).unwrap();
    for item in &picker.candidates {
        write!(html, r#"<button class="launcher-option" data-action="launcher-select" data-value="{}" aria-pressed="{}" title="{}" {}>"#, esc(&item.path), picker.selected == item.path, esc(&item.path), if busy { "disabled" } else { "" }).unwrap();
        if let Some(image) = &item.icon {
            write!(html, r#"<img src="{}" alt=""/>"#, esc(image)).unwrap();
        } else {
            html.push_str(&icon("games"));
        }
        write!(html, "<span>{}</span></button>", esc(&item.name)).unwrap();
    }
    if picker.candidates.is_empty() && picker.error.is_none() {
        write!(
            html,
            "<p role=\"status\">{}</p>",
            t(
                app,
                if busy {
                    "launcher.loading"
                } else {
                    "launcher.empty"
                }
            )
        )
        .unwrap();
    }
    if let Some(error) = &picker.error {
        write!(
            html,
            "<p class=\"setup-error\" role=\"alert\">{}</p>",
            esc(&error.render(app.settings.language))
        )
        .unwrap();
    }
    write!(
        html,
        "</div><footer class=\"library-dialog-footer\">{}{}</footer></section></div>",
        button(
            "launcher-close",
            "",
            t(app, "common.cancel"),
            "close",
            "quiet",
            busy
        ),
        button(
            "launcher-save",
            "",
            t(app, "common.confirm"),
            "check",
            "primary",
            !picker.can_save()
        )
    )
    .unwrap();
}
