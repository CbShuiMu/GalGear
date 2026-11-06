//! 管理员游戏的输入桥接。辅助进程只识别约定快捷键并回传通知，不读写文件或执行外部命令。
//! 配置通过父进程消息窗口的属性发布；代数隔离切换游戏、修改快捷键后的迟到通知。
use super::{keyboard::Keyboard, last_error};
use crate::game_window::magpie;
use galgear_i18n::Message;
use std::{
    ffi::OsStr,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
    sync::mpsc,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, HANDLE, HWND, WAIT_TIMEOUT},
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::{
        Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetPropW,
            GetWindowThreadProcessId, HWND_MESSAGE, IsWindow, MSG, PM_REMOVE, PeekMessageW,
            PostMessageW, RemovePropW, SW_HIDE, SetPropW, WM_APP, WM_INPUT,
        },
    },
};

const ARGUMENT: &str = "--galgear-game-input";
const GENERATION: windows_sys::core::PCWSTR = windows_sys::w!("GalGear.GameInput.Generation");
const TARGET: windows_sys::core::PCWSTR = windows_sys::w!("GalGear.GameInput.Target");
const KEYS: [windows_sys::core::PCWSTR; 2] = [
    windows_sys::w!("GalGear.GameInput.ScreenshotKeys"),
    windows_sys::w!("GalGear.GameInput.FontKeys"),
];
pub(super) const INPUT_MESSAGE: u32 = WM_APP + 74;
pub(super) const ERROR_MESSAGE: u32 = WM_APP + 76;

fn elevated(process: HANDLE) -> Result<bool, String> {
    let mut token = ptr::null_mut();
    // SAFETY: process 是当前调用期间有效的查询句柄；成功时独占持有返回的 token。
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error());
    }
    // SAFETY: OpenProcessToken 将有效句柄的所有权交给调用方。
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut value = TOKEN_ELEVATION::default();
    let mut length = 0;
    // SAFETY: 输出缓冲大小和 TokenElevation 对应的结构一致。
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut value as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    } == 0
    {
        return Err(last_error());
    }
    Ok(value.TokenIsElevated != 0)
}

pub(super) fn required(window: HWND) -> Result<bool, String> {
    // SAFETY: 伪句柄只借用；窗口所属进程查询句柄由本次调用管理。
    unsafe {
        if elevated(GetCurrentProcess())? {
            return Ok(false);
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(last_error());
        }
        let process = OwnedHandle::from_raw_handle(process);
        elevated(process.as_raw_handle())
    }
}

#[derive(Default)]
pub(super) struct Relay {
    generation: usize,
    configuration: Option<(usize, [usize; 2])>,
    pending: Option<mpsc::Receiver<Result<OwnedHandle, Message>>>,
    child: Option<OwnedHandle>,
    attempted: bool,
}

impl Relay {
    /// 发布已由主进程验证的前台游戏和组合键。关闭时首先撤销代数，不等待 UAC。
    pub(super) fn configure(
        &mut self,
        window: HWND,
        target: Option<(HWND, [&[u16]; 2])>,
    ) -> Result<(), Message> {
        let configuration = target.map(|(target, keys)| (target as usize, keys.map(encode)));
        if configuration != self.configuration {
            self.clear(window);
            if let Some((target, keys)) = configuration {
                self.generation = self.generation.wrapping_add(1).max(1);
                // SAFETY: 本线程拥有 window；属性仅保存整数/系统 HWND，不传递内存指针。
                let ok = unsafe {
                    SetPropW(window, TARGET, target as HANDLE) != 0
                        && (keys[0] == 0 || SetPropW(window, KEYS[0], keys[0] as HANDLE) != 0)
                        && (keys[1] == 0 || SetPropW(window, KEYS[1], keys[1] as HANDLE) != 0)
                        && SetPropW(window, GENERATION, self.generation as HANDLE) != 0
                };
                if !ok {
                    self.clear(window);
                    return Err(Message::detail("game_input.error", last_error()));
                }
            }
            self.configuration = configuration;
        }
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    self.child = Some(result?);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    return Err(Message::detail(
                        "game_input.error",
                        "Input helper stopped during startup",
                    ));
                }
            }
        }
        if self.child.as_ref().is_some_and(|child| {
            // SAFETY: child 在本次查询期间存活；零超时不阻塞工作线程。
            unsafe { WaitForSingleObject(child.as_raw_handle(), 0) != WAIT_TIMEOUT }
        }) {
            self.child = None;
            return Err(Message::detail("game_input.error", "Input helper exited"));
        }
        if configuration.is_some() && !self.attempted {
            self.attempted = true;
            let (sender, receiver) = mpsc::sync_channel(1);
            let window = window as usize;
            std::thread::Builder::new()
                .name("game-input-elevation".into())
                .spawn(move || {
                    let _ = sender.send(launch(window));
                })
                .map_err(|e| Message::detail("game_input.error", e))?;
            self.pending = Some(receiver);
        }
        Ok(())
    }

    pub(super) fn accepts(&self, message: &MSG) -> Option<usize> {
        let slot = message.message.checked_sub(INPUT_MESSAGE)? as usize;
        (slot < 2
            && message.wParam == self.generation
            && self
                .configuration
                .is_some_and(|(target, keys)| message.lParam as usize == target && keys[slot] != 0))
        .then_some(slot)
    }

    pub(super) fn clear(&mut self, window: HWND) {
        // SAFETY: 只移除本模块在自有窗口上设置的属性。
        unsafe {
            RemovePropW(window, GENERATION);
            RemovePropW(window, KEYS[0]);
            RemovePropW(window, KEYS[1]);
            RemovePropW(window, TARGET);
        }
        self.configuration = None;
    }

    pub(super) fn retry(&mut self) {
        if self.pending.is_none() && self.child.is_none() {
            self.attempted = false;
        }
    }
}

