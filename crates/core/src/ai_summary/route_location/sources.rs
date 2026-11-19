//! 只从生成当前图的分析工作目录读取结构化脚本索引。
use super::{ambiguous, unmatched};
use crate::{
    ai_summary::{invalid, io, read},
    saves,
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

#[derive(Deserialize)]
struct Index {
    records: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    input: PathBuf,
    output: Option<PathBuf>,
    sha256: Option<String>,
    reader: Option<String>,
}
fn checked_file(root: &Path, relative: &Path) -> Result<PathBuf, Message> {
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(invalid());
    }
    let path = root.join(relative);
    if !path.try_exists().map_err(io)? {
        return Err(unmatched());
    }
    saves::validate_file(&path)?;
    Ok(path)
}

pub(super) fn scripts(
    base: &Path,
    result: &Value,
    extension: &str,
    reader: &str,
    wanted: Option<&str>,
) -> Result<BTreeMap<String, (PathBuf, String)>, Message> {
    let inputs = result["inputs"].as_object().ok_or_else(unmatched)?;
    let workdirs: BTreeSet<_> = inputs
        .keys()
        .filter_map(|s| Path::new(s).strip_prefix(base).ok())
        .filter_map(|p| p.components().next())
        .filter_map(|c| match c {
            Component::Normal(n) if n.to_str()?.starts_with(".skill-analysis-") => {
                Some(base.join(n))
            }
            _ => None,
        })
        .collect();
    let work = super::unique(workdirs.into_iter())?;
    let mut files = BTreeMap::new();
    let indexes = result["scope"]["script_indexes"]
        .as_array()
        .or_else(|| result["reader_indexes"].as_array())
        .ok_or_else(unmatched)?;
    if indexes.len() > 256 {
        return Err(invalid());
    }
    for item in indexes {
        let relative = Path::new(item.as_str().ok_or_else(invalid)?);
        if !relative.starts_with("reader-results")
            || relative.file_name().is_none_or(|n| n != "index.json")
        {
            return Err(invalid());
        }
        let index: Index =
            serde_json::from_slice(&read(&checked_file(&work, relative)?, 4 * 1024 * 1024)?)
                .map_err(io)?;
        for entry in index.records {
            let Some(name) = entry.input.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(extension)
                || wanted.is_some_and(|w| w != name)
                || entry.reader.as_deref() != Some(reader)
            {
                continue;
            }
            let (Some(output), Some(hash)) = (entry.output, entry.sha256) else {
                continue;
            };
            if output.parent() != relative.parent()
                || !inputs.contains_key(entry.input.to_str().ok_or_else(invalid)?)
            {
                return Err(invalid());
            }
            let path = checked_file(&work, &output)?;
            if let Some((_, previous)) = files.insert(name.to_owned(), (path, hash.clone()))
                && previous != hash
            {
                return Err(ambiguous());
            }
        }
    }
    // 文本回退要覆盖已分析的所有同类来源；缺页不能把重复句误报成唯一匹配。
    for input in inputs
        .keys()
        .map(Path::new)
        .filter_map(Path::file_name)
        .filter_map(|n| n.to_str())
    {
        if input.ends_with(extension)
            && wanted.is_none_or(|w| w == input)
            && !files.contains_key(input)
        {
            return Err(unmatched());
        }
    }
    Ok(files)
}
