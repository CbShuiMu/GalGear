//! 稳定 Rust 上的 CreateProcessW 封装，显式传入 LPAC 属性和明确的标准流/网络通道句柄。
use super::{Sandbox, executable};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::Path,
    process::{Command, ExitStatus},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::SECURITY_ATTRIBUTES,
    System::{
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
            GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

pub(in super::super) struct Child {
    process: OwnedHandle,
    thread: OwnedHandle,
    id: u32,
    pub(in super::super) stdin: Option<File>,
}

impl Drop for Child {
    fn drop(&mut self) {
        // 启动后登记失败和测试断言失败也必须回收尚未恢复的主线程。
        let _ = self.kill();
        let _ = self.wait();
    }
}

impl Child {
    pub(in super::super) fn spawn(
        command: &Command,
        work: &Path,
        sandbox: Option<&Sandbox>,
        output: Option<(&File, &File)>,
    ) -> io::Result<Self> {
        let program = executable(command.get_program())?;
        let application = wide(program.as_os_str())?;
        let mut arguments = quote(program.as_os_str())?;
        for argument in command.get_args() {
            arguments.push(32);
            arguments.extend(quote(argument)?);
        }
        arguments.push(0);
        let directory = wide(work.as_os_str())?;
        let (input, writer) = pipe()?;
        let null = File::options().read(true).write(true).open("NUL")?;
        let (stdout, stderr) = output.unwrap_or((&null, &null));
        let stdout = inheritable(stdout.as_raw_handle())?;
        let stderr = inheritable(stderr.as_raw_handle())?;
        let mut handles = vec![
            input.as_raw_handle(),
            stdout.as_raw_handle(),
            stderr.as_raw_handle(),
        ];
        let network_client = sandbox
            .and_then(|sandbox| sandbox.network.as_ref())
            .map(|broker| inheritable(broker.client_handle()))
            .transpose()?;
        if let Some(client) = &network_client {
            handles.push(client.as_raw_handle());
        }
        let environment = environment(
            command,
            network_client.as_ref().map(AsRawHandle::as_raw_handle),
        )?;
        let mut attributes = Attributes::new(if sandbox.is_some() { 3 } else { 1 })?;
        attributes.set_slice(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &handles)?;
        let mut capabilities = Vec::new();
        let security = sandbox.map(|sandbox| sandbox.security(&mut capabilities));
        let policy: u32 = 1; // PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT
        if let Some(security) = &security {
            attributes.set(
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                security,
            )?;
            attributes.set(
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY as usize,
                &policy,
            )?;
        }
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = attributes.pointer();
        let mut process = PROCESS_INFORMATION::default();
        // SAFETY: all strings, attributes, SID buffers and inherited handles remain valid until
        // CreateProcessW returns. The handle allowlist prevents inheriting unrelated host handles.
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                arguments.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful CreateProcessW transfers these two unique handles to the caller.
        Ok(unsafe {
            Self {
                process: OwnedHandle::from_raw_handle(process.hProcess),
                thread: OwnedHandle::from_raw_handle(process.hThread),
                id: process.dwProcessId,
                stdin: Some(File::from(writer)),
            }
        })
    }

    pub(in super::super) fn resume(&self) -> io::Result<()> {
        // SAFETY: the primary thread is live, owned and was created suspended.
        if unsafe { ResumeThread(self.thread.as_raw_handle()) } == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(in super::super) fn id(&self) -> u32 {
        self.id
    }
    pub(in super::super) fn as_handle(&self) -> BorrowedHandle<'_> {
        self.process.as_handle()
    }
    pub(in super::super) fn as_raw_handle(&self) -> HANDLE {
        self.process.as_raw_handle()
    }

    pub(in super::super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: the process handle is kept open for both calls.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => self.status().map(Some),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn status(&self) -> io::Result<ExitStatus> {
        let mut code = 0;
        // SAFETY: process and the output code are valid for this synchronous call.
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ExitStatus::from_raw(code))
    }

    pub(in super::super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.stdin.take();
        // SAFETY: process remains owned throughout the wait.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        self.status()
    }

    pub(in super::super) fn kill(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        // SAFETY: this is the process created by this Child.
        if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

pub(super) fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in Windows process input",
        ));
    }
    value.push(0);
    Ok(value)
}

fn quote(value: &OsStr) -> io::Result<Vec<u16>> {
    let value = wide(value)?;
    let mut result = vec![34];
    let mut slashes = 0;
    for &unit in &value[..value.len() - 1] {
        if unit == 92 {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            92,
            if unit == 34 { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        result.push(unit);
    }
    result.extend(std::iter::repeat_n(92, slashes * 2));
    result.push(34);
    Ok(result)
}

fn environment(command: &Command, network: Option<HANDLE>) -> io::Result<Vec<u16>> {
    // 凭据、开发机 Python 配置及其他环境变量不进入分析进程。
    let mut variables = BTreeMap::<OsString, OsString>::new();
    for name in [
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATH",
        "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS",
    ] {
        if let Some(value) = std::env::var_os(name) {
            variables.insert(name.into(), value);
        }
    }
    for (name, value) in command.get_envs() {
        let key = OsString::from(name.to_string_lossy().to_uppercase());
        if let Some(value) = value {
            variables.insert(key, value.into());
        } else {
            variables.remove(&key);
        }
    }
    if let Some(handle) = network {
        variables.insert(
            "GALGEAR_AI_HTTP_HANDLE".into(),
            (handle as usize).to_string().into(),
        );
    }
    let mut result = Vec::new();
    for (name, value) in variables {
        let mut entry = name;
        entry.push("=");
        entry.push(value);
        result.extend(wide(&entry)?);
    }
    result.push(0);
    Ok(result)
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: 1,
        lpSecurityDescriptor: ptr::null_mut(),
    };
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: initialized outputs and security attributes are valid for CreatePipe.
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreatePipe returns two uniquely owned handles.
    let (read, write) = unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    };
    // SAFETY: the parent writer is live and must not be inherited by the child.
    if unsafe { SetHandleInformation(write.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((read, write))
}

fn inheritable(handle: HANDLE) -> io::Result<OwnedHandle> {
    let mut result = ptr::null_mut();
    // SAFETY: source is borrowed from a live File; DuplicateHandle creates independent ownership.
    unsafe {
        let current = GetCurrentProcess();
        if DuplicateHandle(
            current,
            handle,
            current,
            &mut result,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnedHandle::from_raw_handle(result))
    }
}

struct Attributes(Vec<usize>);
impl Attributes {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: the first call queries the buffer size; the second receives aligned storage.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size);
            let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
            if InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), count, 0, &mut size)
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(storage))
        }
    }
    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }
    fn set<T>(&mut self, key: usize, value: &T) -> io::Result<()> {
        self.set_slice(key, std::slice::from_ref(value))
    }
    fn set_slice<T>(&mut self, key: usize, value: &[T]) -> io::Result<()> {
        // SAFETY: the list is initialized; callers retain every attribute value through process creation.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                key,
                value.as_ptr().cast(),
                size_of_val(value),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: this list was initialized and has not been deleted.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer());
        }
    }
}
