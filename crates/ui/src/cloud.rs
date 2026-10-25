//! WebDAV connection settings and explicit save archive transfers.
use crate::{
    choices, models::SaveSnapshot, requests::Pending, requests::issue, state::App, view::t,
    widgets::button, widgets::bytes, widgets::esc,
};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write;

#[derive(Clone)]
pub(crate) enum Request {
    Status,
    Save,
    Clear,
    Test,
    List(String),
    Upload(String),
    Download(String),
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
struct Settings {
    url: String,
    username: String,
    remote_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: String::new(),
            remote_path: "GalGear".into(),
        }
    }
}

#[derive(Default, Deserialize)]
struct Status {
    settings: Settings,
    password_saved: bool,
}

#[derive(Deserialize)]
struct Snapshot {
    id: String,
    name: String,
    created: u64,
    size: u64,
    note: String,
}

#[derive(Default)]
pub(crate) struct State {
    status: Status,
    draft: Settings,
    password: String,
    loaded: bool,
    error: Option<Message>,
    notice: Option<Message>,
    game: String,
    snapshots: Vec<Snapshot>,
    list_loaded: bool,
    downloaded_game: Option<String>,
}

pub(crate) fn busy(app: &App) -> bool {
    app.requests
        .values()
        .any(|pending| matches!(pending, Pending::Cloud(_)))
}

fn configured(state: &State) -> bool {
    state.status.password_saved && !state.status.settings.url.is_empty()
}

fn dirty(state: &State) -> bool {
    state.draft != state.status.settings || !state.password.is_empty()
}

pub(crate) fn load(app: &mut App) {
    if !app.cloud.loaded && !busy(app) {
        app.cloud.error = None;
        issue(
            app,
            Pending::Cloud(Request::Status),
            "cloud_status",
            json!({}),
        );
    }
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !matches!(
        action,
        "cloud-retry"
            | "cloud-save"
            | "cloud-test"
            | "cloud-clear"
            | "cloud-game"
            | "cloud-open-game"
            | "cloud-refresh"
            | "cloud-upload"
            | "cloud-download"
    ) {
        return false;
    }
    if busy(app) || app.busy() {
        return true;
    }
    if action == "cloud-open-game" {
        if app.cloud.downloaded_game.as_deref() == Some(value)
            && app.catalog.games.iter().any(|game| game.id == value)
        {
            crate::actions::dispatch(app, "game-detail", value);
        }
        return true;
    }
    if action == "cloud-retry" {
        load(app);
        return true;
    }
    if action == "cloud-game" {
        if value.is_empty() || app.catalog.games.iter().any(|game| game.id == value) {
            app.cloud.game = value.into();
            app.cloud.snapshots.clear();
            app.cloud.list_loaded = false;
            app.cloud.downloaded_game = None;
            app.cloud.error = None;
            app.cloud.notice = None;
        }
        return true;
    }
    if !app.cloud.loaded {
        return true;
    }
    app.cloud.error = None;
    app.cloud.notice = None;
    match action {
        "cloud-save" | "cloud-test" => {
            let (request, command) = if action == "cloud-save" {
                (Request::Save, "save_cloud_settings")
            } else {
                (Request::Test, "test_cloud_connection")
            };
            let payload = json!({"settings":app.cloud.draft,"password":app.cloud.password});
            issue(app, Pending::Cloud(request), command, payload);
        }
        "cloud-clear" => issue(
            app,
            Pending::Cloud(Request::Clear),
            "clear_cloud_settings",
            json!({}),
        ),
        _ => {
            if !configured(&app.cloud) || dirty(&app.cloud) {
                return true;
            }
            let id = app.cloud.game.clone();
            let Some(game) = app.catalog.games.iter().find(|game| game.id == id) else {
                return true;
            };
            let (request, command, payload) = match action {
                "cloud-refresh" => (Request::List(id.clone()), "cloud_list", json!({"id":id})),
                "cloud-upload" if game.available => (
                    Request::Upload(id.clone()),
                    "cloud_upload",
                    json!({"id":id}),
                ),
                "cloud-download"
                    if app
                        .cloud
                        .snapshots
                        .iter()
                        .any(|snapshot| snapshot.id == value) =>
                {
                    (
                        Request::Download(id.clone()),
                        "cloud_download",
                        json!({"id":id,"snapshotId":value}),
                    )
                }
                _ => return true,
            };
            issue(app, Pending::Cloud(request), command, payload);
        }
    }
    true
}

