//! Skill 卡片、手动同步及本地可用状态。
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
    if !matches!(kind, Pending::AiRuntime | Pending::SkillUpdate) {
        return false;
    }
    if id < app.skill_update.applied_request {
        return true;
    }
    if !success {
        show_toast(app, Message::from_error(payload), true);
        return true;
    }
    let result = if matches!(kind, Pending::AiRuntime) {
        serde_json::from_str::<SkillRuntimeStatus>(payload).map(|status| {
            app.ai_available = status.available;
            app.skill_update.local = status.skill;
            if let Some(error) = status.error {
                show_toast(app, error, true);
            }
        })
    } else {
        serde_json::from_str::<SkillUpdateResult>(payload).map(|update| {
            app.ai_available = update.available;
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
    write!(html, r#"<section class="settings-card" data-render-key="skill-update"><div class="panel-heading"><h2>{}</h2><span class="version-pill">{}</span></div><label class="edit-field" for="skill-repository"><span>{}</span><input id="skill-repository" value="{}" readonly/></label><div class="detail-actions">{}</div><p class="muted" role="status">{}</p></section>"#,
        t(app,"skills.title"), version, t(app,"skills.repository"), REPOSITORY,
        button("check-skill-update", "", t(app,"settings.check_update"), "update", "quiet", pending || app.update.installing),
        t(app, if pending { "skills.updating" } else { "skills.help" })).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

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
