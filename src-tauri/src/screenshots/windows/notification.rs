//! 原生短提示沿用应用的明暗和配色，显示在游戏客户区右下角，不激活或抢焦点。
use super::{DpiContext, magpie};
use std::{
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
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

fn notice_bounds(client: RECT, scale: f32) -> Option<RECT> {
    let available_width = client.right - client.left;
    let available_height = client.bottom - client.top;
    if available_width <= 0 || available_height <= 0 {
        return None;
    }
    // 小窗口也保留提示在客户区内，避免固定尺寸将提示推到游戏之外。
    let margin = ((24.0 * scale) as i32).min((available_width.min(available_height) - 1) / 2);
    let width = ((400.0 * scale) as i32).min(available_width - 2 * margin);
    let height = ((100.0 * scale) as i32).min(available_height - 2 * margin);
    let right = client.right - margin;
    let bottom = client.bottom - margin;
    Some(RECT {
        left: right - width,
        top: bottom - height,
        right,
        bottom,
    })
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
        // SAFETY: 只查询目标窗口；坐标缓冲有效，DPI 上下文在本线程通过 RAII 恢复。
        unsafe {
            let dpi_context =
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if dpi_context.is_null() {
                return;
            }
            let _dpi = DpiContext(dpi_context);
            // 与采集共用 Magpie 输出关联，提示应落在玩家实际看到的画面内。
            let output = magpie::output(target);
            if IsWindowVisible(output) == 0 || IsIconic(output) != 0 {
                return;
            }
            let mut client = RECT::default();
            if GetClientRect(output, &mut client) == 0 {
                return;
            }
            let mut top_left = POINT {
                x: client.left,
                y: client.top,
            };
            let mut bottom_right = POINT {
                x: client.right,
                y: client.bottom,
            };
            if ClientToScreen(output, &mut top_left) == 0
                || ClientToScreen(output, &mut bottom_right) == 0
            {
                return;
            }
            let scale = GetDpiForWindow(output).max(96) as f32 / 96.0;
            let Some(bounds) = notice_bounds(
                RECT {
                    left: top_left.x,
                    top: top_left.y,
                    right: bottom_right.x,
                    bottom: bottom_right.y,
                },
                scale,
            ) else {
                return;
            };
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
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            self.window = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                CLASS,
                windows_sys::w!("GalGear"),
                WS_POPUP,
                bounds.left,
                bounds.top,
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

#[cfg(test)]
mod tests {
    use super::{Notification, notice_bounds};
    use crate::screenshots::windows::DpiContext;
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::ClientToScreen,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    };

    #[test]
    fn notice_uses_offset_game_client_corner() {
        let bounds = notice_bounds(
            RECT {
                left: 160,
                top: 120,
                right: 1440,
                bottom: 840,
            },
            1.0,
        )
        .unwrap();
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (1016, 716, 1416, 816)
        );
    }

    #[test]
    fn notice_scales_with_dpi_on_negative_monitor_coordinates() {
        let bounds = notice_bounds(
            RECT {
                left: -1800,
                top: -900,
                right: -200,
                bottom: 0,
            },
            1.5,
        )
        .unwrap();
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (-836, -186, -236, -36)
        );
    }

    #[test]
    fn notice_fits_small_clients_and_rejects_empty_clients() {
        let bounds = notice_bounds(
            RECT {
                left: 100,
                top: 80,
                right: 420,
                bottom: 200,
            },
            2.0,
        )
        .unwrap();
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (148, 128, 372, 152)
        );
        assert!(notice_bounds(RECT::default(), 1.0).is_none());
        assert!(
            notice_bounds(
                RECT {
                    left: 10,
                    top: 0,
                    right: 0,
                    bottom: 10
                },
                1.0
            )
            .is_none()
        );
    }

    #[test]
    #[ignore = "需要交互式 Windows 桌面；创建隔离窗口验证提示位置及焦点"]
    fn native_notice_stays_inside_game_client_without_activation() {
        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                // SAFETY: 仅销毁本测试在当前线程创建的窗口。
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        // SAFETY: 窗口和坐标缓冲均由测试拥有，窗口在 DPI 上下文恢复前销毁。
        unsafe {
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            assert!(!previous.is_null());
            let _dpi = DpiContext(previous);
            let target = TestWindow(CreateWindowExW(
                WS_EX_NOACTIVATE,
                windows_sys::w!("STATIC"),
                windows_sys::w!("GalGear notice positioning test"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                800,
                500,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            ));
            assert!(!target.0.is_null());
            ShowWindow(target.0, SW_SHOWNOACTIVATE);
            let mut notification = Notification::default();
            for (x, y, width, height, theme) in
                [(100, 100, 800, 500, "light"), (180, 140, 360, 260, "dark")]
            {
                assert_ne!(
                    SetWindowPos(
                        target.0,
                        ptr::null_mut(),
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOZORDER
                    ),
                    0
                );
                let foreground = GetForegroundWindow();
                notification.show(
                    target.0,
                    "截图已保存并复制到剪贴板。",
                    theme,
                    "white-pink",
                    false,
                );
                assert!(!notification.window.is_null());
                assert_ne!(IsWindowVisible(notification.window), 0);
                assert_eq!(GetForegroundWindow(), foreground, "提示不能抢占焦点");

                let mut client = RECT::default();
                assert_ne!(GetClientRect(target.0, &mut client), 0);
                let mut origin = POINT { x: 0, y: 0 };
                assert_ne!(ClientToScreen(target.0, &mut origin), 0);
                let mut actual = RECT::default();
                assert_ne!(GetWindowRect(notification.window, &mut actual), 0);
                let margin = (24.0 * GetDpiForWindow(target.0) as f32 / 96.0) as i32;
                assert_eq!(actual.right, origin.x + client.right - margin);
                assert_eq!(actual.bottom, origin.y + client.bottom - margin);
                assert!(actual.left >= origin.x && actual.top >= origin.y);
            }
            ShowWindow(target.0, SW_HIDE);
            notification.show(target.0, "hidden", "light", "white", false);
            assert!(
                notification.window.is_null(),
                "目标隐藏后不应回退到屏幕角落"
            );
        }
    }
}
