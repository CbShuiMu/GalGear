//! Verified release downloads and reversible portable-file replacement.
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::{Read, Write}, path::{Component, Path, PathBuf}, time::Duration};

fn error(e: impl std::fmt::Display) -> Message { Message::detail("update.failed", e) }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub available: bool,
    pub asset: Option<Asset>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset { pub name: String, pub url: String, pub size: u64, pub sha256: String }

pub fn endpoint(page: &str) -> Result<String, Message> {
    let url = reqwest::Url::parse(page).map_err(error)?;
    let parts: Vec<_> = url.path_segments().into_iter().flatten().collect();
    if url.scheme() != "https" || url.host_str() != Some("github.com")
        || !url.username().is_empty() || url.password().is_some()
        || parts.len() != 3 || parts[2] != "releases"
        || parts[..2].iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
    { return Err(Message::new("error.site_url")); }
    Ok(format!("https://api.github.com/repos/{}/{}/releases/latest", parts[0], parts[1]))
}

pub fn parse_release(value: &serde_json::Value, current: &str) -> Result<Release, Message> {
    let version = value["tag_name"].as_str().ok_or_else(|| error("Missing release version"))?.to_owned();
    let latest = semver::Version::parse(version.trim_start_matches('v')).map_err(error)?;
    let current = semver::Version::parse(current.trim_start_matches('v')).map_err(error)?;
    let asset = value["assets"].as_array().into_iter().flatten().filter_map(|entry| {
        let name = entry["name"].as_str()?;
        let lower = name.to_ascii_lowercase();
        let rank = if lower.starts_with("galgear") && lower.contains("portable") && (lower.contains("win64") || lower.contains("x64")) && lower.ends_with(".zip") { 0 }
            else if lower == "galgear.exe" { 1 } else { return None };
        let url = entry["browser_download_url"].as_str()?;
        let parsed = reqwest::Url::parse(url).ok()?;
        if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") { return None; }
        let sha256 = entry["digest"].as_str()?.strip_prefix("sha256:")?;
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
        let size = entry["size"].as_u64()?;
        if size == 0 || size > 4 * 1024 * 1024 * 1024 { return None; }
        Some((rank, Asset { name: name.into(), url: url.into(), size, sha256: sha256.to_ascii_lowercase() }))
    }).min_by_key(|(rank, _)| *rank).map(|(_, asset)| asset);
    Ok(Release { version, available: latest > current, asset })
}

pub fn check(page: &str, current: &str) -> Result<Release, Message> {
    let response: serde_json::Value = crate::providers::client()?.get(endpoint(page)?)
        .send().map_err(error)?.error_for_status().map_err(error)?.json().map_err(error)?;
    parse_release(&response, current)
}

pub fn download(asset: &Asset, output: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), Message> {
    let response = reqwest::blocking::Client::builder().https_only(true)
        .user_agent("GalGear updater").connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(1800)).build().map_err(error)?
        .get(&asset.url).send().map_err(error)?.error_for_status().map_err(error)?;
    copy_verified(response, asset, output, &mut progress)
}

fn copy_verified(mut response: impl Read, asset: &Asset, output: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), Message> {
    let mut file = fs::File::create(output).map_err(error)?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut bytes = [0; 64 * 1024];
    loop {
        let count = response.read(&mut bytes).map_err(error)?;
        if count == 0 { break; }
        total += count as u64;
        if total > asset.size { return Err(error("Download exceeds release size")); }
        file.write_all(&bytes[..count]).map_err(error)?;
        hash.update(&bytes[..count]);
        progress(total, asset.size);
    }
    file.sync_all().map_err(error)?;
    if total != asset.size || format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err(Message::new("update.checksum_failed"));
    }
    Ok(())
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|part| matches!(part, Component::Normal(value)
        if !value.to_string_lossy().contains(':') && !value.to_string_lossy().ends_with(['.', ' '])) )
}

