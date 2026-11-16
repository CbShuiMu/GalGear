//! 右下角分析通知独立于普通 toast，生命周期与真实请求绑定。
use crate::{
    models::{AnalysisProgress, AnalysisProgressEvent, AnalysisStageStatus},
    requests::Pending,
    state::{APP, App},
    view::{paint, t},
    widgets::{esc, numbered_stage_text, stage_toast},
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
        Pending::MediaExtract(game, kind) => (game, kind.progress_key()),
        _ => return,
    };
    if app.toast.as_ref().is_some_and(|(message, _)| {
        message.key.starts_with("summary.")
            || message.key.starts_with("route.")
            || message.key.starts_with("media.")
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

pub(crate) fn finish(app: &mut App, request_id: u32) {
    app.summary
        .progress
        .retain(|_, notice| notice.request_id != request_id);
}

// 固定流程位置使缓存跳步、重试和返回早期阶段不会改变分母。
const STAGES: [(&str, &str); 7] = [
    ("checking", "summary.stage_checking"),
    ("resources", "summary.stage_resources"),
    ("scripts", "summary.stage_scripts"),
    ("save", "summary.stage_save"),
    ("route", "summary.stage_route"),
    ("recap", "summary.stage_recap"),
    ("publishing", "summary.stage_publishing"),
];

fn stage_info(stage: &str) -> Option<(usize, &'static str)> {
    STAGES
        .iter()
        .enumerate()
        .find_map(|(index, &(name, key))| (name == stage).then_some((index + 1, key)))
}

fn apply(app: &mut App, event: AnalysisProgressEvent) -> bool {
    let media = matches!(
        app.requests.get(event.request_id),
        Some(Pending::MediaExtract(..))
    );
    let valid = matches!(app.requests.get(event.request_id),
        Some(Pending::SummarySetup(game, true) | Pending::Summary(game) | Pending::MediaExtract(game, _)) if game == &event.game_id);
    if !valid
        || event.progress.stages.len() > STAGES.len()
        || event.progress.stages.iter().any(|stage| {
            if media {
                !matches!(
                    stage.stage.as_str(),
                    "checking" | "resources" | "sampling" | "extracting" | "publishing"
                )
            } else {
                stage_info(&stage.stage).is_none()
            }
        })
        || event
            .progress
            .batch
            .as_ref()
            .is_some_and(|batch| !media || batch.total == 0 || batch.completed > batch.total)
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
        let route_task = matches!(
            app.requests.get(notice.request_id),
            Some(Pending::RouteMapPrepare(_) | Pending::RouteMap(_))
        );
        let media_task = matches!(
            app.requests.get(notice.request_id),
            Some(Pending::MediaExtract(..))
        );
        let (current_key, position) = if media_task {
            match notice.progress.current_stage.as_deref() {
                Some("checking") => ("summary.stage_checking", Some((1, 5))),
                Some("resources") => ("media.identifying", Some((2, 5))),
                Some("sampling") => ("media.sampling", Some((3, 5))),
                Some("extracting") => (notice.fallback, Some((4, 5))),
                Some("publishing") => ("media.publishing", Some((5, 5))),
                _ => (
                    if notice.progress.batch.is_some() {
                        notice.fallback
                    } else {
                        "media.identifying"
                    },
                    None,
                ),
            }
        } else if route_task {
            // 分析完成后的路线图生成是独立命令，不能沿用上一请求的运行中阶段。
            (notice.fallback, Some((1, 1)))
        } else if notice.progress.stages.is_empty() {
            (notice.fallback, None)
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
            match current.and_then(|stage| stage_info(&stage.stage)) {
                Some((number, key)) => (key, Some((number, STAGES.len()))),
                None => ("summary.stage_finalizing", None),
            }
        };
        let mut message = t(app, current_key).to_owned();
        if media_task && let Some(batch) = &notice.progress.batch {
            message.push_str(&galgear_i18n::format_text(
                app.settings.language,
                "media.batch_progress",
                &[
                    ("completed", &batch.completed.to_string()),
                    ("total", &batch.total.to_string()),
                    ("files", &batch.files.to_string()),
                ],
            ));
        }
        let text = match position {
            Some((current, total)) => {
                numbered_stage_text(app.settings.language, current, total, &message)
            }
            None => esc(&message),
        };
        stage_toast(
            html,
            &format!("ai-analysis-stage-{}", esc(game)),
            &text,
            false,
            true,
        );
        html.push_str("</div>");
    }
    html.push_str("</div>");
}
