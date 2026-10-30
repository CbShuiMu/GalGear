//! Patches stored as unpacked directories in a game's `patch` folder.
use super::{Change, Plan, Preview, error, file_hash, is_link, relative, target};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

/// One patch directory and the action implied by the current game resources.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub files: usize,
    pub replaces_existing: bool,
    pub error: Option<Message>,
}

/// The patch storage location and its immediate child directories.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub folder: String,
    pub patches: Vec<Entry>,
}

/// Creates `patch` inside an existing game folder and returns its absolute path.
pub fn directory(folder: &Path) -> Result<PathBuf, Message> {
    let root = fs::canonicalize(folder).map_err(error)?;
    if !root.is_dir() {
        return Err(Message::new("error.game_folder"));
    }
    let path = root.join("patch");
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(error(e)),
    }
    let metadata = fs::symlink_metadata(&path).map_err(error)?;
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(Message::new("local_patches.unsafe"));
    }
    Ok(path)
}

fn source(storage: &Path, name: &str) -> Result<PathBuf, Message> {
    let name = relative(name)?;
    if name.components().count() != 1 {
        return Err(Message::new("local_patches.unsafe"));
    }
    let path = storage.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(error)?;
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(Message::new("local_patches.unsafe"));
    }
    Ok(path)
}

fn files(source: &Path, root: &Path) -> Result<Vec<PathBuf>, Message> {
    let mut pending = vec![source.to_path_buf()];
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(error)? {
            let entry = entry.map_err(error)?;
            entries += 1;
            if entries > 10000 {
                return Err(error("Patch contains too many entries"));
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(error)?;
            if is_link(&metadata) {
                return Err(Message::new("local_patches.unsafe"));
            }
            let path = path.strip_prefix(source).map_err(error)?;
            let name = path
                .to_str()
                .ok_or_else(|| Message::new("local_patches.unsafe"))?;
            relative(name)?;
            // Never let patch contents overwrite the directory holding other patches.
            if matches!(path.components().next(), Some(Component::Normal(part)) if part.to_str().is_some_and(|part| part.eq_ignore_ascii_case("patch")))
                || !seen.insert(name.to_lowercase())
            {
                return Err(Message::new("local_patches.unsafe"));
            }
            if metadata.is_dir() {
                let destination = root.join(path);
                if destination.exists() && !destination.is_dir() {
                    return Err(error(format!("Target is not a directory: {name}")));
                }
                pending.push(source.join(path));
            } else if metadata.is_file() {
                target(root, name)?;
                files.push(path.to_path_buf());
            } else {
                return Err(Message::new("local_patches.unsafe"));
            }
        }
    }
    files.sort();
    Ok(files)
}

fn matching_files(root: &Path, files: &[PathBuf]) -> Result<Vec<PathBuf>, Message> {
    let mut directories = BTreeMap::new();
    for file in files {
        if let (Some(parent), Some(stem)) = (file.parent(), file.file_stem()) {
            directories
                .entry(parent)
                .or_insert_with(BTreeSet::new)
                .insert(stem.to_string_lossy().to_lowercase());
        }
    }
    let mut matches = Vec::new();
    for (directory, stems) in directories {
        let entries = match fs::read_dir(root.join(directory)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(error(e)),
        };
        for entry in entries {
            let entry = entry.map_err(error)?;
            if entry.file_type().map_err(error)?.is_file()
                && entry
                    .path()
                    .file_stem()
                    .is_some_and(|stem| stems.contains(&stem.to_string_lossy().to_lowercase()))
            {
                matches.push(directory.join(entry.file_name()));
            }
        }
    }
    matches.sort();
    Ok(matches)
}

