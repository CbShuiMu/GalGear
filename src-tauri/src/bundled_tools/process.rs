//! Magpie v0.13 accepts WM_MAGPIE_QUIT but has no configuration reload message.
use galgear_i18n::Message;
use std::{
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Threading::{
            CreateEventW, OpenProcess, PROCESS_SYNCHRONIZE, SetEvent, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowThreadProcessId, PostMessageW,
            RegisterWindowMessageW, SW_SHOWNORMAL,
        },
    },
};
use windows_sys::core::w;

mod lifetime;
pub(crate) use lifetime::run_guard;

/// Own the process handle returned by the elevation broker, independently of the shell thread.
pub(crate) struct Child(OwnedHandle, Option<OwnedHandle>);

impl Drop for Child {
    fn drop(&mut self) {
        if let Some(event) = &self.1 {
            // SAFETY: The event remains owned for this call; the elevated guard does the cleanup.
            unsafe { SetEvent(event.as_raw_handle()) };
        }
    }
}

impl Child {
    pub(crate) fn stop(&mut self) -> Result<(), Message> {
        if self.try_wait().map_err(launch_error)?.is_some() {
            return Ok(());
        }
        let event = self
            .1
            .as_ref()
            .ok_or_else(|| Message::new("tools.magpie_restart_failed"))?;
        // SAFETY: Both the event and process handles are owned throughout the bounded wait.
        if unsafe { SetEvent(event.as_raw_handle()) } == 0
            || unsafe { WaitForSingleObject(self.0.as_raw_handle(), 6000) } != WAIT_OBJECT_0
        {
            return Err(Message::new("tools.magpie_restart_failed"));
        }
        Ok(())
    }
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<()>> {
        // SAFETY: The handle remains owned for this synchronous wait.
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(Some(())),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    #[cfg(test)]
    pub(crate) fn id(&self) -> u32 {
        // SAFETY: The process handle remains alive for the call.
        unsafe { windows_sys::Win32::System::Threading::GetProcessId(self.0.as_raw_handle()) }
    }
}

fn shell_request(file: &[u16], directory: &[u16], parameters: &[u16]) -> SHELLEXECUTEINFOW {
    SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    }
}

fn launch_error(failure: io::Error) -> Message {
    if failure.raw_os_error() == Some(ERROR_CANCELLED as i32) {
        Message::new("tools.magpie_cancelled")
    } else {
        Message::detail("tools.error", failure)
    }
}

/// Both initial startup and hotkey-triggered restarts request administrator rights.
pub(crate) fn start(executable: &Path) -> Result<Child, Message> {
    let executable = executable.to_owned();
    // The shell may invoke COM extensions. Give it an STA without changing Tauri's apartment.
    std::thread::Builder::new()
        .name("magpie-elevation".into())
        .spawn(move || start_elevated(&executable))
        .map_err(|e| Message::detail("tools.error", e))?
        .join()
        .map_err(|_| Message::new("tools.magpie_start"))?
}

fn start_elevated(executable: &Path) -> Result<Child, Message> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: This guard owns a successful initialization on this thread.
            unsafe { CoUninitialize() };
        }
    }
    // SAFETY: A fresh thread has no existing COM apartment.
    let result = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if result < 0 {
        return Err(Message::new("tools.magpie_start"));
    }
    let _apartment = Apartment;
    let helper = std::env::current_exe().map_err(launch_error)?;
    #[cfg(test)]
    let helper = std::env::var_os("GALGEAR_BUNDLED_SMOKE_ROOT")
        .map(|root| std::path::PathBuf::from(root).join("GalGear.exe"))
        .unwrap_or(helper);
    let name = format!(
        "Local\\GalGear.Magpie.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| Message::detail("tools.error", e))?
            .as_nanos()
    );
    let name_wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: The name is NUL-terminated; the new manual-reset event belongs to this launch.
    let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, name_wide.as_ptr()) };
    if event.is_null() {
        return Err(launch_error(io::Error::last_os_error()));
    }
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    let parameters = format!(
        "--galgear-magpie-guard {} {name} \"{}\"",
        std::process::id(),
        executable.display()
    );
    let parameters: Vec<u16> = parameters.encode_utf16().chain(Some(0)).collect();
    let file: Vec<u16> = helper.as_os_str().encode_wide().chain(Some(0)).collect();
    let directory: Vec<u16> = executable
        .parent()
        .unwrap_or(Path::new("."))
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut request = shell_request(&file, &directory, &parameters);
    // SAFETY: All strings are NUL-terminated and live throughout the synchronous shell call.
    if unsafe { ShellExecuteExW(&mut request) } == 0 {
        return Err(launch_error(io::Error::last_os_error()));
    }
    if request.hProcess.is_null() {
        return Err(Message::new("tools.magpie_start"));
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS transfers this process handle to the caller.
    let mut child = Child(
        unsafe { OwnedHandle::from_raw_handle(request.hProcess) },
        Some(event),
    );
    wait_ready(executable, Some(&mut child), Duration::from_secs(15))?;
    Ok(child)
}

