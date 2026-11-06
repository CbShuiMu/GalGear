//! 单次字体请求的辅助进程：仅访问被拒绝时请求 UAC；加载和 IPC 共用同一权限。
use super::error;
use galgear_i18n::Message;
use std::{
    ffi::OsStr,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Threading::{CREATE_NO_WINDOW, GetExitCodeProcess, WaitForSingleObject},
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

const ACCESS_DENIED: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(40);

pub(super) fn apply(
    loader: &Path,
    pid: u32,
    created: u64,
    packet: &[u8; galgear_core::font_tools::PACKET_SIZE],
) -> Result<(), Message> {
    let args = arguments(pid, created, packet);
    let code = run(loader, &args)?;
    // 只提升一次明确的字体请求；取消或其他错误均不再启动辅助进程。
    let code = if code == ACCESS_DENIED {
        elevated(loader, &args)?
    } else {
        code
    };
    result(code)
}

fn arguments(
    pid: u32,
    created: u64,
    packet: &[u8; galgear_core::font_tools::PACKET_SIZE],
) -> [String; 3] {
    [
        pid.to_string(),
        format!("{created:016x}"),
        packet.iter().map(|byte| format!("{byte:02x}")).collect(),
    ]
}
pub(super) fn result(code: u32) -> Result<(), Message> {
    match code {
        0 => Ok(()),
        ACCESS_DENIED => Err(Message::new("font.access_denied")),
        8..=10 => Err(Message::new("font.helper_timeout")),
        12 => Err(Message::new("font.busy")),
        13 | 14 => Err(Message::new("font.target_changed")),
        15 => Err(Message::new("font.components_missing")),
        16 => Err(Message::new("font.incompatible")),
        17 => Err(Message::new("font.not_found")),
        18 => Err(Message::new("font.prepare_failed")),
        code => Err(Message::new("font.helper_failed").arg("code", code)),
    }
}
fn run(loader: &Path, args: &[String; 3]) -> Result<u32, Message> {
    let mut child = Command::new(loader)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(error)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(error)? {
            return status
                .code()
                .map(|code| code as u32)
                .ok_or_else(|| error("Font helper exited without a status"));
        }
        if started.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Message::new("font.helper_timeout"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
fn launch_error(error: std::io::Error) -> Message {
    if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
        Message::new("font.uac_cancelled")
    } else {
        super::error(error)
    }
}

fn elevated(loader: &Path, args: &[String; 3]) -> Result<u32, Message> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: 只释放本线程已成功初始化的 COM apartment。
            unsafe {
                CoUninitialize();
            }
        }
    }
    // SAFETY: apply 在独立字体任务线程运行；不会改变 Tauri/面板线程的 apartment。
    let initialized = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if initialized < 0 {
        return Err(error(format!("Font helper COM: {initialized:#x}")));
    }
    let _apartment = Apartment;
    // 与目录打开入口一致，Shell 使用普通 DOS/UNC 路径，不传 canonicalize 的设备前缀。
    let file = wide(OsStr::new(&galgear_core::config::display_path(loader)));
    // 参数仅为 PID、进程创建时间与固定包的十六进制；无字体名/路径的命令行转义歧义。
    let parameters = wide(OsStr::new(&args.join(" ")));
    let directory = wide(OsStr::new(&galgear_core::config::display_path(
        loader
            .parent()
            .ok_or_else(|| error("Font helper directory missing"))?,
    )));
    let mut request = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: windows_sys::w!("runas"),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_HIDE,
        ..Default::default()
    };
    // SAFETY: 字符串在同步 Shell 调用返回前存活；系统负责显示 UAC 并由用户授权。
    if unsafe { ShellExecuteExW(&mut request) } == 0 {
        return Err(launch_error(std::io::Error::last_os_error()));
    }
    if request.hProcess.is_null() {
        return Err(error("Font helper process handle missing"));
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS 将成功创建的进程句柄所有权交给调用方。
    let child = unsafe { OwnedHandle::from_raw_handle(request.hProcess) };
    // SAFETY: 只等待/查询独占持有的辅助进程句柄；等待不会阻塞面板线程。
    unsafe {
        match WaitForSingleObject(child.as_raw_handle(), TIMEOUT.as_millis() as u32) {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if GetExitCodeProcess(child.as_raw_handle(), &mut code) == 0 {
                    return Err(error(std::io::Error::last_os_error()));
                }
                Ok(code)
            }
            WAIT_TIMEOUT => Err(Message::new("font.helper_timeout")),
            _ => Err(error(std::io::Error::last_os_error())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_arguments_preserve_unicode_without_shell_syntax() {
        let profile = galgear_core::font_tools::Profile {
            face: "微软雅黑".into(),
            ..Default::default()
        };
        let bytes = profile.packet().unwrap();
        let args = arguments(42, 0x123456789abcdef0, &bytes);
        assert_eq!(args[0], "42");
        assert_eq!(args[1], "123456789abcdef0");
        assert_eq!(args[2].len(), 256);
        let decoded: Vec<u8> = args[2]
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(decoded, bytes);
        assert!(
            args.iter()
                .all(|arg| arg.bytes().all(|b| b.is_ascii_hexdigit()))
        );
    }
    #[test]
    fn denied_cancelled_and_busy_requests_are_not_success() {
        assert_eq!(result(3).unwrap_err().key, "font.access_denied");
        assert_eq!(result(12).unwrap_err().key, "font.busy");
        assert_eq!(result(13).unwrap_err().key, "font.target_changed");
        assert_eq!(result(16).unwrap_err().key, "font.incompatible");
        assert_eq!(
            launch_error(std::io::Error::from_raw_os_error(ERROR_CANCELLED as i32)).key,
            "font.uac_cancelled"
        );
        assert!(result(0).is_ok());
    }

    #[test]
    #[ignore = "需要构建 x86/x64 字体测试组件；只操作隔离 GDI 进程"]
    fn native_loader_rejects_stale_identity_and_invalid_packet() {
        use std::{
            io::{BufRead, BufReader},
            process::Stdio,
        };
        struct Fixture(std::process::Child);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for arch in ["x86", "x64"] {
            let mut fixture = Fixture(
                Command::new(root.join(format!("target/font-fixtures/{arch}/font-fixture.exe")))
                    .stdout(Stdio::piped())
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .unwrap(),
            );
            let mut ready = String::new();
            BufReader::new(fixture.0.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready.trim(), "READY");
            let loader = root.join(format!("tools/font-hook/{arch}/font-hook-loader.exe"));
            let packet = galgear_core::font_tools::Profile {
                height_percent: 200,
                ..Default::default()
            }
            .packet()
            .unwrap();
            let args = arguments(fixture.0.id(), 1, &packet);
            assert_eq!(run(&loader, &args).unwrap(), 13);
            let mut invalid = packet;
            invalid[0] = 99;
            assert_eq!(
                run(&loader, &arguments(fixture.0.id(), 1, &invalid)).unwrap(),
                2
            );
            assert!(fixture.0.try_wait().unwrap().is_none());

            // An unadapted DLL must be rejected before DllMain can show its UI.
            let isolated = tempfile::tempdir().unwrap();
            let isolated_loader = isolated.path().join("font-hook-loader.exe");
            std::fs::copy(&loader, &isolated_loader).unwrap();
            let system = if arch == "x86" {
                "SysWOW64"
            } else {
                "System32"
            };
            let system_dll = Path::new(&std::env::var("SystemRoot").unwrap())
                .join(system)
                .join("version.dll");
            std::fs::copy(system_dll, isolated.path().join("GalGearFontHook.dll")).unwrap();
            // SAFETY: fixture is owned by the test and stays alive until Fixture drops.
            let created = {
                use std::os::windows::io::AsRawHandle;
                super::super::creation_time(fixture.0.as_raw_handle()).unwrap()
            };
            assert_eq!(
                run(
                    &isolated_loader,
                    &arguments(fixture.0.id(), created, &packet)
                )
                .unwrap(),
                16
            );
            assert!(fixture.0.try_wait().unwrap().is_none());
        }
    }
}
