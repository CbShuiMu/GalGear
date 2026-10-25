//! Collection hierarchy and library queries, independent of DOM and transport.
mod actions;
mod dialogs;
mod state;
mod view;

pub(crate) use crate::models::Collection;
pub(crate) use actions::dispatch;
pub(crate) use dialogs::dialog;
pub(crate) use state::State;
pub(crate) use view::page;

use crate::{models::Catalog, models::Game, models::Settings};
use galgear_i18n::Message;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Clone)]
pub(crate) enum Dialog {
    Edit { id: Option<String> },
    Picker { ids: Vec<String> },
    Delete { id: String },
    Exclude { ids: Vec<String> },
    Warnings,
    Failures,
    Restore,
    Tags { selected: Vec<String> },
    AddGames,
    DeleteFolders { ids: Vec<String> },
}
#[derive(Clone, Deserialize)]
pub(crate) struct Failure {
    pub id: String,
    title: String,
    error: Message,
}
#[derive(Deserialize)]
pub(crate) struct BatchResult {
    pub settings: Settings,
    pub catalog: Catalog,
    pub succeeded: Vec<String>,
    pub failed: Vec<Failure>,
    pub unchanged: usize,
    pub cache_warning: Option<Message>,
}

pub(crate) fn clear_selection(state: &mut State) {
    state.checked.clear();
    state.checked_folders.clear();
    state.batch_failures.clear();
    state.visible_count = 120;
}

pub(crate) fn matching<'a>(
    catalog: &'a Catalog,
    settings: &Settings,
    state: &State,
    page: &str,
) -> Vec<&'a Game> {
    if page == "collections" && state.collection.is_none() {
        return Vec::new();
    }
    let query = state.search.to_lowercase();
    let folder = state
        .collection
        .as_ref()
        .and_then(|id| settings.collections.iter().find(|f| &f.id == id));
    let mut games: Vec<_> = catalog
        .games
        .iter()
        .filter(|g| {
            (state.collection.is_none() || folder.is_some_and(|f| f.games.contains(&g.id)))
                && (g.title.to_lowercase().contains(&query)
                    || g.tags.iter().any(|tag| tag.to_lowercase().contains(&query)))
                && state.tag_filter.iter().all(|tag| {
                    g.tags
                        .iter()
                        .any(|name| galgear_tags::key(name) == galgear_tags::key(tag))
                })
        })
        .collect();
    if settings.sort_by == "recent" {
        games.sort_by_key(|g| std::cmp::Reverse(g.modified));
    } else {
        games.sort_by_cached_key(|g| g.title.to_lowercase());
    }
    games
}

pub(crate) fn folder_path(collections: &[Collection], id: &str) -> String {
    let mut parts = Vec::new();
    let mut next = Some(id);
    let mut visited = BTreeSet::new();
    while let Some(id) = next {
        if !visited.insert(id) {
            break;
        }
        let Some(folder) = collections.iter().find(|f| f.id == id) else {
            break;
        };
        parts.push(folder.name.as_str());
        next = folder.parent.as_deref();
    }
    parts.reverse();
    parts.join(" / ")
}

