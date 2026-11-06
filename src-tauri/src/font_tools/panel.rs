//! Rust 原生字体面板；控件只产生动作，业务与后台任务由服务编排。
use crate::game_window;
use galgear_core::{
    Settings,
    font_tools::{CharsetSpoof, Mapping, Profile, VerticalMetrics},
};
use galgear_i18n::{Language, Message, tr};
use std::{collections::BTreeSet, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{
        CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DeleteObject, EnumFontFamiliesExW,
        FillRect, GetDC, HBRUSH, HFONT, InvalidateRect, LOGFONTW, ReleaseDC, SetBkColor, SetBkMode,
        SetTextColor, TEXTMETRICW, TRANSPARENT,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        SystemServices::{SS_ENDELLIPSIS, SS_OWNERDRAW},
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, SetWindowTheme},
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{
            BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_OWNERDRAW, CB_ADDSTRING, CB_GETCURSEL,
            CB_LIMITTEXT, CB_SETCURSEL, CBS_AUTOHSCROLL, CBS_DROPDOWN, CBS_DROPDOWNLIST,
            CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, ES_MULTILINE,
            ES_READONLY, GA_ROOT, GWLP_USERDATA, GetAncestor, GetClientRect, GetDlgItem,
            GetForegroundWindow, GetWindowLongPtrW, GetWindowTextW, IDC_ARROW, IsChild,
            IsDialogMessageW, IsWindow, IsWindowVisible, LoadCursorW, MSG, RegisterClassW, SW_HIDE,
            SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow,
            SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND,
            WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DRAWITEM,
            WM_ERASEBKGND, WM_KEYDOWN, WM_NCCREATE, WM_NCDESTROY, WM_SETFONT, WNDCLASSW, WS_BORDER,
            WS_CHILD, WS_EX_CONTROLPARENT, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP,
            WS_VISIBLE, WS_VSCROLL,
        },
    },
};

mod paint;
mod preview;

