//! Desktop startup, protocols, tray and command registration.
use crate::{
    accounts, ai, ai_summary, background, cloud_commands,
    collections, covers, data_sources, downloads, game_details, game_setup, library, local_patches,
    metadata, patch_commands, preferences, save_commands, state::AppState, tools,
};
use galgear_i18n::{Message, tr};
use tauri::Manager;

#[tauri::command]
pub(crate) fn open_resource(url: String) -> Result<(), Message> {
    let parsed = reqwest::Url::parse(&url).map_err(|e| Message::detail("error.site_url", e))?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Message::new("error.site_url"));
    }
    open::that_detached(parsed.as_str()).map_err(|e| Message::detail("error.network", e))
}

pub(crate) fn run() {
    tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("diagnostics")
                .js_init_script(crate::diagnostics::FRONTEND_SCRIPT)
                .build(),
        )
        .on_page_load(|window, payload| {
            log::info!("page_load window={} event={:?}", window.label(), payload.event());
        })
        .register_uri_scheme_protocol("galgear-icon", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let bytes = if id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()) {
                std::fs::read(
                    context
                        .app_handle()
                        .state::<crate::startup::Startup>()
                        .data_dir
                        .join("icons")
                        .join(format!("{id}.png")),
                )
                .ok()
            } else {
                None
            };
            tauri::http::Response::builder()
                .status(if bytes.is_some() { 200 } else { 404 })
                .header("Content-Type", "image/png")
                .header("Access-Control-Allow-Origin", "*")
                .body(bytes.unwrap_or_default())
                .expect("Static icon response")
        })
        .register_uri_scheme_protocol("galgear-cover", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let bytes = if id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()) {
                let state = context.app_handle().state::<crate::startup::Startup>();
                std::fs::read(
                    state
                        .data_dir
                        .join("covers")
                        .join("rendered")
                        .join(format!("{id}.jpg")),
                )
                .ok()
            } else {
                None
            };
            tauri::http::Response::builder()
                .status(if bytes.is_some() { 200 } else { 404 })
                .header("Content-Type", "image/jpeg")
                .header("Access-Control-Allow-Origin", "*")
                .header("Cache-Control", "private, max-age=31536000, immutable")
                .body(bytes.unwrap_or_default())
                .expect("Static image response")
        })
        .register_uri_scheme_protocol("galgear-background", |context, request| {
            let id = request.uri().path().trim_start_matches('/');
            let cache = context
                .app_handle()
                .state::<crate::startup::Startup>()
                .data_dir
                .join("backgrounds");
            let bytes = galgear_core::background::cache_path(&cache, id)
                .and_then(|path| std::fs::read(path).ok());
            tauri::http::Response::builder()
                .status(if bytes.is_some() { 200 } else { 404 })
                .header("Content-Type", "image/jpeg")
                .header("Access-Control-Allow-Origin", "*")
                .header("Cache-Control", "private, max-age=31536000, immutable")
                .body(bytes.unwrap_or_default())
                .expect("Static background response")
        })
        .setup(|app| {
            crate::diagnostics::observe_events(app.handle());
            log::info!("desktop_setup webview_version={:?}", tauri::webview_version());
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                crate::diagnostics::observe_webview(&window, move || crate::webview_health::notify_exit(handle.clone()));
            }
            let exe_dir = std::env::current_exe()?
                .parent()
                .ok_or_else(|| std::io::Error::other("Executable directory"))?
                .to_owned();
            let data_dir = if exe_dir.join("portable.flag").exists() {
                exe_dir.join("data")
            } else {
                app.path().app_config_dir()?
            };
            log::info!("loading_state portable={} data_dir={}", exe_dir.join("portable.flag").exists(), data_dir.display());
            crate::startup::start(app.handle(), data_dir, exe_dir);
            let language = galgear_i18n::Language::default();
            let show = tauri::menu::MenuItem::with_id(
                app,
                "show",
                tr(language, "tray.show"),
                true,
                None::<&str>,
            )?;
            let quit = tauri::menu::MenuItem::with_id(
                app,
                "quit",
                tr(language, "settings.exit"),
                true,
                None::<&str>,
            )?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            tauri::tray::TrayIconBuilder::with_id("main")
                .icon(
                    app.default_window_icon()
                        .ok_or_else(|| std::io::Error::other("Missing window icon"))?
                        .clone(),
                )
                .menu(&menu)
                .tooltip("GalGear")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event
                        && let Some(window) = tray.app_handle().get_webview_window("main")
                    {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                })
                .build(app)?;
            log::info!("desktop_ready");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                log::info!("window_close_requested window={}", window.label());
                if crate::startup::close_to_tray(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(crate::startup::commands(crate::diagnostics::trace_commands(tauri::generate_handler![
            crate::diagnostics::record_diagnostic,
            crate::startup::bootstrap,
            crate::screenshots::latest_screenshot,
            crate::screenshots::open_screenshot_folder,
            crate::skill_package::get_ai_runtime,
            crate::skill_package::check_skill_update,
            crate::skill_package::check_skill_version,
            cloud_commands::cloud_status,
            cloud_commands::save_cloud_settings,
            cloud_commands::clear_cloud_settings,
            cloud_commands::test_cloud_connection,
            cloud_commands::cloud_upload,
            cloud_commands::cloud_list,
            cloud_commands::cloud_download,
            ai::get_ai_providers,
            ai::list_ai_models,
            ai::save_ai_provider,
            ai::clear_ai_provider,
            ai::set_active_ai_provider,
            ai_summary::set_ai_summary,
            ai_summary::summarize_game,
            ai_summary::prepare_ai_route_map,
            ai_summary::open_ai_route_map,
            game_setup::choose_setup_executable,
            game_setup::complete_game_setup,
            game_setup::complete_game_setup_batch,
            game_setup::rematch_game_metadata,
            game_setup::list_game_executables,
            game_setup::confirm_game_executable,
            library::reorder_games,
            covers::load_game_icons,
            accounts::load_fan_credentials,
            accounts::save_fan_credentials,
            accounts::clear_fan_credentials,
            accounts::get_fan_status_cache,
            accounts::refresh_fan_status,
            downloads::choose_download_folder,
            downloads::open_download_folder,
            downloads::inspect_fan_resource,
            downloads::start_fan_download,
            downloads::start_nextmoe_download,
            downloads::cancel_fan_download,
            patch_commands::preview_downloaded_patch,
            library::scan_library,
            library::cancel_scan,
            save_commands::detect_saves,
            save_commands::choose_backup_folder,
            save_commands::open_backup_folder,
            save_commands::choose_save_folder,
            save_commands::choose_save_files,
            save_commands::observe_saves,
            save_commands::find_save_with_ai,
            save_commands::open_save_folder,
            game_details::edit_game,
            library::add_manual_game,
            library::relocate_game,
            library::exclude_game,
            preferences::set_launch_behavior,
            preferences::set_close_behavior,
            preferences::set_release_page,
            crate::updater::check_update,
            crate::updater::install_update,
            metadata::search_metadata,
            data_sources::get_data_sources,
            data_sources::save_data_source,
            data_sources::login_data_source,
            data_sources::cancel_data_source_login,
            data_sources::clear_data_source,
            data_sources::set_data_source,
            metadata::select_metadata,
            open_resource,
            metadata::enrich_library,
            patch_commands::preview_patch,
            patch_commands::choose_patch_target,
            patch_commands::install_patch,
            patch_commands::restore_patch,
            patch_commands::list_patches,
            local_patches::list_game_patches,
            local_patches::apply_game_patch,
            local_patches::open_game_patch_folder,
            patch_commands::run_patch_installer,
            patch_commands::set_exe_status,
            patch_commands::search_patches,
            library::choose_library_folder,
            library::add_library_folder,
            library::choose_library_folders,
            library::add_library_folders,
            library::remove_library_folder,
            preferences::save_preferences,
            preferences::set_language,
            covers::choose_cover,
            covers::source_covers,
            covers::select_source_cover,
            tools::open_game_folder,
            tools::launch_game,
            preferences::get_preferences,
            preferences::set_theme,
            preferences::set_color_theme,
            preferences::set_display,
            preferences::set_library_card_size,
            preferences::set_library_display_mode,
            background::choose_background_image,
            background::clear_background_image,
            game_details::edit_title,
            game_details::save_game_details,
            metadata::title_refresh::refresh_title_metadata,
            game_details::set_play_status,
            preferences::set_patch_tab,
            preferences::set_collection_tab,
            game_details::edit_metadata,
            game_details::edit_tags,
            covers::reset_cover,
            game_details::reset_game_tags,
            collections::edit_collection,
            collections::delete_collections,
            collections::batch_games,
            collections::excluded_entries,
            collections::set_library_bounds,
            save_commands::confirm_save,
            save_commands::save_history,
            save_commands::backup_saves,
            save_commands::preview_save_restore,
            save_commands::restore_saves,
            save_commands::update_snapshot,
            tools::configure_tools,
            tools::magpie_modes,
            crate::export::export_potatovn,
            crate::import::prepare_library_import,
            crate::import::confirm_library_import,
            tools::set_game_tools,
            tools::game_runtime,
            tools::upscale_game,
        ])))
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            log::error!("startup_failed {error:?}");
            rfd::MessageDialog::new()
                .set_title("GalGear")
                .set_description(
                    Message::detail("diagnostics.startup_failed", &error).render(Default::default()),
                )
                .set_level(rfd::MessageLevel::Error)
                .show();
            log::logger().flush();
            std::process::exit(1);
        })
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = &event {
                log::info!("exit_requested code={code:?}");
                if crate::startup::defer_exit(app, *code) { api.prevent_exit(); }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                log::info!("shutdown_start");
                if let Some(service) = app.try_state::<crate::screenshots::Service>() { service.stop(); }
                if let Some(service) = app.try_state::<crate::font_tools::Service>() { service.stop(); }
                #[cfg(windows)]
                if let Some(service) = app.try_state::<crate::game_input::Service>() { service.stop(); }
                if let Some(state) = app.try_state::<AppState>()
                    && let Err(error) = tools::shutdown_magpie(&state) {
                    log::error!("Magpie shutdown: {error:?}");
                }
                galgear_core::ai_summary::shutdown();
                crate::diagnostics::finish();
            }
        });
}
