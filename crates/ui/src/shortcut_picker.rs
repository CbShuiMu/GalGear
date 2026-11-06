//! 工具快捷键录制草稿；按键由 DOM 传入，确认后才更新设置卡片草稿。
use crate::{
    features,
    state::App,
    view::t,
    widgets::{button, esc},
};
use serde::Deserialize;
use std::fmt::Write;

#[derive(Default)]
pub(crate) struct State {
    recording: Option<Recording>,
}

struct Recording {
    field: String,
    keys: Vec<String>,
    valid: bool,
    error: bool,
}

#[derive(Deserialize)]
struct KeyPress {
    code: String,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

fn label(field: &str) -> Option<&'static str> {
    match field {
        "tool-hotkey" => Some("tools.hotkey"),
        "screenshot-hotkey" => Some("screenshot.hotkey"),
        "font-hotkey" => Some("font.hotkey"),
        _ => None,
    }
}

fn value(app: &App, field: &str) -> String {
    let saved = match field {
        "tool-hotkey" if app.settings.magpie_hotkey.is_empty() => "Ctrl+Alt+A",
        "tool-hotkey" => &app.settings.magpie_hotkey,
        "screenshot-hotkey" => &app.settings.screenshot_hotkey,
        _ => &app.settings.font_hotkey,
    };
    features::setting(app, field, saved)
}

pub(crate) fn field(html: &mut String, app: &App, field: &str) {
    let Some(key) = label(field) else { return };
    write!(html, r#"<div class="edit-field"><span id="{field}-label">{}</span><button type="button" class="shortcut-field" id="{field}" data-action="shortcut-open" data-value="{field}" aria-labelledby="{field}-label" aria-haspopup="dialog" {}>{}</button></div>"#,
        t(app, key), if app.busy() { "disabled" } else { "" }, esc(&value(app, field))).unwrap();
}

pub(crate) fn dispatch(app: &mut App, action: &str, payload: &str) -> bool {
    match action {
        "shortcut-open" if !app.busy() && label(payload).is_some() => {
            app.shortcut_picker.recording = Some(Recording {
                field: payload.into(),
                keys: Vec::new(),
                valid: false,
                error: false,
            });
        }
        "shortcut-reset" => {
            if let Some(recording) = &mut app.shortcut_picker.recording {
                recording.keys.clear();
                recording.valid = false;
                recording.error = false;
            }
        }
        "shortcut-cancel" => app.shortcut_picker.recording = None,
        "shortcut-pressed" => {
            if let Some(recording) = &mut app.shortcut_picker.recording
                && let Ok(pressed) = serde_json::from_str::<KeyPress>(payload)
            {
                let mut keys = Vec::new();
                for (held, name) in [
                    (pressed.ctrl, "Ctrl"),
                    (pressed.alt, "Alt"),
                    (pressed.shift, "Shift"),
                    (pressed.meta, "Win"),
                ] {
                    if held {
                        keys.push(name.to_owned());
                    }
                }
                let modifier_only = matches!(
                    pressed.code.as_str(),
                    "ControlLeft"
                        | "ControlRight"
                        | "AltLeft"
                        | "AltRight"
                        | "ShiftLeft"
                        | "ShiftRight"
                        | "MetaLeft"
                        | "MetaRight"
                );
                let key = key_name(&pressed.code);
                recording.valid = !keys.is_empty() && key.is_some();
                recording.error = !recording.valid && !modifier_only;
                if let Some(key) = key {
                    keys.push(key);
                }
                recording.keys = keys;
            }
        }
        "shortcut-confirm" => {
            if app
                .shortcut_picker
                .recording
                .as_ref()
                .is_some_and(|recording| recording.valid)
                && let Some(recording) = app.shortcut_picker.recording.take()
            {
                app.form.insert(recording.field, recording.keys.join("+"));
            }
        }
        _ => return false,
    }
    true
}

