//! 将本机显示字体名转换为字体自身的英文家族名，避免转区进程使用不同本地化别名。
//! 只读取 name 表，不修补字体、不改变游戏字符集、不承担 Hook 规则。
use galgear_i18n::Message;
use std::ptr;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontW, DeleteDC, DeleteObject, GDI_ERROR, GetFontData, SelectObject,
};

pub(super) fn canonical_face(face: &str) -> Result<String, Message> {
    if face.is_empty() {
        return Ok(String::new());
    }
    let wide: Vec<_> = face.encode_utf16().chain(Some(0)).collect();
    // SAFETY: 私有内存 DC 仅用于同步字体查询，恢复选择对象后释放所有 GDI 资源。
    let data = unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        let font = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, wide.as_ptr());
        if dc.is_null() || font.is_null() {
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !font.is_null() {
                DeleteObject(font);
            }
            return Err(Message::detail(
                "font.error",
                std::io::Error::last_os_error(),
            ));
        }
        let previous = SelectObject(dc, font);
        let size = GetFontData(dc, 0x656d616e, 0, ptr::null_mut(), 0);
        let result = if size != GDI_ERROR as u32 && size <= 65536 {
            let mut bytes = vec![0; size as usize];
            (GetFontData(dc, 0x656d616e, 0, bytes.as_mut_ptr().cast(), size) == size)
                .then_some(bytes)
        } else {
            None
        };
        SelectObject(dc, previous);
        DeleteObject(font);
        DeleteDC(dc);
        result
    };
    // Raster/device fonts may have no name table; the backend still validates the request.
    let Some(data) = data else {
        return Ok(face.into());
    };
    Ok(english_family(&data, face).unwrap_or_else(|| face.into()))
}

fn english_family(data: &[u8], requested: &str) -> Option<String> {
    let word = |offset: usize| -> Option<usize> {
        Some(u16::from_be_bytes(data.get(offset..offset + 2)?.try_into().ok()?) as usize)
    };
    let count = word(2)?;
    let strings = word(4)?;
    let mut matched = false;
    let mut english = None;
    for i in 0..count {
        let record = 6 + i * 12;
        // Windows Unicode family records; only use the exact font selected by the user.
        if word(record)? != 3 || ![1, 10].contains(&word(record + 2)?) || word(record + 6)? != 1 {
            continue;
        }
        let start = strings.checked_add(word(record + 10)?)?;
        let end = start.checked_add(word(record + 8)?)?;
        let bytes = data.get(start..end)?;
        if !bytes.len().is_multiple_of(2) {
            return None;
        }
        let value = String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|v| u16::from_be_bytes([v[0], v[1]]))
                .collect::<Vec<_>>(),
        )
        .ok()?;
        matched |= value.to_lowercase() == requested.to_lowercase();
        if word(record + 4)? == 0x409 && value.encode_utf16().count() <= 31 {
            english = Some(value);
        }
    }
    if matched { english } else { None }
}

#[cfg(test)]
mod tests {
    use super::english_family;
    #[test]
    fn aliases_require_an_exact_family_match_and_valid_offsets() {
        let mut table = vec![0; 30];
        table[3] = 2;
        table[5] = 30;
        for (i, (language, name)) in [(0x409u16, "SimHei"), (0x804, "黑体")]
            .into_iter()
            .enumerate()
        {
            let offset = table.len() - 30;
            let bytes: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
            for (j, value) in [3, 1, language, 1, bytes.len() as u16, offset as u16]
                .into_iter()
                .enumerate()
            {
                table[6 + i * 12 + j * 2..8 + i * 12 + j * 2].copy_from_slice(&value.to_be_bytes());
            }
            table.extend(bytes);
        }
        assert_eq!(english_family(&table, "黑体").as_deref(), Some("SimHei"));
        assert_eq!(english_family(&table, "not this font"), None);
        table[28] = 255;
        assert_eq!(english_family(&table, "黑体"), None);
    }
}
