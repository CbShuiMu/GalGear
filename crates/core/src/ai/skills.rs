//! Download a commit-pinned repository snapshot before replacing the installed skills.
use galgear_i18n::Message;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    fs,
    io::{Read, Seek},
    path::{Component, Path},
    time::Duration,
};

const REPOSITORY: &str = "Galgear-GalgameReverse-Skill";
const COMMIT_FILE: &str = ".galgear-commit";
const PREVIOUS: &str = ".galgear-skills-previous";
const MAIN_URL: &str =
    "https://api.github.com/repos/CbShuiMu/Galgear-GalgameReverse-Skill/commits/main";
const ARCHIVE_URL: &str = "https://codeload.github.com/CbShuiMu/Galgear-GalgameReverse-Skill/zip";
const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_UNPACKED: u64 = 8 * 1024 * 1024 * 1024;
const REQUIRED: [&str; 4] = [
    "GalgameResourceExtract/SKILL.md",
    "GalgameResourceExtract/scripts/memory.py",
    "GalgameResourceExtract/catalog/formats.json",
    "GalgameResourceExtract/tools/GARbro-cli/GARbro.Console.exe",
];

fn error(detail: impl ToString) -> Message {
    Message::detail("skills.update_failed", detail)
}

fn valid_package(root: &Path) -> bool {
    REQUIRED.iter().all(|file| {
        root.join(file)
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() > 0)
    })
}

/// Checks main on each launch; unchanged commits reuse the local installation.
/// A failed download or validation leaves the installed version intact.
pub fn sync(home: &Path) -> Result<(), Message> {
    let client = Client::builder()
        // Skill updates use a direct connection, including the main-commit check.
        .no_proxy()
        .user_agent(concat!("GalGear/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(error)?;
    sync_from(home, &client, MAIN_URL, ARCHIVE_URL)
}

fn sync_from(
    home: &Path,
    client: &Client,
    main_url: &str,
    archive_url: &str,
) -> Result<(), Message> {
    // An OS lock also coordinates two app instances and is released after a crash.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join(".galgear-skills.lock"))
        .map_err(error)?;
    lock.lock().map_err(error)?;
    let installed = home.join(REPOSITORY);
    let previous = home.join(PREVIOUS);
    if previous.exists() {
        if !installed.exists() {
            fs::rename(&previous, &installed).map_err(error)?;
        } else {
            fs::remove_dir_all(&previous).map_err(error)?;
        }
    }
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let commit: Commit = client
        .get(main_url)
        .timeout(Duration::from_secs(30))
        .header("Accept", "application/vnd.github+json")
        .header("Cache-Control", "no-cache")
        .send()
        .map_err(error)?
        .error_for_status()
        .map_err(error)?
        .json()
        .map_err(error)?;
    if commit.sha.len() != 40 || !commit.sha.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(error("Invalid main commit SHA"));
    }
    if valid_package(&installed)
        && fs::read_to_string(installed.join(COMMIT_FILE)).is_ok_and(|sha| sha.trim() == commit.sha)
    {
        return Ok(());
    }
    let staging = tempfile::Builder::new()
        .prefix(".galgear-skills-")
        .tempdir_in(home)
        .map_err(error)?;
    let mut archive = tempfile::tempfile_in(staging.path()).map_err(error)?;
    let response = client
        .get(format!("{archive_url}/{}", commit.sha))
        .timeout(Duration::from_secs(1800))
        .send()
        .map_err(error)?
        .error_for_status()
        .map_err(error)?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE)
    {
        return Err(error("Skills archive exceeds size limit"));
    }
    if std::io::copy(&mut response.take(MAX_ARCHIVE + 1), &mut archive).map_err(error)?
        > MAX_ARCHIVE
    {
        return Err(error("Skills archive exceeds size limit"));
    }
    archive.rewind().map_err(error)?;
    let unpacked = staging.path().join("repository");
    extract(archive, &unpacked, &commit.sha)?;
    install(&unpacked, &installed, &previous, &commit.sha)
}

