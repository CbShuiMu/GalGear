//! 前台游戏识别与实际显示客户区，供截图和游戏内工具共用。
use crate::state::AppState;
use std::path::Path;
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    UI::WindowsAndMessaging::{
        GetClientRect, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    },
};
pub(crate) mod magpie;
pub(crate) fn foreground() -> Option<(HWND, std::path::PathBuf)> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    };
    // SAFETY: 系统填充有效缓冲；进程查询句柄不跨线程，所有分支均关闭。
    unsafe {
        let window = magpie::source(GetForegroundWindow());
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        if window.is_null() || pid == std::process::id() || IsIconic(window) != 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0; 32768];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        let path = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
        Some((window, path.canonicalize().ok()?))
    }
}

pub(crate) fn foreground_game(
    state: &AppState,
) -> Option<(windows_sys::Win32::Foundation::HWND, galgear_core::Game)> {
    let (window, executable) = foreground()?;
    // 只在复制目录元数据时持锁；后续文件系统访问不持有目录锁。
    let games = state.catalog.lock().ok()?.games.clone();
    let game = games
        .into_iter()
        .filter(|game| {
            game.executable
                .as_ref()
                .is_some_and(|exe| Path::new(exe).canonicalize().is_ok_and(|p| p == executable))
                || Path::new(&game.folder)
                    .canonicalize()
                    .is_ok_and(|root| executable.starts_with(root))
        })
        .max_by_key(|game| game.folder.len())?;
    Some((window, game))
}

pub(crate) fn bounds(window: HWND) -> Option<RECT> {
    // SAFETY: 仅查询窗口，所有输出缓冲位于本线程；调用者采用 per-monitor DPI 上下文。
    unsafe {
        let output = magpie::output(window);
        if IsWindowVisible(output) == 0 || IsIconic(output) != 0 {
            return None;
        }
        let mut rect = RECT::default();
        let mut origin = POINT { x: 0, y: 0 };
        if GetClientRect(output, &mut rect) == 0 || ClientToScreen(output, &mut origin) == 0 {
            return None;
        }
        if rect.right <= 0 || rect.bottom <= 0 {
            return None;
        }
        Some(RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x + rect.right,
            bottom: origin.y + rect.bottom,
        })
    }
}
