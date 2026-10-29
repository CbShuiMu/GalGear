//! Content-addressed artwork: publish both images before committing a completion marker.
use crate::Game;
use image::{DynamicImage, GenericImageView, codecs::jpeg::JpegEncoder, imageops::FilterType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const PIPELINE: &str = "waifu2x-20250915-cunet-cover1-background1-long3840-v2";
const LEGACY_PIPELINE: &str = "waifu2x-20250915-cunet-n1-long3840-v1";
// Conservative common profile targeting RTX 2060 Laptop, independent of the build machine.
const TILE_SIZES: [u32; 3] = [128, 64, 32];
const PREFIX: &str = "http://galgear-cover.localhost/";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artwork {
    pub pipeline: String,
    pub source: String,
    pub cover: String,
    pub background: String,
}

pub fn cover_key(url: &str) -> Option<&str> {
    url.strip_prefix(PREFIX).filter(|key| valid_key(key))
}
fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit())
}
pub fn url(key: &str) -> String {
    format!("{PREFIX}{key}")
}
pub fn image_path(cache: &Path, key: &str) -> PathBuf {
    cache.join(format!("{key}.jpg"))
}
fn marker_path(cache: &Path, source: &str) -> PathBuf {
    versioned_marker_path(cache, source, PIPELINE)
}
fn versioned_marker_path(cache: &Path, source: &str, pipeline: &str) -> PathBuf {
    cache
        .join("enhanced")
        .join(format!("{source}-{pipeline}.json"))
}

impl Artwork {
    pub fn available(&self, cache: &Path) -> bool {
        self.pipeline == PIPELINE && self.files_available(cache)
    }
    fn files_available(&self, cache: &Path) -> bool {
        [&self.source, &self.cover, &self.background]
            .iter()
            .all(|key| {
                valid_key(key) && fs::metadata(image_path(cache, key)).is_ok_and(|m| m.len() > 0)
            })
    }
    pub fn apply(&self, game: &mut Game) {
        game.cover = Some(url(&self.cover));
        game.cover_artwork = Some(self.clone());
    }
}

pub fn completed(cache: &Path, source: &str) -> Option<Artwork> {
    completed_version(cache, source, PIPELINE)
}
fn completed_version(cache: &Path, source: &str, pipeline: &str) -> Option<Artwork> {
    if !valid_key(source) {
        return None;
    }
    let record: Artwork =
        serde_json::from_slice(&fs::read(versioned_marker_path(cache, source, pipeline)).ok()?)
            .ok()?;
    (record.pipeline == pipeline && record.source == source && record.files_available(cache))
        .then_some(record)
}

/// Always regenerate from the original, even when an older pipeline is still displayed.
pub fn source_key(game: &Game) -> Option<&str> {
    let key = game.cover.as_deref().and_then(cover_key)?;
    Some(
        game.cover_artwork
            .as_ref()
            .filter(|record| {
                (record.cover == key || record.source == key) && valid_key(&record.source)
            })
            .map_or(key, |record| record.source.as_str()),
    )
}

/// Reuse results after scanning/restarting; invalidate associations when the user changes covers.
pub fn restore(game: &mut Game, cache: &Path) {
    let Some(source) = source_key(game).map(str::to_owned) else {
        game.cover_artwork = None;
        return;
    };
    let record = completed(cache, &source)
        .or_else(|| {
            game.cover_artwork
                .as_ref()
                .filter(|record| record.source == source && record.files_available(cache))
                .cloned()
        })
        .or_else(|| completed_version(cache, &source, LEGACY_PIPELINE));
    if let Some(record) = record {
        record.apply(game);
    } else {
        game.cover_artwork = None;
        game.cover = Some(url(&source));
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing cache directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(bytes).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

fn publish(cache: &Path, image: &DynamicImage) -> Result<String, String> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 94)
        .encode_image(&image.to_rgb8())
        .map_err(|e| e.to_string())?;
    // Validate encoded output before making it visible to the protocol handler.
    image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let key = format!("{:x}", Sha256::digest(&bytes));
    atomic_write(&image_path(cache, &key), &bytes)?;
    Ok(key)
}

fn read_image(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

#[cfg(windows)]
fn own_process(child: &std::process::Child) -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    // SAFETY: all pointers refer to initialized structures; the returned handle is owned once.
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = OwnedHandle::from_raw_handle(raw);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
            || AssignProcessToJobObject(raw, child.as_raw_handle()) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }
}

