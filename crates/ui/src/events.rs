//! WASM event entry points and action routing.
use crate::{
    accounts, actions, ai, choices, cloud, context_menu, cover_picker, features, game_setup,
    library, local_patches, models::FanStatus, models::Game, models::Resource, playtime,
    requests::Pending, requests::issue, state::APP, state::show_toast, view::paint,
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(start)]
pub fn start() {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        issue(&mut app, Pending::Bootstrap, "bootstrap", json!({}));
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn cover_enhanced(payload: &str) {
    #[derive(Deserialize)]
    struct Update { id: String, previous_cover: String, cover: String, cover_artwork: crate::models::CoverArtwork }
    let Ok(update) = serde_json::from_str::<Update>(payload) else { return; };
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Some(game) = app.catalog.games.iter_mut().find(|g| g.id == update.id)
            && game.cover.as_deref() == Some(update.previous_cover.as_str()) {
            game.cover = Some(update.cover);
            game.cover_artwork = Some(update.cover_artwork);
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn runtime_updated(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(status) = serde_json::from_str::<BTreeMap<String, String>>(payload) {
            let stopped = app.detail.selected.as_ref().is_some_and(|id| {
                app.runtime.get(id).is_some_and(|value| value == "running")
                    && status.get(id).is_some_and(|value| value == "stopped")
            });
            app.runtime = status;
            if stopped { features::refresh_detail_files(&mut app); }
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn detail_files_changed() {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        features::refresh_detail_files(&mut app);
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn magpie_hotkey_updated(payload: &str) {
    #[derive(Deserialize)]
    struct Update { magpie: String, hotkey: String }
    let Ok(update) = serde_json::from_str::<Update>(payload) else { return; };
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if apply_magpie_hotkey(&mut app, &update.magpie, &update.hotkey) {
            paint(&app);
        }
    });
}

fn apply_magpie_hotkey(app: &mut crate::state::App, magpie: &str, hotkey: &str) -> bool {
    if app.settings.magpie != magpie || app.requests.values().any(|p| matches!(p, Pending::Tools)) {
        return false;
    }
    // Only clear a clean draft. An unfinished edit remains visible until Save.
    if app.form.get("tool-hotkey").is_some_and(|draft| features::same_hotkey(draft, &app.settings.magpie_hotkey)) {
        app.form.remove("tool-hotkey");
    }
    app.settings.magpie_hotkey = hotkey.into();
    app.magpie_observed_through = Some(app.requests.last_id());
    true
}

#[derive(Deserialize)]
struct Playtime {
    playtime_seconds: u64,
    last_played: Option<u64>,
}

fn apply_playtime(app: &mut crate::state::App, records: &BTreeMap<String, Playtime>) {
    for game in &mut app.catalog.games {
        if let Some(record) = records.get(&game.id) {
            playtime::merge_record(game, record.playtime_seconds, record.last_played);
        }
    }
}

#[wasm_bindgen]
pub fn playtime_updated(payload: &str) {
    if let Ok(records) = serde_json::from_str::<BTreeMap<String, Playtime>>(payload) {
        APP.with(|state| {
            let mut app = state.borrow_mut();
            apply_playtime(&mut app, &records);
            paint(&app);
        });
    }
}

#[wasm_bindgen]
pub fn tool_failed(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        show_toast(&mut app, Message::from_error(payload), true);
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn backup_completed(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(payload) {
            if !data["error"].is_null()
                && let Ok(error) = serde_json::from_value(data["error"].clone())
            {
                show_toast(&mut app, error, true);
            } else if let Some(created) = data["created"].as_bool() {
                show_toast(&mut app, Message::new(if created { "backup.auto_success" } else { "backup.duplicate" }), false);
            }
            if data["id"].as_str() == app.detail.selected.as_deref() {
                let id = app.detail.selected.clone();
                if let Some(id) = id {
                    issue(
                        &mut app,
                        Pending::History(id.clone()),
                        "save_history",
                        json!({"id":id}),
                    );
                }
            }
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn expire_toast(token: u32) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.toast_generation == token && app.toast.is_some() {
            app.toast = None;
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn expire_download_notice(token: u32) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.downloads.notice_generation == token && app.downloads.notice.is_some() {
            app.downloads.notice = None;
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn dispatch(action: &str, value: &str) {
    if action == "choice-pick" {
        if let Ok([field, selected]) = serde_json::from_str::<[String; 2]>(value) {
            let valid = APP.with(|state| {
                let mut app = state.borrow_mut();
                let valid = app.choices.open.as_deref() == Some(&field)
                    && choices::options(&app, &field)
                        .iter()
                        .any(|o| o.value == selected);
                app.choices.open = None;
                valid
            });
            if valid {
                if let Some((action @ ("setup-source" | "setup-candidate"), id)) =
                    field.split_once(':')
                {
                    dispatch(action, &json!([id, selected]).to_string());
                } else {
                    dispatch(&field, &selected);
                }
            }
        }
        return;
    }
    APP.with(|state| {
        let mut app = state.borrow_mut();
        let setup_action = action.starts_with("setup-")
            || matches!(action, "check-update" | "update-confirm" | "update-cancel")
            || matches!(action, "dismiss-toast" | "choice-close")
            || action == "choice-toggle" && game_setup::choice_enabled(&app, value);
        if app.setup.open && !setup_action {
            return;
        }
        if app.busy()
            && !matches!(
                action,
                "page"
                    | "close-detail"
                    | "dismiss-toast"
                    | "dismiss-download-notice"
                    | "game-detail"
                    | "load-more"
                    | "choice-close"
                    | "toggle-theme"
                    | "set-theme"
                    | "set-color-theme"
                    | "check-update" | "update-confirm" | "update-cancel" | "settings-section"
            )
        {
            return;
        }
        if crate::updates::dispatch(&mut app, action)
            || ai::dispatch(&mut app, action, value)
            || cloud::dispatch(&mut app, action, value)
            || crate::ai_summary::dispatch(&mut app, action, value)
            || game_setup::dispatch(&mut app, action, value)
            || cover_picker::dispatch(&mut app, action, value)
            || local_patches::dispatch(&mut app, action, value)
            || crate::library_import::dispatch(&mut app, action, value).is_some()
            || library::dispatch(&mut app, action, value)
            || features::dispatch(&mut app, action, value)
        {
            paint(&app);
            return;
        }
        if !actions::dispatch(&mut app, action, value) {
            return;
        }

        paint(&app);
    });
}

#[wasm_bindgen]
pub fn game_context(id: &str, x: f64, y: f64) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.busy()
            && app.detail.selected.is_none()
            && app.catalog.games.iter().any(|g| g.id == id)
        {
            app.choices.open = None;
            app.context_menu = Some(context_menu::Menu {
                id: id.into(),
                x,
                y,
            });
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn library_marquee(payload: &str, additive: bool) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.library.batch || app.busy() {
            return;
        }
        if let Ok(selection) = serde_json::from_str::<MarqueeSelection>(payload) {
            apply_marquee(&mut app, selection, additive);
            paint(&app);
        }
    });
}

#[derive(Deserialize)]
struct MarqueeSelection {
    games: BTreeSet<String>,
    folders: BTreeSet<String>,
}

fn apply_marquee(app: &mut crate::state::App, selection: MarqueeSelection, additive: bool) {
    let matching: BTreeSet<_> =
        library::matching(&app.catalog, &app.settings, &app.library, &app.page)
            .iter()
            .map(|g| g.id.clone())
            .collect();
    if !additive {
        app.library.checked.clear();
        app.library.checked_folders.clear();
    }
    app.library
        .checked
        .extend(selection.games.intersection(&matching).cloned());
    if app.page == "collections" {
        app.library.checked_folders.extend(
            app.settings.collections.iter()
                .filter(|folder| {
                    folder.parent == app.library.collection && selection.folders.contains(&folder.id)
                })
                .map(|folder| folder.id.clone()),
        );
    }
}

#[wasm_bindgen]
pub fn library_drag_ids(id: &str) -> String {
    APP.with(|state| {
        let app = state.borrow();
        let ids: Vec<_> = if app.library.batch && app.library.checked.contains(id) {
            app.library.checked.iter().map(String::as_str).collect()
        } else {
            vec![id]
        };
        serde_json::to_string(&ids).unwrap_or_default()
    })
}

#[wasm_bindgen]
pub fn library_drop(collection: &str, payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.busy() {
            return;
        }
        if let Ok(ids) = serde_json::from_str::<Vec<String>>(payload) {
            if ids.len() > 100_000 {
                return;
            }
            app.library.dialog = Some(library::Dialog::Picker { ids });
            library::dispatch(&mut app, "collection-pick", collection);
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn choice_key(field: &str, key: &str) -> bool {
    let mut selected = None;
    let handled = APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.choices.open.is_none()
            && matches!(key, "ArrowDown" | "ArrowUp")
            && !app.busy()
            && (matches!(
                field,
                "sort" | "patch-game" | "patch-search-game" | "fan-mode"
            ) || ai::choice_enabled(&app, field)
                || cloud::choice_enabled(&app, field)
                || crate::tool_preferences::enabled(&app, field)
                || game_setup::choice_enabled(&app, field))
        {
            app.choices.query.clear();
            app.choices.open = Some(field.into());
            app.choices.active = choices::filtered(&app, field)
                .iter()
                .position(|o| o.value == choices::current(&app, field))
                .unwrap_or(0);
            paint(&app);
            return true;
        }
        let Some(open) = app.choices.open.clone() else {
            return false;
        };
        let options = choices::filtered(&app, &open);
        let last = options.len().saturating_sub(1);
        match key {
            "Escape" | "Tab" => {
                app.choices.open = None;
                paint(&app);
                return key == "Escape";
            }
            "ArrowDown" => app.choices.active = (app.choices.active + 1).min(last),
            "ArrowUp" => app.choices.active = app.choices.active.saturating_sub(1),
            "Home" => app.choices.active = 0,
            "End" => app.choices.active = last,
            "PageDown" => app.choices.active = (app.choices.active + 8).min(last),
            "PageUp" => app.choices.active = app.choices.active.saturating_sub(8),
            "Enter" => {
                if let Some(option) = options.get(app.choices.active) {
                    selected = Some(
                        serde_json::to_string(&[open.as_str(), option.value.as_str()]).unwrap(),
                    );
                }
            }
            _ => return false,
        }
        if selected.is_none() {
            paint(&app);
        }
        true
    });
    if let Some(value) = selected {
        dispatch("choice-pick", &value);
    }
    handled
}

#[wasm_bindgen]
pub fn input_changed(field: &str, value: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if ai::input(&mut app, field, value)
            || cloud::input(&mut app, field, value)
            || cover_picker::input(&mut app, field, value)
            || game_setup::input(&mut app, field, value)
        {
            paint(&app);
            return;
        }
        if let Some(metadata_field) = field.strip_prefix("draft-meta-") {
            app.form.remove(&format!("draft-reset-{metadata_field}"));
        }
        match field {
            "choice-query" if app.choices.open.is_some() => {
                app.choices.query = value.into();
                app.choices.active = 0;
                paint(&app);
                return;
            }
            "fan-token" => {
                app.account.credentials.token = value.into();
                return;
            }
            "fan-cookie" => {
                app.account.credentials.cookie = value.into();
                return;
            }
            "search" => {
                library::clear_selection(&mut app.library);
                app.library.search = value.into();
                app.library.visible_count = 120;
            }
            "folder-name" => {
                app.form.insert(field.into(), value.into());
                return;
            }
            "folder-query" | "tag-query" | "game-query" => {
                app.form.insert(field.into(), value.into());
            }
            "library-path" => app.library.path_input = value.into(),
            key if key.starts_with("draft-")
                || key.starts_with("edit-")
                || key.starts_with("meta-")
                || key.starts_with("tool-")
                || key.starts_with("source-")
                || matches!(
                    key,
                    "title-draft"
                        | "backup-note"
                        | "tag-name"
                        | "tag-new"
                        | "locale-profile"
                        | "backup-retention"
                        | "backup-folder"
                        | "metadata-query"
                        | "patch-query"
                        | "release-page"
                ) =>
            {
                app.form.insert(key.into(), value.into());
                if key == "patch-query"
                    && !app
                        .catalog
                        .games
                        .iter()
                        .any(|g| g.id == app.patches.patch_search_game && g.title == value)
                {
                    app.patches.patch_search_game.clear();
                }
                return;
            }
            _ => return,
        }
        paint(&app);
    });
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn magpie_updates_refresh_clean_fields_and_preserve_unsaved_edits() {
        let mut app = crate::state::App::default();
        app.settings.magpie = "tools/magpie/Magpie.exe".into();
        app.settings.magpie_hotkey = "Alt+Shift+A".into();
        app.form.insert("tool-hotkey".into(), "Alt+Shift+A".into());
        let path = app.settings.magpie.clone();
        assert!(apply_magpie_hotkey(&mut app, &path, "Ctrl+F12"));
        assert!(!app.form.contains_key("tool-hotkey"));
        assert_eq!(app.settings.magpie_hotkey, "Ctrl+F12");
        app.form.insert("tool-hotkey".into(), "Alt+Shift+B".into());
        assert!(apply_magpie_hotkey(&mut app, &path, "Ctrl+F10"));
        assert_eq!(app.form["tool-hotkey"], "Alt+Shift+B");
        assert_eq!(app.settings.magpie_hotkey, "Ctrl+F10");
        assert!(!apply_magpie_hotkey(&mut app, "other/Magpie.exe", "Ctrl+F11"));
    }

    #[test]
    fn old_preferences_cannot_undo_magpie_event_and_pending_save_is_not_interrupted() {
        let mut app = crate::state::App::default();
        app.settings.magpie = "tools/magpie/Magpie.exe".into();
        app.settings.magpie_hotkey = "Alt+Shift+A".into();
        let path = app.settings.magpie.clone();
        let old = app.settings.clone();
        issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
        let request = app.requests.last_id();
        apply_magpie_hotkey(&mut app, &path, "Ctrl+F12");
        crate::state::apply_settings(&mut app, old, request);
        assert_eq!(app.settings.magpie_hotkey, "Ctrl+F12");
        issue(&mut app, Pending::Tools, "configure_tools", json!({}));
        assert!(!apply_magpie_hotkey(&mut app, &path, "Ctrl+F10"));
        assert_eq!(app.settings.magpie_hotkey, "Ctrl+F12");
    }

    #[test]
    fn source_credentials_are_kept_in_the_settings_draft() {
        input_changed("source-vndb-token", "Unsaved source token");
        APP.with(|state| {
            let mut app = state.borrow_mut();
            assert_eq!(app.form["source-vndb-token"], "Unsaved source token");
            app.form.remove("source-vndb-token");
        });
    }
}

#[wasm_bindgen]
pub fn download_progress(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app
            .requests
            .values()
            .any(|p| matches!(p, Pending::Download))
            && let Ok(transfer) = serde_json::from_str(payload)
        {
            app.downloads.transfer = Some(transfer);
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn fan_status_updated(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(status) = serde_json::from_str::<FanStatus>(payload) {
            accounts::apply_status(&mut app.account, status);
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn scan_progress(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.library.scanning {
            return;
        }
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(payload) {
            app.library.scan_folders = data["folders_scanned"].as_u64().unwrap_or(0) as usize;
            if let Ok(games) = serde_json::from_value::<Vec<Game>>(data["games"].clone()) {
                for game in games {
                    if !app.settings.excluded_games.contains(&game.id)
                        && !app.catalog.games.iter().any(|g| g.id == game.id)
                    {
                        app.catalog.games.push(game);
                    }
                }
            }
            game_setup::sync(&mut app);
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn game_enriched(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(payload) {
            if let Ok(game) = serde_json::from_value::<Game>(data["game"].clone()) {
                app.settings
                    .game_edits
                    .entry(game.id.clone())
                    .or_default()
                    .metadata
                    .clone_from(&game.metadata);
                if let Some(entry) = app.catalog.games.iter_mut().find(|g| g.id == game.id) {
                    playtime::replace_game(entry, game);
                }
            }
            if let Some(id) = data["id"].as_str() {
                if let Ok(candidates) =
                    serde_json::from_value::<Vec<Resource>>(data["candidates"].clone())
                {
                    app.detail.candidates.insert(id.into(), candidates);
                }
                if let Ok(warning) = serde_json::from_value::<Message>(data["warning"].clone()) {
                    app.detail.metadata_warnings.insert(id.into(), warning);
                }
            }
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn enrichment_progress(payload: &str) {
    #[derive(Deserialize)]
    struct Progress {
        title: String,
        current: usize,
        total: usize,
    }
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.requests.values().any(|p| matches!(p, Pending::Enrich))
            && let Ok(progress) = serde_json::from_str::<Progress>(payload)
        {
            app.enrichment_progress = Some(
                Message::new("network.enrich")
                    .arg("title", progress.title)
                    .arg("current", progress.current)
                    .arg("total", progress.total),
            );
            paint(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_dialog_receives_the_shared_close_action_before_library_dialogs() {
        APP.with(|state| {
            *state.borrow_mut() = crate::state::App::default();
        });
        dispatch("import-open", "");
        APP.with(|state| assert!(state.borrow().library_import.open));
        dispatch("library-close", "");
        APP.with(|state| assert!(!state.borrow().library_import.open));
    }

    #[test]
    fn marquee_keeps_games_and_folders_separate_and_rejects_other_collection_items() {
        let mut app = crate::state::App {
            page: "collections".into(),
            ..Default::default()
        };
        app.library.collection = Some("parent".into());
        app.settings.collections = serde_json::from_value(json!([
            {"id":"parent", "games":["same"]},
            {"id":"same", "parent":"parent"},
            {"id":"other", "parent":"elsewhere"}
        ])).unwrap();
        app.catalog.games = serde_json::from_value(json!([
            {"id":"same", "title":"Game", "folder":"X:/Game", "engine":"unknown", "modified":0},
            {"id":"outside", "title":"Other", "folder":"X:/Other", "engine":"unknown", "modified":0}
        ])).unwrap();
        for game in &app.catalog.games { app.settings.game_edits.entry(game.id.clone()).or_default().setup_confirmed = true; }
        let selection = serde_json::from_value(json!({"games":["same","outside"], "folders":["same","other"]})).unwrap();
        apply_marquee(&mut app, selection, false);
        assert_eq!(app.library.checked, BTreeSet::from(["same".into()]));
        assert_eq!(app.library.checked_folders, BTreeSet::from(["same".into()]));

        let empty = || MarqueeSelection { games: BTreeSet::new(), folders: BTreeSet::new() };
        apply_marquee(&mut app, empty(), true);
        assert_eq!(app.library.checked.len(), 1);
        assert_eq!(app.library.checked_folders.len(), 1);
        apply_marquee(&mut app, empty(), false);
        assert!(app.library.checked.is_empty());
        assert!(app.library.checked_folders.is_empty());
    }

    #[test]
    fn delayed_playtime_events_cannot_roll_back_the_final_checkpoint() {
        let mut app = crate::state::App::default();
        app.catalog.games.push(
            serde_json::from_value(json!({
                "id": "game", "title": "Game", "folder": "X:/Game", "engine": "unknown",
                "modified": 0, "playtime_seconds": 7200, "last_played": 200
            }))
            .unwrap(),
        );
        let records = serde_json::from_value(json!({
            "game": {"playtime_seconds": 3600, "last_played": 100}
        }))
        .unwrap();

        apply_playtime(&mut app, &records);

        assert_eq!(
            (
                app.catalog.games[0].playtime_seconds,
                app.catalog.games[0].last_played
            ),
            (7200, Some(200))
        );
    }

    #[test]
    fn playtime_updates_refresh_known_games_without_changing_library_selection() {
        let mut app = crate::state::App::default();
        app.catalog.games.push(
            serde_json::from_value(json!({
                "id": "game", "title": "Game", "folder": "X:/Game", "engine": "unknown",
                "modified": 0
            }))
            .unwrap(),
        );
        app.library.checked.insert("game".into());
        let records = serde_json::from_value(json!({
            "game": { "playtime_seconds": 5400, "last_played": 1792886400_u64 },
            "removed": { "playtime_seconds": 3600, "last_played": null }
        }))
        .unwrap();

        apply_playtime(&mut app, &records);

        assert_eq!(
            (
                app.catalog.games.len(),
                app.catalog.games[0].playtime_seconds,
                app.catalog.games[0].last_played,
                app.library.checked.contains("game")
            ),
            (1, 5400, Some(1792886400), true)
        );
    }
}
