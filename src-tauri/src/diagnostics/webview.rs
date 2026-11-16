//! 在 Windows 原生侧观察 WebView2 进程故障，即使页面已无法发送 IPC。

use crate::diagnostics::record_webview_failure;
use std::{
    cell::Cell,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use tauri::{WebviewWindow, webview::PlatformWebview};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        COREWEBVIEW2_PROCESS_FAILED_REASON, ICoreWebView2, ICoreWebView2Environment11,
        ICoreWebView2ProcessFailedEventArgs, ICoreWebView2ProcessFailedEventArgs2,
    },
    ProcessFailedEventHandler, take_pwstr,
};
use windows_core::Interface;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// 注册进程诊断；主页面进程退出时通知一次。回调只记录并刷盘，不等待恢复或运行模态循环。
pub(crate) fn observe(window: &WebviewWindow, on_exit: impl Fn() + Send + 'static) {
    let label = window.label().to_owned();
    if let Err(error) = window.with_webview(move |webview| {
        if let Err(error) = subscribe(&webview, &label, on_exit) {
            log::warn!("webview_diagnostics_unavailable window={label} error={error}");
        }
    }) {
        log::warn!(
            "webview_diagnostics_unavailable window={} error={error}",
            window.label()
        );
    }
}

fn subscribe(
    webview: &PlatformWebview,
    label: &str,
    on_exit: impl Fn() + 'static,
) -> windows_core::Result<()> {
    // SAFETY: Tauri 在 WebView 所属 UI 线程提供有效控制器。
    let core = unsafe { webview.controller().CoreWebView2()? };
    // 提前保存实际环境信息，进程退出后 COM 查询和进程路径查询都可能失败。
    let runtime = runtime_context(webview, &core, label);
    let failure_runtime = runtime.clone();
    let window = label.to_owned();
    let notified = Cell::new(false);
    let callback = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        match args.as_ref().map(read_failure) {
            Some(Ok(failure)) => {
                failure.record(&window, &failure_runtime);
                if failure.page_exited() && !notified.replace(true) {
                    on_exit();
                }
            }
            Some(Err(error)) => record_webview_failure(format_args!(
                "webview_process_failed window={window} error={error} {failure_runtime}"
            )),
            None => record_webview_failure(format_args!(
                "webview_process_failed window={window} details=unavailable {failure_runtime}"
            )),
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: Tauri 在 WebView 所属的 UI 线程调用 with_webview。COM 接口和输出缓冲区
    // 在各次调用期间有效。WebView 持有回调，回调不得捕获 WebView 引用而形成循环。
    unsafe {
        core.add_ProcessFailed(&callback, &mut token)?;
    }
    log::info!("webview_diagnostics_ready window={label} {runtime}");
    // 较旧的运行时可以没有 Environment11；读取转储路径失败不能撤销进程监听。
    let report_folder = (|| {
        let environment = webview.environment().cast::<ICoreWebView2Environment11>()?;
        let mut path = windows_core::PWSTR::null();
        // SAFETY: 当前 UI 线程上的有效 COM 环境及输出指针；take_pwstr 释放 COM 分配的字符串。
        unsafe {
            environment.FailureReportFolderPath(&mut path)?;
            Ok::<_, windows_core::Error>(take_pwstr(path))
        }
    })();
    match report_folder {
        Ok(path) => log::info!("webview_failure_report_folder window={label} path={path}"),
        Err(error) => {
            log::warn!("webview_failure_report_folder_unavailable window={label} error={error}")
        }
    }
    Ok(())
}

fn runtime_context(webview: &PlatformWebview, core: &ICoreWebView2, label: &str) -> String {
    let mut pid = 0;
    let mut version = windows_core::PWSTR::null();
    // SAFETY: 接口属于当前 UI 线程，输出指针有效；take_pwstr 释放 COM 分配的字符串。
    let (browser_pid, version) = unsafe {
        let browser_pid = core
            .BrowserProcessId(&mut pid)
            .inspect_err(|error| {
                log::warn!("webview_browser_pid_unavailable window={label} error={error}");
            })
            .ok()
            .map(|()| pid);
        let version = webview
            .environment()
            .BrowserVersionString(&mut version)
            .map(|()| take_pwstr(version))
            .inspect_err(|error| {
                log::warn!("webview_version_unavailable window={label} error={error}");
            })
            .ok();
        (browser_pid, version)
    };
    let executable = browser_pid.and_then(|pid| {
        process_executable(pid)
            .inspect_err(|error| {
                log::warn!(
                    "webview_executable_unavailable window={label} browser_pid={pid} error={error}"
                );
            })
            .ok()
    });
    // 路径使用原始反斜杠进入统一脱敏边界，避免 Debug 转义影响用户名匹配。
    format!(
        "browser_pid={browser_pid:?} webview_version={} browser_exe=\"{}\"",
        version.as_deref().unwrap_or("unavailable"),
        executable.as_deref().unwrap_or("unavailable"),
    )
}

fn process_executable(pid: u32) -> io::Result<String> {
    // SAFETY: 只请求查询权限；成功的独占句柄交由 OwnedHandle 在所有路径释放。
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcess 返回的有效句柄仅在此转移一次所有权。
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: 句柄在查询期间有效，长度为 UTF-16 缓冲区容量；系统返回已写入字符数。
    if unsafe {
        QueryFullProcessImageNameW(process.as_raw_handle(), 0, buffer.as_mut_ptr(), &mut length)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

struct ProcessFailure {
    kind: COREWEBVIEW2_PROCESS_FAILED_KIND,
    reason: Option<i32>,
    exit_code: Option<i32>,
}

impl ProcessFailure {
    fn page_exited(&self) -> bool {
        matches!(
            self.kind,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
        )
    }

    fn record(&self, window: &str, runtime: &str) {
        let kind = match self.kind {
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browser_exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "renderer_exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => "renderer_unresponsive",
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => "frame_renderer_exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => "gpu_exited",
            _ => "other",
        };
        record_webview_failure(format_args!(
            "webview_process_failed window={window} kind={kind} kind_code={} reason_code={:?} exit_code={:?} exit_code_hex={} {runtime}",
            self.kind.0,
            self.reason,
            self.exit_code,
            self.exit_code
                .map(|code| format!("0x{:08X}", code as u32))
                .unwrap_or_else(|| "unavailable".into())
        ));
    }
}

fn read_failure(
    args: &ICoreWebView2ProcessFailedEventArgs,
) -> windows_core::Result<ProcessFailure> {
    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
    let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
    let mut exit_code = 0;
    // SAFETY: 参数由当前 UI 线程上的 WebView2 回调提供。所有输出指针均指向有效的本地值。
    let (reason, exit_code) = unsafe {
        args.ProcessFailedKind(&mut kind)?;
        match args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
            Ok(details) => (
                details.Reason(&mut reason).ok().map(|()| reason.0),
                details.ExitCode(&mut exit_code).ok().map(|()| exit_code),
            ),
            Err(_) => (None, None),
        }
    };
    Ok(ProcessFailure {
        kind,
        reason,
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::process_executable;

    #[test]
    fn executable_query_reads_the_actual_process_image() {
        let actual = process_executable(std::process::id()).unwrap();
        assert_eq!(
            std::fs::canonicalize(actual).unwrap(),
            std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
        );
    }

    #[test]
    fn executable_query_preserves_process_open_failure() {
        assert!(process_executable(0).is_err());
    }
}
