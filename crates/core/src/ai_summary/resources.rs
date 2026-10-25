use super::{invalid, io, read, write_json};
#[cfg(test)]
use galgear_i18n::Language;
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::BTreeMap;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dialogue {
    pub source: String,
    pub position: u64,
    pub text: String,
    #[serde(default)]
    pub speaker: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stamp {
    pub path: String,
    pub size: u64,
    pub modified: u64,
}

pub fn source_inventory(root: &Path) -> Result<Vec<Stamp>, Message> {
    files(root, 3)?
        .into_iter()
        .filter(|p| {
            script_file(p)
                || matches!(
                    p.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase()
                        .as_str(),
                    "noa" | "xp3" | "arc" | "pfs" | "pak" | "ypf" | "xml"
                )
        })
        .map(|p| stamp(&p))
        .collect()
}

pub fn stamp(path: &Path) -> Result<Stamp, Message> {
    let metadata = fs::metadata(path).map_err(io)?;
    Ok(Stamp {
        path: crate::config::display_path(path),
        size: metadata.len(),
        modified: metadata
            .modified()
            .map_err(io)?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io)?
            .as_nanos()
            .min(u64::MAX as u128) as u64,
    })
}

pub fn files(root: &Path, depth: usize) -> Result<Vec<PathBuf>, Message> {
    let mut output = Vec::new();
    fn visit(root: &Path, depth: usize, output: &mut Vec<PathBuf>) -> Result<(), Message> {
        for entry in fs::read_dir(root).map_err(io)?.take(4096) {
            let entry = entry.map_err(io)?;
            let kind = entry.file_type().map_err(io)?;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "savedata" | "save" | "saves" | "node_modules"
                )
            {
                continue;
            }
            if kind.is_file() {
                output.push(entry.path());
            }
            if output.len() > 20_000 {
                return Err(invalid());
            }
            if kind.is_dir() && depth > 0 {
                visit(&entry.path(), depth - 1, output)?;
            }
        }
        Ok(())
    }
    visit(root, depth, &mut output)?;
    output.sort();
    Ok(output)
}

pub fn tool(home: &Path) -> Result<PathBuf, Message> {
    [
        home.join("skills/GalgameResourceExtract/tools/GARbro-cli/GARbro.Console.exe"),
        home.join("GARbro-cli/GARbro.Console.exe"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or_else(|| Message::new("summary.tools_missing"))
}

pub fn child(mut command: Command, timeout: Duration) -> Result<(), Message> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut process = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io)?;
    let start = Instant::now();
    loop {
        if let Some(status) = process.try_wait().map_err(io)? {
            return if status.success() {
                Ok(())
            } else {
                Err(Message::new("summary.decode_failed"))
            };
        }
        if start.elapsed() > timeout {
            let _ = process.kill();
            let _ = process.wait();
            return Err(Message::new("summary.decode_failed"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize)
}
fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn script_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "csx" | "ks" | "txt" | "json" | "csv" | "rpy" | "mes" | "ws2" | "lng" | "scn" | "s" | "scr"
    )
}

#[cfg(test)]
fn csx(data: &[u8], source: &str) -> Result<Vec<Dialogue>, Message> {
    let mut p = 64;
    let mut table = None;
    let mut image_size = 0;
    while p + 16 <= data.len() {
        let size =
            usize::try_from(u64_at(data, p + 8).ok_or_else(invalid)?).map_err(|_| invalid())?;
        let chunk = data
            .get(p + 16..(p + 16).checked_add(size).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        if &data[p..p + 8] == b"conststr" {
            table = Some(chunk);
        }
        if &data[p..p + 8] == b"image   " {
            image_size = size;
        }
        p += 16 + size;
    }
    let data = table.ok_or_else(invalid)?;
    let count = u32_at(data, 0).ok_or_else(invalid)?;
    let mut p = 4;
    if count > 500_000 {
        return Err(invalid());
    }
    let mut result = Vec::new();
    for _ in 0..count {
        let len = u32_at(data, p).ok_or_else(invalid)?;
        p += 4;
        if len > 32_768 {
            return Err(invalid());
        }
        let end = p.checked_add(len * 2).ok_or_else(invalid)?;
        let raw = data.get(p..end).ok_or_else(invalid)?;
        let text = encoding_rs::UTF_16LE.decode(raw).0.into_owned();
        p = end;
        let refs = u32_at(data, p).ok_or_else(invalid)?;
        p += 4;
        if refs > 1_000_000 {
            return Err(invalid());
        }
        for _ in 0..refs {
            let position = u32_at(data, p).ok_or_else(invalid)?;
            p += 4;
            if position < image_size && is_dialogue(&text) {
                result.push(Dialogue {
                    source: source.into(),
                    position: position as u64,
                    text: text.clone(),
                    speaker: None,
                });
            }
        }
    }
    result.sort_by_key(|r| r.position);
    Ok(result)
}

#[cfg(test)]
fn is_dialogue(text: &str) -> bool {
    text.chars().count() >= 2
        && text
            .chars()
            .any(|c| matches!(c, '。' | '、' | '「' | '！' | '？' | '…' | '.' | '!' | '?'))
        && !text.contains(['\\', '{', '}'])
}

#[cfg(test)]
fn dialogue_language(dialogue: &[Dialogue]) -> &'static str {
    let sample: String = dialogue
        .iter()
        .take(2000)
        .map(|d| d.text.as_str())
        .collect();
    let kana = sample
        .chars()
        .filter(|c| ('\u{3040}'..='\u{30ff}').contains(c))
        .count();
    let han = sample
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    if kana >= 3 && kana * 6 > han {
        "ja-JP"
    } else if han > 0 {
        "zh-CN"
    } else {
        "en-US"
    }
}

#[cfg(test)]
fn parse_script(source: &str, data: &[u8]) -> Result<Vec<Dialogue>, Message> {
    if data.starts_with(b"Entis") {
        return csx(data, source);
    }
    let text = decode_text(data);
    // Reject binary blobs instead of labelling arbitrary decoded bytes as dialogue.
    if text
        .chars()
        .filter(|c| *c == '\0' || *c == '\u{fffd}')
        .count()
        * 100
        > text.chars().count().max(1)
    {
        return Ok(Vec::new());
    }
    let tags = regex::Regex::new(r"\[[^\]\r\n]*\]|<[^>\r\n]*>").map_err(io)?;
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = tags.replace_all(line, "");
            let line = line.trim();
            (!line.starts_with([';', '*', '@', '#']) && is_dialogue(line)).then(|| Dialogue {
                source: source.into(),
                position: index as u64,
                text: line.into(),
                speaker: None,
            })
        })
        .collect())
}

