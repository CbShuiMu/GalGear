//! 无损截图持久化；像素采集和剪贴板属于桌面平台层。
use galgear_i18n::Message;
use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use std::path::{Path, PathBuf};

fn error(e: impl ToString) -> Message {
    Message::detail("screenshot.error", e)
}

/// 复用已有按键名称规则，并限制为修饰键加一个普通键；返回规范名称。
pub fn hotkey(value: &str) -> Result<String, Message> {
    let keys = crate::runtime::hotkey_keys(value)
        .map_err(|_| Message::new("screenshot.invalid_hotkey"))?;
    if keys
        .last()
        .is_none_or(|key| matches!(key, 0x10..=0x12 | 0x5b))
    {
        return Err(Message::new("screenshot.invalid_hotkey"));
    }
    let mut modifiers: Vec<_> = keys[..keys.len() - 1].to_vec();
    modifiers.sort_unstable();
    modifiers.push(keys[keys.len() - 1]);
    Ok(modifiers
        .into_iter()
        .filter_map(crate::runtime::hotkey_key_name)
        .collect::<Vec<_>>()
        .join("+"))
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
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

/// 返回游戏根目录中的 screenshots。按需创建，但拒绝链接、重解析点和非目录目标。
/// 游戏根目录允许是用户选择的链接，边界以其规范化后的实际目录为准。
pub fn folder(game: &Path, create: bool) -> Result<PathBuf, Message> {
    let root = game.canonicalize().map_err(error)?;
    if !root.is_dir() {
        return Err(Message::new("screenshot.folder_missing"));
    }
    let folder = root.join("screenshots");
    if create {
        match std::fs::create_dir(&folder) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(error(e)),
        }
    }
    match std::fs::symlink_metadata(&folder) {
        Ok(meta) if is_link(&meta) || !meta.is_dir() => Err(Message::new("screenshot.unsafe_path")),
        Ok(_) => {
            let resolved = folder.canonicalize().map_err(error)?;
            if resolved.parent() != Some(root.as_path()) {
                return Err(Message::new("screenshot.unsafe_path"));
            }
            Ok(resolved)
        }
        Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => Ok(folder),
        Err(e) => Err(error(e)),
    }
}

/// 读取最新 PNG 的预览；仅预览缩小，磁盘上的原始截图保持原尺寸。
/// 无目录或没有 PNG 返回 None；无权限、损坏图片等返回错误。
pub fn latest_preview(game: &Path) -> Result<Option<(String, Vec<u8>)>, Message> {
    let folder = folder(game, false)?;
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
    };
    let mut newest = None;
    for entry in entries {
        let path = entry.map_err(error)?.path();
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path).map_err(error)?;
        if is_link(&meta) || !meta.is_file() {
            continue;
        }
        let candidate = (meta.modified().map_err(error)?, path);
        if newest.as_ref().is_none_or(|current| candidate > *current) {
            newest = Some(candidate);
        }
    }
    let Some((_, path)) = newest else {
        return Ok(None);
    };
    let mut reader = image::ImageReader::open(&path).map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let original = reader.decode().map_err(error)?;
    let preview = original.thumbnail(960.min(original.width()), 540.min(original.height()));
    let mut bytes = std::io::Cursor::new(Vec::new());
    preview
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(error)?;
    Ok(Some((
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        bytes.into_inner(),
    )))
}

/// 将原尺寸 RGBA 像素保存为 PNG，成功前不暴露半成品，且不覆盖同名文件。
/// 目录必须已存在；尺寸与像素长度不匹配时返回错误。
pub fn save_png(folder: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<PathBuf, Message> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len() as u64) {
        return Err(Message::new("screenshot.invalid_frame"));
    }
    if !folder.is_dir() {
        return Err(Message::new("screenshot.folder_missing"));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(folder)
        .map_err(|e| Message::detail("screenshot.error", e))?;
    PngEncoder::new(temporary.as_file_mut())
        .write_image(rgba, width, height, ColorType::Rgba8.into())
        .map_err(|e| Message::detail("screenshot.error", e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| Message::detail("screenshot.error", e))?;
    let now = time::OffsetDateTime::now_utc();
    let name = format!(
        "GalGear-{:04}{:02}{:02}-{:02}{:02}{:02}-{:09}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond()
    );
    for suffix in 0..100 {
        let path = folder.join(format!("{name}-{suffix}.png"));
        match temporary.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.file
            }
            Err(error) => return Err(Message::detail("screenshot.error", error.error)),
        }
    }
    Err(Message::new("screenshot.error"))
}

#[cfg(test)]
mod tests {
    use super::{folder, hotkey, latest_preview, save_png};
    #[test]
    fn defaults_and_shortcuts_are_compatible_and_reject_modifier_only_chords() {
        let settings: crate::Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.screenshot_enabled);
        assert_eq!(settings.screenshot_hotkey, "Shift+S");
        assert_eq!(hotkey(" shift + s ").unwrap(), "Shift+S");
        assert_eq!(hotkey(" ctrl + s ").unwrap(), "Ctrl+S");
        let existing: crate::Settings =
            serde_json::from_str(r#"{"screenshot_hotkey":"Ctrl+S"}"#).unwrap();
        assert_eq!(existing.screenshot_hotkey, "Ctrl+S");
        assert_eq!(hotkey("Alt+F9").unwrap(), "Alt+F9");
        for invalid in ["", "Ctrl", "Ctrl+Alt", "S", "Ctrl+S+A", "Ctrl+Ctrl+S"] {
            assert!(hotkey(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn preview_distinguishes_missing_images_corruption_and_non_directory() {
        let game = tempfile::tempdir().unwrap();
        assert!(latest_preview(game.path()).unwrap().is_none());
        assert!(!game.path().join("screenshots").exists());
        let output = folder(game.path(), true).unwrap();
        std::fs::write(output.join("ignored.jpg"), b"not a screenshot").unwrap();
        assert!(latest_preview(game.path()).unwrap().is_none());
        let png = save_png(&output, 2, 1, &[255; 8]).unwrap();
        let (name, bytes) = latest_preview(game.path()).unwrap().unwrap();
        assert_eq!(std::path::Path::new(&name), png.file_name().unwrap());
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 2);
        std::fs::write(&png, b"broken").unwrap();
        assert!(latest_preview(game.path()).is_err());
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("screenshots"), b"existing data").unwrap();
        assert!(folder(other.path(), true).is_err());
        assert_eq!(
            std::fs::read(other.path().join("screenshots")).unwrap(),
            b"existing data"
        );
    }
    #[test]
    fn png_round_trip_preserves_pixels_and_existing_images() {
        let folder = tempfile::tempdir().unwrap();
        let pixels = [255, 0, 0, 255, 0, 128, 255, 255];
        let first = save_png(folder.path(), 2, 1, &pixels).unwrap();
        let second = save_png(folder.path(), 2, 1, &pixels).unwrap();
        assert_ne!(first, second);
        let decoded = image::open(first).unwrap().into_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.as_raw(), &pixels);
    }
    #[test]
    fn invalid_frames_and_missing_folders_leave_no_output() {
        let folder = tempfile::tempdir().unwrap();
        assert!(save_png(folder.path(), 2, 1, &[0; 4]).is_err());
        assert!(save_png(folder.path(), 0, 0, &[]).is_err());
        assert!(save_png(folder.path(), u32::MAX, u32::MAX, &[]).is_err());
        assert!(save_png(&folder.path().join("missing"), 1, 1, &[0; 4]).is_err());
        assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 0);
    }
}
