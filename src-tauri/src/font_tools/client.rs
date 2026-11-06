//! 校验游戏进程后交给同架构辅助程序加载并发送配置；权限重试只提升辅助程序。
mod identity;
mod process;
use galgear_core::font_tools::{Architecture, Profile, executable_architecture};
use galgear_i18n::Message;
use std::{fs::File, path::Path};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE},
    System::Threading::{
        GetProcessTimes, IsWow64Process2, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    },
};

fn error(detail: impl ToString) -> Message {
    Message::detail("font.error", detail)
}
struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: 仅关闭本次调用拥有的进程查询句柄。
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// 校验实际窗口进程的 PE 和运行中架构，调用相同位数的加载器。
pub(super) fn apply(pid: u32, executable: &Path, profile: &Profile) -> Result<(), Message> {
    let mut request = profile.clone();
    request.face = identity::canonical_face(&profile.face)?;
    let packet = request.packet()?;
    let architecture = executable_architecture(&mut File::open(executable).map_err(error)?)?;
    // SAFETY: 有界输出缓冲；查询句柄保留到操作结束，校验实际映像避免迟到 PID 误用。
    let target_process = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(error(std::io::Error::last_os_error()));
        }
        let process = Process(process);
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        if QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length) == 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        let actual = std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
        if actual.canonicalize().map_err(error)? != executable.canonicalize().map_err(error)? {
            return Err(error("Target process changed"));
        }
        let mut machine = 0;
        let mut native = 0;
        let valid = IsWow64Process2(process.0, &mut machine, &mut native);
        if valid == 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        let machine = if machine == 0 { native } else { machine };
        if !matches!(
            (architecture, machine),
            (Architecture::X86, 0x14c) | (Architecture::X64, 0x8664)
        ) {
            return Err(error("Process and executable architecture mismatch"));
        }
        process
    };
    let created = creation_time(target_process.0)?;
    let loader = loader_path(architecture)?;
    process::apply(&loader, pid, created, &packet)
}

fn loader_path(architecture: Architecture) -> Result<std::path::PathBuf, Message> {
    let base = std::env::current_exe().map_err(error)?;
    let base = base
        .parent()
        .ok_or_else(|| error("Executable has no directory"))?;
    let directory = base.join("tools/font-hook").join(architecture.directory());
    #[cfg(debug_assertions)]
    let directory = if directory.exists() {
        directory
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tools/font-hook")
            .join(architecture.directory())
    };
    if !directory.join("GalGearFontHook.dll").is_file() {
        return Err(Message::new("font.components_missing"));
    }
    let loader = directory
        .join("font-hook-loader.exe")
        .canonicalize()
        .map_err(|_| Message::new("font.components_missing"))?;
    Ok(loader)
}

