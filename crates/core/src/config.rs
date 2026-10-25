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
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
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
