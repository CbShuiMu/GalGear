#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod ai;
mod ai_summary;
mod background;
mod bundled_tools;
mod cloud_commands;
mod collections;
mod covers;
mod cover_art;
mod data_sources;
mod default_browser;
mod desktop;
mod diagnostics;
mod dialogs;
mod export;
mod downloads;
mod fan_cache;
mod folders;
mod font_tools;
mod game_details;
mod game_setup;
#[cfg(windows)]
mod game_window;
#[cfg(windows)]
mod game_input;
mod library;
mod library_devices;
mod import;
mod local_patches;
mod metadata;
mod patch_commands;
mod preferences;
mod save_commands;
mod screenshots;
mod skill_package;
mod state;
mod tools;
mod updater;
#[cfg(windows)]
mod webview_health;

fn main() {
    #[cfg(windows)]
    if game_input::run_helper() { return; }
    if updater::run_helper() { return; }
    #[cfg(windows)]
    if bundled_tools::run_magpie_guard() {
        return;
    }
    diagnostics::initialize();
    desktop::run();
}
