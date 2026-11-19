//! 将最新进度槽的只读快照绑定到已打开的路线图。定位不启动游戏，也不重新生成图。
mod anchor;
mod dialogue;
mod siglus;
mod sources;
#[cfg(test)]
mod tests;

use super::{
    FOLDER, invalid, io, read, resources, route_graph::Graph, save_file_family, skill_analysis,
    slot_shaped_name,
};
use crate::{Game, Settings, saves};
use galgear_i18n::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
/// 路线图中的唯一位置及其存档依据。
pub struct Location {
    /// 当前打开的图中的节点 ID。
    pub node: String,
    /// 供窗口显示的本地化定位依据。
    pub detail: String,
}

fn ambiguous() -> Message {
    Message::new("route.locate_ambiguous")
}
fn unmatched() -> Message {
    Message::new("route.locate_unmatched")
}

fn unique<T>(mut items: impl Iterator<Item = T>) -> Result<T, Message> {
    let first = items.next().ok_or_else(unmatched)?;
    if items.next().is_some() {
        Err(ambiguous())
    } else {
        Ok(first)
    }
}

fn family(path: &Path) -> (String, String) {
    (
        save_file_family(path),
        path.extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase(),
    )
}

fn is_slot(path: &Path, families: &BTreeSet<(String, String)>) -> Result<bool, Message> {
    let name = family(path);
    if families.contains(&name) {
        return Ok(true);
    }
    let related = families.iter().any(|(_, ext)| *ext == name.1);
    if !related && !matches!(name.1.as_str(), "bmp" | "ksd" | "asd") {
        return Ok(false);
    }
    saves::validate_file(path)?;
    Ok(match saves::classify(path) {
        saves::SaveKind::Progress => true,
        saves::SaveKind::Control => false,
        // 新出现的自动/快速槽即使损坏也不能跳过它、退回较旧的存档。
        saves::SaveKind::Unknown => related && slot_shaped_name(path),
    })
}

// 只在本次发现的目录或手动文件集合内找槽位。历史分析提供已辨认的命名格式，
// 不提供待读取的绝对路径。即使新槽损坏或过大，也先按时间选中，再报告读取失败。
fn latest(
    report: &saves::SaveReport,
    families: &BTreeSet<(String, String)>,
) -> Result<resources::Stamp, Message> {
    let mut paths = BTreeSet::new();
    for location in report.locations.iter().filter(|l| l.exists) {
        let root = Path::new(&location.path);
        saves::validate_components(root)?;
        if location.files.is_empty() {
            for entry in fs::read_dir(root).map_err(io)? {
                let path = entry.map_err(io)?.path();
                if !path.is_dir() && is_slot(&path, families)? {
                    paths.insert(path);
                }
            }
        } else {
            for path in location.files.iter().map(PathBuf::from) {
                if is_slot(&path, families)? {
                    paths.insert(path);
                }
            }
        }
    }
    let mut slots = paths
        .iter()
        .map(|path| {
            saves::validate_file(path)?;
            resources::stamp(path)
        })
        .collect::<Result<Vec<_>, Message>>()?;
    let modified = slots
        .iter()
        .map(|s| s.modified)
        .max()
        .ok_or_else(|| Message::new("summary.no_saves"))?;
    slots.retain(|s| s.modified == modified);
    unique(slots.into_iter())
}

/// 重新发现本游戏的进度槽，读取最新槽并在指定版本路线图中查找可达节点。
pub fn locate(game: &Game, settings: &Settings, revision: &str) -> Result<Location, Message> {
    let edit = settings.game_edits.get(&game.id);
    if !edit.is_some_and(|e| e.ai_summary) {
        return Err(Message::new("summary.disabled"));
    }
    let (graph, result, candidates) = skill_analysis::location_inputs(game, settings)?;
    if graph.revision()? != revision {
        return Err(Message::new("route.locate_map_changed"));
    }
    let position = &result["save_position"];
    let analyzed = Path::new(position["file"].as_str().ok_or_else(invalid)?);
    let mut families = BTreeSet::from([family(analyzed)]);
    for candidate in candidates
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["kind"] == "progress")
    {
        if let Some(path) = candidate["path"].as_str() {
            families.insert(family(Path::new(path)));
        }
    }
    let report = saves::discover(game, edit);
    let slot = latest(&report, &families)?;
    let path = Path::new(&slot.path);
    let bytes = read(path, 8 * 1024 * 1024)?;
    let name = path.file_name().ok_or_else(invalid)?.to_string_lossy();
    let base = Path::new(&game.folder).join(FOLDER);
    let found = live_position(&graph, &base, &result, &bytes);
    let (node, message) = match found {
        Ok(found) => found,
        Err(error)
            if (error.key == "route.locate_unsupported"
                || error.key == "route.locate_unmatched")
                && position["sha256"] == format!("{:x}", Sha256::digest(&bytes)) =>
        {
            // 旧分析里的存档节点仅在槽内容逐字节一致时有效。
            let node = graph.cached_story_node(position["node"].as_str())?;
            (node, Message::new("route.locate_cached"))
        }
        Err(error) => return Err(error.arg("file", name)),
    };
    if latest(&saves::discover(game, edit), &families)? != slot {
        return Err(Message::new("summary.save_changed"));
    }
    Ok(Location {
        node: node.into(),
        detail: message.arg("file", name).render(settings.language),
    })
}

fn live_position<'a>(
    graph: &'a Graph,
    base: &Path,
    result: &serde_json::Value,
    bytes: &[u8],
) -> Result<(&'a str, Message), Message> {
    let (anchor, caption) = if let Some(bookmark) = saves::bookmark(bytes) {
        (dialogue::find(base, result, &bookmark)?, bookmark.caption)
    } else if let Some(preview) = siglus::preview(bytes) {
        (siglus::find(base, result, &preview)?, preview.caption)
    } else {
        return Err(Message::new("route.locate_unsupported"));
    };
    let node = unique(
        graph
            .location_nodes(false)
            .filter(|(_, pc, evidence)| anchor.belongs_to(pc, evidence))
            .map(|(id, _, _)| id),
    )?;
    let message = Message::new("route.locate_line")
        .arg("script", anchor.script)
        .arg("scene", anchor.scene)
        .arg("index", anchor.index)
        .arg("text", caption);
    Ok((node, message))
}
