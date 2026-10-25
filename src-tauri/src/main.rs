#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod ai;
mod ai_summary;
mod cloud_commands;
mod collections;
mod covers;
mod data_sources;
mod default_browser;
mod desktop;
mod dialogs;
mod downloads;
mod fan_cache;
mod game_details;
mod game_setup;
mod library;
mod local_patches;
mod metadata;
mod patch_commands;
mod preferences;
mod save_commands;
mod state;
mod tools;

fn main() {
    desktop::run();
}
