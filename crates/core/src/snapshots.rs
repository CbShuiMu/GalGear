//! Complete save snapshots, verified contents, retention, and reversible restoration.
mod migration;
pub use migration::{game_data, migrate};

use crate::{config::display_path, runtime::Status};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct File {
    pub root: usize,
    pub path: String,
    pub size: u64,
    pub hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sequence: u64,
    pub game_id: String,
    pub created: u64,
    pub size: u64,
    pub note: String,
    pub locked: bool,
    pub roots: Vec<String>,
    pub files: Vec<File>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub root: usize,
    pub path: String,
    pub action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub snapshot: Snapshot,
    pub targets: Vec<String>,
    pub changes: Vec<Change>,
    pub current: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    plan: Preview,
    originals: Vec<String>,
    committed: bool,
}
fn journal_path(data: &Path, game: &str) -> Result<PathBuf, Message> {
    Ok(directory(data, game)?.join("restore-journal.json"))
}
fn write_journal(path: &Path, journal: &Journal) -> Result<(), Message> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Missing journal parent"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    serde_json::to_writer(&mut temp, journal).map_err(error)?;
    temp.flush().map_err(error)?;
    temp.as_file().sync_all().map_err(error)?;
    temp.persist(path).map_err(error)?;
    Ok(())
}

/// Finish or roll back an interrupted restore only at the currently confirmed locations.
pub fn recover(
    data: &Path,
    game: &str,
    confirmed: &[String],
    status: Status,
) -> Result<(), Message> {
    let path = journal_path(data, game)?;
    if !path.exists() {
        return Ok(());
    }
    require_stopped(status)?;
    let journal: Journal =
        serde_json::from_slice(&fs::read(&path).map_err(error)?).map_err(error)?;
    if journal.plan.snapshot.game_id != game
        || journal.plan.targets.len() != journal.originals.len()
        || journal.plan.targets.iter().any(|target| {
            !confirmed.iter().any(|p| {
                p.replace('/', "\\")
                    .eq_ignore_ascii_case(&target.replace('/', "\\"))
            })
        })
    {
        return Err(Message::new("backup.preview_required"));
    }
    for (index, old) in journal.originals.iter().enumerate().rev() {
        let target = Path::new(&journal.plan.targets[index]);
        let old = Path::new(old);
        if old.parent() != target.parent()
            || !old.file_name().is_some_and(|n| {
                n.to_string_lossy().starts_with(".galgear-restore-")
                    && n.to_string_lossy().ends_with(".original")
            })
        {
            return Err(error("Unsafe recovery journal"));
        }
        if !old.exists() {
            continue;
        }
        if fs::symlink_metadata(old)
            .map_err(error)?
            .file_type()
            .is_symlink()
        {
            return Err(error("Recovery path is a link"));
        }
        if journal.committed {
            fs::remove_dir_all(old).map_err(error)?;
            continue;
        }
        let mut previous = Vec::new();
        files(old, old, index, &mut previous)?;
        let expected: BTreeMap<_, _> = journal
            .plan
            .current
            .iter()
            .filter(|(key, _)| key.starts_with(&format!("{index}:")))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if fingerprints(&previous) != expected {
            return Err(error("Rollback saves changed; recovery is pending"));
        }
        if target.exists() {
            let mut actual = Vec::new();
            files(target, target, index, &mut actual)?;
            let wanted: Vec<_> = journal
                .plan
                .snapshot
                .files
                .iter()
                .filter(|f| f.root == index)
                .cloned()
                .collect();
            if fingerprints(&actual) != fingerprints(&wanted) {
                return Err(error("Restored saves changed; recovery is pending"));
            }
            fs::remove_dir_all(target).map_err(error)?;
        }
        fs::rename(old, target).map_err(error)?;
    }
    fs::remove_file(path).map_err(error)
}

fn error(value: impl std::fmt::Display) -> Message {
    Message::detail("backup.error", value)
}
pub fn require_stopped(status: Status) -> Result<(), Message> {
    if status == Status::Stopped {
        Ok(())
    } else {
        Err(Message::new("backup.blocked"))
    }
}
fn safe_id(id: &str) -> Result<(), Message> {
    if !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        Ok(())
    } else {
        Err(error("Invalid snapshot identity"))
    }
}
fn directory(data: &Path, game: &str) -> Result<PathBuf, Message> {
    safe_id(game)?;
    Ok(data.join(game))
}
fn relative(path: &str) -> Result<PathBuf, Message> {
    let path = PathBuf::from(path);
    if path.components().all(|c| matches!(c, Component::Normal(_)))
        && !path.as_os_str().is_empty()
        && !path.to_string_lossy().contains(':')
    {
        Ok(path)
    } else {
        Err(error("Unsafe snapshot path"))
    }
}
fn hash(path: &Path) -> Result<String, Message> {
    let mut file = fs::File::open(path).map_err(error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), Message> {
    let mut input = fs::File::open(from).map_err(error)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(error)?;
    std::io::copy(&mut input, &mut output).map_err(error)?;
    output.sync_all().map_err(error)
}
fn files(root: &Path, path: &Path, index: usize, output: &mut Vec<File>) -> Result<(), Message> {
    for entry in fs::read_dir(path).map_err(error)? {
        let entry = entry.map_err(error)?;
        let kind = entry.file_type().map_err(error)?;
        if kind.is_symlink() {
            return Err(error("Save folders must not contain links"));
        }
        if kind.is_dir() {
            files(root, &entry.path(), index, output)?;
        } else if kind.is_file() {
            if output.len() >= 20000 {
                return Err(error("Save file count exceeds 20000"));
            }
            output.push(File {
                root: index,
                path: display_path(entry.path().strip_prefix(root).map_err(error)?),
                size: entry.metadata().map_err(error)?.len(),
                hash: hash(&entry.path())?,
            });
        }
    }
    Ok(())
}
fn inventory(roots: &[String]) -> Result<Vec<File>, Message> {
    let mut result = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        files(Path::new(root), Path::new(root), index, &mut result)?;
    }
    result.sort_by(|a, b| (a.root, &a.path).cmp(&(b.root, &b.path)));
    if result.iter().map(|f| f.size).sum::<u64>() > 2 * 1024 * 1024 * 1024 {
        return Err(error("Save content exceeds 2 GiB"));
    }
    Ok(result)
}

