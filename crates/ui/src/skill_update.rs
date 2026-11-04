//! Skill 卡片、启动版本检查、手动同步及本地可用状态。
use crate::{
    models::{InstalledSkill, SkillRuntimeStatus, SkillUpdateResult},
    requests::{Pending, issue},
    state::{App, show_toast},
    view::t,
    widgets::{button, esc},
};
use galgear_i18n::Message;
use serde_json::json;
use std::fmt::Write;

const REPOSITORY: &str = "https://github.com/CbShuiMu/Galgear-GalgameReverse-Skill";
#[derive(Default)]
pub(crate) struct State {
    local: InstalledSkill,
    applied_request: u32,
    installed_request: u32,
    local_loaded: bool,
    latest_commit: Option<String>,
    check_error: Option<Message>,
}

pub(crate) fn check_on_startup(app: &mut App) {
    issue(app, Pending::SkillCheck, "check_skill_version", json!({}));
}

pub(crate) fn update_available(app: &App) -> bool {
    let state = &app.skill_update;
    state.local_loaded
        && state.latest_commit.as_ref().is_some_and(|latest| {
            !state.local.installed || state.local.commit.as_ref() != Some(latest)
        })
}

fn checking(app: &App) -> bool {
    app.requests
        .values()
        .any(|pending| matches!(pending, Pending::SkillUpdate))
}
pub(crate) fn dispatch(app: &mut App, action: &str) -> bool {
    if action != "check-skill-update" {
        return false;
    }
    if !checking(app) {
        issue(app, Pending::SkillUpdate, "check_skill_update", json!({}));
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
    if !matches!(
        kind,
        Pending::AiRuntime | Pending::SkillUpdate | Pending::SkillCheck
    ) {
        return false;
    }
    let applied = if matches!(kind, Pending::SkillCheck) {
        app.skill_update.installed_request
    } else {
        app.skill_update.applied_request
    };
    if id < applied {
        return true;
    }
    if !success {
        if matches!(kind, Pending::SkillCheck) {
            app.skill_update.check_error = Some(Message::from_error(payload));
        } else {
            show_toast(app, Message::from_error(payload), true);
        }
        return true;
    }
    // 远端查询与本地查询可以乱序完成；它不推进本地状态的请求代数。
    if matches!(kind, Pending::SkillCheck) {
        match serde_json::from_str(payload) {
            Ok(commit) => {
                app.skill_update.latest_commit = Some(commit);
                app.skill_update.check_error = None;
            }
            Err(error) => {
                app.skill_update.check_error = Some(Message::detail("error.read_data", error))
            }
        }
        return true;
    }
    let result = if matches!(kind, Pending::AiRuntime) {
        serde_json::from_str::<SkillRuntimeStatus>(payload).map(|status| {
            app.ai_available = status.available;
            app.skill_update.local = status.skill;
            app.skill_update.local_loaded = true;
            if let Some(error) = status.error {
                show_toast(app, error, true);
            }
        })
    } else {
        serde_json::from_str::<SkillUpdateResult>(payload).map(|update| {
            app.ai_available = update.available;
            app.skill_update.latest_commit = Some(update.commit.clone());
            app.skill_update.installed_request = id;
            app.skill_update.local_loaded = true;
            app.skill_update.check_error = None;
            app.skill_update.local = InstalledSkill {
                installed: true,
                commit: Some(update.commit),
            };
            show_toast(
                app,
                Message::new(if update.changed {
                    "skills.updated"
                } else {
                    "skills.current"
                }),
                false,
            );
        })
    };
    match result {
        Ok(()) => app.skill_update.applied_request = id,
        Err(error) => show_toast(app, Message::detail("error.read_data", error), true),
    }
    true
}

pub(crate) fn render(html: &mut String, app: &App) {
    let local = &app.skill_update.local;
    let version = if !local.installed {
        t(app, "skills.not_installed").into()
    } else if let Some(commit) = &local.commit {
        esc(&commit.chars().take(8).collect::<String>())
    } else {
        t(app, "skills.local_version").into()
    };
    let pending = checking(app);
    write!(html, r#"<section class="settings-card" data-render-key="skill-update"><div class="panel-heading"><h2>{}</h2><div class="update-badges">{}<span class="version-pill">{}</span></div></div><label class="edit-field" for="skill-repository"><span>{}</span><input id="skill-repository" value="{}" readonly/></label><div class="detail-actions">{}</div><p class="muted" role="status">{}</p></section>"#,
        t(app,"skills.title"), crate::widgets::pending_update_badge(t(app, "update.pending"), update_available(app)), version, t(app,"skills.repository"), REPOSITORY,
        button("check-skill-update", "", t(app,"settings.update"), "update", "quiet", pending || app.update.installing),
        if pending { t(app, "skills.updating").into() }
        else if let Some(error) = &app.skill_update.check_error { esc(&error.render(app.settings.language)) }
        else { t(app,"skills.help").into() }).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_and_local_status_can_arrive_in_either_order_and_count_each_component_once() {
        for remote_first in [false, true] {
            let mut app = App::default();
            let local = r#"{"available":true,"skill":{"installed":true,"commit":"aaaaaaaa"}}"#;
            if remote_first {
                resolve(&mut app, &Pending::SkillCheck, 1, true, "\"bbbbbbbb\"");
                assert!(!update_available(&app));
            }
            resolve(&mut app, &Pending::AiRuntime, 2, true, local);
            if !remote_first {
                resolve(&mut app, &Pending::SkillCheck, 1, true, "\"bbbbbbbb\"");
            }
            assert_eq!(crate::updates::pending_count(&app), 1);
            crate::updates::check_on_startup(&mut app);
            crate::updates::resolve(
                &mut app,
                &Pending::BackgroundUpdate,
                true,
                r#"{"version":"v9.0.0","available":true,"asset":{}}"#,
            );
            assert_eq!(crate::updates::pending_count(&app), 2);
            resolve(&mut app, &Pending::SkillUpdate, 5, false, "offline");
            assert_eq!(crate::updates::pending_count(&app), 2);
            resolve(
                &mut app,
                &Pending::SkillUpdate,
                6,
                true,
                r#"{"available":true,"commit":"cccccccc","changed":true}"#,
            );
            assert_eq!(crate::updates::pending_count(&app), 1);
            resolve(&mut app, &Pending::SkillCheck, 1, true, "\"bbbbbbbb\"");
            assert!(!update_available(&app));
        }
    }

    #[test]
    fn background_failure_is_quiet_and_does_not_change_local_availability() {
        let mut app = App::default();
        resolve(
            &mut app,
            &Pending::AiRuntime,
            2,
            true,
            r#"{"available":true,"skill":{"installed":true,"commit":"aaaaaaaa"}}"#,
        );
        resolve(&mut app, &Pending::SkillCheck, 1, false, "offline fixture");
        assert!(app.ai_available);
        assert!(app.toast.is_none());
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("offline fixture"));
        assert!(!update_available(&app));
    }

    #[test]
    fn local_status_does_not_show_network_update_activity() {
        let mut app = App::default();
        issue(&mut app, Pending::AiRuntime, "get_ai_runtime", json!({}));
        assert!(crate::view::network_activities(&app).is_empty());
        dispatch(&mut app, "check-skill-update");
        assert_eq!(
            crate::view::network_activities(&app),
            ["正在检查并更新 Skill…"]
        );
    }

    #[test]
    fn only_explicit_click_updates_and_duplicate_clicks_are_ignored() {
        let mut app = App::default();
        crate::bridge::take_requests();
        resolve(
            &mut app,
            &Pending::AiRuntime,
            1,
            true,
            r#"{"available":false,"skill":{"installed":false}}"#,
        );
        let mut html = String::new();
        render(&mut html, &app);
        assert!(html.contains("Skill"));
        assert!(html.contains(REPOSITORY));
        assert!(crate::bridge::take_requests().is_empty());
        dispatch(&mut app, "check-skill-update");
        dispatch(&mut app, "check-skill-update");
        let requests = crate::bridge::take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "check_skill_update");
        assert!(!app.busy());
    }

    #[test]
    fn failure_keeps_local_version_and_late_startup_cannot_revert_success() {
        let mut app = App::default();
        resolve(
            &mut app,
            &Pending::SkillUpdate,
            8,
            true,
            r#"{"commit":"aaaaaaaa","changed":true,"available":true}"#,
        );
        resolve(
            &mut app,
            &Pending::AiRuntime,
            2,
            true,
            r#"{"available":false}"#,
        );
        assert!(app.ai_available);
        resolve(
            &mut app,
            &Pending::SkillUpdate,
            9,
            false,
            r#"{"key":"skills.update_failed"}"#,
        );
        assert!(app.ai_available);
        assert_eq!(app.skill_update.local.commit.as_deref(), Some("aaaaaaaa"));
        assert!(app.toast.as_ref().unwrap().1);
    }
}
