//! Automatic, bounded analysis of AdvHD Lua/WS2 gallery records.
//! Game-specific flag numbers and executable hashes are not compiled in.
mod arc;
mod lua;
mod ws2;

use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub type AutoResult<T> = Result<T, Message>;
pub(super) fn failure(detail: impl ToString) -> Message {
    Message::detail("auto.unsupported", detail)
}
pub(super) fn io(detail: impl ToString) -> Message {
    Message::detail("auto.io", detail)
}
pub(super) struct Cursor<'a> {
    data: &'a [u8],
    pub position: usize,
}
impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    pub fn take(&mut self, n: usize) -> AutoResult<&'a [u8]> {
        let end = self
            .position
            .checked_add(n)
            .ok_or_else(|| failure("Data length overflow"))?;
        let v = self
            .data
            .get(self.position..end)
            .ok_or_else(|| failure("Truncated resource"))?;
        self.position = end;
        Ok(v)
    }
    pub fn byte(&mut self) -> AutoResult<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> AutoResult<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> AutoResult<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn integer(&mut self, n: usize) -> AutoResult<u64> {
        let v = self.take(n)?;
        let mut out = [0; 8];
        out.get_mut(..n)
            .ok_or_else(|| failure("Integer size"))?
            .copy_from_slice(v);
        Ok(u64::from_le_bytes(out))
    }
}
fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn small(path: &Path) -> AutoResult<Vec<u8>> {
    if fs::metadata(path).map_err(io)?.len() > 64 * 1024 * 1024 {
        return Err(failure("Resource exceeds 64 MiB"));
    }
    fs::read(path).map_err(io)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub flag: u16,
    pub evidence: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub status: String,
    pub engine: String,
    pub cg_groups: usize,
    pub cg_variants: usize,
    pub scenes: usize,
    pub reason: Option<Message>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub summary: Summary,
    pub archive: String,
    pub member: String,
    pub cg: Vec<Record>,
    pub variants: Vec<Record>,
    pub scene: Vec<Record>,
    pub input_hashes: BTreeMap<String, String>,
    pub save_candidates: Vec<String>,
    pub patched_sha256: String,
}
struct Plan {
    report: Analysis,
    replacement: Vec<u8>,
}
const BACKUP: &str = ".galgear-backups/auto-unlock";
const JOURNAL: &str = "manifest.json";

fn root(folder: &Path) -> AutoResult<PathBuf> {
    fs::canonicalize(folder).map_err(io)
}
fn inside(root: &Path, path: &Path) -> AutoResult<()> {
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| failure("Target ancestor"))?;
    }
    let resolved = fs::canonicalize(ancestor).map_err(io)?;
    if !within(&resolved, root) {
        return Err(io(format!(
            "Canonical game path check: {} / {}",
            resolved.display(),
            root.display()
        )));
    }
    Ok(())
}

fn within(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut components = path.components();
        root.components().all(|part| {
            components.next().is_some_and(|other| {
                part.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&other.as_os_str().to_string_lossy())
            })
        })
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}
fn save_roots(source: &Path) -> AutoResult<Vec<PathBuf>> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| failure("APPDATA"))?;
    let appdata = PathBuf::from(appdata);
    let mut roots = vec![fs::canonicalize(&appdata).map_err(io)?];
    let relative = source
        .strip_prefix(&appdata)
        .map_err(|_| failure("Logical save path is outside APPDATA"))?;
    let first = relative
        .components()
        .next()
        .ok_or_else(|| failure("Save vendor path"))?;
    if !matches!(first, std::path::Component::Normal(_)) {
        return Err(failure("Save vendor component"));
    }
    let vendor = appdata.join(first.as_os_str());
    let metadata = fs::symlink_metadata(&vendor).map_err(io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(failure("Save vendor is a link"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(failure("Save vendor is a reparse point"));
        }
    }
    // Resolve the normal, script-declared vendor directory itself. Packaged
    // hosts virtualize newly created Roaming children without redirecting the
    // pre-existing APPDATA root; no symlink or arbitrary package is accepted.
    roots.push(fs::canonicalize(vendor).map_err(io)?);
    Ok(roots)
}

