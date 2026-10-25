//! Previewed patch writes with durable backups and hash-checked reversal.
pub mod folders;

use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

fn error(e: impl ToString) -> Message {
    Message::detail("error.patch", e)
}
fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn file_hash(path: &Path) -> Result<String, Message> {
    let mut file = fs::File::open(path).map_err(error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn atomic_copy(path: &Path, source: &Path) -> Result<(), Message> {
    let parent = path.parent().ok_or_else(|| error("Missing parent"))?;
    fs::create_dir_all(parent).map_err(error)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    std::io::copy(&mut fs::File::open(source).map_err(error)?, &mut file).map_err(error)?;
    file.as_file().sync_all().map_err(error)?;
    file.persist(path).map_err(error)?;
    Ok(())
}
fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn read(path: &Path) -> Result<Vec<u8>, Message> {
    if fs::metadata(path).map_err(error)?.len() > 128 * 1024 * 1024 {
        return Err(error("File exceeds 128 MiB"));
    }
    fs::read(path).map_err(error)
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<(), Message> {
    let parent = path.parent().ok_or_else(|| error("Missing parent"))?;
    fs::create_dir_all(parent).map_err(error)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    file.write_all(bytes).map_err(error)?;
    file.as_file().sync_all().map_err(error)?;
    file.persist(path).map_err(error)?;
    Ok(())
}
fn relative(name: &str) -> Result<PathBuf, Message> {
    let normalized = name.replace('\\', "/");
    let path = PathBuf::from(normalized);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || name.contains(':')
        || name.contains('\0')
        || name
            .split(['/', '\\'])
            .any(|s| s.to_lowercase().starts_with(".galgear") || s.ends_with([' ', '.']))
    {
        return Err(error(format!("Unsafe archive path: {name}")));
    }
    for part in name.split(['/', '\\']) {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ["con", "prn", "aux", "nul"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(error("Reserved Windows file name"));
        }
    }
    Ok(path)
}
fn target(root: &Path, name: &str) -> Result<PathBuf, Message> {
    let path = root.join(relative(name)?);
    for ancestor in path.ancestors().take_while(|ancestor| *ancestor != root) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(error("Patch targets cannot contain links"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(error(e)),
        }
    }
    let mut ancestor = path.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| error("Missing ancestor"))?;
    }
    if !fs::canonicalize(ancestor).map_err(error)?.starts_with(root) {
        return Err(error("Target escapes game folder"));
    }
    if path.is_dir() {
        return Err(error("Target is a directory"));
    }
    Ok(path)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub before: Option<String>,
    /// The installed hash, or `None` when the old resource is removed by a switch.
    pub after: Option<String>,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub title: String,
    pub game_id: String,
    pub target: String,
    pub files: Vec<Change>,
    pub executable: Option<String>,
}
pub struct Plan {
    pub preview: Preview,
    staging: tempfile::TempDir,
    root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installed {
    pub id: String,
    pub preview: Preview,
    pub status: String,
}

pub fn preview(archive: &Path, folder: &Path, game_id: String) -> Result<Plan, Message> {
    let root = fs::canonicalize(folder).map_err(error)?;
    let staging = tempfile::tempdir().map_err(error)?;
    let mut info = Preview {
        title: archive
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        game_id,
        target: crate::config::display_path(&root),
        files: Vec::new(),
        executable: None,
    };
    if archive
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
    {
        info.executable = Some(crate::config::display_path(
            &fs::canonicalize(archive).map_err(error)?,
        ));
        return Ok(Plan {
            preview: info,
            staging,
            root,
        });
    }
    let mut total = 0u64;
    let mut seen = BTreeSet::new();
    let mut append = |name: &str, data: Vec<u8>| -> Result<(), Message> {
        let relative = relative(name)?;
        if !seen.insert(relative.to_string_lossy().to_lowercase()) {
            return Err(error("Archive contains duplicate paths"));
        }
        total += data.len() as u64;
        if total > 1024 * 1024 * 1024 || info.files.len() >= 10000 {
            return Err(error("Patch exceeds size or file limit"));
        }
        let dest = target(&root, name)?;
        let before = if dest.is_file() {
            Some(hash(&read(&dest)?))
        } else {
            None
        };
        fs::write(staging.path().join(info.files.len().to_string()), &data).map_err(error)?;
        info.files.push(Change {
            path: relative.to_string_lossy().into_owned(),
            before,
            after: Some(hash(&data)),
            size: data.len() as u64,
        });
        Ok(())
    };
    if archive
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
    {
        let mut zip =
            zip::ZipArchive::new(fs::File::open(archive).map_err(error)?).map_err(error)?;
        if zip.len() > 10000 {
            return Err(error("Archive contains too many files"));
        }
        for i in 0..zip.len() {
            let file = zip.by_index(i).map_err(error)?;
            if file.is_dir() {
                continue;
            }
            if file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err(error("Archive links are unsupported"));
            }
            if file.size() > 128 * 1024 * 1024 {
                return Err(error("Patch file exceeds 128 MiB"));
            }
            let name = file.name().to_owned();
            let mut data = Vec::new();
            file.take(128 * 1024 * 1024 + 1)
                .read_to_end(&mut data)
                .map_err(error)?;
            if data.len() > 128 * 1024 * 1024 {
                return Err(error("Patch file exceeds size limit"));
            }
            append(&name, data)?;
        }
    } else if archive
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("7z") || s.eq_ignore_ascii_case("rar"))
    {
        extract_7z(archive, &mut append)?;
    } else {
        return Err(error("Select ZIP, RAR, 7z or EXE"));
    }
    if info.files.is_empty() {
        return Err(error("Empty patch"));
    }
    Ok(Plan {
        preview: info,
        staging,
        root,
    })
}