/// One low-priority process at a time; cancel between steps or while a game starts.
fn infer_once(
    engine: &Path,
    input: &Path,
    output: &Path,
    noise: &str,
    scale: &str,
    tile: u32,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    let mut command = Command::new(engine.join("waifu2x-ncnn-vulkan.exe"));
    command
        .current_dir(engine)
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .args([
            "-n",
            noise,
            "-s",
            scale,
            "-t",
            &tile.to_string(),
            "-j",
            "1:1:1",
            "-m",
        ])
        .arg("models-cunet")
        .stdout(Stdio::null());
    let log = fs::File::create(output.with_extension("log")).map_err(|e| e.to_string())?;
    command.stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000); // No console; below-normal priority.
    }
    if cancel() {
        return Err("Artwork processing paused".into());
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let _job = match own_process(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
    };
    let start = Instant::now();
    loop {
        if cancel() || start.elapsed() > Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Artwork processing paused or timed out".into());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let log = fs::read_to_string(output.with_extension("log")).unwrap_or_default();
                return Err(format!(
                    "waifu2x {status}: {}",
                    log.chars().take(1500).collect::<String>()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(500)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    }
}

fn with_memory_fallback(mut run: impl FnMut(u32) -> Result<(), String>) -> Result<(), String> {
    for (index, tile) in TILE_SIZES.into_iter().enumerate() {
        match run(tile) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let message = error.to_ascii_lowercase();
                let memory_error = [
                    "out of memory",
                    "out_of_device_memory",
                    "out_of_host_memory",
                    "vkallocatememory failed",
                    "failed to allocate",
                ]
                .iter()
                .any(|text| message.contains(text));
                if !memory_error || index + 1 == TILE_SIZES.len() {
                    return Err(error);
                }
            }
        }
    }
    Err("No usable inference tile size".into())
}

fn infer(
    engine: &Path,
    input: &Path,
    output: &Path,
    noise: &str,
    scale: &str,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    with_memory_fallback(|tile| {
        // A failed attempt may have left a partial output in this job's temporary directory.
        if output.exists() {
            fs::remove_file(output).map_err(|e| e.to_string())?;
        }
        infer_once(engine, input, output, noise, scale, tile, cancel)
    })
}

fn generate(
    cache: &Path,
    source: &str,
    mut run: impl FnMut(&Path, &Path, &str, &str) -> Result<(), String>,
) -> Result<Artwork, String> {
    let source_path = image_path(cache, source);
    let original = read_image(&source_path)?;
    let work = tempfile::tempdir_in(cache).map_err(|e| e.to_string())?;
    let denoised_path = work.path().join("denoised.png");
    run(&source_path, &denoised_path, "1", "1")?;
    let denoised = read_image(&denoised_path)?;
    if original.dimensions() != denoised.dimensions() {
        return Err("Denoised dimensions do not match".into());
    }
    let mut enlarged = denoised.clone();
    // The background has its own noise+scale inference, using the untouched source.
    // Already-large inputs reuse the validated denoising pass above.
    let mut input = source_path;
    let mut step = 0;
    while enlarged.width().max(enlarged.height()) < 3840 {
        let output = work.path().join(format!("upscale-{step}.png"));
        // Let waifu2x reuse the loaded model across its internal 2x passes.
        let mut scale = 2;
        while scale < 32 && enlarged.width().max(enlarged.height()) * scale < 3840 {
            scale *= 2;
        }
        run(&input, &output, "1", &scale.to_string())?;
        let next = read_image(&output)?;
        if next.dimensions() != (enlarged.width() * scale, enlarged.height() * scale) {
            return Err("Upscaled dimensions do not match".into());
        }
        enlarged = next;
        input = output;
        step += 1;
    }
    let background = enlarged.resize(3840, 3840, FilterType::Lanczos3);
    if background.width().max(background.height()) != 3840 {
        return Err("Invalid 4K background".into());
    }
    let record = Artwork {
        pipeline: PIPELINE.into(),
        source: source.into(),
        cover: publish(cache, &denoised)?,
        background: publish(cache, &background)?,
    };
    atomic_write(
        &marker_path(cache, source),
        &serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
    )?;
    Ok(record)
}

