//! Library view.
use crate::{
    choices, library::matching, models::Game, playtime, state::App, view::f, view::poster, view::t,
    widgets::button, widgets::esc, widgets::icon,
};
use std::{collections::BTreeSet, fmt::Write};

fn warnings_button(html: &mut String, app: &App) {
    if !app.catalog.warnings.is_empty() {
        html.push_str(&button(
            "library-warnings",
            "",
            &f(
                app,
                "games.warnings",
                &[("count", &app.catalog.warnings.len().to_string())],
            ),
            "error",
            "quiet warning-button",
            false,
        ));
    }
}

fn toolbar(html: &mut String, app: &App, games: &[&Game]) {
    let _ = write!(
        html,
        r#"<div class="library-bar {}" id="library-bar" role="toolbar" aria-label="{}">"#,
        if app.library.batch { "selecting" } else { "" },
        t(app, "games.title")
    );
    if app.library.batch {
        let _ = write!(
            html,
            "<span class=\"selection-total\">{}<strong>{}</strong></span>",
            icon("check"),
            f(
                app,
                "bulk.selected",
                &[("count", &app.library.checked.len().to_string())]
            )
        );
        html.push_str(&button(
            "batch-all",
            "",
            t(app, "bulk.all"),
            "done_all",
            "quiet icon-only",
            app.busy() || games.is_empty(),
        ));
        html.push_str(&button(
            "batch-clear",
            "",
            t(app, "common.cancel"),
            "close",
            "quiet",
            app.busy() || app.library.checked.is_empty(),
        ));
        html.push_str("<span class=\"bar-spacer\"></span>");
        html.push_str(&button(
            "collection-picker",
            "",
            t(app, "folders.add_games"),
            "folder_special",
            "secondary",
            app.busy() || app.library.checked.is_empty(),
        ));
        if app.library.collection.is_some() {
            html.push_str(&button(
                "batch-detach",
                "",
                t(app, "folders.detach"),
                "close",
                "quiet",
                app.busy() || app.library.checked.is_empty(),
            ));
        }
        html.push_str(&button(
            "batch-exclude",
            "",
            t(app, "bulk.exclude"),
            "close",
            "quiet danger",
            app.busy() || app.library.checked.is_empty(),
        ));
        if !app.library.batch_failures.is_empty() {
            html.push_str(&button(
                "batch-failures",
                "",
                t(app, "bulk.failures"),
                "error",
                "quiet danger",
                false,
            ));
        }
        warnings_button(html, app);
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.done"),
            "check",
            "primary",
            app.busy(),
        ));
    } else {
        let count = if app.library.collection.is_some()
            || !app.library.search.is_empty()
            || !app.library.tag_filter.is_empty()
        {
            format!("{} / {}", games.len(), app.catalog.games.len())
        } else {
            app.catalog.games.len().to_string()
        };
        let _ = write!(
            html,
            "<div class=\"library-stats\"><span><strong>{}</strong>{}</span><span><strong>{}</strong>{}</span><span class=\"library-playtime\"><strong>{}h</strong>{}</span></div>",
            app.settings.library_folders.len(),
            t(app, "library.libraries"),
            count,
            t(app, "library.games"),
            playtime::hours(games.iter().fold(0_u64, |seconds, game| {
                seconds.saturating_add(game.playtime_seconds)
            })),
            t(app, "library.playtime")
        );
        let _ = write!(
            html,
            r#"<div class="bar-controls"><label class="bar-search {}" for="search">{}<input id="search" data-input="search" type="search" placeholder="{}" value="{}" aria-label="{}" autocomplete="off"/></label>"#,
            if app.library.search.is_empty() {
                ""
            } else {
                "has-query"
            },
            icon("search"),
            t(app, "common.search"),
            esc(&app.library.search),
            t(app, "common.search")
        );
        html.push_str("<div class=\"tag-filter-control\">");
        let filter_label = if app.library.tag_filter.is_empty() {
            t(app, "games.tag_filter").to_owned()
        } else {
            app.library.tag_filter.join("、")
        };
        html.push_str(&button(
            "tag-filter-open",
            "",
            &esc(&filter_label),
            "filter_plus",
            if app.library.tag_filter.is_empty() {
                "secondary"
            } else {
                "secondary active"
            },
            false,
        ));
        if !app.library.tag_filter.is_empty() {
            write!(html,r#"<button type="button" class="icon-button clear-filter" data-action="tag-clear" aria-label="{}" title="{}">{}</button>"#,t(app,"tags.clear_filter"),t(app,"tags.clear_filter"),icon("filter_cancel")).unwrap();
        }
        html.push_str("</div>");
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.enter"),
            "checklist",
            "secondary icon-only",
            app.busy(),
        ));
        let _ = write!(
            html,
            r#"<details class="action-menu add-menu"><summary class="button primary icon-only" aria-label="{}" title="{}">{}</summary><div>{}{}</div></details>"#,
            t(app, "common.add"),
            t(app, "common.add"),
            icon("plus"),
            button(
                "choose-folder",
                "",
                t(app, "games.add_library"),
                "folder",
                "quiet",
                app.busy()
            ),
            button(
                "manual-game",
                "",
                t(app, "games.manual"),
                "plus",
                "quiet",
                app.busy()
            )
        );
        let refresh_label = if app.library.scanning {
            format!("{} {}", t(app, "common.cancel"), app.library.scan_folders)
        } else {
            t(app, "common.refresh").into()
        };
        let _ = write!(
            html,
            r#"<div class="refresh-control" title="{}">{}</div>"#,
            esc(&f(
                app,
                "games.scan_progress",
                &[
                    ("folders", &app.library.scan_folders.to_string()),
                    ("count", &app.catalog.games.len().to_string())
                ]
            )),
            button(
                if app.library.scanning {
                    "cancel-scan"
                } else {
                    "refresh"
                },
                "",
                &refresh_label,
                "refresh",
                if app.library.scanning {
                    "quiet scanning icon-only"
                } else {
                    "quiet icon-only"
                },
                !app.library.scanning && app.busy()
            )
        );
        warnings_button(html, app);
        choices::control(html, app, "sort", t(app, "games.sort_label"), false);
        html.push_str("</div>");
    }
    html.push_str("</div>");
}