fn seven_zip() -> Result<PathBuf, Message> {
    for name in ["7z.exe", "7za.exe"] {
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let path = dir.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    if let Some(dir) = std::env::var_os("ProgramFiles") {
        let path = PathBuf::from(dir).join("7-Zip/7z.exe");
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(error("RAR / 7z requires 7-Zip installed locally"))
}
fn hidden_command(exe: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
fn output_bounded(command: &mut std::process::Command, max: u64) -> Result<Vec<u8>, Message> {
    use std::process::Stdio;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .map_err(error)?;
    let mut bytes = Vec::new();
    let read_result = child
        .stdout
        .take()
        .ok_or_else(|| error("Missing pipe"))?
        .take(max + 1)
        .read_to_end(&mut bytes);
    if bytes.len() as u64 > max || read_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error("7-Zip response exceeds limit"));
    }
    if !child.wait().map_err(error)?.success() {
        return Err(error(
            "7-Zip could not read the archive. Encrypted archives and unsupported formats require manual extraction.",
        ));
    }
    Ok(bytes)
}
fn extract_7z(
    archive: &Path,
    append: &mut impl FnMut(&str, Vec<u8>) -> Result<(), Message>,
) -> Result<(), Message> {
    let exe = seven_zip()?;
    let archive = fs::canonicalize(archive).map_err(error)?;
    let listing = output_bounded(
        hidden_command(&exe)
            .args(["l", "-slt", "-ba", "-sccUTF-8"])
            .arg(&archive),
        4 * 1024 * 1024,
    )?;
    let listing = String::from_utf8(listing)
        .map_err(error)?
        .replace("\r\n", "\n");
    for record in listing.split("\n\n") {
        let props: std::collections::BTreeMap<_, _> =
            record.lines().filter_map(|l| l.split_once(" = ")).collect();
        let Some(name) = props.get("Path") else {
            continue;
        };
        relative(name)?;
        if props.contains_key("Symbolic Link") || props.contains_key("Hard Link") {
            return Err(error("Archive links are unsupported"));
        }
        if props.get("Attributes").is_some_and(|s| s.contains('D'))
            || props.get("Folder") == Some(&"+")
        {
            continue;
        }
        let size: u64 = props
            .get("Size")
            .ok_or_else(|| error("Missing archive file size"))?
            .parse()
            .map_err(error)?;
        if size > 128 * 1024 * 1024 {
            return Err(error("Patch file exceeds 128 MiB"));
        }
        // Stream one selected entry to stdout; archive-controlled paths never become extraction paths.
        let bytes = output_bounded(
            hidden_command(&exe)
                .args(["x", "-so", "-y", "-spd"])
                .arg(&archive)
                .arg("--")
                .arg(name),
            size,
        )?;
        if bytes.len() as u64 != size {
            return Err(error("Archive file size changed"));
        }
        append(name, bytes)?;
    }
    Ok(())
}

fn write_record(dir: &Path, record: &Installed) -> Result<(), Message> {
    atomic(
        &dir.join("manifest.json"),
        &serde_json::to_vec_pretty(record).map_err(error)?,
    )
}
pub fn list(data: &Path) -> Result<Vec<Installed>, Message> {
    let mut items = Vec::new();
    if !data.exists() {
        return Ok(items);
    }
    for entry in fs::read_dir(data).map_err(error)? {
        let entry = entry.map_err(error)?;
        if !entry.file_type().map_err(error)?.is_dir() {
            continue;
        }
        let path = entry.path().join("manifest.json");
        if path.exists() {
            items.push(serde_json::from_slice(&read(&path)?).map_err(error)?);
        }
    }
    items.sort_by(|a: &Installed, b| b.id.cmp(&a.id));
    Ok(items)
}

pub fn install(plan: Plan, data: &Path) -> Result<Installed, Message> {
    if plan.preview.executable.is_some() {
        return Err(error("EXE must be run by the user"));
    }
    // Revalidate every target and staged file before touching the game.
    for (i, file) in plan.preview.files.iter().enumerate() {
        let path = target(&plan.root, &file.path)?;
        let current = if path.exists() {
            Some(file_hash(&path)?)
        } else {
            None
        };
        let staged_changed = match &file.after {
            Some(after) => file_hash(&plan.staging.path().join(i.to_string()))? != *after,
            None => false,
        };
        if current != file.before || staged_changed {
            return Err(error("Files changed since preview; select the patch again"));
        }
    }
    fs::create_dir_all(data).map_err(error)?;
    let id = format!(
        "{:020}-{:08x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(error)?
            .as_nanos(),
        std::process::id()
    );
    let dir = data.join(&id);
    fs::create_dir(&dir).map_err(error)?;
    let mut record = Installed {
        id,
        preview: plan.preview.clone(),
        status: "installing".into(),
    };
    for (i, file) in record.preview.files.iter().enumerate() {
        if file.before.is_some() {
            atomic_copy(
                &dir.join(format!("{i}.bak")),
                &target(&plan.root, &file.path)?,
            )?;
        }
    }
    write_record(&dir, &record)?;
    let result = (|| {
        for (i, file) in record.preview.files.iter().enumerate() {
            let path = target(&plan.root, &file.path)?;
            let current = if path.exists() {
                Some(file_hash(&path)?)
            } else {
                None
            };
            if current != file.before {
                return Err(error("Game files changed during installation"));
            }
            if file.after.is_some() {
                atomic_copy(&path, &plan.staging.path().join(i.to_string()))?;
            } else if path.exists() {
                fs::remove_file(path).map_err(error)?;
            }
        }
        record.status = "installed".into();
        write_record(&dir, &record)
    })();
    if let Err(e) = result {
        record.status = "installing".into();
        rollback(&dir, &mut record)?;
        return Err(e);
    }
    Ok(record)
}

fn rollback(dir: &Path, record: &mut Installed) -> Result<(), Message> {
    let root = fs::canonicalize(&record.preview.target).map_err(error)?;
    // Validate all backups and current files before any restore write.
    for (i, file) in record.preview.files.iter().enumerate() {
        let path = target(&root, &file.path)?;
        let current = if path.exists() {
            Some(file_hash(&path)?)
        } else {
            None
        };
        if current != file.after
            && !(["installing", "restoring"].contains(&record.status.as_str())
                && current == file.before)
        {
            return Err(error(format!(
                "File changed; restoration stopped: {}",
                file.path
            )));
        }
        if let Some(before) = &file.before
            && file_hash(&dir.join(format!("{i}.bak")))? != *before
        {
            return Err(error("Backup checksum mismatch"));
        }
    }
    record.status = "restoring".into();
    write_record(dir, record)?;
    for (i, file) in record.preview.files.iter().enumerate().rev() {
        let path = target(&root, &file.path)?;
        if file.before.is_some() {
            atomic_copy(&path, &dir.join(format!("{i}.bak")))?;
        } else if path.exists() {
            fs::remove_file(path).map_err(error)?;
        }
    }
    record.status = "restored".into();
    write_record(dir, record)
}
pub fn restore(data: &Path, id: &str) -> Result<Installed, Message> {
    if id.contains(['/', '\\', ':']) {
        return Err(error("Invalid patch id"));
    }
    let items = list(data)?;
    let mut record = items
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .ok_or_else(|| error("Patch not found"))?;
    if !["installed", "installing", "restoring"].contains(&record.status.as_str()) {
        return Err(error("Patch is not installed"));
    }
    if items.iter().any(|r| {
        r.id > record.id
            && r.preview.target == record.preview.target
            && r.status == "installed"
            && r.preview.files.iter().any(|f| {
                record
                    .preview
                    .files
                    .iter()
                    .any(|p| p.path.eq_ignore_ascii_case(&f.path))
            })
    }) {
        return Err(error(
            "Restore overlapping patches in reverse installation order",
        ));
    }
    rollback(&data.join(id), &mut record)?;
    Ok(record)
}
pub fn recover(data: &Path) -> Vec<Message> {
    match list(data) {
        Ok(items) => items
            .iter()
            .filter(|r| ["installing", "restoring"].contains(&r.status.as_str()))
            .filter_map(|r| restore(data, &r.id).err())
            .collect(),
        Err(e) => vec![e],
    }
}

pub fn record_exe(preview: Preview, data: &Path) -> Result<Installed, Message> {
    if preview.executable.is_none() {
        return Err(error("Not an EXE installer"));
    }
    if let Some(existing) = list(data)?.into_iter().find(|p| {
        p.preview.executable == preview.executable && p.preview.game_id == preview.game_id
    }) {
        return Ok(existing);
    }
    fs::create_dir_all(data).map_err(error)?;
    let id = format!(
        "{:020}-exe",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(error)?
            .as_nanos()
    );
    let record = Installed {
        id: id.clone(),
        preview,
        status: "manual_pending".into(),
    };
    let dir = data.join(id);
    fs::create_dir(&dir).map_err(error)?;
    write_record(&dir, &record)?;
    Ok(record)
}
pub fn set_exe_status(data: &Path, id: &str, installed: bool) -> Result<Vec<Installed>, Message> {
    if id.contains(['/', '\\', ':']) {
        return Err(error("Invalid patch id"));
    }
    let mut record = list(data)?
        .into_iter()
        .find(|p| p.id == id && p.preview.executable.is_some())
        .ok_or_else(|| error("EXE installer record not found"))?;
    record.status = if installed {
        "manual_installed"
    } else {
        "manual_pending"
    }
    .into();
    write_record(&data.join(id), &record)?;
    list(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn archive(path: &Path, name: &str, bytes: &[u8]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
        zip.finish().unwrap();
    }
    #[test]
    fn reads_existing_patch_records_with_string_hashes() {
        let change: Change =
            serde_json::from_str(r#"{"path":"script.dat","before":"old","after":"new","size":3}"#)
                .unwrap();
        assert_eq!(change.after.as_deref(), Some("new"));
    }

    #[test]
    fn recovers_interrupted_extension_switches_and_keeps_original_backups() {
        for status in ["installing", "restoring"] {
            for original_present in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("game");
                fs::create_dir(&root).unwrap();
                let patch = folders::directory(&root).unwrap().join("Patch");
                fs::create_dir(&patch).unwrap();
                fs::write(root.join("script.noa"), b"original").unwrap();
                fs::write(patch.join("script.csx"), b"patch").unwrap();
                let data = temp.path().join("backups");
                let mut record = install(
                    folders::preview(&root, "Patch", "game".into()).unwrap(),
                    &data,
                )
                .unwrap();
                record.status = status.into();
                write_record(&data.join(&record.id), &record).unwrap();
                if original_present {
                    fs::write(root.join("script.noa"), b"original").unwrap();
                }
                assert!(recover(&data).is_empty());
                assert!(!root.join("script.csx").exists());
                assert_eq!(fs::read(root.join("script.noa")).unwrap(), b"original");
                assert_eq!(
                    fs::read(data.join(&record.id).join("1.bak")).unwrap(),
                    b"original"
                );
            }
        }
    }

    #[test]
    fn preview_install_restore_and_external_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir(&root).unwrap();
        let data = temp.path().join("backups");
        let zip = temp.path().join("patch.zip");
        fs::write(root.join("script.dat"), b"original").unwrap();
        archive(&zip, "script.dat", b"patched");
        let plan = preview(&zip, &root, "game".into()).unwrap();
        assert!(plan.preview.files[0].before.is_some());
        let record = install(plan, &data).unwrap();
        assert_eq!(fs::read(root.join("script.dat")).unwrap(), b"patched");
        fs::write(root.join("script.dat"), b"user changed").unwrap();
        assert!(restore(&data, &record.id).is_err());
        assert_eq!(fs::read(root.join("script.dat")).unwrap(), b"user changed");
        fs::write(root.join("script.dat"), b"patched").unwrap();
        restore(&data, &record.id).unwrap();
        assert_eq!(fs::read(root.join("script.dat")).unwrap(), b"original");
    }
    #[test]
    fn traversal_and_stale_previews_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir(&root).unwrap();
        let zip = temp.path().join("patch.zip");
        archive(&zip, "../outside.txt", b"bad");
        assert!(preview(&zip, &root, "id".into()).is_err());
        archive(&zip, "new.txt", b"patch");
        let plan = preview(&zip, &root, "id".into()).unwrap();
        fs::write(root.join("new.txt"), b"changed").unwrap();
        assert!(install(plan, &temp.path().join("backups")).is_err());
        assert_eq!(fs::read(root.join("new.txt")).unwrap(), b"changed");
    }
    #[test]
    fn overlapping_patches_require_reverse_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir(&root).unwrap();
        let data = temp.path().join("data");
        let zip = temp.path().join("patch.zip");
        fs::write(root.join("script.dat"), b"original").unwrap();
        archive(&zip, "script.dat", b"one");
        let one = install(preview(&zip, &root, "id".into()).unwrap(), &data).unwrap();
        archive(&zip, "script.dat", b"two");
        let two = install(preview(&zip, &root, "id".into()).unwrap(), &data).unwrap();
        assert!(restore(&data, &one.id).is_err());
        restore(&data, &two.id).unwrap();
        restore(&data, &one.id).unwrap();
        assert_eq!(fs::read(root.join("script.dat")).unwrap(), b"original");
    }

    #[test]
    fn startup_finishes_an_interrupted_restore_and_keeps_backups() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("game");
        fs::create_dir(&root).unwrap();
        let data = temp.path().join("data");
        let archive = temp.path().join("patch.zip");
        fs::write(root.join("a.dat"), b"old-a").unwrap();
        fs::write(root.join("b.dat"), b"old-b").unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        for (name, bytes) in [("a.dat", b"new-a"), ("b.dat", b"new-b")] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut record = install(preview(&archive, &root, "id".into()).unwrap(), &data).unwrap();
        record.status = "restoring".into();
        write_record(&data.join(&record.id), &record).unwrap();
        fs::write(root.join("b.dat"), b"old-b").unwrap();
        assert!(recover(&data).is_empty());
        assert_eq!(fs::read(root.join("a.dat")).unwrap(), b"old-a");
        assert_eq!(fs::read(root.join("b.dat")).unwrap(), b"old-b");
        assert!(data.join(&record.id).join("0.bak").exists());
    }
}