/// Reject broad or overlapping locations before any file writes.
pub fn validate_roots(
    roots: &[String],
    game_folder: &Path,
    data: &Path,
) -> Result<Vec<String>, Message> {
    if roots.is_empty() {
        return Err(Message::new("backup.no_saves"));
    }
    let game = fs::canonicalize(game_folder).map_err(error)?;
    let broad: Vec<_> = crate::saves::user_roots()
        .into_iter()
        .map(|(_, p)| p)
        .chain(std::env::var_os("USERPROFILE").map(PathBuf::from))
        .filter_map(|p| fs::canonicalize(p).ok())
        .collect();
    // A not-yet-created backup folder still needs the same canonical prefix as
    // save roots (notably the Windows extended-length path prefix).
    let mut existing = data;
    let mut missing = Vec::new();
    let data = loop {
        match fs::canonicalize(existing) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                break canonical;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    existing
                        .file_name()
                        .ok_or_else(|| error("Invalid backup path"))?,
                );
                existing = existing
                    .parent()
                    .ok_or_else(|| error("Invalid backup parent"))?;
            }
            Err(err) => return Err(error(err)),
        }
    };
    let mut canonical: Vec<PathBuf> = Vec::new();
    for root in roots {
        if fs::symlink_metadata(root)
            .map_err(error)?
            .file_type()
            .is_symlink()
        {
            return Err(error("Save location is a link"));
        }
        let root = fs::canonicalize(root).map_err(error)?;
        if root == game
            || game.starts_with(&root)
            || broad.iter().any(|p| p.starts_with(&root))
            || data.starts_with(&root)
            || root.starts_with(&data)
            || root.parent().is_none()
        {
            return Err(error("Select a dedicated save folder"));
        }
        if canonical
            .iter()
            .any(|p| p.starts_with(&root) || root.starts_with(p))
        {
            return Err(error("Save locations overlap"));
        }
        canonical.push(root);
    }
    Ok(canonical.iter().map(|p| display_path(p)).collect())
}

fn write_manifest(path: &Path, snapshot: &Snapshot) -> Result<(), Message> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Missing manifest directory"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    serde_json::to_writer_pretty(&mut temp, snapshot).map_err(error)?;
    temp.flush().map_err(error)?;
    temp.as_file().sync_all().map_err(error)?;
    temp.persist(path).map_err(error)?;
    Ok(())
}

fn write_sequence(parent: &Path, sequence: u64) -> Result<(), Message> {
    let mut counter = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    serde_json::to_writer(&mut counter, &sequence).map_err(error)?;
    counter.as_file().sync_all().map_err(error)?;
    counter
        .persist(parent.join("sequence.json"))
        .map_err(error)?;
    Ok(())
}

/// Completed manifests are the only snapshots exposed in history.
pub fn list(data: &Path, game_id: &str) -> Result<Vec<Snapshot>, Message> {
    let dir = directory(data, game_id)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut results = Vec::new();
    for entry in fs::read_dir(&dir).map_err(error)? {
        let entry = entry.map_err(error)?;
        if !entry.file_type().map_err(error)?.is_dir()
            || entry.file_name().to_string_lossy().starts_with('.')
        {
            continue;
        }
        let path = entry.path().join("manifest.json");
        if !path.is_file() {
            continue;
        }
        let snapshot: Snapshot =
            serde_json::from_slice(&fs::read(path).map_err(error)?).map_err(error)?;
        if snapshot.game_id == game_id && snapshot.id == entry.file_name().to_string_lossy() {
            results.push(snapshot);
        }
    }
    results.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
    // Older manifests keep their storage identity and receive stable display numbers.
    if results.iter().any(|snapshot| snapshot.sequence == 0) {
        let counter = match fs::read(dir.join("sequence.json")) {
            Ok(bytes) => serde_json::from_slice::<u64>(&bytes).map_err(error)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(error(e)),
        };
        let mut next = counter.max(
            results
                .iter()
                .map(|snapshot| snapshot.sequence)
                .max()
                .unwrap_or(0),
        );
        let count = results
            .iter()
            .filter(|snapshot| snapshot.sequence == 0)
            .count() as u64;
        write_sequence(
            &dir,
            next.checked_add(count)
                .ok_or_else(|| error("Backup sequence exhausted"))?,
        )?;
        for snapshot in results
            .iter_mut()
            .rev()
            .filter(|snapshot| snapshot.sequence == 0)
        {
            next += 1;
            snapshot.sequence = next;
            write_manifest(&dir.join(&snapshot.id).join("manifest.json"), snapshot)?;
        }
    }
    Ok(results)
}

