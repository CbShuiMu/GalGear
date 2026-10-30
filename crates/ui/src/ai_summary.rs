use crate::{
    models::{Game, Settings},
    requests::{Pending, issue},
    state::{App, show_toast},
    view::t,
    widgets::{button, esc},
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, fmt::Write};

#[derive(Default)]
pub(crate) struct State {
    open: Option<String>,
    result: Option<ResultData>,
    error: Option<Message>,
    completed_setup: BTreeMap<String, (u32, bool)>,
}

fn setup_completed(app: &mut App, game: &str, enabled: bool) {
    app.settings
        .game_edits
        .entry(game.into())
        .or_default()
        .ai_summary = enabled;
    // Settings requests already in flight may contain the state before setup finished.
    app.summary
        .completed_setup
        .insert(game.into(), (app.requests.last_id(), enabled));
}

pub(crate) fn preserve_completed_setup(state: &State, settings: &mut Settings, request: u32) {
    for (game, &(through_request, enabled)) in &state.completed_setup {
        if request <= through_request {
            settings
                .game_edits
                .entry(game.clone())
                .or_default()
                .ai_summary = enabled;
        }
    }
}
#[derive(Deserialize)]
struct ResultData {
    game_id: String,
    recap: String,
    current_progress: String,
    provider: String,
    model: String,
    save_file: String,
    source_url: String,
    script_language: String,
    warnings: Vec<String>,
}
pub(crate) fn busy(app: &App, id: &str) -> bool {
    app.requests
        .values()
        .any(|p| matches!(p, Pending::SummarySetup(game, _) | Pending::Summary(game) | Pending::RouteMapPrepare(game) | Pending::RouteMap(game) if game == id))
}
pub(crate) fn route_busy(app: &App, id: &str) -> bool {
    app.requests.values().any(
        |p| matches!(p, Pending::RouteMapPrepare(game) | Pending::RouteMap(game) if game == id),
    )
}
pub(crate) fn status_key(app: &App, id: &str) -> &'static str {
    if route_busy(app, id) {
        "route.generating"
    } else if app
        .requests
        .values()
        .any(|p| matches!(p, Pending::Summary(game) if game == id))
    {
        "summary.summarizing"
    } else {
        "summary.preparing"
    }
}
pub(crate) fn dispatch(app: &mut App, action: &str, _value: &str) -> bool {
    match action {
        "toggle-ai-summary" | "summary-reprepare" | "summary-run" | "route-map-open" => {
            let Some(id) = app.detail.selected.clone() else {
                return true;
            };
            if busy(app, &id) {
                return true;
            }
            let enabled = app
                .settings
                .game_edits
                .get(&id)
                .is_some_and(|e| e.ai_summary);
            if !(app.ai_available || action == "toggle-ai-summary" && enabled) {
                return true;
            }
            if action == "route-map-open" {
                if enabled {
                    issue(
                        app,
                        Pending::RouteMap(id.clone()),
                        "open_ai_route_map",
                        json!({"id":id}),
                    );
                }
            } else if action != "summary-run" {
                let enabled = action == "summary-reprepare" || !enabled;
                issue(
                    app,
                    Pending::SummarySetup(id.clone(), enabled),
                    "set_ai_summary",
                    json!({"id":id,"enabled":enabled}),
                );
            } else if enabled {
                app.summary.open = Some(id.clone());
                app.summary.result = None;
                app.summary.error = None;
                issue(
                    app,
                    Pending::Summary(id.clone()),
                    "summarize_game",
                    json!({"id":id}),
                );
            }
        }
        "summary-close" => {
            app.summary.open = None;
            app.summary.error = None;
        }
        _ => return false,
    }
    true
}
pub(crate) fn resolve(
    app: &mut App,
    kind: &Pending,
    _request: u32,
    success: bool,
    payload: &str,
) -> bool {
    match kind {
        Pending::SummarySetup(game, enabled) => {
            if success {
                match serde_json::from_str::<Settings>(payload) {
                    Ok(settings)
                        if settings
                            .game_edits
                            .get(game)
                            .is_some_and(|edit| edit.ai_summary == *enabled) =>
                    {
                        setup_completed(app, game, *enabled);
                        let message = if *enabled {
                            let title = app
                                .catalog
                                .games
                                .iter()
                                .find(|entry| entry.id == *game)
                                .map(|entry| entry.title.as_str())
                                .unwrap_or(game);
                            Message::new("summary.ready").arg("game", title)
                        } else {
                            Message::new("notice.saved")
                        };
                        show_toast(app, message, false);
                        if *enabled && app.summary.open.as_deref() == Some(game) {
                            app.summary.error = None;
                            issue(
                                app,
                                Pending::Summary(game.clone()),
                                "summarize_game",
                                json!({"id":game}),
                            );
                        } else if *enabled {
                            issue(
                                app,
                                Pending::RouteMapPrepare(game.clone()),
                                "prepare_ai_route_map",
                                json!({"id":game}),
                            );
                        }
                    }
                    _ => show_toast(app, Message::new("ai.invalid_response"), true),
                }
            } else {
                show_toast(app, Message::from_error(payload), true);
            }
        }
        Pending::Summary(id) => {
            if app.summary.open.as_deref() != Some(id) {
                return true;
            }
            if success {
                match serde_json::from_str::<ResultData>(payload) {
                    Ok(result) if result.game_id == *id => app.summary.result = Some(result),
                    _ => app.summary.error = Some(Message::new("ai.invalid_response")),
                }
            } else {
                app.summary.error = Some(Message::from_error(payload));
            }
        }
        Pending::RouteMapPrepare(_) | Pending::RouteMap(_) => {
            let message = if success {
                Message::new(if matches!(kind, Pending::RouteMapPrepare(_)) {
                    "route.ready"
                } else {
                    "route.opened"
                })
            } else {
                Message::detail(
                    "route.failed",
                    Message::from_error(payload).render(app.settings.language),
                )
            };
            show_toast(app, message, !success);
        }
        _ => return false,
    }
    true
}
pub(crate) fn render(html: &mut String, app: &App, game: &Game) {
    if app.summary.open.as_deref() != Some(&game.id) {
        return;
    }
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog ai-summary-dialog" role="dialog" aria-modal="true" aria-labelledby="summary-title"><header><h2 id="summary-title">{}</h2>{}</header><div class="editor-body">"#,t(app,"summary.title"),button("summary-close","",t(app,"common.close"),"close","quiet",false)).unwrap();
    if let Some(error) = &app.summary.error {
        write!(
            html,
            "<p class=\"error\" role=\"alert\">{}</p>{}",
            esc(&error.render(app.settings.language)),
            button(
                "summary-run",
                "",
                t(app, "summary.retry"),
                "refresh",
                "secondary",
                !app.ai_available || busy(app, &game.id)
            )
        )
        .unwrap();
        html.push_str(&button(
            "summary-reprepare",
            "",
            t(app, "summary.reprepare"),
            "settings",
            "quiet",
            !app.ai_available || busy(app, &game.id),
        ));
    } else if let Some(result) = &app.summary.result {
        let model_label = format!("{} · {}", result.provider, result.model);
        write!(html,"<section class=\"ai-summary-section\"><h3>{}</h3><div class=\"ai-summary-content\">{}</div></section><section class=\"ai-summary-section\"><h3>{}</h3><div class=\"ai-summary-content\">{}</div></section><dl class=\"configuration-summary\"><div><dt>{}</dt><dd>{}</dd></div><div><dt>{}</dt><dd>{}</dd></div><div><dt>{}</dt><dd>{}</dd></div></dl>",t(app,"summary.recap"),esc(&result.recap),t(app,"summary.progress"),esc(&result.current_progress),t(app,"summary.model"),esc(&model_label),t(app,"summary.save"),esc(&result.save_file),t(app,"summary.language"),esc(&result.script_language)).unwrap();
        html.push_str(&button(
            "open-resource",
            &result.source_url,
            t(app, "metadata.source"),
            "open_in_new",
            "quiet",
            false,
        ));
        html.push_str("<footer class=\"ai-summary-footer\">");
        let latest_notes = [
            galgear_i18n::Language::Chinese,
            galgear_i18n::Language::Japanese,
            galgear_i18n::Language::English,
        ]
        .map(|language| galgear_i18n::tr(language, "summary.latest_note"));
        for warning in &result.warnings {
            if !warning
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("save ")
                && !latest_notes.contains(&warning.as_str())
            {
                write!(html, "<p>{}</p>", esc(warning)).unwrap();
            }
        }
        write!(html, "<p>{}</p></footer>", t(app, "summary.latest_note")).unwrap();
    } else {
        write!(
            html,
            "<p class=\"ai-summary-loading\" role=\"status\">{}</p>",
            t(app, "summary.summarizing")
        )
        .unwrap();
    }
    html.push_str("</div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bridge::take_requests, state::apply_settings};

    fn selected_game(app: &mut App, enabled: bool) -> Game {
        app.ai_available = true;
        let game: Game = serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","engine":"unknown","modified":0,"tags":[]})).unwrap();
        app.detail.selected = Some(game.id.clone());
        app.settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .ai_summary = enabled;
        game
    }

    #[test]
    fn missing_runtime_disables_analysis_and_blocks_enable_requests() {
        let mut app = App::default();
        let game = selected_game(&mut app, false);
        app.ai_available = false;
        app.detail.editor = Some(crate::detail::Editor::Tools);
        take_requests();
        dispatch(&mut app, "toggle-ai-summary", "");
        assert!(take_requests().is_empty());
        let mut html = String::new();
        crate::detail::render(&mut html, &app, &game);
        let option = html.split("<button").find(|tag| tag.contains("data-action=\"toggle-ai-summary\"")).unwrap();
        assert!(option.split('>').next().unwrap().contains("disabled"));
        assert!(html.contains("Galgear-GalgameReverse-Skill"));
        app.settings.game_edits.get_mut(&game.id).unwrap().ai_summary = true;
        dispatch(&mut app, "summary-run", "");
        assert!(take_requests().is_empty());
        dispatch(&mut app, "toggle-ai-summary", "");
        assert_eq!(take_requests()[0].1, "set_ai_summary");
    }

    #[test]
    fn completed_setup_checks_option_and_exposes_recap_without_another_click() {
        let mut app = App::default();
        let game: Game = serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","engine":"unknown","modified":0,"tags":[]})).unwrap();
        app.detail.editor = Some(crate::detail::Editor::Tools);
        setup_completed(&mut app, &game.id, true);

        let mut html = String::new();
        crate::detail::render(&mut html, &app, &game);

        assert!(html.contains("aria-checked=\"true\" data-action=\"toggle-ai-summary\""));
        assert!(html.contains("data-action=\"summary-run\""));
        assert!(html.contains("AI前情提要"));
        assert!(html.contains("AI分析"));
        let recap = html.find("data-action=\"summary-run\"").unwrap();
        let route_map = html.find("data-action=\"route-map-open\"").unwrap();
        let edit = html.find("data-action=\"detail-edit\"").unwrap();
        assert!(recap < route_map && route_map < edit);
    }

    #[test]
    fn route_map_is_hidden_until_analysis_setup_succeeds() {
        let mut app = App::default();
        let game = selected_game(&mut app, false);
        let mut html = String::new();
        crate::detail::render(&mut html, &app, &game);
        assert!(!html.contains("data-action=\"route-map-open\""));

        dispatch(&mut app, "route-map-open", &game.id);
        assert_eq!(app.requests.last_id(), 0);
    }

    #[test]
    fn successful_analysis_setup_generates_and_saves_route_map_without_opening_it() {
        take_requests();
        let mut app = App::default();
        selected_game(&mut app, false);
        dispatch(&mut app, "toggle-ai-summary", "");
        let setup = app.requests.complete(1).unwrap();
        take_requests();
        let settings = json!({
            "library_folders": [],
            "card_size": 200,
            "sort_by": "title",
            "game_edits": {"game": {"ai_summary": true}},
        });

        resolve(
            &mut app,
            &setup,
            1,
            true,
            &serde_json::to_string(&settings).unwrap(),
        );

        let requests = take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "prepare_ai_route_map");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&requests[0].2).unwrap(),
            json!({"id":"game"})
        );
        assert!(app.settings.game_edits["game"].ai_summary);
        assert!(route_busy(&app, "game"));
        assert!(!app.busy());
    }

    #[test]
    fn failed_analysis_setup_does_not_generate_a_route_map() {
        take_requests();
        let mut app = App::default();
        selected_game(&mut app, false);

        resolve(
            &mut app,
            &Pending::SummarySetup("game".into(), true),
            1,
            false,
            "failed",
        );

        assert!(take_requests().is_empty());
        assert!(!app.settings.game_edits["game"].ai_summary);
    }

    #[test]
    fn route_map_click_dispatches_selected_game_and_serializes_its_ai_actions() {
        take_requests();
        let mut app = App::default();
        let game = selected_game(&mut app, true);

        dispatch(&mut app, "route-map-open", &game.id);
        dispatch(&mut app, "route-map-open", &game.id);
        dispatch(&mut app, "summary-run", "");
        dispatch(&mut app, "toggle-ai-summary", "");

        let requests = take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "open_ai_route_map");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&requests[0].2).unwrap(),
            json!({"id":"game"})
        );
        assert!(busy(&app, &game.id));
        assert!(!busy(&app, "other-game"));
        assert!(!app.busy());
    }

    #[test]
    fn route_generation_shows_its_loading_state_and_disables_the_button() {
        take_requests();
        let mut app = App::default();
        let game = selected_game(&mut app, true);
        app.detail.editor = Some(crate::detail::Editor::Tools);
        dispatch(&mut app, "route-map-open", &game.id);

        let mut html = String::new();
        crate::detail::render(&mut html, &app, &game);

        let button = html
            .split("<button")
            .find(|button| button.contains("data-action=\"route-map-open\""))
            .unwrap();
        assert!(button.split('>').next().unwrap().contains("disabled"));
        assert!(html.contains("正在生成AI路线图…"));
        assert_eq!(status_key(&app, &game.id), "route.generating");
    }

    #[test]
    fn route_generation_failure_keeps_analysis_enabled_and_allows_click_to_retry() {
        take_requests();
        let mut app = App::default();
        selected_game(&mut app, true);
        issue(
            &mut app,
            Pending::RouteMapPrepare("game".into()),
            "prepare_ai_route_map",
            json!({"id":"game"}),
        );
        let request = app.requests.complete(1).unwrap();
        take_requests();
        let error = serde_json::to_string(&Message::new("route.deepseek_required")).unwrap();

        resolve(&mut app, &request, 1, false, &error);

        assert!(app.settings.game_edits["game"].ai_summary);
        assert!(!busy(&app, "game"));
        let (message, error) = app.toast.as_ref().unwrap();
        assert!(*error);
        assert!(
            message
                .render(app.settings.language)
                .contains("AI路线图操作失败")
        );
        assert!(message.render(app.settings.language).contains("DeepSeek"));

        dispatch(&mut app, "route-map-open", "game");
        assert_eq!(take_requests()[0].1, "open_ai_route_map");
    }

    #[test]
    fn route_generation_and_opening_report_distinct_successes() {
        let mut app = App::default();
        resolve(
            &mut app,
            &Pending::RouteMapPrepare("game".into()),
            1,
            true,
            "null",
        );
        assert_eq!(app.toast.as_ref().unwrap().0.key, "route.ready");

        resolve(&mut app, &Pending::RouteMap("game".into()), 2, true, "null");
        assert_eq!(app.toast.as_ref().unwrap().0.key, "route.opened");
        assert!(!app.toast.as_ref().unwrap().1);
    }

    #[test]
    fn delayed_settings_cannot_uncheck_completed_setup() {
        let mut app = App::default();
        let stale = app.settings.clone();
        setup_completed(&mut app, "game", true);

        apply_settings(&mut app, stale, 0);

        assert!(app.settings.game_edits["game"].ai_summary);
    }

    #[test]
    fn delayed_settings_cannot_reenable_disabled_recap() {
        let mut app = App::default();
        app.settings
            .game_edits
            .entry("game".into())
            .or_default()
            .ai_summary = true;
        let stale = app.settings.clone();
        setup_completed(&mut app, "game", false);

        apply_settings(&mut app, stale, 0);

        assert!(!app.settings.game_edits["game"].ai_summary);
    }

    #[test]
    fn settings_requested_after_completion_can_supply_new_state() {
        let mut app = App::default();
        let later = app.settings.clone();
        setup_completed(&mut app, "game", true);

        apply_settings(&mut app, later, 1);

        assert!(!app.settings.game_edits.contains_key("game"));
    }

    #[test]
    fn modal_displays_two_sections_and_cached_result_as_escaped_text() {
        let mut app = App::default();
        let game:Game=serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","library":"F:/Games","engine":"unknown","modified":0,"tags":[]})).unwrap();
        app.summary.open = Some(game.id.clone());
        app.summary.result=Some(serde_json::from_value(json!({"game_id":"game","recap":"<script>unsafe</script>","current_progress":"Chapter one","provider":"deepseek","model":"model","save_file":"save.bmp","source_url":"https://vndb.org/v1","script_language":"zh-CN","warnings":[],"cached":true})).unwrap());
        let mut html = String::new();
        render(&mut html, &app, &game);
        assert!(html.contains("前情提要"));
        assert!(html.contains("当前进度"));
        assert!(!html.contains("最新存档未改变"));
        assert!(html.contains("ai-summary-footer"));
        assert!(html.contains("&lt;script&gt;unsafe&lt;/script&gt;"));
        assert!(!html.contains("<script>"));
    }
}