/// A previous launch may still be initializing when the user retries.
pub(crate) fn wait_existing(executable: &Path) -> Result<(), Message> {
    wait_ready(executable, None, Duration::from_secs(15))
}

fn wait_ready(
    executable: &Path,
    mut child: Option<&mut Child>,
    timeout: Duration,
) -> Result<(), Message> {
    let started = Instant::now();
    loop {
        if let Some(child) = child.as_mut()
            && child
                .try_wait()
                .map_err(|e| Message::detail("tools.error", e))?
                .is_some()
        {
            return Err(Message::new("tools.magpie_exited"));
        }
        // The elevation broker's handle can support synchronization but not WaitForInputIdle.
        // Resolve the exact executable via limited-query handles, then inspect its own windows.
        let ids = galgear_core::runtime::process_ids(executable)
            .map_err(|e| Message::detail("tools.error", e))?;
        if has_ready_window(&ids) {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(Message::new("tools.magpie_timeout"));
        }
        std::thread::sleep(
            Duration::from_millis(100).min(timeout.saturating_sub(started.elapsed())),
        );
    }
}

struct ReadyWindows<'a> {
    ids: &'a [u32],
    found: bool,
}

unsafe extern "system" fn ready_window(window: HWND, context: LPARAM) -> i32 {
    // SAFETY: EnumWindows invokes this synchronously while the context and PID slice are alive.
    let ready = unsafe { &mut *(context as *mut ReadyWindows<'_>) };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if ready.ids.contains(&pid) {
        let mut class = [0_u16; 64];
        // SAFETY: The buffer is writable for the declared length; disappearing windows return 0.
        let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
        let class = &class[..length.max(0) as usize];
        // Magpie 0.13 creates these after initializing its scaling and shortcut services.
        // Its early XAML/IME windows do not indicate readiness, and tray mode has no main window.
        ready.found = class.iter().copied().eq("Magpie_NotifyIcon".encode_utf16())
            || class.iter().copied().eq("Magpie_Main".encode_utf16());
    }
    i32::from(!ready.found)
}

fn has_ready_window(ids: &[u32]) -> bool {
    let mut ready = ReadyWindows { ids, found: false };
    // SAFETY: The callback cannot outlive ready or its borrowed PID slice.
    unsafe {
        EnumWindows(
            Some(ready_window),
            &mut ready as *mut ReadyWindows<'_> as LPARAM,
        )
    };
    ready.found
}

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        // This handle is owned by the OpenProcess call below.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Quit {
    pid: u32,
    message: u32,
    sent: bool,
}

unsafe extern "system" fn quit_window(window: HWND, context: LPARAM) -> i32 {
    // EnumWindows invokes this synchronously while the Quit value is alive.
    let quit = unsafe { &mut *(context as *mut Quit) };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    if pid == quit.pid {
        quit.sent |= unsafe { PostMessageW(window, quit.message, 0, 0) } != 0;
    }
    1
}

