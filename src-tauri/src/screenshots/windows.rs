//! 截取屏幕上前台窗口的客户区，不向游戏注入代码，也不依赖引擎。
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::*,
    System::{DataExchange::*, Memory::*},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

#[cfg(test)]
mod keyboard;
use crate::game_window::magpie;
mod notification;
pub(super) use notification::Notification;

pub(super) struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}

pub(super) fn register(window: HWND, id: i32, hotkey: &str) -> Result<(), galgear_i18n::Message> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let canonical = galgear_core::screenshots::hotkey(hotkey)?;
    let keys = galgear_core::runtime::hotkey_keys(&canonical)
        .map_err(|_| galgear_i18n::Message::new("screenshot.invalid_hotkey"))?;
    let mut modifiers = MOD_NOREPEAT;
    for key in &keys[..keys.len() - 1] {
        modifiers |= match key {
            0x10 => MOD_SHIFT,
            0x11 => MOD_CONTROL,
            0x12 => MOD_ALT,
            0x5b => MOD_WIN,
            _ => 0,
        };
    }
    // SAFETY: 当前线程拥有窗口和模块内固定 ID；按键经过完整解析。
    if unsafe { RegisterHotKey(window, id, modifiers, u32::from(keys[keys.len() - 1])) } == 0 {
        Err(galgear_i18n::Message::detail(
            "screenshot.hotkey_error",
            last_error(),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn unregister(window: HWND, id: i32) {
    // SAFETY: 仅撤销当前线程在自己窗口上注册的固定 ID。
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey(window, id);
    }
}

struct CaptureResources {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
}
impl Drop for CaptureResources {
    fn drop(&mut self) {
        // SAFETY: 句柄仅由本次采集创建；位图在删除前已从 DC 中移出。
        unsafe {
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.screen.is_null() {
                ReleaseDC(ptr::null_mut(), self.screen);
            }
        }
    }
}
struct DpiContext(DPI_AWARENESS_CONTEXT);
impl Drop for DpiContext {
    fn drop(&mut self) {
        // SAFETY: 恢复同一线程先前的 DPI 上下文，不改变其他线程或游戏。
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

pub(super) fn capture(window: HWND) -> Result<Frame, String> {
    // SAFETY: Win32 输出指针均指向有效局部变量；GDI 资源由 RAII 管理，
    // 像素长度经过有界检查，GetDIBits 仅写入对应尺寸的缓冲。
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if previous.is_null() {
            return Err(last_error());
        }
        let _dpi = DpiContext(previous);
        if magpie::source(GetForegroundWindow()) != window {
            return Err("Game is no longer in the foreground".into());
        }
        if window.is_null() || IsIconic(window) != 0 {
            return Err("No foreground game window".into());
        }
        let output = magpie::output(window);
        let mut rect: RECT = std::mem::zeroed();
        let mut origin = POINT { x: 0, y: 0 };
        if GetClientRect(output, &mut rect) == 0 || ClientToScreen(output, &mut origin) == 0 {
            return Err(last_error());
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        // 屏外区域没有实际画面，拒绝截取，避免把未初始化像素当作游戏图像。
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        if width <= 0
            || height <= 0
            || width > 16384
            || height > 16384
            || i64::from(width) * i64::from(height) > 64 * 1024 * 1024
            || origin.x < left
            || origin.y < top
            || i64::from(origin.x) + i64::from(width)
                > i64::from(left) + i64::from(GetSystemMetrics(SM_CXVIRTUALSCREEN))
            || i64::from(origin.y) + i64::from(height)
                > i64::from(top) + i64::from(GetSystemMetrics(SM_CYVIRTUALSCREEN))
        {
            return Err("Window is empty, too large or outside the desktop".into());
        }
        let mut resources = CaptureResources {
            screen: GetDC(ptr::null_mut()),
            memory: ptr::null_mut(),
            bitmap: ptr::null_mut(),
        };
        if resources.screen.is_null() {
            return Err(last_error());
        }
        resources.memory = CreateCompatibleDC(resources.screen);
        resources.bitmap = CreateCompatibleBitmap(resources.screen, width, height);
        if resources.memory.is_null() || resources.bitmap.is_null() {
            return Err(last_error());
        }
        let old = SelectObject(resources.memory, resources.bitmap);
        if old.is_null() || old as isize == -1 {
            return Err(last_error());
        }
        let copied = BitBlt(
            resources.memory,
            0,
            0,
            width,
            height,
            resources.screen,
            origin.x,
            origin.y,
            SRCCOPY | CAPTUREBLT,
        );
        SelectObject(resources.memory, old);
        if magpie::source(GetForegroundWindow()) != window || magpie::output(window) != output {
            return Err("Game focus changed during capture".into());
        }
        if copied == 0 {
            return Err(last_error());
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut rgba = vec![0; width as usize * height as usize * 4];
        if GetDIBits(
            resources.memory,
            resources.bitmap,
            0,
            height as u32,
            rgba.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        ) != height
        {
            return Err(last_error());
        }
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(Frame {
            width: width as u32,
            height: height as u32,
            rgba,
        })
    }
}

struct GlobalBuffer(windows_sys::Win32::Foundation::HGLOBAL);
impl Drop for GlobalBuffer {
    fn drop(&mut self) {
        // SAFETY: 仅释放尚未转移给剪贴板的分配，GlobalLock 已在调用点解除。
        unsafe {
            windows_sys::Win32::Foundation::GlobalFree(self.0);
        }
    }
}
struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        // SAFETY: 只在 OpenClipboard 成功后构造；同一线程配对关闭。
        unsafe {
            CloseClipboard();
        }
    }
}

impl GlobalBuffer {
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        // SAFETY: 分配与复制的长度一致；锁只覆盖复制，失败时 RAII 回收。
        unsafe {
            let buffer = Self(GlobalAlloc(GMEM_MOVEABLE, bytes.len()));
            if buffer.0.is_null() {
                return Err(last_error());
            }
            let pointer = GlobalLock(buffer.0).cast::<u8>();
            if pointer.is_null() {
                return Err(last_error());
            }
            ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
            GlobalUnlock(buffer.0);
            Ok(buffer)
        }
    }

    fn publish(self, format: u32) -> Result<(), String> {
        // SAFETY: 调用者已打开并清空剪贴板；成功后交由系统释放，否则保留 RAII。
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err(last_error());
        }
        std::mem::forget(self);
        Ok(())
    }
}

fn dib(frame: &Frame) -> Result<Vec<u8>, String> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || width * height > 64 * 1024 * 1024
        || frame.rgba.len() != width * height * 4
    {
        return Err("Invalid clipboard image".into());
    }
    // 旧版聊天客户端对 top-down 32 位 BI_RGB 兼容不一，提供传统 bottom-up 24 位 DIB。
    // 每行按 DWORD 对齐，不缩放、不丢弃 RGB 精度；截屏本身是不透明的。
    let stride = (width * 3 + 3) & !3;
    let mut bytes = vec![0u8; size_of::<BITMAPINFOHEADER>() + stride * height];
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        biHeight: height as i32,
        biPlanes: 1,
        biBitCount: 24,
        biCompression: BI_RGB,
        biSizeImage: (stride * height) as u32,
        // SAFETY: BITMAPINFOHEADER 是纯整数结构，零是未使用字段的有效默认值。
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: 头是已初始化的 repr(C) 整数结构，目标包含完整头部空间。
    unsafe {
        ptr::copy_nonoverlapping(
            (&header as *const BITMAPINFOHEADER).cast::<u8>(),
            bytes.as_mut_ptr(),
            size_of::<BITMAPINFOHEADER>(),
        );
    }
    for (y, row) in frame.rgba.chunks_exact(width * 4).enumerate() {
        let start = size_of::<BITMAPINFOHEADER>() + (height - y - 1) * stride;
        for (target, pixel) in bytes[start..start + width * 3]
            .chunks_exact_mut(3)
            .zip(row.chunks_exact(4))
        {
            target.copy_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
    }
    Ok(bytes)
}

pub(super) fn copy(frame: &Frame, owner: HWND, png: &std::path::Path) -> Result<(), String> {
    let bitmap = GlobalBuffer::from_bytes(&dib(frame)?)?;
    // 复用刚保存的无损 PNG，现代客户端可直接读取 PNG，无需依赖位图格式转换。
    let png = GlobalBuffer::from_bytes(&std::fs::read(png).map_err(|e| e.to_string())?)?;
    // SAFETY: 固定格式名有效；窗口归本线程所有。数据均在打开剪贴板前备好，缩短独占时间。
    unsafe {
        let png_format = RegisterClipboardFormatW(windows_sys::w!("PNG"));
        if png_format == 0 {
            return Err(last_error());
        }
        let mut opened = false;
        for _ in 0..20 {
            if OpenClipboard(owner) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !opened {
            return Err(last_error());
        }
        let _clipboard = Clipboard;
        if EmptyClipboard() == 0 {
            return Err(last_error());
        }
        png.publish(png_format)?;
        bitmap.publish(
            8, /* CF_DIB；Windows 同时可合成 CF_BITMAP / CF_DIBV5 */
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_dib_keeps_rgb_orientation_and_pads_odd_widths() {
        let frame = Frame {
            width: 1,
            height: 2,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let bytes = dib(&frame).unwrap();
        assert_eq!(
            &bytes[size_of::<BITMAPINFOHEADER>()..],
            &[255, 0, 0, 0, 0, 0, 255, 0]
        );
        assert!(
            dib(&Frame {
                width: 2,
                height: 2,
                rgba: vec![]
            })
            .is_err()
        );
    }

    #[test]
    #[ignore = "需要交互式 Windows 桌面；创建隔离窗口并写入测试图像到剪贴板"]
    fn native_client_pixels_clipboard_and_hotkey_conflict() {
        // SAFETY: 测试独占创建窗口和 GDI 对象，所有句柄在本线程释放。
        unsafe {
            let old_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            assert!(!old_dpi.is_null());
            let _dpi = DpiContext(old_dpi);
            let brush = CreateSolidBrush(0x003366CC);
            let class_name = windows_sys::w!("GalGearScreenshotTest");
            let class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                lpszClassName: class_name,
                hbrBackground: brush,
                ..std::mem::zeroed()
            };
            assert_ne!(RegisterClassW(&class), 0);
            struct TestClass(HBRUSH);
            impl Drop for TestClass {
                fn drop(&mut self) {
                    // SAFETY: 测试窗口先销毁，再注销本测试类并释放背景画刷。
                    unsafe {
                        UnregisterClassW(windows_sys::w!("GalGearScreenshotTest"), ptr::null_mut());
                        DeleteObject(self.0);
                    }
                }
            }
            let _class = TestClass(brush);
            let window = CreateWindowExW(
                WS_EX_TOPMOST,
                class_name,
                windows_sys::w!("GalGear screenshot test"),
                WS_OVERLAPPEDWINDOW,
                40,
                40,
                380,
                280,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!window.is_null());
            struct TestWindow(HWND);
            impl Drop for TestWindow {
                fn drop(&mut self) {
                    // SAFETY: 仅销毁测试创建的窗口。
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
            let _window = TestWindow(window);
            // 显式矩形区域关闭 Windows 11 的圆角，四角才属于测试客户区的纯色画面。
            let region = CreateRectRgn(0, 0, 380, 280);
            assert_ne!(SetWindowRgn(window, region, 1), 0); // 成功后系统拥有 region。
            ShowWindow(window, SW_SHOW);
            SetForegroundWindow(window);
            UpdateWindow(window);
            // 后台测试进程可能被 Windows 前台锁限制；留出时间由桌面验证工具激活隔离窗口。
            let waiting = std::time::Instant::now();
            while GetForegroundWindow() != window
                && waiting.elapsed() < std::time::Duration::from_secs(30)
            {
                let mut message = std::mem::zeroed();
                while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let mut rect: RECT = std::mem::zeroed();
            assert_ne!(GetClientRect(window, &mut rect), 0);
            let dc = GetDC(window);
            let brush = CreateSolidBrush(0x003366CC);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            ReleaseDC(window, dc);
            GdiFlush();
            std::thread::sleep(std::time::Duration::from_millis(200));
            let frame = capture(window).unwrap();
            assert_eq!(
                (frame.width, frame.height),
                (rect.right as u32, rect.bottom as u32)
            );
            assert_eq!(&frame.rgba[..4], &[204, 102, 51, 255]);
            assert_eq!(&frame.rgba[frame.rgba.len() - 4..], &[204, 102, 51, 255]);
            let temp = tempfile::tempdir().unwrap();
            if std::env::var_os("GALGEAR_TEST_MAGPIE").is_some() {
                // 可选真机联调：在测试窗口中启动 Magpie 全屏缩放。
                let waiting = std::time::Instant::now();
                while magpie::output(window) == window && waiting.elapsed().as_secs() < 60 {
                    let mut message = std::mem::zeroed();
                    while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                assert_ne!(
                    magpie::output(window),
                    window,
                    "请在测试窗口上启动 Magpie 缩放"
                );
                std::thread::sleep(std::time::Duration::from_secs(2));
                let scaled = capture(window).unwrap();
                let center = ((scaled.height / 2 * scaled.width + scaled.width / 2) * 4) as usize;
                for (actual, expected) in scaled.rgba[center..center + 4]
                    .iter()
                    .zip([204u8, 102, 51, 255])
                {
                    assert!(
                        actual.abs_diff(expected) <= 2,
                        "Magpie 中心像素应为源窗口纯色，不能是黑屏或桌面"
                    );
                }
                let mut monitor: MONITORINFO = std::mem::zeroed();
                monitor.cbSize = size_of::<MONITORINFO>() as u32;
                assert_ne!(
                    GetMonitorInfoW(
                        MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                        &mut monitor
                    ),
                    0
                );
                assert_eq!(
                    (scaled.width, scaled.height),
                    (
                        (monitor.rcMonitor.right - monitor.rcMonitor.left) as u32,
                        (monitor.rcMonitor.bottom - monitor.rcMonitor.top) as u32
                    )
                );
                let saved = galgear_core::screenshots::save_png(
                    temp.path(),
                    scaled.width,
                    scaled.height,
                    &scaled.rgba,
                )
                .unwrap();
                eprintln!(
                    "Magpie output verified: {}x{}, {}",
                    scaled.width,
                    scaled.height,
                    saved.display()
                );
            }
            let png = galgear_core::screenshots::save_png(
                temp.path(),
                frame.width,
                frame.height,
                &frame.rgba,
            )
            .unwrap();
            copy(&frame, window, &png).unwrap();
            assert_ne!(OpenClipboard(window), 0);
            let clipboard = Clipboard;
            let handle = GetClipboardData(8);
            assert!(!handle.is_null());
            let pointer = GlobalLock(handle).cast::<u8>();
            assert!(!pointer.is_null());
            let header = &*pointer.cast::<BITMAPINFOHEADER>();
            assert_eq!(header.biWidth, rect.right);
            assert_eq!(header.biHeight, rect.bottom);
            assert_eq!(header.biBitCount, 24);
            assert_ne!(
                IsClipboardFormatAvailable(RegisterClipboardFormatW(windows_sys::w!("PNG"))),
                0
            );
            assert!(!GetClipboardData(2 /* CF_BITMAP */).is_null());
            assert_eq!(
                std::slice::from_raw_parts(pointer.add(size_of::<BITMAPINFOHEADER>()), 3),
                &[51, 102, 204]
            );
            GlobalUnlock(handle);
            drop(clipboard);
            register(window, 71, "Ctrl+Alt+Shift+F10").unwrap();
            let other = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("conflict"),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            let _other = TestWindow(other);
            assert!(register(other, 72, "Ctrl+Alt+Shift+F10").is_err());
            unregister(window, 71);
            register(other, 72, "Ctrl+Alt+Shift+F10").unwrap();
            unregister(other, 72);
        }
    }
}