pub fn unpack(archive: &Path, destination: &Path, zipped: bool) -> Result<PathBuf, Message> {
    fs::create_dir_all(destination).map_err(error)?;
    if !zipped {
        fs::copy(archive, destination.join("GalGear.exe")).map_err(error)?;
    } else {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(error)?).map_err(error)?;
        if zip.len() > 20000 { return Err(error("Too many update entries")); }
        let mut total = 0u64;
        let mut names = BTreeSet::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(error)?;
            let path = entry.enclosed_name().ok_or_else(|| error("Unsafe update path"))?;
            total = total.saturating_add(entry.size());
            if !safe_relative(&path) || total > 16 * 1024 * 1024 * 1024
                || entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000)
                || !names.insert(path.to_string_lossy().to_lowercase()) { return Err(error("Unsafe update entry")); }
            let output = destination.join(path);
            if entry.is_dir() { fs::create_dir_all(output).map_err(error)?; }
            else {
                fs::create_dir_all(output.parent().ok_or_else(|| error("Missing parent"))?).map_err(error)?;
                let mut file = fs::File::create(output).map_err(error)?;
                let expected = entry.size();
                let copied = std::io::copy(&mut (&mut entry).take(expected + 1), &mut file).map_err(error)?;
                if copied != expected { return Err(error("Invalid update entry size")); }
            }
        }
    }
    let root = if destination.join("GalGear.exe").is_file() { destination.to_path_buf() } else {
        let entries = fs::read_dir(destination).map_err(error)?.collect::<Result<Vec<_>, _>>().map_err(error)?;
        if entries.len() != 1 || !entries[0].path().join("GalGear.exe").is_file() { return Err(Message::new("update.package_invalid")); }
        entries[0].path()
    };
    let mut signature = [0; 2];
    fs::File::open(root.join("GalGear.exe")).map_err(error)?.read_exact(&mut signature).map_err(error)?;
    if signature != *b"MZ" { return Err(Message::new("update.package_invalid")); }
    Ok(root)
}

fn preserve(root: &Path, path: &Path) -> bool {
    let value = path.to_string_lossy().replace('\\', "/").to_lowercase();
    value == "portable.flag" || value == "data" || value.starts_with("data/")
        || value == "settings.json" || value.starts_with(".galgear")
        || (value == "tools/magpie/config/config.json" || value == "tools/locale-emulator/leconfig.xml") && root.join(path).exists()
}
fn reject_links(path: &Path) -> Result<(), Message> {
    for path in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            let mut link = metadata.is_symlink();
            #[cfg(windows)] { use std::os::windows::fs::MetadataExt; link |= metadata.file_attributes() & 0x400 != 0; }
            if link { return Err(error("Update destination contains a link")); }
        }
    }
    Ok(())
}

pub struct Transaction { changed: Vec<(PathBuf, Option<PathBuf>)> }
impl Transaction {
    pub fn rollback(&self) -> Result<(), Message> {
        for (target, backup) in self.changed.iter().rev() {
            if target.exists() { fs::remove_file(target).map_err(error)?; }
            if let Some(backup) = backup { fs::rename(backup, target).map_err(error)?; }
        }
        Ok(())
    }
}

