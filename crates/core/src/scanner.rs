use crate::config::display_path;
use crate::{Game, ScanResult, Settings};
use base64::Engine;
use galgear_i18n::Message;
use image::ImageReader;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

const MAX_DEPTH: usize = 12;
const MAX_FOLDERS: usize = 30_000;

pub fn scan(settings: &Settings) -> ScanResult {
    scan_with(settings, &AtomicBool::new(false), &mut |_| {})
}

pub fn scan_with(
    settings: &Settings,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&ScanResult),
) -> ScanResult {
    scan_incremental(settings, &ScanResult::default(), cancel, progress)
}
pub fn scan_incremental(
    settings: &Settings,
    previous: &ScanResult,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&ScanResult),
) -> ScanResult {
    let cache: BTreeMap<_, _> = previous
        .games
        .iter()
        .map(|g| (g.folder.to_lowercase(), g))
        .collect();
    let mut result = ScanResult::default();
    let mut seen = HashSet::new();
    for root in &settings.library_folders {
        let root = Path::new(root);
        if !root.is_dir() {
            result
                .warnings
                .push(Message::new("warnings.library").arg("path", root.display()));
            continue;
        }
        visit(
            root,
            root,
            None,
            0,
            settings,
            &mut seen,
            &mut result,
            cancel,
            progress,
            &cache,
        );
    }
    // Nested executable folders belonging to one game share its identity.
    let mut grouped: BTreeMap<String, Game> = BTreeMap::new();
    for game in result.games.drain(..) {
        match grouped.get(&game.id) {
            Some(previous) if previous.executable.is_some() => {}
            _ => {
                grouped.insert(game.id.clone(), game);
            }
        }
    }
    result.games = grouped.into_values().collect();
    result
        .games
        .sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    result
}