fn extract(archive: impl Read + Seek, destination: &Path, sha: &str) -> Result<(), Message> {
    let mut zip = zip::ZipArchive::new(archive).map_err(error)?;
    if zip.len() > 50_000 {
        return Err(error("Too many files in skills archive"));
    }
    let prefix = format!("{REPOSITORY}-{sha}/");
    let mut remaining = MAX_UNPACKED;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(error)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(error("Links are not allowed in skills archives"));
        }
        let name = entry
            .name()
            .strip_prefix(&prefix)
            .ok_or_else(|| error("Unexpected skills archive root"))?;
        if name.is_empty() {
            continue;
        }
        let relative = Path::new(name);
        if name.contains(['\\', ':'])
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(error("Unsafe skills archive path"));
        }
        let target = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(error)?;
            continue;
        }
        if entry.size() > remaining {
            return Err(error("Skills archive exceeds unpacked size limit"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(error)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(error)?;
        let copied =
            std::io::copy(&mut (&mut entry).take(remaining + 1), &mut output).map_err(error)?;
        remaining = remaining
            .checked_sub(copied)
            .ok_or_else(|| error("Skills archive exceeds unpacked size limit"))?;
    }
    if !valid_package(destination) {
        return Err(error("Skills archive is missing required resources"));
    }
    Ok(())
}

fn install(unpacked: &Path, installed: &Path, previous: &Path, sha: &str) -> Result<(), Message> {
    fs::write(unpacked.join(COMMIT_FILE), sha).map_err(error)?;
    let had_previous = installed.exists();
    if had_previous {
        fs::rename(installed, previous).map_err(error)?;
    }
    if let Err(failure) = fs::rename(unpacked, installed) {
        if had_previous {
            fs::rename(previous, installed)
                .map_err(|rollback| error(format!("{failure}; restore: {rollback}")))?;
        }
        return Err(error(failure));
    }
    // Cleanup failure must not turn a successful install into an unavailable runtime.
    if had_previous {
        let _ = fs::remove_dir_all(previous);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Cursor, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    fn archive(sha: &str, extra: Option<&str>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in REQUIRED.into_iter().chain(extra) {
            zip.start_file(
                format!("{REPOSITORY}-{sha}/{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"fixture").unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn server(
        responses: Vec<(u16, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                recorded
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (url, handle, requests)
    }

    #[test]
    fn downloads_initial_commit_skips_unchanged_and_replaces_new_commit() {
        let home = tempfile::tempdir().unwrap();
        let first = "a".repeat(40);
        let second = "b".repeat(40);
        let (url, handle, requests) = server(vec![
            (200, format!(r#"{{"sha":"{first}"}}"#).into_bytes()),
            (200, archive(&first, Some("old-file"))),
            (200, format!(r#"{{"sha":"{first}"}}"#).into_bytes()),
            (200, format!(r#"{{"sha":"{second}"}}"#).into_bytes()),
            (200, archive(&second, None)),
        ]);
        let client = Client::builder().no_proxy().build().unwrap();
        for _ in 0..3 {
            sync_from(
                home.path(),
                &client,
                &format!("{url}/main"),
                &format!("{url}/zip"),
            )
            .unwrap();
        }
        handle.join().unwrap();
        assert_eq!(
            fs::read_to_string(home.path().join(REPOSITORY).join(COMMIT_FILE)).unwrap(),
            second
        );
        assert!(!home.path().join(REPOSITORY).join("old-file").exists());
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests[1].starts_with(&format!("GET /zip/{first} ")));
        assert!(requests[4].starts_with(&format!("GET /zip/{second} ")));
    }

    #[test]
    fn failed_download_keeps_existing_package_and_commit() {
        let home = tempfile::tempdir().unwrap();
        let installed = home.path().join(REPOSITORY);
        let sha = "a".repeat(40);
        extract(Cursor::new(archive(&sha, None)), &installed, &sha).unwrap();
        fs::write(installed.join(COMMIT_FILE), &sha).unwrap();
        let next = "b".repeat(40);
        for failure in [(500, Vec::new()), (200, b"broken zip".to_vec())] {
            let (url, handle, _) = server(vec![
                (200, format!(r#"{{"sha":"{next}"}}"#).into_bytes()),
                failure,
            ]);
            assert!(
                sync_from(
                    home.path(),
                    &Client::builder().no_proxy().build().unwrap(),
                    &url,
                    &url
                )
                .is_err()
            );
            handle.join().unwrap();
            assert!(valid_package(&installed));
            assert_eq!(
                fs::read_to_string(installed.join(COMMIT_FILE)).unwrap(),
                sha
            );
        }
    }

    #[test]
    fn rejects_archive_paths_outside_the_package() {
        let home = tempfile::tempdir().unwrap();
        let sha = "a".repeat(40);
        for unsafe_path in ["../escaped", "/absolute", "C:/absolute", "folder\\escaped"] {
            let stage = tempfile::tempdir_in(home.path()).unwrap();
            assert!(
                extract(
                    Cursor::new(archive(&sha, Some(unsafe_path))),
                    stage.path(),
                    &sha
                )
                .is_err()
            );
        }
        assert!(!home.path().join("escaped").exists());
    }

    #[test]
    fn repairs_missing_resources_even_when_the_commit_has_not_changed() {
        let home = tempfile::tempdir().unwrap();
        let installed = home.path().join(REPOSITORY);
        let sha = "a".repeat(40);
        extract(Cursor::new(archive(&sha, None)), &installed, &sha).unwrap();
        fs::write(installed.join(COMMIT_FILE), &sha).unwrap();
        fs::remove_file(installed.join(REQUIRED[1])).unwrap();
        let (url, handle, _) = server(vec![
            (200, format!(r#"{{"sha":"{sha}"}}"#).into_bytes()),
            (200, archive(&sha, None)),
        ]);
        sync_from(
            home.path(),
            &Client::builder().no_proxy().build().unwrap(),
            &url,
            &url,
        )
        .unwrap();
        handle.join().unwrap();
        assert!(valid_package(&installed));
    }

    #[test]
    fn restores_interrupted_replacement_before_an_offline_check() {
        let home = tempfile::tempdir().unwrap();
        let sha = "a".repeat(40);
        let previous = home.path().join(PREVIOUS);
        extract(Cursor::new(archive(&sha, None)), &previous, &sha).unwrap();
        fs::write(previous.join(COMMIT_FILE), &sha).unwrap();
        let (url, handle, _) = server(vec![(503, Vec::new())]);
        assert!(
            sync_from(
                home.path(),
                &Client::builder().no_proxy().build().unwrap(),
                &url,
                &url
            )
            .is_err()
        );
        handle.join().unwrap();
        assert!(valid_package(&home.path().join(REPOSITORY)));
        assert_eq!(
            fs::read_to_string(home.path().join(REPOSITORY).join(COMMIT_FILE)).unwrap(),
            sha
        );
    }

    #[test]
    fn rejects_an_archive_without_required_resources() {
        let home = tempfile::tempdir().unwrap();
        let zip = zip::ZipWriter::new(Cursor::new(Vec::new()))
            .finish()
            .unwrap();
        assert!(extract(zip, home.path(), &"a".repeat(40)).is_err());
    }
}
