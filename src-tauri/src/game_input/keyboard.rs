//! 后台 Raw Input 补足游戏吞掉 WM_HOTKEY 的情况，只维护按住状态，不记录输入内容。
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::Input::{
        GetRawInputData, GetRegisteredRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
        RAWINPUTHEADER, RAWKEYBOARD, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD,
        RegisterRawInputDevices,
    },
};

pub(crate) struct Keyboard {
    window: HWND,
    previous: Option<RAWINPUTDEVICE>,
    down: [bool; 256],
}
impl Keyboard {
    pub(crate) fn start(window: HWND) -> Result<Self, String> {
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
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.down.fill(false);
    }
    #[cfg(test)]
    pub(crate) fn pressed(&mut self, message: LPARAM, keys: &[u16]) -> bool {
        self.pressed_many(message, &[keys]).is_some()
    }
    pub(super) fn pressed_many(&mut self, message: LPARAM, keys: &[&[u16]]) -> Option<usize> {
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
                return None;
            }
            self.observe_many(raw.data.keyboard, keys)
        }
    }
    #[cfg(test)]
    fn observe(&mut self, event: RAWKEYBOARD, keys: &[u16]) -> bool {
        self.observe_many(event, &[keys]).is_some()
    }
    fn observe_many(&mut self, event: RAWKEYBOARD, keys: &[&[u16]]) -> Option<usize> {
        if event.VKey >= 255 || event.MakeCode == 255 {
            return None;
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
        if !pressed || repeat {
            return None;
        }
        keys.iter().position(|keys| {
            keys.len() >= 2
                && keys[keys.len() - 1] == key
                && [
                    (0x10, 0xa0, 0xa1),
                    (0x11, 0xa2, 0xa3),
                    (0x12, 0xa4, 0xa5),
                    (0x5b, 0x5b, 0x5c),
                ]
                .into_iter()
                .all(|(modifier, left, right)| {
                    (self.down[left] || self.down[right])
                        == keys[..keys.len() - 1].contains(&modifier)
                })
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
        // SAFETY: 句柄仍属于本进程，移除时目标为 NULL；在销毁接收窗口之前释放登记。
        unsafe {
            if RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
                // 退出/失焦时无法返回错误，保留不含按键内容的系统诊断。
                eprintln!("Game keyboard cleanup: {}", super::last_error());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn screenshot_and_panel_share_one_keyboard_state() {
        for scan in [0x2a, 0x36] {
            let mut keyboard = Keyboard {
                window: ptr::null_mut(),
                previous: None,
                down: [false; 256],
            };
            let keys: [&[u16]; 2] = [&[0x10, 0x53], &[0x10, 0x54]];
            assert_eq!(keyboard.observe_many(event(0x10, 0, scan), &keys), None);
            assert_eq!(keyboard.observe_many(event(0x54, 0, 0x14), &keys), Some(1));
            assert_eq!(keyboard.observe_many(event(0x54, 0, 0x14), &keys), None);
            keyboard.observe_many(event(0x54, 1, 0x14), &keys);
            assert_eq!(keyboard.observe_many(event(0x53, 0, 0x1f), &keys), Some(0));
            keyboard.observe_many(event(0x53, 1, 0x1f), &keys);
            // 截图停用后，同一个 Shift 状态仍能呼出字体面板。
            assert_eq!(
                keyboard.observe_many(event(0x54, 0, 0x14), &[&[], keys[1]]),
                Some(1)
            );
        }
    }
}
