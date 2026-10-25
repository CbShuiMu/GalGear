//! Bounded, verified save transfers through standard WebDAV collections.
use crate::snapshots::{self, Snapshot};
use galgear_i18n::Message;
use reqwest::{
    Method, StatusCode, Url,
    blocking::{Body, Client, RequestBuilder, Response},
    header::{CONTENT_LENGTH, CONTENT_TYPE, IF_NONE_MATCH},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_ARCHIVE: u64 = 500 * 1024 * 1024;
const MAX_CONTENT: u64 = 2 * 1024 * 1024 * 1024;
const MAX_METADATA: u64 = 4 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_HISTORY: usize = 300;
const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;

/// Public configuration; passwords are stored separately in the credential vault.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudSettings {
    pub url: String,
    pub username: String,
    pub remote_path: String,
}

impl CloudSettings {
    /// Normalize the endpoint and collection path without contacting the server.
    /// An entirely empty configuration clears cloud storage.
    pub fn normalized(&self) -> Result<Self, Message> {
        let address = self.url.trim();
        let username = self.username.trim();
        let remote = self.remote_path.trim().trim_matches('/');
        if address.is_empty() && username.is_empty() && remote.is_empty() {
            return Ok(Self::default());
        }
        if address.len() > 2048
            || address.chars().any(char::is_control)
            || username.is_empty()
            || username.len() > 512
            || username.chars().any(|c| c.is_control() || c == ':')
            || remote.len() > 1024
        {
            return Err(Message::new("cloud.invalid_settings"));
        }
        let mut url = Url::parse(address).map_err(|_| Message::new("cloud.invalid_settings"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Message::new("cloud.invalid_settings"));
        }
        let remote = if remote.is_empty() { "GalGear" } else { remote };
        if remote.split('/').any(|part| {
            part.is_empty()
                || matches!(part, "." | "..")
                || part.len() > 255
                || part
                    .chars()
                    .any(|c| c.is_control() || c == '\\' || c == ':')
        }) {
            return Err(Message::new("cloud.invalid_settings"));
        }
        if !url.path().ends_with('/') {
            url.path_segments_mut()
                .map_err(|_| Message::new("cloud.invalid_settings"))?
                .push("");
        }
        Ok(Self {
            url: url.to_string(),
            username: username.into(),
            remote_path: remote.into(),
        })
    }

    /// Require a configured endpoint and valid account and collection fields.
    pub fn validate(&self) -> Result<(), Message> {
        if self.normalized()?.url.is_empty() {
            return Err(Message::new("cloud.invalid_settings"));
        }
        Ok(())
    }
}

/// Validate an application password without placing it in errors or logs.
pub fn validate_password(password: &str) -> Result<(), Message> {
    if password.is_empty() || password.len() > 1024 || password.chars().any(char::is_control) {
        return Err(Message::new("cloud.invalid_password"));
    }
    Ok(())
}

/// A completed remote snapshot, available to download into local history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudSnapshot {
    pub id: String,
    pub name: String,
    pub created: u64,
    pub size: u64,
    pub note: String,
}