/// Copy all confirmed locations and publish only after verifying stable content.
pub fn create(
    data: &Path,
    game_id: &str,
    roots: &[String],
    note: &str,
    retention: usize,
    status: Status,
) -> Result<Snapshot, Message> {
    create_checked(data, game_id, roots, note, retention, || status)
}

/// Repeat the runtime check before publishing a potentially long-running backup.
pub fn create_checked(
    data: &Path,
    game_id: &str,
    roots: &[String],
    note: &str,
    retention: usize,
    check: impl Fn() -> Status,
) -> Result<Snapshot, Message> {
    create_named_checked(data, (game_id, ""), roots, note, retention, check)
}

/// Assign a durable, per-game increasing number before publishing a backup.
pub fn create_named_checked(
    data: &Path,
    game: (&str, &str),
    roots: &[String],
    note: &str,
    retention: usize,
    check: impl Fn() -> Status,
) -> Result<Snapshot, Message> {
    create_unique_named_checked(data, game, roots, note, retention, check)
        .map(|result| result.snapshot)
}

pub struct BackupResult {
    pub snapshot: Snapshot,
    pub created: bool,
}

/// Reuse any verified historical snapshot with identical paths and contents.
/// Duplicate requests do not consume a sequence number or prune history.
pub fn create_unique_named_checked(
    data: &Path,
    game: (&str, &str),
    roots: &[String],
    note: &str,
    retention: usize,
    check: impl Fn() -> Status,
) -> Result<BackupResult, Message> {
    create_compared(data, game, roots, note, retention, check, false)
}

/// Skip automatic copies when only recognized engine bookkeeping changed.
/// Manual and restore-safety backups continue to compare the exact bytes.
pub fn create_automatic_named_checked(
    data: &Path,
    game: (&str, &str),
    roots: &[String],
    note: &str,
    retention: usize,
    check: impl Fn() -> Status,
) -> Result<BackupResult, Message> {
    create_compared(data, game, roots, note, retention, check, true)
}

fn create_compared(
    data: &Path,
    game: (&str, &str),
    roots: &[String],
    note: &str,
    retention: usize,
    check: impl Fn() -> Status,
    automatic: bool,
) -> Result<BackupResult, Message> {
    let (game_id, title) = game;
    let status = check();
    require_stopped(status)?;
    let entries = inventory(roots)?;
    if entries.is_empty() {
        return Err(Message::new("backup.no_saves"));
    }
    if let Some(snapshot) = matching_entries(
        data,
        &list(data, game_id)?,
        roots.len(),
        &entries,
        automatic.then_some(roots),
    ) {
        if fingerprints(&inventory(roots)?) != fingerprints(&entries) {
            return Err(error("Save changed while checking history"));
        }
        require_stopped(check())?;
        return Ok(BackupResult {
            snapshot: snapshot.clone(),
            created: false,
        });
    }
    let parent = directory(data, game_id)?;
    fs::create_dir_all(&parent).map_err(error)?;
    let temporary = tempfile::Builder::new()
        .prefix(".incomplete-")
        .tempdir_in(&parent)
        .map_err(error)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(error)?;
    let created = timestamp.as_secs();
    let counter_path = parent.join("sequence.json");
    let previous: u64 = match fs::read(&counter_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(error)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => list(data, game_id)?
            .iter()
            .map(|snapshot| snapshot.sequence)
            .max()
            .unwrap_or(0),
        Err(e) => return Err(error(e)),
    };
    let sequence = previous
        .checked_add(1)
        .ok_or_else(|| error("Backup sequence exhausted"))?;
    write_sequence(&parent, sequence)?;
    let id = format!(
        "{:020}-{:x}",
        timestamp.as_nanos(),
        Sha256::digest(temporary.path().to_string_lossy().as_bytes())
    );
    for file in &entries {
        let relative = relative(&file.path)?;
        let target = temporary.path().join(file.root.to_string()).join(&relative);
        fs::create_dir_all(target.parent().ok_or_else(|| error("Missing parent"))?)
            .map_err(error)?;
        copy_file(&Path::new(&roots[file.root]).join(&relative), &target)?;
        if hash(&target)? != file.hash {
            return Err(error("Save changed while copying"));
        }
    }
    if fingerprints(&inventory(roots)?) != fingerprints(&entries) {
        return Err(error("Save changed while copying"));
    }
    let snapshot = Snapshot {
        id,
        name: format!("{title}_{created}_{sequence}"),
        sequence,
        game_id: game_id.into(),
        created,
        size: entries.iter().map(|f| f.size).sum(),
        note: note.into(),
        locked: false,
        roots: roots.to_vec(),
        files: entries,
    };
    write_manifest(&temporary.path().join("manifest.json"), &snapshot)?;
    require_stopped(check())?;
    fs::rename(temporary.path(), parent.join(&snapshot.id)).map_err(error)?;
    prune(data, game_id, retention.max(1))?;
    Ok(BackupResult {
        snapshot,
        created: true,
    })
}

