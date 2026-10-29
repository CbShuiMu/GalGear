//! Resolve portable tools relative to GalGear, independent of the launch working directory.
use galgear_core::{Settings, runtime};
use galgear_i18n::Message;
use std::path::{Path, PathBuf};

pub(crate) const MAGPIE: &str = "tools/magpie/Magpie.exe";
pub(crate) const LOCALE: &str = "tools/locale-emulator/LEProc.exe";

pub(crate) fn resolve(base: &Path, configured: &str) -> PathBuf {
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

pub(crate) fn apply_defaults(settings: &mut Settings, base: &Path) -> bool {
    let previous = (
        settings.magpie.clone(),
        settings.locale_emulator.clone(),
        settings.magpie_hotkey.clone(),
    );
    for (configured, bundled) in [
        (&mut settings.magpie, MAGPIE),
        (&mut settings.locale_emulator, LOCALE),
    ] {
        if base.join(bundled).is_file()
            && (configured.trim().is_empty() || !resolve(base, configured).is_file())
        {
            *configured = bundled.into();
        }
    }
    if settings.magpie == MAGPIE {
        // Read the portable configuration on startup as well as before every launch.
        // A malformed tool configuration must not prevent opening the game library.
        if let Ok(hotkey) = magpie_hotkey(&base.join(MAGPIE), &settings.magpie_hotkey) {
            settings.magpie_hotkey = hotkey;
        }
    }
    previous
        != (
            settings.magpie.clone(),
            settings.locale_emulator.clone(),
            settings.magpie_hotkey.clone(),
        )
}

pub(crate) fn magpie_hotkey(executable: &Path, fallback: &str) -> Result<String, Message> {
    let config = executable
        .parent()
        .unwrap_or(Path::new("."))
        .join("config/config.json");
    if !config.is_file() {
        runtime::hotkey_keys(fallback).map_err(|e| Message::detail("tools.error", e))?;
        return Ok(fallback.into());
    }
    let bytes = std::fs::read(config).map_err(|e| Message::detail("tools.error", e))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| Message::detail("tools.error", e))?;
    let encoded = value["shortcuts"]["scale"]
        .as_u64()
        .ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))?;
    decode_hotkey(encoded).ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))
}

fn decode_hotkey(encoded: u64) -> Option<String> {
    let key = u8::try_from(encoded & 0xff).ok()?;
    if encoded > 0xfff || encoded & 0x100 != 0 || !key.is_ascii_alphanumeric() {
        return None;
    }
    let mut parts = Vec::new();
    for (flag, name) in [(0x200, "Ctrl"), (0x400, "Alt"), (0x800, "Shift")] {
        if encoded & flag != 0 {
            parts.push(name.to_owned());
        }
    }
    parts.push(char::from(key).to_string());
    let hotkey = parts.join("+");
    runtime::hotkey_keys(&hotkey).ok()?;
    Some(hotkey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_paths_survive_moving_the_whole_application() {
        let first = tempfile::tempdir().unwrap();
        let moved = tempfile::tempdir().unwrap();
        for base in [first.path(), moved.path()] {
            for path in [MAGPIE, LOCALE] {
                let file = base.join(path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, []).unwrap();
            }
        }
        let mut settings = Settings::default();
        assert!(apply_defaults(&mut settings, first.path()));
        assert_eq!(settings.magpie, MAGPIE);
        assert_eq!(settings.locale_emulator, LOCALE);
        assert!(resolve(moved.path(), &settings.magpie).is_file());
        assert!(resolve(moved.path(), &settings.locale_emulator).is_file());
    }

    #[test]
    fn portable_magpie_reads_the_actual_scaling_shortcut() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("config")).unwrap();
        let config = temp.path().join("config/config.json");
        std::fs::write(&config, r#"{"shortcuts":{"scale":3137}}"#).unwrap();
        assert_eq!(
            magpie_hotkey(&temp.path().join("Magpie.exe"), "Ctrl+Alt+A").unwrap(),
            "Alt+Shift+A"
        );
        std::fs::write(config, r#"{"shortcuts":{"scale":0}}"#).unwrap();
        assert!(magpie_hotkey(&temp.path().join("Magpie.exe"), "Ctrl+Alt+A").is_err());
    }

    #[test]
    fn valid_external_tool_paths_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let custom = temp.path().join("custom.exe");
        std::fs::write(&custom, []).unwrap();
        std::fs::create_dir_all(temp.path().join("tools/magpie")).unwrap();
        std::fs::write(temp.path().join(MAGPIE), []).unwrap();
        let mut settings = Settings {
            magpie: custom.to_string_lossy().into_owned(),
            ..Default::default()
        };
        apply_defaults(&mut settings, temp.path());
        assert_eq!(Path::new(&settings.magpie), custom);
    }
}