/// Lists patch subdirectories; invalid patches stay visible with a disabled action.
pub fn list(folder: &Path) -> Result<Report, Message> {
    let storage = directory(folder)?;
    let root = storage
        .parent()
        .ok_or_else(|| error("Missing game folder"))?;
    let mut patches = Vec::new();
    for entry in fs::read_dir(&storage).map_err(error)? {
        let entry = entry.map_err(error)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
        if !metadata.is_dir() || is_link(&metadata) {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with(".galgear") {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Message::new("local_patches.unsafe"))?;
        let contents = source(&storage, &name)
            .and_then(|source| files(&source, root))
            .and_then(|files| Ok((!matching_files(root, &files)?.is_empty(), files)));
        let mut patch = Entry {
            name,
            files: 0,
            replaces_existing: false,
            error: None,
        };
        match contents {
            Ok((replaces_existing, files)) => {
                patch.files = files.len();
                patch.replaces_existing = replaces_existing;
                if files.is_empty() {
                    patch.error = Some(Message::new("local_patches.empty_patch"));
                }
            }
            Err(error) => patch.error = Some(error),
        }
        patches.push(patch);
    }
    patches.sort_by_cached_key(|patch| patch.name.to_lowercase());
    Ok(Report {
        folder: crate::config::display_path(&storage),
        patches,
    })
}

/// Stages a fresh copy for the existing backed-up, hash-checked installer.
/// Large resources are streamed to disk and the source patch remains intact.
pub fn preview(folder: &Path, name: &str, game_id: String) -> Result<Plan, Message> {
    let storage = directory(folder)?;
    let root = storage
        .parent()
        .ok_or_else(|| error("Missing game folder"))?
        .to_path_buf();
    let source = source(&storage, name)?;
    let files = files(&source, &root)?;
    if files.is_empty() {
        return Err(Message::new("local_patches.empty_patch"));
    }
    let incoming: BTreeSet<_> = files
        .iter()
        .map(|file| file.to_string_lossy().to_lowercase())
        .collect();
    let replaced = matching_files(&root, &files)?;
    let staging = tempfile::tempdir().map_err(error)?;
    let mut preview = Preview {
        title: name.into(),
        game_id,
        target: crate::config::display_path(&root),
        files: Vec::with_capacity(files.len()),
        executable: None,
    };
    for (index, file) in files.iter().enumerate() {
        let name = file
            .to_str()
            .ok_or_else(|| Message::new("local_patches.unsafe"))?;
        let destination = target(&root, name)?;
        let staged = staging.path().join(index.to_string());
        let size = fs::copy(source.join(file), &staged).map_err(error)?;
        preview.files.push(Change {
            path: name.into(),
            before: if destination.exists() {
                Some(file_hash(&destination)?)
            } else {
                None
            },
            after: Some(file_hash(&staged)?),
            size,
        });
    }
    // Keep removals in the same backup record so switches can be reversed in full.
    for file in replaced {
        if incoming.contains(&file.to_string_lossy().to_lowercase()) {
            continue;
        }
        let name = file
            .to_str()
            .ok_or_else(|| Message::new("local_patches.unsafe"))?;
        preview.files.push(Change {
            path: name.into(),
            before: Some(file_hash(&target(&root, name)?)?),
            after: None,
            size: 0,
        });
    }
    Ok(Plan {
        preview,
        staging,
        root,
        reuse_patch_files: true,
    })
}

pub(super) fn matching_backups(
    root: &Path,
    changes: &[Change],
) -> Result<BTreeMap<usize, String>, Message> {
    let mut wanted = BTreeMap::<String, Vec<usize>>::new();
    let mut sizes = BTreeSet::new();
    for (index, change) in changes.iter().enumerate() {
        if let Some(hash) = &change.before {
            wanted.entry(hash.clone()).or_default().push(index);
            sizes.insert(
                fs::metadata(target(root, &change.path)?)
                    .map_err(error)?
                    .len(),
            );
        }
    }
    let storage = directory(root)?;
    let mut matches = BTreeMap::new();
    for entry in fs::read_dir(&storage).map_err(error)? {
        if wanted.is_empty() {
            break;
        }
        let entry = entry.map_err(error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".galgear") {
            continue;
        }
        let Ok(patch) = source(&storage, &name) else {
            continue;
        };
        let Ok(contents) = files(&patch, root) else {
            continue;
        };
        for file in contents {
            let path = patch.join(file);
            if !fs::metadata(&path).is_ok_and(|metadata| sizes.contains(&metadata.len())) {
                continue;
            }
            let hash = file_hash(&path)?;
            if let Some(indices) = wanted.remove(&hash) {
                let relative = path
                    .strip_prefix(root)
                    .map_err(error)?
                    .to_string_lossy()
                    .into_owned();
                for index in indices {
                    matches.insert(index, relative.clone());
                }
            }
        }
    }
    Ok(matches)
}

/// Keep originals as ordinary patch files, so they can also be switched back from the card.
pub(super) fn backup_originals(
    root: &Path,
    patch_name: &str,
    changes: &[Change],
    sources: &mut BTreeMap<usize, String>,
) -> Result<(), Message> {
    let missing: Vec<_> = changes.iter().enumerate()
        .filter(|(index, change)| change.before.is_some() && !sources.contains_key(index))
        .collect();
    if missing.is_empty() { return Ok(()); }
    let name = relative(&format!("{patch_name}_original"))?;
    if name.components().count() != 1 { return Err(Message::new("local_patches.unsafe")); }
    let storage = directory(root)?;
    let staging = tempfile::Builder::new().prefix(".galgear-original-").tempdir_in(&storage).map_err(error)?;
    for (_, change) in &missing {
        let destination = staging.path().join(relative(&change.path)?);
        super::atomic_copy(&destination, &target(root, &change.path)?)?;
        if Some(file_hash(&destination)?) != change.before {
            return Err(error("Game files changed while backing up originals"));
        }
    }
    // Keep older backups intact when this patch is installed over different originals later.
    for suffix in 1..=10000 {
        let folder = if suffix == 1 { storage.join(&name) } else {
            storage.join(format!("{}_{}", name.to_string_lossy(), suffix))
        };
        if fs::symlink_metadata(&folder).is_ok() { continue; }
        match fs::rename(staging.path(), &folder) {
            Ok(()) => {
                for (index, change) in missing {
                    sources.insert(index, folder.join(relative(&change.path)?)
                        .strip_prefix(root).map_err(error)?.to_string_lossy().into_owned());
                }
                return Ok(());
            }
            Err(_) if fs::symlink_metadata(&folder).is_ok() => continue,
            Err(failure) => return Err(error(failure)),
        }
    }
    Err(error("Too many original patch backups"))
}

#[cfg(test)]
mod tests {
    use super::super::{install, restore};
    use super::*;

    #[test]
    fn named_original_folder_keeps_all_names_and_can_be_switched_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(root.join("data")).unwrap();
        let storage = directory(&root).unwrap();
        fs::create_dir_all(storage.join("汉化补丁/data")).unwrap();
        fs::write(storage.join("汉化补丁/data/script.chs"), b"translated").unwrap();
        fs::write(root.join("data/script.dat"), b"original data").unwrap();
        fs::write(root.join("data/script.idx"), b"original index").unwrap();
        let records = temp.path().join("records");
        let installed = install(preview(&root, "汉化补丁", "game".into()).unwrap(), &records).unwrap();
        for (name, bytes) in [("script.dat", b"original data".as_slice()), ("script.idx", b"original index".as_slice())] {
            assert_eq!(fs::read(storage.join("汉化补丁_original/data").join(name)).unwrap(), bytes);
        }
        assert!(fs::read_dir(records.join(&installed.id)).unwrap().all(|file| file.unwrap().path().extension().is_none_or(|ext| ext != "bak")));
        assert!(list(&root).unwrap().patches.iter().any(|patch| patch.name == "汉化补丁_original" && patch.replaces_existing && patch.files == 2));
        install(preview(&root, "汉化补丁_original", "game".into()).unwrap(), &records).unwrap();
        assert!(!root.join("data/script.chs").exists());
        assert_eq!(fs::read(root.join("data/script.dat")).unwrap(), b"original data");
        assert_eq!(fs::read(root.join("data/script.idx")).unwrap(), b"original index");
    }

    #[test]
    fn later_switches_preserve_existing_originals_and_reuse_identical_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let storage = directory(root).unwrap();
        fs::create_dir(storage.join("Patch")).unwrap();
        fs::write(storage.join("Patch/script.dat"), b"patched").unwrap();
        fs::write(root.join("script.dat"), b"first original").unwrap();
        let records = root.join("records");
        install(preview(root, "Patch", "game".into()).unwrap(), &records).unwrap();
        fs::write(root.join("script.dat"), b"second original").unwrap();
        let second = install(preview(root, "Patch", "game".into()).unwrap(), &records).unwrap();
        assert_eq!(fs::read(storage.join("Patch_original/script.dat")).unwrap(), b"first original");
        assert_eq!(fs::read(storage.join("Patch_original_2/script.dat")).unwrap(), b"second original");
        restore(&records, &second.id).unwrap();
        install(preview(root, "Patch", "game".into()).unwrap(), &records).unwrap();
        assert!(!storage.join("Patch_original_3").exists());
    }

    #[test]
    fn reuses_hash_identical_patch_files_but_backs_up_unique_originals() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let storage = directory(root).unwrap();
        fs::create_dir(storage.join("Original")).unwrap();
        fs::create_dir(storage.join("New")).unwrap();
        fs::write(root.join("a.dat"), b"original a").unwrap();
        fs::write(root.join("b.dat"), b"original b").unwrap();
        fs::write(storage.join("Original/different-name.dat"), b"original a").unwrap();
        fs::write(storage.join("Original/b.dat"), b"different!").unwrap();
        fs::write(storage.join("New/a.dat"), b"new a").unwrap();
        fs::write(storage.join("New/b.dat"), b"new b").unwrap();
        let data = root.join("backups");
        let installed = install(preview(root, "New", "game".into()).unwrap(), &data).unwrap();
        assert!(!data.join(&installed.id).join("0.bak").exists());
        assert_eq!(
            fs::read(storage.join("New_original/b.dat")).unwrap(),
            b"original b"
        );
        assert_eq!(installed.backup_sources.len(), 2);
        assert!(!storage.join("New_original/a.dat").exists());
        assert!(!data.join(&installed.id).join("1.bak").exists());
        restore(&data, &installed.id).unwrap();
        assert_eq!(fs::read(root.join("a.dat")).unwrap(), b"original a");
        assert_eq!(fs::read(root.join("b.dat")).unwrap(), b"original b");
    }

    #[test]
    fn changed_reused_backup_stops_restoration_before_any_game_file_is_written() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let storage = directory(root).unwrap();
        for (name, content) in [("Original", b"old"), ("New", b"new")] {
            fs::create_dir(storage.join(name)).unwrap();
            fs::write(storage.join(name).join("script.dat"), content).unwrap();
        }
        fs::write(root.join("script.dat"), b"old").unwrap();
        let data = root.join("backups");
        let installed = install(preview(root, "New", "game".into()).unwrap(), &data).unwrap();
        fs::write(storage.join("Original/script.dat"), b"changed").unwrap();
        assert!(restore(&data, &installed.id).is_err());
        assert_eq!(fs::read(root.join("script.dat")).unwrap(), b"new");
    }

    #[test]
    fn creates_storage_and_lists_only_patch_directories() {
        let temp = tempfile::tempdir().unwrap();
        assert!(list(temp.path()).unwrap().patches.is_empty());
        let storage = temp.path().join("patch");
        fs::create_dir(storage.join("Empty")).unwrap();
        fs::write(storage.join("readme.txt"), b"ignored").unwrap();
        let report = list(temp.path()).unwrap();
        assert_eq!(report.patches.len(), 1);
        assert_eq!(
            report.patches[0].error.as_ref().unwrap().key,
            "local_patches.empty_patch"
        );
    }

    #[test]
    fn lists_matching_file_stems_as_switches_regardless_of_extension() {
        for (existing, incoming) in [
            ("script.noa", "script.csx"),
            ("script.dat", "script.xp3"),
            ("data/script.dat", "data/script.xp3"),
            ("data/script.dat", "data/script"),
            ("data/script", "data/script.xp3"),
            ("data/script.v1.dat", "data/script.v1.xp3"),
            ("data/SCRIPT.dat", "data/script.xp3"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let patch = directory(root).unwrap().join("Patch");
            fs::create_dir_all(root.join(existing).parent().unwrap()).unwrap();
            fs::write(root.join(existing), b"existing").unwrap();
            fs::create_dir_all(patch.join(incoming).parent().unwrap()).unwrap();
            fs::write(patch.join(incoming), b"patch").unwrap();

            assert!(
                list(root).unwrap().patches[0].replaces_existing,
                "{incoming} should match {existing}"
            );
        }
    }

    #[test]
    fn lists_new_files_as_installs_despite_existing_directories_and_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let patch = directory(root).unwrap().join("Patch/data");
        fs::create_dir_all(&patch).unwrap();
        fs::write(patch.join("script.xp3"), b"patch").unwrap();
        fs::create_dir_all(root.join("data/script.dat")).unwrap();
        fs::write(root.join("data/other.dat"), b"unrelated").unwrap();
        fs::write(root.join("data/script.v1.dat"), b"different stem").unwrap();
        fs::write(root.join("script.dat"), b"different directory").unwrap();

        assert!(!list(root).unwrap().patches[0].replaces_existing);
    }

    #[test]
    fn installs_switches_and_restores_nested_resources_without_changing_sources() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir(&root).unwrap();
        let storage = directory(&root).unwrap();
        for (name, bytes) in [
            ("中文补丁", b"first".as_slice()),
            ("Second", b"second".as_slice()),
        ] {
            let patch = storage.join(name).join("data");
            fs::create_dir_all(&patch).unwrap();
            fs::write(patch.join("script.dat"), bytes).unwrap();
        }
        fs::write(root.join("unrelated.dat"), b"keep").unwrap();
        assert!(
            list(&root)
                .unwrap()
                .patches
                .iter()
                .all(|patch| !patch.replaces_existing)
        );
        let backups = temp.path().join("backups");
        let first = install(preview(&root, "中文补丁", "game".into()).unwrap(), &backups).unwrap();
        assert_eq!(fs::read(root.join("data/script.dat")).unwrap(), b"first");
        assert!(
            list(&root)
                .unwrap()
                .patches
                .iter()
                .all(|patch| patch.replaces_existing)
        );
        let second = install(preview(&root, "Second", "game".into()).unwrap(), &backups).unwrap();
        assert_eq!(fs::read(root.join("data/script.dat")).unwrap(), b"second");
        assert!(!backups.join(&second.id).join("0.bak").exists());
        assert_eq!(
            fs::read(root.join(&second.backup_sources[&0])).unwrap(),
            b"first"
        );
        assert_eq!(
            fs::read(storage.join("Second/data/script.dat")).unwrap(),
            b"second"
        );
        restore(&backups, &second.id).unwrap();
        assert_eq!(fs::read(root.join("data/script.dat")).unwrap(), b"first");
        restore(&backups, &first.id).unwrap();
        assert!(!root.join("data/script.dat").exists());
        assert_eq!(fs::read(root.join("unrelated.dat")).unwrap(), b"keep");
    }

    #[test]
    fn switches_extensions_and_restores_original_resources_in_reverse_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join("data/script.noa"), b"original").unwrap();
        fs::write(root.join("script.noa"), b"other directory").unwrap();
        fs::write(root.join("data/other.noa"), b"other resource").unwrap();
        let storage = directory(&root).unwrap();
        for (patch, file, bytes) in [
            ("First", "script.csx", b"first".as_slice()),
            ("Second", "script.noa", b"second".as_slice()),
        ] {
            fs::create_dir_all(storage.join(patch).join("data")).unwrap();
            fs::write(storage.join(patch).join("data").join(file), bytes).unwrap();
        }
        let backups = temp.path().join("backups");
        let first = install(preview(&root, "First", "game".into()).unwrap(), &backups).unwrap();
        assert!(!root.join("data/script.noa").exists());
        assert_eq!(fs::read(root.join("data/script.csx")).unwrap(), b"first");
        assert_eq!(
            fs::read(storage.join("First_original/data/script.noa")).unwrap(),
            b"original"
        );
        let second = install(preview(&root, "Second", "game".into()).unwrap(), &backups).unwrap();
        assert!(!root.join("data/script.csx").exists());
        assert_eq!(fs::read(root.join("data/script.noa")).unwrap(), b"second");
        assert!(restore(&backups, &first.id).is_err());
        restore(&backups, &second.id).unwrap();
        assert!(!root.join("data/script.noa").exists());
        assert_eq!(fs::read(root.join("data/script.csx")).unwrap(), b"first");
        restore(&backups, &first.id).unwrap();
        assert!(!root.join("data/script.csx").exists());
        assert_eq!(fs::read(root.join("data/script.noa")).unwrap(), b"original");
        assert_eq!(
            fs::read(root.join("script.noa")).unwrap(),
            b"other directory"
        );
        assert_eq!(
            fs::read(root.join("data/other.noa")).unwrap(),
            b"other resource"
        );
        assert_eq!(
            fs::read(storage.join("First/data/script.csx")).unwrap(),
            b"first"
        );
    }

    #[test]
    fn keeps_all_incoming_extensions_and_backs_up_all_replaced_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let patch = directory(root).unwrap().join("Patch");
        fs::create_dir(&patch).unwrap();
        for name in ["script.csx", "script.idx"] {
            fs::write(patch.join(name), b"patch").unwrap();
        }
        for name in ["script.noa", "script.xp3", "script.csx"] {
            fs::write(root.join(name), b"original").unwrap();
        }
        let backups = root.join("backups");
        let record = install(preview(root, "Patch", "game".into()).unwrap(), &backups).unwrap();
        assert!(!root.join("script.noa").exists());
        assert!(!root.join("script.xp3").exists());
        assert_eq!(fs::read(root.join("script.csx")).unwrap(), b"patch");
        assert_eq!(fs::read(root.join("script.idx")).unwrap(), b"patch");
        restore(&backups, &record.id).unwrap();
        for name in ["script.noa", "script.xp3", "script.csx"] {
            assert_eq!(fs::read(root.join(name)).unwrap(), b"original");
        }
        assert!(!root.join("script.idx").exists());
    }

    #[test]
    fn rejects_changed_replacement_targets_before_installation_or_restoration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let patch = directory(root).unwrap().join("Patch");
        fs::create_dir(&patch).unwrap();
        fs::write(patch.join("script.csx"), b"patch").unwrap();
        fs::write(root.join("script.noa"), b"original").unwrap();
        let backups = root.join("backups");
        let plan = preview(root, "Patch", "game".into()).unwrap();
        fs::write(root.join("script.noa"), b"changed").unwrap();
        assert!(install(plan, &backups).is_err());
        assert!(!root.join("script.csx").exists());
        assert_eq!(fs::read(root.join("script.noa")).unwrap(), b"changed");
        let record = install(preview(root, "Patch", "game".into()).unwrap(), &backups).unwrap();
        fs::write(root.join("script.noa"), b"external").unwrap();
        assert!(restore(&backups, &record.id).is_err());
        assert_eq!(fs::read(root.join("script.csx")).unwrap(), b"patch");
        assert_eq!(fs::read(root.join("script.noa")).unwrap(), b"external");
    }

    #[test]
    fn rejects_path_traversal_and_patch_storage_targets_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let storage = directory(temp.path()).unwrap();
        fs::create_dir_all(storage.join("Bad/patch/Other")).unwrap();
        fs::write(storage.join("Bad/patch/Other/file"), b"bad").unwrap();
        assert!(preview(temp.path(), "../outside", "game".into()).is_err());
        assert!(preview(temp.path(), "Bad", "game".into()).is_err());
        assert!(list(temp.path()).unwrap().patches[0].error.is_some());
    }

    #[test]
    fn rejects_file_directory_conflicts_and_stale_targets_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let storage = directory(temp.path()).unwrap();
        fs::create_dir(storage.join("Patch")).unwrap();
        fs::write(storage.join("Patch/data"), b"new").unwrap();
        fs::create_dir(temp.path().join("data")).unwrap();
        assert!(preview(temp.path(), "Patch", "game".into()).is_err());
        fs::remove_dir(temp.path().join("data")).unwrap();
        let plan = preview(temp.path(), "Patch", "game".into()).unwrap();
        fs::write(temp.path().join("data"), b"changed").unwrap();
        assert!(install(plan, &temp.path().join("backups")).is_err());
        assert_eq!(fs::read(temp.path().join("data")).unwrap(), b"changed");
    }
}