fn collection_toolbar(html: &mut String, app: &App) {
    write!(
        html,
        r#"<div class="library-bar collection-bar {}" role="toolbar" aria-label="{}">"#,
        if app.library.batch { "selecting" } else { "" },
        t(app, "folders.title")
    )
    .unwrap();
    if app.library.batch {
        write!(
            html,
            "<span class=\"selection-total\"><strong>{}</strong></span>",
            f(
                app,
                "bulk.selected",
                &[(
                    "count",
                    &(app.library.checked.len() + app.library.checked_folders.len()).to_string()
                )]
            )
        )
        .unwrap();
        html.push_str(&button(
            "batch-all",
            "",
            t(app, "bulk.all"),
            "done_all",
            "quiet icon-only",
            app.busy(),
        ));
        html.push_str(&button(
            "batch-clear",
            "",
            t(app, "common.cancel"),
            "close",
            "quiet",
            app.busy(),
        ));
        html.push_str("<span class=\"bar-spacer\"></span>");
        if !app.library.checked_folders.is_empty() {
            html.push_str(&button(
                "folders-delete-selected",
                "",
                t(app, "folders.delete"),
                "trash",
                "quiet danger",
                app.busy(),
            ));
        }
        if app.library.collection.is_some() {
            html.push_str(&button(
                "batch-detach",
                "",
                t(app, "folders.detach"),
                "trash",
                "quiet danger",
                app.busy() || app.library.checked.is_empty(),
            ));
        }
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.done"),
            "check",
            "primary",
            app.busy(),
        ));
    } else {
        html.push_str(&button(
            "folder-new",
            app.library.collection.as_deref().unwrap_or(""),
            t(
                app,
                if app.library.collection.is_some() {
                    "folders.new_child"
                } else {
                    "folders.new"
                },
            ),
            "create_new_folder",
            "primary",
            app.busy(),
        ));
        if let Some(id) = &app.library.collection {
            html.push_str(&button(
                "folder-edit",
                id,
                t(app, "folders.edit"),
                "edit",
                "quiet",
                app.busy(),
            ));
        }
        html.push_str("<span class=\"bar-spacer\"></span>");
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.enter"),
            "checklist",
            "secondary icon-only",
            app.busy(),
        ));
        if app.library.collection.is_some() {
            html.push_str(&button(
                "folder-add-games",
                "",
                t(app, "folders.add_current"),
                "plus",
                "primary",
                app.busy(),
            ));
        }
    }
    html.push_str("</div>");
}