fn creation_time(process: HANDLE) -> Result<u64, Message> {
    let mut created = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: 进程查询句柄在本次调用期间由调用方持有，全部输出缓冲有效。
    if unsafe { GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

#[cfg(test)]
mod tests {
    use super::apply;
    use galgear_core::font_tools::{CharsetSpoof, Mapping, Profile, VerticalMetrics};
    use std::{
        io::{BufRead, BufReader, Write},
        path::Path,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    #[test]
    #[ignore = "需要日文非管理员 Locale Emulator 配置及 x86 字体夹具；不启动真实游戏"]
    fn native_locale_chinese_family_preserves_shift_jis() {
        use std::os::windows::process::CommandExt;
        struct Launcher(std::process::Child);
        impl Drop for Launcher {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let emulator = root.join("tools/locale-emulator/LEProc.exe");
        let profile = galgear_core::runtime::locale_profiles(&emulator)
            .unwrap()
            .into_iter()
            .find(|profile| profile.locale == "ja-JP" && !profile.admin)
            .expect("Japanese non-admin fixture profile required");
        let executable = root
            .join("target/font-fixtures/x86/font-fixture.exe")
            .canonicalize()
            .unwrap();
        for (face, requested, spoof) in [
            ("SimHei", "黑体", None),
            ("STZhongsong", "华文中宋", None),
            ("SimHei", "黑体", Some(CharsetSpoof { from: 128, to: 128 })),
            (
                "STZhongsong",
                "华文中宋",
                Some(CharsetSpoof { from: 128, to: 128 }),
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("font-result.txt");
            let _launcher = Launcher(
                Command::new(&emulator)
                    .args(["-runas", &profile.guid])
                    .arg(&executable)
                    .args(["--locale-family", face])
                    .arg(&output)
                    .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
                    .spawn()
                    .unwrap(),
            );
            let started = Instant::now();
            let mut submitted = false;
            loop {
                let text = std::fs::read_to_string(&output).unwrap_or_default();
                if !submitted && text.contains("READY") {
                    assert!(
                        text.contains("ACP 932"),
                        "fixture did not run under Japanese locale"
                    );
                    let pid = text.split_whitespace().nth(1).unwrap().parse().unwrap();
                    apply(
                        pid,
                        &executable,
                        &Profile {
                            face: requested.into(),
                            charset_spoof: spoof,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    submitted = true;
                }
                if text.contains("RESULT") {
                    assert!(submitted && text.contains("RESULT 0"), "{face}: {text}");
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "locale fixture timed out: {text}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    #[test]
    #[ignore = "需要 x86/x64 字体组件及 SimHei/STZhongsong；只操作隔离 GDI 进程"]
    fn native_chinese_family_matches_glyphs_and_preserves_shift_jis() {
        struct Fixture(std::process::Child);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        for arch in ["x86", "x64"] {
            for (face, requested, spoof) in [
                ("SimHei", "黑体", None),
                ("STZhongsong", "华文中宋", None),
                ("SimHei", "黑体", Some(CharsetSpoof { from: 128, to: 128 })),
                (
                    "STZhongsong",
                    "华文中宋",
                    Some(CharsetSpoof { from: 128, to: 128 }),
                ),
            ] {
                let executable = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../target/font-fixtures/{arch}/font-fixture.exe"))
                    .canonicalize()
                    .unwrap();
                let mut fixture = Fixture(
                    Command::new(&executable)
                        .args(["--family", face])
                        .stdout(Stdio::piped())
                        .spawn()
                        .unwrap(),
                );
                let mut ready = String::new();
                BufReader::new(fixture.0.stdout.take().unwrap())
                    .read_line(&mut ready)
                    .unwrap();
                assert_eq!(ready.trim(), "READY", "{arch} {face} fixture failed");
                apply(
                    fixture.0.id(),
                    &executable,
                    &Profile {
                        face: requested.into(),
                        charset_spoof: spoof,
                        ..Default::default()
                    },
                )
                .unwrap();
                let start = Instant::now();
                loop {
                    if let Some(status) = fixture.0.try_wait().unwrap() {
                        assert!(
                            status.success(),
                            "{arch} {face} glyph/encoding check failed: {status}"
                        );
                        break;
                    }
                    assert!(start.elapsed() < Duration::from_secs(25));
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    #[test]
    #[ignore = "先运行 scripts/build-font-hook.ps1 -Fixtures；仅注入隔离 GDI 测试进程"]
    fn native_live_updates_and_rejection_preserve_the_running_session() {
        struct Fixture(std::process::Child);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        for arch in ["x86", "x64"] {
            let executable = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../target/font-fixtures/{arch}/font-fixture.exe"))
                .canonicalize()
                .unwrap();
            let mut fixture = Fixture(
                Command::new(&executable)
                    .arg("--session")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
            let mut output = BufReader::new(fixture.0.stdout.take().unwrap());
            let mut input = fixture.0.stdin.take().unwrap();
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            let baseline: i32 = line.trim().strip_prefix("READY ").unwrap().parse().unwrap();
            let profile = Profile {
                face: "华文中宋".into(),
                height_percent: 200,
                ..Default::default()
            };
            // Busy is a rejected request, so only that response may be retried.
            let submit = |value: &Profile| {
                let started = Instant::now();
                loop {
                    match apply(fixture.0.id(), &executable, value) {
                        Err(message)
                            if message.key == "font.busy"
                                && started.elapsed() < Duration::from_secs(10) =>
                        {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        result => break result,
                    }
                }
            };
            let mut height = || {
                input.write_all(b"height\n").unwrap();
                let mut line = String::new();
                output.read_line(&mut line).unwrap();
                line.trim()
                    .strip_prefix("HEIGHT ")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap()
            };
            submit(&profile).unwrap();
            let enlarged = height();
            assert!(
                enlarged >= baseline * 3 / 2,
                "{arch}: actual raster did not grow"
            );
            let missing = Profile {
                face: "GG_missing_font_7617".into(),
                ..profile.clone()
            };
            assert_eq!(submit(&missing).unwrap_err().key, "font.not_found");
            assert_eq!(
                height(),
                enlarged,
                "rejected font must preserve the current raster"
            );
            submit(&Profile::default()).unwrap();
            assert_eq!(
                height(),
                baseline,
                "disable must restore the running process"
            );
            submit(&profile).unwrap();
            assert_eq!(height(), enlarged, "reapplying must reuse a working font");
        }
    }

    #[test]
    #[ignore = "先运行 scripts/build-font-hook.ps1 -Fixtures；仅注入隔离 GDI 测试进程"]
    fn native_hook_x86_x64_changes_actual_glyph_pixels() {
        for arch in ["x86", "x64"] {
            let executable = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../target/font-fixtures/{arch}/font-fixture.exe"))
                .canonicalize()
                .unwrap();
            let mut child = Command::new(&executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let result = || {
                let mut ready = String::new();
                BufReader::new(child.stdout.take().unwrap())
                    .read_line(&mut ready)
                    .unwrap();
                assert_eq!(ready.trim(), "READY");
                apply(
                    child.id(),
                    &executable,
                    &Profile {
                        face: "华文中宋".into(),
                        height_percent: 200,
                        width_percent: 100,
                        char_spacing: 2,
                        line_spacing: 4,
                        weight: 700,
                        vertical_metrics: Some(VerticalMetrics {
                            ascent: 900,
                            descent: -150,
                        }),
                        charset_spoof: Some(CharsetSpoof { from: 128, to: 128 }),
                        mapping: Mapping::Japanese,
                        ..Default::default()
                    },
                )
                .unwrap();
                child.stdin.as_mut().unwrap().write_all(b"x").unwrap();
                let start = Instant::now();
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        assert!(status.success(), "{arch} raster test failed: {status}");
                        return;
                    }
                    assert!(start.elapsed() < Duration::from_secs(25));
                    std::thread::sleep(Duration::from_millis(50));
                }
            };
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(result));
            let _ = child.kill();
            let _ = child.wait();
            if let Err(panic) = outcome {
                std::panic::resume_unwind(panic);
            }
        }
    }
}
