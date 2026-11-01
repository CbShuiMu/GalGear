//! 全局截图快捷键及保存编排；工作线程拥有热键、剪贴板窗口和采集资源。
use crate::state::{AppState, find_game, snapshot};
use base64::Engine;
use galgear_core::{Settings, screenshots};
use galgear_i18n::Message;
use std::{path::Path, sync::mpsc};
use tauri::{Emitter, Manager};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::elevated::run_helper as run_input_helper;

type SaveSettings = Box<dyn FnOnce(&AppState) -> Result<Settings, Message> + Send>;

enum Command {
    Configure {
        hotkey: Option<String>,
        save: SaveSettings,
        reply: mpsc::SyncSender<Result<Settings, Message>>,
    },
    Stop,
}
pub(crate) struct Service(mpsc::Sender<Command>);
impl Service {
    /// 在热键所属线程校验后执行设置提交；失败时不撤销原注册。
    /// 通过提交回调保持截图平台模块不依赖工具配置模块。
    pub(crate) fn configure(
        &self,
        hotkey: Option<String>,
        save: impl FnOnce(&AppState) -> Result<Settings, Message> + Send + 'static,
    ) -> Result<Settings, Message> {
        let (reply, result) = mpsc::sync_channel(1);
        self.0
            .send(Command::Configure {
                hotkey,
                save: Box::new(save),
                reply,
            })
            .map_err(|e| Message::detail("screenshot.error", e))?;
        result
            .recv()
            .map_err(|e| Message::detail("screenshot.error", e))?
    }
    pub(crate) fn stop(&self) {
        let _ = self.0.send(Command::Stop);
    }
}

pub(crate) fn start(app: tauri::AppHandle) -> Result<(), Message> {
    let (sender, receiver) = mpsc::channel();
    app.manage(Service(sender));
    std::thread::Builder::new()
        .name("galgear-screenshots".into())
        .spawn(move || worker(app, receiver))
        .map_err(|e| Message::detail("screenshot.error", e))?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn open_screenshot_folder(
    id: String,
    app: tauri::AppHandle,
) -> Result<(), Message> {
    tauri::async_runtime::spawn_blocking(move || {
        let game = find_game(&app.state::<AppState>(), &id)?;
        crate::folders::open(&screenshots::folder(Path::new(&game.folder), true)?)
    })
    .await
    .map_err(|e| Message::detail("screenshot.error", e))?
}

#[derive(serde::Serialize)]
pub(crate) struct Preview {
    name: String,
    image: String,
}

#[tauri::command]
pub(crate) async fn latest_screenshot(
    id: String,
    app: tauri::AppHandle,
) -> Result<Option<Preview>, Message> {
    tauri::async_runtime::spawn_blocking(move || {
        let game = find_game(&app.state::<AppState>(), &id)?;
        screenshots::latest_preview(Path::new(&game.folder)).map(|preview| {
            preview.map(|(name, bytes)| Preview {
                name,
                image: format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ),
            })
        })
    })
    .await
    .map_err(|e| Message::detail("screenshot.error", e))?
}

#[cfg(windows)]
fn foreground_game(
    state: &AppState,
) -> Option<(windows_sys::Win32::Foundation::HWND, galgear_core::Game)> {
    let (window, executable) = windows::foreground()?;
    // 只在复制目录元数据时持锁；后续文件系统访问不持有目录锁。
    let games = state.catalog.lock().ok()?.games.clone();
    let game = games
        .into_iter()
        .filter(|game| {
            game.executable
                .as_ref()
                .is_some_and(|exe| Path::new(exe).canonicalize().is_ok_and(|p| p == executable))
                || Path::new(&game.folder)
                    .canonicalize()
                    .is_ok_and(|root| executable.starts_with(root))
        })
        .max_by_key(|game| game.folder.len())?;
    Some((window, game))
}

