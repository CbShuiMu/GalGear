//! Version-specific CG unlock profile. No save-codec or DRM bypass is involved.
//! The game's own VM writes CG records on its next startup.
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const ORIGINAL_SHA: &str = "24b8cffa195004f0c170958936d4fbaa2ef089c3a515f1710696d7c65c0e761b";
const SOURCE_CSX_SHA: &str = "963daedef2708e89fcce4813302052f97511697774ac3a25ee9bf6f0b3ebb8e3";
const XML_SHA: &str = "7a2633961e8ef7c82baca8aca2c02718e48cb6aa74b087cabd03c04a8ab51a35";
pub const PATCHED_SHA: &str = "6138bda9885482541d239db74d13af72aba2592b490249b6112ab7b43cb6c891";
const BACKUP: &str = ".galgear-backups/angeltear-cg";
const HOOK: usize = 0x1e5f4;
const CALL_LOAD: [u8; 11] = [0x11, 1, 0, 0, 0, 0, 4, 0x3b, 2, 1, 0];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineSupport {
    pub name: String,
    pub description: Message,
    pub experimental: bool,
}

pub fn supported_engines() -> Vec<EngineSupport> {
    vec![EngineSupport {
        name: "Entis GLS / Cotopha".into(),
        description: Message::new("unlock.scope"),
        experimental: true,
    }]
}

