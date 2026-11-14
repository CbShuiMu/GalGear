//! Library dialogs.
use crate::{
    choices, features, library::Dialog, library::filter_brands, library::filter_tags,
    library::folder_path, library::ordered, library::subtree, state::App, view::f, view::form_field,
    view::t, widgets::button, widgets::esc, widgets::icon,
};
use std::{collections::BTreeSet, fmt::Write};

struct TagSearch {
    field: &'static str,
    label: &'static str,
    placeholder: &'static str,
    action: &'static str,
    show_all: bool,
}

const BRAND_SEARCH: TagSearch = TagSearch {
    field: "brand-query",
    label: "tags.brand_search",
    placeholder: "tags.brand_search_placeholder",
    action: "brand-filter",
    show_all: true,
};

const TAG_SEARCH: TagSearch = TagSearch {
    field: "tag-query",
    label: "common.search",
    placeholder: "tags.search_placeholder",
    action: "tag-filter",
    show_all: false,
};

fn tag_search(
    html: &mut String,
    app: &App,
    search: &TagSearch,
    selected: &[String],
    names: impl Iterator<Item = String>,
) {
    let TagSearch {
        field,
        label,
        placeholder,
        action,
        show_all,
    } = *search;
    let query = features::form(app, field);
    let show_results = show_all || !query.trim().is_empty();
    let controls = if show_results {
        format!(r#" aria-controls="{field}-results""#)
    } else {
        String::new()
    };
    write!(html, r#"<div class="tag-search"><label class="edit-field"><span>{}</span><input id="{field}" data-input="{field}" type="search" value="{}" placeholder="{}" autocomplete="off"{controls}/></label>"#, t(app, label), esc(&query), t(app, placeholder)).unwrap();
    if show_results {
        let selected: BTreeSet<_> = selected.iter().map(|name| galgear_tags::key(name)).collect();
        let query = query.trim().to_lowercase();
        let mut names = galgear_tags::normalize(names);
        names.retain(|name| {
            name.to_lowercase().contains(&query) && !selected.contains(&galgear_tags::key(name))
        });
        names.sort_by_cached_key(|name| name.to_lowercase());
        write!(html, r#"<div id="{field}-results" class="tag-search-results">"#).unwrap();
        if names.is_empty() {
            write!(html, "<p class=\"muted\">{}</p>", t(app, "choice.empty")).unwrap();
        }
        for name in &names {
            html.push_str(&button(
                action,
                name,
                &esc(name),
                "",
                "quiet tag-search-option",
                false,
            ));
        }
        html.push_str("</div>");
    }
    html.push_str("</div>");
}

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
        Dialog::RemoveLibrary { .. } => "settings.remove_folder",
        Dialog::Warnings => "library.warnings_title",
        Dialog::Failures => "bulk.failures",
        Dialog::FolderFailures { .. } => "library.folder_failures",
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
        }
        Dialog::Tags {
            selected,
            brands,
            status,
        } => {
            write!(html, r#"<fieldset class="tag-status-filter"><legend>{}</legend><div>"#, t(app, "tags.game_status")).unwrap();
            for (value, key, active) in status.options() {
                write!(html, r#"<button type="button" class="button secondary {}" data-action="tag-status" data-value="{}" aria-pressed="{}">{}</button>"#,
                    if active { "active" } else { "" }, value, active, t(app, key)).unwrap();
            }
            html.push_str("</div></fieldset>");
            let games = &app.catalog.games;
            tag_search(
                html,
                app,
                &BRAND_SEARCH,
                brands,
                games.iter().flat_map(filter_brands),
            );
            tag_search(
                html,
                app,
                &TAG_SEARCH,
                selected,
                games.iter().flat_map(filter_tags),
            );
            if !selected.is_empty() || !brands.is_empty() {
                write!(
                    html,
                    r#"<div class="tag-filter-selected" aria-label="{}">"#,
                    t(app, "tags.selected")
                )
                .unwrap();
                for (names, action) in
                    [(brands, "brand-filter-remove"), (selected, "tag-filter-remove")]
                {
                    for name in names {
                        write!(html, r#"<span class="selected-tag"><span>{}</span><button type="button" class="icon-button" data-action="{action}" data-value="{}" aria-label="{}">{}</button></span>"#, esc(name), esc(name), esc(&f(app, "tags.remove", &[("name", name)])), icon("close")).unwrap();
                    }
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
                if super::accepts_games(&app.settings.collections, &folder.id) && path.to_lowercase().contains(&query) {
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
        }
        Dialog::RemoveLibrary { path } => {
            write!(
                html,
                "<p>{}</p><p>{}</p>",
                esc(path),
                t(app, "settings.remove_folder_tip")
            )
            .unwrap();
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
        Dialog::FolderFailures { failed, page } => {
            crate::library::folders::render_failures(html, app, failed, *page);
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
    let commit = match dialog {
        Dialog::AddGames => Some((
            "picker-games-save",
            "common.add",
            app.busy() || app.library.picker_games.is_empty(),
        )),
        Dialog::DeleteFolders { .. } => {
            Some(("folders-delete-confirm", "common.confirm", app.busy()))
        }
        Dialog::Edit { .. } => Some(("folder-save", "common.save", app.busy())),
        Dialog::Delete { .. } => Some(("folder-delete-confirm", "common.confirm", app.busy())),
        Dialog::Exclude { .. } => Some(("batch-exclude-confirm", "context.delete", app.busy())),
        Dialog::RemoveLibrary { .. } => Some((
            "remove-folder-confirm",
            "common.confirm",
            app.busy() || app.library.scanning,
        )),
        Dialog::Tags { .. } => Some(("tag-filter-apply", "common.confirm", false)),
        _ => None,
    };
    html.push_str("<footer class=\"library-dialog-footer dialog-actions\">");
    html.push_str(&button(
        "library-close",
        "",
        t(
            app,
            if commit.is_some() {
                "common.cancel"
            } else {
                "common.close"
            },
        ),
        "close",
        "quiet",
        false,
    ));
    if let Some((action, label, disabled)) = commit {
        html.push_str(&button(
            action,
            "",
            t(app, label),
            "check",
            "primary",
            disabled,
        ));
    }
    html.push_str("</footer>");
    html.push_str("</section></div>");
}