fn named_member(path: &Path, name: &str) -> AutoResult<Vec<u8>> {
    let entries = arc::index(path)?;
    let e = entries
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| failure(format!("Missing {name}")))?;
    arc::read(path, e)
}
fn find<'a>(root: &'a lua::Function, name: &str) -> AutoResult<&'a lua::Function> {
    let mut functions = Vec::new();
    root.all(&mut functions);
    let candidates: Vec<_> = functions.into_iter().filter(|v| v.name == name).collect();
    if candidates.len() != 1 {
        return Err(failure(format!("Ambiguous or missing {name}")));
    }
    Ok(candidates[0])
}

fn plan(folder: &Path) -> AutoResult<Plan> {
    let root = root(folder)?;
    if root.join(BACKUP).join(JOURNAL).is_file() {
        return Err(Message::new("auto.already_installed"));
    }
    let mut libraries: Vec<_> = fs::read_dir(&root)
        .map_err(io)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|v| v.to_str()).is_some_and(|v| {
                v.to_lowercase().starts_with("script") && v.to_lowercase().ends_with(".arc")
            })
        })
        .collect();
    libraries.sort();
    let mut matching = Vec::new();
    for path in libraries {
        inside(&root, &path)?;
        let entries = arc::index(&path)?;
        if entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case("menu_gallery.lua"))
        {
            matching.push((path, entries));
        }
    }
    if matching.len() != 1 {
        return Err(failure("Expected one AdvHD gallery script archive"));
    }
    let (archive, entries) = matching.pop().unwrap();
    let member = entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case("menu_gallery.lua"))
        .unwrap();
    let original = small(&archive)?;
    let menu = arc::read(&archive, member)?;
    let chunk = lua::parse(&menu)?;
    if !lua::setter_is_used(&chunk.root) {
        return Err(failure(
            "Native SetFlag signature is not proven by the original menu",
        ));
    }
    let cg_getter = find(&chunk.root, "GetCgFlag")?;
    let scene_getter = find(&chunk.root, "GetSceneFlag")?;
    let cg = lua::flag_records(cg_getter)?;
    let scene = lua::flag_records(scene_getter)?;
    let cg_constructor = find(&chunk.root, "GalleryCgMenu")?;
    let scene_constructor = find(&chunk.root, "GallerySceneMenu")?;
    if !cg_constructor
        .children
        .iter()
        .any(|f| f.name == cg_getter.name)
        || !scene_constructor
            .children
            .iter()
            .any(|f| f.name == scene_getter.name)
    {
        return Err(failure("Gallery constructor/getter relationship"));
    }
    let rio = root.join("Rio.arc");
    inside(&root, &rio)?;
    // Loose or localized overrides could change the effective menu/record map.
    for name in ["menu_gallery.lua", "CG_PAGE.ws2", "REPLAY_EXE.ws2"] {
        if root.join(name).exists() {
            return Err(failure(format!("Loose override {name}")));
        }
    }
    let info = named_member(&archive, "GameInfo.lua")?;
    let mut active_directories = vec![
        "Script".to_owned(),
        "Script2".to_owned(),
        "Rio".to_owned(),
        "Rio1".to_owned(),
    ];
    let info_chunk = lua::parse(&info)?;
    for name in ["setLanguagePack", "getDefaultLanguage"] {
        if let Ok(function) = find(&info_chunk.root, name)
            && function.code.len() == 3
            && function.code[0] & 63 == 1
            && let Some(lua::Constant::Text(value)) =
                function.constants.get((function.code[0] >> 14) as usize)
        {
            if value.contains(['/', '\\', ':']) || value == ".." {
                return Err(failure("Language resource path"));
            }
            active_directories.push(value.clone());
        }
    }
    for directory in active_directories
        .into_iter()
        .map(|name| root.join(name))
        .filter(|p| p.is_dir())
    {
        for name in ["menu_gallery.lua", "CG_PAGE.ws2", "REPLAY_EXE.ws2"] {
            if directory.join(name).is_file() {
                return Err(failure(format!("Active loose override {name}")));
            }
        }
        for file in ["Script.arc", "Script2.arc", "Rio.arc"] {
            let path = directory.join(file);
            if path.is_file() {
                inside(&root, &path)?;
                let index = arc::index(&path)?;
                if index.iter().any(|e| {
                    matches!(
                        e.name.to_lowercase().as_str(),
                        "menu_gallery.lua" | "cg_page.ws2" | "replay_exe.ws2"
                    )
                }) {
                    return Err(failure(
                        "Localized gallery override requires a separate resolver",
                    ));
                }
            }
        }
    }
    let differences = ws2::differences(&named_member(&rio, "CG_PAGE.ws2")?)?;
    let routes = ws2::replay_routes(&named_member(&rio, "REPLAY_EXE.ws2")?)?;
    if routes.len() != scene.len() {
        return Err(failure("Scene flag table and replay route counts differ"));
    }
    let mut assets = BTreeSet::new();
    for item in fs::read_dir(&root).map_err(io)?.filter_map(Result::ok) {
        let path = item.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|v| v.to_str()) {
                assets.insert(name.to_lowercase());
            }
            if path
                .extension()
                .is_some_and(|v| v.eq_ignore_ascii_case("arc"))
            {
                inside(&root, &path)?;
                for entry in arc::index(&path)? {
                    assets.insert(entry.name.to_lowercase());
                }
            }
        }
    }
    for record in &differences {
        for asset in record.resource.split('/') {
            if !assets.contains(&asset.to_lowercase()) {
                return Err(failure(format!("Missing CG resource: {asset}")));
            }
        }
    }
    for asset in routes.values() {
        if !assets.contains(&format!("{}.ws2", asset.to_lowercase())) {
            return Err(failure(format!("Missing replay script: {asset}")));
        }
    }
    let mut ids: BTreeSet<_> = cg.values().chain(scene.values()).copied().collect();
    ids.extend(differences.iter().map(|v| v.flag));
    let variables = lua::parse(&named_member(&archive, "VariableSize.lua")?)?;
    let limits = lua::literal_returns(find(&variables.root, "getVariableSize")?)?;
    let capacity = match limits.get(1) {
        Some(lua::Constant::Int(n)) if *n > 0 && *n <= 65536 => *n as u32,
        _ => return Err(failure("Native flag capacity")),
    };
    if ids.iter().any(|id| *id as u32 >= capacity) {
        return Err(failure("Gallery flag exceeds native flag capacity"));
    }
    let patched_menu = lua::inject(&menu, &[cg_constructor, scene_constructor], &ids)?;
    let replacement = arc::replace(&original, &entries, &member.name, &patched_menu)?;
    let archive_name = archive.file_name().unwrap().to_string_lossy().into_owned();
    let mut input_hashes = BTreeMap::from([
        (archive_name.clone(), hash(&original)),
        ("Rio.arc".into(), hash(&small(&rio)?)),
    ]);
    input_hashes.insert("GameInfo.lua".into(), hash(&info));
    let save_candidates = save_candidates(&info)?;
    let records = |map: BTreeMap<String, u16>, method: &str| {
        map.into_iter()
            .map(|(name, flag)| Record {
                name,
                flag,
                evidence: format!("menu_gallery.lua::{method} → LegacyGame__lua_GetFlag"),
            })
            .collect::<Vec<_>>()
    };
    let variants = differences
        .into_iter()
        .map(|v| Record {
            name: v.resource,
            flag: v.flag,
            evidence: format!("CG_PAGE.ws2 @ {:#x} → browser", v.offset),
        })
        .collect::<Vec<_>>();
    let variant_ids: BTreeSet<_> = variants.iter().map(|v| v.flag).collect();
    let summary = Summary {
        status: "available".into(),
        engine: format!("AdvHD / Lua 5.3 ({}-bit)", chunk.size_t * 8),
        cg_groups: cg.len(),
        cg_variants: variant_ids.len(),
        scenes: scene.len(),
        reason: None,
    };
    let report = Analysis {
        summary,
        archive: archive_name,
        member: member.name.clone(),
        cg: records(cg, "GetCgFlag"),
        scene: records(scene, "GetSceneFlag"),
        variants,
        input_hashes,
        save_candidates,
        patched_sha256: hash(&replacement),
    };
    Ok(Plan {
        report,
        replacement,
    })
}

