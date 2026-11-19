//! Siglus 本地存档预览含最后一句与同一文本框的完整正文。只在剧情脚本中匹配两者。
use super::{
    ambiguous,
    anchor::{Anchor, Coordinate},
    sources, unmatched,
};
use crate::ai_summary::{invalid, io, read};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

pub(super) struct Preview {
    pub caption: String,
    full: String,
}

pub(super) fn preview(bytes: &[u8]) -> Option<Preview> {
    // 与随附 Siglus 读取器的 local/wide 布局一致，槽位预览不是 VM 的指令指针。
    const WIDTH: usize = 256 * 2;
    const SIZE_FIELD: usize = 40 + 7 * WIDTH + 256 * 4;
    let word = |at| Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    for (i, range) in [
        (1..=1),
        (0..=32),
        (1970..=2100),
        (1..=12),
        (1..=31),
        (0..=6),
        (0..=23),
        (0..=59),
        (0..=59),
        (0..=999),
    ]
    .into_iter()
    .enumerate()
    {
        if !range.contains(&word(i * 4)?) {
            return None;
        }
    }
    let size = usize::try_from(word(SIZE_FIELD)?).ok()?;
    if size < 8 || bytes.len().checked_sub(SIZE_FIELD + 4)? != size {
        return None;
    }
    let string = |i: usize| {
        let start = 40 + i * WIDTH;
        let units: Vec<_> = bytes
            .get(start..start + WIDTH)?
            .chunks_exact(2)
            .map(|w| u16::from_le_bytes([w[0], w[1]]))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16(&units).ok()
    };
    let caption = string(3)?;
    let full = string(4)?;
    (!caption.trim().is_empty() && full.ends_with(&caption)).then_some(Preview { caption, full })
}

#[derive(Deserialize)]
struct Script {
    source_sha256: String,
    dialogue: Vec<Line>,
}
#[derive(Deserialize)]
struct Line {
    text: String,
    scene: String,
}

pub(super) fn find(base: &Path, result: &Value, preview: &Preview) -> Result<Anchor, Message> {
    let scripts = sources::scripts(base, result, ".dat", "siglus_scene", None)?;
    let mut budget = 256 * 1024 * 1024;
    let mut found = None;
    for (name, (path, hash)) in scripts {
        let bytes = read(&path, budget.min(32 * 1024 * 1024))?;
        budget -= bytes.len() as u64;
        let script: Script = serde_json::from_slice(&bytes).map_err(io)?;
        if script.source_sha256 != hash {
            return Err(invalid());
        }
        for (index, line) in script
            .dialogue
            .iter()
            .enumerate()
            .filter(|(_, line)| line.text == preview.caption)
        {
            let mut remaining = preview.full.as_str();
            for prior in script.dialogue[..=index]
                .iter()
                .rev()
                .take_while(|r| r.scene == line.scene)
            {
                let Some(prefix) = remaining.strip_suffix(&prior.text) else {
                    break;
                };
                remaining = prefix;
                if remaining.is_empty() {
                    break;
                }
            }
            if !remaining.is_empty() {
                continue;
            }
            if found.is_some() {
                return Err(ambiguous());
            }
            found = Some(Anchor {
                script: name.clone(),
                scene: line.scene.clone(),
                index,
                coordinate: Coordinate::Siglus {
                    opening: script
                        .dialogue
                        .iter()
                        .find(|r| r.scene == line.scene)
                        .map(|r| r.text.clone())
                        .unwrap_or_default(),
                },
            });
        }
    }
    found.ok_or_else(unmatched)
}
