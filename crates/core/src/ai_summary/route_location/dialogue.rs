//! 在本次路线分析保存的 PSB 台词表中查找活动书签。原生 point 与台词序号并非同一坐标。
use super::{
    ambiguous,
    anchor::{Anchor, Coordinate},
    unmatched,
};
use crate::{
    ai_summary::{invalid, io, read},
    saves::Bookmark,
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

#[derive(Deserialize)]
struct Projection {
    source_sha256: String,
    scene_dialogue: Vec<Scene>,
}
#[derive(Deserialize)]
struct Scene {
    scene_index: usize,
    label: Option<String>,
    records: Vec<Row>,
}
#[derive(Deserialize)]
struct Row {
    record_index: usize,
    variants: Vec<Variant>,
}
#[derive(Deserialize)]
struct Variant {
    text: String,
}

impl Row {
    fn matches(&self, caption: &str) -> bool {
        !caption.is_empty() && self.variants.iter().any(|v| v.text.trim() == caption)
    }
}

pub(super) fn find(base: &Path, result: &Value, bookmark: &Bookmark) -> Result<Anchor, Message> {
    let (storage, active_label) = bookmark
        .scene
        .as_deref()
        .unwrap_or_default()
        .split_once('*')
        .unwrap_or((bookmark.scene.as_deref().unwrap_or_default(), ""));
    let file = (!storage.is_empty()).then(|| format!("{storage}.scn"));
    let mut best = None;
    let mut tied = false;
    let mut best_score = 0;
    let mut budget = 256 * 1024 * 1024;
    for (name, (path, digest)) in
        super::sources::scripts(base, result, ".ks.scn", "structured", file.as_deref())?
    {
        let bytes = read(&path, budget.min(16 * 1024 * 1024))?;
        budget -= bytes.len() as u64;
        let projection: Projection = serde_json::from_slice(&bytes).map_err(io)?;
        if projection.source_sha256 != digest {
            return Err(invalid());
        }
        for scene in projection.scene_dialogue {
            let Some(label) = scene.label else { continue };
            if !active_label.is_empty() && label.strip_prefix('*') != Some(active_label) {
                continue;
            }
            for (i, row) in scene
                .records
                .iter()
                .enumerate()
                .filter(|(_, row)| row.matches(&bookmark.caption))
            {
                let score = scene.records[..i]
                    .iter()
                    .rev()
                    .zip(&bookmark.preceding)
                    .take_while(|(row, caption)| row.matches(caption))
                    .count();
                if best.is_none() || score > best_score {
                    best = Some(Anchor {
                        script: name.clone(),
                        scene: label.clone(),
                        coordinate: Coordinate::Psb(scene.scene_index),
                        index: row.record_index,
                    });
                    best_score = score;
                    tied = false;
                } else if score == best_score {
                    tied = true;
                }
            }
        }
    }
    if tied {
        Err(ambiguous())
    } else {
        best.ok_or_else(unmatched)
    }
}
