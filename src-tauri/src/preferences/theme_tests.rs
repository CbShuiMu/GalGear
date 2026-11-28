//! 使用隔离的隐藏 WebView 验证原生主题覆盖的设置与清除，不修改系统或用户配置。
use super::native_theme;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use tauri::{App, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[tauri::command]
fn report_theme(dark: bool, sender: tauri::State<'_, Sender<bool>>) {
    sender.send(dark).unwrap();
}

// 回归测试需要逐步驱动真实 Windows 消息循环，检查每次切换后的窗口和 WebView。
#[allow(deprecated)]
fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        app.run_iteration(|_, _| {});
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn webview_dark(app: &mut App, window: &WebviewWindow, receiver: &Receiver<bool>) -> bool {
    window
        .eval("window.__TAURI__.core.invoke('report_theme', {dark: matchMedia('(prefers-color-scheme: dark)').matches})")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        settle(app);
        if let Ok(dark) = receiver.try_recv() {
            return dark;
        }
    }
    panic!("WebView 未上报亮暗外观");
}

#[test]
#[ignore = "需要 Windows 桌面和 WebView2，仅创建临时 profile 下的隐藏窗口，不加载业务状态"]
fn native_system_theme_restores_window_and_webview_after_manual_override() {
    let data = tempfile::tempdir().unwrap();
    let (sender, receiver) = mpsc::channel::<bool>();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let mut app = tauri::Builder::default()
        .any_thread()
        .manage(sender)
        .invoke_handler(tauri::generate_handler![report_theme])
        .build(context)
        .unwrap();
    let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::App("index.html".into()))
        .data_directory(data.path().to_owned())
        .visible(false)
        .build()
        .unwrap();
    settle(&mut app);
    let system = window.theme().unwrap();
    let opposite = if system == Theme::Dark {
        "light"
    } else {
        "dark"
    };
    for mode in [opposite, "system", "dark", "light", "system"] {
        app.handle().set_theme(native_theme(mode));
        settle(&mut app);
        let expected = native_theme(mode).unwrap_or(system);
        assert_eq!(window.theme().unwrap(), expected, "原生窗口：{mode}");
        assert_eq!(
            webview_dark(&mut app, &window, &receiver),
            expected == Theme::Dark,
            "WebView：{mode}"
        );
    }
    window.destroy().unwrap();
    settle(&mut app);
}
