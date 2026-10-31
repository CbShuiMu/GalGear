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
mod dialogs;
mod export;
mod downloads;
mod fan_cache;
mod game_details;
mod game_setup;
mod library;
mod import;
mod local_patches;
mod metadata;
mod patch_commands;
mod preferences;
mod save_commands;
mod skill_package;
mod state;
mod tools;
mod updater;

fn main() {
    if updater::run_helper() { return; }
    #[cfg(windows)]
    if bundled_tools::run_magpie_guard() {
        return;
    }
    desktop::run();
}
