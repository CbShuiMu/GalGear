//! Application shell, navigation and shared presentation.
use crate::patches::{download_notice, patches_page};
use crate::{
    accounts, bridge::render, choices, context_menu, detail, game_setup, library, models::Game,
    preferences, requests::Pending, state::App, widgets::brand, widgets::esc, widgets::icon,
};
use galgear_i18n::{format_text, tr};
use std::fmt::Write;

pub(crate) fn network_activities(app: &App) -> Vec<String> {
    let mut activities = Vec::new();
    for pending in app.requests.values() {
        let key = match pending {
            Pending::AiRuntime => "skills.updating",
            Pending::Cloud(crate::cloud::Request::Upload(_)) => "cloud.upload",
            Pending::Cloud(crate::cloud::Request::Download(_)) => "cloud.download",
            Pending::Cloud(crate::cloud::Request::List(_)) => "cloud.refresh",
            Pending::Cloud(crate::cloud::Request::Test) => "cloud.test",
            Pending::SearchMetadata(..) => "network.metadata_search",
            Pending::CoverOptions => "cover.loading",
            Pending::Cover => "cover.saving",
            Pending::SelectMetadata => "network.metadata_fetch",
            Pending::Resources(_) => "network.patches",
            Pending::Update => "network.update",
            Pending::Account => "network.account",
            Pending::DownloadPreview => "network.resource",
            Pending::Enrich => {
                if let Some(message) = &app.enrichment_progress {
                    let text = message.render(app.settings.language);
                    if !activities.contains(&text) {
                        activities.push(text);
                    }
                }
                continue;
            }
            _ => continue,
        };
        let text = t(app, key).to_owned();
        if !activities.contains(&text) {
            activities.push(text);
        }
    }
    activities
}

pub(crate) fn network_popup(html: &mut String, app: &App) {
    let activities = network_activities(app);
    if activities.is_empty() {
        return;
    }
    write!(html, r#"<aside class="network-popup" role="status" aria-live="polite" aria-atomic="true" aria-label="{}"><span class="network-spinner" aria-hidden="true"></span><div class="network-copy"><strong>{}</strong><ul>"#, t(app,"network.title"),t(app,"network.title")).unwrap();
    for text in activities {
        write!(html, "<li>{}</li>", esc(&text)).unwrap();
    }
    html.push_str("</ul></div></aside>");
}

pub(crate) fn t(app: &App, key: &str) -> &'static str {
    tr(app.settings.language, key)
}

pub(crate) fn f(app: &App, key: &str, args: &[(&str, &str)]) -> String {
    esc(&format_text(app.settings.language, key, args))
}

pub(crate) fn engine<'a>(app: &App, game: &'a Game) -> &'a str {
    if matches!(game.engine.as_str(), "unknown" | "本地游戏") {
        t(app, "games.local_engine")
    } else {
        &game.engine
    }
}

