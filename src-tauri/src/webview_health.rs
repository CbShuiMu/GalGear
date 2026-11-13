//! WebView 已退出时使用原生提示，避免依赖已失效的页面或中断后台写入。

use crate::state::{AppState, snapshot};
use galgear_i18n::Message;
use tauri::Manager;

/// 异步告知界面进程已退出；不自动重启应用，以免打断存档或更新事务。
pub(crate) fn notify_exit(app: tauri::AppHandle) {
    // WebView2 的 COM 回调内不能运行模态消息循环，也不在这里等待配置锁。
    if let Err(error) = std::thread::Builder::new()
        .name("webview-warning".into())
        .spawn(move || {
            let language = app
                .try_state::<AppState>()
                .and_then(|state| snapshot(&state).ok())
                .map(|settings| settings.language)
                .unwrap_or_default();
            rfd::MessageDialog::new()
                .set_title("GalGear")
                .set_description(Message::new("diagnostics.webview_exited").render(language))
                .set_level(rfd::MessageLevel::Error)
                .show();
        })
    {
        log::error!("webview_warning_failed error={error}");
    }
}