/// Identify the current save from content, independently of timestamps and notes.
pub fn matching_current(
    data: &Path,
    history: &[Snapshot],
    roots: &[String],
) -> Result<Option<String>, Message> {
    if history.is_empty() || roots.is_empty() {
        return Ok(None);
    }
    let entries = inventory(roots)?;
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(matching_entries(data, history, roots.len(), &entries, Some(roots)).map(|snapshot| snapshot.id.clone()))
}

fn matching_entries<'a>(
    data: &Path,
    history: &'a [Snapshot],
    root_count: usize,
    entries: &[File],
    semantic_roots: Option<&[String]>,
) -> Option<&'a Snapshot> {
    let current = fingerprints(entries);
    history.iter().find(|snapshot| {
        snapshot.roots.len() == root_count
            && snapshot.files.len() == entries.len()
            && (fingerprints(&snapshot.files) == current
                || semantic_roots.is_some_and(|roots| equivalent_entries(data, snapshot, roots, entries)))
            // A damaged or missing backup must never suppress a new safety copy.
            && content_directory(data, &snapshot.game_id, &snapshot.id).is_ok_and(|directory| {
                snapshot.files.iter().all(|file| {
                    file.root < root_count && relative(&file.path).is_ok_and(|path| {
                        let path = directory.join(file.root.to_string()).join(path);
                        fs::metadata(&path).is_ok_and(|meta| meta.len() == file.size)
                            && hash(&path).is_ok_and(|hash| hash == file.hash)
                    })
                })
            })
    })
}

fn equivalent_entries(
    data: &Path,
    snapshot: &Snapshot,
    roots: &[String],
    entries: &[File],
) -> bool {
    let Ok(directory) = content_directory(data, &snapshot.game_id, &snapshot.id) else {
        return false;
    };
    let previous: BTreeMap<_, _> = snapshot
        .files
        .iter()
        .map(|file| ((file.root, &file.path), file))
        .collect();
    entries.iter().all(|file| {
        let Some(saved) = previous.get(&(file.root, &file.path)) else {
            return false;
        };
        if saved.hash == file.hash {
            return true;
        }
        let Ok(path) = relative(&file.path) else {
            return false;
        };
        let Some(root) = roots.get(file.root) else {
            return false;
        };
        let Some(current) = crate::saves::backup_identity(&Path::new(root).join(&path)) else {
            return false;
        };
        crate::saves::backup_identity(&directory.join(file.root.to_string()).join(path))
            .is_some_and(|previous| previous == current)
    })
}

fn fingerprints(files: &[File]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|f| (format!("{}:{}", f.root, f.path), f.hash.clone()))
        .collect()
}
fn load(data: &Path, game: &str, id: &str) -> Result<Snapshot, Message> {
    safe_id(id)?;
    list(data, game)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| error("Snapshot not found"))
}

/// Locate immutable snapshot contents without accepting path components as IDs.
pub(crate) fn content_directory(data: &Path, game: &str, id: &str) -> Result<PathBuf, Message> {
    safe_id(id)?;
    Ok(directory(data, game)?.join(id))
}

