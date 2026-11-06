//! 仅预览本机字体排版，不复制 Hook 的字符集、文字映射或引擎渲染规则。
use super::{profile_from_controls, wide};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        CreateFontW, DeleteObject, FillRect, GetTextMetricsW, HBRUSH, IntersectClipRect, RestoreDC,
        SaveDC, SelectObject, SetBkMode, SetTextCharacterExtra, SetTextColor, TEXTMETRICW,
        TRANSPARENT, TextOutW,
    },
    UI::Controls::DRAWITEMSTRUCT,
};

pub(super) fn draw(hwnd: HWND, item: &DRAWITEMSTRUCT, background: HBRUSH, text: u32) {
    // SAFETY: WM_DRAWITEM 提供本次绘制有效 DC；恢复选择对象后才释放创建的字体。
    unsafe {
        FillRect(item.hDC, &item.rcItem, background);
        let Ok(profile) = profile_from_controls(hwnd) else {
            return;
        };
        let scale = (item.rcItem.right - item.rcItem.left) as f32 / 430.0;
        let height = (22.0 * scale * profile.height_percent as f32 / 100.0).round() as i32;
        let face = wide(if profile.face.is_empty() {
            "Microsoft YaHei UI"
        } else {
            &profile.face
        });
        let font = CreateFontW(
            -height,
            0,
            0,
            0,
            profile.weight as i32,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        );
        if font.is_null() {
            return;
        }
        let saved = SaveDC(item.hDC);
        if saved == 0 {
            DeleteObject(font);
            return;
        }
        SelectObject(item.hDC, font);
        let mut metrics = TEXTMETRICW::default();
        GetTextMetricsW(item.hDC, &mut metrics);
        let stretched = CreateFontW(
            -height,
            (metrics.tmAveCharWidth as f32 * profile.width_percent as f32 / 100.0).round() as i32,
            0,
            0,
            profile.weight as i32,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr(),
        );
        if !stretched.is_null() {
            SelectObject(item.hDC, stretched);
        }
        IntersectClipRect(
            item.hDC,
            item.rcItem.left,
            item.rcItem.top,
            item.rcItem.right,
            item.rcItem.bottom,
        );
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, text);
        SetTextCharacterExtra(
            item.hDC,
            (profile.char_spacing as f32 * scale).round() as i32,
        );
        let step = profile
            .vertical_metrics
            .map_or(metrics.tmHeight, |m| height * (m.ascent - m.descent) / 1000)
            + (profile.line_spacing as f32 * scale).round() as i32;
        for (line, sample) in ["「测试文字 Sample Text」", "あいうえお 0123456789"]
            .iter()
            .enumerate()
        {
            let value = wide(sample);
            TextOutW(
                item.hDC,
                item.rcItem.left + 4,
                item.rcItem.top + 2 + line as i32 * step,
                value.as_ptr(),
                value.len() as i32 - 1,
            );
        }
        RestoreDC(item.hDC, saved);
        if !stretched.is_null() {
            DeleteObject(stretched);
        }
        DeleteObject(font);
    }
}
