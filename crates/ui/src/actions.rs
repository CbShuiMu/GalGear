//! Navigation and shared action routing; feature actions stay with their state.
use crate::{
    accounts, ai, bridge::reset_scroll, choices, cloud, detail_form, downloads, game_setup,
    library, local_patches, models::GameEdit, patches, requests::Pending, requests::issue,
    state::App, state::show_toast,
};
use galgear_i18n::{Language, Message};
use serde_json::json;

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if let Some(render) = accounts::dispatch(app, action, value)
        .or_else(|| downloads::dispatch(app, action, value))
        .or_else(|| patches::dispatch(app, action, value))
    {
        return render;
    }
    match action {
        "home-select" if app.catalog.games.iter().any(|game| game.id == value) => {
            app.home_selected = Some(value.into());
        }
        "toggle-theme" | "set-theme" | "set-color-theme"
            if !app.requests.values().any(|p| matches!(p, Pending::Theme)) =>
        {
            let (command, payload) = if action == "set-color-theme" {
                if !matches!(value, "pink" | "white-pink" | "white")
                    || value == app.settings.color_theme
                {
                    return false;
                }
                ("set_color_theme", json!({"colorTheme":value}))
            } else {
                let theme = if action == "set-theme" {
                    if !matches!(value, "light" | "dark") {
                        return false;
                    }
                    value
                } else if app.settings.theme == "dark" {
                    "light"
                } else {
                    "dark"
                };
                if theme == app.settings.theme {
                    return false;
                }
                ("set_theme", json!({"theme":theme}))
            };
            app.theme_request = app.requests.next_id();
            issue(app, Pending::Theme, command, payload);
        }
        "choice-toggle"
            if matches!(
                value,
                "sort" | "folder-parent" | "patch-game" | "patch-search-game" | "fan-mode"
            ) || ai::choice_enabled(app, value)
                || cloud::choice_enabled(app, value)
                || crate::tool_preferences::enabled(app, value)
                || game_setup::choice_enabled(app, value) =>
        {
            if app.choices.open.as_deref() == Some(value) {
                app.choices.open = None;
            } else {
                app.choices.query.clear();
                app.choices.active = choices::filtered(app, value)
                    .iter()
                    .position(|o| o.value == choices::current(app, value))
                    .unwrap_or(0);
                app.choices.open = Some(value.into());
            }
        }
        "choice-close" => app.choices.open = None,
        "page"
            if matches!(
                value,
                "home" | "games" | "collections" | "patches" | "settings" | "about"
            ) =>
        {
            library::clear_selection(&mut app.library);
            app.library.batch = false;
            app.library.collection = None;
            app.library.dialog = None;
            app.page = value.into();
            app.choices.open = None;
            app.detail.selected = None;
            app.context_menu = None;
            detail_form::cancel(app);
            if value == "patches"
                && !app.patches.installed_patches_loaded
                && !app
                    .requests
                    .values()
                    .any(|p| matches!(p, Pending::PatchList))
            {
                issue(app, Pending::PatchList, "list_patches", json!({}));
            }
            if value == "settings" && app.settings_section == "paths" { crate::tool_preferences::load(app); }
            if value == "settings" && app.settings_section == "cloud" {
                cloud::load(app);
            }
            if value == "settings"
                && !app.requests.values().any(|p| {
                    matches!(
                        p,
                        Pending::Credentials
                            | Pending::CredentialsSaved
                            | Pending::CredentialsCleared
                    )
                })
            {
                if !app.account.credentials_loaded {
                    issue(app, Pending::Credentials, "load_fan_credentials", json!({}));
                }
                issue(app, Pending::FanCache, "get_fan_status_cache", json!({}));
            }
        }
        "load-more"
            if app.detail.selected.is_none()
                && matches!(app.page.as_str(), "games" | "collections") =>
        {
            app.library.visible_count += 120
        }
        "game-detail" => {
            // Keep the originating library/collection underneath the detail overlay.
            if !matches!(app.page.as_str(), "home" | "games" | "collections") {
                app.page = "games".into();
                app.library.collection = None;
                app.library.search.clear();
                app.library.clear_filters();
                app.library.visible_count = 120;
                reset_scroll();
            }
            let visible_count = app.library.visible_count;
            library::clear_selection(&mut app.library);
            app.library.visible_count = visible_count;
            app.library.batch = false;
            app.library.dialog = None;
            app.choices.open = None;
            app.detail.selected = Some(value.into());
            app.form.retain(|key, _| {
                !key.starts_with("draft-")
                    && !key.starts_with("edit-")
                    && !key.starts_with("meta-")
                    && !matches!(
                        key.as_str(),
                        "metadata-query"
                            | "title-draft"
                            | "backup-note"
                            | "tag-name"
                            | "tag-new"
                            | "locale-profile"
                    )
            });
            app.detail.editing = false;
            app.context_menu = None;
            app.detail.editor = None;
            app.detail.history_open = false;
            app.detail.history_game = None;
            app.detail.archive_open = false;
            app.detail.current_snapshot = None;
            app.detail.history.clear();
            app.detail.confirmation = None;
            app.detail.restore_preview = None;
            if let Some(game) = app.catalog.games.iter().find(|g| g.id == value).cloned() {
                let mut edit = app
                    .settings
                    .game_edits
                    .get(value)
                    .cloned()
                    .unwrap_or_default();
                edit.title = Some(game.title.clone());
                edit.tags = game.tags.clone();
                if edit.executable.is_none() {
                    edit.executable.clone_from(&game.executable);
                }
                app.form
                    .insert("edit-title".into(), edit.title.unwrap_or_default());
                app.form.insert("edit-tags".into(), edit.tags.join(", "));
                app.form
                    .insert("edit-exe".into(), edit.executable.unwrap_or_default());
                app.form
                    .insert("edit-args".into(), edit.arguments.join("\n"));
                app.form.insert(
                    "edit-workdir".into(),
                    edit.working_directory.unwrap_or_default(),
                );
                app.form
                    .insert("edit-launcher".into(), edit.launcher.unwrap_or_default());
                app.form.insert("metadata-query".into(), game.title.clone());
            }
            issue(app, Pending::Saves, "detect_saves", json!({"id":value}));
            local_patches::load(app, value);
            issue(
                app,
                Pending::Runtime(value.into()),
                "game_runtime",
                json!({"id":value}),
            );
            issue(
                app,
                Pending::History(value.into()),
                "save_history",
                json!({"id":value}),
            );
        }
        "close-detail" => {
            detail_form::cancel(app);
            app.detail.selected = None;
            app.detail.editor = None;
            app.detail.history_open = false;
            app.detail.confirmation = None;
        }
        "dismiss-toast" => app.toast = None,
        "refresh" => issue(app, Pending::Scan, "scan_library", json!({"full":true})),
        "cancel-scan" => issue(app, Pending::Open, "cancel_scan", json!({})),
        "detect-saves" => issue(app, Pending::Saves, "detect_saves", json!({"id":value})),
        "choose-save" => issue(
            app,
            Pending::ChooseSave,
            "choose_save_folder",
            json!({"id":value}),
        ),
        "open-save" => {
            if let Some(id) = app.detail.selected.clone() {
                issue(
                    app,
                    Pending::Open,
                    "open_save_folder",
                    json!({"id":id,"path":value}),
                );
            }
        }
        "manual-game" => issue(app, Pending::Manual, "add_manual_game", json!({})),
        "relocate" => issue(app, Pending::Manual, "relocate_game", json!({"id":value})),
        "exclude-game" => issue(
            app,
            Pending::Exclude(value.into()),
            "exclude_game",
            json!({"id":value}),
        ),
        "close-behavior" => issue(
            app,
            Pending::Preferences,
            "set_close_behavior",
            json!({"behavior":value}),
        ),
        "save-release" => {
            let url = app
                .form
                .get("release-page")
                .cloned()
                .unwrap_or_else(|| app.settings.release_page.clone());
            issue(
                app,
                Pending::Preferences,
                "set_release_page",
                json!({"url":url}),
            );
        }
        "check-update" => issue(app, Pending::Update, "check_update", json!({})),
        "launch-behavior" => issue(
            app,
            Pending::Preferences,
            "set_launch_behavior",
            json!({"behavior":value}),
        ),
        "edit-game" => {
            let get = |key: &str| app.form.get(key).cloned().unwrap_or_default();
            let optional = |key: &str| {
                let v = get(key);
                if v.trim().is_empty() { None } else { Some(v) }
            };
            let edit = GameEdit {
                title: Some(get("edit-title")),
                tags: get("edit-tags")
                    .split([',', '，'])
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect(),
                executable: optional("edit-exe"),
                arguments: get("edit-args").lines().map(str::to_owned).collect(),
                working_directory: optional("edit-workdir"),
                launcher: optional("edit-launcher"),
                ..Default::default()
            };
            let locale_profile = get("locale-profile");
            issue(
                app,
                Pending::Edit,
                "edit_game",
                json!({"id":value,"edit":edit,"localeProfile":locale_profile}),
            );
        }
        "metadata-search" => {
            let query = app.form.get("metadata-query").cloned().unwrap_or_default();
            let source = app.detail.metadata_source.clone();
            app.detail.candidates.remove(value);
            issue(
                app,
                Pending::SearchMetadata(value.into(), source.clone()),
                "search_metadata",
                json!({"query":query,"source":source}),
            );
        }
        "metadata-select" | "tags-select" => {
            if let Some(id) = app.detail.selected.clone() {
                issue(
                    app,
                    Pending::SelectMetadata,
                    "select_metadata",
                    json!({"id":id,"url":value,"target":if action == "tags-select" { "tags" } else { "details" }}),
                );
            }
        }
        "open-resource" => issue(app, Pending::Open, "open_resource", json!({"url":value})),
        "choose-folder" => issue(app, Pending::Folder, "choose_library_folder", json!({})),
        "add-folder" => {
            let path = app.library.path_input.trim().to_owned();
            if path.is_empty() {
                show_toast(app, Message::new("error.folder_required"), true);
            } else {
                issue(
                    app,
                    Pending::Add,
                    "add_library_folder",
                    json!({ "path": path }),
                );
            }
        }
        "remove-folder" => issue(
            app,
            Pending::Remove,
            "remove_library_folder",
            json!({"path": value}),
        ),
        "sort" | "sort-direction" => {
            let size = app.settings.card_size;
            let descending = app.settings.sort_descending();
            let (sort_by, descending) = if action == "sort-direction" {
                (app.settings.sort_by.as_str(), !descending)
            } else {
                (value, descending)
            };
            let payload = json!({"cardSize":size,"sortBy":sort_by,"sortDescending":descending});
            app.choices.open = None;
            issue(
                app,
                Pending::Preferences,
                "save_preferences",
                payload,
            );
        }
        "language" if Language::from_code(value).is_some() => issue(
            app,
            Pending::Language,
            "set_language",
            json!({"language": value}),
        ),
        "choose-cover" => issue(app, Pending::Cover, "choose_cover", json!({"id":value})),
        "unused-launch" => issue(app, Pending::Launch, "launch_game", json!({"id":value})),
        "open-folder" => issue(app, Pending::Open, "open_game_folder", json!({"id":value})),
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_details_return_to_the_same_shelf_without_resetting_library_filters() {
        let mut app = App {
            home_selected: Some("game".into()),
            ..Default::default()
        };
        app.library.search = "saved search".into();
        app.library.collection = Some("favorites".into());
        dispatch(&mut app, "game-detail", "game");
        assert_eq!(app.page, "home");
        assert_eq!(app.detail.selected.as_deref(), Some("game"));
        dispatch(&mut app, "close-detail", "");
        assert_eq!(app.page, "home");
        assert!(app.detail.selected.is_none());
        assert_eq!(app.home_selected.as_deref(), Some("game"));
        assert_eq!(app.library.search, "saved search");
        assert_eq!(app.library.collection.as_deref(), Some("favorites"));
    }

    #[test]
    fn library_details_still_return_to_the_library() {
        let mut app = App {
            page: "games".into(),
            ..Default::default()
        };
        dispatch(&mut app, "game-detail", "game");
        dispatch(&mut app, "close-detail", "");
        assert_eq!(app.page, "games");
        assert!(app.detail.selected.is_none());
    }

    #[test]
    fn collection_details_keep_the_folder_search_filters_and_loaded_page() {
        let mut app = App {
            page: "collections".into(),
            ..Default::default()
        };
        app.library.collection = Some("nested-folder".into());
        app.library.search = "saved search".into();
        app.library.tag_filter = vec!["学园".into()];
        app.library.status_filter.finished = Some(false);
        app.library.visible_count = 360;
        app.library.expanded.insert("parent-folder".into());
        dispatch(&mut app, "game-detail", "game");
        assert_eq!(app.page, "collections");
        dispatch(&mut app, "close-detail", "");
        assert_eq!(app.page, "collections");
        assert_eq!(app.library.collection.as_deref(), Some("nested-folder"));
        assert_eq!(app.library.search, "saved search");
        assert_eq!(app.library.tag_filter, ["学园"]);
        assert_eq!(app.library.status_filter.finished, Some(false));
        assert_eq!(app.library.visible_count, 360);
        assert!(app.library.expanded.contains("parent-folder"));
    }

    #[test]
    fn opening_game_details_preserves_patch_search_and_settings_drafts() {
        let mut app = App {
            page: "patches".into(),
            ..Default::default()
        };
        for (key, value) in [
            ("patch-query", "Previous search"),
            ("source-vndb-token", "Unsaved token"),
            ("tool-magpie", "F:/Tools/Magpie.exe"),
            ("backup-folder", "F:/Backups"),
            ("release-page", "https://example.com/releases"),
            ("edit-title", "Previous game title"),
            ("metadata-query", "Previous game search"),
            ("draft-title", "Previous game draft"),
        ] {
            app.form.insert(key.into(), value.into());
        }
        dispatch(&mut app, "game-detail", "game");
        assert_eq!(app.form["patch-query"], "Previous search");
        assert_eq!(app.form["source-vndb-token"], "Unsaved token");
        assert_eq!(app.form["tool-magpie"], "F:/Tools/Magpie.exe");
        assert_eq!(app.form["backup-folder"], "F:/Backups");
        assert_eq!(app.form["release-page"], "https://example.com/releases");
        assert!(!app.form.contains_key("edit-title"));
        assert!(!app.form.contains_key("metadata-query"));
        assert!(!app.form.contains_key("draft-title"));
    }

    #[test]
    fn returning_to_settings_keeps_the_selected_section_and_credential_draft() {
        let mut app = App::default();
        app.account.credentials_loaded = true;
        app.account.credentials.token = "Unsaved token".into();
        app.settings_section = "paths".into();
        dispatch(&mut app, "page", "settings");
        dispatch(&mut app, "page", "games");
        dispatch(&mut app, "page", "settings");
        assert_eq!(app.settings_section, "paths");
        assert_eq!(app.account.credentials.token, "Unsaved token");
        assert!(
            crate::bridge::take_requests()
                .iter()
                .all(|(_, command, _)| command != "load_fan_credentials")
        );
    }

    #[test]
    fn returning_to_patches_preserves_results_filters_and_install_preview() {
        let mut app = App::default();
        app.patches.installed_patches_loaded = true;
        app.patches.resources_searched = true;
        app.patches.patch_source = "kun".into();
        app.patches.patch_search_game = "game".into();
        app.patches.resources.push(crate::models::Resource {
            title: "Previous result".into(),
            ..Default::default()
        });
        app.patches.patch_preview = Some(
            serde_json::from_value(json!({
                "title": "Selected patch", "game_id": "game", "target": "F:/Game",
                "files": [], "executable": null
            }))
            .unwrap(),
        );
        dispatch(&mut app, "page", "patches");
        dispatch(&mut app, "page", "games");
        dispatch(&mut app, "page", "patches");
        assert_eq!(app.patches.resources[0].title, "Previous result");
        assert_eq!(app.patches.patch_source, "kun");
        assert_eq!(app.patches.patch_search_game, "game");
        assert!(app.patches.resources_searched);
        assert_eq!(
            app.patches.patch_preview.as_ref().unwrap().title,
            "Selected patch"
        );
        assert!(
            crate::bridge::take_requests()
                .iter()
                .all(|(_, command, _)| command != "list_patches")
        );
    }
}
