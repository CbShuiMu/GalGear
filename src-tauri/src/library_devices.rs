//! 观察磁盘和游戏库挂载状态，变化时通知 UI 合并一次后台扫描。
use crate::{state::AppState, state::snapshot};
use std::time::Duration;
use tauri::{Emitter, Manager};

pub(crate) fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut previous = None;
        loop {
            let handle = app.clone();
            let observed = tauri::async_runtime::spawn_blocking(move || {
                let settings = snapshot(&handle.state::<AppState>())?;
                #[cfg(windows)]
                // SAFETY: GetLogicalDrives has no pointer arguments or caller-owned resources.
                let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
                #[cfg(windows)]
                if drives == 0 {
                    return Err(galgear_i18n::Message::detail(
                        "error.operation",
                        std::io::Error::last_os_error(),
                    ));
                }
                #[cfg(not(windows))]
                let drives = 0u32;
                let roots: Vec<_> = settings
                    .library_folders
                    .iter()
                    .map(|path| (path.clone(), std::path::Path::new(path).is_dir()))
                    .collect();
                Ok::<_, galgear_i18n::Message>((drives, roots))
            })
            .await;
            match observed {
                Ok(Ok(current)) => {
                    if previous.as_ref().is_some_and(|old| old != &current)
                        && let Err(error) = app.emit("library-devices-changed", ())
                    {
                        log::warn!("library_device_notification_failed {error}");
                    }
                    previous = Some(current);
                }
                error => log::warn!("library_device_check_failed {error:?}"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
