//! Patch search, previews and download presentation.
use crate::{
    choices, downloads, models::InstalledPatch, models::PatchPreview, models::Resource,
    requests::Pending, requests::issue, state::App, view::f, view::form_field, view::t,
    widgets::button, widgets::bytes, widgets::esc, widgets::icon,
};
use galgear_i18n::Message;
use serde_json::json;
use std::fmt::Write;

pub(crate) struct State {
    pub(crate) patch_game: String,
    pub(crate) patch_search_game: String,
    pub(crate) patch_preview: Option<PatchPreview>,
    pub(crate) installed_patches: Vec<InstalledPatch>,
    pub(crate) installed_patches_loaded: bool,
    pub(crate) patch_target: Option<String>,
    pub(crate) resources: Vec<Resource>,
    pub(crate) resources_game: Option<String>,
    pub(crate) resource_warnings: Vec<Message>,
    pub(crate) patch_source: String,
    pub(crate) resources_searched: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            patch_game: String::new(),
            patch_search_game: String::new(),
            patch_preview: None,
            installed_patches: Vec::new(),
            installed_patches_loaded: false,
            patch_target: None,
            resources: Vec::new(),
            resources_game: None,
            resource_warnings: Vec::new(),
            patch_source: "all".into(),
            resources_searched: false,
        }
    }
}

