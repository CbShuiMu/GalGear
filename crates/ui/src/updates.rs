use crate::{state::{App, show_toast}, requests::{Pending, issue}, view::{t, f}, widgets::button};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State { pub release: Option<Release>, pub installing: bool }
#[derive(Deserialize)]
pub(crate) struct Release { version: String, available: bool, asset: Option<serde_json::Value> }

pub(crate) fn checking(app: &App) -> bool { app.requests.values().any(|pending| matches!(pending, Pending::Update)) }
pub(crate) fn dispatch(app: &mut App, action: &str) -> bool {
    if app.update.installing { return true; }
    match action {
        "check-update" => {
            if !checking(app) { issue(app, Pending::Update, "check_update", json!({})); }
        }
        "update-cancel" => app.update.release = None,
        "update-confirm" => {
            if let Some(release) = &app.update.release && release.available && release.asset.is_some() {
                let version = release.version.clone();
                app.update.installing = true;
                issue(app, Pending::InstallUpdate, "install_update", json!({"version":version}));
            }
        }
        _ => return false,
    }
    true
}

pub(crate) fn resolve(app: &mut App, kind: &Pending, success: bool, payload: &str) -> bool {
    if !matches!(kind, Pending::Update | Pending::InstallUpdate) { return false; }
    if !success {
        app.update.installing = false;
        let error = Message::from_error(payload);
        let detail = error.args.get("detail").cloned().unwrap_or_else(|| error.render(app.settings.language));
        show_toast(app, Message::detail("update.failed", detail), true);
    } else if matches!(kind, Pending::Update) {
        match serde_json::from_str(payload) {
            Ok(release) => app.update.release = Some(release),
            Err(error) => show_toast(app, Message::detail("update.failed", error), true),
        }
    }
    true
}

pub(crate) fn render(html: &mut String, app: &App) {
    let Some(release) = &app.update.release else { return };
    write!(html, r#"<div class="editor-overlay update-overlay"><section class="editor-dialog update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-title"><header><h2 id="update-title">{}</h2></header><div class="editor-body"><p>{}</p><p>{}</p><div class="detail-actions">"#,
        t(app,"settings.check_update"), f(app,"settings.latest",&[("version",&release.version)]),
        t(app,if app.update.installing {"update.stopping"} else if !release.available {"update.current"} else if release.asset.is_none() {"update.unavailable"} else {"update.confirm"})).unwrap();
    if release.available && release.asset.is_some() {
        html.push_str(&button("update-confirm","",t(app,"update.install"),"update","primary",app.update.installing));
    }
    html.push_str(&button("update-cancel","",t(app,"common.close"),"","quiet",app.update.installing));
    html.push_str("</div></div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_confirmation_is_explicit_and_has_priority_over_background_work() {
        let mut app = App::default();
        crate::bridge::take_requests();
        issue(&mut app, Pending::Cover, "test", json!({}));
        crate::bridge::take_requests();
        dispatch(&mut app,"check-update");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(),1);
        let kind = app.requests.complete(requests[0].0).unwrap();
        resolve(&mut app,&kind,true,r#"{"version":"v0.3.0","available":true,"asset":{"name":"GalGear.exe"}}"#);
        assert!(crate::bridge::take_requests().is_empty());
        let mut html = String::new();
        render(&mut html,&app);
        assert!(html.contains("v0.3.0"));
        dispatch(&mut app,"update-confirm");
        dispatch(&mut app,"update-confirm");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(),1);
        assert_eq!(requests[0].1,"install_update");
        resolve(&mut app,&Pending::InstallUpdate,false,r#"{"key":"error.operation"}"#);
        assert!(!app.update.installing);
        assert!(app.toast.as_ref().unwrap().1);
    }
}
