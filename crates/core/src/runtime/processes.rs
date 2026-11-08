//! 一次采集进程身份、父进程、映像路径和窗口证据；不在此层决定游戏归属。
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, FILETIME, GetLastError, HANDLE,
        HWND, INVALID_HANDLE_VALUE, LPARAM,
    },
    System::{Diagnostics::ToolHelp::*, Threading::*},
    UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId,
        IsWindowVisible,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Identity {
    pub pid: u32,
    pub created: u64,
}

pub(super) struct Process {
    pub identity: Option<Identity>,
    pub parent: u32,
    pub name: String,
    pub image: Option<PathBuf>,
    pub visible: bool,
    pub foreground: bool,
}

pub(super) struct Snapshot {
    pub captured: Instant,
    pub processes: BTreeMap<u32, Process>,
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle 只拥有本模块成功打开的句柄，且仅关闭一次。
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn process_times(handle: HANDLE) -> Result<(u64, bool), String> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: 调用者持有进程句柄；四个输出指针均指向有效的 FILETIME。
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        exited.dwHighDateTime != 0 || exited.dwLowDateTime != 0,
    ))
}

pub(super) fn child_identity(child: &std::process::Child) -> Result<Identity, String> {
    use std::os::windows::io::AsRawHandle;
    // Child 句柄即使在启动器瞬间退出后仍能查询创建时间，避免下一轮轮询丢失根进程。
    Ok(Identity {
        pid: child.id(),
        created: process_times(child.as_raw_handle())?.0,
    })
}

unsafe extern "system" fn collect_window(window: HWND, data: LPARAM) -> i32 {
    // SAFETY: EnumWindows 同步调用；data 在枚举期间指向独占、有效的进程表。
    let processes = unsafe { &mut *(data as *mut BTreeMap<u32, Process>) };
    // SAFETY: window 是 EnumWindows 提供的窗口句柄，API 不保存输出指针。
    unsafe {
        if IsWindowVisible(window) != 0 && GetWindow(window, GW_OWNER).is_null() {
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if let Some(process) = processes.get_mut(&pid) {
                process.visible = true;
            }
        }
    }
    1
}

impl Snapshot {
    pub fn capture() -> Result<Self, String> {
        Self::collect(None)
    }

    pub fn matching_names(names: &BTreeSet<String>) -> Result<Self, String> {
        Self::collect(Some(names))
    }

    fn collect(names: Option<&BTreeSet<String>>) -> Result<Self, String> {
        let captured = Instant::now();
        // SAFETY: ToolHelp 返回独立快照；句柄由 RAII 释放，结构大小按 API 要求填写。
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let snapshot = Handle(raw);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: 快照有效，entry 是正确初始化的可写结构。
        if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut processes = BTreeMap::new();
        loop {
            let length = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            // 工具的精确路径检查仍只打开同名进程；游戏会话才需要完整父子关系。
            if names.is_none_or(|names| {
                super::executable_stem(Path::new(&name)).is_some_and(|stem| names.contains(&stem))
            }) && let Some(process) = inspect(&entry, name)
            {
                processes.insert(entry.th32ProcessID, process);
            }
            // SAFETY: 快照和输出结构在整个枚举期间有效。
            if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
                // SAFETY: 紧接失败调用读取线程本地错误。
                if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                break;
            }
        }
        // SAFETY: 回调仅在调用期间借用进程表；不会保存 data。
        if names.is_none()
            && unsafe { EnumWindows(Some(collect_window), &mut processes as *mut _ as LPARAM) } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if names.is_none() {
            let mut foreground_pid = 0;
            // SAFETY: 仅查询当前前台窗口和其 PID，输出指针在调用期间有效。
            // 独立采样一次也能覆盖有 owner 的游戏对话框。
            unsafe {
                GetWindowThreadProcessId(GetForegroundWindow(), &mut foreground_pid);
            }
            if let Some(process) = processes.get_mut(&foreground_pid) {
                process.foreground = true;
            }
        }
        Ok(Self {
            captured,
            processes,
        })
    }
}

fn inspect(entry: &PROCESSENTRY32W, name: String) -> Option<Process> {
    let mut process = Process {
        identity: None,
        parent: entry.th32ParentProcessID,
        name,
        image: None,
        visible: false,
        foreground: false,
    };
    // SAFETY: PID 来自系统快照，仅请求查询权限。
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
    if raw.is_null() {
        // SAFETY: 紧接失败调用读取错误；已退出进程不视作权限未知。
        return (unsafe { GetLastError() } != ERROR_INVALID_PARAMETER).then_some(process);
    }
    let handle = Handle(raw);
    match process_times(handle.0) {
        // 持有 Child 句柄时，已退出对象仍可能出现在快照中。
        Ok((_, true)) => return None,
        Ok((created, false)) => {
            process.identity = Some(Identity {
                pid: entry.th32ProcessID,
                created,
            })
        }
        Err(_) => {}
    }
    let mut buffer = vec![0_u16; 32768];
    let mut length = buffer.len() as u32;
    // SAFETY: 进程句柄有效；缓冲区与其长度相符，返回长度仅在成功时使用。
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut length) } != 0 {
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
        // 临时 PE 可能已被启动器删去；Windows 返回的绝对路径仍是有效的归属证据。
        process.image = Some(std::fs::canonicalize(&path).unwrap_or(path));
    }
    Some(process)
}
