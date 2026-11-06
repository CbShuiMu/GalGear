//! 每次进程启动独立记录诊断信息；禁止记录命令正文、成功响应和凭据。
//! 日志直接写入文件而不使用用户态缓冲，错误额外刷盘；Windows 异常使用独立句柄。

mod privacy;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    backtrace::Backtrace,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tauri::Listener;

pub(crate) const FRONTEND_SCRIPT: &str = include_str!("diagnostics/frontend.js");
static LOGGER: OnceLock<SessionLog> = OnceLock::new();

struct SessionLog {
    file: Mutex<File>,
    path: PathBuf,
    started: Instant,
    privacy: privacy::Redactor,
    failed: AtomicBool,
}

impl SessionLog {
    fn create(directory: &Path, timestamp: &str) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        // create_new 同时处理多实例和系统时间回拨，绝不截断已有日志。
        for suffix in 0..1000 {
            let suffix = if suffix == 0 {
                String::new()
            } else {
                format!("_{suffix}")
            };
            let path = directory.join(format!("galgear_{timestamp}{suffix}.log"));
            match OpenOptions::new().append(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        file: Mutex::new(file),
                        path,
                        started: Instant::now(),
                        privacy: privacy::Redactor::new()?,
                        failed: AtomicBool::new(false),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Log filename collisions",
        ))
    }

    fn write(&self, level: Level, target: &str, message: &str) -> io::Result<()> {
        let thread = std::thread::current();
        let entry = json!({
            "time": Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
            "elapsed_ms": self.started.elapsed().as_millis(),
            "level": level.as_str(), "pid": std::process::id(),
            "thread": format!("{:?}", thread.id()), "thread_name": thread.name(),
            "target": target, "message": self.privacy.clean(message),
        });
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        file.write_all(&line)?;
        if level == Level::Error {
            file.sync_data()?;
        }
        Ok(())
    }
}

impl Log for SessionLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        // HTTP/SDK 的第三方 debug 日志可能含请求头，只接受本项目主动写入的记录。
        metadata.level() <= Level::Info && metadata.target().starts_with("galgear")
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata())
            && let Err(error) =
                self.write(record.level(), record.target(), &record.args().to_string())
            && !self.failed.swap(true, Ordering::Relaxed)
        {
            eprintln!("GalGear log write failed: {error}");
            // 避免在故障/持锁线程显示模态框；只通知一次，业务继续运行。
            let _ = std::thread::Builder::new()
                .name("log-warning".into())
                .spawn(move || {
                    show_failure("diagnostics.write_failed", &error.to_string());
                });
        }
    }

    fn flush(&self) {
        if let Ok(file) = self.file.lock()
            && let Err(error) = file.sync_data()
        {
            eprintln!("GalGear log flush failed: {error}");
        }
    }
}

fn initialize_at(directory: &Path) -> io::Result<()> {
    let logger = SessionLog::create(
        directory,
        &Local::now().format("%Y-%m-%d_%H%M%S_%3f").to_string(),
    )?;
    LOGGER
        .set(logger)
        .map_err(|_| io::Error::other("Logger already initialized"))?;
    let logger = LOGGER
        .get()
        .ok_or_else(|| io::Error::other("Logger unavailable"))?;
    log::set_logger(logger).map_err(|error| io::Error::other(error.to_string()))?;
    log::set_max_level(LevelFilter::Info);
    std::panic::set_hook(Box::new(|panic| {
        let payload = panic
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        log::error!(target: "galgear::panic", "panic at {}: {payload}\n{}",
            panic.location().map(|value| value.to_string()).unwrap_or_default(), Backtrace::force_capture());
    }));
    log::info!(target: "galgear::session", "session_start version={} os={} arch={} debug={} exe={} cwd={}",
        env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, cfg!(debug_assertions),
        std::env::current_exe()?.display(), std::env::current_dir()?.display());
    #[cfg(windows)]
    windows::install(&logger.path)?;
    Ok(())
}

