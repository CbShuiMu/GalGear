//! 观察磁盘和游戏库挂载状态，变化时通知 UI 合并一次后台扫描。
use crate::{state::AppState, state::snapshot};
use std::{collections::BTreeMap, time::Duration};
use tauri::{Emitter, Manager};

#[derive(Debug)]
struct DeviceSnapshot {
    drives: u32,
    roots: BTreeMap<String, bool>,
}

impl DeviceSnapshot {
    fn mount_changed_since(&self, previous: &Self) -> bool {
        // 库目录增删已由操作入口刷新。监视器只比较两次都存在的目录，避免重复导入。
        self.drives != previous.drives
            || self.roots.iter().any(|(path, available)| {
                previous
                    .roots
                    .get(path)
                    .is_some_and(|before| before != available)
            })
    }
}

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
                let roots = settings
                    .library_folders
                    .iter()
                    .map(|path| (path.clone(), std::path::Path::new(path).is_dir()))
                    .collect();
                Ok::<_, galgear_i18n::Message>(DeviceSnapshot { drives, roots })
            })
            .await;
            match observed {
                Ok(Ok(current)) => {
                    if previous
                        .as_ref()
                        .is_some_and(|old| current.mount_changed_since(old))
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

#[cfg(test)]
mod tests {
    use super::DeviceSnapshot;

    fn snapshot(drives: u32, roots: &[(&str, bool)]) -> DeviceSnapshot {
        DeviceSnapshot {
            drives,
            roots: roots
                .iter()
                .map(|(path, available)| ((*path).to_owned(), *available))
                .collect(),
        }
    }

    #[test]
    fn library_configuration_changes_do_not_request_another_scan() {
        let mut previous = snapshot(0b1100, &[]);
        for roots in [
            vec![("C:/Games", true)],
            vec![("C:/Games", true), ("D:/Games", false)],
            vec![("D:/Games", false), ("C:/Games", true)],
            vec![("D:/Games", false)],
            vec![],
        ] {
            let current = snapshot(0b1100, &roots);
            assert!(!current.mount_changed_since(&previous));
            previous = current;
        }
    }

    #[test]
    fn unchanged_mounts_do_not_request_another_scan() {
        let current = snapshot(0b1100, &[("C:/Games", true), ("D:/Games", false)]);
        assert!(!current.mount_changed_since(&current));
    }

    #[test]
    fn added_library_is_monitored_after_its_first_observation() {
        let empty = snapshot(0b1100, &[]);
        let added = snapshot(0b1100, &[("D:/Games", true)]);
        let disconnected = snapshot(0b1100, &[("D:/Games", false)]);
        assert!(!added.mount_changed_since(&empty));
        assert!(disconnected.mount_changed_since(&added));
        assert!(added.mount_changed_since(&disconnected));
    }

    #[test]
    fn configuration_changes_do_not_hide_existing_mount_changes() {
        let previous = snapshot(0b1100, &[("C:/Games", true), ("D:/Old", true)]);
        let current = snapshot(0b1100, &[("C:/Games", false), ("D:/New", true)]);
        assert!(current.mount_changed_since(&previous));
        assert!(previous.mount_changed_since(&current));
    }

    #[test]
    fn drive_connection_changes_request_a_scan_without_library_roots() {
        let previous = snapshot(0b0100, &[]);
        let current = snapshot(0b1100, &[]);
        assert!(current.mount_changed_since(&previous));
        assert!(previous.mount_changed_since(&current));
    }
}
