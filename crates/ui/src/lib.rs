//! Rust/WASM UI composition root.

mod accounts;
mod actions;
mod ai;
mod ai_summary;
mod bridge;
mod choices;
mod cloud;
mod context_menu;
mod cover_picker;
mod detail;
mod detail_form;
mod downloads;
mod events;
mod features;
mod game_setup;
mod icons;
mod home;
mod game_actions;
mod library;
mod library_import;
mod local_patches;
mod models;
mod patches;
mod playtime;
mod preferences;
mod tool_preferences;
mod requests;
mod responses;
mod state;
mod view;
mod widgets;
mod updates;

pub use events::{
    cover_enhanced,
    backup_completed, choice_key, dispatch, download_progress, enrichment_progress,
    expire_download_notice, expire_toast, fan_status_updated, game_context, game_enriched,
    input_changed, library_drag_ids, library_drop, library_marquee, playtime_updated,
    runtime_updated, magpie_hotkey_updated, scan_progress, start, tool_failed,
};
pub use responses::resolve;