#[allow(clippy::too_many_arguments)] // A bounded recursive visitor sharing scan state.
fn visit(
    root: &Path,
    dir: &Path,
    unit: Option<&Path>,
    depth: usize,
    settings: &Settings,
    seen: &mut HashSet<PathBuf>,
    result: &mut ScanResult,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&ScanResult),
    cache: &BTreeMap<String, &Game>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    if depth > MAX_DEPTH || result.folders_scanned >= MAX_FOLDERS {
        if result.folders_scanned == MAX_FOLDERS {
            result.warnings.push(Message::new("warnings.limit"));
            result.folders_scanned += 1;
        }
        return;
    }
    let canonical = match fs::canonicalize(dir) {
        Ok(path) => path,
        Err(e) => {
            result.warnings.push(
                Message::new("warnings.access")
                    .arg("path", dir.display())
                    .arg("detail", e),
            );
            return;
        }
    };
    if !seen.insert(canonical.clone()) {
        return;
    }
    result.folders_scanned += 1;
    if result.folders_scanned.is_multiple_of(100) {
        progress(result);
    }
    let reader = match fs::read_dir(dir) {
        Ok(reader) => reader,
        Err(e) => {
            result.warnings.push(
                Message::new("warnings.scan")
                    .arg("path", dir.display())
                    .arg("detail", e),
            );
            return;
        }
    };
    let mut files = Vec::new();
    let mut children = Vec::new();
    for entry in reader {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_file() {
            files.push(entry.path());
        } else if kind.is_dir() && !skip_folder(&entry.file_name().to_string_lossy()) {
            children.push(entry.path());
        }
    }
    files.sort();
    children.sort();
    let executable = choose_executable(&files);
    let assets = files.iter().any(|p| {
        matches!(
            extension(p).as_str(),
            "noa" | "xp3" | "arc" | "pfs" | "rpa" | "pak" | "wolf" | "rgss3a" | "dat"
        )
    }) || children.iter().any(|p| {
        matches!(
            file_name(p).to_lowercase().as_str(),
            "www" | "renpy" | "gamedata"
        )
    }) || files
        .iter()
        .any(|p| matches!(extension(p).as_str(), "ks" | "nsa"));
    let single_exe = executable.as_ref().is_some_and(|p| !generic_executable(p));
    if executable.is_some() && (assets || single_exe) {
        let folder = unit.unwrap_or(dir);
        let identity = fs::canonicalize(folder).unwrap_or_else(|_| folder.to_owned());
        let id = format!(
            "{:x}",
            Sha256::digest(display_path(&identity).to_lowercase().as_bytes())
        );
        let title = clean_title(&file_name(folder));
        let mut stamp = Sha256::new();
        let override_path = settings.cover_overrides.get(&id).map(PathBuf::from);
        let cover_paths: Vec<_> = [find_cover(dir), find_cover(folder)]
            .into_iter()
            .flatten()
            .collect();
        for path in files
            .iter()
            .chain(override_path.iter())
            .chain(cover_paths.iter())
        {
            stamp.update(path.to_string_lossy().as_bytes());
            if let Ok(meta) = fs::metadata(path) {
                stamp.update(meta.len().to_le_bytes());
                stamp.update(
                    meta.modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |t| t.as_nanos())
                        .to_le_bytes(),
                );
            }
        }
        let scan_stamp = format!("{:x}", stamp.finalize());
        if let Some(game) = cache.get(&display_path(dir).to_lowercase())
            && game.scan_stamp == scan_stamp
        {
            result.games.push((*game).clone());
            progress(result);
            if executable.is_some() {
                return;
            }
        } else {
            let cover = settings
                .cover_overrides
                .get(&id)
                .and_then(|path| poster_data(Path::new(path)).ok())
                .or_else(|| find_cover(dir).and_then(|p| poster_data(&p).ok()))
                .or_else(|| {
                    if folder != dir {
                        find_cover(folder).and_then(|p| poster_data(&p).ok())
                    } else {
                        None
                    }
                });
            result.games.push(Game {
                id,
                title,
                folder: display_path(dir),
                library: display_path(root),
                executable: executable.as_ref().map(|p| display_path(p)),
                engine: detect_engine(&files, &children),
                play_status: Default::default(),
                cover,
                cover_artwork: None,
                modified: fs::metadata(dir)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |t| t.as_secs()),
                playtime_seconds: 0,
                last_played: None,
                cg_unlock_status: "unsupported".into(),
                auto_unlock: Default::default(),
                tags: Vec::new(),
                available: true,
                metadata: None,
                scan_stamp,
                metadata_warning: None,
            });
            progress(result);
        }
        if executable.is_some() {
            return;
        }
    }
    let container = is_collection(&file_name(dir)) || looks_like_container(&children);
    for child in children {
        let child_unit = if depth == 0 || container {
            if is_collection(&file_name(&child)) {
                None
            } else {
                Some(child.as_path())
            }
        } else {
            unit
        };
        visit(
            root,
            &child,
            child_unit,
            depth + 1,
            settings,
            seen,
            result,
            cancel,
            progress,
            cache,
        );
    }
}

fn looks_like_container(children: &[PathBuf]) -> bool {
    // A publisher/group folder with several independent game directories.
    // Nested installation directories inside one game are not a group.
    children
        .iter()
        .filter(|path| {
            let name = file_name(path).to_lowercase();
            ![
                "iso+",
                "mdf+",
                "mds",
                "パッケージ",
                "光盘",
                "安装",
                "disc",
                "image",
                "data",
                "voice",
                "sound",
                "video",
                "font",
                "movie",
                "www",
                "renpy",
                "gamedata",
                "bg",
            ]
            .iter()
            .any(|word| name.contains(word))
        })
        .take(2)
        .count()
        >= 2
}

fn is_collection(name: &str) -> bool {
    let name = name.to_lowercase();
    ["合集", "collection", "episode", "bundle", "游戏库"]
        .iter()
        .any(|s| name.contains(s))
}