pub(crate) fn input(app: &mut App, field: &str, value: &str) -> bool {
    if !matches!(
        field,
        "cloud-url" | "cloud-username" | "cloud-password" | "cloud-path"
    ) {
        return false;
    }
    if busy(app) || !app.cloud.loaded {
        return true;
    }
    match field {
        "cloud-url" => app.cloud.draft.url = value.into(),
        "cloud-username" => app.cloud.draft.username = value.into(),
        "cloud-password" => app.cloud.password = value.into(),
        "cloud-path" => app.cloud.draft.remote_path = value.into(),
        _ => return true,
    }
    app.cloud.error = None;
    app.cloud.notice = None;
    true
}

pub(crate) fn resolve(app: &mut App, request: &Request, success: bool, payload: &str) {
    let game = match request {
        Request::List(id) | Request::Upload(id) | Request::Download(id) => Some(id.as_str()),
        _ => None,
    };
    // Navigation and library scans may change the active game while a transfer finishes.
    if game.is_some_and(|id| id != app.cloud.game) {
        return;
    }
    let result = if !success {
        Err(Message::from_error(payload))
    } else {
        match request {
            Request::Status | Request::Save | Request::Clear => {
                serde_json::from_str::<Status>(payload)
                    .map_err(|_| Message::new("cloud.invalid_response"))
                    .map(|status| {
                        app.cloud.draft.clone_from(&status.settings);
                        app.cloud.status = status;
                        app.cloud.password.clear();
                        app.cloud.loaded = true;
                        app.cloud.error = None;
                        if !matches!(request, Request::Status) {
                            app.cloud.snapshots.clear();
                            app.cloud.list_loaded = false;
                            app.cloud.downloaded_game = None;
                            app.cloud.notice =
                                Some(Message::new(if matches!(request, Request::Save) {
                                    "cloud.saved"
                                } else {
                                    "cloud.cleared"
                                }));
                        }
                    })
            }
            Request::Test => {
                app.cloud.notice = Some(Message::new("cloud.test_ok"));
                Ok(())
            }
            Request::List(_) | Request::Upload(_) => serde_json::from_str::<Vec<Snapshot>>(payload)
                .map_err(|_| Message::new("cloud.invalid_response"))
                .map(|mut snapshots| {
                    if matches!(request, Request::Upload(_)) {
                        app.cloud.snapshots.retain(|existing| {
                            !snapshots.iter().any(|snapshot| snapshot.id == existing.id)
                        });
                        snapshots.append(&mut app.cloud.snapshots);
                        snapshots.sort_by(|a, b| {
                            b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id))
                        });
                    }
                    app.cloud.snapshots = snapshots;
                    app.cloud.list_loaded = true;
                    if matches!(request, Request::Upload(_)) {
                        app.cloud.notice = Some(Message::new("cloud.uploaded"));
                    }
                }),
            Request::Download(id) => serde_json::from_str::<Vec<SaveSnapshot>>(payload)
                .map_err(|_| Message::new("cloud.invalid_response"))
                .map(|snapshots| {
                    if app.detail.selected.as_deref() == Some(id) {
                        app.detail.history = snapshots;
                        app.detail.history_game = Some(id.clone());
                        app.detail.restore_preview = None;
                    }
                    app.cloud.downloaded_game = Some(id.clone());
                    app.cloud.notice = Some(Message::new("cloud.downloaded"));
                }),
        }
    };
    if let Err(error) = result {
        app.cloud.error = Some(error);
    }
}

pub(crate) fn choice_enabled(app: &App, field: &str) -> bool {
    field == "cloud-game" && !busy(app) && !app.busy()
}

pub(crate) fn choice_options(app: &App) -> Vec<choices::OptionItem> {
    let mut games: Vec<_> = app.catalog.games.iter().collect();
    games.sort_by_cached_key(|game| game.title.to_lowercase());
    let mut options = vec![choices::item("", t(app, "cloud.game_placeholder"))];
    options.extend(games.into_iter().map(|game| {
        choices::OptionItem {
            value: game.id.clone(),
            label: game.title.clone(),
            subtitle: game.folder.clone(),
            image: app
                .game_icons
                .get(&game.id)
                .cloned()
                .or_else(|| game.cover.clone()),
        }
    }));
    options
}

pub(crate) fn choice_current(app: &App) -> &str {
    &app.cloud.game
}

fn timestamp(created: u64) -> String {
    let Ok(seconds) = i64::try_from(created) else {
        return created.to_string();
    };
    let Ok(date) = time::OffsetDateTime::from_unix_timestamp(seconds) else {
        return created.to_string();
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        date.year(),
        u8::from(date.month()),
        date.day(),
        date.hour(),
        date.minute()
    )
}

