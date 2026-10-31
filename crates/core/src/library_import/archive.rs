//! 按需读取归档成员，不向磁盘解压，也不执行归档中的 SQL。
use super::{MAX_DATA, error};
use base64::Engine;
use galgear_i18n::Message;
use serde_json::Value;
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

pub(super) struct Archive {
    zip: zip::ZipArchive<File>,
    names: BTreeMap<String, usize>,
    read: u64,
}
fn name(value: &str) -> Option<String> {
    let value = value.replace('\\', "/");
    let value = value.trim_start_matches("./");
    if value.starts_with('/') || value.contains(':') || value.split('/').any(|s| s == "..") {
        return None;
    }
    Some(value.to_lowercase())
}
impl Archive {
    pub fn open(path: &Path) -> Result<Self, Message> {
        let mut zip = zip::ZipArchive::new(File::open(path).map_err(error)?).map_err(error)?;
        if zip.len() > 100_000 {
            return Err(Message::new("import.limit"));
        }
        let mut names = BTreeMap::new();
        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(error)?;
            if entry.is_dir() {
                continue;
            }
            let key = name(entry.name()).ok_or_else(|| Message::new("import.unsafe_path"))?;
            if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
                || names.insert(key, i).is_some()
            {
                return Err(Message::new("import.unsafe_path"));
            }
        }
        Ok(Self {
            zip,
            names,
            read: 0,
        })
    }
    pub fn contains(&self, key: &str) -> bool {
        name(key).is_some_and(|n| self.names.contains_key(&n))
    }
    pub fn bytes(&mut self, key: &str, limit: u64) -> Result<Option<Vec<u8>>, Message> {
        let Some(index) = name(key).and_then(|n| self.names.get(&n).copied()) else {
            return Ok(None);
        };
        let mut entry = self.zip.by_index(index).map_err(error)?;
        if entry.size() > limit || self.read.saturating_add(entry.size()) > 512 * 1024 * 1024 {
            return Err(Message::new("import.limit"));
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() as u64 > limit {
            return Err(Message::new("import.limit"));
        }
        self.read += bytes.len() as u64;
        Ok(Some(bytes))
    }
    pub fn json(&mut self, key: &str) -> Result<Value, Message> {
        self.bytes(key, MAX_DATA)?
            .map(|b| {
                serde_json::from_slice(b.strip_prefix(&[239, 187, 191]).unwrap_or(&b))
                    .map_err(error)
            })
            .unwrap_or(Ok(Value::Null))
    }
    pub fn cover(&mut self, key: &str) -> Result<Option<String>, Message> {
        self.bytes(key, 16 * 1024 * 1024)?
            .map(|b| image_data(&b))
            .transpose()
    }
}
pub(super) fn image_data(bytes: &[u8]) -> Result<String, Message> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(error)?.thumbnail(540, 720);
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .map_err(error)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encoded.into_inner())
    ))
}
pub(super) fn local_cover(base: &Path, value: &str) -> Result<Option<String>, Message> {
    let path = Path::new(value);
    let portable = value.replace('\\', "/");
    let candidate = if let Some(i) = portable.find("covers/") {
        base.join(&portable[i..])
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(value)
    };
    let Ok(root) = base.canonicalize() else {
        return Ok(None);
    };
    let Ok(actual) = candidate.canonicalize() else {
        return Ok(remote_cover(value));
    };
    if !actual.starts_with(root) || !actual.is_file() {
        return Ok(None);
    }
    let file = File::open(actual).map_err(error)?;
    if file.metadata().map_err(error)?.len() > 16 * 1024 * 1024 {
        return Err(Message::new("import.limit"));
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    image_data(&bytes).map(Some)
}
pub(super) fn remote_cover(value: &str) -> Option<String> {
    reqwest::Url::parse(value)
        .ok()
        .filter(|u| {
            matches!(u.scheme(), "https" | "http")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        })
        .map(|u| u.to_string())
}
