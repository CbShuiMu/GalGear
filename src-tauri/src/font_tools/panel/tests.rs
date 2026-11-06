use super::{Action, Panel};
use crate::game_window;
use galgear_core::{Settings, font_tools::Profile};
use galgear_i18n::Language;
use std::{
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

#[test]
#[ignore = "需要交互桌面；创建隔离面板，不注入游戏、不写配置"]
fn native_font_panel_position_resize_and_controls() {
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: 只销毁测试拥有的窗口。
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    // SAFETY: 所有窗口属于本测试线程，输出指针有效，DPI 上下文在资源释放后恢复。
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let target = Window(CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("GalGear font panel fixture"),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            960,
            660,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        ));
        assert!(!target.0.is_null());
        ShowWindow(target.0, SW_SHOWNOACTIVATE);
        for (theme, language) in [
            ("light", Language::Chinese),
            ("dark", Language::Japanese),
            ("light", Language::English),
        ] {
            let settings = Settings {
                theme: theme.into(),
                language,
                ..Settings::default()
            };
            let mut panel = Panel::new(target.0, "GalGear — Font preview", &settings).unwrap();
            let profile = Profile {
                face: "华文中宋".into(),
                height_percent: 125,
                width_percent: 90,
                char_spacing: -2,
                line_spacing: 4,
                weight: 700,
                vertical_metrics: Some(galgear_core::font_tools::VerticalMetrics {
                    ascent: 900,
                    descent: -150,
                }),
                charset_spoof: Some(galgear_core::font_tools::CharsetSpoof { from: 134, to: 128 }),
                mapping: galgear_core::font_tools::Mapping::TraditionalToSimplified,
                ..Default::default()
            };
            panel.set_profile(&profile);
            assert_eq!(panel.profile().unwrap(), profile);
            panel.toggle();
            assert_ne!(IsWindowVisible(panel.hwnd), 0);
            SendMessageW(panel.hwnd, WM_COMMAND, 1, 0);
            assert!(matches!(panel.action(), Some(Action::Apply)));
            for (width, height) in [(960, 660), (500, 520)] {
                SetWindowPos(
                    target.0,
                    ptr::null_mut(),
                    120,
                    110,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                panel.follow();
                let client = game_window::bounds(target.0).unwrap();
                let mut actual = std::mem::zeroed();
                GetWindowRect(panel.hwnd, &mut actual);
                assert_eq!(actual.right, client.right - 8);
                assert_eq!(actual.top, client.top + 8);
                assert!(actual.left >= client.left && actual.bottom <= client.bottom);
            }
            if let Ok(duration) = std::env::var("GALGEAR_FONT_PREVIEW_MS") {
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(duration.parse().unwrap()) {
                    let mut msg = std::mem::zeroed();
                    while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            panel.toggle();
            assert_eq!(IsWindowVisible(panel.hwnd), 0);
            SetWindowPos(
                target.0,
                ptr::null_mut(),
                120,
                110,
                200,
                200,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            panel.toggle();
            assert_eq!(
                IsWindowVisible(panel.hwnd),
                0,
                "panel must not spill out of a tiny game window"
            );
            SetWindowPos(
                target.0,
                ptr::null_mut(),
                120,
                110,
                960,
                660,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        drop(target);
        if !previous.is_null() {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}

#[test]
#[ignore = "需要交互桌面；只向隔离窗口发送 Shift+T/S，不截图、不操作真实游戏"]
fn native_shared_raw_input_opens_panel_without_wm_hotkey() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    };
    struct Resources {
        target: HWND,
        input: crate::game_input::Service,
    }
    impl Drop for Resources {
        fn drop(&mut self) {
            self.input.stop();
            // SAFETY: 只销毁本测试创建的窗口。
            unsafe {
                DestroyWindow(self.target);
            }
        }
    }
    fn pump() {
        // SAFETY: 消费本测试线程的窗口消息。
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    fn send(target: HWND, key: u16) {
        let inputs = [
            (0x10, 0),
            (key, 0),
            (key, KEYEVENTF_KEYUP),
            (0x10, KEYEVENTF_KEYUP),
        ]
        .map(|(key, flags)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        });
        // SAFETY: 焦点必须在隔离窗口；发送完整按下/释放序列，不改变游戏或用户数据。
        unsafe {
            SetForegroundWindow(target);
            assert_eq!(GetForegroundWindow(), target);
            assert_eq!(
                SendInput(
                    inputs.len() as u32,
                    inputs.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32
                ),
                inputs.len() as u32
            );
        }
    }
    fn wait(input: &crate::game_input::Subscription) {
        let started = Instant::now();
        loop {
            pump();
            if input.pressed().unwrap() {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "Raw Input did not reach the configured subscriber"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let (service, [mut screenshots, mut fonts]) = crate::game_input::start().unwrap();
    // SAFETY: 本测试拥有窗口；不注册 WM_HOTKEY，强制覆盖这次缺失的后台接收通路。
    let target = unsafe {
        CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("GalGear shortcut fixture"),
            WS_OVERLAPPEDWINDOW,
            160,
            120,
            960,
            660,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    assert!(!target.is_null());
    let resources = Resources {
        target,
        input: service,
    };
    // SAFETY: 只激活测试窗口，测试结束由 Resources 回收。
    unsafe {
        ShowWindow(target, SW_SHOW);
        SetForegroundWindow(target);
    }
    let mut panel = Panel::new(target, "Shortcut fixture", &Settings::default()).unwrap();
    screenshots
        .configure(Some((target, &[0x10, 0x53])))
        .unwrap();
    fonts.configure(Some((target, &[0x10, 0x54]))).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    send(target, 0x54);
    wait(&fonts);
    assert!(!screenshots.pressed().unwrap());
    panel.toggle();
    // SAFETY: 查询本测试拥有的面板窗口。
    assert_ne!(unsafe { IsWindowVisible(panel.hwnd) }, 0);
    panel.toggle();
    std::thread::sleep(Duration::from_millis(150));
    send(target, 0x53);
    wait(&screenshots);
    assert!(!fonts.pressed().unwrap());
    screenshots.configure(None).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    send(target, 0x54);
    wait(&fonts);
    drop(panel);
    drop(fonts);
    drop(screenshots);
    drop(resources);
}
