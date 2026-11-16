//! 以真实 LEP 启动隔离窗口，验证前台识别、Shift+S、原尺寸 PNG 与剪贴板。
use super::{Clipboard, DpiContext, capture, copy, register, unregister};
use std::{
    mem::size_of,
    path::Path,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::BITMAPINFOHEADER,
    System::{DataExchange::*, Memory::*},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

struct Fixture(HWND);
impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: 仅关闭由当前隔离测试启动并经 PID 校验的窗口。
        unsafe {
            PostMessageW(self.0, WM_CLOSE, 0, 0);
        }
    }
}

fn pump() -> bool {
    let mut pressed = false;
    // SAFETY: 只消费当前测试线程的窗口消息。
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            pressed |= message.message == WM_HOTKEY;
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    pressed
}

#[test]
#[ignore = "需要 LEP、x86/x64 字体夹具和交互桌面；向隔离窗口发送 Shift+S，写入测试剪贴板"]
fn native_locale_x86_x64_hotkey_png_and_clipboard() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let emulator = root.join(crate::bundled_tools::LOCALE);
    galgear_core::runtime::prepare_locale_config(&emulator).unwrap();
    let profile = galgear_core::runtime::locale_profiles(&emulator)
        .unwrap()
        .into_iter()
        .find(|p| p.locale == "ja-JP" && !p.admin)
        .unwrap();
    for arch in ["x86", "x64"] {
        let folder = tempfile::tempdir().unwrap();
        let output = folder.path().join("ready.txt");
        let executable = root
            .join(format!("target/font-fixtures/{arch}/font-fixture.exe"))
            .canonicalize()
            .unwrap();
        let mut launcher = std::process::Command::new(&emulator)
            .args(["-runas", &profile.guid])
            .arg(&executable)
            .arg("--locale-window")
            .arg(&output)
            .spawn()
            .unwrap();
        let started = Instant::now();
        let pid = loop {
            let text = std::fs::read_to_string(&output).unwrap_or_default();
            if text.contains("READY") {
                assert!(text.contains("ACP 932"), "{arch}: {text}");
                break text
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .parse::<u32>()
                    .unwrap();
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "{arch}: LEP fixture did not start: {text}"
            );
            std::thread::sleep(Duration::from_millis(30));
        };
        // SAFETY: 枚举回调同步借用本栈的 (PID, HWND)，不访问其他进程内存。
        unsafe extern "system" fn find(window: HWND, data: isize) -> i32 {
            let search = unsafe { &mut *(data as *mut (u32, HWND)) };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(window, &mut pid);
            }
            if pid == search.0 && unsafe { IsWindowVisible(window) } != 0 {
                search.1 = window;
                return 0;
            }
            1
        }
        let mut search: (u32, HWND) = (pid, ptr::null_mut());
        // SAFETY: search 在整个同步枚举期间有效。
        unsafe {
            EnumWindows(Some(find), &mut search as *mut _ as isize);
        }
        assert!(!search.1.is_null(), "{arch}: fixture window missing");
        let fixture = Fixture(search.1);
        // SAFETY: 仅操作隔离窗口；所有消息和 GDI 缓冲归本测试所有。
        unsafe {
            let old_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            assert!(!old_dpi.is_null());
            let _dpi = DpiContext(old_dpi);
            SetForegroundWindow(fixture.0);
            let started = Instant::now();
            while GetForegroundWindow() != fixture.0 {
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "Activate GalGear LEP screenshot fixture"
                );
                pump();
                std::thread::sleep(Duration::from_millis(20));
            }
            let (foreground, image) = crate::game_window::foreground().unwrap();
            assert_eq!(foreground, fixture.0);
            assert_eq!(image, executable);
            assert!(
                galgear_core::runtime::process_ids(&executable)
                    .unwrap()
                    .contains(&pid)
            );
            let (service, [mut screenshots, _fonts]) = crate::game_input::start().unwrap();
            screenshots
                .configure(Some((fixture.0, &[0x10, 0x53])))
                .unwrap();
            register(ptr::null_mut(), 83, "Shift+S").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(GetForegroundWindow(), fixture.0);
            let inputs = [
                (0x2a, 0),
                (0x1f, 0),
                (0x1f, KEYEVENTF_KEYUP),
                (0x2a, KEYEVENTF_KEYUP),
            ]
            .map(|(key, flags)| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: key,
                        dwFlags: flags | KEYEVENTF_SCANCODE,
                        ..Default::default()
                    },
                },
            });
            assert_eq!(SendInput(4, inputs.as_ptr(), size_of::<INPUT>() as i32), 4);
            let started = Instant::now();
            let mut hotkey = false;
            let mut raw = false;
            while !(hotkey && raw) && started.elapsed() < Duration::from_secs(3) {
                hotkey |= pump();
                raw |= screenshots.pressed().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            unregister(ptr::null_mut(), 83);
            screenshots.configure(None).unwrap();
            service.stop();
            assert!(hotkey && raw, "{arch}: WM_HOTKEY={hotkey}, Raw Input={raw}");
            let frame = capture(fixture.0).unwrap();
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            assert_ne!(GetClientRect(fixture.0, &mut rect), 0);
            assert_eq!(
                (frame.width, frame.height),
                (rect.right as u32, rect.bottom as u32)
            );
            let center = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
            assert_eq!(&frame.rgba[center..center + 4], &[204, 102, 51, 255]);
            let path = galgear_core::screenshots::save_png(
                folder.path(),
                frame.width,
                frame.height,
                &frame.rgba,
            )
            .unwrap();
            let png = std::fs::read(&path).unwrap();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(
                u32::from_be_bytes(png[16..20].try_into().unwrap()),
                frame.width
            );
            assert_eq!(
                u32::from_be_bytes(png[20..24].try_into().unwrap()),
                frame.height
            );
            copy(&frame, fixture.0, &path).unwrap();
            assert_ne!(OpenClipboard(ptr::null_mut()), 0);
            let clipboard = Clipboard;
            let dib = GetClipboardData(8);
            assert!(!dib.is_null());
            let header = GlobalLock(dib).cast::<BITMAPINFOHEADER>();
            assert!(!header.is_null());
            assert_eq!(
                ((*header).biWidth, (*header).biHeight),
                (rect.right, rect.bottom)
            );
            GlobalUnlock(dib);
            assert_ne!(
                IsClipboardFormatAvailable(RegisterClipboardFormatW(windows_sys::w!("PNG"))),
                0
            );
            drop(clipboard);
            eprintln!(
                "LEP {arch}: ACP 932, foreground PID, Shift+S (WM_HOTKEY + Raw Input), {}x{} PNG and clipboard verified",
                frame.width, frame.height
            );
        }
        drop(fixture);
        let _ = launcher.wait();
    }
}
