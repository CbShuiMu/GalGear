//! 共享文件传输：进度、取消、体积检查及不覆盖已有文件的原子落盘。
use galgear_i18n::Message;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// 本次传输的文件信息；URL 可能带签名，不应记录或持久化。
pub struct FileInfo<'a> {
    pub url: &'a Url,
    pub referer: &'a str,
    pub title: &'a str,
    pub filename: Option<&'a str>,
    pub bytes: Option<u64>,
}

/// 下载公开/签名直链，不携带任何数据源凭据。
pub fn download(
    file: &FileInfo<'_>,
    directory: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Transfer),
) -> Result<String, Message> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Message::new("download.cancelled"));
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent("GalGear/0.1")
        .timeout(Duration::from_secs(1800))
        .connect_timeout(Duration::from_secs(12))
        .https_only(true)
        .build()
        .map_err(|_| Message::new("download.network"))?;
    let response = client
        .get(file.url.clone())
        .header(reqwest::header::REFERER, file.referer)
        .send()
        .map_err(|_| Message::new("download.network"))?;
    receive(response, file, directory, cancel, progress)
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct Transfer {
    pub phase: String,
    pub title: String,
    pub received: u64,
    pub total: Option<u64>,
    pub path: Option<String>,
    pub error: Option<Message>,
    #[serde(default)]
    pub archive_password: Option<String>,
}
pub(crate) fn receive(
    mut response: reqwest::blocking::Response,
    file_info: &FileInfo<'_>,
    directory: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Transfer),
) -> Result<String, Message> {
    if !response.status().is_success() {
        return Err(Message::new("download.network"));
    }
    let url = response.url().clone();
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/html"))
    {
        return Err(Message::new("download.not_file"));
    }
    let total = file_info.bytes.or(response.content_length());
    if file_info
        .bytes
        .zip(response.content_length())
        .is_some_and(|(expected, actual)| expected != actual)
    {
        return Err(Message::new("download.incomplete"));
    }
    if total.is_some_and(|n| n > 8 * 1024 * 1024 * 1024) {
        return Err(Message::new("download.too_large"));
    }
    std::fs::create_dir_all(directory).map_err(|_| Message::new("download.write"))?;
    let mut file =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| Message::new("download.write"))?;
    let mut received = 0;
    let mut buf = [0u8; 64 * 1024];
    let mut last = std::time::Instant::now() - Duration::from_secs(1);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Message::new("download.cancelled"));
        }
        let n = response
            .read(&mut buf)
            .map_err(|_| Message::new("download.network"))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > 8 * 1024 * 1024 * 1024 {
            return Err(Message::new("download.too_large"));
        }
        file.write_all(&buf[..n])
            .map_err(|_| Message::new("download.write"))?;
        if last.elapsed() > Duration::from_millis(180) {
            progress(Transfer {
                phase: "downloading".into(),
                title: file_info.title.to_owned(),
                received,
                total,
                ..Default::default()
            });
            last = std::time::Instant::now();
        }
    }
    if received == 0 || total.is_some_and(|n| n != received) {
        return Err(Message::new("download.incomplete"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Message::new("download.cancelled"));
    }
    file.as_file()
        .sync_all()
        .map_err(|_| Message::new("download.write"))?;
    let extension = file_info
        .filename
        .unwrap_or(url.path())
        .rsplit('.')
        .next()
        .and_then(|s| {
            ["zip", "rar", "7z", "exe"]
                .into_iter()
                .find(|ext| ext.eq_ignore_ascii_case(s))
        })
        .unwrap_or("bin");
    let name: String = file_info
        .title
        .chars()
        .filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control())
        .take(100)
        .collect();
    let name = name.trim_matches([' ', '.']);
    let name = if name.is_empty() {
        "download".to_owned()
    } else {
        let root = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&root.as_str())
        {
            format!("_{name}")
        } else {
            name.to_owned()
        }
    };
    let mut path = directory.join(format!("{name}.{extension}"));
    for index in 1..1000 {
        if !path.exists() {
            break;
        }
        path = directory.join(format!("{name} ({index}).{extension}"));
    }
    file.persist_noclobber(&path)
        .map_err(|_| Message::new("download.write"))?;
    let path = crate::config::display_path(&path);
    progress(Transfer {
        phase: "complete".into(),
        title: file_info.title.to_owned(),
        received,
        total,
        path: Some(path.clone()),
        ..Default::default()
    });
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(status: u16, body: &'static str) -> reqwest::blocking::Response {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/signed", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let count = stream.read(&mut buf).unwrap();
            assert!(count > 0);
            write!(stream, "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let response = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(url)
            .send()
            .unwrap();
        worker.join().unwrap();
        response
    }

    #[test]
    fn signed_download_names_are_safe_and_existing_files_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let url = Url::parse("https://cdn.example/signed?id=7").unwrap();
        let file = FileInfo {
            url: &url,
            referer: "https://www.moyu.moe/resource/7",
            title: "../../CON",
            filename: Some("../patch.ZIP"),
            bytes: Some(7),
        };
        let cancel = AtomicBool::new(false);
        let first = receive(
            response(200, "fixture"),
            &file,
            directory.path(),
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        let second = receive(
            response(200, "fixture"),
            &file,
            directory.path(),
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        assert_ne!(first, second);
        for path in [first, second] {
            let path = Path::new(&path);
            assert_eq!(path.parent().unwrap(), directory.path());
            assert_eq!(path.extension().unwrap(), "zip");
            assert_eq!(std::fs::read(path).unwrap(), b"fixture");
        }
    }

    #[test]
    fn mismatched_size_and_cancellation_do_not_publish_partial_files() {
        let directory = tempfile::tempdir().unwrap();
        let url = Url::parse("https://cdn.example/file.zip").unwrap();
        let mut file = FileInfo {
            url: &url,
            referer: "",
            title: "patch",
            filename: None,
            bytes: Some(8),
        };
        let cancel = AtomicBool::new(false);
        assert_eq!(
            receive(
                response(200, "fixture"),
                &file,
                directory.path(),
                &cancel,
                &mut |_| {}
            )
            .unwrap_err()
            .key,
            "download.incomplete"
        );
        file.bytes = Some(7);
        assert_eq!(
            receive(
                response(200, "fixture"),
                &file,
                directory.path(),
                &cancel,
                &mut |_| {
                    cancel.store(true, Ordering::Relaxed);
                }
            )
            .unwrap_err()
            .key,
            "download.cancelled"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
