use super::{AutoResult, failure};
#[cfg(test)]
#[path = "arc_tests.rs"]
mod tests;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u32,
}

pub fn index(path: &Path) -> AutoResult<Vec<Entry>> {
    let mut file = fs::File::open(path).map_err(super::io)?;
    let length = file.metadata().map_err(super::io)?.len();
    let mut header = [0; 8];
    file.read_exact(&mut header).map_err(super::io)?;
    let count = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    if count == 0 || count > 100_000 || size > 16_000_000 || size as u64 + 8 > length {
        return Err(failure("ARC index bounds"));
    }
    let mut data = vec![0; size];
    file.read_exact(&mut data).map_err(super::io)?;
    let mut cursor = super::Cursor::new(&data);
    let mut entries = Vec::with_capacity(count);
    let mut names = std::collections::BTreeSet::new();
    for _ in 0..count {
        let size = cursor.u32()?;
        let offset = cursor.u32()? as u64 + header.len() as u64 + data.len() as u64;
        let mut words = Vec::new();
        loop {
            let v = cursor.u16()?;
            if v == 0 {
                break;
            }
            words.push(v);
            if words.len() > 1024 {
                return Err(failure("ARC filename length"));
            }
        }
        let name = String::from_utf16(&words).map_err(|_| failure("ARC filename encoding"))?;
        if name.is_empty()
            || name.contains(['/', '\\', ':'])
            || !names.insert(name.to_lowercase())
            || offset.checked_add(size as u64).is_none_or(|v| v > length)
        {
            return Err(failure("ARC entry bounds or duplicate name"));
        }
        entries.push(Entry { name, offset, size });
    }
    if cursor.position != data.len() {
        return Err(failure("ARC index trailing data"));
    }
    Ok(entries)
}

pub fn read(path: &Path, entry: &Entry) -> AutoResult<Vec<u8>> {
    if entry.size > 64 * 1024 * 1024 {
        return Err(failure("ARC member exceeds limit"));
    }
    let mut file = fs::File::open(path).map_err(super::io)?;
    file.seek(SeekFrom::Start(entry.offset))
        .map_err(super::io)?;
    let mut data = vec![0; entry.size as usize];
    file.read_exact(&mut data).map_err(super::io)?;
    if entry.name.to_lowercase().ends_with(".ws2") {
        for byte in &mut data {
            *byte = byte.rotate_right(2);
        }
    }
    Ok(data)
}

pub fn replace(
    data: &[u8],
    entries: &[Entry],
    target: &str,
    replacement: &[u8],
) -> AutoResult<Vec<u8>> {
    let mut index: Vec<u8> = Vec::new();
    let mut payload: Vec<u8> = Vec::new();
    let mut found = false;
    for entry in entries {
        let content = if entry.name.eq_ignore_ascii_case(target) {
            found = true;
            replacement
        } else {
            let start = usize::try_from(entry.offset).map_err(|_| failure("ARC offset"))?;
            data.get(
                start
                    ..start
                        .checked_add(entry.size as usize)
                        .ok_or_else(|| failure("ARC bounds"))?,
            )
            .ok_or_else(|| failure("ARC bounds"))?
        };
        let size = u32::try_from(content.len()).map_err(|_| failure("ARC size"))?;
        let offset = u32::try_from(payload.len()).map_err(|_| failure("ARC size"))?;
        index.extend(size.to_le_bytes());
        index.extend(offset.to_le_bytes());
        for word in entry.name.encode_utf16().chain(std::iter::once(0)) {
            index.extend(word.to_le_bytes());
        }
        payload.extend(content);
    }
    if !found {
        return Err(failure("ARC target missing"));
    }
    let mut output = (entries.len() as u32).to_le_bytes().to_vec();
    output.extend((index.len() as u32).to_le_bytes());
    output.extend(index);
    output.extend(payload);
    Ok(output)
}
