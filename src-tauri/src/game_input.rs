//! 游戏快捷键共用一个 Raw Input 接收窗口及管理员输入辅助进程。
//! 订阅者只提交目标与组合键；消息带配置代数，失焦/重配后不投递迟到输入。
mod elevated;
mod keyboard;
mod trigger;
pub(crate) use elevated::run_helper;
use galgear_i18n::Message;
#[cfg(test)]
pub(crate) use keyboard::Keyboard;
use std::{ptr, sync::mpsc, time::Duration};
pub(crate) use trigger::Trigger;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow, HWND_MESSAGE, MSG,
        PM_REMOVE, PeekMessageW, WM_INPUT,
    },
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Binding {
    target: usize,
    keys: Vec<u16>,
    generation: usize,
}
enum Command {
    Configure(usize, Binding),
    Retry,
    Stop,
}
enum Event {
    Pressed { target: usize, generation: usize },
    Error(Message),
}
pub(crate) struct Service(mpsc::Sender<Command>);
impl Service {
    pub(crate) fn stop(&self) {
        let _ = self.0.send(Command::Stop);
    }
}
pub(crate) struct Subscription {
    slot: usize,
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    binding: Binding,
}
impl Subscription {
    pub(crate) fn retry(&self) -> Result<(), Message> {
        self.commands.send(Command::Retry).map_err(error)
    }
    /// None 取消接收；仅在配置实际变化时发送，避免清空正在按住的修饰键。
    pub(crate) fn configure(&mut self, target: Option<(HWND, &[u16])>) -> Result<(), Message> {
        let (target, keys) = target.map_or((0, &[][..]), |(w, keys)| (w as usize, keys));
        if target == self.binding.target && keys == self.binding.keys {
            return Ok(());
        }
        self.binding = Binding {
            target,
            keys: keys.into(),
            generation: self.binding.generation.wrapping_add(1).max(1),
        };
        self.commands
            .send(Command::Configure(self.slot, self.binding.clone()))
            .map_err(error)
    }
    /// 仅取当前订阅的事件；回调执行前仍由功能模块核对前台窗口。
    pub(crate) fn pressed(&self) -> Result<bool, Message> {
        let mut pressed = false;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Pressed { target, generation } => {
                    pressed |= self.binding.target != 0
                        && target == self.binding.target
                        && generation == self.binding.generation;
                }
                Event::Error(error) => return Err(error),
            }
        }
        Ok(pressed)
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.configure(None);
    }
}

pub(crate) fn start() -> Result<(Service, [Subscription; 2]), Message> {
    let (commands, receiver) = mpsc::channel();
    let channels: [_; 2] = std::array::from_fn(|_| mpsc::channel());
    let [(first, a), (second, b)] = channels;
    std::thread::Builder::new()
        .name("galgear-game-input".into())
        .spawn(move || worker(receiver, [first, second]))
        .map_err(error)?;
    let subscriptions = [(0, a), (1, b)].map(|(slot, events)| Subscription {
        slot,
        commands: commands.clone(),
        events,
        binding: Binding::default(),
    });
    // 固定两个调用方：截图、字体面板；不建立动态全局快捷键注册表。
    Ok((Service(commands), subscriptions))
}
fn error(detail: impl ToString) -> Message {
    Message::detail("game_input.error", detail)
}
fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}

fn worker(commands: mpsc::Receiver<Command>, events: [mpsc::Sender<Event>; 2]) {
    let report = |message: Message| {
        for event in &events {
            let _ = event.send(Event::Error(message.clone()));
        }
    };
    // SAFETY: 消息窗口仅由本线程创建、消费和销毁，不显示或抢焦点。
    let window = unsafe {
        CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("GalGear game input"),
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
        report(error(last_error()));
        return;
    }
    let mut bindings: [Binding; 2] = std::array::from_fn(|_| Binding::default());
    let mut keyboard = None::<keyboard::Keyboard>;
    let mut relay = elevated::Relay::default();
    let mut last_target = 0;
    let mut relay_required = false;
    'receive: loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Stop => break 'receive,
                Command::Configure(slot, binding) => {
                    bindings[slot] = binding;
                }
                Command::Retry => relay.retry(),
            }
        }
        // SAFETY: 前台句柄只作身份比较；源游戏关联不解引用外部内存。
        let foreground = crate::game_window::magpie::source(unsafe { GetForegroundWindow() });
        let target = if bindings
            .iter()
            .any(|b| b.target != 0 && b.target == foreground as usize)
        {
            foreground as usize
        } else {
            0
        };
        if target != last_target {
            keyboard = None;
            relay_required = false;
            if target != 0 {
                match keyboard::Keyboard::start(window) {
                    Ok(input) => keyboard = Some(input),
                    Err(detail) => report(error(detail)),
                }
                match elevated::required(foreground) {
                    Ok(required) => relay_required = required,
                    Err(detail) => report(error(detail)),
                }
            }
            last_target = target;
        }
        let keys: [&[u16]; 2] = std::array::from_fn(|i| {
            if target != 0 && bindings[i].target == target {
                bindings[i].keys.as_slice()
            } else {
                &[]
            }
        });
        if let Err(error) = relay.configure(
            window,
            (target != 0 && relay_required).then_some((foreground, keys)),
        ) {
            report(error);
        }
        let mut message = MSG::default();
        // SAFETY: 只处理本线程窗口的消息；WM_INPUT 在默认处理清理前读取。
        unsafe {
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                let slot = if message.hwnd == window && message.message == WM_INPUT {
                    keyboard
                        .as_mut()
                        .and_then(|input| input.pressed_many(message.lParam, &keys))
                } else if message.hwnd == window {
                    relay.accepts(&message)
                } else {
                    None
                };
                if let Some(slot) = slot
                    && target != 0
                    && bindings[slot].target == target
                    && crate::game_window::magpie::source(GetForegroundWindow()) as usize == target
                {
                    let _ = events[slot].send(Event::Pressed {
                        target,
                        generation: bindings[slot].generation,
                    });
                }
                if message.hwnd == window && message.message == elevated::ERROR_MESSAGE {
                    report(error("Input helper could not receive keyboard events"));
                }
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(keyboard);
    relay.clear(window);
    // SAFETY: 登记及辅助进程配置已撤销，再释放本线程窗口。
    unsafe {
        DestroyWindow(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscription_rejects_late_events_after_retarget_or_disable() {
        let (commands, _receiver) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let mut input = Subscription {
            slot: 1,
            commands,
            events: receiver,
            binding: Binding::default(),
        };
        input
            .configure(Some((42usize as HWND, &[0x10, 0x54])))
            .unwrap();
        events
            .send(Event::Pressed {
                target: 42,
                generation: 1,
            })
            .unwrap();
        input
            .configure(Some((43usize as HWND, &[0x10, 0x54])))
            .unwrap();
        assert!(!input.pressed().unwrap());
        events
            .send(Event::Pressed {
                target: 43,
                generation: 2,
            })
            .unwrap();
        assert!(input.pressed().unwrap());
        events
            .send(Event::Pressed {
                target: 43,
                generation: 2,
            })
            .unwrap();
        input.configure(None).unwrap();
        assert!(!input.pressed().unwrap());
    }
}
