//! 在 Windows 原生侧观察 WebView2 进程故障，即使页面已无法发送 IPC。

use std::cell::Cell;
use tauri::{WebviewWindow, webview::PlatformWebview};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        COREWEBVIEW2_PROCESS_FAILED_REASON, ICoreWebView2Environment11,
        ICoreWebView2ProcessFailedEventArgs, ICoreWebView2ProcessFailedEventArgs2,
    },
    ProcessFailedEventHandler, take_pwstr,
};
use windows_core::Interface;

/// 注册进程诊断；主页面进程退出时通知一次。回调必须立即返回，不得阻塞 COM 事件。
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
    let window = label.to_owned();
    let notified = Cell::new(false);
    let callback = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        match args.as_ref().map(read_failure) {
            Some(Ok(failure)) => {
                failure.record(&window);
                if failure.page_exited() && !notified.replace(true) {
                    on_exit();
                }
            }
            Some(Err(error)) => log::warn!("webview_process_failed window={window} error={error}"),
            None => log::warn!("webview_process_failed window={window} details=unavailable"),
        }
        Ok(())
    }));
    let mut token = 0;
    let mut pid = 0;
    // SAFETY: Tauri 在 WebView 所属的 UI 线程调用 with_webview。COM 接口和输出缓冲区
    // 在各次调用期间有效。WebView 持有回调，回调不得捕获 WebView 引用而形成循环。
    let browser_pid = unsafe {
        let core = webview.controller().CoreWebView2()?;
        core.add_ProcessFailed(&callback, &mut token)?;
        core.BrowserProcessId(&mut pid).ok().map(|()| pid)
    };
    log::info!("webview_diagnostics_ready window={label} browser_pid={browser_pid:?}");
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

    fn record(&self, window: &str) {
        let kind = match self.kind {
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browser_exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "renderer_exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => "renderer_unresponsive",
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => "frame_renderer_exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => "gpu_exited",
            _ => "other",
        };
        log::warn!(
            "webview_process_failed window={window} kind={kind} kind_code={} reason_code={:?} exit_code={:?} exit_code_hex={}",
            self.kind.0,
            self.reason,
            self.exit_code,
            self.exit_code
                .map(|code| format!("0x{:08X}", code as u32))
                .unwrap_or_else(|| "unavailable".into())
        );
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