pub(crate) fn paint(app: &App) {
    if !app.initialized && app.busy() {
        render(
            r#"<div class="boot-screen" aria-busy="true"><span aria-hidden="true">✦</span><strong>GalGear</strong></div>"#,
        );
        return;
    }
    let mut html = String::with_capacity(24000);
    let background = if app.page == "home" {
        None
    } else {
        app.settings
            .background_image
            .as_ref()
            .and_then(preferences::background_url)
    };
    write!(html, r#"<div class="app-shell cards-240" data-custom-background="{}" data-cover-scale="1.2" data-font-size="{}" data-theme="{}" data-color-theme="{}" lang="{}"><div id="app-background" class="app-background" aria-hidden="true">"#,
        background.is_some(), app.settings.font_size, if app.settings.theme=="dark" {"dark"}else{"light"}, if matches!(app.settings.color_theme.as_str(), "white-pink" | "white") { app.settings.color_theme.as_str() } else { "pink" }, app.settings.language.code()).unwrap();
    if let Some(url) = background {
        write!(html, r#"<img id="app-background-image" src="{}" alt="" decoding="async" draggable="false"/>"#, esc(&url)).unwrap();
    }
    write!(html, r#"</div><aside class="sidebar"><a class="brand" href="./" data-action="page" data-value="home" aria-label="GalGear" title="GalGear"><span class="brand-mark">{}</span></a><nav class="navigation" aria-label="{}">"#, brand(), t(app,"nav.main")).unwrap();
    for (value, key) in [
        ("home", "nav.home"),
        ("games", "nav.games"),
        ("collections", "folders.title"),
        ("patches", "nav.patches"),
    ] {
        if value != "patches" || app.settings.show_patches {
            nav(&mut html, app, value, t(app, key));
        }
    }
    write!(
        html,
        r#"</nav><div class="sidebar-bottom"><nav class="navigation" aria-label="{}">"#,
        t(app, "nav.more")
    )
    .unwrap();
    let dark = app.settings.theme == "dark";
    write!(html,r#"<button type="button" id="theme-toggle" class="nav-item theme-toggle" data-action="toggle-theme" aria-label="{}" title="{}" aria-pressed="{}" {}>{}</button>"#,t(app,if dark{"theme.dark"}else{"theme.light"}),t(app,if dark{"theme.to_light"}else{"theme.to_dark"}),dark,if app.requests.values().any(|p|matches!(p,Pending::Theme)){"disabled"}else{""},icon(if dark {"dark_mode"} else {"light_mode"})).unwrap();
    nav(&mut html, app, "settings", t(app, "nav.settings"));
    nav(&mut html, app, "about", t(app, "nav.about"));
    let scroll_key = if app.page == "settings" {
        format!("settings:{}", app.settings_section)
    } else {
        app.page.clone()
    };
    write!(html, r#"</nav><div class="sidebar-version">GalGear <span>v{}</span></div></div></aside><main class="main-panel"><div class="page-scroll" id="page-scroll" data-scroll-key="{}">"#, esc(&app.version),esc(&scroll_key)).unwrap();
    match app.page.as_str() {
        "home" => crate::home::render(&mut html, app),
        "settings" => settings_page(&mut html, app),
        "about" => about_page(&mut html, app),
        "patches" => patches_page(&mut html, app),
        _ => games_page(&mut html, app),
    }
    html.push_str("</div>");
    if let Some(game) = app
        .detail
        .selected
        .as_ref()
        .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
    {
        detail(&mut html, app, game);
    }
    library::dialog(&mut html, app);
    context_menu::render(&mut html, app);
    choices::popup(&mut html, app);
    game_setup::render(&mut html, app);
    crate::updates::render(&mut html, app);
    html.push_str("</main>");
    if let Some((message, error)) = &app.toast {
        write!(html, r#"<div class="toast {} {}" role="{}">{}<span>{}</span><button class="icon-button" data-action="dismiss-toast" aria-label="{}">{}</button></div>"#,
            if *error { "error" } else { "" }, if message.key.starts_with("save.ai_") || message.key.starts_with("local_patches.") || message.key.starts_with("update.") { "save-ai-toast" } else { "" }, if *error { "alert" } else { "status" },
            if matches!(message.key.as_str(), "local_patches.installing" | "local_patches.switching") { r#"<span class="network-spinner" aria-hidden="true"></span>"#.into() } else { icon(if *error { "error" } else { "check" }) }, esc(&message.render(app.settings.language)), t(app,"common.close"), icon("close")).unwrap();
    }
    network_popup(&mut html, app);
    download_notice(&mut html, app);
    html.push_str("</div>");
    render(&html);
}

pub(crate) fn nav(html: &mut String, app: &App, value: &str, label: &str) {
    let active = app.page == value && (value != "games" || app.library.collection.is_none());
    let _ = write!(
        html,
        r#"<button type="button" class="nav-item {}" data-action="page" data-value="{value}" aria-label="{label}" title="{label}" aria-current="{}">{}</button>"#,
        if active { "active" } else { "" },
        if active { "page" } else { "false" },
        icon(match value {
            "home" => "gamepad",
            "games" => "playstation-shapes",
            "collections" => "star",
            _ => value,
        })
    );
}

pub(crate) fn games_page(html: &mut String, app: &App) {
    library::page(html, app);
}

pub(crate) fn poster(html: &mut String, app: &App, game: &Game) {
    if let Some(cover) = &game.cover {
        write!(
            html,
            r#"<img src="{}" alt="{}" loading="lazy" decoding="async" draggable="false"/>"#,
            esc(cover),
            f(app, "cover.alt", &[("title", &game.title)])
        )
        .unwrap();
    } else {
        let monogram: String = game
            .title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect();
        write!(html,r#"<div class="poster-fallback"><span class="poster-orbit"></span><span class="poster-flower">{}</span><span class="poster-monogram">{}</span><span class="cover-hint">{} {}</span></div>"#,brand(),esc(&monogram.to_uppercase()),icon("image"),t(app,"cover.add")).unwrap();
    }
}

pub(crate) fn settings_page(html: &mut String, app: &App) {
    preferences::render(html, app);
}

pub(crate) fn account_label(app: &App) -> &'static str {
    t(
        app,
        if accounts::busy(&app.account, &app.requests) {
            "auth.checking"
        } else if app.account.status.status == "authenticated" {
            "auth.connected"
        } else if app.account.status.status == "unauthenticated" {
            "auth.disconnected"
        } else if app.account.status.status == "error" {
            "auth.failed"
        } else {
            "auth.unchecked"
        },
    )
}

pub(crate) fn about_page(html: &mut String, app: &App) {
    write!(html, r#"<section class="about-hero" aria-label="{}"><span class="about-logo">{}</span><h2>GalGear</h2><p>{}</p><p>Rust + Tauri</p><p><a class="button secondary" href="https://github.com/CbShuiMu/GalGear" data-action="open-resource" data-value="https://github.com/CbShuiMu/GalGear">GitHub · CbShuiMu/GalGear</a></p></section>"#, t(app,"about.title"),brand(),t(app,"about.description")).unwrap();
}

pub(crate) fn detail(html: &mut String, app: &App, game: &Game) {
    detail::render(html, app, game);
}

pub(crate) fn form_field(html: &mut String, app: &App, field: &str, label: &str, multiline: bool) {
    let value = app.form.get(field).map(String::as_str).unwrap_or_default();
    write!(
        html,
        "<label class=\"edit-field\" for=\"{field}\"><span>{}</span>",
        t(app, label)
    )
    .unwrap();
    if multiline {
        write!(
            html,
            "<textarea id=\"{field}\" data-input=\"{field}\" rows=\"2\">{}</textarea>",
            esc(value)
        )
        .unwrap();
    } else {
        write!(
            html,
            "<input id=\"{field}\" data-input=\"{field}\" value=\"{}\"/>",
            esc(value)
        )
        .unwrap();
    }
    html.push_str("</label>");
}
