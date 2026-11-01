//! 原生短提示沿用应用的明暗和配色，显示在游戏所在屏幕，不激活或抢焦点。
use std::{
    mem::size_of,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

const CLASS: windows_sys::core::PCWSTR = windows_sys::w!("GalGear.ScreenshotNotice");
struct Content {
    text: Vec<u16>,
    background: u32,
    foreground: u32,
    accent: u32,
    scale: f32,
}
fn rgb(value: u32) -> u32 {
    (value & 255) << 16 | value & 0xff00 | value >> 16
}

#[derive(Default)]
pub(crate) struct Notification {
    window: HWND,
    content: Option<Box<Content>>,
    shown: Option<Instant>,
}
impl Notification {
    pub(crate) fn hide(&mut self) {
        if !self.window.is_null() {
            // SAFETY: 窗口仅在创建线程销毁，回调期间 Content 仍然存活。
            unsafe {
                DestroyWindow(self.window);
            }
            self.window = ptr::null_mut();
        }
        self.content = None;
        self.shown = None;
    }
    pub(crate) fn expire(&mut self) {
        if self
            .shown
            .is_some_and(|time| time.elapsed() >= Duration::from_secs(3))
        {
            self.hide();
        }
    }
    pub(crate) fn show(
        &mut self,
        target: HWND,
        text: &str,
        theme: &str,
        palette: &str,
        error: bool,
    ) {
        self.hide();
        // SAFETY: 系统查询有效窗口的监视器和 DPI；无窗口时使用最近屏幕。
        unsafe {
            let dpi_context =
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let scale = GetDpiForWindow(target).max(96) as f32 / 96.0;
            let mut monitor: MONITORINFO = std::mem::zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(
                MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) == 0
            {
                if !dpi_context.is_null() {
                    SetThreadDpiAwarenessContext(dpi_context);
                }
                return;
            }
            let dark = theme == "dark";
            let neutral = palette == "white";
            let background = if dark {
                if neutral { 0x25252b } else { 0x352737 }
            } else if neutral {
                0xffffff
            } else if palette == "white-pink" {
                0xfff8fc
            } else {
                0xf6deeb
            };
            let mut content = Box::new(Content {
                text: text
                    .chars()
                    .take(240)
                    .collect::<String>()
                    .encode_utf16()
                    .chain(Some(0))
                    .collect(),
                background: rgb(background),
                foreground: rgb(if dark { 0xf5eaf2 } else { 0x44313f }),
                accent: rgb(if error {
                    if dark { 0xffb4ab } else { 0xb13f47 }
                } else if neutral {
                    if dark { 0xd2d3dc } else { 0x666671 }
                } else if dark {
                    0xee9fc5
                } else {
                    0xb84479
                }),
                scale,
            });
            let module = GetModuleHandleW(ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module,
                lpszClassName: CLASS,
                ..std::mem::zeroed()
            };
            // 重复注册会返回类已存在；随后 CreateWindowExW 使用同一模块的类。
            RegisterClassW(&class);
            let width = (400.0 * scale) as i32;
            let height = (100.0 * scale) as i32;
            let margin = (24.0 * scale) as i32;
            self.window = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                CLASS,
                windows_sys::w!("GalGear"),
                WS_POPUP,
                monitor.rcWork.right - width - margin,
                monitor.rcWork.bottom - height - margin,
                width,
                height,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                (&mut *content as *mut Content).cast(),
            );
            if !self.window.is_null() {
                let radius = (20.0 * scale) as i32;
                let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
                if !region.is_null() && SetWindowRgn(self.window, region, 0) == 0 {
                    DeleteObject(region);
                }
                self.content = Some(content);
                ShowWindow(self.window, SW_SHOWNOACTIVATE);
                UpdateWindow(self.window);
                self.shown = Some(Instant::now());
            }
            if !dpi_context.is_null() {
                SetThreadDpiAwarenessContext(dpi_context);
            }
        }
    }
}
impl Drop for Notification {
    fn drop(&mut self) {
        self.hide();
    }
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: Windows 调用约定保证创建消息和绘制消息的参数有效；Content 由窗口所有者
    // 持有，且在 DestroyWindow 返回后才释放。其他消息交回系统默认处理。
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        }
        if message == WM_MOUSEACTIVATE {
            return MA_NOACTIVATE as isize;
        }
        if message == WM_NCHITTEST {
            return HTTRANSPARENT as isize;
        }
        if message == WM_PAINT {
            let data = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Content;
            if !data.is_null() {
                paint(window, &*data);
                return 0;
            }
        }
        DefWindowProcW(window, message, wp, lp)
    }
}

fn paint(window: HWND, data: &Content) {
    // SAFETY: 仅在 WM_PAINT 中使用本窗口 DC；所有创建的 GDI 对象在移出 DC 后释放。
    unsafe {
        let mut ps = std::mem::zeroed();
        let dc = BeginPaint(window, &mut ps);
        if dc.is_null() {
            EndPaint(window, &ps);
            return;
        }
        let mut rect = RECT::default();
        GetClientRect(window, &mut rect);
        let background = CreateSolidBrush(data.background);
        FillRect(dc, &rect, background);
        DeleteObject(background);
        let px = |n: f32| (n * data.scale).round() as i32;
        let accent = CreateSolidBrush(data.accent);
        let line = RECT {
            left: px(17.0),
            top: px(20.0),
            right: px(20.0),
            bottom: rect.bottom - px(20.0),
        };
        FillRect(dc, &line, accent);
        DeleteObject(accent);
        SetBkMode(dc, TRANSPARENT as i32);
        let font = CreateFontW(
            -px(14.0),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            windows_sys::w!("Microsoft YaHei UI"),
        );
        let old = if font.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(dc, font)
        };
        let mut title = RECT {
            left: px(34.0),
            top: px(14.0),
            right: rect.right - px(20.0),
            bottom: px(35.0),
        };
        SetTextColor(dc, data.accent);
        DrawTextW(
            dc,
            windows_sys::w!("GalGear"),
            -1,
            &mut title,
            DT_SINGLELINE | DT_NOPREFIX,
        );
        let mut body = RECT {
            left: px(34.0),
            top: px(40.0),
            right: rect.right - px(20.0),
            bottom: rect.bottom - px(12.0),
        };
        SetTextColor(dc, data.foreground);
        DrawTextW(
            dc,
            data.text.as_ptr(),
            -1,
            &mut body,
            DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        if !font.is_null() {
            SelectObject(dc, old);
            DeleteObject(font);
        }
        EndPaint(window, &ps);
    }
}
