//! Publish downloaded patches without touching installed game files.
use super::{error, folders, relative, target};
use galgear_i18n::Message;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn archive_kind(path: &Path) -> Result<Option<&'static str>, Message> {
    let mut magic = [0; 8];
    let count = fs::File::open(path)
        .map_err(error)?
        .read(&mut magic)
        .map_err(error)?;
    let magic = &magic[..count];
    if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
        return Ok(Some("zip"));
    }
    if magic.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Ok(Some("7z"));
    }
    if magic.starts_with(b"Rar!\x1a\x07") {
        return Ok(Some("rar"));
    }
    Ok(
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "zip" => Some("zip"),
            "7z" => Some("7z"),
            "rar" => Some("rar"),
            _ => None,
        },
    )
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), Message> {
    if cancel.load(Ordering::Relaxed) {
        Err(Message::new("download.cancelled"))
    } else {
        Ok(())
    }
}

/// Each download becomes one child of `patch`. Delete the input only after publication.
pub fn prepare(folder: &Path, downloaded: &Path, cancel: &AtomicBool) -> Result<PathBuf, Message> {
    let storage = folders::directory(folder)?;
    finish(&storage, downloaded, true, cancel)
}

/// Archives are unpacked at either destination; loose game patches also get their own directory.
pub fn finish(
    directory: &Path,
    downloaded: &Path,
    game_patch: bool,
    cancel: &AtomicBool,
) -> Result<PathBuf, Message> {
    finish_resource(directory, downloaded, game_patch, false, cancel)
}

