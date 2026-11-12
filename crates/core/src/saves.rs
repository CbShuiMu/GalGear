//! Read-only, bounded save discovery. A matching name is evidence, not certainty.
pub mod ai_find;
mod kirikiri;
mod loose;
pub mod observation;
mod paths;
mod steam;

use crate::{Game, GameEdit, config::display_path};
pub use kirikiri::{SaveKind, classify};
pub(crate) use kirikiri::backup_identity;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveLocation {
    pub path: String,
    /// 混合目录中明确选中的文件。空列表表示专用存档目录。
    #[serde(default)]
    pub files: Vec<String>,
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

/// 展开备份范围，文件集合只返回明确选中的文件。
pub fn location_roots(location: &SaveLocation) -> Vec<String> {
    if location.files.is_empty() {
        vec![location.path.clone()]
    } else {
        location.files.clone()
    }
}

/// 校验文件及其上级路径，拒绝符号链接和 Windows 重解析点。
pub fn validate_file(path: &Path) -> Result<(), galgear_i18n::Message> {
    validate_components(path)?;
    if !path.is_file() || fs::File::open(path).is_err() {
        return Err(galgear_i18n::Message::new("save.ai_not_found"));
    }
    Ok(())
}

pub(crate) fn validate_components(path: &Path) -> Result<(), galgear_i18n::Message> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|error| galgear_i18n::Message::detail("backup.error", error))?;
        let mut linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            linked |= metadata.file_attributes() & 0x400 != 0;
        }
        if linked {
            return Err(galgear_i18n::Message::detail(
                "backup.error",
                "Save path contains a link",
            ));
        }
    }
    Ok(())
}

pub fn user_roots() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(path);
        if let Some(parent) = local.parent() {
            roots.push(("local-low".into(), parent.join("LocalLow")));
        }
        roots.push(("local".into(), local));
    }
    if let Some(path) = std::env::var_os("APPDATA") {
        roots.push(("roaming".into(), PathBuf::from(path)));
    }
    if let Some(path) = documents_folder() {
        roots.push(("documents".into(), path));
    }
    if let Some(path) = saved_games_folder() {
        roots.push(("saved-games".into(), path));
    }
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = std::env::var_os(variable) {
            let path = PathBuf::from(base).join("Steam");
            if path.is_dir() && !roots.iter().any(|(_, existing)| *existing == path) {
                roots.push(("steam".into(), path));
            }
        }
    }
    roots
}

#[cfg(windows)]
fn documents_folder() -> Option<PathBuf> {
    known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Documents)
}
#[cfg(windows)]
fn saved_games_folder() -> Option<PathBuf> {
    known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_SavedGames)
}
#[cfg(windows)]
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use windows_sys::Win32::{System::Com::CoTaskMemFree, UI::Shell::SHGetKnownFolderPath};
    unsafe {
        let mut value = std::ptr::null_mut();
        if SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut value) < 0 {
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
#[cfg(not(windows))]
fn saved_games_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Saved Games"))
}

fn key(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}
fn game_key(value: &str) -> String {
    let mut normalized = key(value);
    // 保留 HD/FHD 版本标识，避免把重制版与原版混为一谈。
    if normalized.ends_with("hd版") {
        normalized.truncate(normalized.len() - "版".len());
    }
    normalized
}
fn save_name(name: &str) -> bool {
    let normalized = key(name);
    matches!(
        normalized.as_str(),
        "savedata" | "save" | "saves" | "saved" | "存档" | "セーブデータ"
    ) || ["save", "savedata"].iter().any(|prefix| {
        normalized.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(suffix, "chs" | "cht" | "cn" | "jp" | "en" | "jpn" | "eng" | "zh" | "zhcn" | "zhtw")
        })
    })
}

/// 只比较目录直属存档文件；复制目录、日志或说明文件的更新时间不是游玩证据。
pub(crate) fn latest_modified(folder: &Path) -> Option<std::time::SystemTime> {
    fs::read_dir(folder)
        .ok()?
        .take(2048)
        .filter_map(Result::ok)
        .filter(save_file)
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
}

fn sort_recent(report: &mut SaveReport) {
    report.locations.sort_by_cached_key(|location| std::cmp::Reverse((
        location.exists,
        location.readable,
        latest_modified(Path::new(&location.path)),
    )));
}