pub fn replace(payload: &Path, executable: &Path, backup: &Path) -> Result<Transaction, Message> {
    let root = executable.parent().ok_or_else(|| error("Missing application directory"))?;
    reject_links(root)?;
    fs::create_dir_all(backup).map_err(error)?;
    let mut pending = vec![payload.to_path_buf()];
    let mut files = Vec::new();
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).map_err(error)? {
            let entry = entry.map_err(error)?;
            let path = entry.path();
            reject_links(&path)?;
            let relative = path.strip_prefix(payload).map_err(error)?;
            if !safe_relative(relative) { return Err(error("Unsafe payload path")); }
            if preserve(root, relative) { continue; }
            if entry.file_type().map_err(error)?.is_dir() { pending.push(path); }
            else {
                let target = if relative == Path::new("GalGear.exe") { executable.to_path_buf() } else { root.join(relative) };
                reject_links(&target)?;
                files.push((path, target));
            }
        }
    }
    // Replace the executable last so every dependency is ready before restart.
    files.sort_by_key(|(_, target)| target == executable);
    let mut transaction = Transaction { changed: Vec::new() };
    for (index, (source, target)) in files.iter().enumerate() {
        let result = (|| {
            fs::create_dir_all(target.parent().ok_or_else(|| error("Missing parent"))?).map_err(error)?;
            let saved = if target.exists() {
                if !target.is_file() { return Err(error("Update target is not a file")); }
                let saved = backup.join(index.to_string());
                fs::rename(target, &saved).map_err(error)?;
                Some(saved)
            } else { None };
            transaction.changed.push((target.clone(), saved));
            fs::copy(source, target).map_err(error)?;
            Ok(())
        })();
        if let Err(failure) = result {
            transaction.rollback()?;
            return Err(failure);
        }
    }
    Ok(transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_use_semver_and_only_select_verifiable_portable_windows_assets() {
        let value = serde_json::json!({"tag_name":"v0.10.0","assets":[
            {"name":"GalGear_0.10.0_x64-setup.exe","browser_download_url":"https://github.com/a/b/releases/download/v0.10.0/setup.exe","size":20,"digest":format!("sha256:{}","a".repeat(64))},
            {"name":"GalGear-0.10.0-win64-portable.zip","browser_download_url":"https://github.com/a/b/releases/download/v0.10.0/app.zip","size":20,"digest":format!("sha256:{}","a".repeat(64))}
        ]});
        let release = parse_release(&value,"0.9.0").unwrap();
        assert!(release.available);
        assert!(release.asset.unwrap().name.ends_with("portable.zip"));
        assert!(!parse_release(&value,"0.10.0").unwrap().available);
        assert!(!parse_release(&value,"0.11.0").unwrap().available);
        assert!(endpoint("https://github.com/a/b/releases").is_ok());
        assert!(endpoint("https://github.com/a/b/issues").is_err());
        let mut value = value;
        value["assets"][1]["digest"] = serde_json::Value::Null;
        assert!(parse_release(&value,"0.9.0").unwrap().asset.is_none());
    }
    #[test]
    fn downloads_require_matching_size_and_hash() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = b"MZfixture";
        let mut asset = Asset { name:"GalGear.exe".into(),url:String::new(),size:bytes.len() as u64,sha256:format!("{:x}",Sha256::digest(bytes)) };
        let output = temp.path().join("download");
        copy_verified(&bytes[..],&asset,&output,|_,_|{}).unwrap();
        assert_eq!(fs::read(&output).unwrap(),bytes);
        asset.sha256 = "0".repeat(64);
        assert!(copy_verified(&bytes[..],&asset,&output,|_,_|{}).is_err());
        asset.size = 1;
        assert!(copy_verified(&bytes[..],&asset,&output,|_,_|{}).is_err());
    }
    fn archive(path: &Path, names: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, bytes) in names {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    #[test]
    fn unpack_rejects_traversal_duplicates_and_non_executable_payloads() {
        for names in [vec![("../escape", &b"bad"[..])],vec![("GalGear.exe",&b"html"[..])],vec![("GalGear.exe",&b"MZ1"[..]),("galgear.exe",&b"MZ2"[..])]] {
            let temp=tempfile::tempdir().unwrap();
            let zip=temp.path().join("app.zip");
            archive(&zip,&names);
            assert!(unpack(&zip,&temp.path().join("payload"),true).is_err());
        }
    }
    #[test]
    fn replacement_preserves_user_data_and_rolls_back_on_any_failure() {
        let temp=tempfile::tempdir().unwrap();
        let install=temp.path().join("installed");
        let payload=temp.path().join("payload");
        fs::create_dir_all(install.join("data")).unwrap();
        fs::create_dir_all(payload.join("data")).unwrap();
        fs::write(install.join("renamed.exe"),b"MZold").unwrap();
        fs::write(install.join("data/settings.json"),b"user config").unwrap();
        fs::write(payload.join("GalGear.exe"),b"MZnew").unwrap();
        fs::write(payload.join("data/settings.json"),b"release config").unwrap();
        fs::write(payload.join("portable.flag"),b"").unwrap();
        fs::write(payload.join("tool.dll"),b"new tool").unwrap();
        let transaction=replace(&payload,&install.join("renamed.exe"),&temp.path().join("backup")).unwrap();
        assert_eq!(fs::read(install.join("renamed.exe")).unwrap(),b"MZnew");
        assert_eq!(fs::read(install.join("data/settings.json")).unwrap(),b"user config");
        assert!(!install.join("portable.flag").exists());
        transaction.rollback().unwrap();
        assert_eq!(fs::read(install.join("renamed.exe")).unwrap(),b"MZold");
        assert!(!install.join("tool.dll").exists());
        // A conflict at the executable is discovered after other files were copied.
        fs::remove_file(install.join("renamed.exe")).unwrap();
        fs::create_dir(install.join("renamed.exe")).unwrap();
        assert!(replace(&payload,&install.join("renamed.exe"),&temp.path().join("backup2")).is_err());
        assert!(!install.join("tool.dll").exists());
    }
}
