//! Library dialogs.
use crate::{
    choices, features, library::Dialog, library::folder_path, library::ordered, library::subtree,
    state::App, view::f, view::form_field, view::t, widgets::button, widgets::esc, widgets::icon,
};
use std::{collections::BTreeSet, fmt::Write};

fn affected(app: &App, id: &str) -> (usize, usize) {
    let mut folders = BTreeSet::from([id.to_owned()]);
    loop {
        let before = folders.len();
        for f in &app.settings.collections {
            if f.parent.as_ref().is_some_and(|p| folders.contains(p)) {
                folders.insert(f.id.clone());
            }
        }
        if folders.len() == before {
            break;
        }
    }
    let games: BTreeSet<_> = app
        .settings
        .collections
        .iter()
        .filter(|f| folders.contains(&f.id))
        .flat_map(|f| &f.games)
        .collect();
    (folders.len().saturating_sub(1), games.len())
}

pub(crate) fn dialog(html: &mut String, app: &App) {
    let Some(dialog) = &app.library.dialog else {
        return;
    };
    let title = match dialog {
        Dialog::Edit { id } => {
            if id.is_some() {
                "folders.edit"
            } else {
                "folders.new"
            }
        }
        Dialog::Picker { .. } => "folders.add_games",
        Dialog::Delete { .. } => "folders.delete",
        Dialog::Exclude { .. } => "bulk.exclude",
        Dialog::Warnings => "library.warnings_title",
        Dialog::Failures => "bulk.failures",
        Dialog::Restore => "library.restore",
        Dialog::Tags { .. } => "games.tag_filter",
        Dialog::AddGames => "folders.add_current",
        Dialog::DeleteFolders { .. } => "folders.delete",
    };
    let _ = write!(
        html,
        r#"<div class="library-overlay"><section class="library-dialog" role="dialog" aria-modal="true" aria-labelledby="library-dialog-title"><header><h2 id="library-dialog-title">{}</h2><button id="library-dialog-close" class="icon-button" data-action="library-close" aria-label="{}">{}</button></header><div class="library-dialog-body">"#,
        t(app, title),
        t(app, "common.close"),
        icon("close")
    );
    match dialog {
        Dialog::AddGames => {
            form_field(html, app, "game-query", "common.search", false);
            let query = features::form(app, "game-query").to_lowercase();
            let folder = app
                .settings
                .collections
                .iter()
                .find(|f| Some(&f.id) == app.library.collection.as_ref());
            html.push_str("<div class=\"game-membership-picker\">");
            let mut available = 0;
            for game in &app.catalog.games {
                if folder.is_some_and(|f| f.games.contains(&game.id))
                    || !game.title.to_lowercase().contains(&query)
                {
                    continue;
                }
                available += 1;
                write!(html,r#"<button class="membership-option {}" data-action="picker-game" data-value="{}" aria-pressed="{}"><span class="membership-check">{}</span><span>{}</span></button>"#,
                    if app.library.picker_games.contains(&game.id){"selected"}else{""},esc(&game.id),app.library.picker_games.contains(&game.id),
                    if app.library.picker_games.contains(&game.id){icon("check")}else{String::new()},esc(&game.title)).unwrap();
            }
            if available == 0 {
                write!(
                    html,
                    "<p class=\"muted\">{}</p>",
                    t(app, "folders.no_candidates")
                )
                .unwrap();
            }
            html.push_str("</div>");
            html.push_str(&button(
                "picker-games-save",
                "",
                t(app, "common.add"),
                "plus",
                "primary",
                app.busy() || app.library.picker_games.is_empty(),
            ));
        }
        Dialog::DeleteFolders { ids } => {
            write!(html, "<p>{}</p>", t(app, "folders.batch_delete_help")).unwrap();
            html.push_str("<ul>");
            for id in ids {
                write!(
                    html,
                    "<li>{}</li>",
                    esc(&folder_path(&app.settings.collections, id))
                )
                .unwrap();
            }
            html.push_str("</ul>");
            html.push_str(&button(
                "folders-delete-confirm",
                "",
                t(app, "common.confirm"),
                "trash",
                "primary danger",
                app.busy(),
            ));
        }
        Dialog::Tags { selected, status } => {
            write!(html, r#"<fieldset class="tag-status-filter"><legend>{}</legend><div>"#, t(app, "tags.game_status")).unwrap();
            for (value, key, active) in status.options() {
                write!(html, r#"<button type="button" class="button secondary {}" data-action="tag-status" data-value="{}" aria-pressed="{}">{}</button>"#,
                    if active { "active" } else { "" }, value, active, t(app, key)).unwrap();
            }
            html.push_str("</div></fieldset>");
            let query = features::form(app, "tag-query");
            write!(html, r#"<div class="tag-search"><label class="edit-field"><span>{}</span><input id="tag-query" data-input="tag-query" type="search" value="{}" placeholder="{}" autocomplete="off" aria-controls="tag-search-results"/></label>"#, t(app, "common.search"), esc(&query), t(app, "tags.search_placeholder")).unwrap();
            if !query.trim().is_empty() {
                let mut tags = galgear_tags::normalize(
                    app.catalog
                        .games
                        .iter()
                        .flat_map(|game| game.tags.iter().cloned()),
                );
                tags.sort_by_cached_key(|tag| tag.to_lowercase());
                let query = query.trim().to_lowercase();
                html.push_str("<div id=\"tag-search-results\" class=\"tag-search-results\">");
                let mut count = 0;
                for tag in tags.iter().filter(|tag| {
                    tag.to_lowercase().contains(&query)
                        && !selected
                            .iter()
                            .any(|name| galgear_tags::key(name) == galgear_tags::key(tag))
                }) {
                    count += 1;
                    html.push_str(&button(
                        "tag-filter",
                        tag,
                        &esc(tag),
                        "plus",
                        "quiet tag-search-option",
                        false,
                    ));
                }
                if count == 0 {
                    write!(html, "<p class=\"muted\">{}</p>", t(app, "choice.empty")).unwrap();
                }
                html.push_str("</div>");
            }
            html.push_str("</div>");
            if !selected.is_empty() {
                write!(
                    html,
                    r#"<div class="tag-filter-selected" aria-label="{}">"#,
                    t(app, "tags.selected")
                )
                .unwrap();
                for tag in selected {
                    write!(html, r#"<span class="selected-tag"><span>{}</span><button type="button" class="icon-button" data-action="tag-filter-remove" data-value="{}" aria-label="{}">{}</button></span>"#, esc(tag), esc(tag), esc(&f(app, "tags.remove", &[("name", tag)])), icon("close")).unwrap();
                }
                html.push_str("</div>");
            }
            write!(
                html,
                "<p class=\"tag-filter-help muted\">{}</p>",
                t(app, "tags.filter_help")
            )
            .unwrap();
        }
        Dialog::Edit { .. } => {
            let _ = write!(
                html,
                r#"<label class="edit-field">{}<input id="folder-name" data-input="folder-name" value="{}" autocomplete="off"/></label><div class="edit-field"><span>{}</span>"#,
                t(app, "folders.name"),
                esc(&features::form(app, "folder-name")),
                t(app, "folders.parent")
            );
            choices::control(
                html,
                app,
                "folder-parent",
                t(app, "folders.parent"),
                app.busy(),
            );
            html.push_str("</div>");
            html.push_str(&button(
                "folder-save",
                "",
                t(app, "common.save"),
                "check",
                "primary",
                app.busy(),
            ));
        }
        Dialog::Picker { ids } => {
            let _ = write!(
                html,
                r#"<p class="muted">{}</p><label class="edit-field"><span>{}</span><input id="folder-query" data-input="folder-query" type="search" value="{}" autocomplete="off"/></label><div class="folder-picker">"#,
                f(app, "bulk.selected", &[("count", &ids.len().to_string())]),
                t(app, "common.search"),
                esc(&features::form(app, "folder-query"))
            );
            let query = features::form(app, "folder-query").to_lowercase();
            for (folder, _) in ordered(&app.settings.collections, &app.library.expanded, false) {
                let path = folder_path(&app.settings.collections, &folder.id);
                if path.to_lowercase().contains(&query) {
                    html.push_str(&button(
                        "collection-pick",
                        &folder.id,
                        &esc(&path),
                        "folder_special",
                        "folder-picker-option",
                        app.busy(),
                    ));
                }
            }
            html.push_str("</div>");
            if app.settings.collections.is_empty() {
                let _ = write!(html, "<p>{}</p>", t(app, "folders.new_hint"));
            }
        }
        Dialog::Delete { id } => {
            let (folders, games) = affected(app, id);
            let _ = write!(
                html,
                "<p>{}</p>",
                esc(&f(
                    app,
                    "folders.delete_confirm",
                    &[
                        ("name", &folder_path(&app.settings.collections, id)),
                        ("folders", &folders.to_string()),
                        ("games", &games.to_string())
                    ]
                ))
            );
            html.push_str("<ul class=\"affected-folders\">");
            let removed = subtree(&app.settings.collections, id);
            for (folder, _) in ordered(&app.settings.collections, &app.library.expanded, false) {
                let path = folder_path(&app.settings.collections, &folder.id);
                if removed.contains(&folder.id) {
                    let _ = write!(html, "<li>{}</li>", esc(&path));
                }
            }
            html.push_str("</ul>");
            html.push_str(&button(
                "folder-delete-confirm",
                "",
                t(app, "common.confirm"),
                "close",
                "primary danger",
                app.busy(),
            ));
        }
        Dialog::Exclude { ids } => {
            let _ = write!(
                html,
                "<p>{}</p>",
                f(
                    app,
                    "bulk.exclude_confirm",
                    &[("count", &ids.len().to_string())]
                )
            );
            html.push_str(&button(
                "batch-exclude-confirm",
                "",
                t(app, "common.confirm"),
                "close",
                "primary danger",
                app.busy(),
            ));
        }
        Dialog::Warnings => {
            html.push_str("<ul>");
            for warning in &app.catalog.warnings {
                let _ = write!(
                    html,
                    "<li>{}</li>",
                    esc(&warning.render(app.settings.language))
                );
            }
            html.push_str("</ul>");
        }
        Dialog::Failures => {
            html.push_str("<ul>");
            for failure in &app.library.batch_failures {
                let _ = write!(
                    html,
                    "<li><strong>{}</strong><p>{}</p></li>",
                    esc(&failure.title),
                    esc(&failure.error.render(app.settings.language))
                );
            }
            html.push_str("</ul>");
            html.push_str(&button(
                "batch-retry",
                "",
                t(app, "bulk.retry"),
                "refresh",
                "primary",
                app.busy() || app.library.batch_failures.is_empty(),
            ));
        }
        Dialog::Restore => {
            if app.library.excluded.is_empty() {
                let _ = write!(html, "<p>{}</p>", t(app, "library.excluded_empty"));
            } else {
                html.push_str(&button(
                    "excluded-restore-all",
                    "",
                    t(app, "library.restore_all"),
                    "refresh",
                    "secondary",
                    app.busy(),
                ));
                for game in &app.library.excluded {
                    let _ = write!(
                        html,
                        "<div class=\"restore-entry\"><span>{}</span>{}</div>",
                        esc(&game.title),
                        button(
                            "excluded-restore",
                            &game.id,
                            t(app, "library.restore_one"),
                            "refresh",
                            "quiet",
                            app.busy()
                        )
                    );
                }
            }
        }
    }
    html.push_str("</div>");
    if matches!(dialog, Dialog::Tags { .. }) {
        html.push_str("<footer class=\"library-dialog-footer\">");
        html.push_str(&button(
            "tag-filter-apply",
            "",
            t(app, "common.confirm"),
            "check",
            "primary",
            false,
        ));
        html.push_str("</footer>");
    }
    html.push_str("</section></div>");
}
