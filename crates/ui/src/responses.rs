//! Backend response routing and state reconciliation.
use crate::{
    accounts, ai, bridge::schedule_download_notice_expiry, cloud, cover_picker, detail,
    detail_form, features, game_setup, library, local_patches, models::Account, models::Bootstrap,
    models::Game, models::Metadata, models::Settings, playtime, requests::Pending, requests::issue,
    state::APP, state::apply_settings, state::show_toast, view::paint,
};
use galgear_i18n::Message;
use serde_json::json;
use wasm_bindgen::prelude::wasm_bindgen;

fn resolve_background(app: &mut crate::state::App, id: u32, success: bool, payload: &str) {
    if !success {
        show_toast(app, Message::from_error(payload), true);
        return;
    }
    match serde_json::from_str::<Option<Settings>>(payload) {
        Ok(Some(settings)) => {
            apply_settings(app, settings, id);
            show_toast(app, Message::new("notice.saved"), false);
        }
        Ok(None) => {}
        Err(error) => show_toast(app, Message::detail("error.read_data", error), true),
    }
}

#[wasm_bindgen]
pub fn resolve(id: u32, success: bool, payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        let Some(kind) = app.requests.complete(id) else {
            return;
        };
        if crate::screenshots::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if detail_form::resolve_refresh(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if crate::skill_update::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if crate::updates::resolve(&mut app, &kind, success, payload) {
            paint(&app);
            return;
        }
        if matches!(kind, Pending::Background) {
            resolve_background(&mut app, id, success, payload);
            paint(&app);
            return;
        }
        if matches!(kind, Pending::Enrich) {
            app.enrichment_progress = None;
        }
        if matches!(kind, Pending::Scan | Pending::BackgroundScan) {
            app.library.scanning = false;
            if std::mem::take(&mut app.library.rescan_pending) {
                issue(&mut app, Pending::BackgroundScan, "scan_library", json!({}));
            }
        }
        if let Pending::Ai(request) = &kind {
            ai::resolve(&mut app, request, id, success, payload);
            paint(&app);
            return;
        }
        if let Pending::Cloud(request) = &kind {
            cloud::resolve(&mut app, request, success, payload);
            paint(&app);
            return;
        }
        if crate::ai_summary::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if crate::launcher_picker::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if game_setup::resolve(&mut app, &kind, id, success, payload) {
            game_setup::sync(&mut app);
            paint(&app);
            return;
        }
        if matches!(kind, Pending::CoverOptions) {
            cover_picker::resolve(&mut app, id, success, payload);
            paint(&app);
            return;
        }
        if crate::library_import::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if crate::tool_preferences::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if local_patches::resolve(&mut app, &kind, id, success, payload) {
            paint(&app);
            return;
        }
        if features::resolve_find_save(&mut app, &kind, success, payload) {
            paint(&app);
            return;
        }
        if !success {
            if matches!(kind, Pending::RestorePreview) {
                app.detail.history_open = false;
            }
            if matches!(kind, Pending::Download | Pending::NextMoeDownload) {
                if let Some(transfer) = &mut app.downloads.transfer {
                    transfer.phase = "error".into();
                    transfer.error = Some(Message::from_error(payload));
                }
                app.downloads.preview = None;
                if matches!(kind, Pending::Download) {
                    issue(&mut app, Pending::Account, "refresh_fan_status", json!({}));
                }
            }
            if matches!(kind, Pending::Account) {
                app.account.status = Account {
                    status: "error".into(),
                    reason: Some(Message::from_error(payload)),
                    ..Default::default()
                };
            }
            show_toast(&mut app, Message::from_error(payload), true);
        } else {
            let handled: Result<(), String> = (|| {
                match kind {
                    Pending::LauncherList | Pending::LauncherSave => {}
                    Pending::Screenshot(..)
                    | Pending::TitleRefresh(..)
                    | Pending::ImportPreview
                    | Pending::ImportCommit
                    | Pending::MagpieModes
                    | Pending::ExportPotato
                    | Pending::FindSave(_)
                    | Pending::ObserveSaves(_)
                    | Pending::Background
                    | Pending::Ai(_)
                    | Pending::Cloud(_)
                    | Pending::SummarySetup(..)
                    | Pending::Summary(_)
                    | Pending::RouteMapPrepare(_)
                    | Pending::RouteMap(_)
                    | Pending::SetupSearch(_)
                    | Pending::LocalPatchList(_)
                    | Pending::LocalPatchApply(..)
                    | Pending::CoverOptions
                    | Pending::SetupExecutable(_)
                    | Pending::SetupBatch(_)
                    | Pending::SetupSave(_) => {}
                    Pending::AiRuntime | Pending::SkillUpdate | Pending::SkillCheck => {}
                    Pending::CollectionSave => {
                        let settings = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, settings, id);
                        if app
                            .library
                            .collection
                            .as_ref()
                            .is_some_and(|id| !app.settings.collections.iter().any(|f| &f.id == id))
                        {
                            app.library.collection = None;
                            library::clear_selection(&mut app.library);
                        }
                        app.library.checked_folders.clear();
                        app.library.dialog = None;
                        app.choices.open = None;
                        show_toast(&mut app, Message::new("notice.saved"), false);
                    }
                    Pending::Excluded => {
                        app.library.excluded =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    }
                    Pending::Batch => {
                        let result: library::BatchResult =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, result.settings, id);
                        playtime::replace_catalog(&mut app.catalog, result.catalog);
                        for done in &result.succeeded {
                            app.library.checked.remove(done);
                        }
                        app.library
                            .excluded
                            .retain(|game| !result.succeeded.contains(&game.id));
                        app.library.batch_failures = result.failed;
                        if app.library.batch {
                            let failed_ids: Vec<_> = app
                                .library
                                .batch_failures
                                .iter()
                                .map(|f| f.id.clone())
                                .collect();
                            app.library.checked.extend(failed_ids);
                        }
                        app.library.dialog = if app.library.batch_failures.is_empty() {
                            None
                        } else {
                            Some(library::Dialog::Failures)
                        };
                        if app
                            .detail
                            .selected
                            .as_ref()
                            .is_some_and(|id| !app.catalog.games.iter().any(|g| &g.id == id))
                        {
                            app.detail.selected = None;
                        }
                        let notice =
                            if result.unchanged > 0 && result.unchanged == result.succeeded.len() {
                                Message::new("folders.already_added")
                            } else {
                                Message::new("bulk.result")
                                    .arg("success", result.succeeded.len())
                                    .arg("failed", app.library.batch_failures.len())
                            };
                        let failed = !app.library.batch_failures.is_empty();
                        show_toast(&mut app, result.cache_warning.unwrap_or(notice), failed);
                    }
                    Pending::History(game_id) => {
                        if app.detail.selected.as_deref() != Some(&game_id) {
                            return Ok(());
                        }
                        let history: crate::models::SaveHistory =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.detail.history = history.snapshots;
                        app.detail.current_snapshot = history.current_snapshot;
                        if let Some(notice) = history.notice {
                            show_toast(&mut app, notice, false);
                        }
                        app.detail.history_game = Some(game_id);
                        app.detail.restore_preview = None;
                        app.detail.history_open = false;
                    }
                    Pending::DataSources(source) | Pending::SourceOAuth(source) => {
                        app.data_sources =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if !source.is_empty() {
                            let prefix = format!("source-{source}-");
                            app.form.retain(|key, _| !key.starts_with(&prefix));
                        }
                    }
                    Pending::RestorePreview => {
                        app.detail.restore_preview =
                            Some(serde_json::from_str(payload).map_err(|e| e.to_string())?);
                    }
                    Pending::Runtime(game_id) => {
                        app.runtime.insert(
                            game_id,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                        );
                    }
                    Pending::ConfirmSave(game_id) => {
                        let settings = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, settings, id);
                        issue(
                            &mut app,
                            Pending::Saves,
                            "detect_saves",
                            json!({"id":game_id}),
                        );
                    }
                    Pending::Upscale => {
                        show_toast(&mut app, Message::new("tools.triggered"), false)
                    }
                    Pending::GameIcons => {
                        app.game_icons = serde_json::from_str(payload).map_err(|e| e.to_string())?
                    }
                    Pending::Credentials
                    | Pending::CredentialsSaved
                    | Pending::CredentialsCleared => {
                        app.account.credentials =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.account.credentials_loaded = true;
                        app.downloads.preview = None;
                        if matches!(kind, Pending::CredentialsCleared) {
                            app.account.status = Account::default();
                            app.account.error = None;
                            issue(
                                &mut app,
                                Pending::FanCache,
                                "get_fan_status_cache",
                                json!({}),
                            );
                        } else if matches!(kind, Pending::CredentialsSaved) {
                            app.account.status = Account::default();
                            app.account.error = None;
                            issue(&mut app, Pending::Account, "refresh_fan_status", json!({}));
                        }
                    }
                    Pending::Account | Pending::FanCache => {
                        accounts::apply_status(
                            &mut app.account,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                        );
                    }
                    Pending::DownloadPreview => {
                        app.downloads.preview =
                            Some(serde_json::from_str(payload).map_err(|e| e.to_string())?)
                    }
                    Pending::Download | Pending::NextMoeDownload => {
                        app.downloads.transfer =
                            Some(serde_json::from_str(payload).map_err(|e| e.to_string())?);
                        if app
                            .downloads
                            .transfer
                            .as_ref()
                            .is_some_and(|t| t.phase == "complete")
                        {
                            app.downloads.notice = app.downloads.transfer.clone();
                            app.downloads.notice_generation =
                                app.downloads.notice_generation.wrapping_add(1);
                            if app.downloads.notice.as_ref().is_none_or(|t| t.archive_password.is_none()) {
                                schedule_download_notice_expiry(app.downloads.notice_generation, 8000);
                            }
                            crate::features::refresh_detail_files(&mut app);
                        }
                        app.downloads.preview = None;
                        if matches!(kind, Pending::Download) {
                            issue(&mut app, Pending::Account, "refresh_fan_status", json!({}));
                        }
                    }
                    Pending::Bootstrap | Pending::Add | Pending::Remove => {
                        let data: Bootstrap =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, data.settings, id);
                        playtime::replace_catalog(&mut app.catalog, data.catalog);
                        app.config_path = data.config_path;
                        app.version = data.version;
                        app.ai_available = data.ai_available;
                        app.initialized = true;
                        if matches!(kind, Pending::Add) {
                            game_setup::refreshed(&mut app);
                        }
                        if matches!(kind, Pending::Bootstrap) {
                            if data.update_complete { show_toast(&mut app, Message::new("update.complete"), false); }
                            crate::startup::resume(&mut app);
                        } else {
                            crate::startup::refresh_library(&mut app);
                        }
                        if !matches!(kind, Pending::Bootstrap) {
                            app.library.path_input.clear();
                            show_toast(&mut app, Message::new("notice.library_saved"), false);
                        }
                    }
                    Pending::Scan | Pending::BackgroundScan => {
                        if matches!(kind, Pending::Scan) {
                            game_setup::refreshed(&mut app);
                        }
                        playtime::replace_catalog(
                            &mut app.catalog,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                        );
                        let excluded = app.settings.excluded_games.clone();
                        app.catalog.games.retain(|g| !excluded.contains(&g.id));
                        issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
                        if !app
                            .requests
                            .values()
                            .any(|p| matches!(p, Pending::GameIcons))
                        {
                            issue(&mut app, Pending::GameIcons, "load_game_icons", json!({}));
                        }
                        if matches!(kind, Pending::Scan) {
                            let count = app.catalog.games.len();
                            show_toast(
                                &mut app,
                                Message::new("notice.scan_complete").arg("count", count),
                                false,
                            );
                        }
                        if !app.requests.values().any(|p| matches!(p, Pending::Enrich)) {
                            issue(&mut app, Pending::Enrich, "enrich_library", json!({}));
                        }
                    }
                    Pending::Folder => {
                        let paths: Option<Vec<String>> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(paths) = paths.filter(|paths| !paths.is_empty()) {
                            issue(
                                &mut app,
                                Pending::AddFolders,
                                "add_library_folders",
                                json!({"paths":paths}),
                            );
                        }
                    }
                    Pending::AddFolders => {
                        let result = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        library::folders::added(&mut app, id, result);
                    }
                    Pending::Theme => {
                        let settings: Settings =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.settings.theme = settings.theme;
                        app.settings.color_theme = settings.color_theme;
                    }
                    Pending::LibrarySize => {
                        let settings: Settings =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.settings.library_card_size = settings.library_card_size;
                        // 已在途的读取可能仍携带保存前的设置。
                        app.library.size_request = app.requests.next_id();
                    }
                    Pending::LibraryDisplay => {
                        let settings: Settings =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.settings.library_display_mode = settings.library_display_mode;
                        // 隔离保存期间发出的旧设置读取，避免后台响应切回旧模式。
                        app.library.display_request = app.requests.next_id();
                    }
                    Pending::Display => {
                        if !app.requests.has_newer_display(id) {
                            apply_settings(
                                &mut app,
                                serde_json::from_str(payload).map_err(|e| e.to_string())?,
                                id,
                            );
                        }
                    }
                    Pending::Preferences | Pending::Tools => {
                        apply_settings(
                            &mut app,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                            id,
                        );
                        if matches!(kind, Pending::Tools) {
                            // Do not let a previous preferences request undo a completed save.
                            app.magpie_observed_through = id.checked_sub(1);
                            app.screenshots.saved(id);
                            app.tool_preferences.saved(id);
                            features::clear_saved_tool_drafts(&mut app);
                            show_toast(&mut app, Message::new("notice.saved"), false);
                            issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
                        }
                    }
                    Pending::Language => {
                        apply_settings(
                            &mut app,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                            id,
                        );
                        show_toast(&mut app, Message::new("notice.language_saved"), false);
                        crate::startup::resume(&mut app);
                    }
                    Pending::Cover => {
                        let game: Option<Game> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(game) = game {
                            if let Some(entry) =
                                app.catalog.games.iter_mut().find(|g| g.id == game.id)
                            {
                                playtime::replace_game(entry, game);
                            }
                            show_toast(&mut app, Message::new("notice.cover_saved"), false);
                            if matches!(app.detail.editor, Some(detail::Editor::Cover)) {
                                app.detail.editor = None;
                            }
                        }
                    }
                    Pending::Launch => show_toast(&mut app, Message::new("notice.launched"), false),
                    Pending::Open => {}
                    Pending::Saves | Pending::ChooseSave => {
                        let data: serde_json::Value =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if !data.is_null() {
                            if let Some(settings) = data.get("settings") {
                                let settings = serde_json::from_value(settings.clone())
                                    .map_err(|e| e.to_string())?;
                                apply_settings(&mut app, settings, id);
                            }
                            let game_id = data["id"].as_str().unwrap_or_default().to_owned();
                            let report = serde_json::from_value(data["report"].clone())
                                .map_err(|e| e.to_string())?;
                            if app.detail.selected.as_deref() == Some(&game_id)
                                && (matches!(kind, Pending::ChooseSave) || !app.requests.values().any(|p| matches!(p, Pending::History(id) if id == &game_id)))
                            {
                                app.detail.current_snapshot = None;
                                issue(&mut app, Pending::History(game_id.clone()), "save_history", json!({"id":game_id}));
                            }
                            app.detail.saves.insert(game_id, report);
                        }
                    }
                    Pending::Edit
                    | Pending::DetailSaved
                    | Pending::PlayStatus
                    | Pending::MetadataField(_)
                    | Pending::SelectMetadata => {
                        let game: Game =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if matches!(kind, Pending::DetailSaved) {
                            detail_form::refresh_changed_title(&mut app, &game);
                        }
                        if let Some(entry) = app.catalog.games.iter_mut().find(|g| g.id == game.id)
                        {
                            playtime::replace_game(entry, game.clone());
                        }
                        if let Pending::MetadataField(field) = &kind {
                            let empty = Metadata::default();
                            let meta = game.metadata.as_ref().unwrap_or(&empty);
                            app.form.insert(
                                format!("meta-{field}"),
                                features::meta_value(meta, field).into(),
                            );
                        }
                        if matches!(kind, Pending::DetailSaved) {
                            detail_form::cancel(&mut app);
                        }
                        if matches!(kind, Pending::Edit) {
                            app.detail.tag_draft = None;
                            app.detail.editor = None;
                        }
                        issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
                        if matches!(kind, Pending::SelectMetadata) {
                            app.settings
                                .game_edits
                                .entry(game.id.clone())
                                .or_default()
                                .metadata
                                .clone_from(&game.metadata);
                            app.detail.candidates.remove(&game.id);
                            if matches!(
                                app.detail.editor,
                                Some(detail::Editor::Association | detail::Editor::TagSource)
                            ) {
                                app.detail.editor = None;
                            }
                        }
                        show_toast(&mut app, Message::new("notice.saved"), false);
                    }
                    Pending::Manual => {
                        issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
                        issue(&mut app, Pending::Enrich, "enrich_library", json!({}));
                        let game: Option<Game> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(game) = game {
                            if let Some(entry) =
                                app.catalog.games.iter_mut().find(|g| g.id == game.id)
                            {
                                playtime::replace_game(entry, game);
                            } else {
                                app.catalog.games.push(game);
                            }
                        }
                    }
                    Pending::Exclude(game_id) => {
                        let result: library::BatchResult = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, result.settings, id);
                        playtime::replace_catalog(&mut app.catalog, result.catalog);
                        if result.succeeded.contains(&game_id) {
                            app.library.checked.remove(&game_id);
                            app.detail.selected = None;
                        }
                        if let Some(failure) = result.failed.first() {
                            show_toast(&mut app, failure.error.clone(), true);
                        } else if let Some(warning) = result.cache_warning {
                            show_toast(&mut app, warning, true);
                        }
                    }
                    Pending::SearchMetadata(game_id, source) => {
                        let results = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if app.detail.metadata_source == source {
                            app.detail.candidates.insert(game_id, results);
                        }
                    }
                    Pending::Resources(game_id) => {
                        let data: serde_json::Value =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.patches.resources_game = game_id;
                        app.patches.resources = serde_json::from_value(data["resources"].clone())
                            .map_err(|e| e.to_string())?;
                        app.patches.resource_warnings =
                            serde_json::from_value(data["warnings"].clone())
                                .map_err(|e| e.to_string())?;
                    }
                    Pending::PatchPreview => {
                        app.patches.patch_preview =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    }
                    Pending::PatchList => {
                        app.patches.installed_patches =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.patches.installed_patches_loaded = true;
                        app.patches.patch_preview = None;
                    }
                    Pending::PatchTarget => {
                        let path: Option<String> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if path.is_some() {
                            app.patches.patch_target = path;
                            app.patches.patch_preview = None;
                        }
                    }
                    Pending::Enrich => {}
                    Pending::Update | Pending::BackgroundUpdate | Pending::InstallUpdate => {}
                }
                Ok(())
            })();
            if let Err(error) = handled {
                show_toast(&mut app, Message::detail("error.read_data", error), true);
            }
        }
        game_setup::sync(&mut app);
        paint(&app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::BackgroundImage, state::App};

    fn previous_background() -> BackgroundImage {
        BackgroundImage {
            id: "a".repeat(64),
            name: "existing.png".into(),
        }
    }

    #[test]
    fn cancelling_background_selection_preserves_the_image_without_a_saved_notice() {
        let mut app = App::default();
        app.settings.background_image = Some(previous_background());

        resolve_background(&mut app, 1, true, "null");

        assert_eq!(app.settings.background_image, Some(previous_background()));
        assert!(app.toast.is_none());
    }

    #[test]
    fn failed_background_selection_preserves_the_existing_image_and_reports_the_error() {
        let mut app = App::default();
        app.settings.background_image = Some(previous_background());

        resolve_background(
            &mut app,
            1,
            false,
            r#"{"key":"background.invalid","args":{}}"#,
        );

        assert_eq!(app.settings.background_image, Some(previous_background()));
        assert!(
            app.toast
                .is_some_and(|(notice, error)| error && notice.key == "background.invalid")
        );
    }

    #[test]
    fn clearing_background_applies_the_saved_settings() {
        let mut app = App::default();
        app.settings.background_image = Some(previous_background());

        resolve_background(
            &mut app,
            1,
            true,
            r#"{"library_folders":[],"card_size":200,"sort_by":"title","background_image":null}"#,
        );

        assert!(app.settings.background_image.is_none());
        assert!(
            app.toast
                .is_some_and(|(notice, error)| !error && notice.key == "notice.saved")
        );
    }
}
