//! 混合目录中的存档集合：组织已选文件，本地自动发现仅使用明确名称或原生格式证据。
use super::{SaveLocation, SaveReport, kirikiri};
use crate::{Game, config::display_path};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(super) fn selected(files: &[String], source: &str, confidence: &str) -> SaveReport {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for file in files {
        if let Some(parent) = Path::new(file).parent() {
            groups
                .entry(display_path(parent))
                .or_default()
                .push(file.clone());
        }
    }
    let locations = groups
        .into_iter()
        .map(|(path, mut files)| {
            files.sort_by_key(|path| path.to_lowercase());
            files.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            SaveLocation {
                exists: files.iter().all(|file| Path::new(file).is_file()),
                readable: files.iter().all(|file| fs::File::open(file).is_ok()),
                path,
                files,
                source: source.into(),
                confidence: confidence.into(),
            }
        })
        .collect();
    SaveReport {
        locations,
        ..Default::default()
    }
}

fn slot(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(suffix))
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()))
}

pub(super) fn nson_save(name: &str) -> bool {
    name == "globalsave.nson" || slot(name, "gamesave", ".nson")
}

pub(super) fn discover(game: &Game, report: &mut SaveReport) {
    let folder = PathBuf::from(&game.folder);
    let mut roots = vec![folder.clone()];
    if let Some(parent) = game
        .executable
        .as_deref()
        .and_then(|exe| Path::new(exe).parent())
        && parent != folder
        && parent.starts_with(&folder)
    {
        roots.push(parent.to_path_buf());
    }
    // Windows 老程序可能在安装路径的 VirtualStore 镜像中保存。
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for root in roots.clone() {
            let relative: PathBuf = root
                .components()
                .filter_map(|component| {
                    if let std::path::Component::Normal(name) = component {
                        Some(name)
                    } else {
                        None
                    }
                })
                .collect();
            roots.push(PathBuf::from(&local).join("VirtualStore").join(relative));
        }
    }
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        let entries: Vec<_> = entries.take(2048).filter_map(Result::ok).collect();
        let names: Vec<_> = entries
            .iter()
            .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
            .collect();
        let ons = names.iter().any(|name| slot(name, "save", ".dat"))
            && names.iter().any(|name| {
                matches!(
                    name.as_str(),
                    "nscript.dat" | "0.txt" | "nscrflog.dat" | "gloval.sav"
                )
            });
        let files = entries
            .iter()
            .filter(|entry| {
                let path = entry.path();
                if super::validate_file(&path).is_err() {
                    return false;
                }
                let name = entry.file_name().to_string_lossy().to_lowercase();
                matches!(name.as_str(), "save.sav" | "read.sav" | "global.sav")
                    || slot(&name, "save", ".sav")
                    || (ons
                        && (slot(&name, "save", ".dat")
                            || matches!(
                                name.as_str(),
                                "gloval.sav"
                                    | "envdata"
                                    | "kidoku.dat"
                                    | "nscrflog.dat"
                                    | "nscrllog.dat"
                            )))
                    || (matches!(
                        path.extension().and_then(|ext| ext.to_str()),
                        Some("ksd" | "asd" | "bmp")
                    ) && kirikiri::recognizable(&path))
            })
            .map(|entry| display_path(&entry.path()))
            .collect::<Vec<_>>();
        report
            .locations
            .extend(selected(&files, "game", "detected").locations);
    }
}
