//! 授权前同步落盘，崩溃后只撤销本任务 SID。独占句柄兼作跨实例的存活标记。
//! 最后一行可能被强制终止截断；只有完整且已同步的记录才允许触发 ACL 修改。
use super::{
    access::{Grant, Sid},
    checked_path,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DISPOSITION_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FileDispositionInfo, SetFileInformationByHandle,
};

const PREFIX: &str = "GalGear.Analysis.";
const EXTENSION: &str = ".jsonl";
const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;

pub(super) struct Journal {
    file: File,
    paths: Vec<PathBuf>,
    removed: bool,
}

pub(super) fn directory() -> io::Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Missing LocalAppData"))?;
    Ok(PathBuf::from(local).join("GalGear/ai-sandbox/v1"))
}

fn open(path: &Path, create: bool) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(create)
        .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn valid_name(name: &str) -> bool {
    name.strip_prefix(PREFIX).is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

impl Journal {
    pub(super) fn create(directory: &Path, name: &str, work: &Path) -> io::Result<Self> {
        if !valid_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid sandbox identity",
            ));
        }
        // 在创建前检查已有祖先，避免通过目录联接把记录放入其他位置。
        let existing = directory
            .ancestors()
            .find(|path| path.exists())
            .ok_or_else(|| io::Error::other("Missing journal ancestor"))?;
        checked_path(existing)?;
        fs::create_dir_all(directory)?;
        let directory = checked_path(directory)?;
        if directory.starts_with(work) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Sandbox work contains recovery records",
            ));
        }
        let file = open(&directory.join(format!("{name}{EXTENSION}")), true)?;
        // 身份名保存在文件名中，空文件也能恢复 profile 创建期间发生的崩溃。
        file.sync_all()?;
        Ok(Self {
            file,
            paths: Vec::new(),
            removed: false,
        })
    }

    pub(super) fn record(&mut self, path: &Path) -> io::Result<()> {
        if self.paths.iter().any(|saved| saved == path) {
            return Ok(());
        }
        let mut bytes = serde_json::to_vec(path)?;
        bytes.push(b'\n');
        if self.file.metadata()?.len() + bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(io::Error::other("Sandbox recovery record is too large"));
        }
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        self.paths.push(path.to_path_buf());
        Ok(())
    }

    pub(super) fn revoke(&mut self, sid: &mut Sid) -> io::Result<()> {
        if self.removed {
            return Ok(());
        }
        let mut failure = None;
        for path in self.paths.iter().rev() {
            if let Err(error) = Grant::revoke_path(path, sid) {
                failure = Some(error);
            }
        }
        if let Err(error) = sid.remove_profile() {
            failure = Some(error);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        // 在仍持有独占句柄时标记删除，避免关闭与按路径删除之间被另一实例打开。
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: file owns an open handle with DELETE access; structure and size match the class.
        if unsafe {
            SetFileInformationByHandle(
                self.file.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        self.removed = true;
        Ok(())
    }
}

pub(super) fn recover(directory: &Path) -> Vec<io::Error> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => return vec![error],
    };
    if let Err(error) = checked_path(directory) {
        return vec![error];
    }
    let mut failures = Vec::new();
    for entry in entries {
        let result = entry.map_err(io::Error::other).and_then(|entry| {
            let file_name = entry.file_name();
            let Some(name) = file_name
                .to_str()
                .and_then(|name| name.strip_suffix(EXTENSION))
                .filter(|name| valid_name(name))
            else {
                return Ok(());
            };
            recover_one(&entry.path(), name)
        });
        if let Err(error) = result {
            failures.push(error);
        }
    }
    failures
}

fn recover_one(path: &Path, name: &str) -> io::Result<()> {
    let mut file = match open(path, false) {
        Ok(file) => file,
        // 活跃任务或另一恢复线程持有独占句柄；不按 PID 猜测，避免 PID 重用误清理。
        Err(error)
            if error.raw_os_error() == Some(32) || error.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() > MAX_RECORD_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid sandbox recovery file",
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut paths = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if line.last() != Some(&b'\n') {
            break;
        }
        let path: PathBuf = serde_json::from_slice(line)?;
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Relative sandbox recovery path",
            ));
        }
        paths.push(path);
    }
    let mut journal = Journal {
        file,
        paths,
        removed: false,
    };
    let mut sid = Sid::existing(name)?;
    journal.revoke(&mut sid)
}

#[cfg(test)]
mod tests;