impl From<&Snapshot> for CloudSnapshot {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            id: snapshot.id.clone(),
            name: snapshot.name.clone(),
            created: snapshot.created,
            size: snapshot.size,
            note: snapshot.note.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RemoteManifest {
    version: u32,
    cloud_key: String,
    archive_size: u64,
    archive_hash: String,
    snapshot: Snapshot,
}

/// A synchronous WebDAV client. Redirects are disabled to keep credentials scoped
/// to the configured server; network errors never include request URLs or bodies.
pub struct CloudClient {
    client: Client,
    endpoint: Url,
    remote: Vec<String>,
    username: String,
    password: String,
}

impl CloudClient {
    /// Create a client without performing a request.
    pub fn new(settings: &CloudSettings, password: &str) -> Result<Self, Message> {
        settings.validate()?;
        validate_password(password)?;
        let settings = settings.normalized()?;
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(600))
                .redirect(Policy::none())
                .user_agent("GalGear/WebDAV")
                .build()
                .map_err(|_| Message::new("cloud.network"))?,
            endpoint: Url::parse(&settings.url)
                .map_err(|_| Message::new("cloud.invalid_settings"))?,
            remote: settings.remote_path.split('/').map(str::to_owned).collect(),
            username: settings.username,
            password: password.into(),
        })
    }

    fn request(&self, method: Method, url: Url) -> RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    fn url(&self, segments: &[&str], collection: bool) -> Result<Url, Message> {
        let mut url = self.endpoint.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| Message::new("cloud.invalid_settings"))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
            if collection {
                path.push("");
            }
        }
        Ok(url)
    }

    fn collection(&self, key: Option<&str>) -> Result<Url, Message> {
        let mut segments: Vec<_> = self.remote.iter().map(String::as_str).collect();
        if let Some(key) = key {
            validate_key(key)?;
            segments.push(key);
        }
        self.url(&segments, true)
    }

    fn item(&self, key: &str, id: &str, extension: &str) -> Result<Url, Message> {
        validate_key(key)?;
        validate_id(id)?;
        let name = format!("{id}.{extension}");
        let mut segments: Vec<_> = self.remote.iter().map(String::as_str).collect();
        segments.extend([key, &name]);
        self.url(&segments, false)
    }

    fn ensure_collections(&self, key: Option<&str>) -> Result<(), Message> {
        let mut segments: Vec<_> = self.remote.iter().map(String::as_str).collect();
        if let Some(key) = key {
            validate_key(key)?;
            segments.push(key);
        }
        for length in 1..=segments.len() {
            let directory = self.url(&segments[..length], true)?;
            let response = send(self.request(
                Method::from_bytes(b"MKCOL").map_err(local_error)?,
                directory.clone(),
            ))?;
            if response.status() == StatusCode::METHOD_NOT_ALLOWED {
                let bytes = read_bounded(self.propfind(directory.clone(), "0")?, MAX_METADATA)?;
                parse_listing(&bytes, &directory)?;
            } else {
                check_response(response)?;
            }
        }
        Ok(())
    }

    fn propfind(&self, collection: Url, depth: &str) -> Result<Response, Message> {
        let response = send(
            self.request(
                Method::from_bytes(b"PROPFIND").map_err(local_error)?,
                collection,
            )
            .header("Depth", depth)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND),
        )?;
        if response.status() != StatusCode::MULTI_STATUS {
            return Err(status_error(response.status()));
        }
        Ok(response)
    }

    /// Check collection access and verify write permission with a temporary probe.
    pub fn test_connection(&self) -> Result<(), Message> {
        self.ensure_collections(None)?;
        let directory = self.collection(None)?;
        let xml = read_bounded(self.propfind(directory.clone(), "0")?, MAX_METADATA)?;
        parse_listing(&xml, &directory)?;
        let temporary = tempfile::NamedTempFile::new().map_err(local_error)?;
        let nonce = unique_id(temporary.path())?;
        let name = format!(".galgear-probe-{nonce}");
        let mut segments: Vec<_> = self.remote.iter().map(String::as_str).collect();
        segments.push(&name);
        let url = self.url(&segments, false)?;
        check_response(send(
            self.request(Method::PUT, url.clone())
                .header(IF_NONE_MATCH, "*")
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(Vec::new()),
        )?)?;
        check_response(send(self.request(Method::DELETE, url))?)?;
        Ok(())
    }

    /// Upload a verified local snapshot. The JSON completion marker is written
    /// after its ZIP; existing remote identities are never intentionally replaced.
    pub fn upload(
        &self,
        data: &Path,
        snapshot: &Snapshot,
        cloud_key: &str,
    ) -> Result<CloudSnapshot, Message> {
        validate_key(cloud_key)?;
        let mut portable = snapshot.clone();
        for file in &mut portable.files {
            file.path = file.path.replace('\\', "/");
        }
        let snapshot = &portable;
        validate_snapshot(snapshot)?;
        let source = snapshots::content_directory(data, &snapshot.game_id, &snapshot.id)?;
        let mut archive = build_archive(&source, snapshot)?;
        let archive_size = archive.as_file().metadata().map_err(local_error)?.len();
        let archive_hash = hash_reader(archive.as_file_mut())?;
        let mut remote_snapshot = snapshot.clone();
        // Absolute paths are private and can never be restore destinations on another PC.
        remote_snapshot.roots = (0..snapshot.roots.len())
            .map(|index| format!("Save {}", index + 1))
            .collect();
        let manifest = RemoteManifest {
            version: 1,
            cloud_key: cloud_key.into(),
            archive_size,
            archive_hash,
            snapshot: remote_snapshot,
        };
        let metadata = serde_json::to_vec(&manifest).map_err(local_error)?;
        if metadata.len() as u64 > MAX_METADATA {
            return Err(Message::new("cloud.too_large"));
        }
        self.ensure_collections(Some(cloud_key))?;
        let archive_url = self.item(cloud_key, &snapshot.id, "zip")?;
        check_response(send(
            self.request(Method::PUT, archive_url.clone())
                .header(IF_NONE_MATCH, "*")
                .header(CONTENT_TYPE, "application/zip")
                .header(CONTENT_LENGTH, archive_size)
                .body(Body::new(archive.reopen().map_err(local_error)?)),
        )?)?;
        let publish = send(
            self.request(Method::PUT, self.item(cloud_key, &snapshot.id, "json")?)
                .header(IF_NONE_MATCH, "*")
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .body(metadata),
        )
        .and_then(check_response);
        if let Err(error) = publish {
            let _ = send(self.request(Method::DELETE, archive_url));
            return Err(error);
        }
        Ok(CloudSnapshot::from(snapshot))
    }

    fn manifest(&self, key: &str, id: &str) -> Result<RemoteManifest, Message> {
        let response = check_response(send(
            self.request(Method::GET, self.item(key, id, "json")?),
        )?)?;
        let bytes = read_bounded(response, MAX_METADATA)?;
        let manifest: RemoteManifest =
            serde_json::from_slice(&bytes).map_err(|_| invalid_backup())?;
        if manifest.version != 1
            || manifest.cloud_key != key
            || manifest.snapshot.id != id
            || manifest.archive_size > MAX_ARCHIVE
            || manifest.archive_size == 0
            || !valid_hash(&manifest.archive_hash)
        {
            return Err(invalid_backup());
        }
        validate_snapshot(&manifest.snapshot)?;
        Ok(manifest)
    }

    /// List completed snapshots; unfinished ZIP uploads have no completion marker.
    pub fn list(&self, cloud_key: &str) -> Result<Vec<CloudSnapshot>, Message> {
        let directory = self.collection(Some(cloud_key))?;
        let response = match self.propfind(directory.clone(), "1") {
            Ok(response) => response,
            Err(error) if error.key == "cloud.not_found" => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let bytes = read_bounded(response, MAX_METADATA)?;
        let ids = parse_listing(&bytes, &directory)?;
        if ids.len() > MAX_HISTORY {
            return Err(Message::new("cloud.too_large"));
        }
        let mut snapshots = Vec::new();
        for id in ids {
            match self.manifest(cloud_key, &id) {
                Ok(manifest) => snapshots.push(CloudSnapshot::from(&manifest.snapshot)),
                // Another client may remove a snapshot after PROPFIND.
                Err(error) if error.key == "cloud.not_found" => continue,
                Err(error) => return Err(error),
            }
        }
        snapshots.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
        Ok(snapshots)
    }

    /// Download and verify into local snapshot history. This never writes to any
    /// live save directory; restoration still requires the normal preview flow.
    pub fn download(
        &self,
        data: &Path,
        game_id: &str,
        cloud_key: &str,
        snapshot_id: &str,
    ) -> Result<Snapshot, Message> {
        validate_id(game_id)?;
        let manifest = self.manifest(cloud_key, snapshot_id)?;
        let response = check_response(send(
            self.request(Method::GET, self.item(cloud_key, snapshot_id, "zip")?),
        )?)?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_ARCHIVE || size != manifest.archive_size)
        {
            return Err(invalid_backup());
        }
        let mut archive = tempfile::NamedTempFile::new().map_err(local_error)?;
        let length = std::io::copy(&mut response.take(MAX_ARCHIVE + 1), archive.as_file_mut())
            .map_err(|_| Message::new("cloud.network"))?;
        if length > MAX_ARCHIVE {
            return Err(Message::new("cloud.too_large"));
        }
        if length != manifest.archive_size
            || hash_reader(archive.as_file_mut())? != manifest.archive_hash
        {
            return Err(invalid_backup());
        }
        archive
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(local_error)?;
        let staged = tempfile::tempdir().map_err(local_error)?;
        extract_archive(archive.as_file_mut(), &manifest.snapshot, staged.path())?;
        snapshots::import_verified(data, game_id, manifest.snapshot, staged.path())
    }
}

