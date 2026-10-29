pub mod ai;
pub mod ai_summary;
pub mod auto_unlock;
pub mod background;
pub mod cover_art;
pub mod catalog;
pub mod cloud;
pub mod collections;
pub mod config;
pub mod credentials;
pub mod data_sources;
pub mod fan_account;
pub mod game_details;
pub mod game_setup;
pub mod launcher_icons;
pub mod patches;
pub mod playtime;
pub mod play_sessions;
pub mod providers;
pub mod runtime;
pub mod saves;
pub mod scanner;
pub mod snapshots;
pub mod tags;
pub mod unlock;

use galgear_i18n::{Language, Message};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub library_folders: Vec<String>,
    pub cover_overrides: BTreeMap<String, String>,
    pub card_size: u32,
    pub sort_by: String,
    pub language: Language,
    pub game_edits: BTreeMap<String, GameEdit>,
    pub excluded_games: Vec<String>,
    pub excluded_entries: BTreeMap<String, Game>,
    pub collections: Vec<collections::Collection>,
    pub next_collection_id: u64,
    pub manual_games: Vec<Game>,
    pub launch_behavior: String,
    pub close_behavior: String,
    pub release_page: String,
    pub download_folder: String,
    pub fan_domain: String,
    pub metadata_checks: BTreeMap<String, u64>,
    pub metadata_attempt_versions: BTreeMap<String, u32>,
    pub dialog_directories: BTreeMap<String, String>,
    pub cover_scale: u32,
    pub font_size: u32,
    /// Legacy shared directory, retained only to migrate existing snapshots.
    pub backup_folder: String,
    pub backup_retention: usize,
    pub cloud: cloud::CloudSettings,
    pub locale_emulator: String,
    pub magpie: String,
    pub magpie_hotkey: String,
    pub theme: String,
    pub color_theme: String,
    pub background_image: Option<background::BackgroundImage>,
    pub show_patches: bool,
    pub metadata_provider: String,
    pub ai_models: BTreeMap<String, String>,
    pub ai_provider: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            library_folders: Vec::new(),
            cover_overrides: BTreeMap::new(),
            card_size: 200,
            sort_by: "title".into(),
            language: Language::default(),
            game_edits: BTreeMap::new(),
            excluded_games: Vec::new(),
            excluded_entries: BTreeMap::new(),
            collections: Vec::new(),
            next_collection_id: 0,
            manual_games: Vec::new(),
            launch_behavior: "keep".into(),
            close_behavior: "exit".into(),
            release_page: String::new(),
            download_folder: String::new(),
            fan_domain: "https://2dfmax.top".into(),
            metadata_checks: BTreeMap::new(),
            metadata_attempt_versions: BTreeMap::new(),
            dialog_directories: BTreeMap::new(),
            cover_scale: 100,
            font_size: 16,
            backup_folder: String::new(),
            backup_retention: 10,
            cloud: cloud::CloudSettings::default(),
            locale_emulator: String::new(),
            magpie: String::new(),
            magpie_hotkey: "Ctrl+Alt+A".into(),
            theme: "light".into(),
            color_theme: "pink".into(),
            background_image: None,
            show_patches: true,
            metadata_provider: "2dfan".into(),
            ai_models: BTreeMap::new(),
            ai_provider: "deepseek".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GameEdit {
    pub metadata_provider: String,
    pub setup_pending: bool,
    pub setup_confirmed: bool,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub source_tags: Option<Vec<String>>,
    pub summary_brand: Option<String>,
    pub summary_release_date: Option<String>,
    pub executable: Option<String>,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub launcher: Option<String>,
    pub save_path: Option<String>,
    pub metadata: Option<providers::Metadata>,
    pub description: Option<String>,
    pub metadata_overrides: BTreeMap<String, String>,
    pub tag_rules: BTreeMap<String, Option<String>>,
    pub save_paths: Vec<String>,
    pub auto_backup: bool,
    pub launch_mode: String,
    pub locale_profile: String,
    pub upscale: bool,
    pub ai_summary: bool,
    pub play_status: PlayStatus,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayStatus {
    #[default]
    Unmarked,
    Wanted,
    Playing,
    Finished,
    Shelved,
    Abandoned,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Game {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub library: String,
    pub executable: Option<String>,
    pub engine: String,
    #[serde(default)]
    pub play_status: PlayStatus,
    pub cover: Option<String>,
    #[serde(default)]
    pub cover_artwork: Option<cover_art::Artwork>,
    pub modified: u64,
    #[serde(default)]
    pub playtime_seconds: u64,
    #[serde(default)]
    pub last_played: Option<u64>,
    pub cg_unlock_status: String,
    pub auto_unlock: auto_unlock::Summary,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "available_default")]
    pub available: bool,
    #[serde(default)]
    pub metadata: Option<providers::Metadata>,
    #[serde(default)]
    pub scan_stamp: String,
    #[serde(default)]
    pub metadata_warning: Option<Message>,
}

fn available_default() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanResult {
    pub games: Vec<Game>,
    pub warnings: Vec<Message>,
    pub folders_scanned: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bootstrap {
    #[serde(default)]
    pub ai_available: bool,
    pub settings: Settings,
    pub catalog: ScanResult,
    pub config_path: String,
    pub version: String,
    pub cg_engines: Vec<unlock::EngineSupport>,
}
