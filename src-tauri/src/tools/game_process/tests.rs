use super::{Child, ShellCommand, launch_error, parameters, spawn};
use std::{
    ffi::OsStr,
    os::windows::io::FromRawHandle,
    process::Command,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, ERROR_CANCELLED, LocalFree},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE},
    UI::Shell::{CommandLineToArgvW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS},
};

#[test]
fn launch_requests_admin_and_preserves_launcher_and_working_directory() {
    let mut command = Command::new(r"\\?\C:\ゲーム\启动器.exe");
    command
        .arg(r"C:\ゲーム\本体.exe")
        .current_dir(r"\\?\C:\ゲーム\工作目录");
    let command = ShellCommand::new(&command).unwrap();
    let request = command.request();
    // SAFETY: request 的 verb 指向静态 NUL 结尾的 runas 字面量。
    assert_eq!(
        unsafe { std::slice::from_raw_parts(request.lpVerb, 6) },
        "runas\0".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(
        request.fMask & SEE_MASK_NOCLOSEPROCESS,
        SEE_MASK_NOCLOSEPROCESS
    );
    assert_eq!(request.fMask & SEE_MASK_NOASYNC, SEE_MASK_NOASYNC);
    assert_eq!(
        String::from_utf16(&command.file).unwrap(),
        "C:\\ゲーム\\启动器.exe\0"
    );
    assert_eq!(
        String::from_utf16(&command.directory).unwrap(),
        "C:\\ゲーム\\工作目录\0"
    );
    assert_eq!(
        String::from_utf16(&command.parameters).unwrap(),
        "\"C:\\ゲーム\\本体.exe\"\0"
    );
}

#[test]
fn windows_argument_parser_round_trips_empty_unicode_quotes_and_slashes() {
    let expected = [
        "",
        "普通参数",
        "日本語 with spaces",
        "a\tb",
        r#"a"b"#,
        r#"a\"b"#,
        r"C:\game dir\",
        r"\\server\共享\",
        "%PATH% & $(test)",
    ];
    let parameters = parameters(expected.iter().map(OsStr::new)).unwrap();
    let mut line: Vec<_> = "probe.exe ".encode_utf16().collect();
    line.extend(parameters);
    let mut count = 0;
    // SAFETY: 命令行以 NUL 结尾，count 是有效输出指针；返回内存由 LocalFree 释放。
    let raw = unsafe { CommandLineToArgvW(line.as_ptr(), &mut count) };
    assert!(!raw.is_null());
    // SAFETY: API 返回 count 个 NUL 结尾字符串，均在 LocalFree 前读取。
    let actual: Vec<_> = unsafe {
        std::slice::from_raw_parts(raw, count as usize)
            .iter()
            .skip(1)
            .map(|arg| {
                let mut len = 0;
                while *arg.add(len) != 0 {
                    len += 1;
                }
                String::from_utf16(std::slice::from_raw_parts(*arg, len)).unwrap()
            })
            .collect()
    };
    // SAFETY: raw 是 CommandLineToArgvW 分配的独立内存，且仅释放一次。
    unsafe { LocalFree(raw.cast()) };
    assert_eq!(actual, expected);
}

#[test]
fn nul_in_launch_input_is_rejected_before_spawning() {
    let mut command = Command::new("game.exe");
    command.arg("safe\0unexpected");
    assert!(ShellCommand::new(&command).is_err());
    assert!(ShellCommand::new(&Command::new("game\0.exe")).is_err());
    assert!(ShellCommand::new(Command::new("game.exe").current_dir("dir\0other")).is_err());
}

#[test]
fn cancelled_authorization_is_distinct_from_launch_failure() {
    assert_eq!(
        launch_error(std::io::Error::from_raw_os_error(ERROR_CANCELLED as i32)).key,
        "error.launch_cancelled"
    );
    assert_eq!(
        launch_error(std::io::Error::from_raw_os_error(
            ERROR_ACCESS_DENIED as i32
        ))
        .key,
        "error.launch"
    );
}

#[test]
fn process_wait_only_requires_synchronization_access() {
    // SAFETY: 打开当前测试进程，仅请求等待权限，句柄独立交给 Child 释放。
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, std::process::id()) };
    assert!(!raw.is_null());
    // SAFETY: raw 是刚成功打开且尚未转移所有权的进程句柄。
    let mut child = Child(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) });
    assert_eq!(child.try_wait().unwrap(), None);
}

#[test]
fn converted_child_keeps_identity_after_a_launcher_exits() {
    use std::os::windows::process::CommandExt;
    let executable = std::env::current_exe().unwrap();
    let mut process = Command::new(&executable)
        .arg("--list")
        .stdout(std::process::Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .spawn()
        .unwrap();
    let pid = process.id();
    assert!(process.wait().unwrap().success());
    let mut child = Child::from(process);
    assert_eq!(child.id(), pid);
    assert_eq!(child.try_wait().unwrap(), Some(()));
    galgear_core::runtime::Monitor::default()
        .launched(&executable, &child)
        .unwrap();
}

#[test]
#[ignore = "需要人工响应 Windows UAC；仅运行临时测试程序，不启动真实游戏"]
fn native_admin_launch_preserves_tracking_and_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("游戏 工作目录");
    std::fs::create_dir(&directory).unwrap();
    let executable = directory.join("管理员 probe.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    struct Stop(std::path::PathBuf);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = std::fs::write(self.0.join("stop"), b"");
        }
    }
    let stop = Stop(directory.clone());
    let mut command = Command::new(&executable);
    command.args([
        "--ignored",
        "--exact",
        "tools::game_process::tests::native_admin_probe",
    ]);
    command.current_dir(&directory);
    let mut child = spawn(&command).unwrap();
    let monitor = galgear_core::runtime::Monitor::default();
    monitor.launched(&executable, &child).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !directory.join("ready.json").exists() {
        assert!(Instant::now() < deadline, "Elevated probe did not start");
        assert!(
            child.try_wait().unwrap().is_none(),
            "Elevated probe exited early"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("ready.json")).unwrap()).unwrap();
    assert_eq!(ready["admin"], true);
    assert_eq!(ready["pid"], child.id());
    assert_eq!(
        std::fs::canonicalize(ready["directory"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&directory).unwrap()
    );
    assert_eq!(
        monitor.detect(executable.to_str()),
        galgear_core::runtime::Status::Running
    );
    drop(stop);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "Elevated probe did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
    while monitor.detect(executable.to_str()) != galgear_core::runtime::Status::Stopped {
        assert!(Instant::now() < deadline, "Elevated session did not finish");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "仅由隔离的管理员启动测试调用"]
fn native_admin_probe() {
    use std::os::windows::io::{AsRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let directory = std::env::current_dir().unwrap();
    assert!(directory.ends_with("游戏 工作目录"));
    let mut raw = std::ptr::null_mut();
    // SAFETY: 查询本测试进程令牌，raw 是有效输出位置。
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) },
        0
    );
    // SAFETY: raw 是刚打开的令牌句柄，所有权只转移一次。
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut length = 0;
    // SAFETY: 令牌有效，输出缓冲及长度对应 TokenElevation 的结构。
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut length,
            )
        },
        0
    );
    let ready = serde_json::json!({"admin": elevation.TokenIsElevated != 0, "pid": std::process::id(), "directory": directory});
    std::fs::write(
        directory.join("ready.tmp"),
        serde_json::to_vec(&ready).unwrap(),
    )
    .unwrap();
    std::fs::rename(directory.join("ready.tmp"), directory.join("ready.json")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !directory.join("stop").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
}
