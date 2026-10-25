//! User-provided Google Material symbols; unmapped icons use the existing renderer.

pub(super) fn google(name: &str) -> Option<&'static str> {
    Some(match name {
        "sparkle" => {
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 2 14.8 9.2 22 12 14.8 14.8 12 22 9.2 14.8 2 12 9.2 9.2Z"/></svg>"#
        }
        "database" => include_str!(
            "../../../src-tauri/icons/database_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "open_in_new" => include_str!(
            "../../../src-tauri/icons/open_in_new_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "download" => include_str!(
            "../../../src-tauri/icons/download_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "gamepad" => include_str!(
            "../../../src-tauri/icons/gamepad_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "star" => {
            include_str!("../../../src-tauri/icons/star_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg")
        }
        "checklist" => include_str!(
            "../../../src-tauri/icons/checklist_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "create_new_folder" => include_str!(
            "../../../src-tauri/icons/create_new_folder_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "save" => {
            include_str!("../../../src-tauri/icons/save_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg")
        }
        "folder_open" => include_str!(
            "../../../src-tauri/icons/folder_open_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "update" => include_str!(
            "../../../src-tauri/icons/update_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "edit" => {
            include_str!("../../../src-tauri/icons/edit_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg")
        }
        "games" => include_str!(
            "../../../src-tauri/icons/videogame_asset_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "patches" => include_str!(
            "../../../src-tauri/icons/extension_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "settings" => include_str!(
            "../../../src-tauri/icons/settings_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "folder" => include_str!(
            "../../../src-tauri/icons/folder_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "search" => include_str!(
            "../../../src-tauri/icons/search_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "plus" => {
            include_str!("../../../src-tauri/icons/add_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg")
        }
        "refresh" => include_str!(
            "../../../src-tauri/icons/refresh_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "close" => include_str!(
            "../../../src-tauri/icons/close_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "play" => include_str!(
            "../../../src-tauri/icons/play_arrow_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "image" => include_str!(
            "../../../src-tauri/icons/image_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "check" => include_str!(
            "../../../src-tauri/icons/check_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "disc" => include_str!(
            "../../../src-tauri/icons/album_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "error" => include_str!(
            "../../../src-tauri/icons/error_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "backup" => include_str!(
            "../../../src-tauri/icons/backup_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "folder_special" => include_str!(
            "../../../src-tauri/icons/folder_special_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "location_searching" => include_str!(
            "../../../src-tauri/icons/location_searching_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "sync" => {
            include_str!("../../../src-tauri/icons/sync_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg")
        }
        "light_mode" => include_str!(
            "../../../src-tauri/icons/light_mode_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        "dark_mode" => include_str!(
            "../../../src-tauri/icons/dark_mode_24dp_1F1F1F_FILL0_wght400_GRAD0_opsz24.svg"
        ),
        _ => return None,
    })
}
