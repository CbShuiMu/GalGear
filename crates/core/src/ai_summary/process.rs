//! Windows owns each AI process tree, even if the desktop is forcibly terminated.
use super::agent::close_aborted_session;
use std::{
    collections::BTreeMap,
    io,
    ops::{Deref, DerefMut},
    os::windows::{
        io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
            WaitForSingleObject,
        },
    },
};

static WORKERS: OnceLock<Workers> = OnceLock::new();

#[derive(Clone, Default)]
struct Workers(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    shutting_down: bool,
    entries: BTreeMap<u32, Entry>,
}

struct Entry {
    job: OwnedHandle,
    process: OwnedHandle,
    session: Option<PathBuf>,
}

impl Entry {
    fn stop(self, status: &str) {
        // Only GalGear owns this non-inheritable handle. Closing it kills all
        // descendants, including the bundled host's bootloader and tool workers.
        drop(self.job);
        // SAFETY: process is an owned process handle and remains open during the wait.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), 5000) } == WAIT_OBJECT_0
            && let Some(work) = self.session
        {
            close_aborted_session(&work, false, status);
        }
    }
}

pub(super) struct Worker {
    child: Child,
    workers: Workers,
}

impl Workers {
    fn state(&self) -> MutexGuard<'_, State> {
        // Shutdown must still reclaim children if a worker panicked under the lock.
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn spawn(&self, command: &mut Command, work: &Path, parse: bool) -> io::Result<Worker> {
        let mut state = self.state();
        if state.shutting_down {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "AI workers are shutting down",
            ));
        }
        let job = create_job()?;
        // Assign before any code runs, so even a one-file bootloader cannot
        // start a child outside the job. The lock serializes spawning and exit.
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let registered = (|| {
            // SAFETY: both handles are live and the child was created suspended.
            if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let process = child.as_handle().try_clone_to_owned()?;
            resume(&child)?;
            Ok(Entry {
                job,
                process,
                session: (!parse).then(|| work.to_path_buf()),
            })
        })();
        match registered {
            Ok(entry) => {
                state.entries.insert(child.id(), entry);
                Ok(Worker {
                    child,
                    workers: self.clone(),
                })
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    fn shutdown(&self) {
        let mut state = self.state();
        state.shutting_down = true;
        for (_, entry) in std::mem::take(&mut state.entries) {
            entry.stop("cancelled");
        }
    }
}

impl Worker {
    pub(super) fn kill(&mut self) -> io::Result<()> {
        let state = self.workers.state();
        if let Some(entry) = state.entries.get(&self.child.id()) {
            // SAFETY: the registry keeps the job handle open for this call.
            if unsafe { TerminateJobObject(entry.job.as_raw_handle(), 1) } != 0 {
                return Ok(());
            }
        }
        self.child.kill()
    }
}

impl Deref for Worker {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.child
    }
}

impl DerefMut for Worker {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(entry) = self.workers.state().entries.remove(&self.child.id()) {
            entry.stop("failed");
        }
        let _ = self.child.wait();
    }
}

pub(super) fn spawn(command: &mut Command, work: &Path, parse: bool) -> io::Result<Worker> {
    WORKERS
        .get_or_init(Workers::default)
        .spawn(command, work, parse)
}

pub(super) fn shutdown() {
    WORKERS.get_or_init(Workers::default).shutdown();
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: callers pass a newly created, uniquely owned Windows handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null security attributes create a non-inheritable, unnamed job.
    let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: limits is the correct structure and size for this information class.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

fn resume(child: &Child) -> io::Result<()> {
    // std::process::Child does not expose the primary thread handle. A suspended
    // process has only its primary thread, which cannot exit before we resume it.
    // SAFETY: snapshot flags request threads and do not use the process-id argument.
    let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
    let mut thread = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    // SAFETY: thread points to an initialized structure with the required size.
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut thread) };
    while found != 0 {
        if thread.th32OwnerProcessID == child.id() {
            // SAFETY: this thread belongs to our still-suspended child.
            let handle =
                owned(unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread.th32ThreadID) })?;
            // SAFETY: handle has suspend/resume rights and is held open for the call.
            if unsafe { ResumeThread(handle.as_raw_handle()) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        // SAFETY: snapshot and thread remain valid for enumeration.
        found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut thread) };
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "AI worker thread not found",
    ))
}