fn invalid() -> Message {
    Message::new("unlock.invalid_archive")
}
fn io(error: impl ToString) -> Message {
    Message::detail("unlock.io", error)
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bytes(data: &[u8], offset: usize, size: usize) -> Result<&[u8], Message> {
    data.get(offset..offset.checked_add(size).ok_or_else(invalid)?)
        .ok_or_else(invalid)
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, Message> {
    Ok(u32::from_le_bytes(
        bytes(data, offset, 4)?.try_into().map_err(|_| invalid())?,
    ))
}
fn u64_at(data: &[u8], offset: usize) -> Result<u64, Message> {
    Ok(u64::from_le_bytes(
        bytes(data, offset, 8)?.try_into().map_err(|_| invalid())?,
    ))
}
fn size(value: u64) -> Result<usize, Message> {
    usize::try_from(value).map_err(|_| invalid())
}
fn put32(output: &mut Vec<u8>, value: u32) {
    output.extend(value.to_le_bytes());
}
fn patch64(data: &mut [u8], offset: usize, value: usize) -> Result<(), Message> {
    let end = offset.checked_add(8).ok_or_else(invalid)?;
    data.get_mut(offset..end)
        .ok_or_else(invalid)?
        .copy_from_slice(&(value as u64).to_le_bytes());
    Ok(())
}
fn wide(text: &str) -> Vec<u8> {
    let words: Vec<_> = text.encode_utf16().collect();
    let mut data = (words.len() as u32).to_le_bytes().to_vec();
    for word in words {
        data.extend(word.to_le_bytes());
    }
    data
}
fn read_small(path: &Path) -> Result<Vec<u8>, Message> {
    if fs::metadata(path).map_err(io)?.len() > 64 * 1024 * 1024 {
        return Err(invalid());
    }
    fs::read(path).map_err(io)
}

pub fn status(folder: &Path) -> &'static str {
    let Ok(data) = read_small(&folder.join("script.noa")) else {
        return "unsupported";
    };
    if sha(&data) != ORIGINAL_SHA || !supported_xml(folder) {
        return "unsupported";
    }
    let overlay = folder.join("script.csx");
    if overlay.exists() {
        return match read_small(&overlay) {
            Ok(data) if sha(&data) == PATCHED_SHA => "installed",
            _ => "unsupported",
        };
    }
    match source_csx(folder) {
        Ok(_) => "experimental",
        Err(_) => "unsupported",
    }
}

fn supported_xml(folder: &Path) -> bool {
    read_small(&folder.join("angeltear.xml")).is_ok_and(|data| sha(&data) == XML_SHA)
}

struct Entry {
    name: String,
    length: usize,
    encoding: u32,
    offset: usize,
}
fn directory(path: &Path) -> Result<Vec<Entry>, Message> {
    let mut file = fs::File::open(path).map_err(io)?;
    let mut header = [0u8; 80];
    file.read_exact(&mut header).map_err(io)?;
    if &header[..8] != b"Entis\x1a\0\0" || &header[64..72] != b"DirEntry" {
        return Err(invalid());
    }
    let length = size(u64_at(&header, 72)?)?;
    if !(4..=16_000_000).contains(&length) {
        return Err(invalid());
    }
    let mut data = vec![0; length];
    file.read_exact(&mut data).map_err(io)?;
    let count = u32_at(&data, 0)?;
    if count > 500_000 {
        return Err(invalid());
    }
    let mut cursor = 4;
    let mut output = Vec::new();
    for _ in 0..count {
        let length = size(u64_at(&data, cursor)?)?;
        let attr = u32_at(&data, cursor + 8)?;
        let encoding = u32_at(&data, cursor + 12)?;
        let offset = size(u64_at(&data, cursor + 16)?)?
            .checked_add(64)
            .ok_or_else(invalid)?;
        cursor += 32;
        let extra = u32_at(&data, cursor)? as usize;
        cursor += 4;
        bytes(&data, cursor, extra)?;
        cursor += extra;
        let name_size = u32_at(&data, cursor)? as usize;
        cursor += 4;
        let name = String::from_utf8_lossy(bytes(&data, cursor, name_size)?)
            .trim_end_matches('\0')
            .to_owned();
        cursor += name_size;
        if attr != 0 {
            return Err(invalid());
        } // This exact profile has flat, raw archives.
        output.push(Entry {
            name,
            length,
            encoding,
            offset,
        });
    }
    Ok(output)
}

fn source_csx(folder: &Path) -> Result<Vec<u8>, Message> {
    let archive = folder.join("ev.noa");
    let entry = directory(&archive)?
        .into_iter()
        .find(|e| e.name == "script.csx")
        .ok_or_else(invalid)?;
    if entry.encoding != 0 || entry.length > 64 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut file = fs::File::open(archive).map_err(io)?;
    file.seek(SeekFrom::Start((entry.offset + 16) as u64))
        .map_err(io)?;
    let mut data = vec![0; entry.length];
    file.read_exact(&mut data).map_err(io)?;
    if sha(&data) != SOURCE_CSX_SHA {
        return Err(Message::new("unlock.version"));
    }
    Ok(data)
}

pub fn build(folder: &Path) -> Result<Vec<u8>, Message> {
    let original = read_small(&folder.join("script.noa"))?;
    if sha(&original) != ORIGINAL_SHA || !supported_xml(folder) {
        return Err(Message::new("unlock.version"));
    }
    // ev.noa precedes script.noa in the supported XML search order. Patch the
    // effective translated script, then install a loose file at the first path.
    let source = source_csx(folder)?;
    let csx = source.as_slice();
    let mut chunks: Vec<([u8; 8], Vec<u8>)> = Vec::new();
    let mut position = 64;
    while position < csx.len() {
        let tag: [u8; 8] = bytes(csx, position, 8)?.try_into().map_err(|_| invalid())?;
        let length = size(u64_at(csx, position + 8)?)?;
        chunks.push((tag, bytes(csx, position + 16, length)?.to_vec()));
        position += 16 + length;
    }
    fn section(chunks: &[([u8; 8], Vec<u8>)], name: &[u8; 8]) -> Result<usize, Message> {
        chunks
            .iter()
            .position(|(tag, _)| tag == name)
            .ok_or_else(invalid)
    }
    let image_index = section(&chunks, b"image   ")?;
    let strings_index = section(&chunks, b"conststr")?;
    let functions_index = section(&chunks, b"funcinfo")?;
    let image = &chunks[image_index].1;
    if bytes(image, HOOK, 11)? != CALL_LOAD {
        return Err(Message::new("unlock.version"));
    }
    let base = u32::try_from(image.len()).map_err(|_| invalid())?;
    let mut tail = CALL_LOAD.to_vec();
    let table = &chunks[strings_index].1;
    let count = u32_at(table, 0)?;
    let mut cursor = 4;
    if count > 200_000 {
        return Err(invalid());
    }
    let mut strings: Vec<(Vec<u8>, Vec<u32>)> = Vec::new();
    let mut lookup = BTreeMap::new();
    for index in 0..count {
        let start = cursor;
        let length = u32_at(table, cursor)? as usize;
        cursor += 4;
        let raw = bytes(table, cursor, length.checked_mul(2).ok_or_else(invalid)?)?;
        let words: Vec<_> = raw
            .chunks_exact(2)
            .map(|word| u16::from_le_bytes([word[0], word[1]]))
            .collect();
        let text = String::from_utf16(&words).map_err(|_| invalid())?;
        cursor += raw.len();
        let wide = table[start..cursor].to_vec();
        let references = u32_at(table, cursor)?;
        cursor += 4;
        if references > 1_000_000 {
            return Err(invalid());
        }
        let mut refs = Vec::new();
        for _ in 0..references {
            refs.push(u32_at(table, cursor)?);
            cursor += 4;
        }
        strings.push((wide, refs));
        lookup.insert(text, index);
    }
    if cursor != table.len() {
        return Err(invalid());
    }
    let files: BTreeSet<_> = directory(&folder.join("ev.noa"))?
        .into_iter()
        .map(|e| e.name.to_lowercase())
        .filter(|name| name.starts_with("ev") && name.ends_with(".eri"))
        .collect();
    if files.len() != 6035 {
        return Err(Message::new("unlock.resources"));
    }
    for name in files {
        tail.extend([2, 4, 4]);
        put32(&mut tail, 4);
        tail.extend([10, 4]);
        put32(&mut tail, 53);
        let index = if let Some(index) = lookup.get(&name) {
            *index
        } else {
            let index = strings.len() as u32;
            strings.push((wide(&name), Vec::new()));
            lookup.insert(name, index);
            index
        };
        let reference = base
            .checked_add(tail.len() as u32)
            .and_then(|v| v.checked_add(7))
            .ok_or_else(invalid)?;
        strings[index as usize].1.push(reference);
        tail.extend([2, 0, 6]);
        put32(&mut tail, 0x80000000);
        put32(&mut tail, index);
        tail.extend([11, 2, 0, 10, 255, 3, 255, 1]);
    }
    tail.extend([2, 4, 4]);
    put32(&mut tail, 4);
    tail.extend([10, 4]);
    put32(&mut tail, 46);
    tail.extend([2, 0, 4]);
    put32(&mut tail, 1);
    tail.extend([3, 255, 1]);
    tail.extend([2, 4, 4]);
    put32(&mut tail, 4);
    tail.push(17);
    put32(&mut tail, 1);
    tail.extend([0, 4]);
    put32(&mut tail, 0x10446);
    let return_offset = (HOOK + 11) as i64 - (base as i64 + tail.len() as i64 + 5);
    tail.push(6);
    tail.extend(
        i32::try_from(return_offset)
            .map_err(|_| invalid())?
            .to_le_bytes(),
    );
    let mut image = image.clone();
    image[HOOK] = 6;
    let jump = i32::try_from(base as i64 - HOOK as i64 - 5).map_err(|_| invalid())?;
    image[HOOK + 1..HOOK + 5].copy_from_slice(&jump.to_le_bytes());
    image[HOOK + 5..HOOK + 11].fill(1);
    image.extend(&tail);
    chunks[image_index].1 = image;
    let mut table = Vec::new();
    put32(&mut table, strings.len() as u32);
    for (raw, refs) in strings {
        table.extend(raw);
        put32(&mut table, refs.len() as u32);
        for reference in refs {
            put32(&mut table, reference);
        }
    }
    chunks[strings_index].1 = table;
    let info = &mut chunks[functions_index].1;
    let count = u32_at(info, 0)?;
    info[..4].copy_from_slice(&(count + 1).to_le_bytes());
    for value in [0, base, tail.len() as u32, 0] {
        put32(info, value);
    }
    info.extend(wide("GalGear::AngelTearUnlockTail"));
    let mut patched_csx = bytes(csx, 0, 64)?.to_vec();
    for (tag, content) in chunks {
        patched_csx.extend(tag);
        patched_csx.extend((content.len() as u64).to_le_bytes());
        patched_csx.extend(content);
    }
    let length = patched_csx.len();
    patch64(&mut patched_csx, 56, length - 64)?;
    if sha(&patched_csx) != PATCHED_SHA {
        return Err(Message::new("unlock.resources"));
    }
    Ok(patched_csx)
}

fn guarded(root: &Path, path: &Path) -> Result<PathBuf, Message> {
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| Message::new("unlock.path"))?;
    }
    let resolved = fs::canonicalize(ancestor).map_err(io)?;
    if !resolved.starts_with(root) {
        return Err(Message::new("unlock.path"));
    }
    Ok(path.to_owned())
}
fn write_atomic(path: &Path, data: &[u8], replace: bool) -> Result<(), Message> {
    let parent = path.parent().ok_or_else(|| Message::new("unlock.path"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
    file.write_all(data).map_err(io)?;
    file.as_file().sync_all().map_err(io)?;
    if replace {
        file.persist(path).map_err(io)?;
    } else {
        file.persist_noclobber(path).map_err(io)?;
    }
    Ok(())
}
fn safe_root(folder: &Path) -> Result<PathBuf, Message> {
    let root = fs::canonicalize(folder).map_err(io)?;
    guarded(&root, &root.join("script.noa"))?;
    guarded(&root, &root.join("ev.noa"))?;
    guarded(&root, &root.join("angeltear.xml"))?;
    guarded(&root, &root.join("script.csx"))?;
    Ok(root)
}

pub fn install(folder: &Path) -> Result<(), Message> {
    ensure_game_closed()?;
    let root = safe_root(folder)?;
    if status(&root) == "installed" {
        return Ok(());
    }
    let patched = build(&root)?;
    if sha(&patched) != PATCHED_SHA {
        return Err(Message::new("unlock.resources"));
    }
    let original = source_csx(&root)?;
    if root.join("script.csx").exists() {
        return Err(Message::new("unlock.version"));
    }
    let backup = guarded(&root, &root.join(BACKUP))?;
    fs::create_dir_all(&backup).map_err(io)?;
    let script_backup = guarded(&root, &backup.join("script.csx"))?;
    if script_backup.exists() {
        if sha(&read_small(&script_backup)?) != SOURCE_CSX_SHA {
            return Err(Message::new("unlock.backup"));
        }
    } else {
        write_atomic(&script_backup, &original, false)?;
    }
    let sysenv = guarded(&root, &root.join("savedata/sysenv.dat"))?;
    let save_backup = guarded(&root, &backup.join("sysenv.dat"))?;
    if sysenv.is_file() && !save_backup.exists() {
        write_atomic(&save_backup, &read_small(&sysenv)?, false)?;
    }
    // A second check closes the gap while resources/backups were being read.
    ensure_game_closed()?;
    if status(&root) != "experimental" {
        return Err(Message::new("unlock.version"));
    }
    write_atomic(&root.join("script.csx"), &patched, false)
}

pub fn restore(folder: &Path) -> Result<(), Message> {
    ensure_game_closed()?;
    let root = safe_root(folder)?;
    if status(&root) != "installed" {
        return Err(Message::new("unlock.version"));
    }
    let backup = guarded(&root, &root.join(BACKUP).join("script.csx"))?;
    let original = read_small(&backup)?;
    if sha(&original) != SOURCE_CSX_SHA {
        return Err(Message::new("unlock.backup"));
    }
    ensure_game_closed()?;
    if status(&root) != "installed" {
        return Err(Message::new("unlock.version"));
    }
    // Removing only our hash-verified overlay restores the original archive
    // search order, without replacing any game archive or save.
    fs::remove_file(root.join("script.csx")).map_err(io)
}

#[cfg(windows)]
pub(crate) fn ensure_game_closed_names(names: &[&str]) -> Result<(), Message> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    // Read-only process enumeration; no game process is stopped or changed.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(Message::new("unlock.process_check"));
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = Process32FirstW(snapshot, &mut entry) != 0;
        if !found {
            CloseHandle(snapshot);
            return Err(Message::new("unlock.process_check"));
        }
        let mut running = false;
        while found {
            let length = entry
                .szExeFile
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(entry.szExeFile.len());
            if names.iter().any(|name| {
                String::from_utf16_lossy(&entry.szExeFile[..length]).eq_ignore_ascii_case(name)
            }) {
                running = true;
                break;
            }
            found = Process32NextW(snapshot, &mut entry) != 0;
        }
        let complete = running || GetLastError() == ERROR_NO_MORE_FILES;
        CloseHandle(snapshot);
        if !complete {
            return Err(Message::new("unlock.process_check"));
        }
        if running {
            Err(Message::new("auto.close_game"))
        } else {
            Ok(())
        }
    }
}
#[cfg(not(windows))]
pub(crate) fn ensure_game_closed_names(_names: &[&str]) -> Result<(), Message> {
    Err(Message::new("unlock.platform"))
}

fn ensure_game_closed() -> Result<(), Message> {
    ensure_game_closed_names(&["angeltear.exe"])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_and_truncated_archives_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("script.noa");
        fs::write(&file, b"broken").unwrap();
        assert!(super::directory(&file).is_err());
        assert!(build(directory.path()).is_err());
        assert_eq!(status(directory.path()), "unsupported");
    }
    #[test]
    fn backup_and_atomic_replacement_preserve_original_data() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("original");
        write_atomic(&file, b"original", false).unwrap();
        assert!(write_atomic(&file, b"other", false).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"original");
        write_atomic(&file, b"updated", true).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"updated");
    }
    #[test]
    fn paths_outside_the_game_folder_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        assert!(guarded(&root, &root.join("inside/file")).is_ok());
        assert!(guarded(&root, &root.join("../outside")).is_err());
    }
}
