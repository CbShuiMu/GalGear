//! 在 Windows 原生侧观察 WebView2 进程故障，即使页面已无法发送 IPC。

use tauri::{WebviewWindow, webview::PlatformWebview};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        COREWEBVIEW2_PROCESS_FAILED_REASON, ICoreWebView2ProcessFailedEventArgs,
        ICoreWebView2ProcessFailedEventArgs2,
    },
    ProcessFailedEventHandler,
};
use windows_core::Interface;

/// 为窗口生命周期注册进程诊断。注册失败只写日志，窗口仍可使用。
pub(crate) fn observe(window: &WebviewWindow) {
    let label = window.label().to_owned();
    if let Err(error) = window.with_webview(move |webview| {
        if let Err(error) = subscribe(&webview, &label) {
            log::warn!("webview_diagnostics_unavailable window={label} error={error}");
        }
    }) {
        log::warn!(
            "webview_diagnostics_unavailable window={} error={error}",
            window.label()
        );
    }
}

fn subscribe(webview: &PlatformWebview, label: &str) -> windows_core::Result<()> {
    let window = label.to_owned();
    let callback = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        match args.as_ref().map(read_failure) {
            Some(Ok(failure)) => failure.record(&window),
            Some(Err(error)) => log::warn!("webview_process_failed window={window} error={error}"),
            None => log::warn!("webview_process_failed window={window} details=unavailable"),
        }
        Ok(())
    }));
    let mut token = 0;
    let mut pid = 0;
    // SAFETY: Tauri 在 WebView 所属的 UI 线程调用 with_webview。COM 接口和输出缓冲区
    // 在各次调用期间有效。WebView 持有回调，回调只捕获窗口标签，不持有 WebView 引用。
    let browser_pid = unsafe {
        let core = webview.controller().CoreWebView2()?;
        core.add_ProcessFailed(&callback, &mut token)?;
        core.BrowserProcessId(&mut pid).ok().map(|()| pid)
    };
    log::info!("webview_diagnostics_ready window={label} browser_pid={browser_pid:?}");
    Ok(())
}

struct ProcessFailure {
    kind: COREWEBVIEW2_PROCESS_FAILED_KIND,
    reason: Option<i32>,
    exit_code: Option<i32>,
}

impl ProcessFailure {
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
            "webview_process_failed window={window} kind={kind} kind_code={} reason_code={:?} exit_code={:?}",
            self.kind.0,
            self.reason,
            self.exit_code
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
