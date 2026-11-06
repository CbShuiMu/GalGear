//! 原生按钮使用主界面的粉色强调色和梅紫色表面，保留标准键盘焦点。
use super::{control_text, wide};
use windows_sys::Win32::{
    Graphics::Gdi::{
        CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect,
        DrawTextW, FillRect, RestoreDC, SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED},
        WindowsAndMessaging::{GetParent, SendMessageW, WM_GETFONT},
    },
};

pub(super) fn button(item: &DRAWITEMSTRUCT, dark: bool, text: u32) {
    // SAFETY: 绘制资源只在 WM_DRAWITEM 的有效 DC 内使用；结束时恢复 DC 并释放画刷。
    unsafe {
        let primary = item.CtlID == 1;
        let color = if primary {
            if item.itemState & ODS_SELECTED != 0 {
                0x7340ac
            } else {
                0x955cd7
            }
        } else if dark {
            0x443348
        } else {
            0xeee5f8
        };
        let brush = CreateSolidBrush(color);
        FillRect(item.hDC, &item.rcItem, brush);
        DeleteObject(brush);
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            return;
        }
        SelectObject(item.hDC, SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as _);
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, if primary { 0xffffff } else { text });
        let value = wide(&control_text(GetParent(item.hwndItem), item.CtlID as i32));
        let mut rect = item.rcItem;
        DrawTextW(
            item.hDC,
            value.as_ptr(),
            value.len() as i32 - 1,
            &mut rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
        if item.itemState & ODS_FOCUS != 0 {
            rect.left += 3;
            rect.top += 3;
            rect.right -= 3;
            rect.bottom -= 3;
            DrawFocusRect(item.hDC, &rect);
        }
        RestoreDC(item.hDC, saved);
    }
}
