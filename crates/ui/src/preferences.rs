//! Section navigation keeps each settings task in its own reading pane.
use super::*;

pub(super) const SOURCES: [(&str, &str); 4] = [
    ("2dfan", "2DFan"),
    ("vndb", "VNDB"),
    ("bangumi", "Bangumi"),
    ("ymgal", "月幕 Galgame"),
];
#[derive(Clone, Default, Deserialize)]
pub(super) struct SourceStatus {
    pub source: String,
    pub configured: bool,
}
pub(super) fn source_for_url(url: &str) -> (&'static str, &'static str) {
    let source = if url.starts_with("https://vndb.org/") {
        "vndb"
    } else if url.starts_with("https://bgm.tv/") || url.starts_with("https://bangumi.tv/") {
        "bangumi"
    } else if url.starts_with("https://www.ymgal.games/") {
        "ymgal"
    } else {
        "2dfan"
    };
    SOURCES
        .into_iter()
        .find(|(id, _)| *id == source)
        .unwrap_or(SOURCES[0])
}

pub(super) fn render(html: &mut String, app: &App) {
    write!(
        html,
        r#"<div class="preferences-workspace"><nav class="preferences-nav" aria-label="{}">"#,
        t(app, "settings.title")
    )
    .unwrap();
    for (section, key) in [
        ("account", "settings.section_account"),
        ("library", "settings.section_library"),
        ("display", "settings.section_display"),
        ("system", "settings.section_system"),
        ("paths", "settings.section_paths"),
        ("about", "settings.section_update"),
    ] {
        write!(html,r#"<button class="preference-tab {}" data-action="settings-section" data-value="{}" aria-current="{}">{}</button>"#,if app.settings_section==section{"active"}else{""},section,if app.settings_section==section{"page"}else{"false"},t(app,key)).unwrap();
    }
    html.push_str("</nav><div class=\"preferences-content\">");
    match app.settings_section.as_str() {
        "library" => library(html, app),
        "display" => display(html, app),
        "system" => system(html, app),
        "paths" => paths(html, app),
        "about" => about(html, app),
        _ => account(html, app),
    }
    html.push_str("</div></div>");
}
fn heading(html: &mut String, app: &App, key: &str) {
    write!(
        html,
        r#"<header class="preference-page-heading"><h1>{}</h1></header>"#,
        t(app, key)
    )
    .unwrap();
}
fn account(html: &mut String, app: &App) {
    heading(html, app, "settings.section_account");
    write!(html,r#"<section class="account-panel"><div class="panel-heading"><h2>{}</h2><span class="connection-status {}">{}</span></div><div class="account-summary"><div><strong>{}</strong><small>{}</small></div><div class="account-balance"><span>{}</span><strong>{}</strong></div></div>"#,t(app,"auth.title"),if app.account.status=="authenticated"{"connected"}else{""},account_label(app),esc(app.account.username.as_deref().unwrap_or(t(app,"auth.account"))),esc(&app.settings.fan_domain),t(app,"auth.points"),app.account.balance.map(|balance|balance.to_string()).unwrap_or_else(||t(app,"auth.unknown").into())).unwrap();
    if let Some(reason) = &app.account.reason {
        write!(
            html,
            "<p class=\"account-notice\" role=\"status\">{}</p>",
            esc(&reason.render(app.settings.language))
        )
        .unwrap();
    }
    write!(html,r#"<label class="edit-field" for="fan-token"><span>Token</span><input type="password" id="fan-token" data-input="fan-token" value="{}" autocomplete="off" spellcheck="false" placeholder="{}"/></label><details class="session-fields"><summary>{}</summary><p class="field-help">{}</p><label class="edit-field" for="fan-cookie"><span>Cookie</span><input type="password" id="fan-cookie" data-input="fan-cookie" value="{}" autocomplete="off" spellcheck="false" placeholder="_session=…"/></label><label class="edit-field" for="fan-mode"><span>{}</span>{}</label></details><div class="account-actions">{}{}</div><p class="field-help vault-note">{}</p>"#,
        esc(&app.credentials.token),t(app,"auth.token_placeholder"),t(app,"auth.session"),t(app,"auth.session_help"),esc(&app.credentials.cookie),t(app,"auth.token_type"),{let mut control=String::new();choices::control(&mut control,app,"fan-mode",t(app,"auth.token_type"),app.busy||account_busy(app)||download_busy(app));control},button("save-fan","",t(app,if account_busy(app){"auth.checking"}else{"auth.save_check"}),"check","primary",app.busy||account_busy(app)||download_busy(app)),button("clear-fan","",t(app,"auth.clear"),"close","quiet",app.busy||account_busy(app)||download_busy(app)),t(app,"auth.vault")).unwrap();
    write!(
        html,
        "<p class=\"field-help\">{}</p>",
        t(app, "auth.cache_help")
    )
    .unwrap();
    if let Some(error) = &app.fan_cache_error {
        write!(
            html,
            "<p class=\"account-notice\">{} {}</p>",
            t(app, "auth.cache_error"),
            esc(&error.render(app.settings.language))
        )
        .unwrap();
    }

    html.push_str("</section>");
    for (source, name) in SOURCES.into_iter().filter(|(source, _)| *source != "2dfan") {
        let configured = app
            .data_sources
            .iter()
            .any(|status| status.source == source && status.configured);
        write!(html,r#"<section class="settings-card source-card" data-render-key="source-{}"><div class="panel-heading"><h2><img src="assets/{}.ico" alt=""/>{}</h2><span class="connection-status {}">{}</span></div>"#,source,source,name,if configured{"connected"}else{""},t(app,if configured{"sources.saved"}else{"sources.public"})).unwrap();
        if source == "ymgal" {
            write!(
                html,
                "<p class=\"field-help\">{}</p>",
                t(app, "sources.ymgal_help")
            )
            .unwrap();
            for (field, label, password) in [
                ("client-id", "Client ID", false),
                ("client-secret", "Client Secret", true),
            ] {
                source_field(html, app, source, field, label, password, configured);
            }
        } else {
            source_field(html, app, source, "token", "Token", true, configured);
        }
        write!(
            html,
            "<div class=\"account-actions\">{}{}</div></section>",
            button(
                "source-save",
                source,
                t(app, "auth.save_check"),
                "check",
                "primary",
                app.busy
            ),
            button(
                "source-clear",
                source,
                t(app, "auth.clear"),
                "close",
                "quiet",
                app.busy
            )
        )
        .unwrap();
    }
    write!(html,"<section class=\"settings-card\"><h2>{}</h2><p class=\"field-help\">{}</p><div class=\"metadata-provider-picker\">",t(app,"sources.default"),t(app,"sources.default_help")).unwrap();
    for (source, name) in SOURCES {
        html.push_str(&button(
            "source-default",
            source,
            name,
            "",
            if app.settings.metadata_provider == source {
                "primary"
            } else {
                "quiet"
            },
            app.busy,
        ));
    }
    html.push_str("</div></section>");
}

fn source_field(
    html: &mut String,
    app: &App,
    source: &str,
    field: &str,
    label: &str,
    password: bool,
    saved: bool,
) {
    let key = format!("source-{source}-{field}");
    write!(html,r#"<label class="edit-field" for="{}"><span>{}</span><input id="{}" data-input="{}" type="{}" value="{}" autocomplete="off" spellcheck="false" placeholder="{}"/></label>"#,key,label,key,key,if password{"password"}else{"text"},esc(&features::form(app,&key)),t(app,if saved{"sources.keep_saved"}else{"sources.optional"})).unwrap();
}
fn library(html: &mut String, app: &App) {
    heading(html, app, "settings.section_library");
    write!(html,r#"<section class="settings-card"><div class="panel-heading"><div><h2>{}</h2><p>{}</p></div>{}</div>"#,t(app,"settings.folders"),t(app,"settings.folders_help"),button("choose-folder","",t(app,"settings.add_folder"),"plus","primary",app.busy)).unwrap();
    for path in &app.settings.library_folders {
        write!(html,r#"<div class="folder-row"><span class="folder-symbol">{}</span><div><strong>{}</strong></div>{}</div>"#,icon("folder"),esc(path),button("remove-folder",path,t(app,"settings.remove_folder"),"close","quiet",app.busy)).unwrap();
    }
    if app.settings.library_folders.is_empty() {
        write!(html, "<p>{}</p>", t(app, "settings.no_folders")).unwrap();
    }
    write!(html,r#"<div class="path-entry"><input id="library-path" data-input="library-path" value="{}" placeholder="{}" aria-label="{}"/>{}</div></section>"#,esc(&app.path_input),t(app,"settings.path_placeholder"),t(app,"settings.path_label"),button("add-folder","",t(app,"common.add"),"plus","secondary",app.busy)).unwrap();
    html.push_str(&button(
        "excluded-open",
        "",
        t(app, "library.restore"),
        "refresh",
        "secondary",
        app.busy,
    ));
}
fn display(html: &mut String, app: &App) {
    heading(html, app, "settings.section_display");
    html.push_str("<section class=\"settings-card\">");
    write!(
        html,
        r#"<div class="preference-row"><h3>{}</h3><div class="segmented-control">"#,
        t(app, "settings.language")
    )
    .unwrap();
    for (language, key) in [
        (Language::Chinese, "settings.language_zh"),
        (Language::Japanese, "settings.language_ja"),
        (Language::English, "settings.language_en"),
    ] {
        segment(
            html,
            app,
            "language",
            language.code(),
            key,
            app.settings.language == language,
        );
    }
    write!(html,r#"</div></div><div class="preference-row"><h3>{}</h3>{}</div><div class="preference-row"><h3>{}</h3><div class="segmented-control">"#,t(app,"settings.theme"),button("toggle-theme","",t(app,if app.settings.theme=="dark"{"theme.dark"}else{"theme.light"}),"dark_mode","secondary",app.busy),t(app,"display.font")).unwrap();
    for size in [14, 16, 18] {
        html.push_str(&button(
            "font-size",
            &size.to_string(),
            &format!("{size}px"),
            "",
            if app.settings.font_size == size {
                "primary"
            } else {
                "quiet"
            },
            app.busy,
        ));
    }
    write!(
        html,
        "</div></div><p>{}</p></section>",
        t(app, "display.help")
    )
    .unwrap();
}
fn system(html: &mut String, app: &App) {
    heading(html, app, "settings.section_system");
    html.push_str("<section class=\"settings-card\">");
    for (label, action, selected, options) in [
        (
            "settings.launch_behavior",
            "launch-behavior",
            app.settings.launch_behavior.as_str(),
            vec![
                ("keep", "settings.keep"),
                ("minimize", "settings.minimize"),
                ("exit", "settings.exit"),
            ],
        ),
        (
            "settings.close_behavior",
            "close-behavior",
            app.settings.close_behavior.as_str(),
            vec![("exit", "settings.exit"), ("tray", "settings.tray")],
        ),
    ] {
        write!(
            html,
            r#"<div class="preference-row"><h3>{}</h3><div class="segmented-control">"#,
            t(app, label)
        )
        .unwrap();
        for (value, key) in options {
            segment(html, app, action, value, key, selected == value);
        }
        html.push_str("</div></div>");
    }
    write!(html,r#"</section><section class="settings-card"><h2>{}</h2><p>{}</p><div class="preference-row"><h3>{}</h3><button id="patch-tab-toggle" class="preference-switch" role="switch" aria-label="{}" aria-checked="{}" data-action="patch-tab" data-value="{}" {}><span></span></button></div></section>"#,t(app,"settings.navigation"),t(app,"settings.navigation_help"),t(app,"nav.patches"),t(app,"settings.patch_tab"),app.settings.show_patches,!app.settings.show_patches,if app.busy{"disabled"}else{""}).unwrap();
}
fn paths(html: &mut String, app: &App) {
    heading(html, app, "settings.section_paths");
    write!(html,r#"<section class="settings-card"><div class="panel-heading"><h2>{}</h2>{}</div><p class="directory-value">{}</p>{}</section><section class="settings-card"><h2>{}</h2><div class="edit-grid">"#,t(app,"download.folder"),button("download-folder","",t(app,"download.change_folder"),"folder","quiet",app.busy),esc(&download_folder(app)),button("open-downloads","",t(app,"download.open_folder"),"folder","secondary",app.busy),t(app,"tools.title")).unwrap();
    for (key, label, fallback) in [
        (
            "tool-locale",
            "tools.locale",
            app.settings.locale_emulator.clone(),
        ),
        ("tool-magpie", "tools.magpie", app.settings.magpie.clone()),
        (
            "tool-hotkey",
            "tools.hotkey",
            app.settings.magpie_hotkey.clone(),
        ),
        (
            "backup-folder",
            "backup.folder",
            app.settings.backup_folder.clone(),
        ),
        (
            "backup-retention",
            "backup.retention",
            app.settings.backup_retention.to_string(),
        ),
    ] {
        write!(html,r#"<label class="edit-field" for="{}"><span>{}</span><input id="{}" data-input="{}" value="{}"/></label>"#,key,t(app,label),key,key,esc(&features::setting(app,key,&fallback))).unwrap();
    }
    write!(
        html,
        "</div><p>{}</p>{}</section><p class=\"config-note\">{}<br>{}</p>",
        t(app, "tools.help"),
        button(
            "configure-tools",
            "",
            t(app, "common.save"),
            "check",
            "primary",
            app.busy
        ),
        t(app, "settings.config_path"),
        esc(&app.config_path)
    )
    .unwrap();
}
fn about(html: &mut String, app: &App) {
    heading(html, app, "settings.section_update");
    write!(html,r#"<section class="settings-card"><div class="panel-heading"><h2>GalGear</h2><span class="version-pill">v{}</span></div>"#,esc(&app.version)).unwrap();
    write!(html,r#"<label class="edit-field" for="release-page"><span>{}</span><input id="release-page" data-input="release-page" value="{}" placeholder="https://github.com/owner/GalGear/releases"/></label>"#,t(app,"settings.release_page"),esc(&features::setting(app,"release-page",&app.settings.release_page))).unwrap();
    write!(
        html,
        r#"<div class="detail-actions">{}{}</div>"#,
        button(
            "save-release",
            "",
            t(app, "common.save"),
            "check",
            "secondary",
            app.busy
        ),
        button(
            "check-update",
            "",
            t(app, "settings.check_update"),
            "refresh",
            "quiet",
            app.busy
        )
    )
    .unwrap();
    if let Some((version, url)) = &app.update {
        write!(
            html,
            "<p>{}</p>{}",
            f(app, "settings.latest", &[("version", version)]),
            button(
                "open-resource",
                url,
                t(app, "settings.download"),
                "arrow",
                "secondary",
                app.busy
            )
        )
        .unwrap();
    }
    html.push_str("</section>");
}
