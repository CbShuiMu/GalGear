//! 游戏字体面板服务：窗口与快捷键在专用线程，Hook 通信在后台执行。
#[cfg(windows)]
mod client;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod shortcut;
use galgear_i18n::Message;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::Manager;

pub(crate) fn validate_hotkey(value: &str, current: &str) -> Result<String, Message> {
    let canonical = galgear_core::runtime::canonical_hotkey(value)
        .map_err(|_| Message::new("shortcut.invalid"))?;
    #[cfg(windows)]
    if canonical != current {
        shortcut::Shortcut::parse(&canonical)?.probe()?;
    }
    #[cfg(not(windows))]
    let _ = current;
    Ok(canonical)
}

pub(crate) struct Service(Arc<AtomicBool>);
impl Service {
    pub(crate) fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
pub(crate) fn start(
    app: tauri::AppHandle,
    #[cfg(windows)] input: crate::game_input::Subscription,
) -> Result<(), Message> {
    let stop = Arc::new(AtomicBool::new(false));
    app.manage(Service(stop.clone()));
    #[cfg(windows)]
    std::thread::Builder::new()
        .name("galgear-font-panel".into())
        .spawn(move || worker(app, stop, input))
        .map_err(|e| Message::detail("font.error", e))?;
    Ok(())
}

#[cfg(windows)]
fn worker(
    app: tauri::AppHandle,
    stop: Arc<AtomicBool>,
    mut input: crate::game_input::Subscription,
) {
    use crate::{
        game_window,
        state::{AppState, snapshot},
    };
    use galgear_core::font_tools::Profile;
    use std::{ptr, sync::mpsc, time::Duration};
    use tauri::Emitter;
    use windows_sys::Win32::UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
        Input::KeyboardAndMouse::UnregisterHotKey,
        WindowsAndMessaging::{
            DispatchMessageW, GetForegroundWindow, GetWindowThreadProcessId, PM_REMOVE,
            PeekMessageW, TranslateMessage, WM_HOTKEY,
        },
    };
    let state = app.state::<AppState>();
    let mut active = None;
    let mut pending: Option<Pending> = None;
    let mut completed: Option<(usize, String, Message)> = None;
    // SAFETY: 专用 UI 线程的 DPI 上下文与窗口生命周期一致。
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut panel = None::<panel::Panel>;
        let mut hotkey = false;
        let mut attempted = false;
        let mut shortcut_name = String::new();
        let mut shortcut = None::<shortcut::Shortcut>;
        let mut tick = 0;
        let mut last_foreground = ptr::null_mut();
        let mut trigger = crate::game_input::Trigger::default();
        while !stop.load(Ordering::Relaxed) {
            // 设置按已有快照入口读取；切换组合时撤销旧注册及迟到按键。
            if tick % 10 == 0
                && let Ok(settings) = snapshot(&state)
                && settings.font_hotkey != shortcut_name
            {
                if hotkey {
                    UnregisterHotKey(ptr::null_mut(), 0x4746);
                    hotkey = false;
                }
                if let Err(error) = input.configure(None) {
                    let _ = app.emit("tool-failed", error);
                }
                if let Err(error) = input.retry() {
                    let _ = app.emit("tool-failed", error);
                }
                shortcut_name = settings.font_hotkey;
                shortcut = match shortcut::Shortcut::parse(&shortcut_name) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        let _ = app.emit("tool-failed", error);
                        None
                    }
                };
                attempted = false;
                trigger = crate::game_input::Trigger::default();
                panel = None;
            }
            let foreground = GetForegroundWindow();
            let panel_focused = panel.as_ref().is_some_and(|p| p.owns(foreground));
            if !panel_focused && (foreground != last_foreground || tick % 10 == 0) {
                let target = game_window::foreground_game(&state).and_then(|(hwnd, game)| {
                    let (current, path) = game_window::foreground()?;
                    (current == hwnd).then_some((hwnd, game, path))
                });
                let changed = active.as_ref().map(|(hwnd, _, _)| *hwnd)
                    != target.as_ref().map(|(hwnd, _, _)| *hwnd);
                if changed {
                    panel = None;
                    attempted = false;
                    trigger = crate::game_input::Trigger::default();
                }
                active = target;
            }
            last_foreground = foreground;
            if active.is_some() && !hotkey && !attempted && shortcut.is_some() {
                attempted = true;
                if let Some(shortcut) = &shortcut {
                    match shortcut.register(0x4746) {
                        Ok(()) => hotkey = true,
                        Err(error) => {
                            let _ = app.emit("tool-failed", error);
                        }
                    }
                }
            } else if active.is_none() && hotkey {
                UnregisterHotKey(ptr::null_mut(), 0x4746);
                hotkey = false;
            }
            if let Err(error) =
                input.configure(shortcut.as_ref().filter(|_| hotkey).and_then(|shortcut| {
                    active
                        .as_ref()
                        .map(|(target, _, _)| (*target, shortcut.keys.as_slice()))
                }))
            {
                let _ = app.emit("tool-failed", error);
            }
            let mut requested = match input.pressed() {
                Ok(pressed) => pressed,
                Err(error) => {
                    let _ = app.emit("tool-failed", error);
                    false
                }
            };
            if let Some(result) = pending
                .as_ref()
                .and_then(|work| match work.receiver.try_recv() {
                    Ok(value) => Some(value),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err(Message::new("font.worker_failed")))
                    }
                    Err(_) => None,
                })
                && let Some(work) = pending.take()
            {
                // UAC 会暂时切走前台并关闭面板；保留一份结果供同一游戏窗口重新打开时显示。
                completed = Some((
                    work.window,
                    work.game_id,
                    result.unwrap_or_else(|message| message),
                ));
            }
            let mut msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_HOTKEY && msg.wParam == 0x4746 {
                    requested = true;
                } else if !panel.as_ref().is_some_and(|p| p.message(&mut msg)) {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if hotkey && trigger.observe(requested, std::time::Instant::now()) {
                // 热键可能在前台轮询间隙到达；不能给已经切走的游戏打开面板。
                let current = GetForegroundWindow();
                if !panel.as_ref().is_some_and(|p| p.owns(current))
                    && active.as_ref().is_none_or(|(target, _, _)| {
                        game_window::magpie::source(current) != *target
                    })
                {
                    requested = false;
                }
                if requested {
                    if let Some(p) = panel.as_mut() {
                        p.toggle();
                    } else if let Some((target, game, _)) = &active
                        && let Ok(settings) = snapshot(&state)
                    {
                        match panel::Panel::new(*target, &game.title, &settings) {
                            Ok(mut p) => {
                                p.set_profile(
                                    settings
                                        .game_fonts
                                        .get(&game.id)
                                        .unwrap_or(&Profile::default()),
                                );
                                p.toggle();
                                panel = Some(p);
                            }
                            Err(error) => {
                                let _ = app.emit("tool-failed", error);
                            }
                        }
                    }
                }
            }
            if let Some(p) = panel.as_mut() {
                p.follow();
                if let Some((hwnd, game_id, message)) = &completed
                    && active.as_ref().is_some_and(|(target, game, _)| {
                        *target as usize == *hwnd && game.id == *game_id
                    })
                {
                    p.status(&message.render(p.language()));
                    completed = None;
                }
                if let Some(action) = p.action() {
                    if pending.is_some() {
                        p.status(galgear_i18n::tr(p.language(), "font.busy"));
                    } else if let Some((target, game, executable)) = &active {
                        let request = match action {
                            panel::Action::Apply => p.profile(),
                            panel::Action::Reset => {
                                p.set_profile(&Profile::default());
                                Ok(Profile::default())
                            }
                        };
                        match request {
                            Err(error) => p.status(&error.render(p.language())),
                            Ok(request) => {
                                completed = None;
                                p.status(galgear_i18n::tr(p.language(), "font.working"));
                                let (sender, receiver) = mpsc::channel();
                                let app = app.clone();
                                let id = game.id.clone();
                                let executable = executable.clone();
                                let mut pid = 0;
                                GetWindowThreadProcessId(*target, &mut pid);
                                std::thread::spawn(move || {
                                    let result = (|| {
                                        let profile = request;
                                        client::apply(pid, &executable, &profile)?;
                                        crate::preferences::update(&app.state::<AppState>(), |s| {
                                            s.game_fonts.insert(id, profile);
                                            Ok(())
                                        })
                                        .map_err(|e| {
                                            Message::new("font.save_failed").arg(
                                                "detail",
                                                e.render(galgear_i18n::Language::English),
                                            )
                                        })?;
                                        Ok(Message::new("font.submitted"))
                                    })();
                                    let _ = sender.send(result);
                                });
                                pending = Some(Pending {
                                    window: *target as usize,
                                    game_id: game.id.clone(),
                                    receiver,
                                });
                            }
                        }
                    }
                }
            }
            tick += 1;
            std::thread::sleep(Duration::from_millis(30));
        }
        drop(input);
        drop(panel);
        if hotkey {
            UnregisterHotKey(ptr::null_mut(), 0x4746);
        }
        if !previous.is_null() {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}

#[cfg(windows)]
struct Pending {
    window: usize,
    game_id: String,
    receiver: std::sync::mpsc::Receiver<Result<Message, Message>>,
}