pub(crate) fn render(html: &mut String, app: &App) {
    let state = &app.cloud;
    let pending = busy(app);
    let disabled = pending || app.busy() || !state.loaded;
    write!(html, r#"<div class="preference-page-heading"><h1>{}</h1><p class="field-help">{}</p></div><section class="settings-card cloud-connection" aria-busy="{pending}"><div class="panel-heading"><h2>{}</h2><span class="connection-status {}" role="status">{}</span></div><div class="edit-grid">"#,
        t(app, "settings.section_cloud"), t(app, "cloud.help"), t(app, "cloud.connection"),
        if configured(state) { "connected" } else { "" },
        t(app, if pending { "cloud.loading" } else if configured(state) { "cloud.configured" } else { "cloud.not_configured" })).unwrap();
    for (id, key, value, input_type, placeholder) in [
        (
            "cloud-url",
            "cloud.url",
            state.draft.url.as_str(),
            "url",
            "https://…",
        ),
        (
            "cloud-username",
            "cloud.username",
            state.draft.username.as_str(),
            "text",
            "",
        ),
        (
            "cloud-password",
            "cloud.password",
            state.password.as_str(),
            "password",
            t(
                app,
                if state.status.password_saved {
                    "cloud.password_saved"
                } else {
                    "cloud.password_placeholder"
                },
            ),
        ),
        (
            "cloud-path",
            "cloud.remote_path",
            state.draft.remote_path.as_str(),
            "text",
            "GalGear",
        ),
    ] {
        write!(html, r#"<label class="edit-field" for="{id}"><span>{}</span><input id="{id}" data-input="{id}" type="{input_type}" value="{}" placeholder="{}" autocomplete="off" autocapitalize="none" spellcheck="false" {} /></label>"#,
            t(app, key), esc(value), esc(placeholder), if disabled { "disabled" } else { "" }).unwrap();
    }
    write!(html, r#"</div><div class="account-actions">{}{}{}</div><p class="field-help vault-note">{}</p><details class="cloud-provider-help"><summary>{}</summary><p class="field-help">{}</p><ul><li>{}</li><li>{}</li></ul></details></section>"#,
        button("cloud-save", "", t(app, "common.save"), "save", "primary", disabled),
        button("cloud-test", "", t(app, "cloud.test"), "refresh", "secondary", disabled),
        button("cloud-clear", "", t(app, "cloud.clear"), "close", "quiet", disabled || (state.status.settings.url.is_empty() && !state.status.password_saved)),
        t(app, "cloud.vault"),
        t(app, "cloud.provider_help"), t(app, "cloud.provider_generic"), t(app, "cloud.provider_123"), t(app, "cloud.provider_jianguoyun")).unwrap();
    if !state.loaded && !pending {
        html.push_str(&button(
            "cloud-retry",
            "",
            t(app, "ai.retry"),
            "refresh",
            "secondary",
            app.busy(),
        ));
    }
    if let Some(error) = &state.error {
        write!(
            html,
            r#"<p class="account-notice cloud-notice" role="alert">{}</p>"#,
            esc(&error.render(app.settings.language))
        )
        .unwrap();
    }
    if let Some(notice) = &state.notice {
        write!(
            html,
            r#"<p class="field-help cloud-notice" role="status">{}</p>"#,
            esc(&notice.render(app.settings.language))
        )
        .unwrap();
    }
    if let Some(id) = &state.downloaded_game {
        html.push_str(&button(
            "cloud-open-game",
            id,
            t(app, "cloud.open_game"),
            "games",
            "secondary",
            app.busy() || pending,
        ));
    }
    write!(html, r#"<section class="settings-card cloud-sync" aria-busy="{pending}"><div class="panel-heading"><h2>{}</h2></div><p class="field-help">{}</p><div class="cloud-game-picker"><label for="cloud-game">{}</label>"#,
        t(app, "cloud.sync"), t(app, "cloud.same_title"), t(app, "cloud.game")).unwrap();
    choices::control(
        html,
        app,
        "cloud-game",
        t(app, "cloud.game"),
        pending || app.busy(),
    );
    let game = app.catalog.games.iter().find(|game| game.id == state.game);
    let sync_disabled = disabled || !configured(state) || dirty(state) || game.is_none();
    write!(
        html,
        r#"</div><div class="account-actions">{}{}</div>"#,
        button(
            "cloud-upload",
            "",
            t(app, "cloud.upload"),
            "save",
            "primary",
            sync_disabled || game.is_none_or(|game| !game.available)
        ),
        button(
            "cloud-refresh",
            "",
            t(app, "cloud.refresh"),
            "refresh",
            "secondary",
            sync_disabled
        )
    )
    .unwrap();
    if dirty(state) || !configured(state) {
        write!(
            html,
            r#"<p class="field-help">{}</p>"#,
            t(app, "cloud.save_first")
        )
        .unwrap();
    }
    if state.snapshots.is_empty() {
        write!(
            html,
            r#"<p class="field-help cloud-empty">{}</p>"#,
            t(
                app,
                if state.list_loaded {
                    "cloud.empty"
                } else {
                    "cloud.list_hint"
                }
            )
        )
        .unwrap();
    } else {
        html.push_str(r#"<ul class="cloud-snapshots">"#);
        for snapshot in &state.snapshots {
            let name = if snapshot.name.is_empty() {
                &snapshot.id
            } else {
                &snapshot.name
            };
            write!(html, r#"<li data-render-key="cloud-snapshot-{}"><div class="cloud-snapshot-copy"><strong>{}</strong><small>{} · {}</small>"#,
                esc(&snapshot.id), esc(name), timestamp(snapshot.created), bytes(snapshot.size)).unwrap();
            if !snapshot.note.is_empty() {
                write!(html, "<p>{}</p>", esc(&snapshot.note)).unwrap();
            }
            write!(
                html,
                "</div>{}</li>",
                button(
                    "cloud-download",
                    &snapshot.id,
                    t(app, "cloud.download"),
                    "download",
                    "secondary",
                    sync_disabled
                )
            )
            .unwrap();
        }
        html.push_str("</ul>");
    }
    html.push_str("</section>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_applies_normalized_settings_and_forgets_the_entered_password() {
        let mut app = App::default();
        app.cloud.password = "a-secret".into();
        resolve(
            &mut app,
            &Request::Save,
            true,
            r#"{"settings":{"url":"https://dav.example/","username":"user","remote_path":"GalGear"},"password_saved":true}"#,
        );
        assert!(app.cloud.password.is_empty());
        assert!(configured(&app.cloud));
        assert!(!dirty(&app.cloud));
    }

    #[test]
    fn late_list_responses_cannot_show_archives_for_another_game() {
        let mut app = App::default();
        app.cloud.game = "second".into();
        resolve(
            &mut app,
            &Request::List("first".into()),
            true,
            r#"[{"id":"old","name":"old game","created":1,"size":2,"note":""}]"#,
        );
        assert!(app.cloud.snapshots.is_empty());
        assert!(!app.cloud.list_loaded);
    }

    #[test]
    fn successful_upload_keeps_the_known_history_without_another_network_listing() {
        let mut app = App::default();
        app.cloud.game = "game".into();
        resolve(
            &mut app,
            &Request::List("game".into()),
            true,
            r#"[{"id":"old","name":"older","created":1,"size":2,"note":""}]"#,
        );
        resolve(
            &mut app,
            &Request::Upload("game".into()),
            true,
            r#"[{"id":"new","name":"newer","created":2,"size":3,"note":""}]"#,
        );
        assert_eq!(
            app.cloud
                .snapshots
                .iter()
                .map(|snapshot| snapshot.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "old"]
        );
        assert_eq!(
            app.cloud.notice.as_ref().map(|notice| notice.key.as_str()),
            Some("cloud.uploaded")
        );
    }

    #[test]
    fn downloaded_archives_never_replace_another_games_history_or_start_a_restore() {
        let mut app = App::default();
        app.cloud.game = "first".into();
        app.detail.selected = Some("second".into());
        resolve(
            &mut app,
            &Request::Download("first".into()),
            true,
            r#"[{"id":"cloud","created":1,"size":2,"note":"","locked":false,"files":[]}]"#,
        );
        assert!(app.detail.history.is_empty());
        assert!(app.detail.restore_preview.is_none());
        assert!(!app.detail.history_open);
        assert_eq!(app.cloud.downloaded_game.as_deref(), Some("first"));
    }

    #[test]
    fn remote_titles_notes_and_service_errors_are_escaped() {
        let mut app = App::default();
        app.cloud.game = "game".into();
        resolve(
            &mut app,
            &Request::List("game".into()),
            true,
            r#"[{"id":"bad\"id","name":"<script>name</script>","created":1,"size":2,"note":"<img src=x onerror=alert(1)>"}]"#,
        );
        resolve(&mut app, &Request::Test, false, "<b>service error</b>");
        let mut html = String::new();
        render(&mut html, &app);
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img src=x"));
        assert!(!html.contains("<b>service error</b>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn download_notice_routes_to_the_downloaded_games_existing_backup_history() {
        let mut app = App::default();
        app.cloud.game = "first".into();
        app.detail.selected = Some("first".into());
        resolve(
            &mut app,
            &Request::Download("first".into()),
            true,
            r#"[{"id":"cloud","created":1,"size":2,"note":"","locked":false,"files":[]}]"#,
        );
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("data-action=\"cloud-open-game\" data-value=\"first\""));
        assert_eq!(app.detail.history_game.as_deref(), Some("first"));
        assert_eq!(app.detail.history[0].id, "cloud");
        assert!(!app.detail.history_open);
    }
}