fn skip_folder(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "savedata"
            | "save"
            | "saves"
            | "runtime"
            | "redist"
            | "_commonredist"
            | "manual"
            | "node_modules"
            | ".git"
            | "image"
    ) || ["补丁", "crack", "patch", "破解", "解压码"]
        .iter()
        .any(|s| lower.contains(s))
        || lower.starts_with('.')
}

fn choose_executable(files: &[PathBuf]) -> Option<PathBuf> {
    let mut choices: Vec<_> = files
        .iter()
        .filter(|p| extension(p) == "exe" && !tool_executable(p))
        .collect();
    choices.sort_by_key(|p| {
        let name = file_name(p).to_lowercase();
        if name == "launcher.exe" {
            0
        } else if name.contains("chs") || name.contains("_cn") || name.contains("中文") {
            1
        } else if name.contains("new") {
            2
        } else {
            3
        }
    });
    choices.first().map(|p| (*p).clone())
}

fn tool_executable(path: &Path) -> bool {
    let name = file_name(path).to_lowercase();
    [
        "unins",
        "uninst",
        "setup",
        "install",
        "protect",
        "patch",
        "update",
        "config",
        "settings",
        "redist",
        "dxsetup",
        "crash",
        "report",
        "converter",
        "noa32",
        "ericomp",
    ]
    .iter()
    .any(|s| name.contains(s))
}

fn generic_executable(path: &Path) -> bool {
    let name = file_name(path).to_lowercase();
    name.starts_with("advhd")
        || matches!(
            name.as_str(),
            "game.exe" | "launcher.exe" | "siglusengine.exe"
        )
}

fn detect_engine(files: &[PathBuf], children: &[PathBuf]) -> String {
    if files.iter().any(|p| extension(p) == "noa") {
        "Entis GLS"
    } else if files.iter().any(|p| extension(p) == "xp3") {
        "KiriKiri"
    } else if files
        .iter()
        .any(|p| file_name(p).to_lowercase().starts_with("advhd"))
    {
        "AdvHD"
    } else if children
        .iter()
        .any(|p| file_name(p).to_lowercase() == "renpy")
    {
        "Ren’Py"
    } else if children
        .iter()
        .any(|p| file_name(p).to_lowercase() == "www")
    {
        "RPG Maker"
    } else {
        "unknown"
    }
    .into()
}

fn find_cover(folder: &Path) -> Option<PathBuf> {
    for name in ["cover", "poster", "folder", "front", "封面", "title"] {
        for extension in ["jpg", "png", "jpeg", "webp", "bmp"] {
            for parent in [
                folder.to_owned(),
                folder.join("image"),
                folder.join("images"),
            ] {
                let path = parent.join(format!("{name}.{extension}"));
                if path.is_file() {
                    return Some(path);
                }
                let upper = parent.join(format!("{name}.{}", extension.to_uppercase()));
                if upper.is_file() {
                    return Some(upper);
                }
            }
        }
    }
    None
}

pub fn poster_data(path: &Path) -> Result<String, Message> {
    let length = fs::metadata(path)
        .map_err(|e| Message::detail("error.image", e))?
        .len();
    if length > 32 * 1024 * 1024 {
        return Err(Message::new("error.image_size"));
    }
    let image = ImageReader::open(path)
        .map_err(|e| Message::detail("error.image", e))?
        .with_guessed_format()
        .map_err(|e| Message::detail("error.image", e))?
        .decode()
        .map_err(|e| Message::detail("error.image", e))?;
    let image = image.thumbnail(540, 720);
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Jpeg)
        .map_err(|e| Message::detail("error.image", e))?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(output.into_inner())
    ))
}