#[cfg(test)]
type CandidatePriority = (bool, bool, bool, bool);
#[cfg(test)]
fn select_script(
    resources: &mut BTreeMap<String, (CandidatePriority, Vec<Dialogue>)>,
    name: String,
    source: String,
    data: &[u8],
    language: Language,
    loose: bool,
) -> Result<(), Message> {
    let rows = parse_script(&source, data)?;
    if rows.is_empty() {
        return Ok(());
    }
    let matches = dialogue_language(&rows) == language.code();
    let source_path = source.replace('\\', "/").to_lowercase();
    let priority = (
        matches,
        !matches && source_path.starts_with("script.noa:"),
        loose,
        source_path.contains("patch"),
    );
    if resources.get(&name).is_none_or(|(old, _)| priority >= *old) {
        resources.insert(name, (priority, rows));
    }
    Ok(())
}

pub fn decode_text(data: &[u8]) -> String {
    if data.starts_with(&[0xff, 0xfe]) {
        encoding_rs::UTF_16LE.decode(&data[2..]).0.into_owned()
    } else if let Ok(s) = std::str::from_utf8(data) {
        s.trim_start_matches('\u{feff}').into()
    } else {
        encoding_rs::SHIFT_JIS.decode(data).0.into_owned()
    }
}

/// Length-prefixed UTF-16 strings preserve offsets, letting generated parsers select fields.
pub fn strings(data: &[u8]) -> Vec<Value> {
    let mut output = Vec::new();
    let title_end = if data.get(64..72) == Some(b"title   ") {
        u64_at(data, 72).unwrap_or(0).saturating_add(80) as usize
    } else {
        0
    };
    for p in 0..data.len().saturating_sub(6) {
        let Some(len) = u32_at(data, p).filter(|n| (2..=2048).contains(n)) else {
            continue;
        };
        let Some(raw) = data.get(p + 4..p + 4 + len * 2) else {
            continue;
        };
        let (text, errors) = encoding_rs::UTF_16LE.decode_without_bom_handling(raw);
        if errors
            || text.chars().any(|c| {
                c.is_control() || c == '\u{fffd}' || ('\u{e000}'..='\u{f8ff}').contains(&c)
            })
            || !text.chars().any(char::is_alphanumeric)
        {
            continue;
        }
        output.push(json!({"offset":p,"text":text,"section":if p<title_end {"title"}else{"context"},"after":(0..6).filter_map(|i|u32_at(data,p+4+len*2+i*4)).collect::<Vec<_>>()}));
        if output.len() >= 10_000 {
            break;
        }
    }
    output
}

