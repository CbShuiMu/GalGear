//! Associate numeric cloud-save folders only with an AppID supplied by local configuration.
use super::{SaveReport, add, display_path};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn small_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > 128 * 1024 {
        return None;
    }
    use std::io::Read;
    let mut text = String::new();
    file.take(128 * 1024 + 1).read_to_string(&mut text).ok()?;
    (text.len() <= 128 * 1024).then_some(text)
}

fn app_id(text: &str) -> Option<String> {
    let text = text.trim().trim_start_matches('\u{feff}').trim();
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
}

fn app_ids(folder: &Path) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for name in ["steam_appid.txt", "steam_settings/steam_appid.txt"] {
        if let Some(id) = small_text(&folder.join(name)).as_deref().and_then(app_id) {
            ids.insert(id);
        }
    }
    for (name, expected_section) in [
        ("steam_settings/configs.app.ini", "app"),
        ("steam_emu.ini", "steam"),
    ] {
        let Some(text) = small_text(&folder.join(name)) else {
            continue;
        };
        let mut section = String::new();
        for line in text.lines().map(str::trim) {
            if let Some(value) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = value.to_ascii_lowercase();
            } else if section == expected_section
                && let Some((name, value)) = line.split_once('=')
                && name.trim().eq_ignore_ascii_case("appid")
                && let Some(id) = app_id(value.split(';').next().unwrap_or("").trim())
            {
                ids.insert(id);
            }
        }
    }
    // A Steam library's manifest binds installdir to AppID even when the game
    // ships no steam_appid.txt. Do not infer identity from a numeric folder alone.
    let Ok(expression) = regex::Regex::new(r#""(appid|installdir)"\s*"([^"\r\n]+)""#) else {
        return ids;
    };
    for common in folder.ancestors().filter(|path| {
        path.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("common"))
    }) {
        let Some(steamapps) = common.parent().filter(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("steamapps"))
        }) else {
            continue;
        };
        for entry in fs::read_dir(steamapps)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .take(1024)
        {
            let filename = entry.file_name();
            let name = filename.to_string_lossy();
            let Some(filename_id) = name
                .strip_prefix("appmanifest_")
                .and_then(|name| name.strip_suffix(".acf"))
                .and_then(app_id)
            else {
                continue;
            };
            let Some(text) = small_text(&entry.path()) else {
                continue;
            };
            let mut manifest_id = None;
            let mut installed = None;
            for fields in expression.captures_iter(&text) {
                match &fields[1] {
                    "appid" => manifest_id = app_id(&fields[2]),
                    "installdir" => installed = Some(fields[2].to_owned()),
                    _ => {}
                }
            }
            if manifest_id.as_ref() == Some(&filename_id)
                && installed.as_ref().is_some_and(|name| {
                    let path = Path::new(name);
                    path.components().count() == 1 && folder.starts_with(common.join(path))
                })
            {
                ids.insert(filename_id);
            }
        }
    }
    ids
}

fn existing(report: &mut SaveReport, path: PathBuf) {
    report.searched.push(display_path(&path));
    if fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        add(report, path, "config", "configured");
    }
}

pub(super) fn discover(folder: &Path, roots: &[(String, PathBuf)], report: &mut SaveReport) {
    let ids = app_ids(folder);
    if ids.is_empty() {
        return;
    }
    let mut userdata = BTreeSet::new();
    for (source, root) in roots {
        match source.as_str() {
            "roaming" => {
                for id in &ids {
                    existing(
                        report,
                        root.join("Goldberg SteamEmu Saves").join(id).join("remote"),
                    );
                }
            }
            "steam" => {
                userdata.insert(root.join("userdata"));
            }
            "steam-userdata" => {
                userdata.insert(root.clone());
            }
            _ => {}
        }
    }
    for steamapps in folder.ancestors().filter(|path| {
        path.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("steamapps"))
    }) {
        if let Some(steam) = steamapps.parent() {
            userdata.insert(steam.join("userdata"));
        }
    }
    for root in userdata {
        report.searched.push(display_path(&root));
        for account in fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .take(256)
        {
            if !account
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                || !account
                    .file_name()
                    .to_string_lossy()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
            {
                continue;
            }
            for id in &ids {
                existing(report, account.path().join(id).join("remote"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{catalog, saves};

    #[test]
    fn appid_configuration_associates_only_its_own_numeric_emulator_directory() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("Unrelated display title");
        let roaming = root.path().join("Roaming");
        fs::create_dir_all(game.join("steam_settings")).unwrap();
        fs::write(
            game.join("steam_settings/steam_appid.txt"),
            "\u{feff}424242\r\n",
        )
        .unwrap();
        for id in ["424242", "131313"] {
            let save = roaming
                .join("Goldberg SteamEmu Saves")
                .join(id)
                .join("remote");
            fs::create_dir_all(&save).unwrap();
            fs::write(save.join("chapter.ksd"), b"data").unwrap();
        }
        let game =
            catalog::manual_game(&game.join("Novel.exe"), "Different localized title".into())
                .unwrap();
        let report = saves::discover_in(&game, None, &[("roaming".into(), roaming)]);
        assert_eq!(report.locations.len(), 1);
        assert!(report.locations[0].path.contains("424242"));
    }

    #[test]
    fn steam_userdata_uses_manifest_install_directory_and_all_local_accounts() {
        let root = tempfile::tempdir().unwrap();
        let steam = root.path().join("Steam library");
        let game = steam.join("steamapps/common/Moon Garden");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            steam.join("steamapps/appmanifest_78901.acf"),
            "\"AppState\" { \"appid\" \"78901\" \"installdir\" \"Moon Garden\" }",
        )
        .unwrap();
        for account in ["101", "202"] {
            let save = steam.join("userdata").join(account).join("78901/remote");
            fs::create_dir_all(&save).unwrap();
            fs::write(save.join("slot.sav"), b"save").unwrap();
        }
        let game =
            catalog::manual_game(&game.join("Novel.exe"), "Localized garden".into()).unwrap();
        let report = saves::discover_in(&game, None, &[]);
        assert_eq!(report.locations.len(), 2);
    }

    #[test]
    fn malformed_or_path_like_appids_do_not_escape_the_emulator_root() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("steam_appid.txt"), "../../123").unwrap();
        let mut report = SaveReport::default();
        discover(
            root.path(),
            &[("roaming".into(), root.path().to_owned())],
            &mut report,
        );
        assert!(report.locations.is_empty());
    }
}
