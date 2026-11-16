//! 详情页媒体提取入口；与剧情任务共用请求互斥及后台进度通知。
use crate::{
    ai_summary,
    models::Game,
    requests::Pending,
    state::{App, show_toast},
    view::t,
    widgets::{button, esc, icon},
};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write;

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Cg,
    Audio,
}

impl Kind {
    pub(crate) fn progress_key(self) -> &'static str {
        match self {
            Self::Cg => "media.extracting_cg",
            Self::Audio => "media.extracting_audio",
        }
    }
}

#[derive(Deserialize)]
struct ResultData {
    game_id: String,
    kind: Kind,
    output: String,
    files: usize,
    warnings: Vec<String>,
}

pub(crate) fn shortcuts(html: &mut String, app: &App, game: &Game) {
    let disabled =
        app.busy() || !app.ai_available || !game.available || ai_summary::busy(app, &game.id);
    write!(html, r#"<div class="media-tools-anchor" data-render-key="media-tools"><div class="media-tools" role="group" aria-label="{}"><button type="button" class="icon-button media-tools-toggle" aria-label="{}" title="{}" {}>{}</button><div class="media-tools-reveal"><div class="media-tools-buttons">"#,
        esc(t(app, "media.title")), esc(t(app, "media.title")), esc(t(app, "media.title")),
        if disabled { "disabled" } else { "" }, icon("upload")).unwrap();
    for (action, label, glyph) in [
        ("extract-cg", "media.cg", "image_search"),
        ("extract-audio", "media.audio", "mic"),
    ] {
        html.push_str(&button(
            action,
            &game.id,
            t(app, label),
            glyph,
            "quiet icon-only media-tool",
            disabled,
        ));
    }
    html.push_str("</div></div></div></div>");
}

pub(crate) fn dispatch(app: &mut App, action: &str) -> bool {
    let kind = match action {
        "extract-cg" => Kind::Cg,
        "extract-audio" => Kind::Audio,
        _ => return false,
    };
    let Some(game) = app
        .detail
        .selected
        .as_ref()
        .and_then(|id| app.catalog.games.iter().find(|game| game.id == *id))
    else {
        return true;
    };
    if app.busy() || !app.ai_available || !game.available || ai_summary::busy(app, &game.id) {
        return true;
    }
    let id = game.id.clone();
    ai_summary::issue(
        app,
        Pending::MediaExtract(id.clone(), kind),
        "extract_game_media",
        json!({"id":id,"kind":kind}),
    );
    true
}

pub(crate) fn resolve(
    app: &mut App,
    pending: &Pending,
    request: u32,
    success: bool,
    payload: &str,
) -> bool {
    let Pending::MediaExtract(id, kind) = pending else {
        return false;
    };
    ai_summary::progress::finish(app, request);
    let result = if success {
        serde_json::from_str::<ResultData>(payload)
            .ok()
            .filter(|result| {
                result.game_id == *id
                    && result.kind == *kind
                    && result.files > 0
                    && !result.output.is_empty()
            })
            .ok_or_else(|| Message::new("ai.invalid_response"))
    } else {
        Err(Message::from_error(payload))
    };
    match result {
        Ok(result) => {
            let partial = !result.warnings.is_empty();
            show_toast(
                app,
                Message::new(if partial {
                    "media.partial"
                } else {
                    "media.completed"
                })
                .arg("count", result.files)
                .arg("path", result.output)
                .arg("detail", result.warnings.join("；")),
                partial,
            );
        }
        Err(error) => show_toast(
            app,
            Message::detail("media.failed", error.render(app.settings.language)),
            true,
        ),
    }
    true
}