fn send(request: RequestBuilder) -> Result<Response, Message> {
    request.send().map_err(|_| Message::new("cloud.network"))
}

fn check_response(response: Response) -> Result<Response, Message> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(status_error(response.status()))
    }
}

fn status_error(status: StatusCode) -> Message {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Message::new("cloud.auth"),
        StatusCode::NOT_FOUND => Message::new("cloud.not_found"),
        StatusCode::CONFLICT | StatusCode::PRECONDITION_FAILED => Message::new("cloud.conflict"),
        StatusCode::PAYLOAD_TOO_LARGE => Message::new("cloud.too_large"),
        _ => Message::new("cloud.http").arg("status", status.as_u16()),
    }
}

fn invalid_backup() -> Message {
    Message::new("cloud.invalid_backup")
}

fn local_error(error: impl std::fmt::Display) -> Message {
    Message::detail("cloud.error", error)
}

fn validate_id(id: &str) -> Result<(), Message> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(invalid_backup());
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), Message> {
    if !valid_hash(key) {
        return Err(invalid_backup());
    }
    Ok(())
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        && path.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && !matches!(part, "." | "..")
                && !part.ends_with([' ', '.'])
                && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), Message> {
    validate_id(&snapshot.id)?;
    validate_id(&snapshot.game_id)?;
    if snapshot.roots.is_empty()
        || snapshot.roots.len() > 32
        || snapshot.files.is_empty()
        || snapshot.files.len() > MAX_FILES
        || snapshot.name.len() > 1024
        || snapshot.note.len() > 16 * 1024
    {
        return Err(invalid_backup());
    }
    let mut identities = BTreeSet::new();
    let mut size = 0u64;
    for file in &snapshot.files {
        if file.root >= snapshot.roots.len()
            || !safe_relative(&file.path)
            || !valid_hash(&file.hash)
            || !identities.insert((file.root, file.path.to_lowercase()))
        {
            return Err(invalid_backup());
        }
        size = size.checked_add(file.size).ok_or_else(invalid_backup)?;
        if size > MAX_CONTENT {
            return Err(Message::new("cloud.too_large"));
        }
    }
    if size != snapshot.size {
        return Err(invalid_backup());
    }
    Ok(())
}

fn read_bounded(response: Response, maximum: u64) -> Result<Vec<u8>, Message> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum)
    {
        return Err(Message::new("cloud.too_large"));
    }
    let mut bytes = Vec::new();
    response
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Message::new("cloud.network"))?;
    if bytes.len() as u64 > maximum {
        return Err(Message::new("cloud.too_large"));
    }
    Ok(bytes)
}