/// 在桌面运行时启动前创建安装目录 logs。失败时明确通知，不静默改写到其他目录。
pub(crate) fn initialize() {
    let result = std::env::current_exe().and_then(|exe| {
        let directory = exe
            .parent()
            .ok_or_else(|| io::Error::other("Executable directory"))?;
        initialize_at(&directory.join("logs"))
    });
    if let Err(error) = result {
        eprintln!("GalGear log initialization failed: {error}");
        show_failure("diagnostics.init_failed", &error.to_string());
    }
}

fn show_failure(key: &str, detail: &str) {
    rfd::MessageDialog::new()
        .set_title("GalGear")
        .set_description(galgear_i18n::Message::detail(key, detail).render(Default::default()))
        .set_level(rfd::MessageLevel::Warning)
        .show();
}

/// 仅完整退出路径调用；后台任务的可恢复 panic 不阻止后续正常退出。
pub(crate) fn finish() {
    log::info!(target: "galgear::session", "session_end");
    log::logger().flush();
}

/// 在后端收到命令时记录起点，即使 WebView 后续崩溃也保留操作线索。
fn command_started(invoke: &tauri::ipc::Invoke) {
    let command = invoke.message.command();
    if command == "record_diagnostic" {
        return;
    }
    let request = invoke
        .message
        .headers()
        .get("x-galgear-request")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("untracked");
    let game_id = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(value) => {
            value.get("id").and_then(Value::as_str).filter(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        }
        _ => None,
    };
    log::info!(target: "galgear::command", "command_start window={} request={} command={} game_id={}",
        invoke.message.webview_ref().label(), request, command, game_id.unwrap_or("-"));
}

/// 包装统一命令分发入口，不改变命令参数、结果或原有权限检查。
pub(crate) fn trace_commands(
    handler: fn(tauri::ipc::Invoke) -> bool,
) -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        command_started(&invoke);
        handler(invoke)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontendKind {
    CommandOk,
    CommandError,
    JavascriptError,
    WasmPanic,
    Ready,
}

#[derive(Deserialize)]
pub(crate) struct FrontendEvent {
    kind: FrontendKind,
    #[serde(default)]
    request: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    elapsed_ms: u64,
    #[serde(default)]
    detail: String,
}

/// 前端只上传诊断文本与关联编号，统一在写入边界脱敏和限长。
#[tauri::command]
pub(crate) fn record_diagnostic(event: FrontendEvent) {
    match event.kind {
        FrontendKind::CommandOk => {
            log::info!(target: "galgear::command", "command_ok request={} command={} elapsed_ms={}", event.request, event.command, event.elapsed_ms)
        }
        FrontendKind::CommandError => {
            log::error!(target: "galgear::command", "command_error request={} command={} elapsed_ms={} error={}", event.request, event.command, event.elapsed_ms, event.detail)
        }
        FrontendKind::JavascriptError => {
            log::error!(target: "galgear::frontend", "javascript_error {}", event.detail)
        }
        FrontendKind::WasmPanic => {
            log::error!(target: "galgear::frontend", "wasm_panic {}", event.detail)
        }
        FrontendKind::Ready => {
            log::info!(target: "galgear::frontend", "frontend_ready {}", event.detail)
        }
    }
}

/// 后台事件在原生侧记录，不依赖前端窗口是否仍然存活。
pub(crate) fn observe_events(app: &tauri::AppHandle) {
    for name in [
        "tool-failed",
        "backup-completed",
        "screenshot-completed",
        "fan-status-updated",
        "enrichment-progress",
    ] {
        app.listen_any(name, move |event| {
            if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
                record_messages(name, &value, 0);
            }
        });
    }
}

fn record_messages(context: &str, value: &Value, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Ok(message) = serde_json::from_value::<galgear_i18n::Message>(value.clone()) {
        log::warn!(target: "galgear::background", "event={context} key={} detail={}", message.key, message);
    } else {
        match value {
            Value::Object(fields) => {
                for value in fields.values() {
                    record_messages(context, value, depth + 1);
                }
            }
            Value::Array(values) => {
                for value in values.iter().take(100) {
                    record_messages(context, value, depth + 1);
                }
            }
            _ => {}
        }
    }
}