pub(crate) fn resource_row(html: &mut String, app: &App, result: &Resource) {
    let fan = result.source == "2DFan";
    let logo = if fan {
        "./assets/2dfan.png"
    } else {
        "./assets/kun.webp"
    };
    write!(html,r#"<article class="resource-row resource-entry"><span class="resource-logo"><img src="{logo}" alt="{}" width="28" height="28"/></span><div class="resource-copy"><div class="resource-origin"><span class="resource-source">{}</span>"#,esc(&result.source),esc(&result.source)).unwrap();
    if !result.game_title.is_empty() {
        write!(
            html,
            "<span class=\"resource-game\">{}</span>",
            esc(&result.game_title)
        )
        .unwrap();
    }
    write!(
        html,
        "</div><h3>{}</h3><div class=\"resource-meta\">",
        esc(&result.title)
    )
    .unwrap();
    for tag in &result.tags {
        let (label, class) = match tag.as_str() {
            "ai" | "AI翻译" => (t(app, "patches.tag_ai"), "ai"),
            "manual" => (t(app, "patches.tag_manual"), ""),
            "全CG存档" => (tag.as_str(), "save"),
            _ => (tag.as_str(), ""),
        };
        write!(
            html,
            "<span class=\"resource-tag {class}\">{}</span>",
            esc(label)
        )
        .unwrap();
    }
    if !result.author.is_empty() {
        write!(
            html,
            "<span class=\"resource-author\" title=\"{}\">{}</span>",
            f(app, "patches.uploaded_by", &[("name", &result.author)]),
            esc(&result.author)
        )
        .unwrap();
    }
    if !result.uploaded_at.is_empty() {
        write!(
            html,
            "<time datetime=\"{}\" title=\"{}\">{}</time>",
            esc(&result.uploaded_at),
            esc(&result.uploaded_at),
            esc(result.uploaded_at.split('T').next().unwrap_or_default())
        )
        .unwrap();
    }
    if !result.size.is_empty() {
        write!(html, "<span>{}</span>", esc(&result.size)).unwrap();
    }
    if result.tags.is_empty()
        && result.author.is_empty()
        && result.game_title.is_empty()
        && !result.description.is_empty()
    {
        write!(html, "<span>{}</span>", esc(&result.description)).unwrap();
    }
    let free = app
        .downloads
        .preview
        .as_ref()
        .is_some_and(|p| p.source_url == result.url && p.free);
    if free {
        write!(
            html,
            "<span class=\"resource-tag free\">{}</span>",
            t(app, "download.free_cost")
        )
        .unwrap();
    }
    write!(
        html,
        "</div></div><div class=\"resource-actions\">{}{}</div></article>",
        if fan && result.url.contains("/downloads/") {
            button(
                "inspect-download",
                &result.url,
                t(app, "download.inspect"),
                "arrow",
                "secondary",
                downloads::busy(&app.requests),
            )
        } else {
            String::new()
        },
        button(
            "open-resource",
            &result.url,
            t(app, "patches.open"),
            "arrow",
            "quiet",
            false
        )
    )
    .unwrap();
}

pub(crate) fn patches_page(html: &mut String, app: &App) {
    write!(html,r#"<div class="patch-workspace" role="region" aria-label="{}"><div class="patch-browser"><section class="resource-search"><div class="patch-controls">"#,t(app,"patches.title")).unwrap();
    form_field(html, app, "patch-query", "patches.query", false);
    html.push_str(&button(
        "search-patches",
        "",
        t(app, "patches.search"),
        "search",
        "primary",
        app.requests
            .values()
            .any(|p| matches!(p, Pending::Resources(_))),
    ));
    html.push_str(&button(
        "page",
        "settings",
        t(app, "auth.title"),
        "settings",
        "quiet",
        false,
    ));
    html.push_str("</div><div class=\"library-search-row\">");
    write!(html, "<span>{}</span>", t(app, "patches.library_search")).unwrap();
    choices::control(
        html,
        app,
        "patch-search-game",
        t(app, "patches.library_search"),
        app.requests
            .values()
            .any(|p| matches!(p, Pending::Resources(_))),
    );
    write!(
        html,
        "</div><div class=\"resource-toolbar\"><div class=\"source-filters\">"
    )
    .unwrap();
    for (source, label) in [
        ("all", t(app, "patches.all_sources")),
        ("2DFan", "2DFan"),
        ("kun", t(app, "patches.kun")),
    ] {
        let count = app
            .patches
            .resources
            .iter()
            .filter(|r| {
                source == "all"
                    || (source == "2DFan" && r.source == "2DFan")
                    || (source == "kun" && r.source != "2DFan")
            })
            .count();
        write!(html,r#"<button class="source-filter {}" data-action="patch-source" data-value="{source}" aria-pressed="{}">{}<span>{count}</span></button>"#,if app.patches.patch_source==source{"selected"}else{""},app.patches.patch_source==source,label).unwrap();
    }
    html.push_str("</div></div></section>");
    for warning in &app.patches.resource_warnings {
        write!(
            html,
            "<p class=\"account-notice\">{}</p>",
            esc(&warning.render(app.settings.language))
        )
        .unwrap();
    }
    download_panel(html, app);
    html.push_str("<div class=\"resource-list\">");
    let results: Vec<_> = app
        .patches
        .resources
        .iter()
        .filter(|r| {
            app.patches.patch_source == "all"
                || (app.patches.patch_source == "2DFan" && r.source == "2DFan")
                || (app.patches.patch_source == "kun" && r.source != "2DFan")
        })
        .collect();
    for result in &results {
        resource_row(html, app, result);
    }
    if results.is_empty() {
        let searching = app
            .requests
            .values()
            .any(|p| matches!(p, Pending::Resources(_)));
        write!(html,r#"<div class="resource-empty"><span class="resource-empty-icon">{}</span><h3>{}</h3><p>{}</p></div>"#,icon("search"),t(app,if searching{"patches.searching"}else if app.patches.resources_searched{"patches.no_results"}else{"patches.find_resources"}),t(app,if app.patches.resources_searched && !searching{"patches.try_query"}else{"patches.resource_help"})).unwrap();
    }
    html.push_str("</div></div>");
    html.push_str("<aside class=\"patch-install-placeholder\" aria-hidden=\"true\"></aside></div>");
}

pub(crate) fn download_notice(html: &mut String, app: &App) {
    let Some(transfer) = &app.downloads.notice else {
        return;
    };
    write!(html,r#"<aside class="download-notice" role="status" aria-live="polite"><div class="download-notice-heading"><span>{}<strong>{}</strong></span><button class="icon-button" data-action="dismiss-download-notice" aria-label="{}">{}</button></div><p title="{}">{}</p><div class="download-notice-actions">{}</div></aside>"#,icon("check"),t(app,"download.complete"),t(app,"common.close"),icon("close"),esc(&transfer.title),esc(&transfer.title),button("open-download-result","",t(app,"download.open_folder"),"folder","quiet",app.busy())).unwrap();
}

pub(crate) fn download_panel(html: &mut String, app: &App) {
    if let Some(preview) = &app.downloads.preview {
        write!(html, r#"<section class="download-inspection"><div class="panel-heading"><h2>{}</h2><button class="icon-button" data-action="close-download" aria-label="{}" {}>{}</button></div><h3>{}</h3><dl class="download-facts"><div><dt>{}</dt><dd>{}</dd></div><div><dt>{}</dt><dd>{}</dd></div></dl>"#,
            t(app,"download.preview"),t(app,"common.close"),if downloads::busy(&app.requests){"disabled"}else{""},icon("close"),esc(&preview.title),t(app,"download.cost"),if preview.free {t(app,"download.free_cost").into()}else if preview.can_download {t(app,"download.available").into()}else{preview.price.map(|n|f(app,"download.points",&[("count",&n.to_string())])).unwrap_or_else(||t(app,"auth.unknown").into())},t(app,"auth.points"),preview.balance.map(|n|f(app,"download.points",&[("count",&n.to_string())])).unwrap_or_else(||t(app,"auth.unknown").into())).unwrap();
        if let Some(reason) = &preview.reason {
            write!(
                html,
                "<p class=\"account-notice\">{}</p>",
                esc(&reason.render(app.settings.language))
            )
            .unwrap();
        }
        write!(
            html,
            "<p class=\"field-help directory-value\">{}: {}</p><div class=\"detail-actions\">",
            t(app, "download.folder"),
            esc(&app.downloads.preview_folder)
        )
        .unwrap();
        if preview.requires_purchase {
            let label = preview
                .price
                .map(|n| f(app, "download.purchase", &[("count", &n.to_string())]))
                .unwrap_or_else(|| t(app, "download.confirm_required").into());
            html.push_str(&button(
                "purchase-download",
                "",
                &label,
                "check",
                "primary",
                !preview.can_purchase || downloads::busy(&app.requests),
            ));
        } else {
            html.push_str(&button(
                "download-free",
                "",
                t(
                    app,
                    if preview.free {
                        "download.free_start"
                    } else {
                        "download.start"
                    },
                ),
                "arrow",
                "primary",
                !preview.can_download || downloads::busy(&app.requests),
            ));
        }
        html.push_str(&button(
            "open-resource",
            &preview.source_url,
            t(app, "patches.open"),
            "arrow",
            "quiet",
            false,
        ));
        if !preview.can_download && !preview.can_purchase {
            html.push_str(&button(
                "page",
                "settings",
                t(app, "auth.configure"),
                "settings",
                "secondary",
                false,
            ));
        }
        html.push_str("</div></section>");
    }
    if let Some(transfer) = app
        .downloads
        .transfer
        .as_ref()
        .filter(|t| t.phase != "complete")
    {
        let active = matches!(
            transfer.phase.as_str(),
            "preparing" | "downloading" | "purchasing" | "extracting"
        );
        write!(html,r#"<section class="transfer-panel" aria-live="polite"><div class="panel-heading"><h2>{}</h2>{}</div><p class="transfer-title">{}</p>"#,
            t(app,match transfer.phase.as_str(){"complete"=>"download.complete","error"=>"download.failed","purchasing"=>"download.purchasing","downloading"=>"download.downloading","extracting"=>"download.extracting",_=>"download.preparing"}),
            if active {button("cancel-download","",t(app,"download.cancel"),"close","quiet",transfer.phase=="purchasing")}else{button("open-download-result","",t(app,"download.open_folder"),"folder","quiet",false)},esc(&transfer.title)).unwrap();
        if active {
            let progress = transfer
                .total
                .filter(|v| *v > 0)
                .map(|v| (transfer.received * 100 / v).min(100));
            write!(html,r#"<progress max="100" {} aria-label="{}"></progress><p class="field-help">{} / {}</p>"#,progress.map(|v|format!("value=\"{v}\"")).unwrap_or_default(),t(app,"download.downloading"),bytes(transfer.received),transfer.total.map(bytes).unwrap_or_else(||t(app,"auth.unknown").into())).unwrap();
        }
        if let Some(path) = &transfer.path {
            write!(
                html,
                "<p class=\"directory-value\">{}</p>",
                esc(path)
            )
            .unwrap();
        }
        if let Some(error) = &transfer.error {
            write!(
                html,
                "<p class=\"account-notice\" role=\"alert\">{}</p>",
                esc(&error.render(app.settings.language))
            )
            .unwrap();
        }
        html.push_str("</section>");
    }
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> Option<bool> {
    match action {
        "patch-source" if matches!(value, "all" | "2DFan" | "kun") => {
            app.patches.patch_source = value.into()
        }
        "use-download" if !app.patches.patch_game.is_empty() => {
            let id = app.patches.patch_game.clone();
            let target = app.patches.patch_target.clone();
            issue(
                app,
                Pending::PatchPreview,
                "preview_downloaded_patch",
                json!({"id":id,"target":target}),
            );
        }
        "exe-status" => issue(
            app,
            Pending::PatchList,
            "set_exe_status",
            json!({"id":value,"installed":true}),
        ),
        "search-patches"
            if !app
                .requests
                .values()
                .any(|p| matches!(p, Pending::Resources(_))) =>
        {
            app.patches.resources_searched = true;
            let query = app.form.get("patch-query").cloned().unwrap_or_default();
            issue(
                app,
                Pending::Resources(
                    app.catalog
                        .games
                        .iter()
                        .find(|game| {
                            game.id == app.patches.patch_search_game && game.title == query
                        })
                        .map(|game| game.id.clone()),
                ),
                "search_patches",
                json!({"query":query}),
            );
        }
        "patch-search-game"
            if !app
                .requests
                .values()
                .any(|p| matches!(p, Pending::Resources(_))) =>
        {
            app.patches.patch_search_game = value.into();
            if let Some(title) = app
                .catalog
                .games
                .iter()
                .find(|g| g.id == value)
                .map(|g| g.title.clone())
            {
                app.form.insert("patch-query".into(), title.clone());
                app.patches.resources_searched = true;
                issue(
                    app,
                    Pending::Resources(Some(value.into())),
                    "search_patches",
                    json!({"query":title}),
                );
            }
        }
        "patch-game" => {
            app.patches.patch_game = value.into();
            app.patches.patch_preview = None;
            app.patches.patch_target = None;
        }
        "patch-target" => {
            let id = app.patches.patch_game.clone();
            let target = app.patches.patch_target.clone();
            issue(
                app,
                Pending::PatchTarget,
                "choose_patch_target",
                json!({"id":id,"target":target}),
            );
        }
        "choose-patch" => {
            let id = app.patches.patch_game.clone();
            let target = app.patches.patch_target.clone();
            issue(
                app,
                Pending::PatchPreview,
                "preview_patch",
                json!({"id":id,"target":target}),
            );
        }
        "install-patch" => issue(app, Pending::PatchList, "install_patch", json!({})),
        "restore-patch" => issue(
            app,
            Pending::PatchList,
            "restore_patch",
            json!({"id":value}),
        ),
        "run-patch" => issue(app, Pending::PatchList, "run_patch_installer", json!({})),
        _ => return None,
    }
    Some(true)
}
