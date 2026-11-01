//! 通过 Magpie 公开的窗口属性关联源游戏与输出；不按窗口标题或屏幕尺寸猜测。
use std::ptr;
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::*};

const CLASS: windows_sys::core::PCWSTR =
    windows_sys::w!("Window_Magpie_967EB565-6F73-4E94-AE53-00CC42592A22");

fn scaling_window() -> HWND {
    // SAFETY: 只查询系统窗口，固定类名以 NUL 结尾。
    unsafe { FindWindowW(CLASS, ptr::null()) }
}

fn associated_source(scaling: HWND) -> HWND {
    // Magpie v0.13 的 ScalingWindow::_SetWindowProps 提供此跨进程接口。
    // 旧版本未发布属性时使用创建缩放窗口时指定的 owner。
    // SAFETY: 属性只被当作窗口句柄查询，绝不解引用外部进程的指针。
    unsafe {
        let source = GetPropW(scaling, windows_sys::w!("Magpie.SrcHWND"));
        if source.is_null() {
            GetWindow(scaling, GW_OWNER)
        } else {
            source
        }
    }
}

pub(super) fn source(foreground: HWND) -> HWND {
    let scaling = scaling_window();
    if !scaling.is_null() && foreground == scaling {
        let source = associated_source(scaling);
        // SAFETY: 对外部窗口句柄作有效性查询，不获取其所有权。
        if unsafe { IsWindow(source) } != 0 {
            return source;
        }
    }
    foreground
}

pub(super) fn output(game: HWND) -> HWND {
    let scaling = scaling_window();
    // SAFETY: 系统查询处理已关闭或无效句柄；调用者在采集后再次验证关联。
    unsafe {
        if scaling.is_null()
            || IsWindowVisible(scaling) == 0
            || IsIconic(scaling) != 0
            || associated_source(scaling) != game
        {
            return game;
        }
        // 窗口模式由子窗口绘制，排除 Magpie 自身边框；全屏直接采集输出客户区。
        let renderer = FindWindowExW(
            scaling,
            ptr::null_mut(),
            windows_sys::w!("Magpie_Renderer"),
            ptr::null(),
        );
        if !renderer.is_null() && IsWindowVisible(renderer) != 0 {
            renderer
        } else {
            scaling
        }
    }
}