fn parse_listing(bytes: &[u8], directory: &Url) -> Result<BTreeSet<String>, Message> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_backup())?;
    let document = roxmltree::Document::parse(text).map_err(|_| invalid_backup())?;
    if !document
        .root_element()
        .has_tag_name(("DAV:", "multistatus"))
    {
        return Err(invalid_backup());
    }
    let mut ids = BTreeSet::new();
    let mut found_collection = false;
    let directory_path = canonical_url_path(directory.path());
    for response in document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name(("DAV:", "response")))
    {
        let successful_properties: Vec<_> = response
            .children()
            .filter(|node| node.has_tag_name(("DAV:", "propstat")))
            .filter(|propstat| {
                propstat.children().any(|node| {
                    node.has_tag_name(("DAV:", "status"))
                        && node
                            .text()
                            .is_some_and(|text| text.split_whitespace().nth(1) == Some("200"))
                })
            })
            .collect();
        let href = response
            .children()
            .find(|node| node.has_tag_name(("DAV:", "href")))
            .and_then(|node| node.text())
            .ok_or_else(invalid_backup)?;
        let url = directory.join(href).map_err(|_| invalid_backup())?;
        if url.origin() != directory.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid_backup());
        }
        let path = canonical_url_path(url.path());
        if path.trim_end_matches('/') == directory_path.trim_end_matches('/') {
            if successful_properties.iter().any(|propstat| {
                propstat
                    .descendants()
                    .any(|node| node.has_tag_name(("DAV:", "collection")))
            }) {
                found_collection = true;
            } else {
                return Err(invalid_backup());
            }
            continue;
        }
        let name = path
            .strip_prefix(&directory_path)
            .ok_or_else(invalid_backup)?;
        let name = name.trim_end_matches('/');
        if name.is_empty() || name.contains('/') {
            return Err(invalid_backup());
        }
        if successful_properties.is_empty()
            || successful_properties.iter().any(|propstat| {
                propstat
                    .descendants()
                    .any(|node| node.has_tag_name(("DAV:", "collection")))
            })
        {
            continue;
        }
        if let Some(id) = name.strip_suffix(".json") {
            validate_id(id)?;
            ids.insert(id.to_owned());
        }
    }
    if !found_collection {
        return Err(invalid_backup());
    }
    Ok(ids)
}

fn canonical_url_path(path: &str) -> String {
    let mut bytes = path.as_bytes().to_vec();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'%'
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            bytes[index + 1].make_ascii_uppercase();
            bytes[index + 2].make_ascii_uppercase();
            index += 3;
        } else {
            index += 1;
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| path.to_owned())
}

fn unique_id(path: &Path) -> Result<String, Message> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(local_error)?;
    Ok(format!(
        "{:020}-{:x}",
        timestamp.as_nanos(),
        Sha256::digest(path.to_string_lossy().as_bytes())
    ))
}

fn hash_reader(file: &mut fs::File) -> Result<String, Message> {
    file.seek(SeekFrom::Start(0)).map_err(local_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let length = file.read(&mut buffer).map_err(local_error)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn regular_source(root: &Path, relative: &str) -> Result<PathBuf, Message> {
    let mut path = root.to_owned();
    for component in std::iter::once("").chain(relative.split('/')) {
        if !component.is_empty() {
            path.push(component);
        }
        let metadata = fs::symlink_metadata(&path).map_err(local_error)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_backup());
        }
    }
    if !fs::metadata(&path).map_err(local_error)?.is_file() {
        return Err(invalid_backup());
    }
    Ok(path)
}

fn build_archive(source: &Path, snapshot: &Snapshot) -> Result<tempfile::NamedTempFile, Message> {
    validate_snapshot(snapshot)?;
    let mut temporary = tempfile::NamedTempFile::new().map_err(local_error)?;
    let monitor = temporary.reopen().map_err(local_error)?;
    {
        let mut writer = ZipWriter::new(temporary.as_file_mut());
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o600);
        let mut buffer = [0u8; 65_536];
        for file in &snapshot.files {
            let name = format!("{}/{}", file.root, file.path);
            let source = regular_source(source, &name)?;
            if fs::metadata(&source).map_err(local_error)?.len() != file.size {
                return Err(invalid_backup());
            }
            let mut input = fs::File::open(source)
                .map_err(local_error)?
                .take(file.size + 1);
            writer.start_file(name, options).map_err(local_error)?;
            let mut digest = Sha256::new();
            let mut size = 0;
            loop {
                let length = input.read(&mut buffer).map_err(local_error)?;
                if length == 0 {
                    break;
                }
                size += length as u64;
                if size > file.size {
                    return Err(invalid_backup());
                }
                digest.update(&buffer[..length]);
                writer.write_all(&buffer[..length]).map_err(local_error)?;
                if monitor.metadata().map_err(local_error)?.len() > MAX_ARCHIVE {
                    return Err(Message::new("cloud.too_large"));
                }
            }
            if size != file.size || format!("{:x}", digest.finalize()) != file.hash {
                return Err(invalid_backup());
            }
        }
        writer.finish().map_err(local_error)?;
    }
    if temporary.as_file().metadata().map_err(local_error)?.len() > MAX_ARCHIVE {
        return Err(Message::new("cloud.too_large"));
    }
    Ok(temporary)
}

