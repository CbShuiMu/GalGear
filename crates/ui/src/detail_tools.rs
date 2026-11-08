//! 详情快捷开关与完整配置共用状态、可用条件和动作，保存仍由既有请求流程负责。
use crate::{
    ai_summary, features,
    models::{Game, GameEdit},
    state::App,
    view::t,
    widgets::{esc, icon},
};
use std::fmt::Write;

struct ToolOption {
    action: &'static str,
    key: &'static str,
    glyph: &'static str,
    class: &'static str,
    checked: bool,
    disabled: bool,
}

fn options(app: &App, game: &Game, edit: &GameEdit) -> [ToolOption; 4] {
    let backup_available = features::can_auto_backup(app, &game.id);
    [
        (
            "toggle-locale",
            "tools.locale_launch",
            "LE",
            "locale",
            edit.launch_mode == "locale",
            false,
        ),
        (
            "toggle-upscale",
            "tools.upscale",
            "M",
            "upscale",
            edit.upscale,
            false,
        ),
        (
            "toggle-auto-backup",
            "backup.auto_short",
            "",
            "backup",
            edit.auto_backup && backup_available,
            !backup_available,
        ),
        (
            "toggle-ai-summary",
            "summary.option",
            "AI",
            "ai",
            edit.ai_summary,
            (!app.ai_available && !edit.ai_summary) || ai_summary::busy(app, &game.id),
        ),
    ]
    .map(
        |(action, key, glyph, class, checked, unavailable)| ToolOption {
            action,
            key,
            glyph,
            class,
            checked,
            disabled: app.busy() || !game.available || unavailable,
        },
    )
}

pub(crate) fn shortcuts(html: &mut String, app: &App, game: &Game, edit: &GameEdit) {
    write!(html, r#"<div class="game-tools-shortcuts" data-render-key="game-tools-shortcuts" role="group" aria-label="{}"><button type="button" class="icon-button game-settings-button" data-action="game-settings" aria-label="{}" title="{}" {}>{}</button><div class="game-tools-reveal"><div class="game-tools-buttons">"#,
        t(app, "detail.configure"), t(app, "detail.configure"), t(app, "detail.configure"),
        if game.available { "" } else { "disabled" }, icon("settings")).unwrap();
    for option in options(app, game, edit) {
        write!(html, r#"<button type="button" class="game-tool-shortcut tool-{}" data-action="{}" aria-pressed="{}" aria-label="{}" title="{}" {}>{}</button>"#,
            option.class, option.action, option.checked, esc(t(app, option.key)), esc(t(app, option.key)),
            if option.disabled { "disabled" } else { "" },
            if option.glyph.is_empty() { icon("backup") } else { format!("<span aria-hidden=\"true\">{}</span>", option.glyph) }).unwrap();
    }
    html.push_str("</div></div></div>");
}

pub(crate) fn configuration_options(html: &mut String, app: &App, game: &Game, edit: &GameEdit) {
    for option in options(app, game, edit) {
        write!(html, r#"<button class="game-tool-option" role="checkbox" aria-checked="{}" data-action="{}" {}><span class="tool-check">{}</span><span>{}</span></button>"#,
            option.checked, option.action, if option.disabled { "disabled" } else { "" },
            if option.checked { icon("check") } else { String::new() }, t(app, option.key)).unwrap();
    }
}