/// Publish verified imported contents as an ordinary local snapshot. The caller
/// owns its extraction staging directory; live save roots are never accessed.
pub(crate) fn import_verified(
    data: &Path,
    game: &str,
    mut snapshot: Snapshot,
    extracted: &Path,
) -> Result<Snapshot, Message> {
    safe_id(&snapshot.id)?;
    let parent = directory(data, game)?;
    fs::create_dir_all(&parent).map_err(error)?;
    let staged = tempfile::Builder::new()
        .prefix(".incomplete-cloud-")
        .tempdir_in(&parent)
        .map_err(error)?;
    let mut identities = std::collections::BTreeSet::new();
    let mut size = 0u64;
    for file in &snapshot.files {
        if file.root >= snapshot.roots.len()
            || !identities.insert((file.root, file.path.to_lowercase()))
        {
            return Err(error("Invalid imported snapshot file"));
        }
        let relative = relative(&file.path)?;
        let source_root = extracted.join(file.root.to_string());
        let mut source = source_root.clone();
        for component in std::iter::once(Component::CurDir).chain(relative.components()) {
            if let Component::Normal(component) = component {
                source.push(component);
            }
            if fs::symlink_metadata(&source)
                .map_err(error)?
                .file_type()
                .is_symlink()
            {
                return Err(error("Imported snapshot contains a link"));
            }
        }
        if !fs::metadata(&source).map_err(error)?.is_file()
            || fs::metadata(&source).map_err(error)?.len() != file.size
            || hash(&source)? != file.hash
        {
            return Err(error("Imported snapshot contents failed verification"));
        }
        let target = staged.path().join(file.root.to_string()).join(relative);
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| error("Missing import parent"))?,
        )
        .map_err(error)?;
        copy_file(&source, &target)?;
        if hash(&target)? != file.hash {
            return Err(error("Imported snapshot changed while copying"));
        }
        size = size
            .checked_add(file.size)
            .ok_or_else(|| error("Imported snapshot is too large"))?;
    }
    if size != snapshot.size || size > 2 * 1024 * 1024 * 1024 {
        return Err(error("Invalid imported snapshot size"));
    }
    let previous = list(data, game)?;
    let counter = match fs::read(parent.join("sequence.json")) {
        Ok(bytes) => serde_json::from_slice::<u64>(&bytes).map_err(error)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(error(e)),
    };
    snapshot.sequence = counter
        .max(previous.iter().map(|item| item.sequence).max().unwrap_or(0))
        .checked_add(1)
        .ok_or_else(|| error("Backup sequence exhausted"))?;
    if parent.join(&snapshot.id).exists() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(error)?;
        snapshot.id = format!(
            "{:020}-{:x}",
            timestamp.as_nanos(),
            Sha256::digest(staged.path().to_string_lossy().as_bytes())
        );
    }
    game.clone_into(&mut snapshot.game_id);
    snapshot.locked = false;
    // Imported root labels retain the root count, never another PC's absolute paths.
    snapshot.roots = (0..snapshot.roots.len())
        .map(|index| format!("Save {}", index + 1))
        .collect();
    for file in &mut snapshot.files {
        // Existing preview fingerprints use the host's native path separators.
        file.path = display_path(&relative(&file.path)?.components().collect::<PathBuf>());
    }
    write_sequence(&parent, snapshot.sequence)?;
    write_manifest(&staged.path().join("manifest.json"), &snapshot)?;
    fs::rename(staged.path(), parent.join(&snapshot.id)).map_err(error)?;
    Ok(snapshot)
}

pub fn update(data: &Path, game: &str, id: &str, locked: bool, note: &str) -> Result<(), Message> {
    let mut snapshot = load(data, game, id)?;
    snapshot.locked = locked;
    note.clone_into(&mut snapshot.note);
    write_manifest(
        &directory(data, game)?.join(id).join("manifest.json"),
        &snapshot,
    )
}
pub fn delete(data: &Path, game: &str, id: &str) -> Result<(), Message> {
    let snapshot = load(data, game, id)?;
    if snapshot.locked {
        return Err(error("Unlock this snapshot before deleting"));
    }
    fs::remove_dir_all(directory(data, game)?.join(id)).map_err(error)
}
fn prune(data: &Path, game: &str, retention: usize) -> Result<(), Message> {
    for snapshot in list(data, game)?
        .into_iter()
        .filter(|s| !s.locked)
        .skip(retention)
    {
        delete(data, game, &snapshot.id)?;
    }
    Ok(())
}

/// Preview uses current confirmed roots; recorded old absolute paths are never targets.
pub fn preview(
    data: &Path,
    game: &str,
    id: &str,
    targets: &[String],
    status: Status,
) -> Result<Preview, Message> {
    require_stopped(status)?;
    let snapshot = load(data, game, id)?;
    if snapshot.roots.len() != targets.len() {
        return Err(error(
            "Confirm all current save locations in the original order",
        ));
    }
    let current = fingerprints(&inventory(targets)?);
    let wanted = fingerprints(&snapshot.files);
    let mut changes = Vec::new();
    for file in &snapshot.files {
        if file.root >= targets.len() {
            return Err(error("Invalid snapshot root"));
        }
        relative(&file.path)?;
        let key = format!("{}:{}", file.root, file.path);
        let action = if current.contains_key(&key) {
            "overwrite"
        } else {
            "add"
        };
        changes.push(Change {
            root: file.root,
            path: file.path.clone(),
            action: action.into(),
        });
    }
    for file in inventory(targets)? {
        if !wanted.contains_key(&format!("{}:{}", file.root, file.path)) {
            changes.push(Change {
                root: file.root,
                path: file.path,
                action: "remove".into(),
            });
        }
    }
    Ok(Preview {
        snapshot,
        targets: targets.to_vec(),
        changes,
        current,
    })
}

/// Validate a preview again, make a safety snapshot, then replace each save tree.
/// Original trees are retained until all replacements succeed; errors roll back.
pub fn restore(
    data: &Path,
    plan: &Preview,
    retention: usize,
    status: Status,
) -> Result<(), Message> {
    restore_checked(data, plan, retention, || status)
}

