//! Collection hierarchy and library queries, independent of DOM and transport.
mod actions;
mod dialogs;
mod state;
mod view;

pub(crate) use crate::models::Collection;
pub(crate) use actions::dispatch;
pub(crate) use dialogs::dialog;
pub(crate) use state::State;
pub(crate) use state::StatusFilter;
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
    Tags { selected: Vec<String>, status: StatusFilter },
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

fn release_date_key(value: &str) -> Option<i32> {
    let mut parts = value.trim().split(['-', '/', '.', '年', '月', '日', 'T', ' ']);
    let year = parts.next()?;
    if year.len() != 4 {
        return None;
    }
    let year = year.parse().ok().filter(|year| *year > 0)?;
    // Partial dates sort at the beginning of their known year or month.
    let component = |value| match value {
        None | Some("" | "??") => Some(1),
        Some(value) => value.parse::<u8>().ok(),
    };
    let month = time::Month::try_from(component(parts.next())?).ok()?;
    let day = component(parts.next())?;
    time::Date::from_calendar_date(year, month, day).ok().map(time::Date::to_julian_day)
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
                && state.status_filter.finished.is_none_or(|finished| (g.play_status == "finished") == finished)
                && state.status_filter.favorite.is_none_or(|favorite| {
                    settings.collections.iter().any(|folder| folder.games.contains(&g.id)) == favorite
                })
                && (g.title.to_lowercase().contains(&query)
                    || g.tags.iter().any(|tag| tag.to_lowercase().contains(&query)))
                && state.tag_filter.iter().all(|tag| {
                    g.tags
                        .iter()
                        .any(|name| galgear_tags::key(name) == galgear_tags::key(tag))
                })
        })
        .collect();
    if settings.sort_by == "release" {
        let descending = settings.sort_descending();
        games.sort_by_cached_key(|game| {
            let release = settings.game_edits.get(&game.id)
                .and_then(|edit| edit.summary_release_date.as_deref())
                .or_else(|| game.metadata.as_ref().map(|meta| meta.release_date.as_str()))
                .and_then(release_date_key);
            let date = release.unwrap_or_default();
            (release.is_none(), if descending { -date } else { date }, game.title.to_lowercase())
        });
    } else if settings.sort_by == "recent" {
        games.sort_by_key(|g| g.modified);
    } else {
        games.sort_by_cached_key(|g| g.title.to_lowercase());
    }
    if settings.sort_by != "release" && settings.sort_descending() {
        games.reverse();
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
    fn status_filters_combine_with_tags_and_follow_current_button_state() {
        let mut app = App { page: "games".into(), ..Default::default() };
        app.catalog.games = (0..4).map(game).collect();
        app.catalog.games[0].play_status = "finished".into();
        app.catalog.games[1].play_status = "finished".into();
        app.settings.collections.push(Collection {
            id: "folder".into(), games: ["0".into(), "2".into()].into(), ..Default::default()
        });
        for (favorite, finished, expected) in [(true, true, "0"), (false, true, "1"), (true, false, "2"), (false, false, "3")] {
            app.library.status_filter = StatusFilter { favorite: Some(favorite), finished: Some(finished) };
            assert_eq!(matching(&app.catalog, &app.settings, &app.library, &app.page).iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), [expected]);
        }
        app.library.status_filter = StatusFilter { favorite: Some(true), finished: Some(false) };
        app.library.tag_filter = vec!["ADV".into()];
        assert_eq!(matching(&app.catalog, &app.settings, &app.library, &app.page)[0].id, "2");
        app.catalog.games[2].play_status = "finished".into();
        assert!(matching(&app.catalog, &app.settings, &app.library, &app.page).is_empty());
        app.catalog.games[2].play_status.clear();
        app.settings.collections[0].games.remove("2");
        assert!(matching(&app.catalog, &app.settings, &app.library, &app.page).is_empty());
    }

    #[test]
    fn status_choices_are_exclusive_drafts_and_clear_with_tags() {
        let mut app = App::default();
        dispatch(&mut app, "tag-filter-open", "");
        dispatch(&mut app, "tag-status", "favorite");
        dispatch(&mut app, "tag-status", "not-favorite");
        dispatch(&mut app, "tag-status", "finished");
        assert!(app.library.status_filter.is_empty());
        dispatch(&mut app, "tag-filter-apply", "");
        assert_eq!(app.library.status_filter, StatusFilter { favorite: Some(false), finished: Some(true) });
        let mut html = String::new();
        page(&mut html, &app);
        assert!(html.contains("未收藏、已通关"));
        dispatch(&mut app, "tag-filter-open", "");
        dispatch(&mut app, "tag-status", "finished");
        dispatch(&mut app, "library-close", "");
        assert_eq!(app.library.status_filter.finished, Some(true));
        dispatch(&mut app, "tag-clear", "");
        assert!(!app.library.has_filters());
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
    fn release_dates_normalize_provider_formats_and_partial_dates() {
        let expected = release_date_key("2024-02-09").unwrap();
        for date in ["2024-2-9", "2024/2/9", "2024.2.9", "2024年2月9日", "2024-02-09T00:00:00Z"] {
            assert_eq!(release_date_key(date), Some(expected), "{date}");
        }
        assert_eq!(release_date_key("2024"), release_date_key("2024-01-01"));
        assert_eq!(release_date_key("2024-02-??"), release_date_key("2024-02-01"));
    }

    #[test]
    fn invalid_release_dates_remain_unknown() {
        for date in ["", "未定", "TBA", "0000-01-01", "2024-13-01", "2023-02-29", "2024-02-30"] {
            assert_eq!(release_date_key(date), None, "{date}");
        }
        assert!(release_date_key("2024-02-29").is_some());
    }

    #[test]
    fn release_sort_uses_detail_overrides_and_keeps_unknown_dates_last_in_both_directions() {
        let mut app = App::default();
        app.settings.sort_by = "release".into();
        app.catalog.games = (1..=10).map(game).collect();
        for (game, date) in app.catalog.games.iter_mut().zip([
            "2025-10-1", "1998-12-31", "", "2026-02-30", "2024/2/29",
            "2024年11月2日", "2020-01-01", "2023", "", "1980-01-01",
        ]) {
            if !date.is_empty() {
                game.metadata = Some(crate::models::Metadata { release_date: date.into(), ..Default::default() });
            }
        }
        for (id, date) in [("7", "2030-01-01"), ("9", "2022-01-01"), ("10", "")] {
            app.settings.game_edits.entry(id.into()).or_default().summary_release_date = Some(date.into());
        }
        for (descending, expected) in [
            (false, ["2", "9", "8", "5", "6", "1", "7", "10", "3", "4"]),
            (true, ["7", "1", "6", "5", "8", "9", "2", "10", "3", "4"]),
        ] {
            app.settings.sort_descending = Some(descending);
            let ids: Vec<_> = matching(&app.catalog, &app.settings, &app.library, "games")
                .iter().map(|game| game.id.as_str()).collect();
            assert_eq!(ids, expected, "descending={descending}");
        }
    }

    #[test]
    fn sort_direction_applies_to_names_and_update_times() {
        let mut app = App::default();
        app.catalog.games = vec![game(1), game(2), game(3)];
        for (game, title) in app.catalog.games.iter_mut().zip(["Beta", "alpha", "Gamma"]) {
            game.title = title.into();
        }
        for (field, descending, expected) in [
            ("title", false, ["2", "1", "3"]),
            ("title", true, ["3", "1", "2"]),
            ("recent", false, ["1", "2", "3"]),
            ("recent", true, ["3", "2", "1"]),
        ] {
            app.settings.sort_by = field.into();
            app.settings.sort_descending = Some(descending);
            let ids: Vec<_> = matching(&app.catalog, &app.settings, &app.library, "games")
                .iter().map(|game| game.id.as_str()).collect();
            assert_eq!(ids, expected, "{field}, descending={descending}");
        }
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
        app.settings.sort_descending = Some(false);
        assert_eq!(matching(&app.catalog, &app.settings, &app.library, &app.page)[0].id, "0");
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
