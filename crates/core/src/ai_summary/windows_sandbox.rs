//! 每次分析使用独立的 LPAC 身份；输入 ACL 只读，唯一可写目录为本次工作目录。
//! 系统运行库由 Windows 的 LPAC 权限提供。任务结束先回收进程树，再撤销此身份的授权。
mod access;
mod child;
mod journal;
mod network;

use access::{Grant, Sid};
pub(super) use child::Child;
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use windows_sys::Win32::Security::{SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES};

pub(super) struct Sandbox {
    sid: Sid,
    capabilities: Vec<Sid>,
    journal: journal::Journal,
    network: Option<network::Broker>,
}

impl Sandbox {
    pub(super) fn new(
        command: &mut Command,
        work: &Path,
        config: &Value,
        parse: bool,
    ) -> io::Result<Self> {
        Self::new_with_journal(command, work, config, parse, &journal::directory()?)
    }

    fn new_with_journal(
        command: &mut Command,
        work: &Path,
        config: &Value,
        parse: bool,
        directory: &Path,
    ) -> io::Result<Self> {
        let work = checked_path(work)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
        let name = format!("GalGear.Analysis.{:032x}", u128::from_le_bytes(random));
        let journal = journal::Journal::create(directory, &name, &work)?;
        let mut sandbox = Self {
            sid: Sid::package(&name)?,
            capabilities: Vec::new(),
            journal,
            network: None,
        };
        Grant::deny_profile_write(&sandbox.sid)?;
        // LPAC 系统能力支持 TLS、.NET 和 WinRT 读取器；不授予用户文件能力或注册表写入。
        for capability in [
            "registryRead",
            "lpacCryptoServices",
            "lpacCom",
            "lpacInstrumentation",
        ] {
            sandbox.capabilities.push(Sid::capability(capability)?);
        }
        if !parse && config["mode"] != "media_batch" {
            let broker = network::Broker::new(&name)?;
            sandbox.network = Some(broker);
            sandbox
                .capabilities
                .push(Sid::capability("internetClient")?);
            sandbox
                .capabilities
                .push(Sid::capability("privateNetworkClientServer")?);
        }
        sandbox.allow(&work, true)?;
        if parse && let Some(skill) = config["skill"].as_str() {
            sandbox.allow(Path::new(skill), false)?;
        }
        if !parse {
            for key in ["game", "skill"] {
                let path = config[key].as_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Missing AI sandbox input root")
                })?;
                sandbox.allow(Path::new(path), false)?;
            }
            let skill = Path::new(config["skill"].as_str().unwrap_or_default());
            if let Some(parent) = skill.parent() {
                for name in ["GalgameReverse", "GalgameResourcePack"] {
                    let path = parent.join(name);
                    if path.join("SKILL.md").is_file() {
                        sandbox.allow(&path, false)?;
                    }
                }
            }
            // 存档集合可能位于共享 AppData 目录；只给选中文件权限，避免开放相邻游戏。
            let candidates = config["save_candidates"].as_array();
            if let Some(candidates) = candidates.filter(|items| !items.is_empty()) {
                for candidate in candidates {
                    let path = candidate["path"].as_str().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "Invalid save candidate")
                    })?;
                    sandbox.allow(Path::new(path), false)?;
                    if let Some(parent) = Path::new(path).parent() {
                        sandbox.list_directory(parent)?;
                    }
                }
            } else {
                let path = config["save"].as_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Missing save directory")
                })?;
                sandbox.allow(Path::new(path), false)?;
            }
        }
        let executable = executable(command.get_program())?;
        // 发布版使用预展开运行库，所有 DLL 在启动前已具备只读 ACL。
        sandbox.allow(&executable, false)?;
        if let Some(directory) = executable.parent() {
            let runtime = directory.join("_internal");
            if runtime.is_dir() {
                sandbox.allow(&runtime, false)?;
            }
        }
        if executable.file_stem().is_some_and(|name| {
            name.eq_ignore_ascii_case("python") || name.eq_ignore_ascii_case("python3")
        }) {
            let runtime = executable
                .parent()
                .ok_or_else(|| io::Error::other("Missing Python runtime directory"))?;
            for name in ["DLLs", "Library/bin"] {
                let directory = runtime.join(name);
                if directory.is_dir() {
                    sandbox.allow(&directory, false)?;
                }
            }
            let library = runtime.join("Lib");
            if library.is_dir() {
                sandbox.list_directory(&library)?;
                // 开发入口使用 -I -S，只加载标准库；不递归授权整套 Conda 的第三方包。
                for entry in fs::read_dir(&library)? {
                    let entry = entry?;
                    if !matches!(
                        entry.file_name().to_str(),
                        Some("site-packages" | "__pycache__")
                    ) {
                        sandbox.allow(&entry.path(), false)?;
                    }
                }
            }
            for entry in fs::read_dir(runtime)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("python")
                    && (name.ends_with(".dll") || name.ends_with(".zip") || name.ends_with("._pth"))
                {
                    sandbox.allow(&entry.path(), false)?;
                }
            }
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ai_summary");
            sandbox.allow(&source, false)?;
        }
        command.current_dir(&work);
        for name in ["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"] {
            command.env(name, &work);
        }
        command.env("PYTHONDONTWRITEBYTECODE", "1");
        Ok(sandbox)
    }

    fn allow(&mut self, path: &Path, writable: bool) -> io::Result<()> {
        let path = checked_path(path)?;
        self.journal.record(&path)?;
        Grant::allow(path, &self.sid, writable)?;
        Ok(())
    }

    fn list_directory(&mut self, path: &Path) -> io::Result<()> {
        let path = checked_path(path)?;
        self.journal.record(&path)?;
        Grant::list_directory(path, &self.sid)?;
        Ok(())
    }

    fn security(&self, attributes: &mut Vec<SID_AND_ATTRIBUTES>) -> SECURITY_CAPABILITIES {
        attributes.extend(self.capabilities.iter().map(|sid| SID_AND_ATTRIBUTES {
            Sid: sid.as_ptr(),
            Attributes: 4, /* SE_GROUP_ENABLED */
        }));
        SECURITY_CAPABILITIES {
            AppContainerSid: self.sid.as_ptr(),
            Capabilities: attributes.as_mut_ptr(),
            CapabilityCount: attributes.len() as u32,
            Reserved: 0,
        }
    }

    pub(super) fn revoke(&mut self) -> io::Result<()> {
        let mut failure = None;
        if let Some(broker) = &mut self.network
            && let Err(error) = broker.stop()
        {
            failure = Some(error);
        }
        if let Err(error) = self.journal.revoke(&mut self.sid) {
            failure = Some(error);
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // 正常退出显式上报失败；异常路径尽力撤销，失败的记录留给下次启动。
        let _ = self.revoke();
    }
}

pub(super) fn recover() -> Vec<io::Error> {
    match journal::directory() {
        Ok(directory) => journal::recover(&directory),
        Err(error) => vec![error],
    }
}

fn checked_path(path: &Path) -> io::Result<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor)?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "AI sandbox roots cannot contain reparse points",
            ));
        }
    }
    path.canonicalize()
}

fn executable(program: &std::ffi::OsStr) -> io::Result<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return checked_path(path);
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(program).with_extension("exe");
        if candidate.is_file() {
            return checked_path(&candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "AI executable not found",
    ))
}

#[cfg(test)]
mod tests;
