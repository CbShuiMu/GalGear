//! Library presentation, selection and collection interactions.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct Collection {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub games: BTreeSet<String>,
}
#[derive(Clone)]
pub(super) enum Dialog {
    Edit { id: Option<String> },
    Picker { ids: Vec<String> },
    Delete { id: String },
    Exclude { ids: Vec<String> },
    Warnings,
    Failures,
    Restore,
    Tags,
    AddGames,
    DeleteFolders { ids: Vec<String> },
}
#[derive(Clone, Deserialize)]
pub(super) struct Failure {
    pub id: String,
    title: String,
    error: Message,
}
#[derive(Deserialize)]
pub(super) struct BatchResult {
    pub settings: Settings,
    pub catalog: Catalog,
    pub succeeded: Vec<String>,
    pub failed: Vec<Failure>,
    pub unchanged: usize,
    pub cache_warning: Option<Message>,
}

pub(super) fn clear_selection(app: &mut App) {
    app.checked.clear();
    app.checked_folders.clear();
    app.batch_failures.clear();
    app.visible_count = 120;
}

pub(super) fn matching(app: &App) -> Vec<&Game> {
    if app.page == "collections" && app.collection.is_none() {
        return Vec::new();
    }
    let query = app.search.to_lowercase();
    let folder = app
        .collection
        .as_ref()
        .and_then(|id| app.settings.collections.iter().find(|f| &f.id == id));
    let mut games: Vec<_> = app
        .catalog
        .games
        .iter()
        .filter(|g| {
            (app.collection.is_none() || folder.is_some_and(|f| f.games.contains(&g.id)))
                && (g.title.to_lowercase().contains(&query)
                    || g.tags.iter().any(|tag| tag.to_lowercase().contains(&query)))
                && (app.tag_filter.is_empty() || g.tags.contains(&app.tag_filter))
        })
        .collect();
    if app.settings.sort_by == "recent" {
        games.sort_by_key(|g| std::cmp::Reverse(g.modified));
    } else {
        games.sort_by_cached_key(|g| g.title.to_lowercase());
    }
    games
}

pub(super) fn folder_path(app: &App, id: &str) -> String {
    let mut parts = Vec::new();
    let mut next = Some(id);
    let mut visited = BTreeSet::new();
    while let Some(id) = next {
        if !visited.insert(id) {
            break;
        }
        let Some(folder) = app.settings.collections.iter().find(|f| f.id == id) else {
            break;
        };
        parts.push(folder.name.as_str());
        next = folder.parent.as_deref();
    }
    parts.reverse();
    parts.join(" / ")
}

