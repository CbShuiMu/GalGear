//! Serializable UI models, independent of rendering and transport.
use galgear_i18n::{Language, Message};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Collection {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) parent: Option<String>,
    pub(crate) games: BTreeSet<String>,
}

#[derive(Clone, Default, Deserialize)]
pub(crate) struct Settings {
    pub(crate) library_folders: Vec<String>,
    #[serde(default)]
    pub(crate) collections: Vec<Collection>,
    #[serde(default)]
    pub(crate) excluded_games: Vec<String>,
    pub(crate) card_size: u32,
    pub(crate) sort_by: String,
    #[serde(default)]
    pub(crate) language: Language,
    #[serde(default)]
    pub(crate) game_edits: BTreeMap<String, GameEdit>,
    #[serde(default)]
    pub(crate) launch_behavior: String,
    #[serde(default)]
    pub(crate) close_behavior: String,
    #[serde(default)]
    pub(crate) release_page: String,
    #[serde(default)]
    pub(crate) download_folder: String,
    #[serde(default)]
    pub(crate) fan_domain: String,
    #[serde(default = "true_value")]
    pub(crate) show_patches: bool,
    #[serde(default = "default_font")]
    pub(crate) font_size: u32,
    #[serde(default = "default_retention")]
    pub(crate) backup_retention: usize,
    #[serde(default)]
    pub(crate) locale_emulator: String,
    #[serde(default)]
    pub(crate) magpie: String,
    #[serde(default)]
    pub(crate) magpie_hotkey: String,
    #[serde(default = "default_theme")]
    pub(crate) theme: String,
    #[serde(default = "default_color_theme")]
    pub(crate) color_theme: String,
    #[serde(default)]
    pub(crate) background_image: Option<BackgroundImage>,
    #[serde(default = "default_source")]
    pub(crate) metadata_provider: String,
    #[serde(default = "default_ai_provider")]
    pub(crate) ai_provider: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct BackgroundImage {
    pub(crate) id: String,
    pub(crate) name: String,
}
pub(crate) fn default_ai_provider() -> String {
    "deepseek".into()
}
pub(crate) fn default_source() -> String {
    "2dfan".into()
}
pub(crate) fn default_theme() -> String {
    "light".into()
}
pub(crate) fn default_color_theme() -> String {
    "pink".into()
}
pub(crate) fn default_font() -> u32 {
    16
}
pub(crate) fn default_retention() -> usize {
    10
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct FanCredentials {
    pub(crate) token: String,
    pub(crate) cookie: String,
    pub(crate) mode: String,
}
#[derive(Clone, Default, Deserialize)]
pub(crate) struct Account {
    pub(crate) status: String,
    pub(crate) username: Option<String>,
    pub(crate) balance: Option<u64>,
    pub(crate) reason: Option<Message>,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct FanStatus {
    pub(crate) account: Account,
    pub(crate) error: Option<Message>,
    pub(crate) refreshing: bool,
    pub(crate) revision: u64,
}
#[derive(Clone, Default, Deserialize)]
pub(crate) struct DownloadPreview {
    pub(crate) ticket: String,
    pub(crate) source_url: String,
    pub(crate) title: String,
    pub(crate) price: Option<u64>,
    pub(crate) balance: Option<u64>,
    pub(crate) requires_purchase: bool,
    pub(crate) can_download: bool,
    pub(crate) can_purchase: bool,
    #[serde(default)]
    pub(crate) free: bool,
    pub(crate) reason: Option<Message>,
}
#[derive(Clone, Default, Deserialize)]
pub(crate) struct Transfer {
    pub(crate) phase: String,
    pub(crate) title: String,
    pub(crate) received: u64,
    pub(crate) total: Option<u64>,
    pub(crate) path: Option<String>,
    pub(crate) error: Option<Message>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct Game {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) folder: String,
    pub(crate) executable: Option<String>,
    pub(crate) engine: String,
    #[serde(default)]
    pub(crate) play_status: String,
    pub(crate) cover: Option<String>,
    #[serde(default)]
    pub(crate) cover_artwork: Option<CoverArtwork>,
    pub(crate) modified: u64,
    #[serde(default)]
    pub(crate) playtime_seconds: u64,
    #[serde(default)]
    pub(crate) last_played: Option<u64>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default = "true_value")]
    pub(crate) available: bool,
    #[serde(default)]
    pub(crate) metadata: Option<Metadata>,
    #[serde(default)]
    pub(crate) metadata_warning: Option<Message>,
}
pub(crate) fn true_value() -> bool {
    true
}

#[derive(Clone, Deserialize)]
pub(crate) struct CoverArtwork {
    pub(crate) background: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct GameEdit {
    pub(crate) metadata_provider: String,
    pub(crate) setup_pending: bool,
    pub(crate) setup_confirmed: bool,
    pub(crate) title: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) source_tags: Option<Vec<String>>,
    pub(crate) summary_brand: Option<String>,
    pub(crate) summary_release_date: Option<String>,
    pub(crate) executable: Option<String>,
    pub(crate) arguments: Vec<String>,
    pub(crate) working_directory: Option<String>,
    pub(crate) launcher: Option<String>,
    pub(crate) save_path: Option<String>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) description: Option<String>,
    pub(crate) metadata_overrides: BTreeMap<String, String>,
    pub(crate) tag_rules: BTreeMap<String, Option<String>>,
    pub(crate) save_paths: Vec<String>,
    pub(crate) auto_backup: bool,
    pub(crate) launch_mode: String,
    pub(crate) locale_profile: String,
    pub(crate) upscale: bool,
    pub(crate) ai_summary: bool,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Metadata {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) developer: String,
    pub(crate) release_date: String,
    pub(crate) publisher: String,
    pub(crate) brand: String,
    pub(crate) artists: String,
    pub(crate) voices: String,
    pub(crate) writers: String,
    pub(crate) music: String,
    pub(crate) singers: String,
    pub(crate) tags: Vec<String>,
    pub(crate) links: Vec<MetadataLink>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct MetadataLink {
    pub(crate) field: String,
    pub(crate) name: String,
    pub(crate) url: String,
}

#[derive(Clone, Deserialize)]
pub(crate) struct SaveSnapshot {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) sequence: u64,
    pub(crate) created: u64,
    pub(crate) size: u64,
    pub(crate) note: String,
    pub(crate) locked: bool,
    pub(crate) files: Vec<SaveFile>,
}
#[derive(Clone, Deserialize)]
pub(crate) struct SaveFile {
    pub(crate) root: usize,
    pub(crate) path: String,
    pub(crate) size: u64,
}
#[derive(Clone, Deserialize)]
pub(crate) struct RestorePreview {
    pub(crate) snapshot: SaveSnapshot,
    pub(crate) targets: Vec<String>,
    pub(crate) changes: Vec<SaveChange>,
}
#[derive(Clone, Deserialize)]
pub(crate) struct SaveChange {
    pub(crate) root: usize,
    pub(crate) path: String,
    pub(crate) action: String,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Resource {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) source: String,
    pub(crate) description: String,
    pub(crate) game_title: String,
    pub(crate) tags: Vec<String>,
    pub(crate) author: String,
    pub(crate) uploaded_at: String,
    pub(crate) size: String,
}
#[derive(Clone, Default, Deserialize)]
pub(crate) struct SaveReport {
    pub(crate) locations: Vec<SaveLocation>,
    pub(crate) limited: bool,
}
#[derive(Clone, Default, Deserialize)]
pub(crate) struct SaveLocation {
    pub(crate) path: String,
    pub(crate) source: String,
    pub(crate) confidence: String,
    pub(crate) exists: bool,
}

