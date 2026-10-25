//! Open a generated SVG with the default web browser's registered launch behavior.
use galgear_i18n::Message;
use std::path::Path;

fn failed() -> Message {
    Message::new("route.open_failed")
}

fn svg_url(path: &Path) -> Result<reqwest::Url, Message> {
    if path.extension().is_none_or(|extension| extension != "svg") || !path.is_file() {
        return Err(failed());
    }
    let path = path.canonicalize().map_err(|_| failed())?;
    reqwest::Url::from_file_path(path).map_err(|_| failed())
}

pub(crate) fn open_svg(path: &Path) -> Result<(), Message> {
    let url = svg_url(path)?;
    // Shell extensions may require a COM apartment. A fresh thread keeps its
    // lifetime independent of Tauri's async runtime and its existing apartments.
    std::thread::Builder::new()
        .name("route-map-browser".into())
        .spawn(move || launch(&url))
        .map_err(|_| failed())?
        .join()
        .map_err(|_| failed())?
}

#[cfg(windows)]
fn shell_request(
    file: &[u16],
    browser: &[u16],
) -> windows_sys::Win32::UI::Shell::SHELLEXECUTEINFOW {
    use windows_sys::{
        Win32::UI::{
            Shell::{SEE_MASK_CLASSNAME, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
        core::w,
    };
    SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_CLASSNAME | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: w!("open"),
        lpFile: file.as_ptr(),
        // Force the protocol handler instead of the user's .svg file association.
        // The shell preserves browser-specific arguments, DDE and DelegateExecute.
        lpClass: browser.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    }
}

#[cfg(windows)]
fn browser_class() -> Result<Vec<u16>, Message> {
    use windows_sys::{
        Win32::UI::Shell::{ASSOCF_IS_PROTOCOL, ASSOCSTR_PROGID, AssocQueryStringW},
        core::w,
    };
    let mut size = 0;
    // SAFETY: The first call queries the size; all input strings are static.
    unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL,
            ASSOCSTR_PROGID,
            w!("https"),
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut size,
        );
    }
    if !(2..=32768).contains(&size) {
        return Err(failed());
    }
    let mut browser = vec![0_u16; size as usize];
    // SAFETY: The buffer has exactly the queried UTF-16 capacity, including NUL.
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL,
            ASSOCSTR_PROGID,
            w!("https"),
            std::ptr::null(),
            browser.as_mut_ptr(),
            &mut size,
        )
    };
    if result != 0 || browser[0] == 0 || !browser.contains(&0) {
        return Err(failed());
    }
    Ok(browser)
}

#[cfg(windows)]
fn shell_file(url: &reqwest::Url) -> Result<Vec<u16>, Message> {
    use std::os::windows::ffi::OsStrExt;
    // ShellExecuteEx applies the explicit browser class to a native document
    // path. A file: URI can bypass that class and fail despite a valid browser.
    let path = url.to_file_path().map_err(|_| failed())?;
    Ok(path.as_os_str().encode_wide().chain(Some(0)).collect())
}

#[cfg(windows)]
fn launch(url: &reqwest::Url) -> Result<(), Message> {
    use windows_sys::Win32::{
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
        },
        UI::Shell::ShellExecuteExW,
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: This guard owns one successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
    // SAFETY: Called on a new thread with no COM apartment; no reserved argument.
    let result = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if result < 0 {
        return Err(failed());
    }
    let _apartment = Apartment;
    let file = shell_file(url)?;
    let browser = browser_class()?;
    let mut request = shell_request(&file, &browser);
    // SAFETY: All UTF-16 strings remain alive and NUL-terminated throughout the
    // synchronous call. No process handle is requested and the apartment is live.
    if unsafe { ShellExecuteExW(&mut request) } == 0 {
        return Err(Message::detail(
            "route.open_failed_detail",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn launch(_url: &reqwest::Url) -> Result<(), Message> {
    Err(failed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_svg_url_preserves_unicode_spaces_and_url_delimiters() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("路线 #1 & 100%.svg");
        std::fs::write(&path, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        let url = svg_url(&path).unwrap();
        assert_eq!(url.scheme(), "file");
        assert!(url.fragment().is_none());
        assert!(url.query().is_none());
        assert_eq!(
            url.to_file_path().unwrap().canonicalize().unwrap(),
            path.canonicalize().unwrap()
        );
    }

    #[test]
    fn missing_or_non_svg_files_are_rejected_before_launch() {
        let root = tempfile::tempdir().unwrap();
        assert!(svg_url(&root.path().join("missing.svg")).is_err());
        let path = root.path().join("route.html");
        std::fs::write(&path, "html").unwrap();
        assert!(svg_url(&path).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn shell_request_uses_native_document_path_and_default_browser_class() {
        use windows_sys::Win32::UI::Shell::{SEE_MASK_CLASSNAME, SEE_MASK_NOASYNC};
        let path = Path::new(r"F:\游戏\路线 #1 & 100%.svg");
        let url = reqwest::Url::from_file_path(path).unwrap();
        let file = shell_file(&url).unwrap();
        assert_eq!(file.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&file[..file.len() - 1]).unwrap(),
            path.to_str().unwrap()
        );
        let browser: Vec<u16> = "UserDefaultBrowser.HTML\0".encode_utf16().collect();
        let request = shell_request(&file, &browser);
        assert_ne!(request.fMask & SEE_MASK_CLASSNAME, 0);
        assert_ne!(request.fMask & SEE_MASK_NOASYNC, 0);
        assert_eq!(request.lpFile, file.as_ptr());
        assert_eq!(request.lpClass, browser.as_ptr());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opens the actual default browser; set GALGEAR_ROUTE_MAP_SMOKE_FILE"]
    fn opens_saved_svg_in_default_browser() {
        let path = std::env::var_os("GALGEAR_ROUTE_MAP_SMOKE_FILE")
            .expect("Set GALGEAR_ROUTE_MAP_SMOKE_FILE to an existing SVG for this manual test");
        open_svg(Path::new(&path)).unwrap();
    }
}