pub(super) fn subtree(app: &App, id: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::from([id.to_owned()]);
    loop {
        let before = ids.len();
        for folder in &app.settings.collections {
            if folder.parent.as_ref().is_some_and(|p| ids.contains(p)) {
                ids.insert(folder.id.clone());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

pub(super) fn ordered(app: &App, expanded_only: bool) -> Vec<(&Collection, usize)> {
    let mut result = Vec::new();
    let mut stack: Vec<_> = app
        .settings
        .collections
        .iter()
        .rev()
        .filter(|f| f.parent.is_none())
        .map(|f| (f, 0))
        .collect();
    let mut visited = BTreeSet::new();
    while let Some((folder, depth)) = stack.pop() {
        if !visited.insert(&folder.id) {
            continue;
        }
        result.push((folder, depth));
        if !expanded_only || app.expanded.contains(&folder.id) {
            stack.extend(
                app.settings
                    .collections
                    .iter()
                    .rev()
                    .filter(|f| f.parent.as_deref() == Some(&folder.id))
                    .map(|f| (f, depth + 1)),
            );
        }
    }
    result
}

fn batch(app: &mut App, operation: &str, ids: Vec<String>, collection: Option<String>) {
    if ids.is_empty() {
        return;
    }
    app.last_batch = Some((operation.into(), collection.clone()));
    issue(
        app,
        Pending::Batch,
        "batch_games",
        json!({"operation":operation,"ids":ids,"collection":collection}),
    );
}

fn open_edit(app: &mut App, id: Option<String>, parent: Option<String>) {
    let name = id
        .as_ref()
        .and_then(|id| app.settings.collections.iter().find(|f| &f.id == id))
        .map(|f| f.name.clone())
        .unwrap_or_default();
    app.form.insert("folder-name".into(), name);
    app.form
        .insert("folder-parent".into(), parent.unwrap_or_default());
    app.library_dialog = Some(Dialog::Edit { id });
}

pub(super) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    match action {
        "tag-filter-open" => {
            app.library_dialog = Some(Dialog::Tags);
            app.form.insert("tag-query".into(), String::new());
            app.choice = None;
        }
        "tag-clear" => {
            app.tag_filter.clear();
            clear_selection(app);
        }
        "folder-tree" => app.tree_open = !app.tree_open,
        "folder-expand" | "folder-hover" => {
            if action == "folder-hover" || !app.expanded.remove(value) {
                app.expanded.insert(value.into());
            }
            app.tree_open = true;
        }
        "folder-open" => {
            if !app.settings.collections.iter().any(|f| f.id == value) {
                return true;
            }
            clear_selection(app);
            app.batch = false;
            app.collection = Some(value.into());
            app.page = "collections".into();
            app.selected = None;
            app.choice = None;
            app.search.clear();
            app.tag_filter.clear();
            reset_scroll();
        }
        "folder-new" => open_edit(app, None, (!value.is_empty()).then(|| value.into())),
        "folder-edit" => {
            let parent = app
                .settings
                .collections
                .iter()
                .find(|f| f.id == value)
                .and_then(|f| f.parent.clone());
            open_edit(app, Some(value.into()), parent);
        }
        "folder-parent" => {
            app.form.insert("folder-parent".into(), value.into());
        }
        "folder-save" => {
            if let Some(Dialog::Edit { id }) = &app.library_dialog {
                let id = id.clone();
                let parent = features::form(app, "folder-parent");
                issue(
                    app,
                    Pending::CollectionSave,
                    "edit_collection",
                    json!({"operation":if id.is_some(){"update"}else{"create"},"id":id.unwrap_or_default(),"name":features::form(app,"folder-name"),"parent":if parent.is_empty(){None}else{Some(parent)}}),
                );
            }
        }
        "folder-delete" => app.library_dialog = Some(Dialog::Delete { id: value.into() }),
        "folder-delete-confirm" => {
            if let Some(Dialog::Delete { id }) = &app.library_dialog {
                let id = id.clone();
                issue(
                    app,
                    Pending::CollectionSave,
                    "edit_collection",
                    json!({"operation":"delete","id":id,"name":"","parent":null}),
                );
            }
        }
        "library-close" => {
            app.library_dialog = None;
            app.choice = None;
        }
        "batch-toggle" => {
            app.batch = !app.batch;
            clear_selection(app);
        }
        "batch-select" => {
            if app.batch && !app.checked.remove(value) {
                app.checked.insert(value.into());
            }
        }
        "folder-select" => {
            if app.batch && !app.checked_folders.remove(value) {
                app.checked_folders.insert(value.into());
            }
        }
        "card-remove-select" => {
            app.batch = true;
            app.checked.insert(value.into());
        }
        "folder-remove-select" => {
            app.batch = true;
            app.checked_folders.insert(value.into());
        }
        "folders-delete-selected" => {
            app.library_dialog = Some(Dialog::DeleteFolders {
                ids: app.checked_folders.iter().cloned().collect(),
            });
        }
        "folders-delete-confirm" => {
            if let Some(Dialog::DeleteFolders { ids }) = &app.library_dialog {
                let ids = ids.clone();
                issue(
                    app,
                    Pending::CollectionSave,
                    "delete_collections",
                    json!({"ids": ids}),
                );
            }
        }
        "folder-add-games" => {
            app.picker_games.clear();
            app.form.insert("game-query".into(), String::new());
            app.library_dialog = Some(Dialog::AddGames);
        }
        "picker-game" => {
            if !app.picker_games.remove(value) {
                app.picker_games.insert(value.into());
            }
        }
        "picker-games-save" => batch(
            app,
            "add",
            app.picker_games.iter().cloned().collect(),
            app.collection.clone(),
        ),
        "batch-all" => {
            app.checked = matching(app).iter().map(|g| g.id.clone()).collect();
            if app.page == "collections" {
                app.checked_folders = app
                    .settings
                    .collections
                    .iter()
                    .filter(|f| f.parent == app.collection)
                    .map(|f| f.id.clone())
                    .collect();
            }
        }
        "batch-clear" => {
            app.checked.clear();
            app.checked_folders.clear();
            app.batch_failures.clear();
        }
        "collection-picker" => {
            let ids = if value.is_empty() {
                app.checked.iter().cloned().collect()
            } else {
                vec![value.into()]
            };
            app.form.insert("folder-query".into(), String::new());
            app.library_dialog = Some(Dialog::Picker { ids });
        }
        "collection-pick" => {
            if let Some(Dialog::Picker { ids }) = &app.library_dialog {
                batch(app, "add", ids.clone(), Some(value.into()));
            }
        }
        "batch-exclude" => {
            app.library_dialog = Some(Dialog::Exclude {
                ids: app.checked.iter().cloned().collect(),
            })
        }
        "batch-exclude-confirm" => {
            if let Some(Dialog::Exclude { ids }) = &app.library_dialog {
                batch(app, "exclude", ids.clone(), None);
            }
        }
        "batch-detach" => batch(
            app,
            "detach",
            app.checked.iter().cloned().collect(),
            app.collection.clone(),
        ),
        "batch-failures" => app.library_dialog = Some(Dialog::Failures),
        "batch-retry" => {
            if let Some((operation, folder)) = app.last_batch.clone() {
                batch(
                    app,
                    &operation,
                    app.batch_failures.iter().map(|f| f.id.clone()).collect(),
                    folder,
                );
            }
        }
        "library-warnings" => app.library_dialog = Some(Dialog::Warnings),
        "excluded-open" => {
            app.library_dialog = Some(Dialog::Restore);
            issue(app, Pending::Excluded, "excluded_entries", json!({}));
        }
        "excluded-restore" => batch(app, "restore", vec![value.into()], None),
        "excluded-restore-all" => batch(
            app,
            "restore",
            app.excluded.iter().map(|g| g.id.clone()).collect(),
            None,
        ),
        _ => return false,
    }
    true
}

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
        if app.batch { "selecting" } else { "" },
        t(app, "games.title")
    );
    if app.batch {
        let _ = write!(
            html,
            "<span class=\"selection-total\">{}<strong>{}</strong></span>",
            icon("check"),
            f(
                app,
                "bulk.selected",
                &[("count", &app.checked.len().to_string())]
            )
        );
        html.push_str(&button(
            "batch-all",
            "",
            t(app, "bulk.all"),
            "check",
            "quiet",
            app.busy || games.is_empty(),
        ));
        html.push_str(&button(
            "batch-clear",
            "",
            t(app, "bulk.clear"),
            "close",
            "quiet",
            app.busy || app.checked.is_empty(),
        ));
        html.push_str("<span class=\"bar-spacer\"></span>");
        html.push_str(&button(
            "collection-picker",
            "",
            t(app, "folders.add_games"),
            "folder_special",
            "secondary",
            app.busy || app.checked.is_empty(),
        ));
        if app.collection.is_some() {
            html.push_str(&button(
                "batch-detach",
                "",
                t(app, "folders.detach"),
                "close",
                "quiet",
                app.busy || app.checked.is_empty(),
            ));
        }
        html.push_str(&button(
            "batch-exclude",
            "",
            t(app, "bulk.exclude"),
            "close",
            "quiet danger",
            app.busy || app.checked.is_empty(),
        ));
        if !app.batch_failures.is_empty() {
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
            app.busy,
        ));
    } else {
        let count =
            if app.collection.is_some() || !app.search.is_empty() || !app.tag_filter.is_empty() {
                format!("{} / {}", games.len(), app.catalog.games.len())
            } else {
                app.catalog.games.len().to_string()
            };
        let _ = write!(
            html,
            "<div class=\"library-stats\"><span><strong>{}</strong>{}</span><span><strong>{}</strong>{}</span></div>",
            app.settings.library_folders.len(),
            t(app, "library.libraries"),
            count,
            t(app, "library.games")
        );
        let _ = write!(
            html,
            r#"<div class="bar-controls"><label class="bar-search {}" for="search">{}<input id="search" data-input="search" type="search" placeholder="{}" value="{}" aria-label="{}" autocomplete="off"/></label>"#,
            if app.search.is_empty() {
                ""
            } else {
                "has-query"
            },
            icon("search"),
            t(app, "common.search"),
            esc(&app.search),
            t(app, "common.search")
        );
        html.push_str("<div class=\"tag-filter-control\">");
        html.push_str(&button(
            "tag-filter-open",
            "",
            if app.tag_filter.is_empty() {
                t(app, "games.tag_filter")
            } else {
                &app.tag_filter
            },
            "",
            if app.tag_filter.is_empty() {
                "secondary"
            } else {
                "secondary active"
            },
            false,
        ));
        if !app.tag_filter.is_empty() {
            html.push_str(&button(
                "tag-clear",
                "",
                "",
                "close",
                "quiet clear-filter",
                false,
            ));
        }
        html.push_str("</div>");
        choices::control(html, app, "sort", t(app, "games.sort_label"), false);
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.enter"),
            "check",
            "secondary",
            app.busy,
        ));
        let _ = write!(
            html,
            r#"<details class="action-menu add-menu"><summary class="button primary">{}{}</summary><div>{}{}</div></details>"#,
            icon("plus"),
            t(app, "common.add"),
            button(
                "choose-folder",
                "",
                t(app, "games.add_library"),
                "folder",
                "quiet",
                app.busy
            ),
            button(
                "manual-game",
                "",
                t(app, "games.manual"),
                "plus",
                "quiet",
                app.busy
            )
        );
        let refresh_label = if app.scanning {
            format!("{} {}", t(app, "common.cancel"), app.scan_folders)
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
                    ("folders", &app.scan_folders.to_string()),
                    ("count", &app.catalog.games.len().to_string())
                ]
            )),
            button(
                if app.scanning {
                    "cancel-scan"
                } else {
                    "refresh"
                },
                "",
                &refresh_label,
                "refresh",
                if app.scanning {
                    "quiet scanning"
                } else {
                    "quiet"
                },
                !app.scanning && app.busy
            )
        );
        warnings_button(html, app);
        html.push_str("</div>");
    }
    html.push_str("</div>");
}

