//! Stateless HTML primitives and formatting.
use crate::icons;

pub(crate) fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn icon(name: &str) -> String {
    if let Some(svg) = icons::google(name) {
        return svg
            .replacen(
                "<svg",
                r#"<svg class="icon google-icon" aria-hidden="true""#,
                1,
            )
            .replace("fill=\"#1f1f1f\"", "fill=\"currentColor\"")
            .replace("stroke=\"#1f1f1f\"", "stroke=\"currentColor\"");
    }
    let body = match name {
        "top" => r#"<path d="m6 14 6-6 6 6M6 4h12"/>"#,
        "trash" => r#"<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 10v7m4-7v7"/>"#,
        "star" => {
            r#"<path d="m12 3 2.8 5.7 6.3.9-4.6 4.5 1.1 6.3-5.6-3-5.6 3 1.1-6.3L3 9.6l6.2-.9Z"/>"#
        }
        "star-filled" => {
            r#"<path fill="currentColor" stroke="none" d="m12 2.5 2.94 5.96 6.58.95-4.76 4.64 1.12 6.55L12 17.51l-5.88 3.09 1.12-6.55L2.48 9.41l6.58-.95Z"/>"#
        }
        "trophy" => {
            r#"<path d="M8 3h8v6a4 4 0 0 1-8 0ZM8 5H4v2a4 4 0 0 0 4 4m8-6h4v2a4 4 0 0 1-4 4m-4 2v5m-4 3h8m-7 0v-3h6v3"/>"#
        }
        "trophy-filled" => {
            r#"<path fill-rule="evenodd" d="M7 2h10v2h4v4a5 5 0 0 1-4.35 4.96A5 5 0 0 1 13 15.9V18h4v4H7v-4h4v-2.1a5 5 0 0 1-3.65-2.94A5 5 0 0 1 3 8V4h4ZM7 6H5v2a3 3 0 0 0 2 2.83Zm10 4.83A3 3 0 0 0 19 8V6h-2Z"/>"#
        }
        "chevron" => r#"<path d="m7 10 5 5 5-5"/>"#,
        "games" => {
            r#"<rect x="3" y="5" width="18" height="14" rx="4"/><path d="M7 10v4m-2-2h4m6-1h.01M18 14h.01"/>"#
        }
        "patches" => {
            r#"<path d="M12 3v4m0 10v4M3 12h4m10 0h4"/><rect x="7" y="7" width="10" height="10" rx="3"/><path d="m10 12 1.5 1.5L14 11"/>"#
        }
        "settings" => {
            r#"<path d="m9 3-1 3-3 1-2 3 2 2-1 3 3 2 3-1 2 2 3-1 1-3 3-1 1-3-2-2 1-3-3-2-3 1-2-2Z"/><circle cx="11" cy="11" r="3"/>"#
        }
        "about" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/>"#,
        "folder" => {
            r#"<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>"#
        }
        "search" => r#"<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4 4"/>"#,
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "refresh" => {
            r#"<path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 7a7 7 0 0 1 12-2l2 3M4 16l2 3a7 7 0 0 0 12-2"/>"#
        }
        "arrow" => r#"<path d="M5 12h14m-5-5 5 5-5 5"/>"#,
        "close" => r#"<path d="m6 6 12 12M6 18 18 6"/>"#,
        "play" => r#"<path d="m9 5 11 7-11 7Z"/>"#,
        "image" => {
            r#"<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8" cy="8" r="1.5"/><path d="m3 17 5-5 4 4 4-6 5 7"/>"#
        }
        "check" => r#"<path d="m5 12 4 4L19 6"/>"#,
        "disc" => {
            r#"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="2.5"/><path d="m7 5 3 4m4 6 3 4"/>"#
        }
        "language" => {
            r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c-5 5-5 13 0 18m0-18c5 5 5 13 0 18"/>"#
        }
        "heart" => r#"<path d="M20 5c-3-3-7-1-8 1-1-2-5-4-8-1-4 4 2 10 8 14 6-4 12-10 8-14Z"/>"#,
        _ => r#"<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5Z"/>"#,
    };
    format!(
        r#"<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{body}</svg>"#
    )
}

pub(crate) fn brand() -> &'static str {
    r#"<svg viewBox="0 0 48 48" fill="none" aria-hidden="true"><path d="M24 8c-12-12-22 5-11 11C-4 20 6 37 16 31c-3 17 17 17 16 4 13 9 24-7 12-14C50 8 33 1 28 14Z" fill="currentColor" transform="translate(-2 2) scale(.92)"/><circle cx="24" cy="24" r="5" fill="white"/><path d="m20 24 3 3 6-7" stroke="white" stroke-width="2"/></svg>"#
}

pub(crate) fn button(
    action: &str,
    value: &str,
    label: &str,
    symbol: &str,
    class: &str,
    disabled: bool,
) -> String {
    if class.split_whitespace().any(|name| name == "icon-only") {
        return icon_button(action, value, label, symbol, class, disabled);
    }
    if matches!(action, "editor-close" | "summary-close" | "close-history") {
        return format!(
            r#"<button type="button" class="button {class} close-button" data-action="{action}" data-value="{}" aria-label="{}" title="{}" {}>{}</button>"#,
            esc(value),
            esc(label),
            esc(label),
            if disabled { "disabled" } else { "" },
            icon("close")
        );
    }
    format!(
        r#"<button type="button" class="button {class}" data-action="{action}" data-value="{}" {}>{}<span>{label}</span></button>"#,
        esc(value),
        if disabled { "disabled" } else { "" },
        if symbol.is_empty() {
            String::new()
        } else {
            icon(symbol)
        }
    )
}

pub(crate) fn icon_button(
    action: &str,
    value: &str,
    label: &str,
    symbol: &str,
    class: &str,
    disabled: bool,
) -> String {
    format!(
        r#"<button type="button" class="button icon-only {class}" data-action="{action}" data-value="{}" aria-label="{}" title="{}" {}>{}</button>"#,
        esc(value),
        esc(label),
        esc(label),
        if disabled { "disabled" } else { "" },
        icon(symbol)
    )
}

pub(crate) fn bytes(value: u64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{:.1} KiB", value as f64 / 1024.)
    } else {
        format!("{:.1} MiB", value as f64 / (1024. * 1024.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_are_html_escaped() {
        assert_eq!(esc("游戏\"<img>&'"), "游戏&quot;&lt;img&gt;&amp;&#39;");
    }
}