fn select_recent_ai(report: &mut SaveReport, selected: &Path, game: &Game) {
    add(report, selected.to_path_buf(), "ai", "confirmed");
    let selected = display_path(selected);
    let game_root = Path::new(&game.folder).canonicalize().ok();
    report.locations.retain(|location| {
        location.exists
            && Path::new(&location.path).canonicalize().ok() != game_root
            && !paths::has_placeholder(Path::new(&location.path))
            && location.readable
    });
    // Keep the AI choice on ties, but prefer a more recently written save directory.
    report.locations.sort_by_key(|location| !location.path.eq_ignore_ascii_case(&selected));
    report.locations.sort_by_cached_key(|location| std::cmp::Reverse(latest_modified(Path::new(&location.path))));
    report.locations.truncate(1);
    if let Some(location) = report.locations.first_mut() {
        location.source = "ai".into();
        location.confidence = "confirmed".into();
    }
}

fn save_files(folder: &Path) -> bool {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .take(2048)
        .filter_map(Result::ok)
        .any(|e| save_file(&e))
}
fn save_file(e: &fs::DirEntry) -> bool {
    let Ok(metadata) = e.metadata() else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    let name = e.file_name().to_string_lossy().to_lowercase();
    metadata.is_file()
        && !metadata.is_symlink()
        && (name == "persistent"
            || name.starts_with("save")
            || loose::nson_save(&name)
            || [
                ".dat",
                ".sav",
                ".save",
                ".ksd",
                ".asd",
                ".rpgsave",
                ".rmmzsave",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
            || (name.ends_with(".bmp") && classify(&e.path()) == SaveKind::Progress))
        && fs::File::open(e.path()).is_ok()
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
            files: Vec::new(),
            exists: Path::new(&path).is_dir(),
            // 已由 AI 或用户确定的目录只检查可访问性，不能再用格式规则否定选择。
            readable: if matches!(source, "ai" | "manual") {
                fs::read_dir(&path).is_ok()
            } else {
                save_files(Path::new(&path))
            },
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
    if let Some(edit) = edit.filter(|edit| !edit.save_files.is_empty()) {
        let mut report = loose::selected(&edit.save_files, "manual", "confirmed");
        for path in &edit.save_paths {
            add(&mut report, PathBuf::from(path), "manual", "confirmed");
        }
        return report;
    }
    if let Some(edit) = edit.filter(|edit| edit.save_path_auto)
        && let Some(path) = edit.save_path.as_deref()
    {
        let mut report = discover_in(game, None, roots);
        select_recent_ai(&mut report, Path::new(path), game);
        return report;
    }
    let mut report = SaveReport::default();
    if let Some(path) = edit
        .and_then(|e| e.save_path.as_ref())
        .filter(|p| !p.trim().is_empty())
    {
        add(&mut report, PathBuf::from(path), "manual", "confirmed");
        return report;
    }
    if let Some(edit) = edit {
        for path in &edit.save_paths {
            add(&mut report, PathBuf::from(path), "manual", "confirmed");
        }
        // Older configurations stored a single chosen folder in save_paths.
        if !edit.save_paths.is_empty() {
            return report;
        }
    }
    if game.executable.is_none() {
        return report;
    }
    let folder = Path::new(&game.folder);
    loose::discover(game, &mut report);
    steam::discover(folder, roots, &mut report);
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
        game_key(&game.title),
        game_key(
            folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
        ),
    ]
    .into();
    if let Some(exe) = &game.executable {
        let value = game_key(
            Path::new(exe)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
        );
        if !["advhd", "advhdnew", "game", "launcher", "siglusengine"].contains(&value.as_str()) {
            aliases.insert(value);
        }
    }
    aliases.extend(game.aliases.iter().map(|alias| game_key(alias)));
    aliases.retain(|s| s.chars().count() >= 4);
    search_local_game_folders(folder, &aliases, 0, false, &mut 0, &mut report);
    for value in configured {
        let Some(resolved) = paths::expand(&value, |variable| match variable {
            "CURRENT" => Some(folder.to_path_buf()),
            "LOCALAPPDATA" => roots.iter().find(|(source, _)| source == "local").map(|(_, path)| path.clone()),
            "APPDATA" => roots
                .iter()
                .find(|(source, _)| source == "roaming")
                .map(|(_, path)| path.clone()),
            _ => None,
        }) else {
            continue;
        };
        let normalized = resolved.to_string_lossy().replace('\\', "/");
        if let Some(base) = &configured_base {
            for (source, root) in roots {
                let token = match source.as_str() {
                    "roaming" => "<ROAMINGAPPDATA>",
                    "documents" => "<PERSONAL>",
                    "saved-games" => "<SAVEDGAMES>",
                    _ => continue,
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
        if Path::new(&normalized).is_absolute() {
            add(
                &mut report,
                PathBuf::from(normalized),
                "config",
                "configured",
            );
        } else if !normalized.contains("..") && !normalized.contains('$') {
            if let Some(first) = normalized.split('/').next() {
                aliases.insert(game_key(first));
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
    for (source, root) in roots {
        if matches!(source.as_str(), "steam" | "steam-userdata") {
            continue;
        }
        let mut visited = 0;
        report.searched.push(display_path(root));
        search(root, source, &aliases, 0, false, &mut visited, &mut report);
    }
    for location in &mut report.locations {
        if location.source == "config" && location.exists && location.readable {
            location.confidence = "confirmed".into();
        }
        if location.source == "game"
            && location.files.is_empty()
            && location.readable
            && matches!(
                game.engine.as_str(),
                "AdvHD" | "KiriKiri" | "Entis GLS" | "Ren’Py" | "RPG Maker"
            )
        {
            location.confidence = "confirmed".into();
        }
    }
    sort_recent(&mut report);
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
    if depth > 4 || paths::has_placeholder(path) {
        return;
    }
    if *visited >= 5000 {
        report.limited = true;
        return;
    }
    *visited += 1;
    let name = game_key(&path.file_name().unwrap_or_default().to_string_lossy());
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
            if [
                "node_modules",
                "cache",
                "logs",
                "crashdumps",
                ".git",
                ".galgear-backups",
                "patch",
            ]
            .contains(&lower.as_str())
            {
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

fn search_local_game_folders(
    path: &Path,
    aliases: &BTreeSet<String>,
    depth: usize,
    matched: bool,
    visited: &mut usize,
    report: &mut SaveReport,
) {
    if depth >= 2 || *visited >= 256 {
        return;
    }
    for entry in fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        if *visited >= 256 {
            report.limited = true;
            break;
        }
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            continue;
        }
        *visited += 1;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if paths::has_placeholder(&entry.path())
            || name.starts_with(".galgear-")
            || ["cache", "logs", ".git", "node_modules", "patch"].contains(&name.as_str())
        {
            continue;
        }
        let normalized = game_key(&name);
        let matches = matched
            || save_name(&name)
            || aliases.iter().any(|alias| {
                normalized == *alias || (alias.chars().count() >= 8 && normalized.contains(alias))
            });
        if matches && local_save_evidence(&entry.path()) {
            add(report, entry.path(), "game", "detected");
        } else if matches || depth == 0 {
            search_local_game_folders(&entry.path(), aliases, depth + 1, matches, visited, report);
        }
    }
}

fn local_save_evidence(folder: &Path) -> bool {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(128)
        .any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && (entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "sav" | "save" | "rpgsave" | "rmmzsave"
                        ) || extension.to_ascii_lowercase().ends_with("save-willplus")
                    })
                    || kirikiri::recognizable(&entry.path()))
        })
}

#[cfg(test)]
mod discovery_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_save_files_win_over_bundled_saves_and_directory_copy_times() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("savedata");
        let chinese = temp.path().join("save_chs");
        let patch = temp.path().join("patch/Patch/save_chs");
        let modified = |path: &Path, seconds| {
            fs::File::options().write(true).open(path).unwrap()
                .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))).unwrap();
        };
        for (folder, time) in [(&old, 1000), (&chinese, 2000), (&patch, 9000)] {
            fs::create_dir_all(folder).unwrap();
            fs::write(folder.join("slot.sav"), b"save").unwrap();
            modified(&folder.join("slot.sav"), time);
        }
        let game = crate::catalog::manual_game(&temp.path().join("SiglusEngine_cn.exe"), "Stella".into()).unwrap();
        let mut report = discover_in(&game, None, &[]);
        assert_eq!(report.locations.len(), 2);
        assert_eq!(report.locations[0].path, display_path(&chinese));
        fs::write(temp.path().join("Gameexe.dat"), b"game resource").unwrap();
        add(&mut report, temp.path().to_path_buf(), "documents", "matched");
        select_recent_ai(&mut report, &old, &game);
        assert_eq!(report.locations[0].path, display_path(&chinese));
        let mut automatic = GameEdit { save_path: Some(display_path(&old)), save_path_auto: true, ..Default::default() };
        let report = discover_in(&game, Some(&automatic), &[]);
        assert_eq!(report.locations.len(), 1);
        assert_eq!(report.locations[0].path, display_path(&chinese));
        assert_eq!(report.locations[0].confidence, "confirmed");
        // Overwriting an existing slot need not change the directory's timestamp.
        modified(&old.join("slot.sav"), 3000);
        assert_eq!(discover_in(&game, Some(&automatic), &[]).locations[0].path, display_path(&old));
        automatic.save_path_auto = false;
        automatic.save_path = Some(display_path(&chinese));
        assert_eq!(discover_in(&game, Some(&automatic), &[]).locations[0].path, display_path(&chinese));
    }
    #[test]
    fn local_normalized_title_directory_is_discovered_without_claiming_archive_assets() {
        let root = tempfile::tempdir().unwrap();
        let game_folder = root.path().join("Silver×Garden -After Story-");
        let save = game_folder.join("silver_garden_after_story");
        let assets = game_folder.join("assets/silver_garden_after_story");
        fs::create_dir_all(&save).unwrap();
        fs::create_dir_all(&assets).unwrap();
        fs::write(save.join("slot.ksd"), br#"(const) %["currentStorage" => "menu.ks", "mainConductor" => (const) %["pos" => 7]]"#).unwrap();
        fs::write(assets.join("texture.dat"), b"resource bytes").unwrap();
        let game = crate::catalog::manual_game(
            &game_folder.join("SilverGarden.exe"),
            "Localized garden title".into(),
        )
        .unwrap();
        let report = discover_in(&game, None, &[]);
        assert_eq!(report.locations.len(), 1);
        assert_eq!(report.locations[0].path, display_path(&save));
    }
    #[test]
    fn documents_search_excludes_backups_inside_game_folder() {
        let t = tempfile::tempdir().unwrap();
        let documents = t.path().join("Documents");
        let game_dir = documents.join("Test Game");
        let saves = game_dir.join("savedata");
        let backup = game_dir.join(".galgear-backups/saves/game-id/snapshot-id/0");
        fs::create_dir_all(&saves).unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(saves.join("slot.sav"), b"current save").unwrap();
        fs::write(backup.join("slot.sav"), b"old save").unwrap();
        let game =
            crate::catalog::manual_game(&game_dir.join("game.exe"), "Test Game".into()).unwrap();

        let report = discover_in(&game, None, &[("documents".into(), documents)]);

        assert_eq!(
            report
                .locations
                .iter()
                .map(|location| &location.path)
                .collect::<Vec<_>>(),
            vec![&display_path(&saves)]
        );
    }

    #[test]
    fn saved_games_matches_normalized_name_and_custom_save_suffix() {
        let t = tempfile::tempdir().unwrap();
        let game_dir = t.path().join("Nightmare×Sisters");
        fs::create_dir(&game_dir).unwrap();
        let saved = t.path().join("Saved Games");
        let target = saved.join("WillPlus").join("Nightmare_sisters");
        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join("Save001.Nightmare_sistersSave-WillPlus"),
            b"save",
        )
        .unwrap();
        let game =
            crate::catalog::manual_game(&game_dir.join("AdvHD.exe"), "Nightmare×Sisters".into())
                .unwrap();
        let report = discover_in(&game, None, &[("saved-games".into(), saved)]);
        assert_eq!(report.locations.len(), 1);
        assert_eq!(report.locations[0].path, display_path(&target));
        assert!(report.locations[0].readable);
    }

    #[test]
    fn selected_location_excludes_other_candidates_even_when_missing() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("savedata")).unwrap();
        let game =
            crate::catalog::manual_game(&t.path().join("game.exe"), "Test Game".into()).unwrap();
        let chosen = display_path(&t.path().join("chosen"));
        let edit = GameEdit {
            save_path: Some(chosen.clone()),
            save_paths: vec![display_path(&t.path().join("old"))],
            ..Default::default()
        };
        let report = discover_in(&game, Some(&edit), &[]);
        assert_eq!(report.locations.len(), 1);
        assert_eq!(report.locations[0].path, chosen);
        assert_eq!(report.locations[0].confidence, "confirmed");
        assert!(!report.locations[0].exists);
    }

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