#[cfg(test)]
mod tests {
    use super::{Worker, Workers, owned};
    use serde_json::Value;
    use std::{
        fs,
        os::windows::{io::AsRawHandle, process::CommandExt},
        path::Path,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            CREATE_NO_WINDOW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        },
    };

    fn fixture(mode: &str, ready: &Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "ai_summary::process::tests::worker_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("GALGEAR_WORKER_TEST_MODE", mode)
            .env("GALGEAR_WORKER_TEST_READY", ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        command
    }

    // Each fixture is short-lived even if an assertion fails before cleanup.
    #[test]
    #[ignore = "subprocess fixture for process-lifecycle tests"]
    fn worker_fixture() {
        let Ok(mode) = std::env::var("GALGEAR_WORKER_TEST_MODE") else {
            return;
        };
        let ready = std::env::var_os("GALGEAR_WORKER_TEST_READY").unwrap();
        let ready = Path::new(&ready);
        match mode.as_str() {
            "owner" => {
                let _worker = Workers::default()
                    .spawn(&mut fixture("tree", ready), ready.parent().unwrap(), false)
                    .unwrap();
                thread::sleep(Duration::from_secs(30));
            }
            "tree" => {
                let mut descendant = fixture("leaf", ready).spawn().unwrap();
                fs::write(
                    ready,
                    serde_json::to_vec(&[std::process::id(), descendant.id()]).unwrap(),
                )
                .unwrap();
                thread::sleep(Duration::from_secs(30));
                let _ = descendant.kill();
                let _ = descendant.wait();
            }
            "leaf" => thread::sleep(Duration::from_secs(30)),
            _ => panic!("Unknown worker fixture"),
        }
    }

    fn tree(ready: &Path) -> Vec<std::os::windows::io::OwnedHandle> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let pids: Vec<u32> = loop {
            if let Ok(bytes) = fs::read(ready)
                && let Ok(pids) = serde_json::from_slice(&bytes)
            {
                break pids;
            }
            assert!(
                Instant::now() < deadline,
                "AI process fixture did not become ready"
            );
            thread::sleep(Duration::from_millis(20));
        };
        pids.into_iter()
            .map(|pid| {
                // SAFETY: only synchronization rights are requested for the test's processes.
                let handle = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) }).unwrap();
                // SAFETY: handle is a live process handle held throughout the call.
                assert_eq!(
                    unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
                    WAIT_TIMEOUT
                );
                handle
            })
            .collect()
    }

    fn assert_stopped(handles: &[std::os::windows::io::OwnedHandle]) {
        for handle in handles {
            // SAFETY: the owned process handle stays open while waiting.
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
                WAIT_OBJECT_0
            );
        }
    }

    fn start(workers: &Workers, work: &Path) -> Worker {
        super::super::write_json(
            &work.join("session.json"),
            &serde_json::json!({"status":"running", "closed":false}),
        )
        .unwrap();
        workers
            .spawn(&mut fixture("tree", &work.join("ready.json")), work, false)
            .unwrap()
    }

    struct Cleanup(Child);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn shutdown_stops_workers_and_descendants_and_rejects_new_work() {
        let work = tempfile::tempdir().unwrap();
        let workers = Workers::default();
        let mut worker = start(&workers, work.path());
        let handles = tree(&work.path().join("ready.json"));
        let unrelated = Cleanup(
            fixture("leaf", &work.path().join("unused"))
                .spawn()
                .unwrap(),
        );
        workers.shutdown();
        workers.shutdown();
        assert_stopped(&handles);
        assert!(worker.try_wait().unwrap().is_some());
        assert!(
            workers
                .spawn(
                    &mut fixture("leaf", &work.path().join("late")),
                    work.path(),
                    true
                )
                .is_err()
        );
        let session: Value =
            serde_json::from_slice(&fs::read(work.path().join("session.json")).unwrap()).unwrap();
        assert_eq!(session["status"], "cancelled");
        assert_eq!(session["closed"], true);
        // SAFETY: unrelated owns this process handle; AI shutdown must leave it running.
        assert_eq!(
            unsafe { WaitForSingleObject(unrelated.0.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
    }

    #[test]
    fn dropping_a_worker_reclaims_its_entire_tree() {
        let work = tempfile::tempdir().unwrap();
        let worker = start(&Workers::default(), work.path());
        let handles = tree(&work.path().join("ready.json"));
        drop(worker);
        assert_stopped(&handles);
        let session: Value =
            serde_json::from_slice(&fs::read(work.path().join("session.json")).unwrap()).unwrap();
        assert_eq!(session["closed"], true);
    }

    #[test]
    fn terminating_the_owner_also_kills_ai_workers_and_tools() {
        let work = tempfile::tempdir().unwrap();
        let ready = work.path().join("ready.json");
        let mut owner = Cleanup(fixture("owner", &ready).spawn().unwrap());
        let handles = tree(&ready);
        owner.0.kill().unwrap();
        owner.0.wait().unwrap();
        assert_stopped(&handles);
    }
}
