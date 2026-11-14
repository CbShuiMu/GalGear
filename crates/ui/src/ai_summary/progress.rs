//! 右下角分析通知独立于普通 toast，生命周期与真实请求绑定。
use crate::{
    models::{AnalysisProgress, AnalysisProgressEvent, AnalysisStageStatus},
    requests::Pending,
    state::{APP, App},
    view::{paint, t},
    widgets::{esc, stage_toast},
};
use std::fmt::Write;
use wasm_bindgen::prelude::wasm_bindgen;

pub(super) struct Notice {
    request_id: u32,
    title: String,
    fallback: &'static str,
    progress: AnalysisProgress,
}

pub(super) fn begin(app: &mut App, kind: &Pending, request_id: u32) {
    let (game, fallback) = match kind {
        Pending::SummarySetup(game, true) => (game, "summary.progress_preparing"),
        Pending::Summary(game) => (game, "summary.progress_summarizing"),
        Pending::RouteMapPrepare(game) | Pending::RouteMap(game) => (game, "route.generating"),
        _ => return,
    };
    if app.toast.as_ref().is_some_and(|(message, _)| {
        message.key.starts_with("summary.") || message.key.starts_with("route.")
    }) {
        app.toast = None;
    }
    let title = app
        .catalog
        .games
        .iter()
        .find(|entry| entry.id == *game)
        .map(|entry| entry.title.clone())
        .unwrap_or_else(|| game.clone());
    // 首次启用后的路线图落盘沿用同一张卡片，保留已完成的 AI 阶段。
    let progress = if matches!(kind, Pending::RouteMapPrepare(_)) {
        app.summary
            .progress
            .remove(game)
            .map(|notice| notice.progress)
            .unwrap_or_default()
    } else {
        AnalysisProgress::default()
    };
    app.summary.progress.insert(
        game.clone(),
        Notice {
            request_id,
            title,
            fallback,
            progress,
        },
    );
}

pub(super) fn finish(app: &mut App, request_id: u32) {
    app.summary
        .progress
        .retain(|_, notice| notice.request_id != request_id);
}

fn stage_key(stage: &str) -> Option<&'static str> {
    Some(match stage {
        "checking" => "summary.stage_checking",
        "resources" => "summary.stage_resources",
        "scripts" => "summary.stage_scripts",
        "save" => "summary.stage_save",
        "route" => "summary.stage_route",
        "recap" => "summary.stage_recap",
        "publishing" => "summary.stage_publishing",
        _ => return None,
    })
}

fn apply(app: &mut App, event: AnalysisProgressEvent) -> bool {
    let valid = matches!(app.requests.get(event.request_id),
        Some(Pending::SummarySetup(game, true) | Pending::Summary(game)) if game == &event.game_id);
    if !valid
        || event.progress.stages.len() > 7
        || event
            .progress
            .stages
            .iter()
            .any(|stage| stage_key(&stage.stage).is_none())
    {
        return false;
    }
    let Some(notice) = app.summary.progress.get_mut(&event.game_id) else {
        return false;
    };
    if notice.request_id != event.request_id || event.progress.sequence <= notice.progress.sequence
    {
        return false;
    }
    notice.progress = event.progress;
    true
}

#[wasm_bindgen]
pub fn ai_summary_progress(payload: &str) {
    let Ok(event) = serde_json::from_str(payload) else {
        return;
    };
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if apply(&mut app, event) {
            paint(&app);
        }
    });
}

pub(crate) fn render(html: &mut String, app: &App) {
    if app.summary.progress.is_empty() {
        return;
    }
    write!(
        html,
        r#"<div class="ai-analysis-notices" tabindex="0" aria-label="{}">"#,
        t(app, "summary.analysis_progress")
    )
    .unwrap();
    for (game, notice) in &app.summary.progress {
        write!(
            html,
            r#"<div id="ai-analysis-{}" class="ai-analysis-notice" title="{}">"#,
            esc(game),
            esc(&notice.title)
        )
        .unwrap();
        let current_key = if notice.progress.stages.is_empty() {
            notice.fallback
        } else {
            let current = notice
                .progress
                .stages
                .iter()
                .find(|stage| {
                    notice.progress.current_stage.as_deref() == Some(stage.stage.as_str())
                        && stage.status == AnalysisStageStatus::Running
                })
                .or_else(|| {
                    notice
                        .progress
                        .stages
                        .iter()
                        .rfind(|stage| stage.status == AnalysisStageStatus::Running)
                });
            let fallback = if matches!(
                app.requests.get(notice.request_id),
                Some(Pending::RouteMapPrepare(_) | Pending::RouteMap(_))
            ) {
                notice.fallback
            } else {
                "summary.stage_finalizing"
            };
            current
                .and_then(|stage| stage_key(&stage.stage))
                .unwrap_or(fallback)
        };
        stage_toast(
            html,
            &format!("ai-analysis-stage-{}", esc(game)),
            t(app, current_key),
            false,
            true,
        );
        html.push_str("</div>");
    }
    html.push_str("</div>");
}