fn save_candidates(info: &[u8]) -> AutoResult<Vec<String>> {
    let chunk = lua::parse(info)?;
    let function = find(&chunk.root, "getAppInfo")?;
    let values = lua::literal_returns(function)?;
    let text = |index: usize| -> AutoResult<String> {
        match values.get(index) {
            Some(lua::Constant::Text(s))
                if !s.is_empty() && !s.contains(['/', '\\', ':']) && s != ".." && s != "." =>
            {
                Ok(s.clone())
            }
            _ => Err(failure("Application save identity")),
        }
    };
    let product = text(3)?;
    let brand = text(4)?;
    let vendor = text(10)?;
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| failure("APPDATA not available"))?;
    let base = PathBuf::from(appdata).join(&vendor);
    Ok(vec![
        base.join(&product).to_string_lossy().into_owned(),
        base.join(&brand)
            .join(&product)
            .join("savedata")
            .to_string_lossy()
            .into_owned(),
    ])
}

/// Save identity can be read independently of gallery unlock support.
pub fn save_locations(folder: &Path) -> AutoResult<Vec<String>> {
    for name in ["Script.arc", "Script2.arc", "Rio.arc"] {
        let archive = folder.join(name);
        if archive.is_file()
            && let Ok(info) = named_member(&archive, "GameInfo.lua")
        {
            return save_candidates(&info);
        }
    }
    Ok(Vec::new())
}