fn folder_cards(html: &mut String, app: &App) {
    html.push_str("<div class=\"folder-grid\">");
    for folder in app
        .settings
        .collections
        .iter()
        .filter(|f| f.parent == app.library.collection)
    {
        let selected = app.library.checked_folders.contains(&folder.id);
        write!(html,r#"<article class="folder-card {}" data-folder-id="{}" data-drop-folder="{}"><button class="folder-face" data-action="{}" data-value="{}" {}>{}<strong>{}</strong></button>"#,
            if selected{"is-selected"}else{""},esc(&folder.id),esc(&folder.id),
            if app.library.batch{"folder-select"}else{"folder-open"},esc(&folder.id),
            if app.library.batch {format!("aria-pressed=\"{selected}\"")}else{String::new()},icon("folder"),esc(&folder.name)).unwrap();
        if app.library.batch {
            write!(
                html,
                r#"<span class="selection-check" aria-hidden="true">{}</span>"#,
                if selected {
                    icon("check")
                } else {
                    String::new()
                }
            )
            .unwrap();
        } else {
            write!(html,r#"<button class="card-remove icon-button" data-action="folder-remove-select" data-value="{}" aria-label="{}" title="{}">{}</button>"#,esc(&folder.id),t(app,"folders.delete"),t(app,"folders.delete"),icon("trash")).unwrap();
        }
        html.push_str("</article>");
    }
    html.push_str("</div>");
}

pub(crate) fn page(html: &mut String, app: &App) {
    page_content(html, app);
}

pub(super) fn breadcrumbs(html: &mut String, app: &App, id: &str) {
    let mut ancestors = Vec::new();
    let mut next = Some(id);
    let mut visited = BTreeSet::new();
    while let Some(id) = next {
        if !visited.insert(id) {
            break;
        }
        let Some(folder) = app.settings.collections.iter().find(|f| f.id == id) else {
            break;
        };
        ancestors.push(folder);
        next = folder.parent.as_deref();
    }
    ancestors.reverse();
    write!(html, r#"<nav class="folder-breadcrumbs" aria-label="{}"><button data-action="page" data-value="collections">{}</button>"#, t(app,"folders.path"),t(app,"folders.home")).unwrap();
    for folder in ancestors {
        html.push_str("<span aria-hidden=\"true\">/</span>");
        if folder.id == id {
            write!(
                html,
                r#"<span aria-current="page">{}</span>"#,
                esc(&folder.name)
            )
            .unwrap();
        } else {
            write!(
                html,
                r#"<button data-action="folder-open" data-value="{}">{}</button>"#,
                esc(&folder.id),
                esc(&folder.name)
            )
            .unwrap();
        }
    }
    html.push_str("</nav>");
}

fn page_content(html: &mut String, app: &App) {
    let folder = app
        .library
        .collection
        .as_ref()
        .and_then(|id| app.settings.collections.iter().find(|f| &f.id == id));
    let title = t(
        app,
        if app.page == "collections" {
            "folders.title"
        } else {
            "games.title"
        },
    );
    let _ = write!(
        html,
        r#"<section class="collection library-page" aria-label="{}">"#,
        esc(title)
    );
    if app.page == "collections"
        && let Some(folder) = folder
    {
        breadcrumbs(html, app, &folder.id);
    }
    let games = matching(&app.catalog, &app.settings, &app.library, &app.page);
    if app.page == "collections" {
        collection_toolbar(html, app);
        folder_cards(html, app);
        if app.library.collection.is_none() {
            html.push_str("</section>");
            return;
        }
    } else {
        toolbar(html, app, &games);
    }
    if games.is_empty() {
        let empty_folder =
            folder.is_some() && app.library.search.is_empty() && app.library.tag_filter.is_empty();
        let _ = write!(
            html,
            "<div class=\"empty-state\"><span class=\"empty-icon\">{}</span><h3>{}</h3><p>{}</p>{}</div>",
            icon(if folder.is_some() {
                "folder_special"
            } else {
                "games"
            }),
            t(
                app,
                if empty_folder {
                    "folders.empty"
                } else if app.catalog.games.is_empty() {
                    "games.none"
                } else {
                    "games.no_match"
                }
            ),
            t(
                app,
                if empty_folder {
                    "folders.empty_hint"
                } else if app.catalog.games.is_empty() {
                    "games.none_description"
                } else {
                    "games.no_match_description"
                }
            ),
            button(
                if folder.is_some() {
                    "folder-add-games"
                } else {
                    "page"
                },
                if folder.is_some() { "" } else { "games" },
                t(
                    app,
                    if folder.is_some() {
                        "folders.add_current"
                    } else {
                        "nav.games"
                    }
                ),
                if folder.is_some() { "plus" } else { "games" },
                "secondary",
                false
            )
        );
    } else {
        html.push_str("<div class=\"game-grid\">");
        for (index, game) in games.iter().take(app.library.visible_count).enumerate() {
            let selected = app.library.checked.contains(&game.id);
            let _ = write!(
                html,
                r#"<article class="game-card {} {}" data-game-id="{}" draggable="false"><button id="game-{}" class="game-face" data-action="{}" data-value="{}" aria-label="{}" {}><div class="poster poster-{}">"#,
                if selected { "is-selected" } else { "" },
                if app.library.batch {
                    "is-selectable"
                } else {
                    ""
                },
                esc(&game.id),
                esc(&game.id),
                if app.library.batch {
                    "batch-select"
                } else {
                    "game-detail"
                },
                esc(&game.id),
                esc(&game.title),
                if app.library.batch {
                    format!("aria-pressed=\"{selected}\"")
                } else {
                    String::new()
                },
                index % 5
            );
            poster(html, app, game);
            if app.library.batch {
                let _ = write!(
                    html,
                    "<span class=\"selection-check\" aria-hidden=\"true\">{}</span>",
                    if selected {
                        icon("check")
                    } else {
                        String::new()
                    }
                );
            }
            html.push_str("</div></button>");
            if !app.library.batch {
                card_overlay(html, app, game);
            }
            html.push_str("</article>");
        }
        html.push_str("</div>");
        if games.len() > app.library.visible_count {
            html.push_str(&button(
                "load-more",
                "",
                t(app, "games.more"),
                "plus",
                "quiet",
                false,
            ));
        }
    }
    html.push_str("</section>");
}

fn card_overlay(html: &mut String, app: &App, game: &Game) {
    let _ = write!(
        html,
        r#"<div class="card-overlay"><div class="card-heading"><h3><button type="button" class="card-title" data-action="game-detail" data-value="{}" title="{}">{}</button></h3>"#,
        esc(&game.id),
        esc(&game.title),
        esc(&game.title)
    );
    if !game.available {
        let _ = write!(
            html,
            r#"<span class="path-warning" role="img" aria-label="{}" title="{}">{}</span>"#,
            t(app, "library.path_warning"),
            t(app, "library.path_warning"),
            icon("error")
        );
    }
    let _ = write!(
        html,
        r#"</div><div class="card-tags" aria-label="{}">"#,
        t(app, "detail.tags")
    );
    for tag in game.tags.iter().take(3) {
        let _ = write!(html, "<span class=\"card-tag\">{}</span>", esc(tag));
    }
    if game.tags.is_empty() {
        let _ = write!(html, "<p class=\"muted\">{}</p>", t(app, "detail.no_tags"));
    }
    html.push_str("</div><div class=\"card-actions\">");
    crate::game_actions::render(html, app, game);
    html.push_str(&crate::widgets::icon_button(
        "card-delete",
        &game.id,
        t(app, "context.delete"),
        "trash",
        "card-action delete",
        app.busy(),
    ));
    html.push_str("</div></div>");
}
