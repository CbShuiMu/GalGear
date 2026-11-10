//! 首次语言选择与初始化后的后台任务；保存成功后才继续启动。
use crate::{
    requests::{Pending, issue},
    state::App,
    view::t,
    widgets::{button, esc},
};
use galgear_i18n::{Language, tr};
use serde_json::json;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State {
    started: bool,
}

pub(crate) fn resume(app: &mut App) {
    if app.settings.initial_setup_pending || app.startup.started {
        return;
    }
    app.startup.started = true;
    crate::updates::check_on_startup(app);
    crate::skill_update::check_on_startup(app);
    issue(app, Pending::AiRuntime, "get_ai_runtime", json!({}));
    issue(
        app,
        Pending::DataSources(String::new()),
        "get_data_sources",
        json!({}),
    );
    issue(app, Pending::FanCache, "get_fan_status_cache", json!({}));
    refresh_library(app);
}

pub(crate) fn refresh_library(app: &mut App) {
    if !app
        .requests
        .values()
        .any(|p| matches!(p, Pending::GameIcons))
    {
        issue(app, Pending::GameIcons, "load_game_icons", json!({}));
    }
    issue(app, Pending::BackgroundScan, "scan_library", json!({}));
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if !app.settings.initial_setup_pending {
        return false;
    }
    if action == "startup-language" && !app.busy() && Language::from_code(value).is_some() {
        issue(
            app,
            Pending::Language,
            "set_language",
            json!({"language":value}),
        );
    }
    // 首次选择完成前不执行底层页面动作，也不允许 Escape 绕过持久化。
    true
}

pub(crate) fn render(html: &mut String, app: &App) {
    if !app.initialized || !app.settings.initial_setup_pending {
        return;
    }
    write!(html, r#"<div class="editor-overlay startup-overlay"><section class="editor-dialog startup-dialog" role="dialog" aria-modal="true" aria-labelledby="startup-title"><header><h2 id="startup-title">{}</h2></header><div class="editor-body"><p>{}</p>"#,
        t(app,"startup.title"),t(app,"startup.help")).unwrap();
    if let Some((message, true)) = &app.toast {
        write!(
            html,
            r#"<p class="account-notice" role="alert">{}</p>"#,
            esc(&message.render(app.settings.language))
        )
        .unwrap();
    }
    html.push_str(r#"<div class="startup-languages">"#);
    for (language, label) in [
        (Language::Chinese, "settings.language_zh"),
        (Language::English, "settings.language_en"),
        (Language::Japanese, "settings.language_ja"),
    ] {
        write!(
            html,
            r#"<div lang="{}">{}<p class="field-help">{}</p></div>"#,
            language.code(),
            button(
                "startup-language",
                language.code(),
                t(app, label),
                "",
                "secondary",
                app.busy()
            ),
            esc(tr(language, "startup.default_source"))
        )
        .unwrap();
    }
    html.push_str("</div></div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::{dispatch, resume};
    use crate::{bridge::take_requests, state::App};

    #[test]
    fn first_run_waits_for_saved_language_and_starts_background_tasks_once() {
        let mut app = App::default();
        app.settings.initial_setup_pending = true;
        take_requests();
        resume(&mut app);
        assert!(take_requests().is_empty());
        assert!(dispatch(&mut app, "page", "settings"));
        dispatch(&mut app, "startup-language", "ja-JP");
        dispatch(&mut app, "startup-language", "en-US");
        let requests = take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "set_language");
        assert!(app.settings.initial_setup_pending);
        app.settings.initial_setup_pending = false;
        resume(&mut app);
        resume(&mut app);
        assert_eq!(
            take_requests()
                .iter()
                .filter(|r| r.1 == "check_update")
                .count(),
            1
        );
    }
}
