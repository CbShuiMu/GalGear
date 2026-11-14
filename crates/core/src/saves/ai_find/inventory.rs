//! 为 AI 收集有界目录元数据；先搜索再按游戏身份裁剪，避免无关目录挤掉用户存档。
use super::{super::paths, validated_location};
use crate::{Game, config::display_path, saves::SaveReport};
use serde_json::{Value, json};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs,
    path::{Path, PathBuf},
};

const MAX_ENTRIES_PER_ROOT: usize = 20_000;
const MAX_ENTRIES_PER_FOLDER: usize = 4096;
const MAX_DIRECTORIES_PER_ROOT: usize = 6000;
const MAX_CONTEXT_DIRECTORIES: usize = 400;
const MAX_DEPTH: usize = 4;
const CONTEXT_BYTES: usize = 96 * 1024;

pub(super) fn context(game: &Game, roots: &[PathBuf], report: &SaveReport) -> String {
    let identity = Identity::new(game);
    let mut directories = Vec::new();
    let mut limited = report.limited;
    // 每个根目录独享预算，安装包大小不能影响 AppData 的搜索和输出。
    for root in roots {
        let (found, truncated) = directory_inventory(root, &identity);
        limited |= truncated;
        let mut remaining = CONTEXT_BYTES / roots.len().max(1);
        let mut count = 0;
        for (_, _, path) in found {
            let entry = json!({
                "path": display_path(&path),
                "last_modified_unix": modified_unix(&path),
            });
            let bytes = entry.to_string().len();
            if count >= MAX_CONTEXT_DIRECTORIES || bytes > remaining {
                limited = true;
                continue;
            }
            remaining -= bytes;
            count += 1;
            directories.push(entry);
        }
    }
    let candidates = report
        .locations
        .iter()
        .filter(|location| {
            location.exists
                && crate::saves::location_roots(location)
                    .iter()
                    .all(|path| validated_location(path, roots).is_ok())
        })
        .enumerate()
        .map(|(index, location)| {
            json!({
                "index": index,
                "path": location.path,
                "files": location.files,
                "source": location.source,
                "confidence": location.confidence,
                "last_modified_unix": modified_unix(Path::new(&location.path)),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "installation_files": installation_files(game, roots),
        "save_candidates": candidates,
        "discovery_limited": limited,
        "game_title": game.title,
        "game_aliases": game.aliases,
        "metadata_title": game.metadata.as_ref().map(|metadata| &metadata.title),
        "brand": game.metadata.as_ref().map(|metadata| &metadata.brand),
        "developer": game.metadata.as_ref().map(|metadata| &metadata.developer),
        "engine": game.engine,
        "folder_name": Path::new(&game.folder).file_name().map(|name| name.to_string_lossy()),
        "executable_filename": game.executable.as_deref().and_then(|path| Path::new(path).file_name()).map(|name| name.to_string_lossy()),
        "game_directory": game.folder,
        "search_roots": roots.iter().map(|path| display_path(path)).collect::<Vec<_>>(),
        "existing_directories": directories,
    })
    .to_string()
}

struct Identity {
    titles: Vec<String>,
    vendors: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Relevance {
    Game,
    Vendor,
    Other,
}

impl Identity {
    fn new(game: &Game) -> Self {
        let mut titles: Vec<_> = std::iter::once(&game.title)
            .chain(&game.aliases)
            .map(|title| crate::saves::game_key(title))
            .collect();
        for name in [
            Path::new(&game.folder).file_name(),
            game.executable
                .as_deref()
                .and_then(|exe| Path::new(exe).file_stem()),
        ]
        .into_iter()
        .flatten()
        {
            titles.push(crate::saves::game_key(&name.to_string_lossy()));
        }
        let mut vendors = Vec::new();
        if let Some(metadata) = &game.metadata {
            titles.push(crate::saves::game_key(&metadata.title));
            vendors.extend(
                [&metadata.brand, &metadata.developer].map(|name| crate::saves::game_key(name)),
            );
        }
        titles.retain(|value| {
            value.chars().count() >= 3
                && ![
                    "game",
                    "launcher",
                    "advhd",
                    "advhdnew",
                    "siglusengine",
                    "bootmenu",
                ]
                .contains(&value.as_str())
        });
        vendors.retain(|value| value.chars().count() >= 3);
        Self { titles, vendors }
    }

    fn priority(&self, relative: &Path) -> Relevance {
        let matches = |names: &[String]| {
            relative.components().any(|component| {
                let name = crate::saves::game_key(&component.as_os_str().to_string_lossy());
                names.iter().any(|alias| {
                    name == *alias || (alias.chars().count() >= 4 && name.contains(alias))
                })
            })
        };
        // 包含匹配仅决定清单优先级；带汉化组前缀的短标题也应送给 AI 判断版本。
        if matches(&self.titles) {
            Relevance::Game
        } else if matches(&self.vendors) {
            Relevance::Vendor
        } else {
            Relevance::Other
        }
    }
}

fn directory_inventory(
    root: &Path,
    identity: &Identity,
) -> (Vec<(Relevance, usize, PathBuf)>, bool) {
    let mut found = Vec::new();
    let mut limited = false;
    if crate::saves::validate_components(root).is_err() {
        return (found, true);
    }
    let mut queue = BinaryHeap::from([Reverse((Relevance::Other, 0, root.to_path_buf()))]);
    let mut visited = 0;
    while let Some(Reverse((_, depth, folder))) = queue.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            limited = true;
            continue;
        };
        for (index, entry) in entries.enumerate() {
            if visited >= MAX_ENTRIES_PER_ROOT || found.len() >= MAX_DIRECTORIES_PER_ROOT {
                found.sort();
                return (found, true);
            }
            if index >= MAX_ENTRIES_PER_FOLDER {
                limited = true;
                break;
            }
            visited += 1;
            let Ok(entry) = entry else {
                limited = true;
                continue;
            };
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if paths::has_placeholder(&path)
                || paths::excluded_directory(&path)
                || ["cache", "logs", "crashdumps", ".git", "node_modules"].contains(&name.as_str())
            {
                continue;
            }
            if crate::saves::validate_components(&path).is_err() {
                limited = true;
                continue;
            }
            let priority = identity.priority(path.strip_prefix(root).unwrap_or(&path));
            found.push((priority, depth + 1, path.clone()));
            if depth + 1 < MAX_DEPTH {
                queue.push(Reverse((priority, depth + 1, path)));
            } else {
                limited = true;
            }
        }
    }
    found.sort();
    (found, limited)
}

// 目录时间仅用于辅助判断；覆盖已有存档文件不一定改变目录时间。
fn modified_unix(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn installation_files(game: &Game, roots: &[PathBuf]) -> Vec<Value> {
    let mut folders = vec![Path::new(&game.folder)];
    if let Some(parent) = game
        .executable
        .as_deref()
        .and_then(|exe| Path::new(exe).parent())
        && parent.starts_with(&game.folder)
        && !folders.contains(&parent)
    {
        folders.push(parent);
    }
    folders
        .into_iter()
        .flat_map(|folder| {
            fs::read_dir(folder)
                .into_iter()
                .flatten()
                .take(512)
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .filter(|entry| validated_location(&display_path(&entry.path()), roots).is_ok())
                .map(|entry| {
                    json!({
                        "path": display_path(&entry.path()),
                        "filename": entry.file_name().to_string_lossy(),
                        "last_modified_unix": modified_unix(&entry.path()),
                    })
                })
        })
        .collect()
}
