use crate::{
    requests::{Pending, issue},
    state::{App, show_toast},
    view::{f, t},
    widgets::{button, esc, icon_button},
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State {
    pub release: Option<Release>,
    pub installing: bool,
    dialog_open: bool,
    source: String,
    check_error: Option<Message>,
}
#[derive(Deserialize)]
pub(crate) struct Release {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    available: bool,
    asset: Option<serde_json::Value>,
}

pub(crate) fn checking(app: &App) -> bool {
    app.requests
        .values()
        .any(|pending| matches!(pending, Pending::Update | Pending::BackgroundUpdate))
}

pub(crate) fn check_on_startup(app: &mut App) {
    app.update.source.clone_from(&app.settings.release_page);
    issue(app, Pending::BackgroundUpdate, "check_update", json!({}));
}

pub(crate) fn update_available(app: &App) -> bool {
    app.update.source == app.settings.release_page
        && app
            .update
            .release
            .as_ref()
            .is_some_and(|release| release.available)
}

pub(crate) fn pending_count(app: &App) -> usize {
    usize::from(update_available(app)) + usize::from(crate::skill_update::update_available(app))
}

pub(crate) fn render_check_error(html: &mut String, app: &App) {
    if app.update.source == app.settings.release_page
        && let Some(error) = &app.update.check_error
    {
        let _ = write!(
            html,
            r#"<p class="muted" role="status">{}</p>"#,
            esc(&error.render(app.settings.language))
        );
    }
}
pub(crate) fn dispatch(app: &mut App, action: &str) -> bool {
    if app.update.installing {
        return true;
    }
    match action {
        "check-update" => {
            if !checking(app) {
                if app.update.source != app.settings.release_page {
                    app.update.release = None;
                }
                app.update.source.clone_from(&app.settings.release_page);
                app.update.check_error = None;
                issue(app, Pending::Update, "check_update", json!({}));
            }
        }
        "update-cancel" => app.update.dialog_open = false,
        "update-confirm" => {
            if app.update.dialog_open
                && app.update.source == app.settings.release_page
                && let Some(release) = &app.update.release
                && release.available
                && release.asset.is_some()
            {
                let version = release.version.clone();
                app.update.installing = true;
                issue(
                    app,
                    Pending::InstallUpdate,
                    "install_update",
                    json!({"version":version}),
                );
            }
        }
        _ => return false,
    }
    true
}

pub(crate) fn resolve(app: &mut App, kind: &Pending, success: bool, payload: &str) -> bool {
    if !matches!(
        kind,
        Pending::Update | Pending::BackgroundUpdate | Pending::InstallUpdate
    ) {
        return false;
    }
    if !matches!(kind, Pending::InstallUpdate) && app.update.source != app.settings.release_page {
        return true;
    }
    let background = matches!(kind, Pending::BackgroundUpdate);
    if !success {
        app.update.installing = false;
        let error = Message::from_error(payload);
        let detail = error
            .args
            .get("detail")
            .cloned()
            .unwrap_or_else(|| error.render(app.settings.language));
        let error = Message::detail("update.failed", detail);
        app.update.check_error = Some(error.clone());
        if !background {
            show_toast(app, error, true);
        }
    } else if matches!(kind, Pending::Update | Pending::BackgroundUpdate) {
        match serde_json::from_str::<Release>(payload) {
            Ok(release) => {
                app.update.dialog_open = !background || release.available;
                app.update.release = Some(release);
                app.update.check_error = None;
            }
            Err(error) => {
                let error = Message::detail("update.failed", error);
                app.update.check_error = Some(error.clone());
                if !background {
                    show_toast(app, error, true);
                }
            }
        }
    }
    true
}

pub(crate) fn render(html: &mut String, app: &App) {
    if !app.update.dialog_open || app.update.source != app.settings.release_page {
        return;
    }
    let Some(release) = &app.update.release else {
        return;
    };
    write!(html, r#"<div class="editor-overlay update-overlay"><section class="editor-dialog update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-title"><header><h2 id="update-title">{}</h2>{}</header><div class="editor-body" tabindex="0" role="region" aria-labelledby="update-title"><p>{}</p>"#,
        t(app,"settings.update"), icon_button("update-cancel", "", t(app,"common.close"), "close", "", app.update.installing), f(app,"settings.latest",&[("version",&release.version)])).unwrap();
    if release.available {
        let notes = release
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|notes| !notes.is_empty());
        // 发布正文按文本展示，保留换行，避免把外部 HTML 当作界面执行。
        write!(html, r#"<section class="update-notes" aria-labelledby="update-notes-title"><h3 id="update-notes-title">{}</h3><div class="update-notes-content">{}</div></section>"#,
            t(app, "update.notes"), esc(notes.unwrap_or_else(|| t(app, "update.notes_empty")))).unwrap();
    }
    write!(
        html,
        r#"<p>{}</p></div><footer class="dialog-actions">"#,
        t(
            app,
            if app.update.installing {
                "update.stopping"
            } else if !release.available {
                "update.current"
            } else if release.asset.is_none() {
                "update.unavailable"
            } else {
                "update.confirm"
            }
        )
    )
    .unwrap();
    html.push_str(&button(
        "update-cancel",
        "",
        t(
            app,
            if release.available {
                "update.later"
            } else {
                "common.close"
            },
        ),
        "",
        "quiet",
        app.update.installing,
    ));
    if release.available {
        html.push_str(&button(
            "update-confirm",
            "",
            t(app, "update.install"),
            "update",
            "primary",
            app.update.installing || release.asset.is_none(),
        ));
    }
    html.push_str("</footer></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_dialog_shows_notes_from_background_and_manual_checks_as_text() {
        for kind in [Pending::BackgroundUpdate, Pending::Update] {
            let mut app = App::default();
            let payload = json!({
                "version": "v9.0.0", "available": true, "asset": {},
                "notes": "## 新增\n- 新功能\n\n## 修复\n<script>alert('x')</script> & 完成"
            });
            resolve(&mut app, &kind, true, &payload.to_string());
            let mut html = String::new();
            render(&mut html, &app);
            assert!(html.contains("## 新增\n- 新功能\n\n## 修复\n"));
            assert!(html.contains("&lt;script&gt;"));
            assert!(html.contains("&amp; 完成"));
            assert!(!html.contains("<script>"));
            assert!(html.find("update-notes-title").unwrap() < html.find("<footer").unwrap());
        }
    }

    #[test]
    fn missing_or_blank_notes_show_a_localized_fallback_without_blocking_updates() {
        use galgear_i18n::Language;
        for language in [Language::Chinese, Language::Japanese, Language::English] {
            for notes in [
                None,
                Some(serde_json::Value::Null),
                Some(json!("")),
                Some(json!(" \r\n\t")),
            ] {
                let mut app = App::default();
                app.settings.language = language;
                let mut payload = json!({"version":"v9.0.0","available":true,"asset":{}});
                if let Some(notes) = notes {
                    payload["notes"] = notes;
                }
                resolve(&mut app, &Pending::Update, true, &payload.to_string());
                let mut html = String::new();
                render(&mut html, &app);
                assert!(html.contains(t(&app, "update.notes")));
                assert!(html.contains(t(&app, "update.notes_empty")));
                assert!(html.contains("data-action=\"update-confirm\""));
            }
        }
    }

    #[test]
    fn startup_prompts_for_an_update_and_deferring_keeps_the_notice() {
        let mut app = App::default();
        crate::bridge::take_requests();
        check_on_startup(&mut app);
        assert!(!app.busy());
        assert!(crate::view::network_activities(&app).is_empty());
        let request = crate::bridge::take_requests().remove(0);
        let kind = app.requests.complete(request.0).unwrap();
        resolve(
            &mut app,
            &kind,
            true,
            r#"{"version":"v9.0.0","available":true,"asset":{}}"#,
        );
        assert_eq!(pending_count(&app), 1);
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("update-overlay"));
        assert!(html.contains("v9.0.0"));
        let footer = html.split("<footer").nth(1).unwrap();
        assert!(footer.find("稍后处理").unwrap() < footer.find("立即更新").unwrap());
        assert!(crate::bridge::take_requests().is_empty());
        dispatch(&mut app, "update-cancel");
        html.clear();
        render(&mut html, &app);
        assert!(html.is_empty());
        dispatch(&mut app, "update-confirm");
        assert!(crate::bridge::take_requests().is_empty());
        assert_eq!(pending_count(&app), 1);
        assert!(app.toast.is_none());
    }

    #[test]
    fn startup_stays_quiet_when_no_new_version_is_available() {
        let mut app = App::default();
        check_on_startup(&mut app);
        resolve(
            &mut app,
            &Pending::BackgroundUpdate,
            true,
            r#"{"version":"v0.3.0","available":false,"asset":null}"#,
        );
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.is_empty());
        assert_eq!(pending_count(&app), 0);
        assert!(app.toast.is_none());
    }

    #[test]
    fn startup_confirmation_installs_only_once_after_a_click() {
        let mut app = App::default();
        check_on_startup(&mut app);
        crate::bridge::take_requests();
        resolve(
            &mut app,
            &Pending::BackgroundUpdate,
            true,
            r#"{"version":"v9.0.0","available":true,"asset":{}}"#,
        );
        assert!(crate::bridge::take_requests().is_empty());
        dispatch(&mut app, "update-confirm");
        dispatch(&mut app, "update-confirm");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "install_update");
        assert_eq!(requests[0].2, json!({"version":"v9.0.0"}).to_string());
    }

    #[test]
    fn missing_asset_shows_the_notice_but_disables_installation() {
        let mut app = App::default();
        check_on_startup(&mut app);
        crate::bridge::take_requests();
        resolve(
            &mut app,
            &Pending::BackgroundUpdate,
            true,
            r#"{"version":"v9.0.0","available":true,"asset":null}"#,
        );
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("update-overlay"));
        assert!(html.contains(t(&app, "update.unavailable")));
        let confirm = html.split("data-action=\"update-confirm\"").nth(1).unwrap();
        assert!(confirm.split('>').next().unwrap().contains("disabled"));
        dispatch(&mut app, "update-confirm");
        assert!(crate::bridge::take_requests().is_empty());
    }

    #[test]
    fn background_failures_are_visible_in_the_card_without_a_popup() {
        let mut app = App::default();
        check_on_startup(&mut app);
        resolve(
            &mut app,
            &Pending::BackgroundUpdate,
            false,
            "offline fixture",
        );
        assert!(app.toast.is_none());
        assert_eq!(pending_count(&app), 0);
        let mut html = String::new();
        render_check_error(&mut html, &app);
        assert!(html.contains("offline fixture"));
        resolve(
            &mut app,
            &Pending::BackgroundUpdate,
            true,
            "invalid response",
        );
        assert!(app.toast.is_none());
    }

    #[test]
    fn changed_release_source_does_not_reuse_an_old_notification() {
        let mut app = App::default();
        check_on_startup(&mut app);
        app.settings.release_page = "https://github.com/other/repo/releases".into();
        resolve(
            &mut app,
            &Pending::BackgroundUpdate,
            true,
            r#"{"version":"v9.0.0","available":true,"asset":{}}"#,
        );
        assert!(!update_available(&app));
        assert!(app.update.release.is_none());
    }

    #[test]
    fn update_confirmation_is_explicit_and_has_priority_over_background_work() {
        let mut app = App::default();
        crate::bridge::take_requests();
        issue(&mut app, Pending::Cover, "test", json!({}));
        crate::bridge::take_requests();
        dispatch(&mut app, "check-update");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        let kind = app.requests.complete(requests[0].0).unwrap();
        resolve(
            &mut app,
            &kind,
            true,
            r#"{"version":"v0.3.0","available":true,"asset":{"name":"GalGear.exe"}}"#,
        );
        assert!(crate::bridge::take_requests().is_empty());
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("v0.3.0"));
        dispatch(&mut app, "update-confirm");
        dispatch(&mut app, "update-confirm");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "install_update");
        resolve(
            &mut app,
            &Pending::InstallUpdate,
            false,
            r#"{"key":"error.operation"}"#,
        );
        assert!(!app.update.installing);
        assert!(app.toast.as_ref().unwrap().1);
    }
}
