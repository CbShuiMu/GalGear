use super::{Snapshot, copy_file, directory, error, hash, relative, safe_id, write_sequence};
use galgear_i18n::Message;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Root for save backups belonging to this installed game.
pub fn game_data(game_folder: &Path) -> PathBuf {
    game_folder.join(".galgear-backups").join("saves")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    File { size: u64, hash: String },
}
type Tree = BTreeMap<PathBuf, Entry>;

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions are reparse points even when is_symlink() is false.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn reject_links(path: &Path) -> Result<(), Message> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(error(format!(
                    "Backup path contains a link: {}",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(error(err)),
        }
    }
    Ok(())
}

fn tree(root: &Path) -> Result<Tree, Message> {
    fn visit(root: &Path, path: &Path, result: &mut Tree) -> Result<(), Message> {
        for entry in fs::read_dir(path).map_err(error)? {
            let entry = entry.map_err(error)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
            if is_link(&metadata) {
                return Err(error("Backup contents contain a link"));
            }
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(error)?.to_owned();
            if metadata.is_dir() {
                result.insert(relative, Entry::Directory);
                visit(root, &path, result)?;
            } else if metadata.is_file() {
                result.insert(
                    relative,
                    Entry::File {
                        size: metadata.len(),
                        hash: hash(&path)?,
                    },
                );
            } else {
                return Err(error("Backup contains a special file"));
            }
        }
        Ok(())
    }
    reject_links(root)?;
    let mut result = BTreeMap::new();
    visit(root, root, &mut result)?;
    Ok(result)
}

fn verify_snapshot(path: &Path, game: &str) -> Result<(Snapshot, Tree), Message> {
    let contents = tree(path)?;
    let snapshot: Snapshot =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).map_err(error)?)
            .map_err(error)?;
    safe_id(&snapshot.id)?;
    if snapshot.game_id != game || path.file_name() != Some(snapshot.id.as_ref()) {
        return Err(error(
            "Backup manifest identity does not match its directory",
        ));
    }
    let mut identities = BTreeSet::new();
    let mut size = 0u64;
    for file in &snapshot.files {
        let relative = relative(&file.path)?;
        if file.root >= snapshot.roots.len()
            || !identities.insert((file.root, relative.to_string_lossy().to_lowercase()))
            || contents.get(&PathBuf::from(file.root.to_string()).join(relative))
                != Some(&Entry::File {
                    size: file.size,
                    hash: file.hash.clone(),
                })
        {
            return Err(error("Backup contents failed migration verification"));
        }
        size = size
            .checked_add(file.size)
            .ok_or_else(|| error("Invalid backup size"))?;
    }
    if size != snapshot.size {
        return Err(error("Backup manifest size does not match its contents"));
    }
    Ok((snapshot, contents))
}

fn counter(path: &Path) -> Result<Option<(u64, Vec<u8>)>, Message> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some((
            serde_json::from_slice(&bytes).map_err(error)?,
            bytes,
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(error(err)),
    }
}

fn ensure_directory(path: &Path) -> Result<(), Message> {
    reject_links(path)?;
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(err) => Err(error(err)),
    }
}

