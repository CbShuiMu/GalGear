//! 桌面目录打开入口，避免资源管理器命令行对路径的二次解析。
use galgear_i18n::Message;
use std::path::Path;

fn directory_path(path: &Path) -> Result<std::path::PathBuf, Message> {
    if path.as_os_str().is_empty() || !path.is_dir() {
        return Err(Message::new("error.game_folder"));
    }
    path.canonicalize()
        .map_err(|error| Message::detail("error.open_folder", error))
}

/// 打开现有目录；不存在或无法交给系统打开时返回错误。
pub(crate) fn open(path: &Path) -> Result<(), Message> {
    let path = directory_path(path)?;
    // Shell 扩展需要 COM apartment；独立线程不改变 Tauri 线程的 COM 状态。
    std::thread::Builder::new()
        .name("open-folder".into())
        .spawn(move || launch(&path))
        .map_err(|error| Message::detail("error.open_folder", error))?
        .join()
        .map_err(|_| Message::new("error.open_folder"))?
}

#[cfg(windows)]
fn launch(path: &Path) -> Result<(), Message> {
    use windows_sys::Win32::{
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
        UI::{
            Shell::{
                SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
                SHParseDisplayName, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };
    fn check(result: i32) -> Result<(), Message> {
        if result < 0 {
            Err(Message::detail(
                "error.open_folder",
                format!("HRESULT {result:#010x}"),
            ))
        } else {
            Ok(())
        }
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: 仅在本线程成功初始化 COM 后创建此守卫。
            unsafe { CoUninitialize() };
        }
    }
    // SAFETY: 此独立线程尚未初始化 COM，保留参数为空。
    check(unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    })?;
    let _apartment = Apartment;
    let path: Vec<u16> = galgear_core::config::display_path(path)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut item = std::ptr::null_mut();
    // SAFETY: 路径已验证且以 NUL 终止，输出指针有效，COM apartment 存活。
    check(unsafe {
        SHParseDisplayName(
            path.as_ptr(),
            std::ptr::null_mut(),
            &mut item,
            0,
            std::ptr::null_mut(),
        )
    })?;
    let mut request = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: windows_sys::core::w!("open"),
        lpIDList: item.cast(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    // SAFETY: item 由 Shell 成功解析，在同步调用结束前保持有效，COM apartment 存活。
    let result = if unsafe { ShellExecuteExW(&mut request) } == 0 {
        Err(Message::detail(
            "error.open_folder",
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    };
    // SAFETY: item 由 SHParseDisplayName 分配，使用 COM 分配器释放一次。
    unsafe { CoTaskMemFree(item.cast()) };
    result
}

#[cfg(not(windows))]
fn launch(path: &Path) -> Result<(), Message> {
    open::that(path).map_err(|error| Message::detail("error.open_folder", error))
}

#[cfg(test)]
mod tests {
    use super::directory_path;
    use std::{fs, path::Path};

    #[test]
    fn resolves_directory_with_special_characters() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("存档 セーブ, data & (1)");
        fs::create_dir(&path).unwrap();
        assert_eq!(directory_path(&path).unwrap(), path.canonicalize().unwrap());
    }

    #[test]
    fn rejects_missing_directory_and_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("save.dat");
        assert!(directory_path(&path).is_err());
        fs::write(&path, b"test").unwrap();
        assert!(directory_path(&path).is_err());
        assert!(directory_path(Path::new("")).is_err());
    }

    #[test]
    #[ignore = "打开资源管理器，仅用于手动桌面验证"]
    fn opens_temporary_directory_on_desktop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("存档 セーブ, data & (1)");
        fs::create_dir(&path).unwrap();
        super::open(&path).unwrap();
        println!(
            "Opened directory: {}",
            temp.keep().join("存档 セーブ, data & (1)").display()
        );
    }
}