const CLASS: windows_sys::core::PCWSTR = windows_sys::w!("GalGear.FontPanel.v1");
#[cfg(test)]
mod tests;
#[derive(Clone, Copy)]
pub(super) enum Action {
    Apply,
    Reset,
}
struct Content {
    action: Option<Action>,
    background: HBRUSH,
    background_color: u32,
    dark: bool,
    text: u32,
    close: bool,
    preview_dirty: bool,
}
pub(super) struct Panel {
    hwnd: HWND,
    target: HWND,
    content: Box<Content>,
    font: HFONT,
    language: Language,
    scale: f32,
    layout: Vec<(HWND, [i32; 4])>,
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
impl Panel {
    pub(super) fn new(target: HWND, title: &str, settings: &Settings) -> Result<Self, Message> {
        let bounds = game_window::bounds(target).ok_or_else(|| Message::new("font.no_window"))?;
        // SAFETY: 只查询目标窗口 DPI，不持有窗口资源。
        let dpi = unsafe { GetDpiForWindow(game_window::magpie::output(target)) };
        let scale = (dpi.max(96) as f32 / 96.0)
            .min((bounds.right - bounds.left - 16) as f32 / 460.0)
            .min((bounds.bottom - bounds.top - 16) as f32 / 700.0);
        if scale < 0.65 {
            return Err(Message::new("font.window_small"));
        }
        let dark = settings.theme == "dark";
        // SAFETY: 本线程拥有全部 HWND/GDI 资源，Content 在销毁窗口后才释放。
        unsafe {
            let background_color = if dark { 0x2a1c22 } else { 0xfcf8ff };
            let background = CreateSolidBrush(background_color);
            let mut content = Box::new(Content {
                action: None,
                background,
                background_color,
                dark,
                text: if dark { 0xf2eaf5 } else { 0x3f3144 },
                close: false,
                preview_dirty: true,
            });
            let module = GetModuleHandleW(ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module,
                lpszClassName: CLASS,
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_CONTROLPARENT,
                CLASS,
                wide(tr(settings.language, "font.title")).as_ptr(),
                WS_POPUP | WS_BORDER,
                0,
                0,
                (460.0 * scale) as i32,
                (700.0 * scale) as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                (&mut *content as *mut Content).cast(),
            );
            if hwnd.is_null() {
                DeleteObject(background);
                return Err(Message::detail(
                    "font.error",
                    std::io::Error::last_os_error(),
                ));
            }
            let font = CreateFontW(
                -(14.0 * scale).round() as i32,
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
            let mut panel = Self {
                hwnd,
                target,
                content,
                font,
                language: settings.language,
                scale,
                layout: Vec::new(),
            };
            let mut layout = Vec::new();
            let mut add = |class: windows_sys::core::PCWSTR,
                           text: &str,
                           id: usize,
                           style: u32,
                           x: i32,
                           y: i32,
                           w: i32,
                           h: i32| {
                let control = CreateWindowExW(
                    0,
                    class,
                    wide(text).as_ptr(),
                    WS_CHILD
                        | WS_VISIBLE
                        | style
                        | if matches!(id, 1 | 2 | 9) {
                            BS_OWNERDRAW as u32
                        } else {
                            0
                        },
                    (x as f32 * scale) as i32,
                    (y as f32 * scale) as i32,
                    (w as f32 * scale) as i32,
                    (h as f32 * scale) as i32,
                    hwnd,
                    id as _,
                    module,
                    ptr::null(),
                );
                SetWindowTheme(control, windows_sys::w!(""), windows_sys::w!(""));
                layout.push((control, [x, y, w, h]));
                SendMessageW(control, WM_SETFONT, font as usize, 1);
                control
            };
            add(
                windows_sys::w!("STATIC"),
                &format!("{} · {}", tr(settings.language, "font.title"), settings.font_hotkey),
                0,
                0,
                14,
                10,
                395,
                24,
            );
            add(
                windows_sys::w!("BUTTON"),
                "×",
                9,
                WS_TABSTOP,
                420,
                7,
                26,
                26,
            );
            add(
                windows_sys::w!("STATIC"),
                title,
                0,
                SS_ENDELLIPSIS,
                14,
                38,
                430,
                23,
            );
            add(
                windows_sys::w!("STATIC"),
                tr(settings.language, "font.face"),
                0,
                0,
                14,
                69,
                430,
                22,
            );
            let combo = add(
                windows_sys::w!("COMBOBOX"),
                "",
                10,
                WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWN as u32 | CBS_AUTOHSCROLL as u32,
                14,
                94,
                430,
                210,
            );
            SendMessageW(combo, CB_LIMITTEXT, 31, 0);
            for face in font_names() {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(&face).as_ptr() as isize);
            }
            for (id, key, value, x, y) in [
                (11, "font.height", "100", 14, 128),
                (12, "font.width", "100", 238, 128),
                (13, "font.char_spacing", "0", 14, 184),
                (14, "font.line_spacing", "0", 238, 184),
                (15, "font.weight", "400", 14, 240),
                (16, "font.ascent", "880", 14, 296),
                (17, "font.descent", "-120", 238, 296),
            ] {
                add(
                    windows_sys::w!("STATIC"),
                    tr(settings.language, key),
                    0,
                    0,
                    x,
                    y,
                    206,
                    22,
                );
                add(
                    windows_sys::w!("EDIT"),
                    value,
                    id,
                    WS_TABSTOP | WS_BORDER,
                    x,
                    y + 23,
                    206,
                    27,
                );
            }
            for (key, x) in [("font.ascent_help", 14), ("font.descent_help", 238)] {
                add(
                    windows_sys::w!("STATIC"),
                    tr(settings.language, key),
                    0,
                    0,
                    x,
                    350,
                    206,
                    22,
                );
            }
            for (id, key, x, y) in [(32, "font.vertical", 238, 252), (33, "font.spoof", 14, 377)] {
                add(
                    windows_sys::w!("BUTTON"),
                    tr(settings.language, key),
                    id,
                    WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                    x,
                    y,
                    206,
                    26,
                );
            }
            for (id, x, y, items) in [
                (
                    34,
                    14,
                    407,
                    vec![
                        "Default (1)",
                        "Shift-JIS (128)",
                        "Korean (129)",
                        "GB2312 (134)",
                        "Big5 (136)",
                    ],
                ),
                (
                    35,
                    238,
                    407,
                    vec![
                        "Default (1)",
                        "Shift-JIS (128)",
                        "Korean (129)",
                        "GB2312 (134)",
                        "Big5 (136)",
                    ],
                ),
                (
                    36,
                    14,
                    465,
                    vec![
                        tr(settings.language, "font.mapping_off"),
                        tr(settings.language, "font.mapping_jp"),
                        tr(settings.language, "font.mapping_t2s"),
                        tr(settings.language, "font.mapping_s2t"),
                    ],
                ),
                (
                    37,
                    238,
                    465,
                    vec![
                        "ACP (0)",
                        "Shift-JIS (932)",
                        "GBK (936)",
                        "Korean (949)",
                        "Big5 (950)",
                        "UTF-8 (65001)",
                    ],
                ),
            ] {
                let control = add(
                    windows_sys::w!("COMBOBOX"),
                    "",
                    id,
                    WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
                    x,
                    y,
                    206,
                    180,
                );
                for item in items {
                    SendMessageW(control, CB_ADDSTRING, 0, wide(item).as_ptr() as isize);
                }
                SendMessageW(control, CB_SETCURSEL, 0, 0);
            }
            add(windows_sys::w!("STATIC"), "→", 0, 0, 222, 410, 16, 22);
            add(
                windows_sys::w!("STATIC"),
                tr(settings.language, "font.mapping"),
                0,
                0,
                14,
                442,
                206,
                22,
            );
            add(
                windows_sys::w!("STATIC"),
                tr(settings.language, "font.codepage"),
                0,
                0,
                238,
                442,
                206,
                22,
            );
            add(
                windows_sys::w!("STATIC"),
                tr(settings.language, "font.preview"),
                0,
                0,
                14,
                504,
                430,
                22,
            );
            add(
                windows_sys::w!("STATIC"),
                "",
                40,
                SS_OWNERDRAW,
                14,
                530,
                430,
                68,
            );
            add(
                windows_sys::w!("BUTTON"),
                tr(settings.language, "font.apply"),
                1,
                WS_TABSTOP,
                14,
                610,
                206,
                32,
            );
            add(
                windows_sys::w!("BUTTON"),
                tr(settings.language, "font.reset"),
                2,
                WS_TABSTOP,
                238,
                610,
                206,
                32,
            );
            add(
                windows_sys::w!("EDIT"),
                tr(settings.language, "font.hint"),
                20,
                WS_TABSTOP | ES_MULTILINE as u32 | ES_READONLY as u32 | WS_VSCROLL,
                14,
                654,
                430,
                32,
            );
            if layout.iter().any(|(hwnd, _)| hwnd.is_null()) {
                return Err(Message::detail(
                    "font.error",
                    std::io::Error::last_os_error(),
                ));
            }
            panel.layout = layout;
            panel.follow();
            Ok(panel)
        }
    }
    pub(super) fn language(&self) -> Language {
        self.language
    }
    pub(super) fn status(&mut self, text: &str) {
        self.set(20, text);
    }
    fn set(&self, id: i32, text: &str) {
        // SAFETY: 控件属于本面板，字符串包含 NUL 终止符。
        unsafe {
            SetWindowTextW(GetDlgItem(self.hwnd, id), wide(text).as_ptr());
        }
    }
    pub(super) fn set_profile(&self, p: &Profile) {
        self.set(10, &p.face);
        for (id, value) in [
            (11, p.height_percent as i32),
            (12, p.width_percent as i32),
            (13, p.char_spacing),
            (14, p.line_spacing),
            (15, p.weight as i32),
            (16, p.vertical_metrics.map_or(880, |m| m.ascent)),
            (17, p.vertical_metrics.map_or(-120, |m| m.descent)),
        ] {
            self.set(id, &value.to_string());
        }
        // SAFETY: 消息仅发送到本线程拥有的标准控件。
        unsafe {
            SendMessageW(
                GetDlgItem(self.hwnd, 32),
                BM_SETCHECK,
                usize::from(p.vertical_metrics.is_some()),
                0,
            );
            SendMessageW(
                GetDlgItem(self.hwnd, 33),
                BM_SETCHECK,
                usize::from(p.charset_spoof.is_some()),
                0,
            );
            let spoof = p
                .charset_spoof
                .unwrap_or(CharsetSpoof { from: 134, to: 128 });
            for (id, value) in [(34, spoof.from), (35, spoof.to)] {
                let index = [1, 128, 129, 134, 136]
                    .iter()
                    .position(|v| *v == value)
                    .unwrap_or(0);
                SendMessageW(GetDlgItem(self.hwnd, id), CB_SETCURSEL, index, 0);
            }
            SendMessageW(
                GetDlgItem(self.hwnd, 36),
                CB_SETCURSEL,
                p.mapping as usize,
                0,
            );
            let index = [0, 932, 936, 949, 950, 65001]
                .iter()
                .position(|v| *v == p.mapping_codepage)
                .unwrap_or(1);
            SendMessageW(GetDlgItem(self.hwnd, 37), CB_SETCURSEL, index, 0);
            InvalidateRect(GetDlgItem(self.hwnd, 40), ptr::null(), 1);
        }
    }
    pub(super) fn profile(&self) -> Result<Profile, Message> {
        profile_from_controls(self.hwnd)
    }
    pub(super) fn owns(&self, hwnd: HWND) -> bool {
        // SAFETY: 仅查询窗口关系，不取得外部句柄所有权。
        unsafe { hwnd == self.hwnd || IsChild(self.hwnd, hwnd) != 0 }
    }
    pub(super) fn toggle(&mut self) {
        // SAFETY: 仅操作本线程窗口；关闭后恢复已验证目标的焦点。
        unsafe {
            if IsWindowVisible(self.hwnd) != 0 {
                self.hide();
            } else if self.follow() {
                ShowWindow(self.hwnd, SW_SHOW);
                SetForegroundWindow(self.hwnd);
                SetFocus(GetDlgItem(self.hwnd, 10));
            }
        }
    }
    fn hide(&self) {
        // SAFETY: 隐藏自己拥有的窗口；仅当当前焦点属于面板时恢复游戏焦点。
        unsafe {
            let focused = self.owns(GetForegroundWindow());
            ShowWindow(self.hwnd, SW_HIDE);
            if focused && IsWindow(self.target) != 0 {
                SetForegroundWindow(GetAncestor(
                    game_window::magpie::output(self.target),
                    GA_ROOT,
                ));
            }
        }
    }
    pub(super) fn follow(&mut self) -> bool {
        // SAFETY: 更新本线程窗口的位置，不改变目标大小、Z 序或焦点。
        unsafe {
            if self.content.close {
                self.content.close = false;
                self.hide();
            }
            if let Some(b) = game_window::bounds(self.target) {
                let scale = (GetDpiForWindow(game_window::magpie::output(self.target)).max(96)
                    as f32
                    / 96.0)
                    .min((b.right - b.left - 16) as f32 / 460.0)
                    .min((b.bottom - b.top - 16) as f32 / 700.0);
                if scale < 0.65 {
                    self.hide();
                    return false;
                }
                if (scale - self.scale).abs() > 0.01 {
                    let font = CreateFontW(
                        -(14.0 * scale).round() as i32,
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
                    for (control, [x, y, w, h]) in &self.layout {
                        SetWindowPos(
                            *control,
                            ptr::null_mut(),
                            (*x as f32 * scale) as i32,
                            (*y as f32 * scale) as i32,
                            (*w as f32 * scale) as i32,
                            (*h as f32 * scale) as i32,
                            SWP_NOACTIVATE | SWP_NOZORDER,
                        );
                        SendMessageW(*control, WM_SETFONT, font as usize, 1);
                    }
                    DeleteObject(self.font);
                    self.font = font;
                    self.scale = scale;
                }
                SetWindowPos(
                    self.hwnd,
                    ptr::null_mut(),
                    b.right - (460.0 * self.scale) as i32 - 8,
                    b.top + 8,
                    (460.0 * self.scale) as i32,
                    (700.0 * self.scale) as i32,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            } else {
                self.hide();
                return false;
            }
        }
        true
    }
    pub(super) fn action(&mut self) -> Option<Action> {
        if self.content.preview_dirty {
            self.content.preview_dirty = false;
            // SAFETY: 预览控件属于当前面板，重绘不提交游戏配置。
            unsafe {
                InvalidateRect(GetDlgItem(self.hwnd, 40), ptr::null(), 1);
            }
        }
        self.content.action.take()
    }
    pub(super) fn message(&self, msg: &mut MSG) -> bool {
        // SAFETY: 消息来自当前线程队列；仅处理面板与其子控件的键盘消息。
        unsafe {
            if !self.owns(msg.hwnd) {
                return false;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == 27 {
                self.hide();
                return true;
            }
            IsDialogMessageW(self.hwnd, msg) != 0
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        // SAFETY: 控件先销毁，再释放其使用的字体和画刷，均由本线程创建。
        unsafe {
            DestroyWindow(self.hwnd);
            DeleteObject(self.font);
            DeleteObject(self.content.background);
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: Windows 传入有效创建参数；Box 的地址固定且活到 DestroyWindow 返回。
    unsafe {
        if msg == WM_NCCREATE {
            let c = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
        }
        let data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Content;
        if !data.is_null() {
            if msg == WM_DRAWITEM && matches!(wp, 1 | 2 | 9) {
                paint::button(&*(lp as *const DRAWITEMSTRUCT), (*data).dark, (*data).text);
                return 1;
            }
            if msg == WM_DRAWITEM && wp == 40 {
                preview::draw(
                    hwnd,
                    &*(lp as *const DRAWITEMSTRUCT),
                    (*data).background,
                    (*data).text,
                );
                return 1;
            }
            if msg == WM_COMMAND {
                (*data).preview_dirty = true;
            }
            if msg == WM_COMMAND && (wp >> 16) == 0 {
                match wp & 0xffff {
                    1 => (*data).action = Some(Action::Apply),
                    2 => (*data).action = Some(Action::Reset),
                    9 => (*data).close = true,
                    _ => {}
                }
                return 0;
            }
            if msg == WM_CLOSE {
                (*data).close = true;
                return 0;
            }
            if msg == WM_ERASEBKGND {
                let mut rect = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                FillRect(wp as _, &rect, (*data).background);
                return 1;
            }
            if matches!(
                msg,
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN
            ) {
                SetTextColor(wp as _, (*data).text);
                SetBkColor(wp as _, (*data).background_color);
                SetBkMode(wp as _, TRANSPARENT as i32);
                return (*data).background as isize;
            }
        }
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}
fn font_names() -> BTreeSet<String> {
    unsafe extern "system" fn collect(
        font: *const LOGFONTW,
        _: *const TEXTMETRICW,
        _: u32,
        data: LPARAM,
    ) -> i32 {
        // SAFETY: EnumFontFamiliesExW 同步调用回调，data 为有效集合，字体指针由 GDI 提供。
        unsafe {
            let name = (*font).lfFaceName;
            let n = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            let name = String::from_utf16_lossy(&name[..n]);
            if !name.starts_with('@') {
                (*(data as *mut BTreeSet<String>)).insert(name);
            }
        }
        1
    }
    let mut names = BTreeSet::new();
    // SAFETY: DC 只用于同步枚举并及时释放；集合指针在整个调用期间有效。
    unsafe {
        let dc = GetDC(ptr::null_mut());
        if !dc.is_null() {
            let font = LOGFONTW {
                lfCharSet: DEFAULT_CHARSET,
                ..std::mem::zeroed()
            };
            EnumFontFamiliesExW(dc, &font, Some(collect), &mut names as *mut _ as isize, 0);
            ReleaseDC(ptr::null_mut(), dc);
        }
    }
    names
}

fn profile_from_controls(hwnd: HWND) -> Result<Profile, Message> {
    fn number<T: std::str::FromStr>(hwnd: HWND, id: i32) -> Result<T, Message> {
        control_text(hwnd, id)
            .trim()
            .parse()
            .map_err(|_| Message::new("font.invalid"))
    }
    fn selected(hwnd: HWND, id: i32, values: &[u32]) -> Result<u32, Message> {
        // SAFETY: 查询本线程拥有的组合框，不保留控件指针。
        let index = unsafe { SendMessageW(GetDlgItem(hwnd, id), CB_GETCURSEL, 0, 0) };
        values
            .get(index as usize)
            .copied()
            .ok_or_else(|| Message::new("font.invalid"))
    }
    // SAFETY: 查询本面板的标准复选框。
    let checked = |id| unsafe { SendMessageW(GetDlgItem(hwnd, id), BM_GETCHECK, 0, 0) == 1 };
    let profile = Profile {
        face: control_text(hwnd, 10).trim().into(),
        height_percent: number(hwnd, 11)?,
        width_percent: number(hwnd, 12)?,
        char_spacing: number(hwnd, 13)?,
        line_spacing: number(hwnd, 14)?,
        weight: number(hwnd, 15)?,
        vertical_metrics: if checked(32) {
            Some(VerticalMetrics {
                ascent: number(hwnd, 16)?,
                descent: number(hwnd, 17)?,
            })
        } else {
            None
        },
        charset_spoof: if checked(33) {
            Some(CharsetSpoof {
                from: selected(hwnd, 34, &[1, 128, 129, 134, 136])?,
                to: selected(hwnd, 35, &[1, 128, 129, 134, 136])?,
            })
        } else {
            None
        },
        mapping: match selected(hwnd, 36, &[0, 1, 2, 3])? {
            1 => Mapping::Japanese,
            2 => Mapping::TraditionalToSimplified,
            3 => Mapping::SimplifiedToTraditional,
            _ => Mapping::Off,
        },
        mapping_codepage: selected(hwnd, 37, &[0, 932, 936, 949, 950, 65001])?,
    };
    profile.validate()?;
    Ok(profile)
}

fn control_text(hwnd: HWND, id: i32) -> String {
    // SAFETY: 有界 UTF-16 缓冲；仅查询本线程控件。
    unsafe {
        let mut buffer = [0u16; 128];
        let n = GetWindowTextW(GetDlgItem(hwnd, id), buffer.as_mut_ptr(), 128);
        String::from_utf16_lossy(&buffer[..n as usize])
    }
}