fn publish_file(path: &Path, bytes: &[u8]) -> Result<(), Message> {
    if path.exists() {
        if fs::read(path).map_err(error)? == bytes {
            return Ok(());
        }
        return Err(error(
            "Existing restore journal conflicts with the old backup location",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| error("Missing backup parent"))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    staged.write_all(bytes).map_err(error)?;
    staged.as_file().sync_all().map_err(error)?;
    staged.persist_noclobber(path).map_err(error)?;
    Ok(())
}

/// Move one game's old history only after publishing and verifying complete copies.
/// A failed or interrupted call retains old files that still need migration and can be retried.
pub fn migrate(legacy_data: &Path, data: &Path, game_id: &str) -> Result<usize, Message> {
    let source = directory(legacy_data, game_id)?;
    // Validate the new root even when there is no old history to migrate, since
    // callers use this check before creating their first local snapshot.
    reject_links(&directory(data, game_id)?)?;
    match fs::symlink_metadata(&source) {
        Ok(metadata) if !metadata.is_dir() || is_link(&metadata) => {
            return Err(error("Old backup location must be an ordinary directory"));
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(error(err)),
    }
    reject_links(&source)?;
    let game_folder = data
        .parent()
        .and_then(Path::parent)
        .filter(|folder| game_data(folder) == data)
        .ok_or_else(|| error("Backups must be stored in the installed game directory"))?;
    if !game_folder.is_dir() {
        return Err(error(
            "Game folder is unavailable; old backups were retained",
        ));
    }
    let source = fs::canonicalize(source).map_err(error)?;
    let game_folder = fs::canonicalize(game_folder).map_err(error)?;
    let destination = directory(&game_data(&game_folder), game_id)?;
    reject_links(&destination)?;
    if destination.exists() && fs::canonicalize(&destination).map_err(error)? == source {
        return Ok(0);
    }
    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err(error("Old and new backup directories overlap"));
    }

    // Preflight the complete history before committing any move.
    let mut snapshots = Vec::new();
    let mut maximum_sequence = 0;
    for entry in fs::read_dir(&source).map_err(error)? {
        let entry = entry.map_err(error)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
        if is_link(&metadata) {
            return Err(error("Old backup directory contains a link"));
        }
        if metadata.is_dir()
            && !entry.file_name().to_string_lossy().starts_with('.')
            && entry.path().join("manifest.json").exists()
        {
            let (snapshot, contents) = verify_snapshot(&entry.path(), game_id)?;
            let target = destination.join(&snapshot.id);
            if target.exists() && tree(&target)? != contents {
                return Err(error(format!(
                    "Conflicting backup {}; old copy was retained",
                    snapshot.id
                )));
            }
            maximum_sequence = maximum_sequence.max(snapshot.sequence);
            snapshots.push((snapshot.id, contents));
        }
    }
    let sequence_source = source.join("sequence.json");
    let old_counter = counter(&sequence_source)?;
    maximum_sequence = maximum_sequence.max(old_counter.as_ref().map_or(0, |(n, _)| *n));
    maximum_sequence = maximum_sequence.max(
        counter(&destination.join("sequence.json"))?
            .as_ref()
            .map_or(0, |(n, _)| *n),
    );
    if destination.exists() {
        reject_links(&destination)?;
        for entry in fs::read_dir(&destination).map_err(error)? {
            let entry = entry.map_err(error)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
            if is_link(&metadata) {
                return Err(error("New backup directory contains a link"));
            }
            if metadata.is_dir()
                && !entry.file_name().to_string_lossy().starts_with('.')
                && entry.path().join("manifest.json").exists()
            {
                let (snapshot, _) = verify_snapshot(&entry.path(), game_id)?;
                maximum_sequence = maximum_sequence.max(snapshot.sequence);
            }
        }
    }
    let journal_source = source.join("restore-journal.json");
    let journal = match fs::read(&journal_source) {
        Ok(bytes) => {
            let journal: super::Journal = serde_json::from_slice(&bytes).map_err(error)?;
            if journal.plan.snapshot.game_id != game_id {
                return Err(error("Restore journal belongs to a different game"));
            }
            let target = destination.join("restore-journal.json");
            if target.exists() && fs::read(target).map_err(error)? != bytes {
                return Err(error(
                    "Existing restore journal conflicts with the old backup location",
                ));
            }
            Some(bytes)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(error(err)),
    };

    ensure_directory(&game_folder.join(".galgear-backups"))?;
    ensure_directory(&game_data(&game_folder))?;
    ensure_directory(&destination)?;
    for (id, contents) in &snapshots {
        let target = destination.join(id);
        if target.exists() {
            continue;
        }
        let stage = tempfile::Builder::new()
            .prefix(".migrating-")
            .tempdir_in(&destination)
            .map_err(error)?;
        for (relative, entry) in contents {
            let to = stage.path().join(relative);
            match entry {
                Entry::Directory => fs::create_dir(&to).map_err(error)?,
                Entry::File { .. } => copy_file(&source.join(id).join(relative), &to)?,
            }
        }
        if tree(stage.path())? != *contents || tree(&source.join(id))? != *contents {
            return Err(error(
                "Backup changed while migrating; old copy was retained",
            ));
        }
        fs::rename(stage.path(), &target).map_err(error)?;
    }
    if let Some(bytes) = &journal {
        publish_file(&destination.join("restore-journal.json"), bytes)?;
    }
    write_sequence(&destination, maximum_sequence)?;

    for (id, contents) in &snapshots {
        let old = source.join(id);
        // Check the resolved recursive-delete target remains an immediate child of
        // the verified old game backup directory, including on Windows junctions.
        reject_links(&old)?;
        let resolved = fs::canonicalize(&old).map_err(error)?;
        if resolved.parent() != Some(source.as_path())
            || tree(&resolved)? != *contents
            || tree(&destination.join(id))? != *contents
        {
            return Err(error(
                "Backup changed before cleanup; old copy was retained",
            ));
        }
        fs::remove_dir_all(&resolved).map_err(error)?;
    }
    if let Some(bytes) = &journal {
        if fs::read(&journal_source).map_err(error)? != *bytes
            || fs::read(destination.join("restore-journal.json")).map_err(error)? != *bytes
        {
            return Err(error("Restore journal changed before cleanup"));
        }
        fs::remove_file(journal_source).map_err(error)?;
    }
    if let Some((_, bytes)) = old_counter {
        if fs::read(&sequence_source).map_err(error)? != bytes {
            return Err(error("Old backup counter changed before cleanup"));
        }
        fs::remove_file(sequence_source).map_err(error)?;
    }
    // Unknown files and incomplete snapshots stay available for manual recovery.
    if fs::read_dir(&source).map_err(error)?.next().is_none() {
        fs::remove_dir(&source).map_err(error)?;
    }
    Ok(snapshots.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::display_path, runtime::Status, snapshots};

    struct Fixture {
        _temp: tempfile::TempDir,
        legacy: PathBuf,
        data: PathBuf,
        roots: Vec<String>,
        snapshot: Snapshot,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let game = temp.path().join("游戏");
            let saves = game.join("savedata");
            fs::create_dir_all(&saves).unwrap();
            fs::write(saves.join("slot.sav"), b"saved contents").unwrap();
            let roots = vec![display_path(&saves)];
            let legacy = temp.path().join("legacy");
            let snapshot = snapshots::create_named_checked(
                &legacy,
                ("game", "游戏"),
                &roots,
                "note",
                10,
                || Status::Stopped,
            )
            .unwrap();
            Self {
                _temp: temp,
                legacy,
                data: game_data(&game),
                roots,
                snapshot,
            }
        }

        fn old(&self) -> PathBuf {
            self.legacy.join("game")
        }
        fn target(&self) -> PathBuf {
            self.data.join("game")
        }
    }

    #[test]
    fn migration_preserves_manifests_contents_and_monotonic_numbers() {
        let fixture = Fixture::new();
        snapshots::update(
            &fixture.legacy,
            "game",
            &fixture.snapshot.id,
            true,
            "locked note",
        )
        .unwrap();
        fs::write(fixture.old().join("sequence.json"), b"50").unwrap();
        let previous = tree(&fixture.old()).unwrap();
        assert_eq!(migrate(&fixture.legacy, &fixture.data, "game").unwrap(), 1);
        assert_eq!(tree(&fixture.target()).unwrap(), previous);
        assert!(!fixture.old().exists());
        let next = snapshots::create_named_checked(
            &fixture.data,
            ("game", "游戏"),
            &fixture.roots,
            "",
            10,
            || Status::Stopped,
        )
        .unwrap();
        assert_eq!(next.sequence, 51);
        assert_eq!(migrate(&fixture.legacy, &fixture.data, "game").unwrap(), 0);
    }

    #[test]
    fn retry_accepts_an_identical_published_snapshot_and_keeps_larger_target_counter() {
        let fixture = Fixture::new();
        let old_tree = tree(&fixture.old()).unwrap();
        fs::create_dir_all(fixture.target()).unwrap();
        for (relative, entry) in old_tree {
            match entry {
                Entry::Directory => fs::create_dir(fixture.target().join(relative)).unwrap(),
                Entry::File { .. } => {
                    fs::copy(
                        fixture.old().join(&relative),
                        fixture.target().join(relative),
                    )
                    .unwrap();
                }
            }
        }
        fs::write(fixture.target().join("sequence.json"), b"100").unwrap();
        assert_eq!(migrate(&fixture.legacy, &fixture.data, "game").unwrap(), 1);
        assert_eq!(
            counter(&fixture.target().join("sequence.json"))
                .unwrap()
                .unwrap()
                .0,
            100
        );
    }

    #[test]
    fn conflicting_target_retains_both_original_trees() {
        let fixture = Fixture::new();
        let target = fixture.target().join(&fixture.snapshot.id);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("manifest.json"), b"unrelated").unwrap();
        let source_before = tree(&fixture.old()).unwrap();
        let target_before = tree(&fixture.target()).unwrap();
        assert!(migrate(&fixture.legacy, &fixture.data, "game").is_err());
        assert_eq!(tree(&fixture.old()).unwrap(), source_before);
        assert_eq!(tree(&fixture.target()).unwrap(), target_before);
    }

    #[test]
    fn corrupt_source_stays_in_old_location() {
        let fixture = Fixture::new();
        fs::write(
            fixture.old().join(&fixture.snapshot.id).join("0/slot.sav"),
            b"corrupt",
        )
        .unwrap();
        assert!(migrate(&fixture.legacy, &fixture.data, "game").is_err());
        assert!(fixture.old().join(&fixture.snapshot.id).exists());
        assert!(!fixture.target().exists());
    }

    #[test]
    fn missing_game_does_not_create_a_replacement_install_directory() {
        let fixture = Fixture::new();
        let game = fixture._temp.path().join("missing-game");
        assert!(migrate(&fixture.legacy, &game_data(&game), "game").is_err());
        assert!(!game.exists());
        assert!(fixture.old().exists());
    }

    #[test]
    fn unwritable_destination_keeps_source() {
        let fixture = Fixture::new();
        fs::write(fixture.data.parent().unwrap(), b"blocking file").unwrap();
        assert!(migrate(&fixture.legacy, &fixture.data, "game").is_err());
        assert!(fixture.old().join(&fixture.snapshot.id).exists());
    }

    #[test]
    fn migration_preserves_pending_restore_journal_for_recovery() {
        let fixture = Fixture::new();
        fs::write(Path::new(&fixture.roots[0]).join("slot.sav"), b"current").unwrap();
        let plan = snapshots::preview(
            &fixture.legacy,
            "game",
            &fixture.snapshot.id,
            &fixture.roots,
            Status::Stopped,
        )
        .unwrap();
        let old = Path::new(&fixture.roots[0]).with_file_name(".galgear-restore-test.original");
        fs::rename(&fixture.roots[0], &old).unwrap();
        fs::create_dir(&fixture.roots[0]).unwrap();
        fs::write(
            Path::new(&fixture.roots[0]).join("slot.sav"),
            b"saved contents",
        )
        .unwrap();
        let journal = snapshots::Journal {
            plan,
            originals: vec![display_path(&old)],
            committed: false,
        };
        let bytes = serde_json::to_vec(&journal).unwrap();
        fs::write(fixture.old().join("restore-journal.json"), &bytes).unwrap();
        migrate(&fixture.legacy, &fixture.data, "game").unwrap();
        assert_eq!(
            fs::read(fixture.target().join("restore-journal.json")).unwrap(),
            bytes
        );
        snapshots::recover(&fixture.data, "game", &fixture.roots, Status::Stopped).unwrap();
        assert_eq!(
            fs::read(Path::new(&fixture.roots[0]).join("slot.sav")).unwrap(),
            b"current"
        );
    }

    #[test]
    fn migration_rejects_manifest_path_traversal_without_touching_source() {
        let fixture = Fixture::new();
        let path = fixture
            .old()
            .join(&fixture.snapshot.id)
            .join("manifest.json");
        let mut snapshot = fixture.snapshot.clone();
        snapshot.files[0].path = "../outside.sav".into();
        fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(migrate(&fixture.legacy, &fixture.data, "game").is_err());
        assert!(fixture.old().join(&fixture.snapshot.id).exists());
    }

    #[test]
    fn conflicting_restore_journals_stop_before_moving_history() {
        let fixture = Fixture::new();
        let journal = snapshots::Journal {
            plan: snapshots::preview(
                &fixture.legacy,
                "game",
                &fixture.snapshot.id,
                &fixture.roots,
                Status::Stopped,
            )
            .unwrap(),
            originals: Vec::new(),
            committed: false,
        };
        fs::write(
            fixture.old().join("restore-journal.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(fixture.target()).unwrap();
        fs::write(
            fixture.target().join("restore-journal.json"),
            b"different pending recovery",
        )
        .unwrap();
        let previous = tree(&fixture.old()).unwrap();
        assert!(migrate(&fixture.legacy, &fixture.data, "game").is_err());
        assert_eq!(tree(&fixture.old()).unwrap(), previous);
        assert!(!fixture.target().join(&fixture.snapshot.id).exists());
    }

    #[test]
    fn unfinished_and_unknown_files_are_retained_for_recovery() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.old().join(".incomplete-test")).unwrap();
        fs::write(fixture.old().join("unknown.txt"), b"keep").unwrap();
        migrate(&fixture.legacy, &fixture.data, "game").unwrap();
        assert!(fixture.old().join(".incomplete-test").exists());
        assert_eq!(
            fs::read(fixture.old().join("unknown.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn save_roots_cannot_include_a_backup_tree_before_it_is_created() {
        let fixture = Fixture::new();
        let game = fixture.data.parent().unwrap().parent().unwrap();
        let backup_parent = fixture.data.parent().unwrap();
        fs::create_dir(backup_parent).unwrap();
        assert!(
            snapshots::validate_roots(&[display_path(backup_parent)], game, &fixture.data).is_err()
        );
        assert!(snapshots::validate_roots(&fixture.roots, game, &fixture.data).is_ok());
        fs::create_dir(&fixture.data).unwrap();
        let nested = fixture.data.join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(snapshots::validate_roots(&[display_path(&nested)], game, &fixture.data).is_err());
    }
}