pub fn save_inputs(paths: &[PathBuf], home: &Path, scratch: &Path) -> Result<Vec<Value>, Message> {
    let mut jobs = Vec::new();
    let mut output = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let before = stamp(path)?;
        let data = read(path, 8 * 1024 * 1024)?;
        if stamp(path)? != before {
            return Err(Message::new("summary.save_changed"));
        }
        let base = if data.starts_with(b"BM") {
            u32_at(&data, 2).ok_or_else(invalid)?
        } else {
            0
        };
        let mut item = json!({"file":before,"sha256":format!("{:x}",Sha256::digest(&data)),"strings":[],"text":""});
        if data.get(base..base + 8) == Some(b"Entis\x1a\0\0") {
            let input = scratch.join(format!("save-{index}.bin"));
            fs::write(&input, &data).map_err(io)?;
            jobs.push(json!({"input":input,"output":scratch.join(format!("decoded-{index}.bin"))}));
        } else {
            item["strings"] = json!(strings(&data));
            let text = decode_text(&data);
            if !text.contains('\0') {
                item["text"] = json!(text.chars().take(32_768).collect::<String>());
            }
        }
        output.push(item);
    }
    if !jobs.is_empty() {
        let helper = scratch.join("decode-entis.ps1");
        fs::write(&helper, include_str!("decode-entis.ps1")).map_err(io)?;
        let job_file = scratch.join("jobs.json");
        write_json(&job_file, &jobs)?;
        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(helper)
        .arg("-Assembly")
        .arg(tool(home)?.with_file_name("ArcFormats.dll"))
        .arg("-Jobs")
        .arg(job_file);
        child(cmd, Duration::from_secs(60))?;
        for (index, item) in output.iter_mut().enumerate() {
            let path = scratch.join(format!("decoded-{index}.bin"));
            if path.exists() {
                item["strings"] = json!(strings(&read(&path, 8 * 1024 * 1024)?));
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_language_wins_even_when_a_foreign_patch_is_loose() {
        let mut candidates = BTreeMap::new();
        select_script(
            &mut candidates,
            "scene.ks".into(),
            "original:scene.ks".into(),
            b"This is the English story.",
            Language::English,
            false,
        )
        .unwrap();
        select_script(
            &mut candidates,
            "scene.ks".into(),
            "patch/scene.ks".into(),
            "这是中文补丁的故事台词。".as_bytes(),
            Language::English,
            true,
        )
        .unwrap();
        assert_eq!(
            candidates["scene.ks"].1[0].text,
            "This is the English story."
        );
    }

    #[test]
    fn chinese_archive_overrides_japanese_and_short_anchors_survive() {
        let mut candidates = BTreeMap::new();
        select_script(
            &mut candidates,
            "script.csx".into(),
            "script.noa:script.csx".into(),
            "「これは日本語の台詞です。」".as_bytes(),
            Language::Chinese,
            false,
        )
        .unwrap();
        select_script(
            &mut candidates,
            "script.csx".into(),
            "ev.noa:script.csx".into(),
            "「这是中文台词。」\n好！".as_bytes(),
            Language::Chinese,
            false,
        )
        .unwrap();
        assert_eq!(
            candidates["script.csx"]
                .1
                .iter()
                .map(|d| d.text.as_str())
                .collect::<Vec<_>>(),
            ["「这是中文台词。」", "好！"]
        );
    }

    #[test]
    fn missing_translation_falls_back_to_original_script() {
        let mut candidates = BTreeMap::new();
        select_script(
            &mut candidates,
            "script.csx".into(),
            "script.noa:script.csx".into(),
            "「これは日本語の台詞です。」".as_bytes(),
            Language::English,
            false,
        )
        .unwrap();
        select_script(
            &mut candidates,
            "script.csx".into(),
            "ev.noa:script.csx".into(),
            "「这是中文台词。」".as_bytes(),
            Language::English,
            false,
        )
        .unwrap();
        assert_eq!(dialogue_language(&candidates["script.csx"].1), "ja-JP");
    }
}
