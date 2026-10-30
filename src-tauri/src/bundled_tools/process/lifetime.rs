//! Keep shutdown at Magpie's privilege level without another UAC prompt on exit.
use super::{Child, launch_error, stop, wait_ready};
use galgear_i18n::Message;
use std::{
    ffi::OsStr,
    os::windows::{
        ffi::OsStrExt,
        io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{OpenEventW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForMultipleObjects},
};

pub(crate) fn run_guard() -> bool {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(OsStr::new("--galgear-magpie-guard")) {
        return false;
    }
    let result = (|| {
        let owner = arguments
            .next()
            .and_then(|id| id.to_str()?.parse::<u32>().ok())
            .filter(|id| *id != 0 && *id != std::process::id())
            .ok_or_else(|| Message::new("tools.magpie_start"))?;
        let event = arguments
            .next()
            .ok_or_else(|| Message::new("tools.magpie_start"))?;
        let executable = arguments
            .next()
            .ok_or_else(|| Message::new("tools.magpie_start"))?;
        if arguments.next().is_some() {
            return Err(Message::new("tools.magpie_start"));
        }
        guard(owner, &event, Path::new(&executable))
    })();
    if let Err(error) = result {
        eprintln!("Magpie lifecycle: {error:?}");
    }
    true
}

fn guard(owner: u32, event: &OsStr, executable: &Path) -> Result<(), Message> {
    // Open the parent first: a delayed UAC response must not leave an orphaned Magpie.
    let owner = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, owner) };
    if owner.is_null() {
        return Err(launch_error(std::io::Error::last_os_error()));
    }
    let owner = unsafe { OwnedHandle::from_raw_handle(owner) };
    let event: Vec<u16> = event.encode_wide().chain(Some(0)).collect();
    let event = unsafe { OpenEventW(PROCESS_SYNCHRONIZE, 0, event.as_ptr()) };
    if event.is_null() {
        return Err(launch_error(std::io::Error::last_os_error()));
    }
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    let handles = [owner.as_raw_handle(), event.as_raw_handle()];
    if unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, 0) } != WAIT_TIMEOUT {
        return stop(executable).map(|_| ());
    }
    let ids = galgear_core::runtime::process_ids(executable)
        .map_err(|e| Message::detail("tools.error", e))?;
    let mut child = if let Some(&id) = ids.first() {
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, id) };
        if handle.is_null() {
            return Err(launch_error(std::io::Error::last_os_error()));
        }
        Child(unsafe { OwnedHandle::from_raw_handle(handle) }, None)
    } else {
        // This helper already has the elevation requested at startup. Magpie inherits it.
        let process = std::process::Command::new(executable)
            .arg("-t")
            .current_dir(executable.parent().unwrap_or(Path::new(".")))
            .spawn()
            .map_err(launch_error)?;
        Child(
            process
                .as_handle()
                .try_clone_to_owned()
                .map_err(launch_error)?,
            None,
        )
    };
    wait_ready(executable, Some(&mut child), Duration::from_secs(15))?;
    watch_owner(&owner, &event, &mut child, || stop(executable).map(|_| ()))
}

fn watch_owner(
    owner: &OwnedHandle,
    event: &OwnedHandle,
    child: &mut Child,
    stop: impl FnOnce() -> Result<(), Message>,
) -> Result<(), Message> {
    let handles = [
        owner.as_raw_handle(),
        event.as_raw_handle(),
        child.0.as_raw_handle(),
    ];
    // SAFETY: All three handles remain owned throughout this blocking wait in the helper.
    match unsafe { WaitForMultipleObjects(3, handles.as_ptr(), 0, u32::MAX) } {
        WAIT_OBJECT_0 => stop(),
        result if result == WAIT_OBJECT_0 + 1 => stop(),
        result if result == WAIT_OBJECT_0 + 2 => Ok(()),
        _ => Err(launch_error(std::io::Error::last_os_error())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CreateEventW, SetEvent};

    struct Probe(std::process::Child);

    impl Probe {
        fn start() -> Self {
            Self(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "bundled_tools::process::lifetime::tests::lifetime_probe",
                        "--ignored",
                    ])
                    .env("GALGEAR_MAGPIE_LIFETIME_PROBE", "1")
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            )
        }

        fn handle(&self) -> OwnedHandle {
            self.0.as_handle().try_clone_to_owned().unwrap()
        }

        fn exit(&mut self) {
            drop(self.0.stdin.take());
            assert!(self.0.wait().unwrap().success());
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            drop(self.0.stdin.take());
            let _ = self.0.wait();
        }
    }

    fn event() -> OwnedHandle {
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!raw.is_null());
        unsafe { OwnedHandle::from_raw_handle(raw) }
    }

    #[test]
    fn parent_exit_closes_the_managed_process() {
        let mut owner = Probe::start();
        let mut magpie = Probe::start();
        let mut child = Child(magpie.handle(), None);
        owner.exit();
        watch_owner(&owner.handle(), &event(), &mut child, || {
            magpie.exit();
            Ok(())
        })
        .unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn shutdown_request_closes_magpie_while_parent_is_still_alive() {
        let mut owner = Probe::start();
        let mut magpie = Probe::start();
        let mut child = Child(magpie.handle(), None);
        let event = event();
        assert_ne!(unsafe { SetEvent(event.as_raw_handle()) }, 0);
        watch_owner(&owner.handle(), &event, &mut child, || {
            magpie.exit();
            Ok(())
        })
        .unwrap();
        assert!(child.try_wait().unwrap().is_some());
        assert!(owner.0.try_wait().unwrap().is_none());
        owner.exit();
    }

    #[test]
    fn manually_closed_magpie_leaves_parent_running() {
        let mut owner = Probe::start();
        let mut magpie = Probe::start();
        let mut child = Child(magpie.handle(), None);
        magpie.exit();
        watch_owner(&owner.handle(), &event(), &mut child, || {
            panic!("Magpie already exited")
        })
        .unwrap();
        assert!(owner.0.try_wait().unwrap().is_none());
        owner.exit();
    }

    #[test]
    #[ignore = "Subprocess fixture for the Magpie lifetime tests"]
    fn lifetime_probe() {
        if std::env::var_os("GALGEAR_MAGPIE_LIFETIME_PROBE").is_some() {
            let _ = std::io::stdin().read(&mut [0]);
        }
    }
}