pub(crate) fn subtree(collections: &[Collection], id: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::from([id.to_owned()]);
    loop {
        let before = ids.len();
        for folder in collections {
            if folder.parent.as_ref().is_some_and(|p| ids.contains(p)) {
                ids.insert(folder.id.clone());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

pub(crate) fn ordered<'a>(
    collections: &'a [Collection],
    expanded: &BTreeSet<String>,
    expanded_only: bool,
) -> Vec<(&'a Collection, usize)> {
    let mut result = Vec::new();
    let mut stack: Vec<_> = collections
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
        if !expanded_only || expanded.contains(&folder.id) {
            stack.extend(
                collections
                    .iter()
                    .rev()
                    .filter(|f| f.parent.as_deref() == Some(&folder.id))
                    .map(|f| (f, depth + 1)),
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::view::breadcrumbs;
    use super::*;
    use crate::state::App;
    use serde_json::json;

    #[test]
    fn toolbar_clear_button_removes_all_tag_filters_and_preserves_search_and_sort() {
        let mut app = App {
            library: State {
                tag_filter: vec!["ADV".into(), "Alice".into()],
                search: "Game".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        app.settings.sort_by = "recent".into();
        let mut html = String::new();
        page(&mut html, &app);
        assert!(html.contains("data-action=\"tag-clear\""));
        assert!(
            html.find("data-action=\"tag-filter-open\"").unwrap()
                < html.find("data-action=\"tag-clear\"").unwrap()
        );
        dispatch(&mut app, "tag-clear", "");
        assert!(app.library.tag_filter.is_empty());
        assert_eq!(app.library.search, "Game");
        assert_eq!(app.settings.sort_by, "recent");
        html.clear();
        page(&mut html, &app);
        assert!(!html.contains("data-action=\"tag-clear\""));
    }
    #[test]
    fn tag_choices_are_drafts_until_confirmation_and_cancel_discards_them() {
        let mut app = App::default();
        app.catalog.games = vec![game(0), game(1)];
        app.catalog.games[0].tags.push("Alice Smith".into());
        dispatch(&mut app, "tag-filter-open", "");
        let mut html = String::new();
        dialog(&mut html, &app);
        assert!(!html.contains("class=\"tag-search-results\""));
        assert!(!html.contains("tag-filter-selected"));
        app.form.insert("tag-query".into(), "alice".into());
        html.clear();
        dialog(&mut html, &app);
        assert!(html.contains("Alice Smith"));
        assert!(!html.contains("data-value=\"Other\""));
        dispatch(&mut app, "tag-filter", "alice smith");
        dispatch(&mut app, "tag-filter", "ADV");
        dispatch(&mut app, "tag-filter", "adv");
        assert_eq!(
            matching(&app.catalog, &app.settings, &app.library, &app.page).len(),
            2
        );
        html.clear();
        dialog(&mut html, &app);
        assert!(html.contains("tag-filter-remove"));
        assert!(html.contains("library-dialog-footer"));
        dispatch(&mut app, "tag-filter-apply", "");
        assert_eq!(app.library.tag_filter, vec!["Alice Smith", "ADV"]);
        assert_eq!(
            matching(&app.catalog, &app.settings, &app.library, &app.page)
                .iter()
                .map(|game| game.id.as_str())
                .collect::<Vec<_>>(),
            vec!["0"]
        );
        dispatch(&mut app, "tag-filter-open", "");
        dispatch(&mut app, "tag-filter-remove", "ADV");
        dispatch(&mut app, "library-close", "");
        assert_eq!(app.library.tag_filter, vec!["Alice Smith", "ADV"]);
        dispatch(&mut app, "tag-filter-open", "");
        dispatch(&mut app, "tag-filter-remove", "ADV");
        dispatch(&mut app, "tag-filter-remove", "Alice Smith");
        dispatch(&mut app, "tag-filter-apply", "");
        assert_eq!(
            matching(&app.catalog, &app.settings, &app.library, &app.page).len(),
            2
        );
    }

    #[test]
    fn detail_tag_opens_the_whole_library_and_replaces_previous_filters() {
        let mut app = App {
            page: "collections".into(),
            library: State {
                collection: Some("folder".into()),
                search: "unrelated".into(),
                tag_filter: vec!["Other".into()],
                ..Default::default()
            },
            detail: crate::detail::State {
                selected: Some("0".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        app.catalog.games = vec![game(0), game(1)];
        dispatch(&mut app, "detail-tag", "adv");
        assert_eq!(app.page, "games");
        assert!(
            app.library.collection.is_none()
                && app.detail.selected.is_none()
                && app.library.search.is_empty()
        );
        assert_eq!(
            matching(&app.catalog, &app.settings, &app.library, &app.page)[0].id,
            "0"
        );
    }

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
            library: State {
                collection: Some("a".into()),
                ..Default::default()
            },
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
        assert!(app.library.batch && app.library.checked.contains("g"));
        assert!(app.settings.collections[0].games.contains("g"));
        dispatch(&mut app, "folder-remove-select", "a");
        assert!(app.library.checked_folders.contains("a"));
        dispatch(&mut app, "folder-add-games", "");
        let mut html = String::new();
        dialog(&mut html, &app);
        assert!(!html.contains("data-action=\"picker-game\""));
        assert!(html.contains("data-action=\"picker-games-save\""));
    }
    #[test]
    fn collection_page_omits_the_redundant_heading_and_keeps_folder_context() {
        let mut app = App {
            page: "collections".into(),
            library: State {
                collection: Some("child".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        app.settings.collections.push(Collection {
            id: "child".into(),
            name: "Folder B".into(),
            ..Default::default()
        });
        let mut html = String::new();
        page(&mut html, &app);
        assert!(!html.contains("<h1"));
        assert!(html.contains(r#"<span aria-current="page">Folder B</span>"#));
        assert!(html.find("folder-breadcrumbs").unwrap() < html.find("collection-bar").unwrap());
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
        app.library.collection = Some("child".into());
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
        assert_eq!(app.library.checked.len(), 300);
    }
    #[test]
    fn toolbar_playtime_includes_matching_games_beyond_the_render_limit() {
        let mut app = App::default();
        app.catalog.games = (0..300)
            .map(|id| {
                let mut game = game(id);
                game.playtime_seconds = 360;
                game
            })
            .collect();
        app.library.tag_filter = vec!["ADV".into()];
        app.library.visible_count = 10;
        let mut html = String::new();
        page(&mut html, &app);
        assert!(html.contains(r#"class="library-playtime"><strong>15h</strong>游玩时长"#));
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
        app.library.collection = Some("folder".into());
        app.library.tag_filter = vec!["ADV".into()];
        dispatch(&mut app, "batch-all", "");
        assert_eq!(app.library.checked.len(), 20);
    }
    #[test]
    fn sorting_does_not_change_selected_identities_and_exiting_clears_selection() {
        let mut app = App::default();
        app.catalog.games = (0..300).map(game).collect();
        dispatch(&mut app, "batch-all", "");
        let selected = app.library.checked.clone();
        app.settings.sort_by = "recent".into();
        assert_eq!(
            matching(&app.catalog, &app.settings, &app.library, &app.page)[0].id,
            "299"
        );
        assert_eq!(app.library.checked, selected);
        dispatch(&mut app, "batch-toggle", "");
        assert!(app.library.checked.is_empty());
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
        assert_eq!(
            subtree(&app.settings.collections, "a"),
            BTreeSet::from(["a".into(), "c".into()])
        );
    }
}