fn encode(keys: &[u16]) -> usize {
    keys.iter().fold(0, |value, key| {
        value
            | match key {
                0x10 => 0x100,
                0x11 => 0x200,
                0x12 => 0x400,
                0x5b => 0x800,
                key => usize::from(*key),
            }
    })
}

fn decode(value: usize) -> Option<Vec<u16>> {
    let key = u16::try_from(value & 255).ok()?;
    if value > 0xfff
        || value & 0xf00 == 0
        || galgear_core::runtime::hotkey_key_name(key).is_none()
        || matches!(key, 0x10..=0x12 | 0x5b | 0x5c)
    {
        return None;
    }
    let mut keys = Vec::with_capacity(5);
    for (flag, key) in [(0x100, 0x10), (0x200, 0x11), (0x400, 0x12), (0x800, 0x5b)] {
        if value & flag != 0 {
            keys.push(key);
        }
    }
    keys.push(key);
    Some(keys)
}

fn launch(window: usize) -> Result<OwnedHandle, Message> {
    let error = |error| Message::detail("game_input.error", error);
    let executable = std::env::current_exe().map_err(error)?;
    let file: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let parameters: Vec<u16> = format!("{ARGUMENT} {} {window}", std::process::id())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: 新建专用线程无既有 COM apartment；Shell 调用期间所有字符串有效。
    unsafe {
        let result = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
        if result < 0 {
            return Err(Message::detail("game_input.error", result));
        }
        let mut request = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: windows_sys::w!("runas"),
            lpFile: file.as_ptr(),
            lpParameters: parameters.as_ptr(),
            nShow: SW_HIDE,
            ..Default::default()
        };
        let success = ShellExecuteExW(&mut request);
        let failure = std::io::Error::last_os_error();
        CoUninitialize();
        if success == 0 {
            return Err(if failure.raw_os_error() == Some(ERROR_CANCELLED as i32) {
                Message::new("game_input.cancelled")
            } else {
                error(failure)
            });
        }
        if request.hProcess.is_null() {
            return Err(Message::detail(
                "game_input.error",
                "Missing process handle",
            ));
        }
        Ok(OwnedHandle::from_raw_handle(request.hProcess))
    }
}

/// 必须在创建 Tauri 之前分派；只接受本程序父进程及其消息窗口，不接受文件/命令参数。
pub(crate) fn run_helper() -> bool {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(OsStr::new(ARGUMENT)) {
        return false;
    }
    let owner = arguments
        .next()
        .and_then(|value| value.to_str()?.parse::<u32>().ok());
    let window = arguments
        .next()
        .and_then(|value| value.to_str()?.parse::<usize>().ok());
    if let (Some(owner), Some(window), None) = (owner, window, arguments.next())
        && let Err(error) = receive(owner, window as HWND)
    {
        eprintln!("Game input: {error}");
    }
    true
}

