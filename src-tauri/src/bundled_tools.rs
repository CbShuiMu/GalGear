//! Resolve portable tools relative to GalGear, independent of the launch working directory.
use galgear_core::{Settings, runtime};
use galgear_i18n::Message;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod process;
#[cfg(windows)]
pub(crate) use process::{
    Child as MagpieProcess, start as start_magpie, stop as stop_magpie,
    wait_existing as wait_magpie, run_guard as run_magpie_guard,
};

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
    if !settings.magpie.is_empty() {
        // Read the portable configuration on startup as well as before every launch.
        // A malformed tool configuration must not prevent opening the game library.
        if let Ok(hotkey) = magpie_hotkey(&resolve(base, &settings.magpie), &settings.magpie_hotkey)
        {
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
    let config = magpie_config(executable);
    if !config.is_file() {
        runtime::hotkey_keys(fallback).map_err(|e| Message::detail("tools.error", e))?;
        return Ok(fallback.into());
    }
    let bytes = std::fs::read(config).map_err(|e| Message::detail("tools.error", e))?;
    let value: serde_json::Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|e| Message::detail("tools.error", e))?;
    let encoded = value["shortcuts"]["scale"]
        .as_u64()
        .ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))?;
    decode_hotkey(encoded).ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))
}

fn decode_hotkey(encoded: u64) -> Option<String> {
    let key = u8::try_from(encoded & 0xff).ok()?;
    if encoded > 0xfff {
        return None;
    }
    let mut parts = Vec::new();
    for (flag, name) in [
        (0x100, "Win"),
        (0x200, "Ctrl"),
        (0x400, "Alt"),
        (0x800, "Shift"),
    ] {
        if encoded & flag != 0 {
            parts.push(name.to_owned());
        }
    }
    if key != 0 {
        if matches!(key, 0x10..=0x12 | 0x5b) {
            return None;
        }
        parts.push(runtime::hotkey_key_name(u16::from(key))?);
    }
    let hotkey = parts.join("+");
    runtime::hotkey_keys(&hotkey).ok()?;
    Some(hotkey)
}

fn encode_hotkey(hotkey: &str) -> Result<u64, Message> {
    let keys =
        runtime::hotkey_keys(hotkey).map_err(|_| Message::new("tools.magpie_hotkey_invalid"))?;
    Ok(keys.into_iter().fold(0, |encoded, key| {
        encoded
            | match key {
                0x5b => 0x100,
                0x11 => 0x200,
                0x12 => 0x400,
                0x10 => 0x800,
                key => u64::from(key),
            }
    }))
}

pub(crate) fn canonical_magpie_hotkey(hotkey: &str) -> Result<String, Message> {
    decode_hotkey(encode_hotkey(hotkey)?).ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))
}

fn magpie_config(executable: &Path) -> PathBuf {
    let portable = executable
        .parent()
        .unwrap_or(Path::new("."))
        .join("config/config.json");
    if !portable.is_file()
        && let Some(local) = std::env::var_os("LOCALAPPDATA")
    {
        let installed = PathBuf::from(local).join("Magpie/config/v5/config.json");
        if installed.is_file() {
            return installed;
        }
    }
    portable
}

/// Replace only the scaling shortcut, keeping all other Magpie settings intact.
/// The caller stops a running Magpie first, so its pending save cannot overwrite this edit.
pub(crate) fn write_magpie_hotkey(executable: &Path, hotkey: &str) -> Result<String, Message> {
    let (config, value, encoded) = hotkey_document(executable, hotkey)?;
    let mut temporary = tempfile::NamedTempFile::new_in(config.parent().unwrap_or(Path::new(".")))
        .map_err(|e| Message::detail("tools.error", e))?;
    let output =
        serde_json::to_vec_pretty(&value).map_err(|e| Message::detail("tools.error", e))?;
    temporary
        .write_all(&output)
        .map_err(|e| Message::detail("tools.error", e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| Message::detail("tools.error", e))?;
    temporary
        .persist(config)
        .map_err(|e| Message::detail("tools.error", e))?;
    decode_hotkey(encoded).ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))
}

pub(crate) fn validate_magpie_hotkey(executable: &Path, hotkey: &str) -> Result<(), Message> {
    hotkey_document(executable, hotkey).map(|_| ())
}

fn hotkey_document(
    executable: &Path,
    hotkey: &str,
) -> Result<(PathBuf, serde_json::Value, u64), Message> {
    let encoded = encode_hotkey(hotkey)?;
    let config = magpie_config(executable);
    let bytes = std::fs::read(&config).map_err(|e| Message::detail("tools.error", e))?;
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|e| Message::detail("tools.error", e))?;
    let shortcuts = value
        .get_mut("shortcuts")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| Message::new("tools.magpie_hotkey_invalid"))?;
    if shortcuts
        .iter()
        .any(|(action, key)| action != "scale" && key.as_u64() == Some(encoded))
    {
        return Err(Message::new("tools.magpie_hotkey_conflict"));
    }
    shortcuts.insert("scale".into(), encoded.into());
    Ok((config, value, encoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_update_preserves_profiles_and_other_shortcuts() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("config")).unwrap();
        let config = temp.path().join("config/config.json");
        let original = serde_json::json!({"shortcuts":{"scale":3137,"windowedModeScale":3153},"scalingModes":[{"name":"My mode","effects":["FSR"]}],"profiles":[{"name":"Game"}],"unknown":{"keep":true}});
        std::fs::write(&config, serde_json::to_vec(&original).unwrap()).unwrap();
        let executable = temp.path().join("Magpie.exe");
        assert_eq!(
            write_magpie_hotkey(&executable, "shift+alt+b").unwrap(),
            "Alt+Shift+B"
        );
        let mut expected = original;
        expected["shortcuts"]["scale"] = 3138.into();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&config).unwrap()).unwrap(),
            expected
        );
        assert_eq!(magpie_hotkey(&executable, "").unwrap(), "Alt+Shift+B");
    }

    #[test]
    fn conflicting_or_malformed_config_is_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("config")).unwrap();
        let config = temp.path().join("config/config.json");
        let executable = temp.path().join("Magpie.exe");
        let conflict = br#"{"shortcuts":{"scale":3137,"windowedModeScale":3138}}"#;
        std::fs::write(&config, conflict).unwrap();
        assert_eq!(
            write_magpie_hotkey(&executable, "Alt+Shift+B")
                .unwrap_err()
                .key,
            "tools.magpie_hotkey_conflict"
        );
        assert_eq!(std::fs::read(&config).unwrap(), conflict);
        std::fs::write(&config, b"{partial").unwrap();
        assert!(write_magpie_hotkey(&executable, "Alt+Shift+B").is_err());
        assert_eq!(std::fs::read(&config).unwrap(), b"{partial");
    }

    #[test]
    fn magpie_key_codes_roundtrip_function_windows_and_modifier_shortcuts() {
        for hotkey in [
            "Alt+Shift+A",
            "Win+Ctrl+F12",
            "Ctrl+NumPad3",
            "Alt+Tab",
            "Ctrl+PrintScreen",
            "Ctrl+Alt",
            "Shift+Semicolon",
        ] {
            assert_eq!(canonical_magpie_hotkey(hotkey).unwrap(), hotkey);
        }
        for hotkey in ["Ctrl+A+B", "Ctrl+Ctrl+A", "A+Ctrl", "F12"] {
            assert!(canonical_magpie_hotkey(hotkey).is_err(), "{hotkey}");
        }
    }

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
