//! User-provided Google Material symbols; unmapped icons use the existing renderer.

pub(super) fn google(name: &str) -> Option<&'static str> {
    Some(match name {
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
