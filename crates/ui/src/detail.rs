//! Reference-led detail reading page, with cached credits and compact utilities.
use crate::{
    context_menu, cover_picker, features, local_patches, models::Game, models::GameEdit,
    models::Metadata, models::Resource, models::RestorePreview, models::SaveReport,
    models::SaveSnapshot, models::default_source, playtime, preferences, requests::Pending,
    state::App, view::form_field, view::poster, view::t, widgets::button, widgets::esc,
    widgets::icon,
};
use galgear_i18n::Message;
use std::{collections::BTreeMap, fmt::Write};
pub(crate) struct State {
    pub(crate) selected: Option<String>,
    pub(crate) history_open: bool,
    pub(crate) archive_open: bool,
    pub(crate) current_snapshot: Option<String>,
    pub(crate) history: Vec<SaveSnapshot>,
    pub(crate) history_game: Option<String>,
    pub(crate) restore_preview: Option<RestorePreview>,
    pub(crate) confirmation: Option<(String, String)>,
    pub(crate) saves: BTreeMap<String, SaveReport>,
    pub(crate) candidates: BTreeMap<String, Vec<Resource>>,
    pub(crate) metadata_source: String,
    pub(crate) metadata_warnings: BTreeMap<String, Message>,
    pub(crate) editor: Option<Editor>,
    pub(crate) editing: bool,
    pub(crate) editing_configuration: bool,
    pub(crate) tag_draft: Option<crate::detail_tags::Draft>,
    pub(crate) title_refreshing: std::collections::BTreeSet<u32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            selected: None,
            history_open: false,
            archive_open: false,
            current_snapshot: None,
            history: Vec::new(),
            history_game: None,
            restore_preview: None,
            confirmation: None,
            saves: BTreeMap::new(),
            candidates: BTreeMap::new(),
            metadata_source: default_source(),
            metadata_warnings: BTreeMap::new(),
            editor: None,
            editing: false,
            editing_configuration: false,
            tag_draft: None,
            title_refreshing: Default::default(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Editor {
    Association,
    TagSource,
    Tools,
    Cover,
}

fn editor(html: &mut String, app: &App, game: &Game) {
    match app.detail.editor {
        None => return,
        Some(Editor::Tools) => {
            tools_dialog(html, app, game);
            return;
        }
        Some(Editor::Cover) => {
            cover_picker::render(html, app, game);
            return;
        }
        Some(Editor::Association | Editor::TagSource) => {}
    }
    let tags = matches!(app.detail.editor, Some(Editor::TagSource));
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog association-dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title"><header><h2 id="editor-title">{}</h2>{}</header><div class="editor-body"><div class="association-search">"#,t(app,if tags { "detail.change_tag_source" } else { "detail.change_association" }),button("editor-close","",t(app,"common.close"),"close","quiet",false)).unwrap();
    form_field(html, app, "metadata-query", "metadata.query", false);
    html.push_str(&button(
        "metadata-search",
        &game.id,
        t(app, "metadata.search"),
        "search",
        "secondary",
        app.busy()
            || app
                .requests
                .values()
                .any(|pending| matches!(pending, Pending::SearchMetadata(..))),
    ));
    html.push_str("</div><div class=\"metadata-provider-picker\">");
    preferences::source_picker(
        html,
        app,
        "metadata-provider",
        &app.detail.metadata_source,
        app.busy(),
    );
    html.push_str("</div>");
    if let Some(results) = app.detail.candidates.get(&game.id) {
        for result in results {
            write!(
                html,
                r#"<div class="candidate-row association-candidate"><span title="{}">{}</span><button type="button" class="button secondary association-select" data-action="{}" data-value="{}" aria-label="{}" title="{}" {}>{}</button></div>"#,
                esc(&result.title),
                esc(&result.title),
                if tags { "tags-select" } else { "metadata-select" },
                esc(&result.url),
                t(app, "metadata.select"),
                t(app, "metadata.select"),
                if app.busy() { "disabled" } else { "" },
                icon("check")
            )
            .unwrap();
        }
    }
    html.push_str("</div></section></div>");
}

fn value<'a>(app: &App, text: &'a str) -> &'a str
where
    App: 'a,
{
    if text.is_empty() {
        t(app, "detail.not_set")
    } else {
        text
    }
}
fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn credit(html: &mut String, meta: &Metadata, field: &str, text: &str) {
    if !galgear_tags::CREDIT_FIELDS.contains(&field) {
        html.push_str(&esc(text));
        return;
    }
    let names = galgear_tags::credit_names(
        text,
        meta.links
            .iter()
            .filter(|link| link.field == field)
            .map(|link| link.name.as_str()),
    );
    for name in names {
        html.push_str(&button(
            "detail-tag",
            &name,
            &esc(&name),
            "",
            "tag-button credit-tag",
            false,
        ));
    }
}

pub(crate) fn render(html: &mut String, app: &App, game: &Game) {
    let edit = app
        .settings
        .game_edits
        .get(&game.id)
        .cloned()
        .unwrap_or_default();
    let runtime = app
        .runtime
        .get(&game.id)
        .map(String::as_str)
        .unwrap_or("unknown");
    write!(html,r#"<div class="detail-overlay reference-detail {}" data-render-key="detail-{}" data-detail-origin="{}" data-detail-game="{}"><section class="detail-panel" aria-labelledby="detail-title"><div class="detail-scroll" data-render-key="detail-scroll"><div class="detail-back-row"><button id="close-detail" class="back-library" data-action="close-detail">{}<span>{}</span></button><div class="detail-edit-actions">"#,if app.detail.editing && !app.detail.editing_configuration{"editing"}else{""},esc(&game.id),esc(&app.page),esc(&game.id),icon("arrow"),t(app,match app.page.as_str() { "home" => "detail.back_home", "collections" => "detail.back_collections", _ => "detail.back" })).unwrap();
    if app.detail.editing && !app.detail.editing_configuration {
        html.push_str(&button(
            "detail-rematch",
            "",
            t(app, "detail.rematch"),
            "database",
            "quiet icon-only",
            app.busy() || app.setup.open,
        ));
        html.push_str(&button(
            "detail-cancel",
            "",
            t(app, "common.cancel"),
            "close",
            "quiet",
            app.busy(),
        ));
    }
    if edit.ai_summary {
        html.push_str(&button(
            "summary-run",
            &game.id,
            t(app, "summary.title"),
            "sparkle",
            "secondary ai-summary-button",
            app.busy()
                || !app.ai_available
                || !game.available
                || crate::ai_summary::busy(app, &game.id),
        ));
        html.push_str(&button(
            "route-map-open",
            &game.id,
            t(
                app,
                if crate::ai_summary::route_busy(app, &game.id) {
                    "route.generating"
                } else {
                    "route.title"
                },
            ),
            "open_in_new",
            "secondary ai-route-map-button",
            app.busy()
                || !app.ai_available
                || !game.available
                || crate::ai_summary::busy(app, &game.id),
        ));
    }
    html.push_str(&button(
        if app.detail.editing && !app.detail.editing_configuration {
            "detail-save"
        } else {
            "detail-edit"
        },
        &game.id,
        t(
            app,
            if app.detail.editing && !app.detail.editing_configuration {
                "common.save"
            } else {
                "detail.edit_configuration"
            },
        ),
        if app.detail.editing && !app.detail.editing_configuration {
            "check"
        } else {
            "edit"
        },
        "primary detail-edit-toggle",
        app.busy(),
    ));
    html.push_str(
        "</div></div><div class=\"detail-hero\"><div class=\"detail-cover poster poster-1\">",
    );
    poster(html, app, game);
    write!(html, r#"<button type="button" class="cover-change" data-action="cover-picker" data-value="{}" aria-label="{}" {}>{}<span>{}</span></button>"#,esc(&game.id),t(app,"detail.cover"),if app.busy(){"disabled"}else{""},icon("image"),t(app,"detail.cover")).unwrap();
    html.push_str("</div><div class=\"detail-summary\"><div class=\"detail-heading\">");
    if !game.available {
        write!(
            html,
            r#"<span class="version-pill unavailable">{}</span>"#,
            t(app, "games.unavailable")
        )
        .unwrap();
    }
    if !game.play_status.is_empty() && !matches!(game.play_status.as_str(), "unmarked" | "finished")
    {
        write!(
            html,
            r#"<span class="game-play-status {}">{}</span>"#,
            esc(&game.play_status),
            context_menu::label(app, &game.play_status)
        )
        .unwrap();
    }
    if app.detail.editing && !app.detail.editing_configuration {
        html.push_str("<div id=\"detail-title\">");
        form_field(html, app, "draft-title", "edit.name", false);
        html.push_str("</div>");
    } else {
        write!(html, r#"<h2 id="detail-title">{}</h2>"#, esc(&game.title)).unwrap();
    }
    write!(
        html,
        "<dl class=\"hero-facts\"><div><dt>{}</dt><dd>",
        t(app, "metadata.brand")
    )
    .unwrap();
    if let Some(meta) = &game.metadata {
        // 顶部与制作资料共用当前品牌，不能被旧资料源的摘要快照覆盖。
        if meta.brand.is_empty() {
            html.push_str(t(app, "detail.not_set"));
        } else {
            credit(html, meta, "brand", &meta.brand);
        }
    } else {
        html.push_str(t(app, "detail.not_set"));
    }
    let release_date = edit.summary_release_date.as_deref().unwrap_or_else(|| {
        game.metadata
            .as_ref()
            .map_or("", |meta| meta.release_date.as_str())
    });
    let last_played = game
        .last_played
        .map(|seconds| playtime::recent_date(app.settings.language, seconds))
        .filter(|date| !date.is_empty());
    write!(
        html,
        r#"</dd></div><div><dt>{}</dt><dd>{}</dd></div><div class="game-playtime"><dt>{}</dt><dd>{}</dd></div><div class="last-played"><dt>{}</dt><dd>{}</dd></div></dl>"#,
        t(app, "metadata.date"),
        esc(value(app, release_date)),
        t(app, "detail.playtime_label"),
        playtime::duration(
            app.settings.language,
            game.playtime_seconds,
            playtime::DurationStyle::Full
        ),
        t(app, "detail.last_played"),
        last_played.as_deref().unwrap_or(t(app, "detail.never_played"))
    )
    .unwrap();
    html.push_str("</div><div class=\"detail-actions hero-actions\">");
    {
        html.push_str(&button(
            "launch",
            &game.id,
            t(app, "detail.launch"),
            "play",
            "primary game-launch-button",
            app.busy() || !game.available || runtime == "running",
        ));
    }
    html.push_str("<div class=\"detail-tool-actions\">");
    crate::detail_tools::shortcuts(html, app, game, &edit);
    html.push_str("<div class=\"detail-state-actions\">");
    crate::game_actions::render(html, app, game);
    html.push_str("</div></div>");
    write!(
        html,
        "</div><section class=\"hero-tags\" aria-label=\"{}\"><div class=\"game-tags\" data-tag-reorder=\"{}\">",
        t(app, "detail.tags"),
        !app.busy() && !app.detail.editing && app.detail.editor.is_none()
            && app.detail.tag_draft.is_none() && app.detail.confirmation.is_none()
    )
    .unwrap();
    if app.detail.editing && !app.detail.editing_configuration {
        form_field(html, app, "draft-tags", "edit.tags", false);
        html.push_str(&button(
            "detail-tags-reset",
            "",
            t(app, "tags.restore"),
            "refresh",
            "quiet draft-reset",
            app.busy(),
        ));
    } else {
        for tag in &game.tags {
            crate::detail_tags::render_tag(html, app, tag);
        }
    }
    html.push_str("<span class=\"tag-inline-actions\">");
    if !app.detail.editing || app.detail.editing_configuration {
        crate::detail_tags::render_add(html, app);
    }
    if !app.detail.editing {
        html.push_str(&button(
            "tags-association",
            "",
            t(app, "detail.change_tag_source"),
            "database",
            "icon-only tag-source-button",
            app.busy(),
        ));
    }
    html.push_str("</span></div></section></div></div>");
    if app.detail.editor.is_none()
        && app.detail.confirmation.as_ref().is_none_or(|(action, _)| {
            !matches!(action.as_str(), "snapshot-delete" | "detail-tag-remove")
        })
    {
        features::confirmation(html, app);
    }
    reading(html, app, game);
    saves_card(html, app, game, &edit);
    local_patches::render(html, app, game);
    crate::screenshots::render_card(html, app, &game.id);
    write!(html,"<section class=\"detail-card official-card\" hidden><h3>{}</h3><p>{}</p>{}<small>{}</small></section></aside></div>",t(app,"official.title"),t(app,"official.help"),button("open-resource","https://www.dlsite.com/",t(app,"official.dlsite"),"arrow","secondary",false),t(app,"official.homepage")).unwrap();
    write!(html,r#"<button class="detail-back-top icon-button" data-action="detail-top" aria-label="{}" title="{}">{}</button>"#,t(app,"detail.back_top"),t(app,"detail.back_top"),icon("top")).unwrap();
    html.push_str("</div></section></div>");
    editor(html, app, game);
    if app.detail.archive_open {
        features::archive_dialog(html, app, game);
    }
    if app.detail.history_open {
        features::history(html, app, game);
    }
    features::delete_snapshot_dialog(html, app, game);
    crate::detail_tags::render_dialog(html, app);
    crate::ai_summary::render(html, app, game);
}

fn reading(html: &mut String, app: &App, game: &Game) {
    html.push_str("<div class=\"detail-grid\"><section class=\"reading-card\" data-render-key=\"reading-card\"><div class=\"reading-heading\"><div class=\"metadata-source\">");
    if let Some(meta) = &game.metadata {
        let (source, name_key) = preferences::source_for_url(&meta.url);
        write!(
            html,
            r#"<img src="{}" alt=""/><span>{}</span>"#,
            preferences::source_icon(source),
            t(app, name_key)
        )
        .unwrap();
    } else {
        write!(html, "<span>{}</span>", t(app, "detail.local_source")).unwrap();
    }
    html.push_str("</div><div class=\"reading-controls metadata-actions\">");
    metadata_actions(html, app, game);
    html.push_str("</div></div><section class=\"story-section\"><div class=\"story-body\">");
    let story = game
        .metadata
        .as_ref()
        .map(|meta| meta.description.as_str())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(t(app, "metadata.no_story"));
    if app.detail.editing && !app.detail.editing_configuration {
        form_field(html, app, "draft-meta-description", "metadata.story", true);
    } else {
        for paragraph in story.split("\n\n").filter(|text| !text.trim().is_empty()) {
            write!(html, "<p>{}</p>", esc(paragraph.trim())).unwrap();
        }
    }
    html.push_str("</div></section><section class=\"credits-section\"><dl>");
    if app.detail.editing && !app.detail.editing_configuration {
        for (field, label) in features::METADATA_FIELDS
            .iter()
            .filter(|(field, _)| *field != "description")
        {
            form_field(html, app, &format!("draft-meta-{field}"), label, false);
        }
    } else {
        if let Some(meta) = &game.metadata {
            for (field, label) in features::METADATA_FIELDS
                .iter()
                .filter(|(field, _)| *field != "description")
            {
                let text = features::meta_value(meta, field);
                if text.is_empty() {
                    continue;
                }
                write!(html, "<div><dt>{}</dt><dd>", t(app, label)).unwrap();
                credit(html, meta, field, text);
                html.push_str("</dd></div>");
            }
        }
    }
    html.push_str("</dl></section>");
    if let Some(warning) = game
        .metadata_warning
        .as_ref()
        .or(app.detail.metadata_warnings.get(&game.id))
    {
        write!(
            html,
            "<p class=\"detail-note\">{}</p>",
            esc(&warning.render(app.settings.language))
        )
        .unwrap();
    }
    html.push_str(
        "</section><aside class=\"auxiliary-column\" data-render-key=\"auxiliary-column\">",
    );
}

fn metadata_actions(html: &mut String, app: &App, game: &Game) {
    if let Some(meta) = &game.metadata
        && !meta.url.is_empty()
    {
        html.push_str(&button(
            "open-resource",
            &meta.url,
            t(app, "metadata.source"),
            "open_in_new",
            "quiet icon-only",
            false,
        ));
        html.push_str(&button(
            "metadata-select",
            &meta.url,
            t(app, "detail.refresh_metadata"),
            "refresh",
            "quiet icon-only",
            false,
        ));
    }
    html.push_str(&button(
        "metadata-association",
        "",
        t(app, "detail.change_association"),
        "database",
        "quiet icon-only",
        (app.detail.editing && !app.detail.editing_configuration) || app.busy(),
    ));
}

fn configuration(html: &mut String, app: &App, game: &Game, edit: &GameEdit) {
    write!(html,r#"<section class="detail-card config-card"><div class="utility-heading"><h3>{}</h3>{}</div>"#,t(app,"detail.configure"),button(
        if app.detail.editing { "detail-save" } else { "detail-edit" },
        "",
        t(app, if app.detail.editing { "common.save" } else { "detail.edit_configuration" }),
        if app.detail.editing { "check" } else { "edit" },
        if app.detail.editing { "primary" } else { "secondary" },
        app.busy() || !game.available,
    )).unwrap();
    if app.detail.editing {
        write!(
            html,
            "<fieldset class=\"configuration-fields\" {}>",
            if game.available { "" } else { "disabled" }
        )
        .unwrap();
        for (field, label, multiline) in [
            ("draft-executable", "edit.executable", false),
            ("draft-workdir", "edit.workdir", false),
            ("draft-launcher", "edit.launcher", false),
            ("draft-arguments", "edit.args", true),
            ("draft-locale", "tools.profile", false),
        ] {
            form_field(html, app, field, label, multiline);
        }
        html.push_str("</fieldset></section>");
        return;
    }
    html.push_str("<dl class=\"configuration-summary\">");
    let arguments = edit.arguments.join(" ");
    for (label, text) in [
        (
            "detail.executable",
            game.executable
                .as_deref()
                .map(file_name)
                .unwrap_or_default(),
        ),
        ("edit.args", arguments.as_str()),
        (
            "edit.launcher",
            edit.launcher.as_deref().unwrap_or_default(),
        ),
    ] {
        write!(
            html,
            "<div><dt>{}</dt><dd>{}</dd></div>",
            t(app, label),
            esc(value(app, text))
        )
        .unwrap();
    }
    write!(
        html,
        "</dl><div class=\"runtime-line\"><span>{}</span></div></section>",
        t(
            app,
            match app.runtime.get(&game.id).map(String::as_str) {
                Some("running") => "runtime.running",
                Some("stopped") => "runtime.stopped",
                _ => "runtime.unknown",
            }
        )
    )
    .unwrap();
}

fn location(html: &mut String, app: &App, game: &Game) {
    write!(html,"<section class=\"detail-card location-card\"><div class=\"utility-heading\"><h3>{}</h3><div class=\"card-heading-actions\">{}{}</div></div><p class=\"location-status\">{}</p><p class=\"path-value\">{}</p></section>",t(app,"detail.location"),button("open-folder",&game.id,t(app,"detail.open_directory"),"folder","quiet",app.busy()),button("relocate",&game.id,t(app,"detail.relocate"),"location_searching","quiet",app.busy()),t(app,if game.available{"detail.located"}else{"games.unavailable"}),esc(&game.folder)).unwrap();
}

fn save_file_label(app: &App, folder: &str, file: &str) -> String {
    let folder = folder.replace('/', "\\");
    let folder = folder.trim_end_matches('\\');
    let file = file.replace('/', "\\");
    if file
        .get(..folder.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(folder))
        && let Some(relative) = file
            .get(folder.len()..)
            .and_then(|suffix| suffix.strip_prefix('\\'))
    {
        format!("{}\\{}", t(app, "save.game_directory"), relative)
    } else {
        file
    }
}

fn saves_card(html: &mut String, app: &App, game: &Game, edit: &GameEdit) {
    let report = app.detail.saves.get(&game.id);
    let selected = edit
        .save_files
        .is_empty()
        .then(|| {
            edit.save_path
                .as_deref()
                .filter(|path| !path.trim().is_empty())
                .or_else(|| (edit.save_paths.len() == 1).then(|| edit.save_paths[0].as_str()))
        })
        .flatten();
    let has_confirmed = report.is_some_and(|report| {
        report
            .locations
            .iter()
            .any(|location| location.confidence == "confirmed")
    });
    let locations: Vec<_> = report
        .into_iter()
        .flat_map(|report| &report.locations)
        .filter(|location| {
            matches!(location.source.as_str(), "ai" | "observed")
                || edit.save_path_auto
                || selected.is_none_or(|path| location.path.eq_ignore_ascii_case(path))
        })
        .filter(|location| {
            matches!(location.source.as_str(), "ai" | "observed")
                || !has_confirmed
                || location.confidence == "confirmed"
        })
        .collect();
    write!(html, "<section class=\"detail-card save-card\"><div class=\"utility-heading\"><h3>{}</h3><div class=\"card-heading-actions\">{}{}{}</div></div>",
        t(app, "save.title"),
        button("ai-find-save", &game.id, t(app, "save.ai_find"), "", "quiet", features::save_busy(app)),
        button("choose-save", &game.id, t(app, "save.choose"), "folder", "quiet", features::save_busy(app)),
        crate::widgets::icon_button("open-save", "", t(app, "detail.open_directory"), "folder_open", "quiet", !locations.iter().any(|location| location.exists))
    ).unwrap();
    if let Some(report) = app.detail.saves.get(&game.id) {
        if report.locations.is_empty() {
            write!(
                html,
                "<p class=\"muted\">{}</p>",
                t(
                    app,
                    if game.executable.is_none() {
                        "save.disc"
                    } else {
                        "save.none"
                    }
                )
            )
            .unwrap();
        }
        for location in locations {
            write!(html, "<div class=\"save-entry\"><p class=\"save-status\" title=\"{}\"><span></span>{}</p>",
                t(app, match location.source.as_str() { "manual" => "save.manual", "config" => "save.config", "roaming" => "save.roaming", "documents" => "save.documents", "saved-games" => "save.saved_games", _ => "save.game" }),
                t(app, if !location.exists { "save.uncreated" } else if location.confidence == "confirmed" { "backup.confirmed" } else { "save.candidate" })
            ).unwrap();
            if location.files.is_empty() {
                write!(html, "<p class=\"path-value\">{}</p>", esc(&location.path)).unwrap();
            } else {
                for file in &location.files {
                    write!(
                        html,
                        "<p class=\"path-value\">{}</p>",
                        esc(&save_file_label(app, &game.folder, file))
                    )
                    .unwrap();
                }
            }
            html.push_str("<div class=\"detail-actions\">");
            if location.confidence == "confirmed" {
                html.push_str("<span class=\"save-backup-actions\">");
                html.push_str(&button(
                    "backup-saves",
                    &game.id,
                    t(app, "backup.create"),
                    "backup",
                    "quiet icon-only",
                    features::save_busy(app) || crate::cloud::busy(app),
                ));
                html.push_str(&button(
                    "save-history",
                    &game.id,
                    t(app, "backup.archive"),
                    "history",
                    "quiet icon-only",
                    features::save_busy(app) || crate::cloud::busy(app),
                ));
                crate::cloud::detail_upload_button(html, app, &game.id);
                html.push_str("</span>");
            } else if location.exists {
                html.push_str(&button(
                    "confirm-save",
                    &location.path,
                    t(app, "common.confirm"),
                    "check",
                    "secondary",
                    features::save_busy(app),
                ));
            }
            html.push_str("</div></div>");
        }
        if report.limited && !has_confirmed {
            write!(html, "<p>{}</p>", t(app, "save.limited")).unwrap();
        }
    }
    if app.detail.history_game.as_deref() == Some(&game.id) && !app.detail.history.is_empty() {
        features::current_snapshot(html, app, game);
        write!(
            html,
            "<div class=\"save-card-history\"><h4>{}</h4><ul>",
            t(app, "backup.history")
        )
        .unwrap();
        for snapshot in app.detail.history.iter().take(2) {
            features::snapshot_row(html, app, game, snapshot, false);
        }
        html.push_str("</ul></div>");
    }
    html.push_str("</section>");
}

fn tools_dialog(html: &mut String, app: &App, game: &Game) {
    let edit = app
        .settings
        .game_edits
        .get(&game.id)
        .cloned()
        .unwrap_or_default();
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog game-settings-dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title"><header><h2 id="editor-title">{}</h2>{}</header><div class="editor-body"><div class="game-tool-options">"#,t(app,"detail.configure"),button("editor-close","",t(app,"common.close"),"close","quiet",app.busy())).unwrap();
    crate::detail_tools::configuration_options(html, app, game, &edit);
    html.push_str("</div>");
    if !app.ai_available {
        write!(html, "<p class=\"detail-note\">{}</p>", t(app, "summary.tools_missing")).unwrap();
    }
    if crate::ai_summary::busy(app, &game.id) {
        write!(
            html,
            "<p class=\"detail-note\" role=\"status\">{}</p>",
            t(app, crate::ai_summary::status_key(app, &game.id))
        )
        .unwrap();
    }
    configuration(html, app, game, &edit);
    location(html, app, game);
    html.push_str("</div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detail_form;
    use crate::models::MetadataLink;
    use galgear_i18n::Language;
    use serde_json::json;
    #[test]
    fn loose_save_card_shows_file_paths_and_a_group_confirmation() {
        let mut app = App::default();
        let game: Game = serde_json::from_value(json!({"id":"air","title":"AIR","folder":"D:/Games/AIR","engine":"unknown","modified":0})).unwrap();
        app.detail.saves.insert(game.id.clone(), serde_json::from_value(json!({"locations":[
            {"path":"D:/Games/AIR","files":["D:/Games/AIR/SAVE.SAV","D:/Games/AIR/READ.SAV"],"source":"game","confidence":"detected","exists":true,"readable":true}
        ],"limited":false})).unwrap());
        let mut html = String::new();
        saves_card(&mut html, &app, &game, &GameEdit::default());
        assert!(html.contains("游戏目录\\SAVE.SAV"));
        assert!(html.contains("游戏目录\\READ.SAV"));
        assert!(!html.contains("<p class=\"path-value\">D:/Games/AIR</p>"));
        assert_eq!(html.matches("<button").count(), 4);
        for action in ["ai-find-save", "choose-save", "open-save"] {
            assert_eq!(
                html.matches(&format!("data-action=\"{action}\"")).count(),
                1
            );
        }
        assert!(html.contains("data-action=\"confirm-save\" data-value=\"D:/Games/AIR\""));
        assert_eq!(
            save_file_label(&app, "D:/Games/AIR", "D:/Games/AIR2/SAVE.SAV"),
            "D:\\Games\\AIR2\\SAVE.SAV"
        );
    }

    #[test]
    fn save_card_limits_history_but_current_save_and_archive_include_older_backups() {
        let mut app = App::default();
        let game: Game = serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Game","engine":"unknown","modified":0})).unwrap();
        app.detail.selected = Some(game.id.clone());
        app.detail.history_game = Some(game.id.clone());
        app.detail.current_snapshot = Some("backup-1".into());
        app.detail.history = (1..=8).rev().map(|index| serde_json::from_value(json!({
            "id":format!("backup-{index}"), "name":format!("Title_1793285352_{index}"), "created":1793285352_u64,
            "sequence":index,"size":10,"note":"","locked":false,"files":[]
        })).unwrap()).collect();
        let mut card = String::new();
        saves_card(&mut card, &app, &game, &GameEdit::default());
        assert_eq!(card.matches("data-action=\"preview-restore\"").count(), 2);
        assert!(card.contains("当前存档：Title_1793285352_1"));
        let mut archive = String::new();
        features::archive_dialog(&mut archive, &app, &game);
        assert_eq!(archive.matches("data-action=\"preview-restore\"").count(), 8);
        assert_eq!(archive.matches("<time>").count(), 8);
        app.detail.confirmation = Some(("snapshot-delete".into(), "backup-1".into()));
        let mut card = String::new();
        saves_card(&mut card, &app, &game, &GameEdit::default());
        assert!(!card.contains("删除这份备份快照"));
        let mut dialog = String::new();
        features::delete_snapshot_dialog(&mut dialog, &app, &game);
        assert!(dialog.contains("role=\"alertdialog\""));
        assert!(dialog.contains("Title_1793285352_1"));
    }
    #[test]
    fn selected_save_hides_candidates_while_discovery_refreshes() {
        let mut app = App::default();
        let game:Game=serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","library":"F:/Games","executable":"F:/Games/Title/game.exe","engine":"unknown","cover":null,"modified":0,"tags":[]})).unwrap();
        app.detail.saves.insert(game.id.clone(),serde_json::from_value(json!({"locations":[{"path":"C:/Chosen","source":"manual","confidence":"confirmed","exists":true,"readable":true},{"path":"C:/OtherCandidate","source":"roaming","confidence":"matched","exists":true,"readable":true}],"limited":false})).unwrap());
        let mut html = String::new();
        saves_card(
            &mut html,
            &app,
            &game,
            &GameEdit {
                save_path: Some("C:/Chosen".into()),
                ..Default::default()
            },
        );
        assert!(html.contains("C:/Chosen"));
        assert!(!html.contains("C:/OtherCandidate"));
        assert!(html.contains("data-action=\"choose-save\""));
        let mut automatic = GameEdit { save_path: Some("C:/Old".into()), save_path_auto: true, ..Default::default() };
        let mut updated = String::new();
        saves_card(&mut updated, &app, &game, &automatic);
        assert!(updated.contains("C:/Chosen"));
        assert!(!updated.contains("C:/OtherCandidate"));
        automatic.save_path_auto = false;
        let mut manual = String::new();
        saves_card(&mut manual, &app, &game, &automatic);
        assert!(!manual.contains("C:/Chosen"));
    }

    #[test]
    fn automatically_confirmed_save_hides_other_candidates_without_a_manual_selection() {
        let mut app = App::default();
        let game: Game = serde_json::from_value(json!({
            "id":"atri", "title":"ATRI", "folder":"F:/Games/ATRI",
            "engine":"unknown", "modified":0
        })).unwrap();
        app.detail.saves.insert(game.id.clone(), serde_json::from_value(json!({
            "locations":[
                {"path":"C:/Goldberg SteamEmu Saves/1230140/remote", "source":"config", "confidence":"confirmed", "exists":true, "readable":true},
                {"path":"C:/Aniplex/ATRI-MyDearMoments-", "source":"roaming", "confidence":"matched", "exists":true, "readable":true},
                {"path":"C:/Uncreated", "source":"game", "confidence":"matched", "exists":false, "readable":false}
            ],
            "limited":true
        })).unwrap());
        let mut html = String::new();
        saves_card(&mut html, &app, &game, &GameEdit::default());
        assert!(html.contains("C:/Goldberg SteamEmu Saves/1230140/remote"));
        assert!(!html.contains("C:/Aniplex/ATRI-MyDearMoments-"));
        assert!(!html.contains(t(&app, "save.limited")));
        assert!(!html.contains("data-action=\"confirm-save\""));
        assert!(html.contains("data-action=\"backup-saves\""));

        app.detail.saves.get_mut(&game.id).unwrap().locations[0].confidence = "matched".into();
        html.clear();
        saves_card(&mut html, &app, &game, &GameEdit::default());
        assert!(html.contains("C:/Goldberg SteamEmu Saves/1230140/remote"));
        assert!(html.contains("C:/Aniplex/ATRI-MyDearMoments-"));
        assert!(html.contains(t(&app, "save.limited")));
        assert!(html.contains("C:/Uncreated"));
        assert_eq!(html.matches("data-action=\"confirm-save\"").count(), 2);
        assert_eq!(html.matches("data-action=\"open-save\"").count(), 1);
    }
    #[test]
    fn detail_shows_playtime_and_never_played_without_metadata() {
        let app = App::default();
        let game: Game = serde_json::from_value(json!({
            "id": "game", "title": "Title", "folder": "F:/Games/Title",
            "engine": "unknown", "modified": 0
        }))
        .unwrap();
        let mut html = String::new();
        render(&mut html, &app, &game);
        assert!(html.contains("<dt>品牌</dt><dd>未设置</dd>"));
        assert!(html.contains("<dt>发售日期</dt><dd>未设置</dd>"));
        assert!(html.contains(r#"class="game-playtime"><dt>游玩时间</dt><dd>0 分钟</dd></div><div class="last-played"><dt>上次游玩</dt><dd>未游玩</dd>"#));
    }
    #[test]
    fn detail_orders_brand_release_playtime_and_last_played() {
        let app = App::default();
        let game: Game = serde_json::from_value(json!({
            "id": "game", "title": "Title", "folder": "F:/Games/Title",
            "engine": "unknown", "modified": 0,
            "metadata": {"brand": "Key", "release_date": "2025-03-01"},
            "last_played": 946684800_u64, "playtime_seconds": 4500
        }))
        .unwrap();
        let mut html = String::new();
        render(&mut html, &app, &game);
        assert!(html.find("<dt>品牌</dt>").unwrap() < html.find("<dt>发售日期</dt>").unwrap());
        assert!(html.contains(r#"<dt>发售日期</dt><dd>2025-03-01</dd></div><div class="game-playtime"><dt>游玩时间</dt><dd>1.3 小时</dd></div><div class="last-played"><dt>上次游玩</dt><dd>2000-01-01</dd>"#));
    }
    #[test]
    fn detail_brand_uses_current_metadata_even_with_a_legacy_summary() {
        let mut app = App::default();
        // 旧配置曾在切换资料源时冻结顶部品牌；加载后应直接显示当前资料。
        app.settings.game_edits.insert(
            "game".into(),
            serde_json::from_value(json!({
                "summary_brand": "5pb."
            }))
            .unwrap(),
        );
        for brand in ["elf", "Manual <Brand>", ""] {
            let game: Game = serde_json::from_value(json!({
                "id": "game", "title": "Title", "folder": "F:/Games/Title",
                "engine": "unknown", "modified": 0, "metadata": {"brand": brand}
            }))
            .unwrap();
            app.catalog.games = vec![game.clone()];
            app.detail.selected = Some(game.id.clone());
            for editing in [false, true] {
                if editing {
                    detail_form::begin(&mut app);
                } else {
                    detail_form::cancel(&mut app);
                }
                let mut html = String::new();
                render(&mut html, &app, &game);
                let facts = html
                    .split_once("<dl class=\"hero-facts\">")
                    .unwrap()
                    .1
                    .split_once("</dl>")
                    .unwrap()
                    .0;
                assert!(!html.contains("5pb."));
                if brand.is_empty() {
                    assert!(!facts.contains("data-action=\"detail-tag\""));
                } else {
                    assert!(facts.contains(&format!("data-value=\"{}\"", esc(brand))));
                    if editing {
                        assert!(html.contains(&format!(
                            "data-input=\"draft-meta-brand\" value=\"{}\"",
                            esc(brand)
                        )));
                    } else {
                        assert_eq!(
                            html.matches(&format!("data-value=\"{}\"", esc(brand)))
                                .count(),
                            2
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn linked_and_unlinked_credits_filter_the_library_instead_of_opening_websites() {
        let meta = Metadata {
            writers: "Alice Smith、Bob".into(),
            links: vec![MetadataLink {
                field: "writers".into(),
                name: "Alice Smith".into(),
                url: "https://example.com/person/1".into(),
            }],
            ..Default::default()
        };
        let mut html = String::new();
        credit(&mut html, &meta, "writers", &meta.writers);
        assert_eq!(html.matches("data-action=\"detail-tag\"").count(), 2);
        assert!(html.contains("data-value=\"Alice Smith\""));
        assert!(html.contains("data-value=\"Bob\""));
        assert!(!html.contains("https://example.com"));
    }
    #[test]
    fn cancelling_complete_draft_restores_saved_configuration_on_reopening() {
        let mut app = App::default();
        let game:Game=serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","library":"F:/Games","executable":"F:/Games/Title/game.exe","engine":"unknown","cover":null,"modified":0,"tags":[]})).unwrap();
        app.catalog.games.push(game);
        app.detail.selected = Some("game".into());
        detail_form::begin(&mut app);
        app.form
            .insert("draft-executable".into(), "unsaved.exe".into());
        detail_form::cancel(&mut app);
        detail_form::begin(&mut app);
        assert_eq!(
            features::form(&app, "draft-executable"),
            "F:/Games/Title/game.exe"
        );
    }
    #[test]
    fn complete_detail_editor_renders_with_one_save_in_all_supported_languages() {
        let game:Game=serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","library":"F:/Games","executable":"F:/Games/Title/game.exe","engine":"unknown","cover":null,"modified":0,"tags":["学园"],"metadata":{"description":"Story","brand":"Brand"}})).unwrap();
        for language in [Language::Chinese, Language::Japanese, Language::English] {
            let mut app = App::default();
            app.settings.language = language;
            app.catalog.games.push(game.clone());
            app.detail.selected = Some(game.id.clone());
            detail_form::begin(&mut app);
            let mut html = String::new();
            render(&mut html, &app, &game);
            assert_eq!(html.matches("data-action=\"detail-save\"").count(), 1);
            assert!(html.contains("draft-meta-description"));
            assert!(!html.contains("data-action=\"tags-association\""));
            assert!(
                html.find("data-action=\"detail-rematch\"").unwrap()
                    < html.find("data-action=\"detail-cancel\"").unwrap()
            );
            assert!(!html.contains("editor-open"));
        }
    }
}