/// Gracefully stop only instances of the configured executable; never broadcast or force kill.
pub(crate) fn stop(executable: &Path) -> Result<bool, Message> {
    let ids = galgear_core::runtime::process_ids(executable)
        .map_err(|e| Message::detail("tools.error", e))?;
    let running = !ids.is_empty();
    let name: Vec<u16> = "WM_MAGPIE_QUIT\0".encode_utf16().collect();
    let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    if message == 0 {
        return Err(Message::new("tools.magpie_restart_failed"));
    }
    for pid in ids {
        let process = Process(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) });
        if process.0.is_null() {
            return Err(Message::new("tools.magpie_restart_failed"));
        }
        let mut quit = Quit {
            pid,
            message,
            sent: false,
        };
        unsafe {
            EnumWindows(Some(quit_window), &mut quit as *mut Quit as LPARAM);
        }
        if !quit.sent || unsafe { WaitForSingleObject(process.0, 5000) } != WAIT_OBJECT_0 {
            return Err(Message::new("tools.magpie_restart_failed"));
        }
    }
    Ok(running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magpie_readiness_accepts_a_synchronize_only_handle_after_the_tray_window_appears() {
        use windows_sys::Win32::{
            Foundation::WAIT_FAILED,
            System::Threading::WaitForInputIdle,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, UnregisterClassW,
                WNDCLASSW,
            },
        };
        let executable = std::env::current_exe().unwrap();
        let pid = std::process::id();
        // Reproduce the restricted handle returned across the UAC boundary without elevating tests.
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!raw.is_null());
        let mut child = Child(unsafe { OwnedHandle::from_raw_handle(raw) }, None);
        assert_eq!(unsafe { WaitForInputIdle(raw, 0) }, WAIT_FAILED);
        assert!(child.try_wait().unwrap().is_none());
        assert_eq!(
            wait_ready(&executable, Some(&mut child), Duration::ZERO)
                .unwrap_err()
                .key,
            "tools.magpie_timeout"
        );

        let (release, released) = std::sync::mpsc::channel();
        let fixture = std::thread::spawn(move || {
            // Allow the readiness loop to observe a running process before it creates a window.
            std::thread::sleep(Duration::from_millis(200));
            let class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                lpszClassName: w!("Magpie_NotifyIcon"),
                ..Default::default()
            };
            unsafe {
                assert_ne!(RegisterClassW(&class), 0);
                let window = CreateWindowExW(
                    0,
                    class.lpszClassName,
                    w!(""),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!window.is_null());
                let _ = released.recv_timeout(Duration::from_secs(5));
                DestroyWindow(window);
                UnregisterClassW(class.lpszClassName, std::ptr::null_mut());
            }
        });
        let result = wait_ready(&executable, Some(&mut child), Duration::from_secs(3));
        let own_window = has_ready_window(&[pid]);
        let foreign_window = has_ready_window(&[0]);
        let _ = release.send(());
        fixture.join().unwrap();
        assert!(result.is_ok(), "{result:?}");
        assert!(own_window);
        assert!(!foreign_window);
    }

    #[test]
    fn magpie_readiness_reports_a_child_that_exited_before_initialization() {
        use std::{os::windows::io::AsHandle, os::windows::process::CommandExt, process::Stdio};
        let executable = std::env::current_exe().unwrap();
        let mut process = std::process::Command::new(&executable)
            .arg("--list")
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = Child(process.as_handle().try_clone_to_owned().unwrap(), None);
        process.wait().unwrap();
        assert_eq!(
            wait_ready(&executable, Some(&mut child), Duration::from_secs(1))
                .unwrap_err()
                .key,
            "tools.magpie_exited"
        );
    }

    #[test]
    #[ignore = "Read-only check of a running Magpie; set GALGEAR_BUNDLED_SMOKE_ROOT"]
    fn running_elevated_magpie_is_ready_without_full_process_access() {
        let root = std::path::PathBuf::from(std::env::var("GALGEAR_BUNDLED_SMOKE_ROOT").unwrap());
        wait_existing(&root.join(crate::bundled_tools::MAGPIE)).unwrap();
    }

    #[test]
    fn magpie_launch_requests_admin_and_preserves_tray_mode_and_working_directory() {
        let file: Vec<u16> = r"C:\Games & Tools\超分\Magpie.exe"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let directory: Vec<u16> = r"C:\Games & Tools\超分"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let parameters: Vec<u16> =
            "--galgear-magpie-guard 123 Local\\fixture \"C:\\Tools\\Magpie.exe\""
                .encode_utf16()
                .chain(Some(0))
                .collect();
        let request = shell_request(&file, &directory, &parameters);
        // SAFETY: These static strings in shell_request include their terminating NUL.
        let verb = unsafe { std::slice::from_raw_parts(request.lpVerb, 6) };
        let arguments =
            unsafe { std::slice::from_raw_parts(request.lpParameters, parameters.len()) };
        assert_eq!(verb, "runas\0".encode_utf16().collect::<Vec<_>>());
        assert_eq!(arguments, parameters);
        assert_eq!(request.lpFile, file.as_ptr());
        assert_eq!(request.lpDirectory, directory.as_ptr());
        assert_ne!(request.fMask & SEE_MASK_NOCLOSEPROCESS, 0);
        assert_ne!(request.fMask & SEE_MASK_NOASYNC, 0);
    }

    #[test]
    fn cancelled_elevation_has_a_specific_notice() {
        assert_eq!(
            launch_error(io::Error::from_raw_os_error(ERROR_CANCELLED as i32)).key,
            "tools.magpie_cancelled"
        );
        assert_eq!(
            launch_error(io::Error::from_raw_os_error(5)).key,
            "tools.error"
        );
    }
}