/// Publishes the resource type together with its contents, before announcing completion.
pub fn finish_resource(directory: &Path, downloaded: &Path, game_patch: bool, save_data: bool, cancel: &AtomicBool) -> Result<PathBuf, Message> {
    let storage = directory.canonicalize().map_err(error)?;
    let downloaded = downloaded.canonicalize().map_err(error)?;
    if downloaded.parent() != Some(storage.as_path()) || !downloaded.is_file() {
        return Err(Message::new("local_patches.unsafe"));
    }
    check_cancel(cancel)?;
    let save_data = save_data || folders::is_save_name(&downloaded.file_name().unwrap_or_default().to_string_lossy());
    let kind = archive_kind(&downloaded)?;
    if kind.is_none() && !game_patch {
        return Ok(downloaded);
    }
    let name = downloaded
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Patch");
    let name = if relative(name).is_ok() {
        name
    } else {
        "Patch"
    };
    let staging = tempfile::Builder::new()
        .prefix(".galgear-unpack-")
        .tempdir_in(&storage)
        .map_err(error)?;
    let contents = staging.path().join("contents");
    fs::create_dir(&contents).map_err(error)?;
    let contents = contents.canonicalize().map_err(error)?;
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut append = |name: &str, input: &mut dyn Read, size: u64| -> Result<(), Message> {
        check_cancel(cancel)?;
        let path = relative(name)?;
        if !seen.insert(path.to_string_lossy().to_lowercase()) || seen.len() > 10000 {
            return Err(error("Archive contains duplicate paths or too many files"));
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| error("Archive exceeds size limit"))?;
        if total > 16 * 1024 * 1024 * 1024 {
            return Err(error("Archive exceeds size limit"));
        }
        let destination = target(&contents, name)?;
        fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| error("Missing parent"))?,
        )
        .map_err(error)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(error)?;
        let mut input = input.take(size + 1);
        let mut buffer = [0; 64 * 1024];
        let mut written = 0;
        loop {
            check_cancel(cancel)?;
            let count = input.read(&mut buffer).map_err(error)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count]).map_err(error)?;
            written += count as u64;
        }
        if written != size {
            return Err(error("Archive file size mismatch"));
        }
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(error)
    };
    match kind {
        Some("zip") => {
            let mut archive =
                zip::ZipArchive::new(fs::File::open(&downloaded).map_err(error)?).map_err(error)?;
            if archive.len() > 10000 {
                return Err(error("Archive contains too many entries"));
            }
            for index in 0..archive.len() {
                let mut file = archive.by_index(index).map_err(error)?;
                relative(file.name().trim_end_matches(['/', '\\']))?;
                if file
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(error("Archive links are unsupported"));
                }
                if file.is_dir() {
                    continue;
                }
                let (name, size) = (file.name().to_owned(), file.size());
                append(&name, &mut file, size)?;
            }
        }
        Some(_) => super::extract_7z_stream(&downloaded, &mut append)?,
        None => {
            let name = downloaded
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| Message::new("local_patches.unsafe"))?;
            let mut input = fs::File::open(&downloaded).map_err(error)?;
            let size = input.metadata().map_err(error)?.len();
            append(name, &mut input, size)?;
        }
    }
    if seen.is_empty() {
        return Err(Message::new("local_patches.empty_patch"));
    }
    check_cancel(cancel)?;
    // Strip only a matching archive wrapper; meaningful directories such as `data` stay intact.
    let entries = fs::read_dir(&contents)
        .map_err(error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    let publish = if entries.len() == 1
        && entries[0].file_type().map_err(error)?.is_dir()
        && entries[0]
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
    {
        entries[0].path()
    } else {
        contents
    };
    if game_patch || save_data {
        let mut marker = fs::OpenOptions::new().write(true).create_new(true)
            .open(publish.join(folders::RESOURCE_MARKER)).map_err(error)?;
        marker.write_all(if save_data { br#"{"save_data":true}"# } else { br#"{"save_data":false}"# }).map_err(error)?;
        marker.sync_all().map_err(error)?;
    }
    for suffix in 0..1000 {
        let destination = storage.join(if suffix == 0 {
            name.to_owned()
        } else {
            format!("{name} ({suffix})")
        });
        if destination.exists() {
            continue;
        }
        fs::rename(&publish, &destination).map_err(error)?;
        fs::remove_file(&downloaded).map_err(error)?;
        return Ok(destination);
    }
    Err(Message::new("download.write"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn archive(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    #[test]
    fn save_classification_is_published_and_cannot_be_spoofed_by_archive() {
        let temp=tempfile::tempdir().unwrap(); let storage=folders::directory(temp.path()).unwrap();
        let download=storage.join("resource.zip");
        archive(&download,&[("slot.dat",b"save")]);
        finish_resource(&storage,&download,true,true,&AtomicBool::new(false)).unwrap();
        let report=folders::list(temp.path()).unwrap();
        assert!(report.patches[0].save_data); assert_eq!(report.patches[0].files,1);
        let bad=storage.join("bad.zip");
        archive(&bad,&[(folders::RESOURCE_MARKER,br#"{"save_data":false}"#),("slot.dat",b"data")]);
        assert!(finish_resource(&storage,&bad,true,true,&AtomicBool::new(false)).is_err());
        assert!(bad.exists()); assert!(!storage.join("bad").exists());
    }
    #[test]
    fn extracts_signature_detected_archives_and_deletes_only_after_success() {
        let temp = tempfile::tempdir().unwrap();
        let storage = folders::directory(temp.path()).unwrap();
        let download = storage.join("Translation.bin");
        archive(&download, &[("data/script.xp3", b"patch")]);
        let result = prepare(temp.path(), &download, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(result.join("data/script.xp3")).unwrap(), b"patch");
        assert!(!download.exists());
        assert!(!temp.path().join("data").exists());
        assert_eq!(folders::list(temp.path()).unwrap().patches[0].files, 1);
    }
    #[test]
    fn failed_or_cancelled_extraction_keeps_archive_and_never_publishes_partial_files() {
        let temp = tempfile::tempdir().unwrap();
        let storage = folders::directory(temp.path()).unwrap();
        let download = storage.join("Bad.zip");
        for names in [
            vec![
                ("valid.txt", b"data".as_slice()),
                ("../outside.txt", b"bad".as_slice()),
            ],
            vec![("DATA.txt", b"1".as_slice()), ("data.txt", b"2".as_slice())],
        ] {
            archive(&download, &names);
            assert!(prepare(temp.path(), &download, &AtomicBool::new(false)).is_err());
            assert!(download.is_file());
            assert_eq!(fs::read_dir(&storage).unwrap().count(), 1);
        }
        archive(&download, &[("file.txt", b"patch")]);
        assert!(prepare(temp.path(), &download, &AtomicBool::new(true)).is_err());
        assert!(download.is_file());
    }
    #[test]
    fn rar5_empty_link_metadata_does_not_reject_regular_files() {
        if super::super::seven_zip().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let storage = folders::directory(temp.path()).unwrap();
        let download = storage.join("Patch.rar");
        // Fixture from https://github.com/markokr/rarfile/blob/master/test/files/rar5-times.rar
        fs::write(&download, include_bytes!("fixtures/rar5-times.rar")).unwrap();
        let result = prepare(temp.path(), &download, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::metadata(result.join("stest1.txt")).unwrap().len(), 2048);
        assert!(!download.exists());
    }

    #[test]
    fn seven_zip_downloads_are_extracted_through_the_local_tool() {
        let Ok(exe) = super::super::seven_zip() else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("script.xp3"), b"seven zip patch").unwrap();
        let storage = folders::directory(temp.path()).unwrap();
        let download = storage.join("Patch.7z");
        let output = super::super::hidden_command(&exe)
            .current_dir(&input)
            .args(["a", "-t7z"])
            .arg(&download)
            .arg("script.xp3")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = prepare(temp.path(), &download, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            fs::read(result.join("script.xp3")).unwrap(),
            b"seven zip patch"
        );
        assert!(!download.exists());
    }

    #[test]
    fn ordinary_downloads_unpack_archives_in_place_and_leave_loose_files_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let download = temp.path().join("Patch.zip");
        archive(&download, &[("Patch/script.xp3", b"patch")]);
        let result = finish(temp.path(), &download, false, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(result.join("script.xp3")).unwrap(), b"patch");
        assert!(!download.exists());
        let loose = temp.path().join("readme.txt");
        fs::write(&loose, b"instructions").unwrap();
        assert_eq!(
            finish(temp.path(), &loose, false, &AtomicBool::new(false)).unwrap(),
            loose.canonicalize().unwrap()
        );
    }

    #[test]
    fn standalone_files_and_repeated_names_keep_existing_patches() {
        let temp = tempfile::tempdir().unwrap();
        let storage = folders::directory(temp.path()).unwrap();
        let download = storage.join("patch.xp3");
        fs::write(&download, b"first").unwrap();
        let first = prepare(temp.path(), &download, &AtomicBool::new(false)).unwrap();
        fs::write(&download, b"second").unwrap();
        let second = prepare(temp.path(), &download, &AtomicBool::new(false)).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first.join("patch.xp3")).unwrap(), b"first");
        assert_eq!(fs::read(second.join("patch.xp3")).unwrap(), b"second");
    }
}