fn extract_archive(
    input: &mut fs::File,
    snapshot: &Snapshot,
    destination: &Path,
) -> Result<(), Message> {
    validate_snapshot(snapshot)?;
    preflight_archive(input, snapshot.files.len())?;
    let mut archive = ZipArchive::new(input).map_err(|_| invalid_backup())?;
    if archive.len() != snapshot.files.len() {
        return Err(invalid_backup());
    }
    let expected: BTreeMap<_, _> = snapshot
        .files
        .iter()
        .map(|file| (format!("{}/{}", file.root, file.path), file))
        .collect();
    let mut seen = BTreeSet::new();
    for root in 0..snapshot.roots.len() {
        fs::create_dir(destination.join(root.to_string())).map_err(local_error)?;
    }
    let mut buffer = [0u8; 65_536];
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| invalid_backup())?;
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| invalid_backup())?
            .to_owned();
        if !entry.is_file()
            || entry.is_symlink()
            || entry.encrypted()
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000))
            || !seen.insert(name.clone())
        {
            return Err(invalid_backup());
        }
        let expected = expected.get(&name).ok_or_else(invalid_backup)?;
        if entry.size() != expected.size {
            return Err(invalid_backup());
        }
        let target = destination.join(&name);
        fs::create_dir_all(target.parent().ok_or_else(invalid_backup)?).map_err(local_error)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(local_error)?;
        let mut digest = Sha256::new();
        let mut length = 0;
        loop {
            let size = entry.read(&mut buffer).map_err(|_| invalid_backup())?;
            if size == 0 {
                break;
            }
            length += size as u64;
            if length > expected.size {
                return Err(invalid_backup());
            }
            digest.update(&buffer[..size]);
            output.write_all(&buffer[..size]).map_err(local_error)?;
        }
        if length != expected.size || format!("{:x}", digest.finalize()) != expected.hash {
            return Err(invalid_backup());
        }
        output.sync_all().map_err(local_error)?;
    }
    Ok(())
}

