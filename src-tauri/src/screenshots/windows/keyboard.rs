//! 后台 Raw Input 补足游戏吞掉 WM_HOTKEY 的情况，只维护按住状态，不记录输入内容。
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::Input::*,
};

pub(in crate::screenshots) struct Keyboard {
    window: HWND,
    previous: Option<RAWINPUTDEVICE>,
    down: [bool; 256],
}
impl Keyboard {
    pub(in crate::screenshots) fn start(window: HWND) -> Result<Self, String> {
        // SAFETY: Win32 仅填充按其返回长度分配的设备列表，设备结构均正确对齐。
        unsafe {
            let mut count = 0;
            if GetRegisteredRawInputDevices(
                ptr::null_mut(),
                &mut count,
                size_of::<RAWINPUTDEVICE>() as u32,
            ) == u32::MAX
            {
                return Err(super::last_error());
            }
            let mut devices = vec![RAWINPUTDEVICE::default(); count as usize];
            if count > 0
                && GetRegisteredRawInputDevices(
                    devices.as_mut_ptr(),
                    &mut count,
                    size_of::<RAWINPUTDEVICE>() as u32,
                ) == u32::MAX
            {
                return Err(super::last_error());
            }
            let previous = devices
                .into_iter()
                .find(|device| device.usUsagePage == 1 && device.usUsage == 6);
            let device = RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: window,
            };
            if RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
                return Err(super::last_error());
            }
            Ok(Self {
                window,
                previous,
                down: [false; 256],
            })
        }
    }
    pub(in crate::screenshots) fn clear(&mut self) {
        self.down.fill(false);
    }
    pub(in crate::screenshots) fn pressed(&mut self, message: LPARAM, keys: &[u16]) -> bool {
        // SAFETY: WM_INPUT 的句柄只在当前消息分发期间读取；RAWINPUT 已正确对齐且
        // 容量覆盖键盘结构。检查类型及实际字节数后才读取联合体的 keyboard 分支。
        unsafe {
            let mut raw = RAWINPUT::default();
            let mut size = size_of::<RAWINPUT>() as u32;
            let read = GetRawInputData(
                message as HRAWINPUT,
                RID_INPUT,
                (&mut raw as *mut RAWINPUT).cast(),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            );
            if read == u32::MAX
                || read < (size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>()) as u32
                || raw.header.dwType != RIM_TYPEKEYBOARD
            {
                return false;
            }
            self.observe(raw.data.keyboard, keys)
        }
    }
    fn observe(&mut self, event: RAWKEYBOARD, keys: &[u16]) -> bool {
        if event.VKey >= 255 || event.MakeCode == 255 || keys.len() < 2 {
            return false;
        }
        let key = match event.VKey {
            0x10 => {
                if event.MakeCode == 0x36 {
                    0xa1
                } else {
                    0xa0
                }
            }
            0x11 => {
                if event.Flags & 2 != 0 {
                    0xa3
                } else {
                    0xa2
                }
            }
            0x12 => {
                if event.Flags & 2 != 0 {
                    0xa5
                } else {
                    0xa4
                }
            }
            key => key,
        };
        let pressed = event.Flags & 1 == 0;
        let repeat = self.down[usize::from(key)];
        self.down[usize::from(key)] = pressed;
        if !pressed || repeat || key != keys[keys.len() - 1] {
            return false;
        }
        [
            (0x10, 0xa0, 0xa1),
            (0x11, 0xa2, 0xa3),
            (0x12, 0xa4, 0xa5),
            (0x5b, 0x5b, 0x5c),
        ]
        .into_iter()
        .all(|(modifier, left, right)| {
            (self.down[left] || self.down[right]) == keys[..keys.len() - 1].contains(&modifier)
        })
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        // Raw Input 在同一进程内每类设备只允许一个窗口。游戏失焦时恢复先前注册，
        // 避免永久覆盖桌面框架的设备事件接收窗口。
        if self.window.is_null() {
            return;
        }
        let device = self.previous.unwrap_or(RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 6,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: ptr::null_mut(),
        });
        // SAFETY: 句柄仍属于本进程，移除时目标为 NULL；在销毁截图窗口之前释放登记。
        unsafe {
            if RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
                // 退出/失焦时无法返回错误，保留不含按键内容的系统诊断。
                eprintln!("Screenshot keyboard cleanup: {}", super::last_error());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                        && let Some((game, executable)) = super::super::foreground()
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

    fn event(key: u16, flags: u16, scan: u16) -> RAWKEYBOARD {
        RAWKEYBOARD {
            VKey: key,
            Flags: flags,
            MakeCode: scan,
            ..Default::default()
        }
    }
    #[test]
    fn raw_input_handles_both_shifts_repeat_and_exact_modifiers() {
        let mut keyboard = Keyboard {
            window: ptr::null_mut(),
            previous: None,
            down: [false; 256],
        };
        assert!(!keyboard.observe(event(0x10, 0, 0x36), &[0x10, 0x53]));
        assert!(keyboard.observe(event(0x53, 0, 0x1f), &[0x10, 0x53]));
        assert!(!keyboard.observe(event(0x53, 0, 0x1f), &[0x10, 0x53]));
        keyboard.observe(event(0x53, 1, 0x1f), &[0x10, 0x53]);
        keyboard.observe(event(0x11, 0, 0x1d), &[0x10, 0x53]);
        assert!(!keyboard.observe(event(0x53, 0, 0x1f), &[0x10, 0x53]));
        keyboard.clear();
        assert!(!keyboard.observe(event(0x53, 0, 0x1f), &[0x10, 0x53]));
    }
}