#[derive(Clone, Deserialize)]
pub(crate) struct PatchPreview {
    pub(crate) title: String,
    pub(crate) game_id: String,
    pub(crate) target: String,
    pub(crate) files: Vec<PatchChange>,
    pub(crate) executable: Option<String>,
}
#[derive(Clone, Deserialize)]
pub(crate) struct PatchChange {
    pub(crate) path: String,
    pub(crate) before: Option<String>,
    pub(crate) size: u64,
}
#[derive(Clone, Deserialize)]
pub(crate) struct InstalledPatch {
    pub(crate) id: String,
    pub(crate) preview: PatchPreview,
    pub(crate) status: String,
}

#[derive(Clone, Default, Deserialize)]
pub(crate) struct Catalog {
    pub(crate) games: Vec<Game>,
    pub(crate) warnings: Vec<Message>,
}

#[derive(Deserialize)]
pub(crate) struct Bootstrap {
    #[serde(default)]
    pub(crate) ai_available: bool,
    pub(crate) settings: Settings,
    pub(crate) catalog: Catalog,
    pub(crate) config_path: String,
    pub(crate) version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn configuration_edits_preserve_cached_website_tags_schema_and_cover_fields() {
        let edit: GameEdit=serde_json::from_value(json!({"metadata":{"url":"https://example.invalid/game","tags":["ADV"],"schema_version":2,"cover_url":"https://example.invalid/cover.jpg"}})).unwrap();
        let roundtrip = serde_json::to_value(edit).unwrap();
        assert_eq!(
            (
                roundtrip["metadata"]["tags"].clone(),
                roundtrip["metadata"]["schema_version"].clone(),
                roundtrip["metadata"]["cover_url"].clone()
            ),
            (
                json!(["ADV"]),
                json!(2),
                json!("https://example.invalid/cover.jpg")
            )
        );
    }
}