fn collection_toolbar(html: &mut String, app: &App) {
    write!(
        html,
        r#"<div class="library-bar collection-bar {}" role="toolbar" aria-label="{}">"#,
        if app.batch { "selecting" } else { "" },
        t(app, "folders.title")
    )
    .unwrap();
    if app.batch {
        write!(
            html,
            "<span class=\"selection-total\"><strong>{}</strong></span>",
            f(
                app,
                "bulk.selected",
                &[(
                    "count",
                    &(app.checked.len() + app.checked_folders.len()).to_string()
                )]
            )
        )
        .unwrap();
        html.push_str(&button(
            "batch-all",
            "",
            t(app, "bulk.all"),
            "check",
            "quiet",
            app.busy,
        ));
        html.push_str(&button(
            "batch-clear",
            "",
            t(app, "bulk.clear"),
            "close",
            "quiet",
            app.busy,
        ));
        html.push_str("<span class=\"bar-spacer\"></span>");
        if !app.checked_folders.is_empty() {
            html.push_str(&button(
                "folders-delete-selected",
                "",
                t(app, "folders.delete"),
                "trash",
                "quiet danger",
                app.busy,
            ));
        }
        if app.collection.is_some() {
            html.push_str(&button(
                "batch-detach",
                "",
                t(app, "folders.detach"),
                "trash",
                "quiet danger",
                app.busy || app.checked.is_empty(),
            ));
        }
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.done"),
            "check",
            "primary",
            app.busy,
        ));
    } else {
        html.push_str(&button(
            "folder-new",
            app.collection.as_deref().unwrap_or(""),
            t(
                app,
                if app.collection.is_some() {
                    "folders.new_child"
                } else {
                    "folders.new"
                },
            ),
            "plus",
            "primary",
            app.busy,
        ));
        if let Some(id) = &app.collection {
            html.push_str(&button(
                "folder-edit",
                id,
                t(app, "folders.edit"),
                "edit",
                "quiet",
                app.busy,
            ));
        }
        html.push_str("<span class=\"bar-spacer\"></span>");
        html.push_str(&button(
            "batch-toggle",
            "",
            t(app, "bulk.enter"),
            "check",
            "secondary",
            app.busy,
        ));
        if app.collection.is_some() {
            html.push_str(&button(
                "folder-add-games",
                "",
                t(app, "folders.add_current"),
                "plus",
                "primary",
                app.busy,
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
        .filter(|f| f.parent == app.collection)
    {
        let selected = app.checked_folders.contains(&folder.id);
        write!(html,r#"<article class="folder-card {}" data-folder-id="{}" data-drop-folder="{}"><button class="folder-face" data-action="{}" data-value="{}" {}>{}<strong>{}</strong></button>"#,
            if selected{"is-selected"}else{""},esc(&folder.id),esc(&folder.id),
            if app.batch{"folder-select"}else{"folder-open"},esc(&folder.id),
            if app.batch {format!("aria-pressed=\"{selected}\"")}else{String::new()},icon("folder"),esc(&folder.name)).unwrap();
        if app.batch {
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

pub(super) fn page(html: &mut String, app: &App) {
    page_content(html, app);
}

fn breadcrumbs(html: &mut String, app: &App, id: &str) {
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
        .collection
        .as_ref()
        .and_then(|id| app.settings.collections.iter().find(|f| &f.id == id));
    let title = folder.map_or_else(
        || {
            t(
                app,
                if app.page == "collections" {
                    "folders.title"
                } else {
                    "games.title"
                },
            )
            .to_owned()
        },
        |_| t(app, "folders.title").to_owned(),
    );
    let _ = write!(
        html,
        r#"<section class="collection library-page"><header class="page-heading section-heading"><h1 title="{}">{}</h1>"#,
        esc(&title),
        esc(&title)
    );
    html.push_str("</header>");
    if app.page == "collections" {
        breadcrumbs(html, app, folder.map_or("", |f| f.id.as_str()));
    }
    let games = matching(app);
    if app.page == "collections" {
        collection_toolbar(html, app);
        folder_cards(html, app);
        if app.collection.is_none() {
            html.push_str("</section>");
            return;
        }
    } else {
        toolbar(html, app, &games);
    }
    if games.is_empty() {
        let empty_folder = folder.is_some() && app.search.is_empty() && app.tag_filter.is_empty();
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
        for (index, game) in games.iter().take(app.visible_count).enumerate() {
            let selected = app.checked.contains(&game.id);
            let _ = write!(
                html,
                r#"<article class="game-card {} {}" data-game-id="{}" draggable="false"><button id="game-{}" class="game-face" data-action="{}" data-value="{}" aria-label="{}" {}><div class="poster poster-{}">"#,
                if selected { "is-selected" } else { "" },
                if app.batch { "is-selectable" } else { "" },
                esc(&game.id),
                esc(&game.id),
                if app.batch {
                    "batch-select"
                } else {
                    "game-detail"
                },
                esc(&game.id),
                esc(&game.title),
                if app.batch {
                    format!("aria-pressed=\"{selected}\"")
                } else {
                    String::new()
                },
                index % 5
            );
            poster(html, app, game);
            if app.batch {
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
            if !app.batch {
                card_overlay(html, app, game);
            }
            html.push_str("</article>");
        }
        html.push_str("</div>");
        if games.len() > app.visible_count {
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
    let collected = app
        .settings
        .collections
        .iter()
        .any(|collection| collection.games.contains(&game.id));
    let finished = game.play_status == "finished";
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
    for (action, key, symbol, kind, active) in [
        (
            "collection-picker",
            "card.favorite",
            "star",
            "favorite",
            collected,
        ),
        (
            "card-finished",
            "card.finished",
            "trophy",
            "finished",
            finished,
        ),
        ("card-delete", "context.delete", "trash", "delete", false),
    ] {
        let _ = write!(
            html,
            r#"<button type="button" class="card-action {} {}" data-action="{}" data-value="{}" aria-label="{}" title="{}" {} {}>{}</button>"#,
            kind,
            if active { "is-active" } else { "" },
            action,
            esc(&game.id),
            t(app, key),
            t(app, key),
            match action {
                "collection-picker" => "aria-haspopup=\"dialog\"".to_owned(),
                "card-finished" => format!("aria-pressed=\"{finished}\""),
                _ => String::new(),
            },
            if app.busy { "disabled" } else { "" },
            icon(if symbol == "trophy" && active {
                "trophy-filled"
            } else {
                symbol
            })
        );
    }
    html.push_str("</div></div>");
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

pub(super) fn dialog(html: &mut String, app: &App) {
    let Some(dialog) = &app.library_dialog else {
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
        Dialog::Tags => "games.tag_filter",
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
                .find(|f| Some(&f.id) == app.collection.as_ref());
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
                    if app.picker_games.contains(&game.id){"selected"}else{""},esc(&game.id),app.picker_games.contains(&game.id),
                    if app.picker_games.contains(&game.id){icon("check")}else{String::new()},esc(&game.title)).unwrap();
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
                app.busy || app.picker_games.is_empty(),
            ));
        }
        Dialog::DeleteFolders { ids } => {
            write!(html, "<p>{}</p>", t(app, "folders.batch_delete_help")).unwrap();
            html.push_str("<ul>");
            for id in ids {
                write!(html, "<li>{}</li>", esc(&folder_path(app, id))).unwrap();
            }
            html.push_str("</ul>");
            html.push_str(&button(
                "folders-delete-confirm",
                "",
                t(app, "common.confirm"),
                "trash",
                "primary danger",
                app.busy,
            ));
        }
        Dialog::Tags => {
            form_field(html, app, "tag-query", "common.search", false);
            let query = features::form(app, "tag-query").to_lowercase();
            let tags: BTreeSet<_> = app
                .catalog
                .games
                .iter()
                .flat_map(|game| game.tags.iter())
                .collect();
            html.push_str("<div class=\"tag-filter-options\">");
            for tag in tags {
                if tag.to_lowercase().contains(&query) {
                    html.push_str(&button(
                        "tag-filter",
                        tag,
                        &esc(tag),
                        "",
                        if app.tag_filter == *tag {
                            "primary"
                        } else {
                            "secondary"
                        },
                        false,
                    ));
                }
            }
            html.push_str("</div>");
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
                app.busy,
            );
            html.push_str("</div>");
            html.push_str(&button(
                "folder-save",
                "",
                t(app, "common.save"),
                "check",
                "primary",
                app.busy,
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
            for (folder, _) in ordered(app, false) {
                let path = folder_path(app, &folder.id);
                if path.to_lowercase().contains(&query) {
                    html.push_str(&button(
                        "collection-pick",
                        &folder.id,
                        &esc(&path),
                        "folder_special",
                        "folder-picker-option",
                        app.busy,
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
                        ("name", &folder_path(app, id)),
                        ("folders", &folders.to_string()),
                        ("games", &games.to_string())
                    ]
                ))
            );
            html.push_str("<ul class=\"affected-folders\">");
            let removed = subtree(app, id);
            for (folder, _) in ordered(app, false) {
                let path = folder_path(app, &folder.id);
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
                app.busy,
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
                app.busy,
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
            for failure in &app.batch_failures {
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
                app.busy || app.batch_failures.is_empty(),
            ));
        }
        Dialog::Restore => {
            if app.excluded.is_empty() {
                let _ = write!(html, "<p>{}</p>", t(app, "library.excluded_empty"));
            } else {
                html.push_str(&button(
                    "excluded-restore-all",
                    "",
                    t(app, "library.restore_all"),
                    "refresh",
                    "secondary",
                    app.busy,
                ));
                for game in &app.excluded {
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
                            app.busy
                        )
                    );
                }
            }
        }
    }
    html.push_str("</div></section></div>");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_root_never_leaks_library_games_and_starts_with_no_cards() {
        let mut app = App {
            page: "collections".into(),
            ..Default::default()
        };
        let game:Game=serde_json::from_value(json!({"id":"g","title":"Game","folder":"F:/Game","library":"F:/","executable":"F:/Game/game.exe","engine":"unknown","modified":0,"tags":[]})).unwrap();
        app.catalog.games.push(game);
        let mut html = String::new();
        page(&mut html, &app);
        assert!(!html.contains("data-game-id"));
        assert!(!html.contains("data-folder-id"));
        assert!(!html.contains("collections-panel"));
        assert!(!html.contains("bar-search"));
        assert!(html.contains("collection-bar"));
        app.settings.collections.push(Collection {
            id: "a".into(),
            name: "A".into(),
            ..Default::default()
        });
        app.settings.collections.push(Collection {
            id: "b".into(),
            name: "B".into(),
            parent: Some("a".into()),
            ..Default::default()
        });
        html.clear();
        page(&mut html, &app);
        assert!(html.contains("data-folder-id=\"a\""));
        assert!(!html.contains("data-folder-id=\"b\""));
    }

    #[test]
    fn trash_shortcuts_enter_batch_without_deleting_and_picker_excludes_existing_members() {
        let mut app = App {
            page: "collections".into(),
            collection: Some("a".into()),
            ..Default::default()
        };
        app.settings.collections.push(Collection {
            id: "a".into(),
            name: "A".into(),
            games: BTreeSet::from(["g".into()]),
            ..Default::default()
        });
        app.catalog.games.push(serde_json::from_value(json!({"id":"g","title":"Game","folder":"F:/Game","library":"F:/","executable":"F:/Game/game.exe","engine":"unknown","modified":0,"tags":[]})).unwrap());
        dispatch(&mut app, "card-remove-select", "g");
        assert!(app.batch && app.checked.contains("g"));
        assert!(app.settings.collections[0].games.contains("g"));
        dispatch(&mut app, "folder-remove-select", "a");
        assert!(app.checked_folders.contains("a"));
        dispatch(&mut app, "folder-add-games", "");
        let mut html = String::new();
        dialog(&mut html, &app);
        assert!(!html.contains("data-action=\"picker-game\""));
        assert!(html.contains("data-action=\"picker-games-save\""));
    }
    #[test]
    fn collection_heading_is_fixed_and_path_is_below_the_heading() {
        let mut app = App {
            page: "collections".into(),
            collection: Some("child".into()),
            ..Default::default()
        };
        app.settings.collections.push(Collection {
            id: "child".into(),
            name: "Folder B".into(),
            ..Default::default()
        });
        let mut html = String::new();
        page(&mut html, &app);
        assert!(html.contains(r#"<h1 title="收藏夹">收藏夹</h1>"#));
        assert!(html.find("<h1").unwrap() < html.find("folder-breadcrumbs").unwrap());
    }
    #[test]
    fn breadcrumb_navigation_uses_ancestor_ids_even_when_names_contain_separators() {
        let mut app = App::default();
        app.settings.collections = vec![
            Collection {
                id: "parent".into(),
                name: "A / quoted <name>".into(),
                ..Default::default()
            },
            Collection {
                id: "child".into(),
                name: "B".into(),
                parent: Some("parent".into()),
                ..Default::default()
            },
        ];
        app.collection = Some("child".into());
        let mut html = String::new();
        breadcrumbs(&mut html, &app, "child");
        assert!(html.contains(r#"data-action="folder-open" data-value="parent""#));
        assert!(html.contains("A / quoted &lt;name&gt;"));
    }
    fn game(id: usize) -> Game {
        serde_json::from_value(json!({"id":id.to_string(),"title":format!("Game {id}"),"folder":"X:/games","library":"X:/","executable":"X:/games/game.exe","media_path":null,"engine":"unknown","status":"installed","cover":null,"modified":id,"cg_unlock_status":"unsupported","tags":if id.is_multiple_of(2) {vec!["ADV"]}else{vec!["Other"]},"available":true})).unwrap()
    }
    #[test]
    fn all_matching_entries_are_selected_beyond_the_render_limit() {
        let mut app = App::default();
        app.catalog.games = (0..300).map(game).collect();
        dispatch(&mut app, "batch-all", "");
        assert_eq!(app.checked.len(), 300);
    }
    #[test]
    fn selection_respects_direct_collection_membership_and_tag_filter() {
        let mut app = App::default();
        app.catalog.games = (0..300).map(game).collect();
        app.settings.collections.push(Collection {
            id: "folder".into(),
            games: (0..40).map(|n| n.to_string()).collect(),
            ..Default::default()
        });
        app.collection = Some("folder".into());
        app.tag_filter = "ADV".into();
        dispatch(&mut app, "batch-all", "");
        assert_eq!(app.checked.len(), 20);
    }
    #[test]
    fn sorting_does_not_change_selected_identities_and_exiting_clears_selection() {
        let mut app = App::default();
        app.catalog.games = (0..300).map(game).collect();
        dispatch(&mut app, "batch-all", "");
        let selected = app.checked.clone();
        app.settings.sort_by = "recent".into();
        assert_eq!(matching(&app)[0].id, "299");
        assert_eq!(app.checked, selected);
        dispatch(&mut app, "batch-toggle", "");
        assert!(app.checked.is_empty());
    }
    #[test]
    fn rendered_library_omits_type_badges_and_global_tag_management() {
        let mut app = App::default();
        app.catalog.games = vec![game(1)];
        let mut html = String::new();
        page(&mut html, &app);
        assert!(!html.contains("poster-badge"));
        assert!(!html.contains("tag-manager"));
        assert!(html.contains("library-bar"));
        assert!(!html.contains("collection-footer"));
    }
    #[test]
    fn collection_descendants_use_identity_even_when_names_contain_path_separators() {
        let mut app = App::default();
        app.settings.collections = vec![
            Collection {
                id: "a".into(),
                name: "A".into(),
                ..Default::default()
            },
            Collection {
                id: "b".into(),
                name: "A / B".into(),
                ..Default::default()
            },
            Collection {
                id: "c".into(),
                name: "B".into(),
                parent: Some("a".into()),
                ..Default::default()
            },
        ];
        assert_eq!(subtree(&app, "a"), BTreeSet::from(["a".into(), "c".into()]));
    }
}
