//! Per-installation setup cache, generated save parser and grounded progress summaries.
mod agent;
mod background_identity;
mod context;
#[cfg(windows)]
mod process;
mod report;
mod resources;
mod route_map;
mod sandbox;

pub use route_map::route_map;

use crate::{Game, Settings, ai, data_sources, saves};
use galgear_i18n::{Language, Message};
use resources::{Dialogue, Stamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

const SCHEMA: u32 = 5;
const REPORT_SCHEMA: u32 = 5;
pub const FOLDER: &str = ".galgear-ai-summary";

/// Stop AI workers and their tools before the desktop runtime exits.
/// Once shutdown starts, no new workers may be launched in this process.
pub fn shutdown() {
    #[cfg(windows)]
    process::shutdown();
}

fn invalid() -> Message {
    Message::new("summary.invalid_data")
}
fn io(error: impl std::fmt::Display) -> Message {
    Message::detail("summary.io", error)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, Message> {
    let file = fs::File::open(path).map_err(io)?;
    if file.metadata().map_err(io)?.len() > limit {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    Ok(bytes)
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Message> {
    let parent = path.parent().ok_or_else(invalid)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
    serde_json::to_writer_pretty(&mut file, value).map_err(io)?;
    file.as_file().sync_all().map_err(io)?;
    file.persist(path).map_err(io)?;
    Ok(())
}
fn hash(path: &Path) -> Result<String, Message> {
    let mut file = fs::File::open(path).map_err(io)?;
    let mut sha = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(io)?;
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn language_code(value: &str) -> String {
    ["zh-CN", "ja-JP", "en-US"]
        .into_iter()
        .find(|code| value.starts_with(code))
        .unwrap_or(value)
        .to_owned()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Background {
    pub title: String,
    pub source: String,
    pub url: String,
    pub description: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    schema: u32,
    game_id: String,
    language: Language,
    script_language: String,
    sources: Vec<Stamp>,
    inventory: Vec<Stamp>,
    program_hash: String,
    dialogue_hash: String,
    background_hash: String,
    dialogue_count: usize,
    #[serde(default)]
    python_agent: bool,
    #[serde(default)]
    script_program_hash: Option<String>,
    #[serde(default)]
    save_extensions: Vec<String>,
    #[serde(default)]
    speaker_verification_version: u32,
    #[serde(default)]
    save_verification_version: u32,
    #[serde(default)]
    script_verification_version: u32,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Setup {
    pub reused: bool,
    pub directory: String,
    pub script_language: String,
    pub dialogue_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub game_id: String,
    #[serde(flatten)]
    pub content: report::Content,
    pub provider: String,
    pub model: String,
    pub generated_at: u64,
    pub save_file: String,
    pub source_url: String,
    pub script_language: String,
    pub warnings: Vec<String>,
    pub cached: bool,
    pub save_hash: String,
}

#[derive(Serialize, Deserialize)]
struct CachedSummary {
    schema: u32,
    key: String,
    save_content: Value,
    summary: Summary,
}

fn summary_key(
    manifest: &Manifest,
    save_hash: &str,
    settings: &Settings,
) -> Result<String, Message> {
    let bytes = serde_json::to_vec(&(
        REPORT_SCHEMA,
        manifest.game_id.as_str(),
        manifest.language,
        &manifest.program_hash,
        &manifest.dialogue_hash,
        &manifest.background_hash,
        save_hash,
        &settings.ai_provider,
        settings.ai_models.get(&settings.ai_provider),
        report::fingerprint(),
        context::fingerprint(),
    ))
    .map_err(io)?;
    let mut digest = Sha256::new();
    digest.update(bytes);
    if manifest.python_agent {
        // Legacy non-Python summary keys keep their existing offline behavior.
        digest.update(b"\0save_verification_version:");
        digest.update(manifest.save_verification_version.to_le_bytes());
        digest.update(b"\0script_verification_version:");
        digest.update(manifest.script_verification_version.to_le_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn cached_summary(directory: &Path, key: &str, game: &Game) -> Option<CachedSummary> {
    let bytes = read(
        &directory.join("summaries").join(format!("{key}.json")),
        1024 * 1024,
    )
    .ok()?;
    let cached: CachedSummary = serde_json::from_slice(&bytes).ok()?;
    (cached.schema == REPORT_SCHEMA
        && cached.key == key
        && cached.summary.game_id == game.id
        && cached.summary.content.validate().is_ok())
    .then_some(cached)
}

fn complete_part(
    directory: &Path,
    source: &str,
    token: &str,
    model: &str,
    system: &str,
    input: &str,
) -> Result<String, Message> {
    complete_part_with_validation(
        directory,
        source,
        token,
        model,
        system,
        input,
        report::cacheable_response,
    )
}

fn complete_part_with_validation(
    directory: &Path,
    source: &str,
    token: &str,
    model: &str,
    system: &str,
    input: &str,
    cacheable: impl Fn(&str) -> bool,
) -> Result<String, Message> {
    let mut digest = Sha256::new();
    for field in [source, model, system, input] {
        digest.update((field.len() as u64).to_le_bytes());
        digest.update(field.as_bytes());
    }
    let parts = directory.join("analysis");
    fs::create_dir_all(&parts).map_err(io)?;
    let path = parts.join(format!("{:x}.json", digest.finalize()));
    if let Ok(bytes) = read(&path, 256 * 1024)
        && let Ok(response) = serde_json::from_slice::<String>(&bytes)
        && cacheable(&response)
    {
        return Ok(response);
    }
    let response = ai::complete(source, token, model, system, input, true)?;
    if cacheable(&response) {
        write_json(&path, &response)?;
    } else {
        // Keep rejected replies for local diagnosis, outside the reusable cache.
        // They contain model output only; credentials and request headers are absent.
        let rejected = directory.join("analysis-invalid");
        if fs::create_dir_all(&rejected).is_ok() {
            let _ = write_json(
                &rejected.join(path.file_name().ok_or_else(invalid)?),
                &response,
            );
        }
    }
    Ok(response)
}

pub fn cache(game: &Game, language: Language) -> PathBuf {
    Path::new(&game.folder).join(FOLDER).join(language.code())
}

fn reusable_speakers(directory: &Path, version: u32) -> Result<bool, Message> {
    if version >= 1 {
        let result_path = directory.join("result.json");
        if result_path.is_file() {
            let result: Value = serde_json::from_slice(&read(&result_path, 4 * 1024 * 1024)?)
                .map_err(|_| invalid())?;
            if result["speaker_verification"]["external_rows"]
                .as_u64()
                .is_some_and(|count| count > 0)
                && result["speaker_verification"]["scope_version"].as_u64() != Some(2)
            {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    // Older hosts could finish with every speaker unresolved despite a companion
    // name table. Keep proven inline caches; resume the incomplete ones for review.
    for row in BufReader::new(fs::File::open(directory.join("dialogue.jsonl")).map_err(io)?).lines()
    {
        let row: Dialogue = serde_json::from_str(&row.map_err(io)?).map_err(io)?;
        if row
            .speaker
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn manifest(game: &Game, language: Language) -> Result<Manifest, Message> {
    let directory = cache(game, language);
    if directory.join("repair-required.json").exists() {
        return Err(invalid());
    }
    // Generated programs cannot be redirected to another directory through links.
    for name in ["", "manifest.json", "dialogue.jsonl", "background.json"] {
        let path = directory.join(name);
        if fs::symlink_metadata(&path)
            .map_err(io)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid());
        }
    }
    let mut value: Manifest =
        serde_json::from_slice(&read(&directory.join("manifest.json"), 1024 * 1024)?)
            .map_err(|_| invalid())?;
    value.script_language = language_code(&value.script_language);
    if value.python_agent
        && (value.save_verification_version != agent::SAVE_VERIFICATION_VERSION
            || value.script_verification_version != agent::SCRIPT_VERIFICATION_VERSION)
    {
        return Err(invalid());
    }
    let programs: &[&str] = if value.python_agent {
        &["save_extractor.py", "script_extractor.py"]
    } else {
        &["extractor.rhai"]
    };
    for name in programs {
        if fs::symlink_metadata(directory.join(name))
            .map_err(io)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid());
        }
    }
    if value.schema != SCHEMA
        || value.game_id != game.id
        || value.language != language
        || value.dialogue_count == 0
        || value.program_hash != hash(&directory.join(programs[0]))?
        || value.dialogue_hash != hash(&directory.join("dialogue.jsonl"))?
        || value.background_hash != hash(&directory.join("background.json"))?
    {
        return Err(invalid());
    }
    if value.python_agent
        && value.script_program_hash.as_ref()
            != Some(&hash(&directory.join("script_extractor.py"))?)
    {
        return Err(invalid());
    }
    if value.python_agent && !reusable_speakers(&directory, value.speaker_verification_version)? {
        return Err(invalid());
    }
    for source in &value.sources {
        let current = resources::stamp(Path::new(&source.path))?;
        if current.size != source.size || current.modified != source.modified {
            return Err(invalid());
        }
    }
    // Match the canonical root used when this manifest was prepared.
    let root = Path::new(&game.folder).canonicalize().map_err(io)?;
    if resources::source_inventory(&root)? != value.inventory {
        return Err(invalid());
    }
    Ok(value)
}

fn provider(settings: &Settings, config: &Path) -> Result<(String, String, String), Message> {
    let (source, selected) = ai::selected_model(settings)?;
    if let Some(model) = selected {
        let token = ai::token_for(config, source, "")?;
        return Ok((source.into(), model.into(), token));
    }
    if source == "deepseek"
        && let Ok(token) = ai::token_for(config, source, "")
    {
        let models = ai::list_models(source, &token)?;
        if let Some(model) = models
            .iter()
            .find(|m| m.id.contains("flash") || m.id.contains("chat"))
        {
            return Ok((source.into(), model.id.clone(), token));
        }
    }
    Err(Message::new("summary.ai_missing"))
}

/// Resolve a uniquely matched story introduction without changing settings or game metadata.
pub fn background(game: &Game, settings: &Settings, config: &Path) -> Result<Background, Message> {
    let sources: &[&str] = if settings.language == Language::English {
        &["vndb"]
    } else {
        &["bangumi", "2dfan"]
    };
    if let Some(meta) = &game.metadata
        && let Ok((source, _)) = data_sources::source_for_url(&meta.url)
        && sources.contains(&source)
        && !meta.description.trim().is_empty()
    {
        return Ok(Background {
            title: meta.title.clone(),
            source: source.into(),
            url: meta.url.clone(),
            description: meta.description.clone(),
        });
    }
    for source in sources {
        let credentials = if *source == "2dfan" {
            data_sources::Credentials::default()
        } else {
            data_sources::load(config, source)?
        };
        for identity in background_identity::queries(game) {
            let query: String = identity.chars().take(95).collect();
            let Ok(results) =
                data_sources::search(source, &settings.fan_domain, &query, &credentials)
            else {
                continue;
            };
            let Some(result) = background_identity::unique_match(&results, &identity) else {
                continue;
            };
            if let Ok(meta) =
                data_sources::metadata(&settings.fan_domain, &result.url, &credentials)
                && background_identity::normalize(&meta.title)
                    == background_identity::normalize(&identity)
                && !meta.description.trim().is_empty()
            {
                return Ok(Background {
                    title: meta.title,
                    source: (*source).into(),
                    url: meta.url,
                    description: meta.description,
                });
            }
        }
    }
    Err(Message::new("summary.background_missing"))
}

#[cfg(test)]
fn save_files(game: &Game, settings: &Settings) -> Result<Vec<PathBuf>, Message> {
    save_files_with_extensions(game, settings, None)
}

/// A bounded native-state observation, without a current-dialogue claim.
#[derive(Debug, Serialize)]
pub struct SaveCandidate {
    pub path: String,
    pub kind: saves::SaveKind,
    pub modified: u64,
    pub source: String,
}

/// The exact save selection used by preparation, safe to inspect without credentials.
#[derive(Debug, Serialize)]
pub struct PreparationSaveInputs {
    pub save: String,
    pub required_saves: Vec<String>,
    pub candidates: Vec<SaveCandidate>,
}

/// Discover saves and require the latest file only when its native role is known.
/// Unknown formats remain available for the extraction agent to identify itself.
pub fn preparation_save_inputs(
    game: &Game,
    settings: &Settings,
) -> Result<PreparationSaveInputs, Message> {
    let candidates = save_candidates(game, settings, None)?;
    let save = if let Some(save) = preferred_save_root(&candidates) {
        save
    } else {
        saves::discover(game, settings.game_edits.get(&game.id))
            .locations
            .into_iter()
            .find(|location| location.exists)
            .map(|location| PathBuf::from(location.path))
            .ok_or_else(|| Message::new("summary.no_saves"))?
    };
    // A newer unknown file must be inspected rather than silently replaced by
    // an older recognized save. Control files remain in the diagnostic ledger.
    let required_saves = candidates
        .iter()
        .find(|candidate| {
            candidate.kind != saves::SaveKind::Control
                && Path::new(&candidate.path).parent() == Some(save.as_path())
        })
        .filter(|candidate| candidate.kind == saves::SaveKind::Progress)
        .map(|candidate| vec![candidate.path.clone()])
        .unwrap_or_default();
    Ok(PreparationSaveInputs {
        save: crate::config::display_path(&save),
        required_saves,
        candidates,
    })
}

fn save_source_priority(source: &str) -> u8 {
    match source {
        "manual" => 4,
        "config" => 3,
        "game" => 2,
        _ => 1,
    }
}

fn preferred_save_root(candidates: &[SaveCandidate]) -> Option<PathBuf> {
    candidates
        .iter()
        .filter(|candidate| candidate.kind != saves::SaveKind::Control)
        .max_by(|left, right| {
            save_source_priority(&left.source)
                .cmp(&save_source_priority(&right.source))
                .then_with(|| {
                    (left.kind == saves::SaveKind::Progress)
                        .cmp(&(right.kind == saves::SaveKind::Progress))
                })
                .then_with(|| left.modified.cmp(&right.modified))
                .then_with(|| left.path.cmp(&right.path))
        })
        .and_then(|candidate| Path::new(&candidate.path).parent().map(Path::to_path_buf))
}

fn save_files_with_extensions(
    game: &Game,
    settings: &Settings,
    extensions: Option<&[String]>,
) -> Result<Vec<PathBuf>, Message> {
    let candidates = save_candidates(game, settings, extensions)?;
    let paths = selected_save_paths(&candidates);
    if paths.is_empty() {
        return Err(Message::new("summary.no_saves"));
    }
    Ok(paths)
}

fn selected_save_paths(candidates: &[SaveCandidate]) -> Vec<PathBuf> {
    let root = preferred_save_root(candidates);
    let selected = candidates
        .iter()
        .filter(|candidate| {
            candidate.kind != saves::SaveKind::Control
                && Path::new(&candidate.path).parent() == root.as_deref()
        })
        .collect::<Vec<_>>();
    // An observed WillPlus slot family can disambiguate its companion .dat
    // files, but must never erase a different, more authoritative save root.
    let willplus = selected
        .iter()
        .any(|candidate| willplus_slot(Path::new(&candidate.path)));
    selected
        .into_iter()
        .filter(|candidate| !willplus || willplus_slot(Path::new(&candidate.path)))
        .take(8)
        .map(|candidate| PathBuf::from(&candidate.path))
        .collect()
}

fn willplus_slot(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.to_ascii_lowercase().ends_with("save-willplus"))
}

fn save_candidates(
    game: &Game,
    settings: &Settings,
    extensions: Option<&[String]>,
) -> Result<Vec<SaveCandidate>, Message> {
    let report = saves::discover(game, settings.game_edits.get(&game.id));
    save_candidates_in_report(&report, extensions)
}

fn save_candidates_in_report(
    report: &saves::SaveReport,
    extensions: Option<&[String]>,
) -> Result<Vec<SaveCandidate>, Message> {
    let mut found = Vec::new();
    for location in report.locations.iter().filter(|l| l.exists) {
        let root = Path::new(&location.path).canonicalize().map_err(io)?;
        for entry in fs::read_dir(&root).map_err(io)?.take(2048) {
            let entry = entry.map_err(io)?;
            let path = entry.path();
            if !entry.file_type().map_err(io)?.is_file() {
                continue;
            }
            if fs::metadata(&path).map_err(io)?.len() <= 8 * 1024 * 1024 {
                let kind = saves::classify(&path);
                found.push((
                    resources::stamp(&path)?.modified,
                    path,
                    kind,
                    location.source.clone(),
                ));
            }
        }
    }
    let mut native =
        std::collections::BTreeMap::<PathBuf, (u64, std::collections::BTreeSet<String>)>::new();
    for (modified, path, kind, _) in &found {
        if *kind == saves::SaveKind::Progress {
            let entry = native
                .entry(path.parent().ok_or_else(invalid)?.to_path_buf())
                .or_default();
            entry.0 = entry.0.max(*modified);
            entry.1.insert(save_file_family(path));
        }
    }
    found.retain(|(modified, path, kind, source)| {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(extensions) = extensions {
            // A saved extension set records past validation, not every native
            // progress format this installation can produce. Independently parse
            // a newly recognized progress container; failures use normal repair.
            // Unknown formats keep the cached boundary until preparation, so
            // ordinary new logs/media cannot silently select a different save.
            return *kind == saves::SaveKind::Progress || extensions.contains(&extension);
        }
        if *kind != saves::SaveKind::Unknown || known_save_extension(&extension) {
            return true;
        }
        // Unsupported native wrappers remain discoverable even when their
        // extension resembles a log/media file. This does not verify a node.
        if possible_native_save_header(path) {
            return true;
        }
        if auxiliary_save_extension(&extension) {
            return false;
        }
        let same_folder = path.parent().and_then(|folder| native.get(folder));
        // An explicit save directory supplies context for opaque files. Weaker
        // locations need a slot-shaped name, an observed family, or a newer file
        // beside proven progress. Such files stay Unknown for agent inspection;
        // their timestamps cannot silently force an older recognized save.
        matches!(source.as_str(), "manual" | "config")
            || slot_shaped_name(path)
            || same_folder.is_some_and(|(latest, families)| {
                *modified > *latest || families.contains(&save_file_family(path))
            })
    });
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    found.dedup_by(|a, b| a.1 == b.1);
    Ok(found
        .into_iter()
        .map(|(modified, path, kind, source)| SaveCandidate {
            path: crate::config::display_path(&path),
            kind,
            modified,
            source,
        })
        .collect())
}

fn known_save_extension(extension: &str) -> bool {
    matches!(
        extension,
        "sav" | "dat" | "bmp" | "ksd" | "asd" | "json" | "rpgsave" | "rmmzsave" | "save"
    ) || extension.ends_with("save-willplus")
}

fn auxiliary_save_extension(extension: &str) -> bool {
    matches!(
        extension,
        "log"
            | "txt"
            | "md"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "ico"
            | "svg"
            | "wav"
            | "ogg"
            | "mp3"
            | "mp4"
            | "avi"
            | "exe"
            | "dll"
            | "ini"
            | "cfg"
            | "xml"
            | "html"
            | "htm"
            | "css"
            | "js"
            | "tmp"
            | "bak"
            | "zip"
            | "7z"
            | "rar"
    )
}

fn slot_shaped_name(path: &Path) -> bool {
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    [
        "savegame",
        "save",
        "slot",
        "quicksave",
        "autosave",
        "quick",
        "auto",
        "continue",
        "data",
    ]
    .iter()
    .any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|tail| {
            tail.is_empty()
                || tail
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_digit() || b"_-. ".contains(byte))
        })
    })
}

fn save_file_family(path: &Path) -> String {
    let mut family = String::new();
    let mut digit = false;
    for character in path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
        .chars()
    {
        if character.is_ascii_digit() {
            if !digit {
                family.push('#');
            }
            digit = true;
        } else {
            family.push(character);
            digit = false;
        }
    }
    family
}

fn possible_native_save_header(path: &Path) -> bool {
    fn header(bytes: &[u8]) -> bool {
        if bytes.len() >= 5
            && bytes[..2] == [0xfe, 0xfe]
            && bytes[2] <= 2
            && bytes[3..5] == [0xff, 0xfe]
        {
            return true;
        }
        let text = if bytes.starts_with(&[0xff, 0xfe]) {
            String::from_utf16_lossy(
                &bytes[2..]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>(),
            )
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        let text = text.trim_start_matches('\u{feff}').trim_start();
        text.starts_with("%[")
            || text
                .strip_prefix("(const)")
                .is_some_and(|tail| tail.trim_start().starts_with("%["))
    }
    use std::io::{Seek, SeekFrom};
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut bytes = [0; 64];
    let Ok(length) = file.read(&mut bytes) else {
        return false;
    };
    if header(&bytes[..length]) {
        return true;
    }
    if length >= 14 && bytes[..2] == *b"BM" {
        let size = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        if size >= 14 && file.seek(SeekFrom::Start(u64::from(size))).is_ok() {
            return file
                .read(&mut bytes)
                .is_ok_and(|length| header(&bytes[..length]));
        }
    }
    false
}

fn grounded(facts: &sandbox::Facts, input: &Value) -> bool {
    let title: Vec<_> = input["strings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["section"] == "title")
        .filter_map(|s| s["text"].as_str())
        .filter(|s| !matches!(*s, "SaveDataTitle" | "Time"))
        .collect();
    if let Some(current) = title.last()
        && (!facts.anchors.iter().any(|a| a == current)
            || !facts.anchors.iter().all(|a| title.contains(&a.as_str())))
    {
        return false;
    }
    !facts.anchors.is_empty()
        && facts.anchors.iter().all(|anchor| {
            !anchor.trim().is_empty()
                && (input["strings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["text"].as_str() == Some(anchor))
                    || input["text"].as_str().is_some_and(|s| s.contains(anchor)))
        })
}

/// Re-enabling a complete cache returns before any network request or extraction.
pub fn prepare(
    game: &Game,
    settings: &Settings,
    config: &Path,
    home: &Path,
) -> Result<Setup, Message> {
    if let Ok(m) = manifest(game, settings.language) {
        return Ok(Setup {
            reused: true,
            directory: crate::config::display_path(&cache(game, settings.language)),
            script_language: m.script_language,
            dialogue_count: m.dialogue_count,
        });
    }
    let (source, model, token) = provider(settings, config)?;
    let background = background(game, settings, config)?;
    let root = Path::new(&game.folder).canonicalize().map_err(io)?;
    let base = root.join(FOLDER);
    if base.exists()
        && fs::symlink_metadata(&base)
            .map_err(io)?
            .file_type()
            .is_symlink()
    {
        return Err(invalid());
    }
    fs::create_dir_all(&base).map_err(io)?;
    let staging = tempfile::Builder::new()
        .prefix(".preparing-")
        .tempdir_in(&base)
        .map_err(io)?;
    let save_inputs = preparation_save_inputs(game, settings)?;
    let work = base.join(format!(".agent-{}-{}", settings.language.code(), source));
    if fs::symlink_metadata(&work).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(invalid());
    }
    let inventory = resources::source_inventory(&root)?;
    let repair_reason = read(
        &cache(game, settings.language).join("repair-required.json"),
        32 * 1024,
    )
    .ok()
    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let result = agent::prepare(
        home,
        &work,
        json!({
            "provider":source,"model":model,"token":token,"title":game.title,
            "language":settings.language.code(),"game":crate::config::display_path(&root),
            "save":save_inputs.save,
            "skill":crate::config::display_path(&home.join("skills/GalgameResourceExtract")),
            "work":crate::config::display_path(&work),"max_turns":150,
            "runtime_limit_seconds":agent::PREPARATION_TIMEOUT_SECONDS,
            "save_samples":1,"inventory":inventory,
            "required_saves":save_inputs.required_saves,
            "save_candidates":save_inputs.candidates
            ,"repair_reason":repair_reason
        }),
    )?;
    if resources::source_inventory(&root)? != inventory {
        return Err(invalid());
    }
    let directory = staging.path();
    for name in [
        "save_extractor.py",
        "script_extractor.py",
        "dialogue.jsonl",
        "result.json",
        "events.json",
    ] {
        fs::copy(work.join(name), directory.join(name)).map_err(io)?;
    }
    let script_language = result["script_language"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    let script_language = language_code(&script_language);
    let dialogue_count = result["dialogue_count"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 500_000)
        .ok_or_else(invalid)? as usize;
    let sources = result["sources"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|source| {
            let path = Path::new(source.as_str().ok_or_else(invalid)?)
                .canonicalize()
                .map_err(io)?;
            if !path.starts_with(&root) {
                return Err(invalid());
            }
            resources::stamp(&path)
        })
        .collect::<Result<Vec<_>, Message>>()?;
    let save_extensions: Vec<String> = result["validated_saves"]
        .as_object()
        .ok_or_else(invalid)?
        .keys()
        .map(|path| {
            Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase()
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if save_extensions.is_empty() {
        return Err(invalid());
    }
    write_json(&directory.join("background.json"), &background)?;
    write_json(
        &directory.join("manifest.json"),
        &Manifest {
            schema: SCHEMA,
            game_id: game.id.clone(),
            language: settings.language,
            script_language: script_language.clone(),
            sources,
            inventory,
            program_hash: hash(&directory.join("save_extractor.py"))?,
            python_agent: true,
            script_program_hash: Some(hash(&directory.join("script_extractor.py"))?),
            save_extensions,
            speaker_verification_version: result["speaker_verification"]["version"]
                .as_u64()
                .and_then(|version| u32::try_from(version).ok())
                .ok_or_else(invalid)?,
            save_verification_version: result["save_verification_version"]
                .as_u64()
                .and_then(|version| u32::try_from(version).ok())
                .ok_or_else(invalid)?,
            script_verification_version: result["script_verification_version"]
                .as_u64()
                .and_then(|version| u32::try_from(version).ok())
                .ok_or_else(invalid)?,
            dialogue_hash: hash(&directory.join("dialogue.jsonl"))?,
            background_hash: hash(&directory.join("background.json"))?,
            dialogue_count,
        },
    )?;
    let target = cache(game, settings.language);
    if target.exists() {
        fs::rename(
            &target,
            base.join(format!(".previous-{}-{}", settings.language.code(), now())),
        )
        .map_err(io)?;
    }
    fs::rename(directory, &target).map_err(io)?;
    Ok(Setup {
        reused: false,
        directory: crate::config::display_path(&target),
        script_language,
        dialogue_count,
    })
}

fn script_context(
    directory: &Path,
    facts: &sandbox::Facts,
    primary: Option<&str>,
) -> Result<Vec<Dialogue>, Message> {
    let file = fs::File::open(directory.join("dialogue.jsonl")).map_err(io)?;
    if file.metadata().map_err(io)?.len() > 256 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut lines = Vec::new();
    let mut matches = Vec::new();
    let mut markers = std::collections::BTreeMap::<String, std::collections::BTreeSet<u64>>::new();
    let mut scanned = 0usize;
    for line in BufReader::new(file).lines() {
        scanned += 1;
        if scanned > 500_000 {
            return Err(invalid());
        }
        let line = line.map_err(io)?;
        if line.len() > 64 * 1024 {
            return Err(invalid());
        }
        let raw: Value = serde_json::from_str(&line).map_err(|_| invalid())?;
        let marker = context::is_resume_marker(&raw);
        let entry: Dialogue = serde_json::from_value(raw).map_err(|_| invalid())?;
        if entry.source.is_empty()
            || entry.text.trim().is_empty()
            || entry
                .speaker
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > 1024)
        {
            return Err(invalid());
        }
        if marker {
            // Restore labels join native save state to a script entry. Their
            // captions are structural metadata, never spoken or read dialogue.
            markers
                .entry(entry.source)
                .or_default()
                .insert(entry.position);
            continue;
        }
        if primary.or_else(|| facts.anchors.first().map(String::as_str))
            == Some(entry.text.as_str())
            && facts
                .source
                .as_ref()
                .is_none_or(|source| *source == entry.source)
            && facts.line.is_none_or(|position| position == entry.position)
        {
            matches.push(lines.len());
        }
        lines.push(entry);
        if lines.len() > 500_000 {
            return Err(invalid());
        }
    }
    if let Some(resume) = &facts.resume {
        if !facts.valid_resume() {
            return Err(invalid());
        }
        return if facts.history.is_empty() {
            context::before_resume(&lines, &resume.source, resume.position)
        } else {
            recorded_history(&lines, &facts.history, &markers)
        };
    }
    if matches.is_empty() {
        return Err(Message::new("summary.no_match"));
    }
    if matches.len() != 1 {
        return Err(Message::new("summary.ambiguous_anchor"));
    }
    let anchor = &lines[matches[0]];
    // Code neighbours supply plot candidates, not a record of the player's route.
    if facts.history.is_empty() {
        return context::candidates(&lines, anchor);
    }
    let context = recorded_history(&lines, &facts.history, &markers)?;
    if context
        .last()
        .is_none_or(|row| row.source != anchor.source || row.position != anchor.position)
    {
        return Err(invalid());
    }
    Ok(context)
}

fn recorded_history(
    lines: &[Dialogue],
    history: &[sandbox::NodeRef],
    markers: &std::collections::BTreeMap<String, std::collections::BTreeSet<u64>>,
) -> Result<Vec<Dialogue>, Message> {
    if history.len() > 10_000 {
        return Err(invalid());
    }
    let mut context = Vec::new();
    for node in history {
        if markers
            .get(&node.source)
            .is_some_and(|positions| positions.contains(&node.position))
        {
            continue;
        }
        let mut candidates = lines
            .iter()
            .filter(|row| row.source == node.source && row.position == node.position);
        let row = candidates.next().ok_or_else(invalid)?;
        if candidates.next().is_some() {
            return Err(Message::new("summary.ambiguous_anchor"));
        }
        context.push(row.clone());
    }
    Ok(context)
}

pub fn summarize(
    game: &Game,
    settings: &Settings,
    config: &Path,
    home: &Path,
) -> Result<Summary, Message> {
    summarize_attempt(game, settings, config, home, true)
}

fn summarize_attempt(
    game: &Game,
    settings: &Settings,
    config: &Path,
    home: &Path,
    can_repair: bool,
) -> Result<Summary, Message> {
    if !settings
        .game_edits
        .get(&game.id)
        .is_some_and(|e| e.ai_summary)
    {
        return Err(Message::new("summary.disabled"));
    }
    let m =
        manifest(game, settings.language).map_err(|_| Message::new("summary.setup_required"))?;
    let directory = cache(game, settings.language);
    let paths = save_files_with_extensions(
        game,
        settings,
        (!m.save_extensions.is_empty()).then_some(m.save_extensions.as_slice()),
    )?;
    let save_hash = hash(&paths[0])?;
    let key = summary_key(&m, &save_hash, settings)?;
    // This check deliberately precedes the generated parser, resource tools and AI
    // credentials: an unchanged save remains available even when offline.
    if let Some(mut cached) = cached_summary(&directory, &key, game) {
        cached.summary.cached = true;
        cached.summary.save_file = crate::config::display_path(&paths[0]);
        cached.summary.script_language = m.script_language.clone();
        write_json(&directory.join("latest-save.json"), &cached.save_content)?;
        write_json(&directory.join("latest-summary.json"), &cached.summary)?;
        return Ok(cached.summary);
    }
    let (input, facts, primary) = if m.python_agent {
        let facts = match agent::parse(home, &directory, &paths[0]) {
            Ok(facts) => facts,
            Err(error) => {
                if hash(&paths[0])? != save_hash {
                    return Err(Message::new("summary.save_changed"));
                }
                write_json(
                    &directory.join("repair-required.json"),
                    &json!({"save_file":crate::config::display_path(&paths[0]),"sha256":save_hash,"error":error}),
                )?;
                if can_repair {
                    // One repair attempt uses the existing model checkpoint and revalidates
                    // the current save. A second failure remains explicit and retryable.
                    prepare(game, settings, config, home)?;
                    return summarize_attempt(game, settings, config, home, false);
                }
                return Err(error);
            }
        };
        if hash(&paths[0])? != save_hash {
            return Err(Message::new("summary.save_changed"));
        }
        let file = resources::stamp(&paths[0])?;
        (json!({"file":file,"sha256":save_hash}), facts, None)
    } else {
        let program = String::from_utf8(read(&directory.join("extractor.rhai"), 64 * 1024)?)
            .map_err(|_| invalid())?;
        let scratch = tempfile::tempdir().map_err(io)?;
        let inputs = resources::save_inputs(&paths[..1], home, scratch.path())?;
        if inputs[0]["sha256"].as_str() != Some(&save_hash) {
            return Err(Message::new("summary.save_changed"));
        }
        let facts = sandbox::run(&program, &inputs[0])?;
        if !grounded(&facts, &inputs[0]) {
            return Err(Message::new("summary.invalid_program"));
        }
        let primary = inputs[0]["strings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["section"] == "title")
            .filter_map(|s| s["text"].as_str())
            .rfind(|s| !matches!(*s, "SaveDataTitle" | "Time"))
            .map(str::to_owned);
        (
            inputs.into_iter().next().ok_or_else(invalid)?,
            facts,
            primary,
        )
    };
    let dialogue = script_context(&directory, &facts, primary.as_deref())?;
    let background: Background =
        serde_json::from_slice(&read(&directory.join("background.json"), 1024 * 1024)?)
            .map_err(|_| invalid())?;
    let serialized_context = serde_json::to_vec(&dialogue).map_err(io)?;
    let current = json!({
        "file":input["file"],"sha256":save_hash,"facts":facts,
        "summary_evidence":{
            "kind":if facts.history.is_empty() { "script_candidates" } else { "save_recorded_history" },
            "history_verified":!facts.history.is_empty(),
            "rows":dialogue.len(),"serialized_bytes":serialized_context.len(),
            "sha256":format!("{:x}",Sha256::digest(&serialized_context)),
            "source":dialogue.last().map(|row| &row.source),
            "current_position":if facts.resume.is_some() { None } else { dialogue.last().map(|row| row.position) },
            "save_state_role":if facts.resume.is_some() { "resume_node" } else { "current_node" },
            "resume":facts.resume
        }
    });
    let (source, model, token) = provider(settings, config)?;
    // Metadata, when provided, lets larger-context models use fewer analysis
    // passes. One UTF-8 byte per token is a conservative upper bound.
    let input_budget = ai::list_models(&source, &token)
        .ok()
        .and_then(|models| models.into_iter().find(|m| m.id == model))
        .and_then(|m| m.context_window)
        .map(|tokens| tokens.saturating_sub(16_384) as usize)
        .unwrap_or(100_000)
        .clamp(32_000, 1_000_000);
    let content = report::generate(
        report::Evidence {
            game: &game.title,
            language: settings.language,
            script_language: &m.script_language,
            save_hash: &save_hash,
            dialogue: &dialogue,
            input_budget,
            scene: &facts.scene,
            history_verified: !facts.history.is_empty(),
            resume: facts.resume.as_ref(),
        },
        |system, input| complete_part(&directory, &source, &token, &model, system, input),
    )?;
    let mut warnings = facts.warnings;
    if facts.resume.is_some() {
        warnings.push(match settings.language {
            Language::Chinese => "存档只确认恢复入口，未保存最后显示的台词；入口前的脚本内容是候选前情，不代表已读历史。",
            Language::Japanese => "確認済みなのは再開位置です。最後の表示台詞は保存されておらず、再開位置より前の候補文を読了履歴とは見なしません。",
            Language::English => "The save verifies a restore entry, not the last displayed line. Earlier script candidates are not verified reading history.",
        }.into());
    }
    if facts.history.is_empty() {
        warnings.push(galgear_i18n::tr(settings.language, "summary.limited_history").into());
    }
    warnings.push(galgear_i18n::tr(settings.language, "summary.latest_note").into());
    if m.script_language != settings.language.code() {
        warnings.push(galgear_i18n::tr(settings.language, "summary.original_language").into());
    }
    let result = Summary {
        game_id: game.id.clone(),
        content,
        provider: source,
        model,
        generated_at: now(),
        save_file: crate::config::display_path(&paths[0]),
        source_url: background.url,
        script_language: m.script_language,
        warnings,
        cached: false,
        save_hash,
    };
    let summaries = directory.join("summaries");
    fs::create_dir_all(&summaries).map_err(io)?;
    write_json(
        &summaries.join(format!("{key}.json")),
        &CachedSummary {
            schema: REPORT_SCHEMA,
            key,
            save_content: current.clone(),
            summary: result.clone(),
        },
    )?;
    write_json(&directory.join("latest-save.json"), &current)?;
    write_json(&directory.join("latest-summary.json"), &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_save_fixture() -> (tempfile::TempDir, Game, Settings, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let save = root.path().join("chosen");
        fs::create_dir(&save).unwrap();
        let game =
            crate::catalog::manual_game(&root.path().join("Novel.exe"), "A different game".into())
                .unwrap();
        let mut settings = Settings::default();
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .save_path = Some(crate::config::display_path(&save));
        (root, game, settings, save)
    }

    fn native_file(path: &Path, text: &str, seconds: u64) {
        let raw = text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut file = fs::File::create(path).unwrap();
        use std::io::Write;
        file.write_all(&[0xff, 0xfe]).unwrap();
        file.write_all(&raw).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    }

    #[test]
    fn preparation_selects_native_ksd_progress_over_newer_global_states() {
        let (_root, game, settings, save) = selected_save_fixture();
        native_file(
            &save.join("chapter03.ksd"),
            r#"(const) %["currentStorage" => "forest.ks", "historyData" => (const) %["data" => (const) ["a native record"]]]"#,
            10,
        );
        native_file(
            &save.join("preferences.ksd"),
            r#"(const) %["fullScreen" => 1, "windowWidth" => 1280]"#,
            20,
        );
        native_file(
            &save.join("completion.ksd"),
            r#"(const) %["trail_forest" => 1, "cg_portrait" => 1]"#,
            30,
        );
        let inputs = preparation_save_inputs(&game, &settings).unwrap();
        assert_eq!(
            inputs.required_saves,
            vec![crate::config::display_path(
                &save.join("chapter03.ksd").canonicalize().unwrap()
            )]
        );
        let files = save_files(&game, &settings).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "chapter03.ksd");
    }

    #[test]
    fn unknown_newer_file_is_retained_without_forcing_or_downgrading_the_target() {
        let (_root, game, settings, save) = selected_save_fixture();
        native_file(
            &save.join("old.ksd"),
            r#"(const) %["currentStorage" => "town.ks", "historyData" => (const) %["data" => (const) ["a native record"]]]"#,
            10,
        );
        fs::write(save.join("new.sav"), b"unrecognized engine state").unwrap();
        let inputs = preparation_save_inputs(&game, &settings).unwrap();
        assert!(inputs.required_saves.is_empty());
        assert_eq!(inputs.candidates[0].kind, saves::SaveKind::Unknown);
    }

    #[test]
    fn unrecognized_suffix_and_controls_only_folders_still_reach_agent_inspection() {
        let (_root, game, settings, save) = selected_save_fixture();
        fs::write(save.join("slot.custom"), b"custom bytes").unwrap();
        let unknown = preparation_save_inputs(&game, &settings).unwrap();
        assert!(unknown.required_saves.is_empty());
        assert_eq!(
            Path::new(&unknown.save).canonicalize().unwrap(),
            save.canonicalize().unwrap()
        );
        fs::remove_file(save.join("slot.custom")).unwrap();
        native_file(
            &save.join("settings.ksd"),
            r#"(const) %["fullScreen" => 1, "windowWidth" => 1280]"#,
            20,
        );
        let controls = preparation_save_inputs(&game, &settings).unwrap();
        assert!(controls.required_saves.is_empty());
        assert_eq!(controls.candidates[0].kind, saves::SaveKind::Control);
    }

    fn raw_file(path: &Path, bytes: &[u8], seconds: u64) {
        use std::io::Write;
        let mut file = fs::File::create(path).unwrap();
        file.write_all(bytes).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    }

    fn save_report(locations: &[(&Path, &str)]) -> saves::SaveReport {
        saves::SaveReport {
            locations: locations
                .iter()
                .map(|(path, source)| saves::SaveLocation {
                    path: crate::config::display_path(path),
                    source: (*source).into(),
                    confidence: "configured".into(),
                    exists: true,
                    readable: true,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn cached_extensions_do_not_hide_a_newer_native_ksd_progress_save() {
        let (_root, game, settings, save) = selected_save_fixture();
        let old = save.join("old_slot.bmp");
        native_file(
            &old,
            r#"(const) %["currentStorage" => "forest.ks", "historyData" => (const) %["data" => (const) ["earlier native history"]]]"#,
            10,
        );
        let payload = fs::read(&old).unwrap();
        let mut bmp = vec![0; 54];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&payload);
        raw_file(&old, &bmp, 10);
        native_file(
            &save.join("new_slot.ksd"),
            r#"(const) %["currentStorage" => "town.ks", "historyData" => (const) %["data" => (const) ["new native history"]]]"#,
            20,
        );
        let extensions = vec!["bmp".into()];
        let paths = save_files_with_extensions(&game, &settings, Some(&extensions)).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].file_name().unwrap(), "new_slot.ksd");
        assert_eq!(saves::classify(&paths[0]), saves::SaveKind::Progress);
    }

    #[test]
    fn cached_extensions_do_not_promote_new_unknown_or_control_files_to_progress() {
        let (_root, game, settings, save) = selected_save_fixture();
        native_file(
            &save.join("slot.ksd"),
            r#"(const) %["currentStorage" => "forest.ks", "historyData" => (const) %["data" => (const) ["native history"]]]"#,
            10,
        );
        raw_file(&save.join("slot.custom"), b"newer unknown opaque state", 20);
        raw_file(&save.join("slot.log"), b"newer ordinary log", 30);
        native_file(
            &save.join("options.control"),
            r#"(const) %["fullScreen" => 1, "windowWidth" => 1280]"#,
            40,
        );
        let extensions = vec!["ksd".into()];
        let paths = save_files_with_extensions(&game, &settings, Some(&extensions)).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "slot.ksd");
    }

    #[test]
    fn new_opaque_slot_requires_discovery_but_newer_logs_and_images_do_not() {
        let (_root, game, settings, save) = selected_save_fixture();
        native_file(
            &save.join("chapter01.ksd"),
            r#"(const) %["currentStorage" => "forest.ks", "historyData" => (const) %["data" => (const) ["native history"]]]"#,
            10,
        );
        raw_file(&save.join("slot.custom"), b"unsupported native state", 20);
        raw_file(&save.join("slot.log"), b"diagnostic log", 30);
        raw_file(&save.join("slot.jpg"), b"thumbnail", 40);
        let inputs = preparation_save_inputs(&game, &settings).unwrap();
        assert!(inputs.required_saves.is_empty());
        assert_eq!(inputs.candidates.len(), 2);
        assert!(inputs.candidates[0].path.ends_with("slot.custom"));
        assert_eq!(inputs.candidates[0].kind, saves::SaveKind::Unknown);
    }

    #[test]
    fn native_magic_and_observed_slot_families_keep_unfamiliar_suffixes_discoverable() {
        let (_root, _game, _settings, save) = selected_save_fixture();
        native_file(
            &save.join("chapter02.ksd"),
            r#"(const) %["currentStorage" => "forest.ks", "historyData" => (const) %["data" => (const) ["native history"]]]"#,
            20,
        );
        raw_file(
            &save.join("chapter01.opaque"),
            b"unsupported older slot",
            10,
        );
        raw_file(
            &save.join("mystery.log"),
            &[0xfe, 0xfe, 0, 0xff, 0xfe, 1, 2],
            30,
        );
        raw_file(&save.join("thumbnail.jpg"), b"ordinary image", 40);
        let candidates = save_candidates_in_report(&save_report(&[(&save, "game")]), None).unwrap();
        assert_eq!(candidates.len(), 3);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.path.ends_with("chapter01.opaque"))
        );
        assert!(candidates[0].path.ends_with("mystery.log"));
    }

    #[test]
    fn direct_kag_conductor_with_options_is_unknown_and_not_removed_from_ksd_only_saves() {
        let (_root, game, settings, save) = selected_save_fixture();
        native_file(
            &save.join("slot.ksd"),
            r#"(const) %["storageName" => "chapter.ks", "curLabel" => "*checkpoint", "curLine" => 12, "fullScreen" => 1, "windowWidth" => 1280]"#,
            10,
        );
        let inputs = preparation_save_inputs(&game, &settings).unwrap();
        assert_eq!(inputs.candidates.len(), 1);
        assert_eq!(inputs.candidates[0].kind, saves::SaveKind::Unknown);
        assert!(inputs.required_saves.is_empty());
        assert_eq!(save_files(&game, &settings).unwrap().len(), 1);
    }

    #[test]
    fn weak_willplus_location_cannot_delete_an_authoritative_opaque_or_ksd_candidate() {
        let root = tempfile::tempdir().unwrap();
        let explicit = root.path().join("configured");
        let guessed = root.path().join("guessed");
        fs::create_dir(&explicit).unwrap();
        fs::create_dir(&guessed).unwrap();
        raw_file(&explicit.join("slot.custom"), b"new unsupported state", 20);
        raw_file(
            &guessed.join("Save001.OtherSave-WillPlus"),
            b"older different slot family",
            10,
        );
        let report = save_report(&[(&explicit, "config"), (&guessed, "roaming")]);
        let opaque = save_candidates_in_report(&report, None).unwrap();
        assert_eq!(opaque.len(), 2);
        let paths = selected_save_paths(&opaque);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "slot.custom");
        native_file(
            &explicit.join("chapter.ksd"),
            r#"(const) %["currentStorage" => "chapter.ks", "historyData" => (const) %["data" => (const) ["native history"]]]"#,
            30,
        );
        let native = save_candidates_in_report(&report, None).unwrap();
        let paths = selected_save_paths(&native);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].file_name().unwrap(), "chapter.ksd");
    }

    #[test]
    fn explicit_appid_location_wins_over_a_newer_name_matched_initialization() {
        let root = tempfile::tempdir().unwrap();
        let named = root.path().join("Publisher/Localized Novel");
        let configured = root.path().join("Emulator/424242/remote");
        fs::create_dir_all(&named).unwrap();
        fs::create_dir_all(&configured).unwrap();
        let initialization = named.join("anchor.ksd");
        let story = configured.join("slot.ksd");
        native_file(
            &initialization,
            r#"(const) %["currentStorage" => "menu.ks", "mainConductor" => (const) %["pos" => 7]]"#,
            30,
        );
        native_file(
            &story,
            r#"(const) %["currentStorage" => "forest.ks", "historyData" => (const) %["data" => (const) ["native history"]]]"#,
            10,
        );
        let candidates = vec![
            SaveCandidate {
                path: crate::config::display_path(&initialization),
                kind: saves::classify(&initialization),
                modified: 30,
                source: "roaming".into(),
            },
            SaveCandidate {
                path: crate::config::display_path(&story),
                kind: saves::classify(&story),
                modified: 10,
                source: "config".into(),
            },
        ];
        assert_eq!(candidates[0].kind, saves::SaveKind::Unknown);
        assert_eq!(preferred_save_root(&candidates), Some(configured));
    }

    #[test]
    fn unknown_configured_state_is_not_replaced_by_an_older_heuristic_story() {
        let candidates = vec![
            SaveCandidate {
                path: "configured/new.sav".into(),
                kind: saves::SaveKind::Unknown,
                modified: 30,
                source: "config".into(),
            },
            SaveCandidate {
                path: "guessed/old.ksd".into(),
                kind: saves::SaveKind::Progress,
                modified: 10,
                source: "roaming".into(),
            },
        ];
        assert_eq!(
            preferred_save_root(&candidates),
            Some(PathBuf::from("configured"))
        );
    }

    #[test]
    fn custom_progress_suffix_excludes_same_folder_settings_files() {
        let root = tempfile::tempdir().unwrap();
        let save = root.path().join("chosen");
        fs::create_dir(&save).unwrap();
        fs::write(save.join("Save001.ProductSave-WillPlus"), b"progress").unwrap();
        fs::write(save.join("Save000.dat"), b"settings").unwrap();
        fs::write(save.join("Save001.png"), b"thumbnail").unwrap();
        let game =
            crate::catalog::manual_game(&root.path().join("game.exe"), "Game".into()).unwrap();
        let mut settings = Settings::default();
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .save_path = Some(crate::config::display_path(&save));
        let files = save_files(&game, &settings).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().unwrap(),
            "Save001.ProductSave-WillPlus"
        );
        fs::write(save.join("slot01.unknown_engine"), b"raw progress").unwrap();
        let extensions = vec!["unknown_engine".into()];
        let files = save_files_with_extensions(&game, &settings, Some(&extensions)).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "slot01.unknown_engine");
    }

    fn cached_fixture() -> (tempfile::TempDir, Game, Settings, Manifest) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("game.exe"), b"game").unwrap();
        let game =
            crate::catalog::manual_game(&root.path().join("game.exe"), "Game".into()).unwrap();
        let mut settings = Settings::default();
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .ai_summary = true;
        let directory = cache(&game, settings.language);
        fs::create_dir_all(directory.join("summaries")).unwrap();
        // A cache hit must work without invoking even a valid parser or API key.
        fs::write(
            directory.join("extractor.rhai"),
            "fn extract(data) { throw \"must not run\"; }",
        )
        .unwrap();
        fs::write(
            directory.join("dialogue.jsonl"),
            "{\"source\":\"chapter\",\"position\":1,\"text\":\"Current dialogue\"}\n",
        )
        .unwrap();
        write_json(
            &directory.join("background.json"),
            &Background {
                title: "Game".into(),
                source: "bangumi".into(),
                url: "https://bangumi.tv/subject/1".into(),
                description: "Background".into(),
            },
        )
        .unwrap();
        let manifest = Manifest {
            schema: SCHEMA,
            game_id: game.id.clone(),
            language: settings.language,
            script_language: settings.language.code().into(),
            sources: Vec::new(),
            inventory: Vec::new(),
            program_hash: hash(&directory.join("extractor.rhai")).unwrap(),
            dialogue_hash: hash(&directory.join("dialogue.jsonl")).unwrap(),
            background_hash: hash(&directory.join("background.json")).unwrap(),
            dialogue_count: 1,
            python_agent: false,
            script_program_hash: None,
            save_extensions: Vec::new(),
            speaker_verification_version: 0,
            save_verification_version: 0,
            script_verification_version: 0,
        };
        write_json(&directory.join("manifest.json"), &manifest).unwrap();
        (root, game, settings, manifest)
    }

    fn python_cached_fixture() -> (tempfile::TempDir, Game, Settings, Manifest) {
        let (root, game, settings, mut cached) = cached_fixture();
        let directory = cache(&game, settings.language);
        for name in ["save_extractor.py", "script_extractor.py"] {
            fs::write(directory.join(name), "must never execute").unwrap();
        }
        fs::write(
            directory.join("dialogue.jsonl"),
            "{\"source\":\"chapter\",\"position\":1,\"text\":\"Current dialogue\",\"speaker\":null}\n",
        )
        .unwrap();
        cached.python_agent = true;
        cached.program_hash = hash(&directory.join("save_extractor.py")).unwrap();
        cached.script_program_hash = Some(hash(&directory.join("script_extractor.py")).unwrap());
        cached.dialogue_hash = hash(&directory.join("dialogue.jsonl")).unwrap();
        cached.speaker_verification_version = 1;
        cached.save_verification_version = agent::SAVE_VERIFICATION_VERSION;
        cached.script_verification_version = agent::SCRIPT_VERIFICATION_VERSION;
        write_json(&directory.join("manifest.json"), &cached).unwrap();
        (root, game, settings, cached)
    }

    fn seed_python_summary(root: &Path, game: &Game, settings: &Settings, manifest: &Manifest) {
        fs::create_dir(root.join("savedata")).unwrap();
        let save = root.join("savedata/save.json");
        fs::write(&save, b"synthetic unchanged save").unwrap();
        let save_hash = hash(&save).unwrap();
        let key = summary_key(manifest, &save_hash, settings).unwrap();
        let summary = Summary {
            game_id: game.id.clone(),
            content: report::Content {
                recap: "Saved recap".into(),
                current_progress: "Saved current node".into(),
            },
            provider: "deepseek".into(),
            model: "offline-model".into(),
            generated_at: 42,
            save_file: crate::config::display_path(&save),
            source_url: "https://bangumi.tv/subject/1".into(),
            script_language: "zh-CN".into(),
            warnings: Vec::new(),
            cached: false,
            save_hash,
        };
        write_json(
            &cache(game, settings.language)
                .join("summaries")
                .join(format!("{key}.json")),
            &CachedSummary {
                schema: REPORT_SCHEMA,
                key,
                save_content: json!({"facts":{"source":"chapter","line":1}}),
                summary,
            },
        )
        .unwrap();
    }

    #[cfg(windows)]
    fn forward_slash_python_cached_fixture() -> (tempfile::TempDir, Game, Settings) {
        let (root, mut game, settings, mut cached) = python_cached_fixture();
        fs::write(root.path().join("scenario.arc"), b"synthetic archive").unwrap();
        cached.inventory =
            resources::source_inventory(&root.path().canonicalize().unwrap()).unwrap();
        assert_eq!(cached.inventory.len(), 1);
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &cached,
        )
        .unwrap();
        game.folder = crate::config::display_path(root.path()).replace('\\', "/");
        game.executable = Some(format!("{}/game.exe", game.folder));
        (root, game, settings)
    }

    #[cfg(windows)]
    #[test]
    fn forward_slash_folder_reuses_preparation_with_nonempty_inventory() {
        let (root, game, settings) = forward_slash_python_cached_fixture();
        let setup = prepare(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap();
        assert!(setup.reused);
    }

    #[cfg(windows)]
    #[test]
    fn forward_slash_folder_reuses_verified_summary_offline() {
        let (root, game, settings) = forward_slash_python_cached_fixture();
        let cached: Manifest = serde_json::from_slice(
            &fs::read(cache(&game, settings.language).join("manifest.json")).unwrap(),
        )
        .unwrap();
        seed_python_summary(root.path(), &game, &settings, &cached);
        let summary = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap();
        assert!(summary.cached);
        assert_eq!(summary.generated_at, 42);
    }

    #[cfg(windows)]
    #[test]
    fn forward_slash_folder_still_invalidates_changed_source_inventory() {
        let (root, game, settings) = forward_slash_python_cached_fixture();
        assert!(manifest(&game, settings.language).is_ok());
        fs::write(
            root.path().join("scenario.arc"),
            b"changed synthetic archive contents",
        )
        .unwrap();
        assert!(manifest(&game, settings.language).is_err());
    }

    #[test]
    fn old_python_manifest_cannot_reuse_a_cached_summary_without_save_verification() {
        let (root, game, settings, mut old) = python_cached_fixture();
        old.save_verification_version = 0;
        seed_python_summary(root.path(), &game, &settings, &old);
        let mut value = serde_json::to_value(&old).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("save_verification_version");
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &value,
        )
        .unwrap();
        let decoded: Manifest = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.save_verification_version, 0);
        let error = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap_err();
        assert_eq!(error.key, "summary.setup_required");
    }

    #[test]
    fn old_python_manifest_cannot_reuse_a_cached_summary_without_script_verification() {
        let (root, game, settings, mut old) = python_cached_fixture();
        old.script_verification_version = 0;
        seed_python_summary(root.path(), &game, &settings, &old);
        let mut value = serde_json::to_value(&old).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("script_verification_version");
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &value,
        )
        .unwrap();
        let decoded: Manifest = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.script_verification_version, 0);
        assert_eq!(
            decoded.save_verification_version,
            agent::SAVE_VERIFICATION_VERSION
        );
        let error = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap_err();
        assert_eq!(error.key, "summary.setup_required");
    }

    #[test]
    fn old_python_manifest_requires_preparation_without_script_verification() {
        let (root, game, mut settings, old) = python_cached_fixture();
        let mut value = serde_json::to_value(&old).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("script_verification_version");
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &value,
        )
        .unwrap();
        settings.ai_provider = "unavailable-provider".into();
        let config = root.path().join("no-config.json");
        let expected = provider(&settings, &config).unwrap_err();
        let error = prepare(&game, &settings, &config, Path::new("missing-tools")).unwrap_err();
        assert_eq!(error.key, expected.key);
    }

    #[test]
    fn current_python_manifest_keeps_verified_summary_reuse_offline() {
        let (root, game, settings, current) = python_cached_fixture();
        seed_python_summary(root.path(), &game, &settings, &current);
        let summary = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap();
        assert!(summary.cached);
        assert_eq!(summary.generated_at, 42);
        assert_eq!(
            manifest(&game, settings.language)
                .unwrap()
                .save_verification_version,
            agent::SAVE_VERIFICATION_VERSION
        );
        assert_eq!(
            manifest(&game, settings.language)
                .unwrap()
                .script_verification_version,
            agent::SCRIPT_VERIFICATION_VERSION
        );
    }

    #[test]
    fn unsupported_python_script_verification_version_requires_preparation() {
        let (_root, game, settings, mut future) = python_cached_fixture();
        future.script_verification_version = agent::SCRIPT_VERIFICATION_VERSION + 1;
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &future,
        )
        .unwrap();
        assert!(manifest(&game, settings.language).is_err());
    }

    #[test]
    fn former_script_verification_caches_cannot_bypass_current_script_policy() {
        for version in [1, 2] {
            let (root, game, settings, mut old) = python_cached_fixture();
            old.script_verification_version = version;
            seed_python_summary(root.path(), &game, &settings, &old);
            write_json(&cache(&game, settings.language).join("manifest.json"), &old).unwrap();
            assert!(manifest(&game, settings.language).is_err());
            let error = summarize(
                &game,
                &settings,
                &root.path().join("unused-settings.json"),
                Path::new("unused-tools"),
            )
            .unwrap_err();
            assert_eq!(
                error.key, "summary.setup_required",
                "script policy {version}"
            );
        }
    }

    #[test]
    fn unsupported_python_save_verification_version_requires_preparation() {
        let (_root, game, settings, mut future) = python_cached_fixture();
        future.save_verification_version = agent::SAVE_VERIFICATION_VERSION + 1;
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &future,
        )
        .unwrap();
        assert!(manifest(&game, settings.language).is_err());
    }

    #[test]
    fn former_save_verification_caches_cannot_bypass_current_role_policy() {
        for version in [1, 2] {
            let (root, game, mut settings, mut old) = python_cached_fixture();
            old.save_verification_version = version;
            seed_python_summary(root.path(), &game, &settings, &old);
            write_json(&cache(&game, settings.language).join("manifest.json"), &old).unwrap();
            assert!(manifest(&game, settings.language).is_err());
            let config = root.path().join("unused-settings.json");
            let error =
                summarize(&game, &settings, &config, Path::new("unused-tools")).unwrap_err();
            assert_eq!(error.key, "summary.setup_required", "save policy {version}");
            settings.ai_provider = "unavailable-provider".into();
            let expected = provider(&settings, &config).unwrap_err();
            let error = prepare(&game, &settings, &config, Path::new("unused-tools")).unwrap_err();
            assert_eq!(error.key, expected.key, "save policy {version}");
        }
    }

    #[test]
    fn save_verification_version_changes_python_summary_key_only() {
        let (_root, _game, settings, mut value) = cached_fixture();
        let legacy = summary_key(&value, "unchanged save", &settings).unwrap();
        value.save_verification_version = agent::SAVE_VERIFICATION_VERSION;
        assert_eq!(
            legacy,
            summary_key(&value, "unchanged save", &settings).unwrap()
        );
        value.python_agent = true;
        let current = summary_key(&value, "unchanged save", &settings).unwrap();
        for version in [0, 1, 2] {
            value.save_verification_version = version;
            assert_ne!(
                current,
                summary_key(&value, "unchanged save", &settings).unwrap(),
                "save policy {version}"
            );
        }
    }

    #[test]
    fn script_verification_version_changes_python_summary_key_only() {
        let (_root, _game, settings, mut value) = cached_fixture();
        let legacy = summary_key(&value, "unchanged save", &settings).unwrap();
        value.script_verification_version = agent::SCRIPT_VERIFICATION_VERSION;
        assert_eq!(
            legacy,
            summary_key(&value, "unchanged save", &settings).unwrap()
        );
        value.python_agent = true;
        let current = summary_key(&value, "unchanged save", &settings).unwrap();
        for version in [0, 1, 2] {
            value.script_verification_version = version;
            assert_ne!(
                current,
                summary_key(&value, "unchanged save", &settings).unwrap(),
                "script policy {version}"
            );
        }
    }

    #[test]
    fn legacy_non_python_manifest_without_verification_versions_keeps_offline_preparation_reuse() {
        let (root, game, settings, legacy) = cached_fixture();
        let mut value = serde_json::to_value(&legacy).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("save_verification_version");
        fields.remove("script_verification_version");
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &value,
        )
        .unwrap();
        assert!(
            prepare(
                &game,
                &settings,
                &root.path().join("no-config.json"),
                Path::new("missing-tools")
            )
            .unwrap()
            .reused
        );
    }

    #[test]
    fn legacy_non_python_manifest_without_verification_versions_keeps_offline_summary_reuse() {
        let (root, game, settings, legacy) = cached_fixture();
        seed_python_summary(root.path(), &game, &settings, &legacy);
        let mut value = serde_json::to_value(&legacy).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("save_verification_version");
        fields.remove("script_verification_version");
        write_json(
            &cache(&game, settings.language).join("manifest.json"),
            &value,
        )
        .unwrap();
        let summary = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap();
        assert!(summary.cached);
        assert_eq!(summary.generated_at, 42);
    }

    #[test]
    fn reenable_reuses_preparation_without_credentials_tools_or_saves() {
        let (root, game, settings, _) = cached_fixture();
        let setup = prepare(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap();
        assert!(setup.reused);
        assert_eq!(setup.dialogue_count, 1);
    }

    #[test]
    fn missing_history_requires_the_selected_ai_provider_instead_of_local_placeholder_success() {
        let (root, game, mut settings, mut manifest) = cached_fixture();
        settings.ai_provider = "unavailable-provider".into();
        assert!(provider(&settings, &root.path().join("no-config.json")).is_err());
        let directory = cache(&game, settings.language);
        let program =
            r#"fn extract(data) { #{ scene: "Confirmed scene", anchors: ["Current dialogue"] } }"#;
        fs::write(directory.join("extractor.rhai"), program).unwrap();
        manifest.program_hash = hash(&directory.join("extractor.rhai")).unwrap();
        write_json(&directory.join("manifest.json"), &manifest).unwrap();
        fs::create_dir(root.path().join("savedata")).unwrap();
        fs::write(root.path().join("savedata/save.json"), b"Current dialogue").unwrap();
        let error = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap_err();
        assert_eq!(error.key, "ai.invalid_provider");
        assert!(!directory.join("analysis").exists());
        assert!(!directory.join("latest-summary.json").exists());
    }

    #[test]
    fn same_content_reuses_saved_report_but_changed_content_requires_new_analysis() {
        let (root, game, settings, manifest) = cached_fixture();
        fs::create_dir(root.path().join("savedata")).unwrap();
        let save = root.path().join("savedata/save.json");
        fs::write(&save, br#"{"chapter":1}"#).unwrap();
        let save_hash = hash(&save).unwrap();
        let key = summary_key(&manifest, &save_hash, &settings).unwrap();
        let summary = Summary {
            game_id: game.id.clone(),
            content: report::Content {
                recap: "Saved recap".into(),
                current_progress: "Chapter one".into(),
            },
            provider: "deepseek".into(),
            model: "offline-model".into(),
            generated_at: 42,
            save_file: crate::config::display_path(&save),
            source_url: "https://bangumi.tv/subject/1".into(),
            script_language: "zh-CN".into(),
            warnings: Vec::new(),
            cached: false,
            save_hash,
        };
        let directory = cache(&game, settings.language);
        write_json(
            &directory.join("summaries").join(format!("{key}.json")),
            &CachedSummary {
                schema: REPORT_SCHEMA,
                key,
                save_content: json!({"facts":{"scene":"one"}}),
                summary,
            },
        )
        .unwrap();
        let result = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap();
        assert!(result.cached);
        assert_eq!(result.content.recap, "Saved recap");
        assert_eq!(result.generated_at, 42);
        // Same path, size and timestamp; content hashing must still invalidate it.
        let modified = fs::metadata(&save).unwrap().modified().unwrap();
        fs::write(&save, br#"{"chapter":2}"#).unwrap();
        fs::File::options()
            .write(true)
            .open(&save)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let error = summarize(
            &game,
            &settings,
            &root.path().join("no-config.json"),
            Path::new("missing-tools"),
        )
        .unwrap_err();
        assert_eq!(error.key, "summary.invalid_program");
    }
    #[test]
    fn script_context_stops_at_save_anchor() {
        let temp = tempfile::tempdir().unwrap();
        let lines = [
            Dialogue {
                source: "chapter".into(),
                position: 0,
                text: "Before".into(),
                speaker: Some("Alice".into()),
            },
            Dialogue {
                source: "chapter".into(),
                position: 1,
                text: "Current anchor".into(),
                speaker: None,
            },
            Dialogue {
                source: "chapter".into(),
                position: 2,
                text: "Future spoiler".into(),
                speaker: Some("Future identity".into()),
            },
        ];
        let text = lines
            .iter()
            .map(|d| serde_json::to_string(d).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(temp.path().join("dialogue.jsonl"), text).unwrap();
        let context = script_context(
            temp.path(),
            &sandbox::Facts {
                anchors: vec!["Current anchor".into()],
                history: vec![
                    sandbox::NodeRef {
                        source: "chapter".into(),
                        position: 0,
                    },
                    sandbox::NodeRef {
                        source: "chapter".into(),
                        position: 1,
                    },
                ],
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(
            context.iter().map(|d| d.text.as_str()).collect::<Vec<_>>(),
            ["Before", "Current anchor"]
        );
        assert_eq!(context[0].speaker.as_deref(), Some("Alice"));
        report::generate(
            report::Evidence {
                game: "Title",
                language: Language::English,
                script_language: "en-US",
                save_hash: "hash",
                dialogue: &context,
                input_budget: 4096,
                scene: "Confirmed scene",
                history_verified: true,
                resume: None,
            },
            |_, input| {
                assert!(!input.contains("Future spoiler"));
                assert!(!input.contains("Future identity"));
                let value: Value = serde_json::from_str(input).unwrap();
                assert_eq!(value["verified_preceding_dialogue"][0]["speaker"], "Alice");
                Ok(r#"{"recap":"Recap","current_progress":"Scene"}"#.into())
            },
        )
        .unwrap();
    }

    fn transcript_fixture(rows: &[(&str, u64, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let text = rows
            .iter()
            .map(|(source, position, text)| {
                serde_json::to_string(&Dialogue {
                    source: (*source).into(),
                    position: *position,
                    text: (*text).into(),
                    speaker: None,
                })
                .unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.path().join("dialogue.jsonl"), text).unwrap();
        root
    }

    fn append_resume_markers(directory: &Path) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(directory.join("dialogue.jsonl"))
            .unwrap();
        if file.metadata().unwrap().len() > 0 {
            writeln!(file).unwrap();
        }
        for (position, text) in [
            (20, "*earlier|LABEL_CAPTION_IS_NOT_DIALOGUE"),
            (50, "*departure|ENTRY_CAPTION_IS_NOT_DIALOGUE"),
        ] {
            writeln!(file, "{}", json!({"source":"chapter.ks","position":position,"text":text,"speaker":null,
                "_native":[{"kind":"kag_label_resume","role":"resume_node","profile":"kag_parser_label_v1"}]})).unwrap();
        }
    }

    #[test]
    fn signed_resume_markers_are_excluded_from_candidates_history_and_report_story() {
        let root = transcript_fixture(&[
            ("chapter.ks", 10, "They discuss leaving town."),
            ("chapter.ks", 30, "They gather their luggage."),
            ("chapter.ks", 80, "Future scene"),
        ]);
        append_resume_markers(root.path());
        let mut facts: sandbox::Facts =
            serde_json::from_value(json!({"source":"chapter.ks","line":50,
            "anchors":[],"resume":{"profile":"kag_parser_label_v1","state_role":"resume_node",
                "source":"chapter.ks","label":"*departure","position":50}}))
            .unwrap();
        let candidates = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|row| row.position)
                .collect::<Vec<_>>(),
            [10, 30]
        );
        facts.history = vec![
            sandbox::NodeRef {
                source: "chapter.ks".into(),
                position: 20,
            },
            sandbox::NodeRef {
                source: "chapter.ks".into(),
                position: 10,
            },
            sandbox::NodeRef {
                source: "chapter.ks".into(),
                position: 30,
            },
            sandbox::NodeRef {
                source: "chapter.ks".into(),
                position: 50,
            },
        ];
        let history = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(
            history.iter().map(|row| row.position).collect::<Vec<_>>(),
            [10, 30]
        );
        report::generate(report::Evidence { game: "Synthetic story", language: Language::English,
            script_language: "en-US", save_hash: "hash", dialogue: &history, input_budget: 8192,
            scene: "Departure", history_verified: true, resume: facts.resume.as_ref() }, |_, input| {
                assert!(!input.contains("CAPTION_IS_NOT_DIALOGUE") && !input.contains("Future scene"));
                let value: Value = serde_json::from_str(input).unwrap();
                assert_eq!(value["verified_preceding_dialogue"].as_array().unwrap().len(), 2);
                Ok(r#"{"recap":"They discussed leaving town and gathered their luggage.","current_progress":"The save restores the departure entry; its displayed line is not recorded."}"#.into())
            }).unwrap();
    }

    #[test]
    fn marker_only_resume_context_cannot_generate_a_substantive_report() {
        let root = transcript_fixture(&[]);
        append_resume_markers(root.path());
        let facts: sandbox::Facts = serde_json::from_value(json!({"source":"chapter.ks","line":50,
            "anchors":[],"resume":{"profile":"kag_parser_label_v1","state_role":"resume_node",
                "source":"chapter.ks","label":"*departure","position":50},
            "history":[{"source":"chapter.ks","position":20}]}))
        .unwrap();
        let context = script_context(root.path(), &facts, None).unwrap();
        assert!(context.is_empty());
        let result = report::generate(
            report::Evidence {
                game: "Synthetic story",
                language: Language::English,
                script_language: "en-US",
                save_hash: "hash",
                dialogue: &context,
                input_budget: 8192,
                scene: "Departure",
                history_verified: false,
                resume: facts.resume.as_ref(),
            },
            |_, _| {
                panic!("structural label captions must never be sent as substantive story context")
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn verified_resume_uses_only_earlier_candidates_without_fabricating_an_anchor() {
        let root = transcript_fixture(&[
            ("chapter.ks", 10, "They prepare to leave town."),
            ("chapter.ks", 50, "Unreached entry text"),
            ("chapter.ks", 80, "Future event"),
        ]);
        let facts: sandbox::Facts = serde_json::from_value(json!({"source":"chapter.ks","line":50,
            "anchors":[],"resume":{"profile":"kag_parser_label_v1","state_role":"resume_node",
                "source":"chapter.ks","label":"*departure","position":50}}))
        .unwrap();
        let context = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(context.len(), 1);
        report::generate(report::Evidence { game: "Synthetic story", language: Language::English,
            script_language: "en-US", save_hash: "hash", dialogue: &context, input_budget: 8192,
            scene: "Departure", history_verified: false, resume: facts.resume.as_ref() }, |system, input| {
                let value: Value = serde_json::from_str(input).unwrap();
                assert_eq!(value["save_state_role"], "resume_node");
                assert_eq!(value["save_node"]["position"], 50);
                assert!(value["save_node"]["anchor"].is_null());
                assert!(value["save_node"]["speaker"].is_null());
                assert!(!input.contains("Future event") && !input.contains("Unreached entry text"));
                assert!(system.contains("NOT the last displayed line"));
                Ok(r#"{"recap":"They prepare to leave town, inferred from the script before the restore entry.","current_progress":"The save restores the departure entry; its displayed line is not recorded."}"#.into())
            }).unwrap();
    }

    #[test]
    fn resume_preserves_verified_history_order_without_treating_the_entry_as_a_read_line() {
        let root = transcript_fixture(&[
            ("chapter.ks", 10, "Earlier code"),
            ("other.ks", 100, "Read before return"),
        ]);
        let facts: sandbox::Facts = serde_json::from_value(json!({"source":"chapter.ks","line":50,"anchors":[],
            "resume":{"profile":"kag_parser_label_v1","state_role":"resume_node","source":"chapter.ks","label":"*return","position":50},
            "history":[{"source":"other.ks","position":100},{"source":"chapter.ks","position":10}]})).unwrap();
        let context = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(
            context
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>(),
            ["Read before return", "Earlier code"]
        );
    }

    #[test]
    fn repeated_anchors_require_a_unique_source_and_position() {
        let root = transcript_fixture(&[
            ("route-a", 10, "Repeated"),
            ("route-a", 20, "Future event"),
            ("route-a", 30, "Repeated"),
            ("route-b", 10, "Repeated"),
        ]);
        let mut facts = sandbox::Facts {
            anchors: vec!["Repeated".into()],
            ..Default::default()
        };
        assert_eq!(
            script_context(root.path(), &facts, None).unwrap_err().key,
            "summary.ambiguous_anchor"
        );
        facts.line = Some(10);
        assert_eq!(
            script_context(root.path(), &facts, None).unwrap_err().key,
            "summary.ambiguous_anchor"
        );
        facts.source = Some("route-a".into());
        let context = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].position, 10);
    }

    #[test]
    fn mismatched_save_coordinates_never_fall_back_to_another_anchor() {
        let root = transcript_fixture(&[("chapter", 10, "Current"), ("chapter", 20, "Future")]);
        for (source, position) in [("other", 10), ("chapter", 20), ("chapter", 11)] {
            let facts = sandbox::Facts {
                source: Some(source.into()),
                line: Some(position),
                anchors: vec!["Current".into(), "Future".into()],
                ..Default::default()
            };
            assert_eq!(
                script_context(root.path(), &facts, None).unwrap_err().key,
                "summary.no_match"
            );
        }
    }

    #[test]
    fn absent_history_supplies_same_source_script_candidates_without_later_code() {
        // Extraction order is deliberately unrelated to script or story order.
        let root = transcript_fixture(&[
            ("chapter", 300, "Future"),
            ("other-route", 1, "Other route spoiler"),
            ("chapter", 200, "Current"),
            ("chapter", 100, "Before"),
        ]);
        let facts = sandbox::Facts {
            anchors: vec!["Current".into()],
            ..Default::default()
        };
        let context = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(
            context.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["Before", "Current"]
        );
    }

    #[test]
    fn save_recorded_history_keeps_early_rows_beyond_sixty() {
        let rows: Vec<_> = (0..202)
            .map(|position| ("chapter", position, "Dialogue"))
            .collect();
        let root = transcript_fixture(&rows);
        let facts = sandbox::Facts {
            source: Some("chapter".into()),
            line: Some(200),
            history: (0..=200)
                .map(|position| sandbox::NodeRef {
                    source: "chapter".into(),
                    position,
                })
                .collect(),
            anchors: vec!["Dialogue".into()],
            ..Default::default()
        };
        let context = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(
            context.iter().map(|r| r.position).collect::<Vec<_>>(),
            (0..=200).collect::<Vec<_>>()
        );
        let mut seen = Vec::new();
        let mut calls = 0;
        report::generate(
            report::Evidence {
                game: "Title",
                language: Language::English,
                script_language: "en-US",
                save_hash: "hash",
                dialogue: &context,
                input_budget: 4096,
                scene: "Confirmed scene",
                history_verified: true,
                resume: None,
            },
            |_, input| {
                calls += 1;
                let value: Value = serde_json::from_str(input).unwrap();
                assert_eq!(value["save_node"]["position"], 200);
                if let Some(part) = value["preceding_dialogue_part"].as_array() {
                    seen.extend(part.iter().map(|r| r["position"].as_u64().unwrap()));
                    // Exercise JSON repair without ever expanding the evidence boundary.
                    if calls == 1 {
                        return Ok("invalid JSON".into());
                    }
                    Ok(r#"{"observations":[],"uncertainties":[]}"#.into())
                } else {
                    Ok(r#"{"recap":"Recap","current_progress":"Scene"}"#.into())
                }
            },
        )
        .unwrap();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, (0..=200).collect::<Vec<_>>());
    }

    #[test]
    fn conflicting_rows_at_one_position_are_rejected() {
        let root = transcript_fixture(&[
            ("chapter", 10, "Before"),
            ("chapter", 10, "Conflicting"),
            ("chapter", 20, "Current"),
        ]);
        let facts = sandbox::Facts {
            anchors: vec!["Current".into()],
            history: vec![
                sandbox::NodeRef {
                    source: "chapter".into(),
                    position: 10,
                },
                sandbox::NodeRef {
                    source: "chapter".into(),
                    position: 20,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            script_context(root.path(), &facts, None).unwrap_err().key,
            "summary.ambiguous_anchor"
        );
    }

    #[test]
    fn preparation_without_the_speaker_contract_requires_repreparation() {
        let (_root, game, settings, mut old) = cached_fixture();
        old.schema = SCHEMA - 1;
        write_json(&cache(&game, settings.language).join("manifest.json"), &old).unwrap();
        assert!(manifest(&game, settings.language).is_err());
    }

    #[test]
    fn legacy_unresolved_speakers_resume_review_while_verified_caches_stay_offline() {
        let (root, game, settings, mut cached) = cached_fixture();
        let directory = cache(&game, settings.language);
        for name in ["save_extractor.py", "script_extractor.py"] {
            fs::write(directory.join(name), "must never execute").unwrap();
        }
        cached.python_agent = true;
        cached.save_verification_version = agent::SAVE_VERIFICATION_VERSION;
        cached.script_verification_version = agent::SCRIPT_VERIFICATION_VERSION;
        cached.program_hash = hash(&directory.join("save_extractor.py")).unwrap();
        cached.script_program_hash = Some(hash(&directory.join("script_extractor.py")).unwrap());
        write_json(&directory.join("manifest.json"), &cached).unwrap();
        assert!(manifest(&game, settings.language).is_err());

        fs::write(
            directory.join("dialogue.jsonl"),
            "{\"source\":\"chapter\",\"position\":1,\"text\":\"Current dialogue\",\"speaker\":\"Alice\"}\n",
        )
        .unwrap();
        cached.dialogue_hash = hash(&directory.join("dialogue.jsonl")).unwrap();
        write_json(&directory.join("manifest.json"), &cached).unwrap();
        assert!(
            prepare(
                &game,
                &settings,
                &root.path().join("missing.json"),
                Path::new("missing-tools")
            )
            .unwrap()
            .reused
        );

        fs::write(
            directory.join("dialogue.jsonl"),
            "{\"source\":\"chapter\",\"position\":1,\"text\":\"Narration\",\"speaker\":null}\n",
        )
        .unwrap();
        cached.dialogue_hash = hash(&directory.join("dialogue.jsonl")).unwrap();
        cached.speaker_verification_version = 1;
        write_json(&directory.join("manifest.json"), &cached).unwrap();
        assert!(
            prepare(
                &game,
                &settings,
                &root.path().join("missing.json"),
                Path::new("missing-tools")
            )
            .unwrap()
            .reused
        );
    }

    #[test]
    fn unscoped_external_speaker_cache_must_be_repaired() {
        let root = tempfile::tempdir().unwrap();
        let result = root.path().join("result.json");
        write_json(
            &result,
            &json!({"speaker_verification":{"version":1,"external_rows":10}}),
        )
        .unwrap();
        assert!(!reusable_speakers(root.path(), 1).unwrap());
        write_json(
            &result,
            &json!({"speaker_verification":{"version":1,"external_rows":10,"scope_version":2}}),
        )
        .unwrap();
        assert!(reusable_speakers(root.path(), 1).unwrap());
        write_json(
            &result,
            &json!({"speaker_verification":{"version":1,"external_rows":0}}),
        )
        .unwrap();
        assert!(reusable_speakers(root.path(), 1).unwrap());
    }

    #[test]
    fn summary_cache_changes_with_provider_and_selected_model() {
        let (_root, _game, mut settings, manifest) = cached_fixture();
        let original = summary_key(&manifest, "same save", &settings).unwrap();
        settings
            .ai_models
            .insert(settings.ai_provider.clone(), "different-model".into());
        assert_ne!(
            original,
            summary_key(&manifest, "same save", &settings).unwrap()
        );
        let changed_model = summary_key(&manifest, "same save", &settings).unwrap();
        settings.ai_provider = "openai".into();
        assert_ne!(
            changed_model,
            summary_key(&manifest, "same save", &settings).unwrap()
        );
    }

    #[test]
    fn recorded_history_preserves_cross_scene_and_backward_code_positions() {
        let root = transcript_fixture(&[
            ("a", 800, "Read earlier"),
            ("b", 20, "Current"),
            ("b", 10, "Unverified prefix"),
        ]);
        let facts = sandbox::Facts {
            source: Some("b".into()),
            line: Some(20),
            anchors: vec!["Current".into()],
            history: vec![
                sandbox::NodeRef {
                    source: "a".into(),
                    position: 800,
                },
                sandbox::NodeRef {
                    source: "b".into(),
                    position: 20,
                },
            ],
            ..Default::default()
        };
        let rows = script_context(root.path(), &facts, None).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["Read earlier", "Current"]
        );
    }

    #[test]
    fn old_reports_cannot_bypass_the_new_spoiler_boundary() {
        let (_root, game, settings, manifest) = cached_fixture();
        let key = summary_key(&manifest, "save hash", &settings).unwrap();
        let legacy = json!({
            "schema": REPORT_SCHEMA - 1, "key": key, "save_content": {},
            "summary": {"game_id":game.id,"recap":"Old full-script recap","current_progress":"Scene",
                "provider":"test","model":"test","generated_at":1,"save_file":"slot.sav",
                "source_url":"https://vndb.org/v1","script_language":"en-US","warnings":[],
                "cached":false,"save_hash":"save hash"}
        });
        let directory = cache(&game, settings.language);
        write_json(
            &directory.join("summaries").join(format!("{key}.json")),
            &legacy,
        )
        .unwrap();
        assert!(cached_summary(&directory, &key, &game).is_none());
    }
    #[test]
    fn parser_fabrications_are_rejected() {
        assert!(!grounded(
            &sandbox::Facts {
                anchors: vec!["invented".into()],
                ..Default::default()
            },
            &json!({"strings":[{"text":"actual"}]})
        ));
    }

    #[test]
    fn current_dialogue_anchor_takes_precedence_over_the_scene_caption() {
        let temp = tempfile::tempdir().unwrap();
        let rows = ["Scene caption", "Before", "Current dialogue", "Future"];
        let transcript = rows
            .iter()
            .enumerate()
            .map(|(position, text)| {
                serde_json::to_string(&Dialogue {
                    source: "chapter".into(),
                    position: position as u64,
                    text: (*text).into(),
                    speaker: None,
                })
                .unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(temp.path().join("dialogue.jsonl"), transcript).unwrap();
        let facts = sandbox::Facts {
            anchors: vec!["Current dialogue".into(), "Scene caption".into()],
            ..Default::default()
        };
        let context = script_context(temp.path(), &facts, Some("Current dialogue")).unwrap();
        assert_eq!(context.last().unwrap().text, "Current dialogue");
        assert!(!context.iter().any(|d| d.text == "Future"));
        assert!(!grounded(
            &sandbox::Facts {
                anchors: vec!["Stale backlog".into()],
                ..Default::default()
            },
            &json!({"strings":[{"section":"title","text":"Current dialogue"},{"section":"context","text":"Stale backlog"}]})
        ));
    }
}