pub fn analyze(folder: &Path) -> AutoResult<Analysis> {
    let root = root(folder)?;
    let journal = root.join(BACKUP).join(JOURNAL);
    if journal.is_file() {
        inside(&root, &journal)?;
        let mut record: Journal = serde_json::from_slice(&small(&journal)?).map_err(io)?;
        validate_journal(&record)?;
        inside(&root, &root.join(&record.report.archive))?;
        let current = hash(&small(&root.join(&record.report.archive))?);
        if current == record.report.input_hashes[&record.report.archive] {
            record.report.summary.status = "recoverable".into();
            return Ok(record.report);
        }
        if current != record.report.patched_sha256 {
            return Err(Message::new("auto.changed"));
        }
        record.report.summary.status = "installed".into();
        return Ok(record.report);
    }
    Ok(plan(folder)?.report)
}
pub fn summary(folder: &Path) -> Summary {
    match analyze(folder) {
        Ok(v) => v.summary,
        Err(reason) => Summary {
            status: "unsupported".into(),
            reason: Some(reason),
            ..Summary::default()
        },
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    report: Analysis,
    save_backups: Vec<String>,
}

fn validate_journal(journal: &Journal) -> AutoResult<()> {
    let name = &journal.report.archive;
    if journal.version != 1
        || name.contains(['/', '\\', ':'])
        || !name.to_lowercase().starts_with("script")
        || !name.to_lowercase().ends_with(".arc")
        || !journal
            .report
            .member
            .eq_ignore_ascii_case("menu_gallery.lua")
        || !journal.report.input_hashes.contains_key(name)
    {
        return Err(failure("Backup journal scope"));
    }
    Ok(())
}
fn atomic(path: &Path, data: &[u8], replace: bool) -> AutoResult<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or_else(|| failure("File parent"))?)
            .map_err(io)?;
    file.write_all(data).map_err(io)?;
    file.as_file().sync_all().map_err(io)?;
    if replace {
        file.persist(path).map_err(io)?;
    } else {
        file.persist_noclobber(path).map_err(io)?;
    }
    Ok(())
}
pub fn install(folder: &Path) -> AutoResult<Analysis> {
    super::unlock::ensure_game_closed_names(&["AdvHD.exe", "AdvHD_new.exe"])?;
    let root = root(folder)?;
    let plan = plan(&root)?;
    let target = root.join(&plan.report.archive);
    inside(&root, &target)?;
    let backup = root.join(BACKUP);
    inside(&root, &backup)?;
    fs::create_dir_all(&backup).map_err(io)?;
    let original = small(&target)?;
    let expected = plan
        .report
        .input_hashes
        .get(&plan.report.archive)
        .ok_or_else(|| failure("Input hash"))?;
    let original_backup = backup.join("original.arc");
    inside(&root, &original_backup)?;
    if original_backup.exists() {
        if hash(&small(&original_backup)?) != *expected {
            return Err(Message::new("unlock.backup"));
        }
    } else {
        atomic(&original_backup, &original, false)?;
    }
    let mut save_backups = Vec::new();
    for (i, path) in plan.report.save_candidates.iter().enumerate() {
        let source = PathBuf::from(path);
        if !source.exists() {
            continue;
        }
        let canonical = fs::canonicalize(&source).map_err(io)?;
        let allowed = save_roots(&source)?;
        if !allowed.iter().any(|root| within(&canonical, root)) {
            return Err(io(format!(
                "Canonical AppData path check: {} / {}",
                canonical.display(),
                allowed
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        for file in fs::read_dir(&source).map_err(io)?.filter_map(Result::ok) {
            if !file.path().is_file() {
                continue;
            }
            let name = file.file_name().to_string_lossy().into_owned();
            if !matches!(
                name.to_lowercase().as_str(),
                "save000" | "save000gs" | "save000favorite" | "messageflag.dat" | "nmsbookmark.dat"
            ) {
                continue;
            }
            let path = file.path();
            let resolved = fs::canonicalize(&path).map_err(io)?;
            if !within(&resolved, &canonical) {
                return Err(io(format!(
                    "Canonical save path check: {} / {}",
                    resolved.display(),
                    canonical.display()
                )));
            }
            let output = backup.join(format!("save-{i}-{name}"));
            inside(&root, &output)?;
            if !output.exists() {
                atomic(&output, &small(&path)?, false)?;
            }
            save_backups.push(path.to_string_lossy().into_owned());
        }
    }
    super::unlock::ensure_game_closed_names(&["AdvHD.exe", "AdvHD_new.exe"])?;
    if hash(&small(&target)?) != *expected
        || hash(&small(&root.join("Rio.arc"))?) != plan.report.input_hashes["Rio.arc"]
    {
        return Err(Message::new("auto.changed"));
    }
    let journal = Journal {
        version: 1,
        report: plan.report.clone(),
        save_backups,
    };
    let journal_path = backup.join(JOURNAL);
    inside(&root, &journal_path)?;
    // Write the recovery metadata first. Any interrupted install can therefore
    // restore the unchanged original or the completed replacement.
    atomic(
        &journal_path,
        &serde_json::to_vec_pretty(&journal).map_err(io)?,
        false,
    )?;
    atomic(&target, &plan.replacement, true)?;
    let mut report = plan.report;
    report.summary.status = "installed".into();
    Ok(report)
}
pub fn restore(folder: &Path) -> AutoResult<Analysis> {
    super::unlock::ensure_game_closed_names(&["AdvHD.exe", "AdvHD_new.exe"])?;
    let root = root(folder)?;
    let backup = root.join(BACKUP);
    let journal_path = backup.join(JOURNAL);
    inside(&root, &journal_path)?;
    let journal: Journal = serde_json::from_slice(&small(&journal_path)?).map_err(io)?;
    validate_journal(&journal)?;
    let target = root.join(&journal.report.archive);
    inside(&root, &target)?;
    let original_backup = backup.join("original.arc");
    inside(&root, &original_backup)?;
    let original = small(&original_backup)?;
    let expected = journal
        .report
        .input_hashes
        .get(&journal.report.archive)
        .ok_or_else(|| failure("Backup input hash"))?;
    if hash(&original) != *expected {
        return Err(Message::new("unlock.backup"));
    }
    let current = hash(&small(&target)?);
    if current != journal.report.patched_sha256 && current != *expected {
        return Err(Message::new("auto.changed"));
    }
    super::unlock::ensure_game_closed_names(&["AdvHD.exe", "AdvHD_new.exe"])?;
    if hash(&small(&target)?) != current {
        return Err(Message::new("auto.changed"));
    }
    atomic(&target, &original, true)?;
    fs::remove_file(journal_path).map_err(io)?;
    analyze(&root)
}