pub fn cache_poster(source: &Path, destination: &Path) -> Result<String, Message> {
    let data = poster_data(source)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim_start_matches("data:image/jpeg;base64,"))
        .map_err(|e| Message::detail("error.image", e))?;
    let parent = destination
        .parent()
        .ok_or_else(|| Message::new("error.cover_path"))?;
    fs::create_dir_all(parent).map_err(|e| Message::detail("error.image", e))?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| Message::detail("error.image", e))?;
    file.write_all(&bytes)
        .map_err(|e| Message::detail("error.image", e))?;
    file.as_file()
        .sync_all()
        .map_err(|e| Message::detail("error.image", e))?;
    file.persist(destination)
        .map_err(|e| Message::detail("error.image", e))?;
    Ok(data)
}

fn clean_title(name: &str) -> String {
    let cut = [
        " パッケージ版",
        " (iso+",
        " (mdf+",
        " [+",
        " Crack",
        " crack",
    ]
    .iter()
    .filter_map(|s| name.find(s))
    .min()
    .unwrap_or(name.len());
    name[..cut].trim().to_owned()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, []).unwrap();
    }

    #[test]
    fn nested_collection_ignores_patch_executables_and_image_only_games() {
        let tmp = tempfile::tempdir().unwrap();
        file(&tmp.path().join("Angel Tear/angeltear.exe"));
        file(&tmp.path().join("Angel Tear/script.noa"));
        file(&tmp.path().join("8Episode/作一/安装目录/AdvHD.exe"));
        file(&tmp.path().join("8Episode/作一/安装目录/data.arc"));
        file(&tmp.path().join("8Episode/作一/光盘/GAME.ISO"));
        file(&tmp.path().join("8Episode/作一/免DVD补丁/AdvHD.exe"));
        file(&tmp.path().join("8Episode/作二/光盘/DISC.mdf"));
        file(&tmp.path().join("8Episode/作二/修正补丁/setup.exe"));
        let settings = Settings {
            library_folders: vec![display_path(tmp.path())],
            ..Default::default()
        };
        let result = scan(&settings);
        assert_eq!(result.games.len(), 2);
        let first = result.games.iter().find(|g| g.title == "作一").unwrap();
        assert!(!first.executable.as_ref().unwrap().contains("补丁"));
        assert!(!result.games.iter().any(|g| g.title == "作二"));
    }

    #[test]
    fn missing_and_overlapping_libraries_do_not_duplicate_games() {
        let tmp = tempfile::tempdir().unwrap();
        file(&tmp.path().join("中文游戏/game.exe"));
        file(&tmp.path().join("中文游戏/data.xp3"));
        let settings = Settings {
            library_folders: vec![
                display_path(tmp.path()),
                display_path(&tmp.path().join("中文游戏")),
                display_path(&tmp.path().join("missing")),
            ],
            ..Default::default()
        };
        let result = scan(&settings);
        assert_eq!(result.games.len(), 1);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn unnamed_publisher_container_keeps_independent_games() {
        let tmp = tempfile::tempdir().unwrap();
        file(&tmp.path().join("发行商/游戏一/game.exe"));
        file(&tmp.path().join("发行商/游戏一/data.xp3"));
        file(&tmp.path().join("发行商/游戏二/game.exe"));
        file(&tmp.path().join("发行商/游戏二/data.xp3"));
        let settings = Settings {
            library_folders: vec![display_path(tmp.path())],
            ..Default::default()
        };
        let result = scan(&settings);
        assert_eq!(result.games.len(), 2);
        assert!(result.games.iter().any(|game| game.title == "游戏一"));
        assert!(result.games.iter().any(|game| game.title == "游戏二"));
    }

    #[test]
    fn cached_cover_survives_the_source_being_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("original.png");
        let cache = tmp.path().join("covers/game.jpg");
        image::RgbImage::from_pixel(24, 32, image::Rgb([220, 100, 160]))
            .save(&source)
            .unwrap();
        let original = cache_poster(&source, &cache).unwrap();
        fs::rename(&source, tmp.path().join("moved.png")).unwrap();
        assert!(cache.is_file());
        assert!(original.starts_with("data:image/jpeg;base64,"));
        assert!(poster_data(&cache).is_ok());
    }
}
