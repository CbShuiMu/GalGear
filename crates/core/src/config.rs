use crate::Settings;
use galgear_i18n::Message;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct ConfigStore {
    pub path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Settings, Message> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let mut settings: Settings = serde_json::from_slice(&bytes).map_err(|e| {
                    Message::detail(
                        "error.config_parse",
                        format!("{}: {e}", self.path.display()),
                    )
                })?;
                if settings.version != 1 {
                    return Err(Message::new("error.config_version"));
                }
                // The retired white theme becomes the light white-pink palette.
                if settings.theme == "white" {
                    settings.theme = "light".into();
                    settings.color_theme = "white-pink".into();
                }
                if settings.release_page.trim().is_empty() {
                    settings.release_page = crate::RELEASE_PAGE.into();
                }
                // An empty launch mode is the old untouched default; explicit
                // user choices are stored as "normal" or "locale".
                for edit in settings.game_edits.values_mut() {
                    if edit.launch_mode.is_empty() {
                        edit.launch_mode = "normal".into();
                    }
                }
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings {
                initial_setup_pending: true,
                ..Settings::default()
            }),
            Err(e) => Err(Message::detail("error.config_read", e)),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), Message> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Message::new("error.config_path"))?;
        fs::create_dir_all(parent).map_err(|e| Message::detail("error.config_write", e))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| Message::detail("error.config_write", e))?;
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|e| Message::detail("error.config_write", e))?;
        temporary
            .write_all(&bytes)
            .map_err(|e| Message::detail("error.config_write", e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| Message::detail("error.config_write", e))?;
        temporary
            .persist(&self.path)
            .map_err(|e| Message::detail("error.config_write", e))?;
        Ok(())
    }
}

/// 设置界面语言；仅首次选择时同时初始化默认数据源，后续切换保留来源偏好。
pub fn select_language(settings: &mut Settings, language: galgear_i18n::Language) {
    settings.language = language;
    if settings.initial_setup_pending {
        settings.metadata_provider = match language {
            galgear_i18n::Language::Chinese => "nextmoe",
            galgear_i18n::Language::English => "vndb",
            galgear_i18n::Language::Japanese => "erogamescape",
        }
        .into();
        settings.initial_setup_pending = false;
    }
}

pub fn normalize_folder(path: &str) -> Result<String, Message> {
    if path.trim().is_empty() {
        return Err(Message::new("error.folder_required"));
    }
    let canonical = fs::canonicalize(Path::new(path.trim()))
        .map_err(|e| Message::detail("error.folder_access", e))?;
    if !canonical.is_dir() {
        return Err(Message::new("error.folder_select"));
    }
    Ok(display_path(&canonical))
}

pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(network) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{network}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_card_size_defaults_for_old_configs_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.json"));
        fs::write(
            &store.path,
            r#"{"version":1,"card_size":240,"cover_scale":150}"#,
        )
        .unwrap();
        let mut settings = store.load().unwrap();
        assert_eq!(settings.library_card_size, "normal");
        for size in ["small", "tiny", "normal"] {
            settings.library_card_size = size.into();
            store.save(&settings).unwrap();
            let loaded = store.load().unwrap();
            assert_eq!(loaded.library_card_size, size);
            assert_eq!((loaded.card_size, loaded.cover_scale), (240, 150));
        }
    }

    #[test]
    fn first_language_initializes_source_once_and_survives_reopening() {
        for (language, source) in [
            (galgear_i18n::Language::Chinese, "nextmoe"),
            (galgear_i18n::Language::English, "vndb"),
            (galgear_i18n::Language::Japanese, "erogamescape"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let store = ConfigStore::new(temp.path().join("settings.json"));
            let mut settings = store.load().unwrap();
            assert!(settings.initial_setup_pending);
            // 未完成的首次设置即使被其他配置写入保存，下次仍继续提示。
            store.save(&settings).unwrap();
            assert!(store.load().unwrap().initial_setup_pending);
            select_language(&mut settings, language);
            store.save(&settings).unwrap();
            let mut reopened = store.load().unwrap();
            assert_eq!(reopened.language, language);
            assert_eq!(reopened.metadata_provider, source);
            assert!(!reopened.initial_setup_pending);
            reopened.metadata_provider = "bangumi".into();
            select_language(&mut reopened, galgear_i18n::Language::English);
            assert_eq!(reopened.metadata_provider, "bangumi");
        }
    }

    #[test]
    fn old_or_invalid_config_is_never_treated_as_a_new_installation() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("settings.json"));
        fs::write(
            &store.path,
            r#"{"version":1,"language":"ja-JP","metadata_provider":"vndb"}"#,
        )
        .unwrap();
        let settings = store.load().unwrap();
        assert!(!settings.initial_setup_pending);
        assert_eq!(settings.metadata_provider, "vndb");
        assert_eq!(settings.language, galgear_i18n::Language::Japanese);
        fs::write(&store.path, "invalid configuration").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn font_shortcut_defaults_for_old_config_and_survives_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("settings.json"));
        fs::write(&store.path, r#"{"version":1}"#).unwrap();
        let mut settings = store.load().unwrap();
        assert_eq!(settings.font_hotkey, "Shift+T");
        settings.font_hotkey = "Ctrl+Alt+F9".into();
        store.save(&settings).unwrap();
        assert_eq!(ConfigStore::new(store.path).load().unwrap(), settings);
    }

    #[test]
    fn feature_visibility_defaults_on_and_survives_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("settings.json"));
        fs::write(&store.path, r#"{"version":1}"#).unwrap();
        let mut settings = store.load().unwrap();
        assert!(settings.show_collections && settings.show_patches);
        settings.show_collections = false;
        store.save(&settings).unwrap();
        let reloaded = store.load().unwrap();
        assert!(!reloaded.show_collections && reloaded.show_patches);
    }

    #[test]
    fn launch_defaults_are_off_and_preserve_saved_choices() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("settings.json"));
        fs::write(
            &store.path,
            r#"{"version":1,"game_edits":{
            "legacy":{"launch_mode":"","upscale":false},
            "missing":{},
            "custom":{"launch_mode":"normal","upscale":false},
            "enabled":{"launch_mode":"locale","upscale":true},
            "legacy_upscale":{"launch_mode":"","upscale":true}
        }}"#,
        )
        .unwrap();
        let mut settings = store.load().unwrap();
        for id in ["legacy", "missing"] {
            assert_eq!(settings.game_edits[id].launch_mode, "normal");
            assert!(!settings.game_edits[id].upscale);
        }
        assert_eq!(settings.game_edits["custom"].launch_mode, "normal");
        assert!(!settings.game_edits["custom"].upscale);
        assert_eq!(settings.game_edits["enabled"].launch_mode, "locale");
        assert!(settings.game_edits["enabled"].upscale);
        assert_eq!(settings.game_edits["legacy_upscale"].launch_mode, "normal");
        assert!(settings.game_edits["legacy_upscale"].upscale);
        settings
            .game_edits
            .insert("new".into(), crate::GameEdit::default());
        assert_eq!(settings.game_edits["new"].launch_mode, "normal");
        assert!(!settings.game_edits["new"].upscale);
        let edit = settings.game_edits.get_mut("legacy").unwrap();
        edit.launch_mode = "locale".into();
        edit.upscale = true;
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn release_page_defaults_for_new_and_old_settings_and_keeps_custom_urls() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp.path().join("settings.json"));
        assert_eq!(store.load().unwrap().release_page, crate::RELEASE_PAGE);
        for json in [r#"{"version":1}"#, r#"{"version":1,"release_page":"  "}"#] {
            fs::write(&store.path, json).unwrap();
            assert_eq!(store.load().unwrap().release_page, crate::RELEASE_PAGE);
        }
        let settings = Settings {
            release_page: "https://github.com/example/custom/releases".into(),
            ..Default::default()
        };
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap().release_page, settings.release_page);
    }

    #[test]
    fn custom_background_defaults_off_and_survives_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, r#"{"version":1}"#).unwrap();
        let store = ConfigStore::new(path.clone());
        let mut settings = store.load().unwrap();
        assert!(settings.background_image.is_none());
        let image = crate::background::BackgroundImage {
            id: "a".repeat(64),
            name: "自选背景.png".into(),
        };
        settings.background_image = Some(image.clone());
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap().background_image, Some(image));
        settings.background_image = None;
        store.save(&settings).unwrap();
        assert!(store.load().unwrap().background_image.is_none());
    }

    #[test]
    fn cloud_settings_round_trip_and_older_settings_default_to_disconnected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, r#"{"version":1}"#).unwrap();
        let store = ConfigStore::new(path.clone());
        let mut settings = store.load().unwrap();
        assert!(settings.cloud.url.is_empty());
        settings.cloud = crate::cloud::CloudSettings {
            url: "https://dav.example.com/dav/".into(),
            username: "user@example.com".into(),
            remote_path: "GalGear/存档".into(),
        };
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(json["cloud"].get("password").is_none());
    }

    #[test]
    fn settings_survive_reopening_and_replacing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let store = ConfigStore::new(path.clone());
        let mut settings = Settings::default();
        settings.library_folders.push("F:\\游戏库\\中文目录".into());
        settings.card_size = 240;
        settings.theme = "dark".into();
        store.save(&settings).unwrap();
        assert_eq!(ConfigStore::new(path.clone()).load().unwrap(), settings);
        settings.card_size = 180;
        settings.color_theme = "white-pink".into();
        settings.sort_descending = Some(true);
        settings.sort_by = "release".into();
        store.save(&settings).unwrap();
        assert_eq!(ConfigStore::new(path).load().unwrap(), settings);
    }

    #[test]
    fn retired_white_theme_loads_as_light_white_pink_without_changing_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let original = r#"{"version":1,"theme":"white"}"#;
        fs::write(&path, original).unwrap();
        let settings = ConfigStore::new(path.clone()).load().unwrap();
        assert_eq!(
            (settings.theme.as_str(), settings.color_theme.as_str()),
            ("light", "white-pink")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn existing_dark_theme_keeps_the_original_pink_palette() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, r#"{"version":1,"theme":"dark"}"#).unwrap();
        let settings = ConfigStore::new(path).load().unwrap();
        assert_eq!(
            (settings.theme.as_str(), settings.color_theme.as_str()),
            ("dark", "pink")
        );
    }

    #[test]
    fn malformed_config_is_reported_and_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, "broken data").unwrap();
        assert!(ConfigStore::new(path.clone()).load().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken data");
    }

    #[test]
    fn old_config_defaults_to_chinese_and_language_survives_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(
            &path,
            r#"{"version":1,"library_folders":["F:\\GALGame"],"card_size":240,"sort_by":"recent"}"#,
        )
        .unwrap();
        let store = ConfigStore::new(path.clone());
        let mut settings = store.load().unwrap();
        assert_eq!(settings.language, galgear_i18n::Language::Chinese);
        settings.language = galgear_i18n::Language::Japanese;
        store.save(&settings).unwrap();
        let restored = ConfigStore::new(path).load().unwrap();
        assert_eq!(restored, settings);
        assert_eq!(restored.library_folders, vec!["F:\\GALGame"]);
    }
}
