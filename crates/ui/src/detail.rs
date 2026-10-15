//! Reference-led detail reading page, with cached credits and compact utilities.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Editor {
    Association,
    Tools,
}

fn editor(html: &mut String, app: &App, game: &Game) {
    match app.detail_editor {
        None => return,
        Some(Editor::Tools) => {
            tools_dialog(html, app, game);
            return;
        }
        Some(Editor::Association) => {}
    }
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title"><header><h2 id="editor-title">{}</h2>{}</header><div class="editor-body">"#,t(app,"detail.change_association"),button("editor-close","",t(app,"common.close"),"close","quiet",app.busy)).unwrap();
    form_field(html, app, "metadata-query", "metadata.query", false);
    html.push_str("<div class=\"metadata-provider-picker\">");
    for (source, name) in preferences::SOURCES {
        html.push_str(&button(
            "metadata-provider",
            source,
            name,
            "",
            if app.metadata_source == source {
                "primary"
            } else {
                "quiet"
            },
            app.busy,
        ));
    }
    html.push_str("</div>");
    html.push_str(&button(
        "metadata-search",
        &game.id,
        t(app, "metadata.search"),
        "search",
        "secondary",
        app.busy,
    ));
    if let Some(results) = app.candidates.get(&game.id) {
        for result in results {
            write!(
                html,
                r#"<div class="candidate-row"><span>{}</span>{}</div>"#,
                esc(&result.title),
                button(
                    "metadata-select",
                    &result.url,
                    t(app, "metadata.select"),
                    "check",
                    "secondary",
                    app.busy
                )
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
    let mut remaining = text;
    for link in meta.links.iter().filter(|link| link.field == field) {
        if let Some(index) = remaining.find(&link.name) {
            html.push_str(&esc(&remaining[..index]));
            write!(html,r#"<button type="button" class="credit-link" data-action="open-resource" data-value="{}">{}</button>"#,esc(&link.url),esc(&link.name)).unwrap();
            remaining = &remaining[index + link.name.len()..];
        }
    }
    html.push_str(&esc(remaining));
}

pub(super) fn render(html: &mut String, app: &App, game: &Game) {
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
    write!(html,r#"<div class="detail-overlay reference-detail {}" data-render-key="detail-{}"><section class="detail-panel" aria-labelledby="detail-title"><div class="detail-scroll" data-render-key="detail-scroll"><div class="detail-back-row"><button id="close-detail" class="back-library" data-action="close-detail">{}<span>{}</span></button><div class="detail-edit-actions">"#,if app.detail_editing{"editing"}else{""},esc(&game.id),icon("arrow"),t(app,"detail.back")).unwrap();
    if app.detail_editing {
        html.push_str(&button(
            "detail-cancel",
            "",
            t(app, "common.cancel"),
            "close",
            "quiet",
            app.busy,
        ));
    }
    html.push_str(&button(
        if app.detail_editing {
            "detail-save"
        } else {
            "detail-edit"
        },
        &game.id,
        t(
            app,
            if app.detail_editing {
                "common.save"
            } else {
                "detail.edit_configuration"
            },
        ),
        if app.detail_editing { "check" } else { "edit" },
        "primary detail-edit-toggle",
        app.busy,
    ));
    html.push_str(
        "</div></div><div class=\"detail-hero\"><div class=\"detail-cover poster poster-1\">",
    );
    poster(html, app, game);
    if app.detail_editing {
        write!(
            html,
            r#"<div class="cover-edit-actions">{}{}</div>"#,
            button(
                "choose-cover",
                &game.id,
                t(app, "detail.cover"),
                "image",
                "quiet",
                app.busy
            ),
            button(
                "reset-cover",
                &game.id,
                t(app, "metadata.reset"),
                "refresh",
                "quiet",
                app.busy
            )
        )
        .unwrap();
    }
    html.push_str("</div><div class=\"detail-summary\"><div class=\"detail-heading\">");
    if !game.available {
        write!(
            html,
            r#"<span class="version-pill unavailable">{}</span>"#,
            t(app, "games.unavailable")
        )
        .unwrap();
    }
    if !game.play_status.is_empty() && game.play_status != "unmarked" {
        write!(
            html,
            r#"<span class="game-play-status {}">{}</span>"#,
            esc(&game.play_status),
            context_menu::label(app, &game.play_status)
        )
        .unwrap();
    }
    if app.detail_editing {
        html.push_str("<div id=\"detail-title\">");
        form_field(html, app, "draft-title", "edit.name", false);
        html.push_str("</div>");
    } else {
        write!(html, r#"<h2 id="detail-title">{}</h2>"#, esc(&game.title)).unwrap();
    }
    html.push_str("<div class=\"hero-facts\">");
    if let Some(meta) = &game.metadata {
        if !meta.brand.is_empty() {
            html.push_str("<span>");
            credit(html, meta, "brand", &meta.brand);
            html.push_str("</span>");
        }
        if !meta.release_date.is_empty() {
            write!(html, "<span>{}</span>", esc(&meta.release_date)).unwrap();
        }
    }
    html.push_str("</div></div><div class=\"detail-actions hero-actions\">");
    if game.executable.is_some() {
        html.push_str(&button(
            "launch",
            &game.id,
            t(app, "detail.launch"),
            "play",
            "primary",
            app.busy || !game.available || runtime == "running",
        ));
    }
    write!(html,r#"<button class="icon-button game-settings-button" data-action="game-settings" aria-label="{}" title="{}">{}</button>"#,t(app,"detail.configure"),t(app,"detail.configure"),icon("settings")).unwrap();
    write!(
        html,
        "</div><section class=\"hero-tags\" aria-label=\"{}\"><div class=\"utility-heading\"><h3>{}</h3></div><div class=\"game-tags\">",
        t(app, "detail.tags"), t(app,"detail.tags")
    )
    .unwrap();
    if app.detail_editing {
        form_field(html, app, "draft-tags", "edit.tags", false);
        html.push_str(&button(
            "detail-tags-reset",
            "",
            t(app, "tags.restore"),
            "refresh",
            "quiet draft-reset",
            app.busy,
        ));
    } else {
        for tag in &game.tags {
            html.push_str(&button(
                "detail-tag",
                tag,
                &esc(tag),
                "",
                "tag-button",
                false,
            ));
        }
        if game.tags.is_empty() {
            write!(html, "<p class=\"muted\">{}</p>", t(app, "detail.no_tags")).unwrap();
        }
    }
    html.push_str("</div></section></div></div>");
    if app.detail_editor.is_none() {
        features::confirmation(html, app);
    }
    reading(html, app, game);
    saves_card(html, app, game, &edit);
    write!(html,"<section class=\"detail-card official-card\"><h3>{}</h3><p>{}</p>{}<small>{}</small></section></aside></div>",t(app,"official.title"),t(app,"official.help"),button("open-resource","https://www.dlsite.com/",t(app,"official.dlsite"),"arrow","secondary",false),t(app,"official.homepage")).unwrap();
    if app.history_open {
        features::history(html, app, game);
    }
    write!(html,r#"<button class="detail-back-top icon-button" data-action="detail-top" aria-label="{}" title="{}">{}</button>"#,t(app,"detail.back_top"),t(app,"detail.back_top"),icon("top")).unwrap();
    html.push_str("</div></section></div>");
    editor(html, app, game);
}

fn reading(html: &mut String, app: &App, game: &Game) {
    html.push_str("<div class=\"detail-grid\"><section class=\"reading-card\" data-render-key=\"reading-card\"><div class=\"reading-heading\"><div class=\"metadata-source\">");
    if let Some(meta) = &game.metadata {
        let (source, name) = preferences::source_for_url(&meta.url);
        write!(
            html,
            r#"<img src="assets/{}.{}" alt=""/><span>{}</span>"#,
            source,
            if source == "2dfan" { "png" } else { "ico" },
            name
        )
        .unwrap();
    } else {
        write!(html, "<span>{}</span>", t(app, "detail.local_source")).unwrap();
    }
    html.push_str("</div><div class=\"reading-controls metadata-actions\">");
    metadata_actions(html, app, game);
    write!(
        html,
        "</div></div><section class=\"story-section\"><h3>{}</h3><div class=\"story-body\">",
        t(app, "metadata.story")
    )
    .unwrap();
    let story = game
        .metadata
        .as_ref()
        .map(|meta| meta.description.as_str())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(t(app, "metadata.no_story"));
    if app.detail_editing {
        form_field(html, app, "draft-meta-description", "metadata.story", true);
        html.push_str(&button(
            "detail-field-reset",
            "description",
            t(app, "metadata.reset"),
            "refresh",
            "quiet draft-reset",
            app.busy,
        ));
    } else {
        for paragraph in story.split("\n\n").filter(|text| !text.trim().is_empty()) {
            write!(html, "<p>{}</p>", esc(paragraph.trim())).unwrap();
        }
    }
    write!(
        html,
        "</div></section><section class=\"credits-section\"><h3>{}</h3><dl>",
        t(app, "detail.credits")
    )
    .unwrap();
    if app.detail_editing {
        for (field, label) in features::METADATA_FIELDS
            .iter()
            .filter(|(field, _)| *field != "description")
        {
            form_field(html, app, &format!("draft-meta-{field}"), label, false);
            html.push_str(&button(
                "detail-field-reset",
                field,
                t(app, "metadata.reset"),
                "refresh",
                "quiet draft-reset",
                app.busy,
            ));
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
        .or(app.metadata_warnings.get(&game.id))
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
            "arrow",
            "quiet",
            false,
        ));
        html.push_str(&button(
            "metadata-select",
            &meta.url,
            t(app, "detail.refresh_metadata"),
            "refresh",
            "quiet",
            false,
        ));
    }
    html.push_str(&button(
        "metadata-association",
        "",
        t(app, "detail.change_association"),
        "",
        "quiet",
        app.detail_editing || app.busy,
    ));
}

fn configuration(html: &mut String, app: &App, game: &Game, edit: &GameEdit) {
    write!(html,r#"<section class="detail-card config-card"><div class="utility-heading"><h3>{}</h3></div>"#,t(app,"detail.configure")).unwrap();
    if app.detail_editing {
        for (field, label, multiline) in [
            ("draft-executable", "edit.executable", false),
            ("draft-workdir", "edit.workdir", false),
            ("draft-launcher", "edit.launcher", false),
            ("draft-arguments", "edit.args", true),
            ("draft-locale", "tools.profile", false),
        ] {
            form_field(html, app, field, label, multiline);
        }
        html.push_str("</section>");
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
    write!(html,"<section class=\"detail-card location-card\"><div class=\"utility-heading\"><h3>{}</h3><span>{}</span></div><p class=\"path-value\">{}</p><div class=\"detail-actions\">{}{}</div></section>",t(app,"detail.location"),t(app,if game.available{"detail.located"}else{"games.unavailable"}),esc(&game.folder),button("open-folder",&game.id,t(app,"detail.open_directory"),"folder","quiet",app.busy),button("relocate",&game.id,t(app,"detail.relocate"),"location_searching","quiet",app.busy)).unwrap();
}

fn saves_card(html: &mut String, app: &App, game: &Game, _edit: &GameEdit) {
    write!(html,"<section class=\"detail-card save-card\"><div class=\"utility-heading\"><h3>{}</h3>{}</div>",t(app,"save.title"),button("save-history",&game.id,t(app,"backup.history"),"folder_special","quiet",false)).unwrap();
    if let Some(report) = app.saves.get(&game.id) {
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
        for location in &report.locations {
            write!(html,"<div class=\"save-entry\"><p class=\"save-status\" title=\"{}\"><span></span>{}</p><p class=\"path-value\">{}</p><div class=\"detail-actions\">",t(app,match location.source.as_str(){"manual"=>"save.manual","config"=>"save.config","roaming"=>"save.roaming","documents"=>"save.documents",_=>"save.game"}),t(app,if !location.exists{"save.uncreated"}else if location.confidence=="confirmed"{"backup.confirmed"}else{"save.candidate"}),esc(&location.path)).unwrap();
            if location.exists {
                html.push_str(&button(
                    "open-save",
                    &location.path,
                    t(app, "detail.open_directory"),
                    "folder",
                    "quiet",
                    false,
                ));
                if location.confidence == "confirmed" {
                    html.push_str(&button(
                        "backup-saves",
                        &game.id,
                        t(app, "backup.create"),
                        "backup",
                        "quiet",
                        features::save_busy(app),
                    ));
                } else {
                    html.push_str(&button(
                        "confirm-save",
                        &location.path,
                        t(app, "common.confirm"),
                        "check",
                        "secondary",
                        false,
                    ));
                }
            }
            html.push_str("</div></div>");
        }
        if report.limited {
            write!(html, "<p>{}</p>", t(app, "save.limited")).unwrap();
        }
    }
    if app.history_game.as_deref() == Some(&game.id) && !app.history.is_empty() {
        write!(
            html,
            "<div class=\"save-card-history\"><h4>{}</h4><ul>",
            t(app, "backup.history")
        )
        .unwrap();
        for snapshot in &app.history {
            write!(
                html,
                "<li>{}</li>",
                esc(&features::snapshot_name(game, snapshot))
            )
            .unwrap();
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
    write!(html,r#"<div class="editor-overlay"><section class="editor-dialog game-settings-dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title"><header><h2 id="editor-title">{}</h2>{}</header><div class="editor-body"><div class="game-tool-options">"#,t(app,"detail.configure"),button("editor-close","",t(app,"common.close"),"close","quiet",app.busy)).unwrap();
    for (action, key, checked) in [
        (
            "toggle-locale",
            "tools.locale_launch",
            edit.launch_mode == "locale",
        ),
        ("toggle-upscale", "tools.upscale", edit.upscale),
    ] {
        write!(html,r#"<button class="game-tool-option" role="checkbox" aria-checked="{}" data-action="{}" {}><span class="tool-check">{}</span><span>{}</span></button>"#,checked,action,if app.busy{"disabled"}else{""},if checked{icon("check")}else{String::new()},t(app,key)).unwrap();
    }
    html.push_str("</div>");
    configuration(html, app, game, &edit);
    if !app.detail_editing {
        html.push_str(&button(
            "detail-edit",
            "",
            t(app, "detail.edit_configuration"),
            "edit",
            "secondary",
            app.busy,
        ));
    } else {
        html.push_str(&button(
            "detail-save",
            "",
            t(app, "common.save"),
            "check",
            "primary",
            app.busy,
        ));
    }
    location(html, app, game);
    html.push_str("</div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelling_complete_draft_restores_saved_configuration_on_reopening() {
        let mut app = App::default();
        let game:Game=serde_json::from_value(json!({"id":"game","title":"Title","folder":"F:/Games/Title","library":"F:/Games","executable":"F:/Games/Title/game.exe","engine":"unknown","cover":null,"modified":0,"tags":[]})).unwrap();
        app.catalog.games.push(game);
        app.selected = Some("game".into());
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
            app.selected = Some(game.id.clone());
            detail_form::begin(&mut app);
            let mut html = String::new();
            render(&mut html, &app, &game);
            assert_eq!(html.matches("data-action=\"detail-save\"").count(), 1);
            assert!(html.contains("draft-meta-description"));
            assert!(!html.contains("editor-open"));
        }
    }
}