/// Inspect runtime again immediately before replacing live save directories.
pub fn restore_checked(
    data: &Path,
    plan: &Preview,
    retention: usize,
    check: impl Fn() -> Status,
) -> Result<(), Message> {
    let status = check();
    require_stopped(status)?;
    if journal_path(data, &plan.snapshot.game_id)?.exists() {
        return Err(error("An interrupted restore needs recovery first"));
    }
    if fingerprints(&inventory(&plan.targets)?) != plan.current {
        return Err(error("Saves changed after preview; preview again"));
    }
    let original = load(data, &plan.snapshot.game_id, &plan.snapshot.id)?;
    if fingerprints(&original.files) != fingerprints(&plan.snapshot.files) {
        return Err(error("Snapshot changed after preview"));
    }
    let source = directory(data, &original.game_id)?.join(&original.id);
    let mut stages = Vec::new();
    for (index, target) in plan.targets.iter().enumerate() {
        let parent = Path::new(target)
            .parent()
            .ok_or_else(|| error("Missing save parent"))?;
        let stage = tempfile::Builder::new()
            .prefix(".galgear-restore-")
            .tempdir_in(parent)
            .map_err(error)?;
        for file in original.files.iter().filter(|f| f.root == index) {
            let relative = relative(&file.path)?;
            let from = source.join(index.to_string()).join(&relative);
            if hash(&from)? != file.hash {
                return Err(error("Snapshot contents failed verification"));
            }
            let to = stage.path().join(&relative);
            fs::create_dir_all(to.parent().ok_or_else(|| error("Missing file parent"))?)
                .map_err(error)?;
            copy_file(&from, &to)?;
            if hash(&to)? != file.hash {
                return Err(error("Staged restore failed verification"));
            }
        }
        stages.push(stage);
    }
    if !plan.current.is_empty() {
        create_checked(
            data,
            &original.game_id,
            &plan.targets,
            "Before restore",
            retention,
            &check,
        )?;
    }
    if fingerprints(&inventory(&plan.targets)?) != plan.current {
        return Err(error("Saves changed during preparation"));
    }
    let path = journal_path(data, &original.game_id)?;
    let mut journal = Journal {
        plan: plan.clone(),
        originals: stages
            .iter()
            .map(|s| display_path(&s.path().with_extension("original")))
            .collect(),
        committed: false,
    };
    write_journal(&path, &journal)?;
    let mut originals = Vec::new();
    require_stopped(check())?;
    for (index, stage) in stages.iter().enumerate() {
        let target = Path::new(&plan.targets[index]);
        let old = stage.path().with_extension("original");
        if old.exists() {
            return Err(error("Rollback directory already exists"));
        }
        if let Err(err) = fs::rename(target, &old).and_then(|()| fs::rename(stage.path(), target)) {
            if old.exists() && !target.exists() {
                fs::rename(&old, target).map_err(error)?;
            }
            for (target, old) in originals.iter().rev() {
                fs::remove_dir_all(target).map_err(error)?;
                fs::rename(old, target).map_err(error)?;
            }
            fs::remove_file(&path).map_err(error)?;
            return Err(error(err));
        }
        originals.push((target.to_owned(), old));
    }
    journal.committed = true;
    write_journal(&path, &journal)?;
    for (_, old) in originals {
        fs::remove_dir_all(old).map_err(error)?;
    }
    fs::remove_file(path).map_err(error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn control_bytes(text: &str) -> Vec<u8> {
        let raw: Vec<_> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let packed = encoder.finish().unwrap();
        let mut bytes = vec![0xfe, 0xfe, 2, 0xff, 0xfe];
        bytes.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        bytes.extend(packed);
        bytes
    }

    #[test]
    fn automatic_backup_ignores_boot_counters_and_dictionary_order_but_manual_is_exact() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("saves");
        fs::create_dir(&saves).unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        fs::write(saves.join("data_continue.bmp"), b"unchanged story slot").unwrap();
        fs::write(saves.join("datasc.ksd"), control_bytes(r#"(const) %["voice" => (const) %["a" => 80, "b" => 90], "fullScreen" => 1, "windowWidth" => 1280]"#)).unwrap();
        fs::write(saves.join("datasu.ksd"), control_bytes(r#"(const) %["trail_custom_exit_end" => 9, "trail_first_first" => 17, "cg_scene" => 1]"#)).unwrap();
        let first = create(&data, "game", &roots, "manual", 10, Status::Stopped).unwrap();
        fs::write(saves.join("datasc.ksd"), control_bytes(r#"(const) %["windowWidth" => 1280, "fullScreen" => 1, "voice" => (const) %["b" => 90, "a" => 80]]"#)).unwrap();
        fs::write(saves.join("datasu.ksd"), control_bytes(r#"(const) %["cg_scene" => 1, "trail_first_first" => 18, "trail_custom_exit_end" => 10]"#)).unwrap();
        for _ in 0..3 {
            let result =
                create_automatic_named_checked(&data, ("game", "Game"), &roots, "auto", 1, || {
                    Status::Stopped
                })
                .unwrap();
            assert!(!result.created);
            assert_eq!(result.snapshot.id, first.id);
        }
        assert_eq!(list(&data, "game").unwrap().len(), 1);
        assert_eq!(
            matching_current(&data, &list(&data, "game").unwrap(), &roots).unwrap(),
            Some(first.id)
        );
        let manual =
            create_unique_named_checked(&data, ("game", "Game"), &roots, "manual", 10, || {
                Status::Stopped
            })
            .unwrap();
        assert!(manual.created);
        assert_eq!(manual.snapshot.sequence, 2);
    }

    #[test]
    fn automatic_backup_preserves_real_changes_and_rejects_damaged_history() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("saves");
        fs::create_dir(&saves).unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let control = saves.join("system.ksd");
        let initial = r#"(const) %["fullScreen" => 1, "windowWidth" => 1280, "trail_custom_exit_end" => 9, "cg_scene" => 0, "trail_story" => 1, "values" => (const) [1, 2], "volume" => 0x1.4p3, "optional" => null]"#;
        fs::write(&control, control_bytes(initial)).unwrap();
        let first = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        for (from, to) in [
            ("1280", "1920"),
            ("\"cg_scene\" => 0", "\"cg_scene\" => 1"),
            ("\"trail_story\" => 1", "\"trail_story\" => 2"),
            ("[1, 2]", "[2, 1]"),
            ("0x1.4p3", "0x1.5p3"),
            ("null", "void"),
            ("1280", "\"number:1280\""),
            ("=> 9", "=> 0"),
            ("\"cg_scene\"", "\"cg_new_scene\""),
        ] {
            fs::write(&control, control_bytes(&initial.replace(from, to))).unwrap();
            assert_eq!(
                matching_current(&data, std::slice::from_ref(&first), &roots).unwrap(),
                None,
                "{from} -> {to}"
            );
        }
        fs::write(&control, control_bytes(&initial.replace("=> 9", "=> 10"))).unwrap();
        fs::write(
            data.join("game").join(&first.id).join("0/system.ksd"),
            b"damaged backup",
        )
        .unwrap();
        assert!(
            create_automatic_named_checked(&data, ("game", "Game"), &roots, "auto", 10, || {
                Status::Stopped
            })
            .unwrap()
            .created
        );
        fs::write(saves.join("slot.sav"), b"new progress").unwrap();
        assert!(
            create_automatic_named_checked(&data, ("game", "Game"), &roots, "auto", 10, || {
                Status::Stopped
            })
            .unwrap()
            .created
        );
        fs::write(saves.join("slot.sav"), b"changed progress").unwrap();
        assert!(
            create_automatic_named_checked(&data, ("game", "Game"), &roots, "auto", 10, || {
                Status::Stopped
            })
            .unwrap()
            .created
        );
    }

    #[test]
    fn duplicate_backup_reuses_older_content_without_pruning_or_consuming_sequence() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let first = create(&data, "game", &roots, "manual", 10, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"other").unwrap();
        let second = create(&data, "game", &roots, "manual", 10, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let duplicate = create_unique_named_checked(
            &data,
            ("game", "Game"),
            &roots,
            "Automatic backup",
            1,
            || Status::Stopped,
        )
        .unwrap();
        assert!(!duplicate.created);
        assert_eq!(duplicate.snapshot.id, first.id);
        let history = list(&data, "game").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(
            matching_current(&data, &history, &roots).unwrap(),
            Some(first.id)
        );
        fs::write(saves.join("slot.sav"), b"third").unwrap();
        assert_eq!(
            create(&data, "game", &roots, "", 10, Status::Stopped)
                .unwrap()
                .sequence,
            second.sequence + 1
        );
    }

    #[test]
    fn restore_reuses_existing_safety_backup_and_matches_restored_content() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let first = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"second").unwrap();
        create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        let plan = preview(&data, "game", &first.id, &roots, Status::Stopped).unwrap();
        restore(&data, &plan, 10, Status::Stopped).unwrap();
        let history = list(&data, "game").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(
            matching_current(&data, &history, &roots).unwrap(),
            Some(first.id)
        );
    }

    #[test]
    fn damaged_backup_does_not_suppress_new_backup() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let first = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        fs::write(
            data.join("game").join(&first.id).join("0/slot.sav"),
            b"wrong",
        )
        .unwrap();
        assert_eq!(
            matching_current(&data, &list(&data, "game").unwrap(), &roots).unwrap(),
            None
        );
        assert!(
            create_unique_named_checked(&data, ("game", "Game"), &roots, "", 10, || {
                Status::Stopped
            })
            .unwrap()
            .created
        );
    }

    #[test]
    fn duplicate_detection_accounts_for_file_paths_and_all_roots() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        let extra = temp.path().join("extra");
        fs::create_dir(&saves).unwrap();
        fs::create_dir(&extra).unwrap();
        let roots = vec![display_path(&saves), display_path(&extra)];
        let data = temp.path().join("backups");
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        fs::write(extra.join("system.sav"), b"settings").unwrap();
        create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        fs::rename(saves.join("slot.sav"), saves.join("other.sav")).unwrap();
        assert_eq!(
            matching_current(&data, &list(&data, "game").unwrap(), &roots).unwrap(),
            None
        );
        create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        fs::write(extra.join("system.sav"), b"changed!").unwrap();
        assert!(
            create_unique_named_checked(&data, ("game", "Game"), &roots, "", 10, || {
                Status::Stopped
            })
            .unwrap()
            .created
        );
        assert_eq!(
            matching_current(&data, &list(&data, "game").unwrap(), &roots[..1]).unwrap(),
            None
        );
    }
    #[test]
    fn named_backup_numbers_increase_after_history_is_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"save").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let first = create_named_checked(&data, ("game", "游戏"), &roots, "", 10, || {
            Status::Stopped
        })
        .unwrap();
        delete(&data, "game", &first.id).unwrap();
        let second = create_named_checked(&data, ("game", "游戏"), &roots, "", 10, || {
            Status::Stopped
        })
        .unwrap();
        assert_eq!(second.sequence, 2);
        assert_eq!(second.name, format!("游戏_{}_2", second.created));
    }
    #[test]
    fn legacy_history_migrates_without_renaming_storage_or_resetting_the_counter() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"save").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let first = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        let path = data.join("game").join(&first.id).join("manifest.json");
        let mut legacy = serde_json::to_value(&first).unwrap();
        legacy.as_object_mut().unwrap().remove("sequence");
        legacy.as_object_mut().unwrap().remove("name");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let migrated = list(&data, "game").unwrap().remove(0);
        assert_eq!(migrated.id, first.id);
        assert!(migrated.sequence > 0);
        fs::write(saves.join("slot.sav"), b"new save").unwrap();
        let next =
            create_named_checked(&data, ("game", "Game"), &roots, "", 10, || Status::Stopped)
                .unwrap();
        assert!(next.sequence > migrated.sequence);
    }
    #[test]
    fn retention_keeps_locked_snapshots_and_failed_backup_cannot_prune() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"one").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let first = create(&data, "game", &roots, "locked", 1, Status::Stopped).unwrap();
        update(&data, "game", &first.id, true, "locked").unwrap();
        fs::write(saves.join("slot.sav"), b"two").unwrap();
        create(&data, "game", &roots, "second", 1, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"three").unwrap();
        create(&data, "game", &roots, "third", 1, Status::Stopped).unwrap();
        assert_eq!(list(&data, "game").unwrap().len(), 2);
        fs::remove_file(saves.join("slot.sav")).unwrap();
        assert!(create(&data, "game", &roots, "failed", 1, Status::Stopped).is_err());
        assert_eq!(list(&data, "game").unwrap().len(), 2);
    }
    #[test]
    fn interrupted_restore_rolls_back_only_at_current_confirmed_targets() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let snapshot = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"current").unwrap();
        let plan = preview(&data, "game", &snapshot.id, &roots, Status::Stopped).unwrap();
        let old = temp.path().join(".galgear-restore-test.original");
        fs::rename(&saves, &old).unwrap();
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        write_journal(
            &journal_path(&data, "game").unwrap(),
            &Journal {
                plan,
                originals: vec![display_path(&old)],
                committed: false,
            },
        )
        .unwrap();
        assert!(recover(&data, "game", &[], Status::Stopped).is_err());
        assert!(recover(&data, "game", &roots, Status::Running).is_err());
        recover(&data, "game", &roots, Status::Stopped).unwrap();
        assert_eq!(fs::read(saves.join("slot.sav")).unwrap(), b"current");
    }
    #[test]
    fn changed_saves_after_preview_stop_before_any_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"one").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let snapshot = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        let plan = preview(&data, "game", &snapshot.id, &roots, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"external").unwrap();
        assert!(restore(&data, &plan, 10, Status::Stopped).is_err());
        assert_eq!(fs::read(saves.join("slot.sav")).unwrap(), b"external");
    }
    #[test]
    fn complete_restore_removes_new_files_and_keeps_safety_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let snapshot = create(&data, "game", &roots, "first", 10, Status::Stopped).unwrap();
        fs::write(saves.join("slot.sav"), b"second").unwrap();
        fs::write(saves.join("new.sav"), b"new").unwrap();
        let plan = preview(&data, "game", &snapshot.id, &roots, Status::Stopped).unwrap();
        restore(&data, &plan, 10, Status::Stopped).unwrap();
        assert_eq!(fs::read(saves.join("slot.sav")).unwrap(), b"first");
        assert!(!saves.join("new.sav").exists());
        assert_eq!(list(&data, "game").unwrap().len(), 2);
    }
    #[test]
    fn failure_keeps_previous_snapshots_and_blocks_unknown_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let roots = vec![display_path(temp.path())];
        assert!(create(temp.path(), "game", &roots, "", 1, Status::Unknown).is_err());
        assert!(create(temp.path(), "game", &roots, "", 1, Status::Stopped).is_err());
    }
    #[test]
    fn corrupted_snapshot_cannot_modify_current_saves() {
        let temp = tempfile::tempdir().unwrap();
        let saves = temp.path().join("save");
        fs::create_dir(&saves).unwrap();
        fs::write(saves.join("slot.sav"), b"first").unwrap();
        let roots = vec![display_path(&saves)];
        let data = temp.path().join("backups");
        let snapshot = create(&data, "game", &roots, "", 10, Status::Stopped).unwrap();
        fs::write(
            data.join("game").join(&snapshot.id).join("0/slot.sav"),
            b"bad",
        )
        .unwrap();
        fs::write(saves.join("slot.sav"), b"current").unwrap();
        let plan = preview(&data, "game", &snapshot.id, &roots, Status::Stopped).unwrap();
        assert!(restore(&data, &plan, 10, Status::Stopped).is_err());
        assert_eq!(fs::read(saves.join("slot.sav")).unwrap(), b"current");
    }
}