pub fn process(
    engine: &Path,
    cache: &Path,
    source: &str,
    cancel: impl Fn() -> bool,
) -> Result<Artwork, String> {
    if !valid_key(source) {
        return Err("Invalid cover key".into());
    }
    if let Some(record) = completed(cache, source) {
        return Ok(record);
    }
    // OS-owned lock is released even after a crash. Also deduplicates multiple app instances.
    let lock_dir = cache.join("enhanced");
    fs::create_dir_all(&lock_dir).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_dir.join("worker.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|e| e.to_string())?;
    if let Some(record) = completed(cache, source) {
        return Ok(record);
    }
    generate(cache, source, |input, output, noise, scale| {
        infer(engine, input, output, noise, scale, &cancel)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(cache: &Path) -> String {
        publish(cache, &DynamicImage::new_rgb8(480, 720)).unwrap()
    }
    #[test]
    fn failed_upscale_never_commits_or_changes_original() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path();
        let key = fixture(cache);
        let before = fs::read(image_path(cache, &key)).unwrap();
        let result = generate(cache, &key, |input, output, _, scale| {
            if scale != "1" {
                return Err("GPU failure".into());
            }
            image::open(input).unwrap().save(output).unwrap();
            Ok(())
        });
        assert!(result.is_err());
        assert!(completed(cache, &key).is_none());
        assert_eq!(before, fs::read(image_path(cache, &key)).unwrap());
    }
    #[test]
    fn successful_pair_is_reused_and_changed_cover_invalidates_it() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path();
        let key = fixture(cache);
        let mut passes = Vec::new();
        let record = generate(cache, &key, |input, output, noise, scale| {
            passes.push((input.to_owned(), noise.to_owned(), scale.to_owned()));
            let image = image::open(input).unwrap();
            let scale: u32 = scale.parse().unwrap();
            let image = if scale > 1 {
                image.resize_exact(
                    image.width() * scale,
                    image.height() * scale,
                    FilterType::Nearest,
                )
            } else {
                image
            };
            image.save(output).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(
            passes,
            vec![
                (image_path(cache, &key), "1".into(), "1".into()),
                (image_path(cache, &key), "1".into(), "8".into()),
            ]
        );
        assert_eq!(
            image::image_dimensions(image_path(cache, &record.background)).unwrap(),
            (2560, 3840)
        );
        // No engine exists: this can succeed only if generation is skipped.
        assert_eq!(
            process(Path::new("missing-engine"), cache, &key, || false).unwrap(),
            record
        );
        let mut game = crate::catalog::manual_game(Path::new("game.exe"), "Test".into()).unwrap();
        game.cover = Some(url(&key));
        restore(&mut game, cache);
        assert_eq!(game.cover_artwork.as_ref(), Some(&record));
        game.cover = Some(url(&"b".repeat(64)));
        restore(&mut game, cache);
        assert!(game.cover_artwork.is_none());
        record.apply(&mut game);
        fs::remove_file(image_path(cache, &record.background)).unwrap();
        restore(&mut game, cache);
        assert_eq!(game.cover, Some(url(&key)));
        assert!(game.cover_artwork.is_none());
    }
    #[test]
    fn rejects_traversal_keys() {
        assert!(cover_key("http://galgear-cover.localhost/../settings.json").is_none());
    }
    #[test]
    fn invalid_model_output_never_creates_a_completion_marker() {
        let root = tempfile::tempdir().unwrap();
        let key = fixture(root.path());
        let result = generate(root.path(), &key, |_, output, _, _| {
            fs::write(output, b"truncated image").map_err(|e| e.to_string())
        });
        assert!(result.is_err());
        assert!(completed(root.path(), &key).is_none());
    }

    #[test]
    fn version_upgrade_keeps_old_artwork_until_new_pair_is_committed() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path();
        let old = Artwork {
            pipeline: LEGACY_PIPELINE.into(),
            source: "a".repeat(64),
            cover: "b".repeat(64),
            background: "c".repeat(64),
        };
        for key in [&old.source, &old.cover, &old.background] {
            fs::write(image_path(cache, key), b"existing image").unwrap();
        }
        atomic_write(
            &versioned_marker_path(cache, &old.source, LEGACY_PIPELINE),
            &serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        let mut game = crate::catalog::manual_game(Path::new("game.exe"), "Test".into()).unwrap();
        old.apply(&mut game);
        restore(&mut game, cache);
        assert_eq!(game.cover_artwork.as_ref(), Some(&old));
        assert_eq!(source_key(&game), Some(old.source.as_str()));
        assert!(!old.available(cache));
        // Full scans start from the original cover; they must also retain the old background.
        game.cover_artwork = None;
        game.cover = Some(url(&old.source));
        restore(&mut game, cache);
        assert_eq!(game.cover_artwork.as_ref(), Some(&old));
        let new = Artwork {
            pipeline: PIPELINE.into(),
            background: "d".repeat(64),
            ..old.clone()
        };
        fs::write(image_path(cache, &new.background), b"new validated image").unwrap();
        atomic_write(
            &marker_path(cache, &new.source),
            &serde_json::to_vec(&new).unwrap(),
        )
        .unwrap();
        restore(&mut game, cache);
        assert_eq!(game.cover_artwork.as_ref(), Some(&new));
        assert!(new.available(cache));
    }

    #[test]
    fn memory_pressure_retries_smaller_tiles_but_other_errors_stop() {
        let mut attempts = Vec::new();
        with_memory_fallback(|tile| {
            attempts.push(tile);
            if tile > 32 {
                Err("vkAllocateMemory failed -2".into())
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(attempts, vec![128, 64, 32]);
        for message in ["paused", "timed out", "missing model file"] {
            attempts.clear();
            assert!(
                with_memory_fallback(|tile| {
                    attempts.push(tile);
                    Err(message.into())
                })
                .is_err()
            );
            assert_eq!(attempts, vec![128]);
        }
        attempts.clear();
        assert!(
            with_memory_fallback(|tile| {
                attempts.push(tile);
                Err("out of memory".into())
            })
            .is_err()
        );
        assert_eq!(attempts, vec![128, 64, 32]);
    }
}
