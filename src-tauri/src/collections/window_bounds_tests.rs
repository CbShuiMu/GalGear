//! 真实 Tauri/Windows 窗口的尺寸回归，不加载应用状态或真实游戏数据。
use super::set_library_bounds;
use std::time::{Duration, Instant};
use tauri::{App, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows_sys::Win32::UI::WindowsAndMessaging::{MINMAXINFO, SendMessageW, WM_GETMINMAXINFO};

// 测试需要逐步驱动原生事件循环并检查每个中间状态。
#[allow(deprecated)]
fn settle(app: &mut App) {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        app.run_iteration(|_, _| {});
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn geometry(window: &WebviewWindow) -> (tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    (
        window.outer_position().unwrap(),
        window.outer_size().unwrap(),
    )
}

#[test]
#[ignore = "需要交互式 Windows 桌面，会创建隔离的空白 WebView 窗口"]
fn native_library_bounds_preserve_window_modes() {
    let data = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let mut app = tauri::Builder::default()
        .any_thread()
        .build(context)
        .unwrap();
    let window = WebviewWindowBuilder::new(
        &app,
        "bounds-test",
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .title("GalGear window bounds regression")
    .data_directory(data.path().to_owned())
    .inner_size(1100.0, 700.0)
    .position(120.0, 100.0)
    .build()
    .unwrap();
    settle(&mut app);

    window.maximize().unwrap();
    settle(&mut app);
    assert!(window.is_maximized().unwrap());
    let maximized = geometry(&window);
    for width in [1122.0, 1362.0, 1500.0] {
        assert!(!set_library_bounds(width, 640.0, window.clone()).unwrap());
        settle(&mut app);
        assert!(window.is_maximized().unwrap());
        assert_eq!(geometry(&window), maximized);
    }
    assert!(set_library_bounds(f64::NAN, 640.0, window.clone()).is_err());
    assert!(set_library_bounds(1122.0, 0.0, window.clone()).is_err());

    window.unmaximize().unwrap();
    settle(&mut app);
    window.set_fullscreen(true).unwrap();
    settle(&mut app);
    let fullscreen = geometry(&window);
    assert!(!set_library_bounds(1362.0, 640.0, window.clone()).unwrap());
    settle(&mut app);
    assert!(window.is_fullscreen().unwrap());
    assert_eq!(geometry(&window), fullscreen);

    window.set_fullscreen(false).unwrap();
    settle(&mut app);
    window.minimize().unwrap();
    settle(&mut app);
    assert!(!set_library_bounds(1362.0, 640.0, window.clone()).unwrap());
    settle(&mut app);
    assert!(window.is_minimized().unwrap());

    window.unminimize().unwrap();
    settle(&mut app);
    let restored_position = window.outer_position().unwrap();
    assert!(set_library_bounds(1362.0, 640.0, window.clone()).unwrap());
    settle(&mut app);
    assert!(!window.is_maximized().unwrap());
    assert_eq!(window.outer_position().unwrap(), restored_position);
    let size = window
        .inner_size()
        .unwrap()
        .to_logical::<f64>(window.scale_factor().unwrap());
    assert!(size.width >= 1362.0 && size.height >= 640.0);
    let restored = geometry(&window);
    assert!(set_library_bounds(1122.0, 640.0, window.clone()).unwrap());
    settle(&mut app);
    assert_eq!(geometry(&window), restored, "降低最小尺寸不能缩小窗口");
    // 程序 set_size 可绕过拖动约束；查询 Windows 实际使用的最小跟踪尺寸。
    let mut limits = MINMAXINFO::default();
    let hwnd = window.hwnd().unwrap();
    // SAFETY: hwnd 属于本测试仍存活的窗口，limits 在同步消息返回前始终有效。
    unsafe {
        SendMessageW(
            hwnd.0,
            WM_GETMINMAXINFO,
            0,
            (&mut limits as *mut MINMAXINFO) as isize,
        );
    }
    let outer = window.outer_size().unwrap();
    let inner = window.inner_size().unwrap();
    let scale = window.scale_factor().unwrap();
    assert_eq!(
        limits.ptMinTrackSize.x,
        (1122.0 * scale).round() as i32 + (outer.width - inner.width) as i32
    );
    assert_eq!(
        limits.ptMinTrackSize.y,
        (640.0 * scale).round() as i32 + (outer.height - inner.height) as i32
    );
    window.destroy().unwrap();
    settle(&mut app);
}
