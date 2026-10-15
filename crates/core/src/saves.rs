//! Read-only, bounded save discovery. A matching name is evidence, not certainty.
use crate::{Game, GameEdit, config::display_path};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveLocation {
    pub path: String,
    pub source: String,
    pub confidence: String,
    pub exists: bool,
    #[serde(default)]
    pub readable: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SaveReport {
    pub locations: Vec<SaveLocation>,
    pub searched: Vec<String>,
    pub limited: bool,
}

pub fn user_roots() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("APPDATA") {
        roots.push(("roaming".into(), PathBuf::from(path)));
    }
    if let Some(path) = documents_folder() {
        roots.push(("documents".into(), path));
    }
    roots
}

#[cfg(windows)]
fn documents_folder() -> Option<PathBuf> {
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath},
    };
    unsafe {
        let mut value = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut value) < 0 {
            return None;
        }
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        let result = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
            value, length,
        )));
        CoTaskMemFree(value.cast());
        Some(result)
    }
}
#[cfg(not(windows))]
fn documents_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Documents"))
}

fn key(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}
fn save_name(name: &str) -> bool {
    matches!(
        key(name).as_str(),
        "savedata" | "save" | "saves" | "saved" | "存档" | "セーブデータ"
    )
}
fn save_files(folder: &Path) -> bool {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .take(100)
        .filter_map(Result::ok)
        .any(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            e.file_type().is_ok_and(|t| t.is_file())
                && (name.starts_with("save")
                    || [".dat", ".sav", ".ksd", ".asd", ".rpgsave", ".rmmzsave"]
                        .iter()
                        .any(|suffix| name.ends_with(suffix)))
                && fs::File::open(e.path()).is_ok()
        })
}
fn add(report: &mut SaveReport, path: PathBuf, source: &str, confidence: &str) {
    let path = display_path(&path).replace('/', std::path::MAIN_SEPARATOR_STR);
    if let Some(existing) = report
        .locations
        .iter_mut()
        .find(|p| p.path.eq_ignore_ascii_case(&path))
    {
        if existing.source != "manual" && source == "config" {
            existing.source = source.into();
            existing.confidence = confidence.into();
        }
        return;
    }
    if !report
        .locations
        .iter()
        .any(|p| p.path.eq_ignore_ascii_case(&path))
    {
        report.locations.push(SaveLocation {
            exists: Path::new(&path).is_dir(),
            readable: save_files(Path::new(&path)),
            path,
            source: source.into(),
            confidence: confidence.into(),
        });
    }
}

pub fn discover(game: &Game, edit: Option<&GameEdit>) -> SaveReport {
    discover_in(game, edit, &user_roots())
}

