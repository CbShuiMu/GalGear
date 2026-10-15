//! Extract launcher resources without starting the executable; cache by path and file version.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn cached_icon(executable: &Path, cache: &Path) -> Option<PathBuf> {
    if !executable
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let metadata = executable.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let stamp = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let key = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{}:{stamp}",
            executable.to_string_lossy().to_lowercase(),
            metadata.len()
        ))
    );
    let target = cache.join(format!("{key}.png"));
    if target.is_file() {
        return Some(target);
    }
    let rgba = extract(executable)?;
    std::fs::create_dir_all(cache).ok()?;
    let mut temporary = tempfile::NamedTempFile::new_in(cache).ok()?;
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut temporary, image::ImageFormat::Png)
        .ok()?;
    match temporary.persist_noclobber(&target) {
        Ok(_) => Some(target),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Some(target),
        Err(_) => None,
    }
}

#[cfg(windows)]
fn extract(path: &Path) -> Option<image::RgbaImage> {
    use windows_sys::Win32::{
        Graphics::Gdi::*,
        UI::{
            Shell::ExtractIconExW,
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx},
        },
    };
    let path: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let mut icon = std::ptr::null_mut();
        if ExtractIconExW(path.as_ptr(), 0, &mut icon, std::ptr::null_mut(), 1) == 0
            || icon.is_null()
        {
            return None;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            DestroyIcon(icon);
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 32,
                biHeight: -32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut pixels,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() {
            DeleteDC(dc);
            DestroyIcon(icon);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        // A white icon tile also renders legacy masked icons correctly, matching the UI's surface.
        std::ptr::write_bytes(pixels.cast::<u8>(), 255, 32 * 32 * 4);
        let drawn = DrawIconEx(dc, 0, 0, icon, 32, 32, 0, std::ptr::null_mut(), DI_NORMAL) != 0;
        GdiFlush();
        let result = if drawn {
            let mut rgba = std::slice::from_raw_parts(pixels.cast::<u8>(), 32 * 32 * 4).to_vec();
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            image::RgbaImage::from_raw(32, 32, rgba)
        } else {
            None
        };
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        DestroyIcon(icon);
        result
    }
}
#[cfg(not(windows))]
fn extract(_: &Path) -> Option<image::RgbaImage> {
    None
}
