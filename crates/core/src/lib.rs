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
pub mod downloads;
pub mod fan_account;
mod fan_http;
pub mod font_tools;
pub mod game_details;
pub mod game_setup;
pub mod launcher_icons;
pub mod library_import;
pub mod library_order;
pub mod patches;
pub mod playtime;
pub mod potatovn;
pub mod play_sessions;
pub mod providers;
pub mod rate_limit;
pub mod runtime;
pub mod saves;
pub mod scanner;
pub mod screenshots;
pub mod snapshots;
pub mod tags;
pub mod unlock;
pub mod updates;

use galgear_i18n::{Language, Message};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RELEASE_PAGE: &str = "https://github.com/CbShuiMu/GalGear/releases";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub library_folders: Vec<String>,
    pub cover_overrides: BTreeMap<String, String>,
    pub card_size: u32,
    /// 游戏库卡面档位；旧配置保持 normal，独立于历史封面缩放设置。
    pub library_card_size: String,
    /// 游戏库显示模式，poster 为海报墙，caption 为封面加标题；旧配置默认海报墙。
    pub library_display_mode: String,
    pub sort_by: String,
    pub custom_order: Vec<String>,
    pub sort_descending: Option<bool>,
    pub language: Language,
    /// 仅首次创建配置时为 true；旧配置缺失该字段时不触发首次语言选择。
    pub initial_setup_pending: bool,
    pub game_edits: BTreeMap<String, GameEdit>,
    pub excluded_games: Vec<String>,
    /// 永久移除后仅保留扫描排除路径，显式重新导入时清除。
    pub excluded_paths: Vec<String>,
    pub excluded_entries: BTreeMap<String, Game>,
    pub collections: Vec<collections::Collection>,
    pub next_collection_id: u64,
    pub manual_games: Vec<Game>,
    pub launch_behavior: String,
    pub close_behavior: String,
    pub release_page: String,
    pub download_folder: String,
    /// 旧版自选截图目录，仅保留配置兼容；新截图固定存入游戏目录。
    pub screenshot_folder: String,
    pub screenshot_enabled: bool,
    /// 游戏前台时生效的全局截图快捷键。
    pub screenshot_hotkey: String,
    /// 游戏前台时打开字体调节面板的快捷键；旧配置默认 Shift+T。
    pub font_hotkey: String,
    /// 游戏字体面板的已提交参数；旧配置默认为空。
    pub game_fonts: BTreeMap<String, font_tools::Profile>,
    pub fan_domain: String,
    pub metadata_checks: BTreeMap<String, u64>,
    pub metadata_attempt_versions: BTreeMap<String, u32>,
    pub dialog_directories: BTreeMap<String, String>,
    pub cover_scale: u32,
    pub font_size: u32,
    /// 全局存档备份目录，空值使用应用数据目录下的 save-backups。
    pub backup_folder: String,
    /// 变更备份目录后继续迁移旧根目录，包括暂时离线的位置。
    pub backup_legacy_folders: Vec<String>,
    pub backup_retention: usize,
    pub cloud: cloud::CloudSettings,
    pub locale_emulator: String,
    pub magpie: String,
    pub magpie_hotkey: String,
    pub theme: String,
    pub color_theme: String,
    pub background_image: Option<background::BackgroundImage>,
    pub show_patches: bool,
    pub show_collections: bool,
    pub metadata_provider: String,
    /// 各入口手动选择的数据源；没有覆盖的入口跟随默认数据源。
    pub source_preferences: BTreeMap<String, String>,
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
            library_card_size: "normal".into(),
            library_display_mode: "poster".into(),
            sort_by: "title".into(),
            custom_order: Vec::new(),
            sort_descending: None,
            language: Language::default(),
            initial_setup_pending: false,
            game_edits: BTreeMap::new(),
            excluded_games: Vec::new(),
            excluded_paths: Vec::new(),
            excluded_entries: BTreeMap::new(),
            collections: Vec::new(),
            next_collection_id: 0,
            manual_games: Vec::new(),
            launch_behavior: "keep".into(),
            close_behavior: "exit".into(),
            release_page: RELEASE_PAGE.into(),
            download_folder: String::new(),
            screenshot_folder: String::new(),
            screenshot_enabled: true,
            screenshot_hotkey: "Shift+S".into(),
            font_hotkey: "Shift+T".into(),
            game_fonts: BTreeMap::new(),
            fan_domain: "https://2dfmax.top".into(),
            metadata_checks: BTreeMap::new(),
            metadata_attempt_versions: BTreeMap::new(),
            dialog_directories: BTreeMap::new(),
            cover_scale: 100,
            font_size: 16,
            backup_folder: String::new(),
            backup_legacy_folders: Vec::new(),
            backup_retention: 10,
            cloud: cloud::CloudSettings::default(),
            locale_emulator: String::new(),
            magpie: String::new(),
            magpie_hotkey: "Ctrl+Alt+A".into(),
            theme: "light".into(),
            color_theme: "pink".into(),
            background_image: None,
            show_patches: true,
            show_collections: true,
            metadata_provider: "nextmoe".into(),
            source_preferences: BTreeMap::new(),
            ai_models: BTreeMap::new(),
            ai_provider: "deepseek".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GameEdit {
    pub metadata_provider: String,
    pub setup_pending: bool,
    pub setup_confirmed: bool,
    pub launcher_pending: bool,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub source_tags: Option<Vec<String>>,
    /// 独立标签关联的词条；旧配置缺失时按既有来源偏好回退。
    pub tag_source_url: Option<String>,
    /// 兼容旧配置的品牌快照；界面统一使用当前 metadata 的品牌。
    pub summary_brand: Option<String>,
    pub summary_release_date: Option<String>,
    pub executable: Option<String>,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub launcher: Option<String>,
    pub save_files: Vec<String>,
    pub save_path: Option<String>,
    pub save_path_auto: bool,
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

impl Default for GameEdit {
    fn default() -> Self {
        Self {
            metadata_provider: String::new(),
            setup_pending: false,
            setup_confirmed: false,
            launcher_pending: false,
            title: None,
            tags: Vec::new(),
            source_tags: None,
            tag_source_url: None,
            summary_brand: None,
            summary_release_date: None,
            executable: None,
            arguments: Vec::new(),
            working_directory: None,
            launcher: None,
            save_files: Vec::new(),
            save_path: None,
            save_path_auto: false,
            metadata: None,
            description: None,
            metadata_overrides: BTreeMap::new(),
            tag_rules: BTreeMap::new(),
            save_paths: Vec::new(),
            auto_backup: false,
            launch_mode: "normal".into(),
            locale_profile: String::new(),
            upscale: false,
            ai_summary: false,
            play_status: PlayStatus::default(),
        }
    }
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
    /// 来自 NextMoe 的多语言名称与别名；旧缓存默认为空。
    #[serde(default)]
    pub aliases: Vec<String>,
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
    #[serde(default)]
    pub units_completed: usize,
    #[serde(default)]
    pub units_total: usize,
    pub games: Vec<Game>,
    pub warnings: Vec<Message>,
    pub folders_scanned: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bootstrap {
    #[serde(default)]
    pub update_complete: bool,
    #[serde(default)]
    pub ai_available: bool,
    pub settings: Settings,
    pub catalog: ScanResult,
    pub config_path: String,
    pub version: String,
    pub cg_engines: Vec<unlock::EngineSupport>,
}
