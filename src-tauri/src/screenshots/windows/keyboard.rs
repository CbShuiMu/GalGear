#[cfg(test)]
mod tests {
    use crate::game_input::Keyboard;
    use std::ptr;
    #[test]
    #[ignore = "需要手动按 Shift+S；只采集 GALGEAR_SCREENSHOT_TEST_GAME 指定的前台游戏"]
    fn live_game_raw_hotkey_capture_and_clipboard() {
        use std::time::{Duration, Instant};
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let expected = std::path::PathBuf::from(
            std::env::var_os("GALGEAR_SCREENSHOT_TEST_GAME").expect("指定测试游戏目录"),
        )
        .canonicalize()
        .unwrap();
        // SAFETY: 消息窗口在当前线程创建、消费并销毁；仅查询与显式指定目录匹配的前台。
        unsafe {
            let window = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("GalGear screenshot input test"),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!window.is_null());
            let mut keyboard = Keyboard::start(window).unwrap();
            let start = Instant::now();
            let mut completed = false;
            eprintln!("READY: 在指定游戏的标题画面按 Shift+S（等待 90 秒）");
            while start.elapsed() < Duration::from_secs(90) {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if message.hwnd == window
                        && message.message == WM_INPUT
                        && keyboard.pressed(message.lParam, &[0x10, 0x53])
                        && let Some((game, executable)) = crate::game_window::foreground()
                        && executable.starts_with(&expected)
                    {
                        let frame = super::super::capture(game).unwrap();
                        let folder = tempfile::tempdir().unwrap();
                        let path = galgear_core::screenshots::save_png(
                            folder.path(),
                            frame.width,
                            frame.height,
                            &frame.rgba,
                        )
                        .unwrap();
                        super::super::copy(&frame, window, &path).unwrap();
                        eprintln!(
                            "Raw Input -> PNG -> Clipboard verified: {}x{}",
                            frame.width, frame.height
                        );
                        let mut notice = super::super::Notification::default();
                        notice.show(game, "截图已保存并复制到剪贴板。", "light", "pink", false);
                        let shown = Instant::now();
                        while shown.elapsed() < Duration::from_secs(3) {
                            let mut message = MSG::default();
                            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0
                            {
                                TranslateMessage(&message);
                                DispatchMessageW(&message);
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        completed = true;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if completed {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(keyboard);
            DestroyWindow(window);
            assert!(completed, "未收到指定前台游戏的 Shift+S");
        }
    }

}