fn receive(owner: u32, parent: HWND) -> Result<(), String> {
    // SAFETY: 每个句柄均检查失败，并通过 RAII 或函数末尾销毁；跨进程只传整数标识。
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(parent, &mut pid);
        if owner == 0 || owner == std::process::id() || pid != owner {
            return Err("Invalid input owner".into());
        }
        let process = OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            owner,
        );
        if process.is_null() {
            return Err(last_error());
        }
        let process = OwnedHandle::from_raw_handle(process);
        let mut path = vec![0; 32768];
        let mut length = path.len() as u32;
        if QueryFullProcessImageNameW(process.as_raw_handle(), 0, path.as_mut_ptr(), &mut length)
            == 0
        {
            return Err(last_error());
        }
        let owner_path =
            std::path::PathBuf::from(std::ffi::OsString::from_wide(&path[..length as usize]));
        if owner_path.canonicalize().map_err(|e| e.to_string())?
            != std::env::current_exe()
                .and_then(|p| p.canonicalize())
                .map_err(|e| e.to_string())?
        {
            return Err("Input owner executable mismatch".into());
        }
        let window = CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("GalGear elevated game input"),
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
        if window.is_null() {
            return Err(last_error());
        }
        let result = receive_loop(process.as_raw_handle(), parent, window);
        if result.is_err() {
            PostMessageW(parent, ERROR_MESSAGE, 0, 0);
        }
        DestroyWindow(window);
        result
    }
}

fn receive_loop(owner: HANDLE, parent: HWND, window: HWND) -> Result<(), String> {
    let mut keyboard: Option<Keyboard> = None;
    let mut configuration = (0, ptr::null_mut(), [0; 2]);
    let mut keys: [Vec<u16>; 2] = std::array::from_fn(|_| Vec::new());
    loop {
        // SAFETY: owner 句柄由调用方持有；窗口属性仅作值查询，从不解引用其内容。
        unsafe {
            if WaitForSingleObject(owner, 0) != WAIT_TIMEOUT || IsWindow(parent) == 0 {
                break;
            }
            let generation = GetPropW(parent, GENERATION) as usize;
            let target = GetPropW(parent, TARGET);
            let encoded = KEYS.map(|key| GetPropW(parent, key) as usize);
            let active = generation != 0
                && generation == GetPropW(parent, GENERATION) as usize
                && magpie::source(GetForegroundWindow()) == target;
            let next = if active {
                (generation, target, encoded)
            } else {
                (0, ptr::null_mut(), [0; 2])
            };
            if next != configuration {
                keyboard = None;
                keys = encoded.map(|encoded| decode(encoded).unwrap_or_default());
                if active && keys.iter().any(|key| !key.is_empty()) {
                    keyboard = Some(Keyboard::start(window)?);
                }
                configuration = next;
            }
            let mut message = MSG::default();
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.hwnd == window
                    && message.message == WM_INPUT
                    && let Some(input) = &mut keyboard
                    && let Some(slot) = input.pressed_many(message.lParam, &[&keys[0], &keys[1]])
                    && magpie::source(GetForegroundWindow()) == configuration.1
                    && GetPropW(parent, GENERATION) as usize == configuration.0
                {
                    PostMessageW(
                        parent,
                        INPUT_MESSAGE + slot as u32,
                        configuration.0,
                        configuration.1 as isize,
                    );
                }
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

use std::os::windows::ffi::OsStringExt;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_protocol_roundtrips_and_rejects_invalid_keys() {
        for hotkey in ["Shift+S", "Ctrl+Alt+F10", "Win+Shift+F9"] {
            let keys = galgear_core::runtime::hotkey_keys(hotkey).unwrap();
            let decoded = decode(encode(&keys)).unwrap();
            assert_eq!(encode(&decoded), encode(&keys));
        }
        for value in [0, 0x53, 0x110, 0x1ff, 0x10000] {
            assert!(decode(value).is_none());
        }
    }
    #[test]
    fn relay_rejects_notifications_from_old_configuration() {
        let relay = Relay {
            generation: 7,
            configuration: Some((42, [0x153, 0x154])),
            ..Default::default()
        };
        let mut message = MSG {
            message: INPUT_MESSAGE,
            wParam: 7,
            lParam: 42,
            ..Default::default()
        };
        assert_eq!(relay.accepts(&message), Some(0));
        message.message += 1;
        assert_eq!(relay.accepts(&message), Some(1));
        message.wParam = 6;
        assert_eq!(relay.accepts(&message), None);
        message.wParam = 7;
        message.lParam = 41;
        assert_eq!(relay.accepts(&message), None);
    }
}