fn preflight_archive(input: &mut fs::File, expected_files: usize) -> Result<(), Message> {
    // Our bounded exports use ZIP32 and no archive comment. Check the directory
    // count before zip allocates its entry table; zip otherwise collapses duplicates.
    let size = input.metadata().map_err(local_error)?.len();
    if !(22..=MAX_ARCHIVE).contains(&size) {
        return Err(invalid_backup());
    }
    input.seek(SeekFrom::End(-22)).map_err(local_error)?;
    let mut footer = [0u8; 22];
    input
        .read_exact(&mut footer)
        .map_err(|_| invalid_backup())?;
    let count = u16::from_le_bytes([footer[10], footer[11]]) as usize;
    let disk_count = u16::from_le_bytes([footer[8], footer[9]]) as usize;
    let directory_size =
        u32::from_le_bytes([footer[12], footer[13], footer[14], footer[15]]) as u64;
    let directory_start =
        u32::from_le_bytes([footer[16], footer[17], footer[18], footer[19]]) as u64;
    if &footer[..4] != b"PK\x05\x06"
        || footer[4..8] != [0; 4]
        || footer[20..22] != [0; 2]
        || count != expected_files
        || disk_count != expected_files
        || directory_size > 32 * 1024 * 1024
        || directory_start.checked_add(directory_size) != Some(size - 22)
    {
        return Err(invalid_backup());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::display_path, runtime::Status};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::{
        net::{TcpListener, TcpStream},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
    };

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[derive(Default)]
    struct DavState {
        files: BTreeMap<String, Vec<u8>>,
        directories: BTreeSet<String>,
        requests: Vec<(String, String, BTreeMap<String, String>)>,
        fail_metadata: bool,
        forced_status: Option<u16>,
        redirect: String,
    }

    struct MockDav {
        address: String,
        state: Arc<Mutex<DavState>>,
        stopped: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl MockDav {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let state = Arc::new(Mutex::new(DavState::default()));
            state.lock().unwrap().directories.insert("/dav/".into());
            let stopped = Arc::new(AtomicBool::new(false));
            let shared = state.clone();
            let stop = stopped.clone();
            let worker = thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut stream = stream.unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut buffer = [0u8; 8192];
                    let split = loop {
                        let length = stream.read(&mut buffer).unwrap();
                        assert!(length > 0, "request ended before headers");
                        bytes.extend_from_slice(&buffer[..length]);
                        if let Some(position) =
                            bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            break position + 4;
                        }
                    };
                    let header = std::str::from_utf8(&bytes[..split]).unwrap();
                    let mut lines = header.split("\r\n");
                    let mut first = lines.next().unwrap().split_whitespace();
                    let method = first.next().unwrap().to_owned();
                    let path = first.next().unwrap().to_owned();
                    let headers: BTreeMap<_, _> = lines
                        .filter_map(|line| line.split_once(':'))
                        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                        .collect();
                    let length = headers
                        .get("content-length")
                        .map(|text| text.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    while bytes.len() - split < length {
                        let count = stream.read(&mut buffer).unwrap();
                        assert!(count > 0, "request ended before body");
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    let body = bytes[split..split + length].to_vec();
                    let mut state = shared.lock().unwrap();
                    state
                        .requests
                        .push((method.clone(), path.clone(), headers.clone()));
                    let auth = format!("Basic {}", STANDARD.encode("account:密码"));
                    let mut response_body = Vec::new();
                    let mut location = String::new();
                    let status = if let Some(status) = state.forced_status {
                        location.clone_from(&state.redirect);
                        status
                    } else if headers.get("authorization") != Some(&auth) {
                        401
                    } else {
                        match method.as_str() {
                            "MKCOL" => {
                                if state.directories.insert(path.clone()) {
                                    201
                                } else {
                                    405
                                }
                            }
                            "PROPFIND" => {
                                if state.directories.contains(&path) {
                                    let mut entries = String::new();
                                    if headers.get("depth").is_some_and(|depth| depth == "1") {
                                        for file in state.files.keys().filter(|file| {
                                            file.strip_prefix(&path)
                                                .is_some_and(|tail| !tail.contains('/'))
                                        }) {
                                            entries.push_str(&response_xml(file, false));
                                        }
                                    }
                                    response_body = format!("<d:multistatus xmlns:d=\"DAV:\">{}{entries}</d:multistatus>", response_xml(&path, true)).into_bytes();
                                    207
                                } else {
                                    404
                                }
                            }
                            "PUT" => {
                                if state.fail_metadata && path.ends_with(".json") {
                                    500
                                } else if state.files.contains_key(&path)
                                    && headers
                                        .get("if-none-match")
                                        .is_some_and(|value| value == "*")
                                {
                                    412
                                } else {
                                    state.files.insert(path.clone(), body);
                                    201
                                }
                            }
                            "GET" => {
                                if let Some(body) = state.files.get(&path) {
                                    response_body.clone_from(body);
                                    200
                                } else {
                                    404
                                }
                            }
                            "DELETE" => {
                                state.files.remove(&path);
                                204
                            }
                            _ => 405,
                        }
                    };
                    drop(state);
                    let redirect = if location.is_empty() {
                        String::new()
                    } else {
                        format!("Location: {location}\r\n")
                    };
                    write!(stream, "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n{redirect}\r\n", response_body.len()).unwrap();
                    stream.write_all(&response_body).unwrap();
                }
            });
            Self {
                address,
                state,
                stopped,
                worker: Some(worker),
            }
        }

        fn client(&self) -> CloudClient {
            CloudClient::new(
                &CloudSettings {
                    url: format!("http://{}/dav/", self.address),
                    username: "account".into(),
                    remote_path: "云 存档".into(),
                },
                "密码",
            )
            .unwrap()
        }
    }

    impl Drop for MockDav {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(&self.address);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    fn response_xml(href: &str, collection: bool) -> String {
        let resource = if collection { "<d:collection/>" } else { "" };
        format!(
            "<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:resourcetype>{resource}</d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
        )
    }

    fn local_snapshot(directory: &Path) -> (PathBuf, PathBuf, Snapshot) {
        let saves = directory.join("saves");
        fs::create_dir_all(saves.join("sub")).unwrap();
        fs::write(saves.join("sub/slot.sav"), b"original save").unwrap();
        let data = directory.join("backups");
        let snapshot = snapshots::create_named_checked(
            &data,
            ("game", "游戏"),
            &[display_path(&saves)],
            "note",
            20,
            || Status::Stopped,
        )
        .unwrap();
        (data, saves, snapshot)
    }

    #[test]
    fn settings_reject_credential_urls_and_unsafe_collection_paths() {
        for url in [
            "ftp://example.com",
            "https://user:password@example.com/dav",
            "https://example.com/dav?password=secret",
            "https://example.com/#fragment",
        ] {
            let settings = CloudSettings {
                url: url.into(),
                username: "user".into(),
                remote_path: "GalGear".into(),
            };
            assert_eq!(
                settings.validate().unwrap_err().key,
                "cloud.invalid_settings"
            );
        }
        for path in [
            "../GalGear",
            "GalGear/../other",
            "GalGear\\other",
            "GalGear//other",
        ] {
            let settings = CloudSettings {
                url: "https://example.com/dav".into(),
                username: "user".into(),
                remote_path: path.into(),
            };
            assert!(settings.validate().is_err(), "{path}");
        }
        assert!(validate_password("密码").is_ok());
        assert_eq!(
            validate_password("secret\r\n").unwrap_err().key,
            "cloud.invalid_password"
        );
    }

    #[test]
    fn propfind_accepts_namespace_variants_and_rejects_untrusted_hrefs() {
        let directory = Url::parse("https://example.com/dav/%E4%BA%91/abc/").unwrap();
        let self_response = response_xml(directory.path(), true);
        let child = response_xml("https://example.com/dav/%E4%BA%91/abc/backup-1.json", false);
        let xml = format!("<d:multistatus xmlns:d=\"DAV:\">{self_response}{child}</d:multistatus>");
        for xml in [
            xml.clone(),
            xml.replace("xmlns:d=", "xmlns=").replace("d:", ""),
        ] {
            assert!(
                parse_listing(xml.as_bytes(), &directory)
                    .unwrap()
                    .contains("backup-1")
            );
        }
        let lowercase_hex = xml.replace("%E4%BA%91", "%e4%ba%91");
        assert!(
            parse_listing(lowercase_hex.as_bytes(), &directory)
                .unwrap()
                .contains("backup-1")
        );
        for href in [
            "https://attacker.example/backup-1.json",
            "../backup-1.json",
            "nested/backup-1.json",
            "backup-1.json?token=secret",
        ] {
            let child = response_xml(href, false);
            let xml =
                format!("<d:multistatus xmlns:d=\"DAV:\">{self_response}{child}</d:multistatus>");
            assert_eq!(
                parse_listing(xml.as_bytes(), &directory).unwrap_err().key,
                "cloud.invalid_backup"
            );
        }
    }

    #[test]
    fn webdav_round_trip_imports_history_without_touching_live_saves() {
        let remote = MockDav::new();
        let client = remote.client();
        client.test_connection().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let (data, saves, mut snapshot) = local_snapshot(temp.path());
        // Local Windows manifests contain backslashes even for nested save files.
        snapshot.files[0].path = "sub\\slot.sav".into();
        client.upload(&data, &snapshot, KEY).unwrap();
        assert_eq!(client.list(KEY).unwrap().len(), 1);
        fs::write(saves.join("sub/slot.sav"), b"current save").unwrap();
        let imported = client.download(&data, "game", KEY, &snapshot.id).unwrap();
        assert_ne!(
            imported.id, snapshot.id,
            "same remote snapshot must not replace local history"
        );
        assert!(imported.sequence > snapshot.sequence);
        assert_eq!(
            fs::read(saves.join("sub/slot.sav")).unwrap(),
            b"current save"
        );
        assert_eq!(snapshots::list(&data, "game").unwrap().len(), 2);
        let roots = vec![display_path(&saves)];
        let plan =
            snapshots::preview(&data, "game", &imported.id, &roots, Status::Stopped).unwrap();
        assert_eq!(
            plan.changes.len(),
            1,
            "imported paths must use local fingerprint separators"
        );
        snapshots::restore(&data, &plan, 20, Status::Stopped).unwrap();
        assert_eq!(
            fs::read(saves.join("sub/slot.sav")).unwrap(),
            b"original save"
        );
        let state = remote.state.lock().unwrap();
        let uploads: Vec<_> = state
            .requests
            .iter()
            .filter(|(method, _, _)| method == "PUT")
            .collect();
        assert!(uploads[uploads.len() - 2].1.ends_with(".zip"));
        assert!(uploads[uploads.len() - 1].1.ends_with(".json"));
        let manifest = state
            .files
            .iter()
            .find(|(name, _)| name.ends_with(".json"))
            .unwrap()
            .1;
        assert!(!String::from_utf8_lossy(manifest).contains(&display_path(&saves)));
        assert!(
            state
                .requests
                .iter()
                .any(|(_, path, _)| path.contains("%20"))
        );
    }

    #[test]
    fn failed_metadata_publish_removes_unlisted_archive() {
        let remote = MockDav::new();
        remote.state.lock().unwrap().fail_metadata = true;
        let temp = tempfile::tempdir().unwrap();
        let (data, _, snapshot) = local_snapshot(temp.path());
        let client = remote.client();
        assert_eq!(
            client.upload(&data, &snapshot, KEY).unwrap_err().key,
            "cloud.http"
        );
        assert!(remote.state.lock().unwrap().files.is_empty());
        assert!(client.list(KEY).unwrap().is_empty());
    }

    #[test]
    fn existing_cloud_snapshot_is_not_overwritten() {
        let remote = MockDav::new();
        let temp = tempfile::tempdir().unwrap();
        let (data, _, snapshot) = local_snapshot(temp.path());
        let client = remote.client();
        client.upload(&data, &snapshot, KEY).unwrap();
        let previous = remote.state.lock().unwrap().files.clone();
        assert_eq!(
            client.upload(&data, &snapshot, KEY).unwrap_err().key,
            "cloud.conflict"
        );
        assert_eq!(remote.state.lock().unwrap().files, previous);
    }

    #[test]
    fn corrupted_download_cannot_publish_history_or_change_saves() {
        let remote = MockDav::new();
        let temp = tempfile::tempdir().unwrap();
        let (data, saves, snapshot) = local_snapshot(temp.path());
        let client = remote.client();
        client.upload(&data, &snapshot, KEY).unwrap();
        remote
            .state
            .lock()
            .unwrap()
            .files
            .iter_mut()
            .find(|(name, _)| name.ends_with(".zip"))
            .unwrap()
            .1[0] ^= 0xff;
        assert_eq!(
            client
                .download(&data, "game", KEY, &snapshot.id)
                .unwrap_err()
                .key,
            "cloud.invalid_backup"
        );
        assert_eq!(snapshots::list(&data, "game").unwrap().len(), 1);
        assert_eq!(
            fs::read(saves.join("sub/slot.sav")).unwrap(),
            b"original save"
        );
    }

    #[test]
    fn authentication_errors_and_redirects_never_expose_or_forward_credentials() {
        let remote = MockDav::new();
        let destination = MockDav::new();
        let client = remote.client();
        for (status, key) in [
            (401, "cloud.auth"),
            (403, "cloud.auth"),
            (302, "cloud.http"),
        ] {
            {
                let mut state = remote.state.lock().unwrap();
                state.forced_status = Some(status);
                state.redirect = format!("http://{}/dav/", destination.address);
            }
            let error = client.list(KEY).unwrap_err();
            assert_eq!(error.key, key);
            let serialized = serde_json::to_string(&error).unwrap();
            assert!(!serialized.contains("密码") && !serialized.contains("account"));
        }
        assert!(destination.state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn metadata_rejects_traversal_root_errors_size_errors_and_windows_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let (_, _, original) = local_snapshot(temp.path());
        for path in [
            "../escape",
            "/absolute",
            "sub\\slot.sav",
            "slot.sav:stream",
            "CON.sav",
            "save./slot.sav",
            "sub//slot.sav",
        ] {
            let mut snapshot = original.clone();
            snapshot.files[0].path = path.into();
            assert!(validate_snapshot(&snapshot).is_err(), "{path}");
        }
        let mut snapshot = original.clone();
        snapshot.files[0].root = 1;
        assert!(validate_snapshot(&snapshot).is_err());
        let mut snapshot = original.clone();
        snapshot.size += 1;
        assert!(validate_snapshot(&snapshot).is_err());
        let mut snapshot = original;
        snapshot.files[0].path = "slot.sav".into();
        let mut alias = snapshot.files[0].clone();
        alias.path = "SLOT.SAV".into();
        snapshot.files.push(alias);
        snapshot.size *= 2;
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn zip_import_rejects_traversal_symlinks_and_content_hash_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let (_, _, mut snapshot) = local_snapshot(temp.path());
        snapshot.files[0].path = "sub/slot.sav".into();
        for kind in ["traversal", "symlink", "corrupt"] {
            let mut archive = tempfile::NamedTempFile::new().unwrap();
            {
                let mut writer = ZipWriter::new(archive.as_file_mut());
                let options = SimpleFileOptions::default();
                match kind {
                    "traversal" => {
                        writer.start_file("../escape", options).unwrap();
                        writer.write_all(b"original save").unwrap();
                    }
                    "symlink" => {
                        writer
                            .add_symlink("0/sub/slot.sav", "../../../live", options)
                            .unwrap();
                    }
                    _ => {
                        writer.start_file("0/sub/slot.sav", options).unwrap();
                        writer.write_all(b"corrupt! save").unwrap();
                    }
                }
                writer.finish().unwrap();
            }
            let extracted = tempfile::tempdir().unwrap();
            assert_eq!(
                extract_archive(archive.as_file_mut(), &snapshot, extracted.path())
                    .unwrap_err()
                    .key,
                "cloud.invalid_backup",
                "{kind}"
            );
            assert!(!extracted.path().parent().unwrap().join("escape").exists());
        }
    }

    #[test]
    fn zip_preflight_bounds_directory_and_detects_duplicate_central_entries() {
        let temp = tempfile::tempdir().unwrap();
        let (data, _, mut snapshot) = local_snapshot(temp.path());
        snapshot.files[0].path = "sub/slot.sav".into();
        let source = snapshots::content_directory(&data, "game", &snapshot.id).unwrap();
        let archive = build_archive(&source, &snapshot).unwrap();
        let bytes = fs::read(archive.path()).unwrap();
        let position = bytes.len() - 22;
        let mut footer: Vec<_> = bytes[position..].to_vec();
        let directory_size = u32::from_le_bytes(footer[12..16].try_into().unwrap()) as usize;
        let directory_start = u32::from_le_bytes(footer[16..20].try_into().unwrap()) as usize;

        footer[12..16].copy_from_slice(&(33u32 * 1024 * 1024).to_le_bytes());
        let mut oversized = tempfile::NamedTempFile::new().unwrap();
        oversized.write_all(&bytes[..position]).unwrap();
        oversized.write_all(&footer).unwrap();
        assert!(preflight_archive(oversized.as_file_mut(), 1).is_err());

        let mut footer = bytes[position..].to_vec();
        footer[8..10].copy_from_slice(&2u16.to_le_bytes());
        footer[10..12].copy_from_slice(&2u16.to_le_bytes());
        footer[12..16].copy_from_slice(&((directory_size * 2) as u32).to_le_bytes());
        let mut duplicate = tempfile::NamedTempFile::new().unwrap();
        duplicate.write_all(&bytes[..position]).unwrap();
        duplicate
            .write_all(&bytes[directory_start..position])
            .unwrap();
        duplicate.write_all(&footer).unwrap();
        let mut other = snapshot.files[0].clone();
        other.path = "other.sav".into();
        snapshot.files.push(other);
        snapshot.size *= 2;
        let extracted = tempfile::tempdir().unwrap();
        assert_eq!(
            extract_archive(duplicate.as_file_mut(), &snapshot, extracted.path())
                .unwrap_err()
                .key,
            "cloud.invalid_backup"
        );
    }
}