pub fn discover_in(
    game: &Game,
    edit: Option<&GameEdit>,
    roots: &[(String, PathBuf)],
) -> SaveReport {
    let mut report = SaveReport::default();
    if let Some(path) = edit
        .and_then(|e| e.save_path.as_ref())
        .filter(|p| !p.trim().is_empty())
    {
        add(&mut report, PathBuf::from(path), "manual", "confirmed");
    }
    if let Some(edit) = edit {
        for path in &edit.save_paths {
            add(&mut report, PathBuf::from(path), "manual", "confirmed");
        }
    }
    if game.executable.is_none() {
        return report;
    }
    let folder = Path::new(&game.folder);
    if game.engine == "AdvHD"
        && let Ok(paths) = crate::auto_unlock::save_locations(folder)
    {
        for path in paths {
            let path = PathBuf::from(path);
            if path.is_dir() || path.ends_with("savedata") {
                add(&mut report, path, "config", "configured");
            }
        }
    }
    report.searched.push(display_path(folder));
    for entry in fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_ok_and(|t| t.is_dir())
            && save_name(&entry.file_name().to_string_lossy())
        {
            add(&mut report, entry.path(), "game", "detected");
        }
    }
    // Explicit engine configuration provides useful evidence even before first launch.
    let mut configured = Vec::new();
    let mut configured_base = None;
    for name in ["SETUP.INI", "setup.ini", "angeltear.xml", "config.ini"] {
        let path = folder.join(name);
        if !fs::metadata(&path).is_ok_and(|m| m.len() <= 128 * 1024) {
            continue;
        }
        if let Ok(bytes) = fs::read(&path) {
            let text = if let Ok(text) = std::str::from_utf8(&bytes) {
                text.to_owned()
            } else if bytes.starts_with(&[0xff, 0xfe]) {
                encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned()
            } else {
                encoding_rs::SHIFT_JIS.decode(&bytes).0.into_owned()
            };
            for line in text.lines() {
                let line = line.trim();
                if let Some((k, v)) = line.split_once('=')
                    && matches!(key(k).as_str(), "savedir" | "savepath" | "savedatapath")
                {
                    configured.push(v.trim().trim_matches('"').to_owned());
                }
                if let Some((k, v)) = line.split_once('=')
                    && key(k) == "savedatalocation"
                {
                    configured_base = Some(v.trim().to_owned());
                }
                if line.contains("<save_dir")
                    && let Some(v) = line
                        .split("path=\"")
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                {
                    configured.push(v.to_owned());
                }
            }
        }
    }
    let mut aliases: BTreeSet<String> = [
        key(&game.title),
        key(folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref()),
    ]
    .into();
    if let Some(exe) = &game.executable {
        let value = key(Path::new(exe)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref());
        if !["advhd", "advhdnew", "game", "launcher", "siglusengine"].contains(&value.as_str()) {
            aliases.insert(value);
        }
    }
    aliases.retain(|s| s.chars().count() >= 4);
    for value in configured {
        let normalized = value.replace('\\', "/");
        if let Some(base) = &configured_base {
            for (source, root) in roots {
                let token = if source == "roaming" {
                    "<ROAMINGAPPDATA>"
                } else {
                    "<PERSONAL>"
                };
                if let Some(relative) = base.replace('\\', "/").strip_prefix(token) {
                    add(
                        &mut report,
                        root.join(relative.trim_start_matches('/'))
                            .join(&normalized),
                        "config",
                        "configured",
                    );
                }
            }
        }
        if normalized.contains("$(CURRENT)") {
            add(
                &mut report,
                folder.join(normalized.replace("$(CURRENT)", "").trim_start_matches('/')),
                "config",
                "configured",
            );
        } else if Path::new(&normalized).is_absolute() {
            add(
                &mut report,
                PathBuf::from(normalized),
                "config",
                "configured",
            );
        } else if !normalized.contains("..") && !normalized.contains('$') {
            if let Some(first) = normalized.split('/').next() {
                aliases.insert(key(first));
            }
            let local = folder.join(&normalized);
            if local.is_dir() {
                add(&mut report, local, "config", "configured");
            }
            for (source, root) in roots {
                let path = root.join(&normalized);
                if path.is_dir()
                    || (configured_base.is_none() && game.engine == "AdvHD" && source == "roaming")
                {
                    add(&mut report, path, "config", "configured");
                }
            }
        }
    }
    let mut visited = 0;
    for (source, root) in roots {
        report.searched.push(display_path(root));
        search(root, source, &aliases, 0, false, &mut visited, &mut report);
    }
    for location in &mut report.locations {
        if location.source == "config" && location.exists && location.readable {
            location.confidence = "confirmed".into();
        }
        if location.source == "game"
            && location.readable
            && matches!(
                game.engine.as_str(),
                "AdvHD" | "KiriKiri" | "Entis GLS" | "Ren’Py" | "RPG Maker"
            )
        {
            location.confidence = "confirmed".into();
        }
    }
    report
}