fn key_name(code: &str) -> Option<String> {
    for prefix in ["Key", "Digit", "Numpad"] {
        if let Some(key) = code.strip_prefix(prefix)
            && key.len() == 1
            && (if prefix == "Key" {
                key.as_bytes()[0].is_ascii_uppercase()
            } else {
                key.as_bytes()[0].is_ascii_digit()
            })
        {
            return Some(if prefix == "Numpad" {
                format!("NumPad{key}")
            } else {
                key.into()
            });
        }
    }
    if code
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
    {
        return Some(code.into());
    }
    Some(
        match code {
            "Space" | "Tab" | "Enter" | "Backspace" | "Delete" | "Insert" | "Home" | "End"
            | "PageUp" | "PageDown" | "Pause" | "PrintScreen" | "Semicolon" | "Comma" | "Minus"
            | "Period" | "Slash" | "Backslash" | "Quote" => code,
            "ArrowLeft" => "Left",
            "ArrowRight" => "Right",
            "ArrowUp" => "Up",
            "ArrowDown" => "Down",
            "Equal" => "Equals",
            "Backquote" => "Backtick",
            "BracketLeft" => "LeftBracket",
            "BracketRight" => "RightBracket",
            "NumpadAdd" => "Add",
            "NumpadSubtract" => "Subtract",
            "NumpadMultiply" => "Multiply",
            "NumpadDivide" => "Divide",
            "NumpadDecimal" => "Decimal",
            "NumpadEnter" => "Enter",
            _ => return None,
        }
        .into(),
    )
}

pub(crate) fn render(html: &mut String, app: &App) {
    let Some(recording) = &app.shortcut_picker.recording else {
        return;
    };
    write!(html, r#"<div class="editor-overlay shortcut-overlay"><section class="editor-dialog shortcut-dialog" role="dialog" aria-modal="true" aria-labelledby="shortcut-title" aria-describedby="shortcut-help" data-shortcut-field="{}"><header><h2 id="shortcut-title">{}</h2></header><div class="editor-body"><p id="shortcut-help">{}</p><div id="shortcut-capture" class="shortcut-capture" tabindex="0" aria-live="polite">"#,
        recording.field, t(app, label(&recording.field).unwrap_or("tools.hotkey")), t(app,"shortcut.help")).unwrap();
    if recording.keys.is_empty() {
        write!(
            html,
            "<span class=\"muted\">{}</span>",
            t(app, "shortcut.waiting")
        )
        .unwrap();
    } else {
        for (index, key) in recording.keys.iter().enumerate() {
            if index > 0 {
                html.push_str("<span aria-hidden=\"true\">+</span>");
            }
            write!(html, "<kbd>{}</kbd>", esc(key)).unwrap();
        }
    }
    html.push_str("</div>");
    if recording.error {
        write!(
            html,
            "<p class=\"error-text\" role=\"alert\">{}</p>",
            t(app, "shortcut.invalid")
        )
        .unwrap();
    }
    write!(
        html,
        "</div><footer class=\"shortcut-actions\">{}{}{}</footer></section></div>",
        button(
            "shortcut-reset",
            "",
            t(app, "shortcut.reset"),
            "refresh",
            "quiet",
            false
        ),
        button(
            "shortcut-cancel",
            "",
            t(app, "common.cancel"),
            "close",
            "secondary",
            false
        ),
        button(
            "shortcut-confirm",
            "",
            t(app, "common.confirm"),
            "check",
            "primary",
            !recording.valid
        )
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_reset_cancel_and_confirm_only_change_the_draft() {
        let mut app = App::default();
        dispatch(&mut app, "shortcut-open", "font-hotkey");
        dispatch(
            &mut app,
            "shortcut-pressed",
            r#"{"code":"KeyA","ctrl":false,"alt":true,"shift":false,"meta":false}"#,
        );
        assert!(!app.form.contains_key("font-hotkey"));
        dispatch(&mut app, "shortcut-reset", "");
        dispatch(&mut app, "shortcut-confirm", "");
        assert!(app.shortcut_picker.recording.is_some());
        dispatch(&mut app, "shortcut-cancel", "");
        assert!(!app.form.contains_key("font-hotkey"));
        dispatch(&mut app, "shortcut-open", "font-hotkey");
        dispatch(
            &mut app,
            "shortcut-pressed",
            r#"{"code":"KeyA","ctrl":false,"alt":true,"shift":false,"meta":false}"#,
        );
        dispatch(&mut app, "shortcut-confirm", "");
        assert_eq!(app.form["font-hotkey"], "Alt+A");
        assert_eq!(app.settings.font_hotkey, "Shift+T");
    }
    #[test]
    fn incomplete_or_unsupported_chords_cannot_be_confirmed() {
        let mut app = App::default();
        dispatch(&mut app, "shortcut-open", "tool-hotkey");
        for (code, alt) in [("KeyA", false), ("AltLeft", true), ("AudioVolumeUp", true)] {
            dispatch(
                &mut app,
                "shortcut-pressed",
                &serde_json::json!({"code":code,"alt":alt,"ctrl":false,"shift":false,"meta":false})
                    .to_string(),
            );
            dispatch(&mut app, "shortcut-confirm", "");
            assert!(app.shortcut_picker.recording.is_some());
            assert!(!app.form.contains_key("tool-hotkey"));
        }
    }
}