#[cfg(windows)]
fn worker(app: tauri::AppHandle, receiver: mpsc::Receiver<Command>) {
    use std::{
        ptr,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    // SAFETY: 系统 STATIC 类创建本线程专用的不可见窗口；结束前销毁。
    let window = unsafe {
        CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("GalGear screenshots"),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if window.is_null() {
        return;
    }
    let state = app.state::<AppState>();
    let mut registered = false;
    let mut target = None;
    let mut attempted = None;
    let mut last_window = ptr::null_mut();
    let mut refreshed = Instant::now() - Duration::from_secs(2);
    let mut notification = windows::Notification::default();
    let mut trigger = Trigger::default();
    let mut keyboard: Option<windows::Keyboard> = None;
    let mut keys = Vec::new();
    let mut relay = windows::elevated::Relay::default();
    let mut relay_required = false;
    loop {
        notification.expire();
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Command::Configure {
                hotkey,
                save,
                reply,
            }) => {
                let result = (|| {
                    let current = snapshot(&state)?;
                    // 先用另一 ID 验证新组合。失败时保留原注册和持久化设置。
                    if let Some(hotkey) = &hotkey
                        && (!registered || current.screenshot_hotkey != *hotkey)
                    {
                        windows::register(window, 2, hotkey)?;
                        windows::unregister(window, 2);
                    }
                    let saved = save(&state)?;
                    if registered {
                        windows::unregister(window, 1);
                        registered = false;
                    }
                    attempted = None;
                    trigger = Trigger::default();
                    keyboard = None;
                    relay.clear(window);
                    relay.retry();
                    last_window = ptr::null_mut();
                    refreshed = Instant::now() - Duration::from_secs(2);
                    Ok(saved)
                })();
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // SAFETY: 仅查询系统拥有的前台窗口句柄，不保存跨线程裸指针。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != last_window || refreshed.elapsed() >= Duration::from_secs(1) {
            let foreground_changed = foreground != last_window;
            if foreground_changed {
                attempted = None;
                trigger = Trigger::default();
                if let Some(keyboard) = &mut keyboard {
                    keyboard.clear();
                }
            }
            last_window = foreground;
            refreshed = Instant::now();
            target = foreground_game(&state);
            relay_required = if target.is_some() {
                match windows::elevated::required(foreground) {
                    Ok(required) => required,
                    Err(detail) => {
                        if foreground_changed {
                            let error = Message::detail("screenshot.input_error", detail);
                            let _ = app.emit("tool-failed", &error);
                            show_notice(&mut notification, foreground, &error, true, &state);
                        }
                        false
                    }
                }
            } else {
                false
            };
            let enabled = snapshot(&state).is_ok_and(|s| s.screenshot_enabled);
            if registered && (!enabled || target.is_none()) {
                windows::unregister(window, 1);
                registered = false;
                keyboard = None;
            }
            if target.is_none() {
                attempted = None;
            }
            if enabled
                && target.is_some()
                && !registered
                && attempted != target.as_ref().map(|(_, game)| game.id.clone())
            {
                attempted = target.as_ref().map(|(_, game)| game.id.clone());
                let result = snapshot(&state)
                    .and_then(|s| windows::register(window, 1, &s.screenshot_hotkey));
                match result {
                    Ok(()) => {
                        registered = true;
                        keys = snapshot(&state)
                            .ok()
                            .and_then(|s| {
                                galgear_core::runtime::hotkey_keys(&s.screenshot_hotkey).ok()
                            })
                            .unwrap_or_default();
                        match windows::Keyboard::start(window) {
                            Ok(input) => keyboard = Some(input),
                            Err(error) => {
                                let error = Message::detail("screenshot.hotkey_error", error);
                                let _ = app.emit("tool-failed", &error);
                                show_notice(&mut notification, foreground, &error, true, &state);
                            }
                        }
                    }
                    Err(error) => {
                        let _ = app.emit("tool-failed", &error);
                        show_notice(&mut notification, foreground, &error, true, &state);
                    }
                }
            }
        }
        let relay_target = if registered && relay_required {
            target
                .as_ref()
                .map(|(window, _)| (*window, keys.as_slice()))
        } else {
            None
        };
        if let Err(error) = relay.configure(window, relay_target) {
            let _ = app.emit("tool-failed", &error);
            show_notice(&mut notification, foreground, &error, true, &state);
        }
        let mut hotkey_message = false;
        // SAFETY: 消息结构有效，仅在创建窗口的线程消费消息。
        let mut message = unsafe { std::mem::zeroed() };
        // SAFETY: message 为本线程有效缓冲，读取本线程消息队列。
        while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.hwnd == window && relay.accepts(&message) {
                hotkey_message = true;
            } else if message.hwnd == window && message.message == windows::elevated::ERROR_MESSAGE
            {
                let error = Message::detail(
                    "screenshot.input_error",
                    "Input helper could not receive keyboard events",
                );
                let _ = app.emit("tool-failed", &error);
                show_notice(&mut notification, foreground, &error, true, &state);
            } else if message.hwnd == window && message.message == WM_INPUT {
                if let Some(keyboard) = &mut keyboard {
                    hotkey_message |= keyboard.pressed(message.lParam, &keys);
                }
                // SAFETY: 系统默认过程负责 WM_INPUT 清理，数据已在上面读取。
                unsafe {
                    DispatchMessageW(&message);
                }
            } else if message.hwnd == window && message.message == WM_HOTKEY && message.wParam == 1
            {
                hotkey_message = true;
            } else {
                // SAFETY: 标准消息循环，message 来自本线程 PeekMessageW。
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        // 两种输入通知共用短间隔去重；Raw Input 自身过滤长按重复。
        if registered
            && trigger.observe(hotkey_message, Instant::now())
            && let Some((game_window, game)) = &target
        {
            notification.hide();
            let result = capture_game(&app, window, *game_window, game);
            let (notice, error) = match result {
                Ok(notice) => (notice, false),
                Err(error) => (error, true),
            };
            show_notice(&mut notification, *game_window, &notice, error, &state);
            let _ = app.emit(
                "screenshot-completed",
                serde_json::json!({"notice":notice,"error":error}),
            );
        }
    }
    drop(keyboard);
    relay.clear(window);
    if registered {
        windows::unregister(window, 1);
    }
    // SAFETY: 所有截图和消息处理已结束，释放本线程创建的窗口。
    unsafe {
        DestroyWindow(window);
    }
}

#[cfg(windows)]
fn capture_game(
    app: &tauri::AppHandle,
    clipboard: windows_sys::Win32::Foundation::HWND,
    window: windows_sys::Win32::Foundation::HWND,
    game: &galgear_core::Game,
) -> Result<Message, Message> {
    let frame = windows::capture(window).map_err(|e| Message::detail("screenshot.error", e))?;
    let folder = screenshots::folder(Path::new(&game.folder), true)?;
    let path = screenshots::save_png(&folder, frame.width, frame.height, &frame.rgba)?;
    let _ = app.emit("screenshot-updated", &game.id);
    windows::copy(&frame, clipboard, &path).map_err(|e| {
        Message::detail("screenshot.clipboard_error", e).arg("path", path.display())
    })?;
    Ok(Message::new("screenshot.saved"))
}

#[cfg(windows)]
fn show_notice(
    notification: &mut windows::Notification,
    target: windows_sys::Win32::Foundation::HWND,
    notice: &Message,
    error: bool,
    state: &AppState,
) {
    if let Ok(settings) = snapshot(state) {
        notification.show(
            target,
            &notice.render(settings.language),
            &settings.theme,
            &settings.color_theme,
            error,
        );
    }
}

#[cfg(any(windows, test))]
#[derive(Default)]
struct Trigger {
    last: Option<std::time::Instant>,
}
#[cfg(any(windows, test))]
impl Trigger {
    fn observe(&mut self, pressed: bool, now: std::time::Instant) -> bool {
        // WM_HOTKEY 和 WM_INPUT 可能跨消息轮次到达，编码期间也可能已松键。
        if pressed
            && self
                .last
                .is_none_or(|last| now.duration_since(last).as_millis() >= 250)
        {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Trigger;
    use std::time::{Duration, Instant};
    #[test]
    fn duplicate_input_notifications_only_capture_once() {
        let now = Instant::now();
        let mut trigger = Trigger::default();
        assert!(trigger.observe(true, now));
        assert!(!trigger.observe(true, now + Duration::from_millis(20)));
        assert!(!trigger.observe(false, now + Duration::from_millis(300)));
        assert!(trigger.observe(true, now + Duration::from_millis(400)));
    }
}

#[cfg(not(windows))]
fn worker(app: tauri::AppHandle, receiver: mpsc::Receiver<Command>) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Stop => break,
            Command::Configure { save, reply, .. } => {
                let _ = reply.send(save(&app.state::<AppState>()));
            }
        }
    }
}