fn search(
    path: &Path,
    source: &str,
    aliases: &BTreeSet<String>,
    depth: usize,
    matched: bool,
    visited: &mut usize,
    report: &mut SaveReport,
) {
    if depth > 4 {
        return;
    }
    if *visited >= 5000 {
        report.limited = true;
        return;
    }
    *visited += 1;
    let name = key(&path.file_name().unwrap_or_default().to_string_lossy());
    let matches = matched
        || aliases
            .iter()
            .any(|a| name == *a || (a.chars().count() >= 8 && name.contains(a)));
    if matches && (save_name(&name) || save_files(path)) {
        add(report, path.to_owned(), source, "matched");
        return;
    }
    for entry in fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        if *visited >= 5000 {
            report.limited = true;
            break;
        }
        if entry
            .file_type()
            .is_ok_and(|t| t.is_dir() && !t.is_symlink())
        {
            let lower = entry.file_name().to_string_lossy().to_lowercase();
            if ["node_modules", "cache", "logs", "crashdumps", ".git"].contains(&lower.as_str()) {
                continue;
            }
            search(
                &entry.path(),
                source,
                aliases,
                depth + 1,
                matches,
                visited,
                report,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_three_roots_and_does_not_claim_another_games_saves() {
        let t = tempfile::tempdir().unwrap();
        let game_dir = t.path().join("Angel Tear");
        let roaming = t.path().join("Roaming");
        let docs = t.path().join("Redirected Documents");
        for dir in [
            game_dir.join("SaveData"),
            roaming.join("Nitro+/Angel Tear/savedata"),
            docs.join("Leaf/Angel Tear/save"),
            docs.join("Other Game/save"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        let game =
            crate::catalog::manual_game(&game_dir.join("angeltear.exe"), "Angel Tear".into())
                .unwrap();
        let report = discover_in(
            &game,
            None,
            &[("roaming".into(), roaming), ("documents".into(), docs)],
        );
        assert_eq!(report.locations.len(), 3);
        assert!(report.locations.iter().all(|l| l.exists));
        assert!(
            !report
                .locations
                .iter()
                .any(|l| l.path.contains("Other Game"))
        );
    }
    #[test]
    fn configured_missing_path_is_shown_as_uncreated() {
        let t = tempfile::tempdir().unwrap();
        fs::write(
            t.path().join("SETUP.INI"),
            "SAVEDIR=NightmareXBlackCat\\savedata",
        )
        .unwrap();
        let mut game =
            crate::catalog::manual_game(&t.path().join("AdvHD.exe"), "BlackCat".into()).unwrap();
        game.engine = "AdvHD".into();
        let report = discover_in(&game, None, &[("roaming".into(), t.path().join("Roaming"))]);
        assert_eq!(report.locations.len(), 1);
        assert!(!report.locations[0].exists);
        assert_eq!(report.locations[0].confidence, "configured");
    }

    #[test]
    fn japanese_setup_resolves_roaming_base_and_deduplicates_paths() {
        let t = tempfile::tempdir().unwrap();
        let content = "DLGTITLE=インストール\nSAVEDIR=GameProduct\\savedata\nSAVEDATALOCATION=<ROAMINGAPPDATA>\\WillPlus\\Guilty";
        let bytes = encoding_rs::SHIFT_JIS.encode(content).0.into_owned();
        fs::write(t.path().join("SETUP.INI"), bytes).unwrap();
        let mut game =
            crate::catalog::manual_game(&t.path().join("AdvHD.exe"), "ゲーム".into()).unwrap();
        game.engine = "AdvHD".into();
        let report = discover_in(&game, None, &[("roaming".into(), t.path().join("Roaming"))]);
        assert_eq!(report.locations.len(), 1);
        assert!(report.locations[0].path.ends_with(&format!(
            "WillPlus{0}Guilty{0}GameProduct{0}savedata",
            std::path::MAIN_SEPARATOR
        )));
        assert!(!report.locations[0].exists);
    }
}
