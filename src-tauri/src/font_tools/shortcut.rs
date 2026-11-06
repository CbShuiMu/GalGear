//! 字体面板快捷键的原生注册；与 Raw Input 共用同一组已校验按键。
use galgear_core::runtime;
use galgear_i18n::Message;
use std::ptr;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};

pub(super) struct Shortcut {
    pub(super) name: String,
    pub(super) keys: Vec<u16>,
    modifiers: u32,
    key: u32,
}

impl Shortcut {
    pub(super) fn parse(value: &str) -> Result<Self, Message> {
        let name =
            runtime::canonical_hotkey(value).map_err(|_| Message::new("shortcut.invalid"))?;
        let keys = runtime::hotkey_keys(&name).map_err(|_| Message::new("shortcut.invalid"))?;
        let key = u32::from(keys[keys.len() - 1]);
        let mut modifiers = MOD_NOREPEAT;
        for modifier in &keys[..keys.len() - 1] {
            modifiers |= match modifier {
                0x10 => MOD_SHIFT,
                0x11 => MOD_CONTROL,
                0x12 => MOD_ALT,
                0x5b => MOD_WIN,
                _ => 0,
            };
        }
        Ok(Self {
            name,
            keys,
            modifiers,
            key,
        })
    }

    pub(super) fn register(&self, id: i32) -> Result<(), Message> {
        // SAFETY: 组合已完整校验；NULL HWND 将注册限定在当前线程的固定 ID。
        if unsafe { RegisterHotKey(ptr::null_mut(), id, self.modifiers, self.key) } == 0 {
            Err(Message::new("font.hotkey_conflict").arg("shortcut", &self.name))
        } else {
            Ok(())
        }
    }

    pub(super) fn probe(&self) -> Result<(), Message> {
        self.register(0x4747)?;
        // SAFETY: 释放本线程刚刚成功注册的测试 ID，不影响字体面板工作线程。
        unsafe {
            UnregisterHotKey(ptr::null_mut(), 0x4747);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Shortcut;
    use std::ptr;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;

    #[test]
    #[ignore = "需要交互桌面；临时注册测试组合，不发送按键、不操作游戏"]
    fn custom_shortcut_registration_detects_conflicts_and_releases_old_keys() {
        struct Registration(i32);
        impl Drop for Registration {
            fn drop(&mut self) {
                // SAFETY: 撤销本测试线程成功注册的固定测试 ID。
                unsafe {
                    UnregisterHotKey(ptr::null_mut(), self.0);
                }
            }
        }
        let first = Shortcut::parse("ctrl+alt+F23").unwrap();
        assert_eq!(first.name, "Ctrl+Alt+F23");
        assert_eq!(first.keys, [0x11, 0x12, 0x86]);
        first.register(0x4748).unwrap();
        let guard = Registration(0x4748);
        assert!(
            std::thread::spawn(|| Shortcut::parse("Alt+Ctrl+F23").unwrap().probe())
                .join()
                .unwrap()
                .is_err()
        );
        drop(guard);
        first.probe().unwrap();
        Shortcut::parse("Shift+F24").unwrap().probe().unwrap();
    }
}
