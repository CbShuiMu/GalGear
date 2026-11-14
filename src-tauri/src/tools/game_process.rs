//! Windows 游戏启动适配：请求管理员授权并持有进程句柄，供重复启动检查与会话跟踪使用。
use galgear_i18n::Message;
use std::{
    ffi::OsStr,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    process::Command,
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Threading::{GetProcessId, WaitForSingleObject},
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

pub(super) struct Child(OwnedHandle);

impl From<std::process::Child> for Child {
    fn from(child: std::process::Child) -> Self {
        Self(child.into())
    }
}

impl AsHandle for Child {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.0.as_handle()
    }
}

impl Child {
    pub(super) fn id(&self) -> u32 {
        // SAFETY: 进程句柄由 self 持有，在查询期间有效。
        unsafe { GetProcessId(self.0.as_raw_handle()) }
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<()>> {
        // 提权返回的句柄不保证有终止进程等权限；只等待退出，不请求完整进程访问权。
        // SAFETY: 句柄由 self 持有，零超时查询不会阻塞调用线程。
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(Some(())),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

struct ShellCommand {
    file: Vec<u16>,
    directory: Vec<u16>,
    parameters: Vec<u16>,
}

impl ShellCommand {
    fn new(command: &Command) -> io::Result<Self> {
        // Shell 使用 DOS/UNC 路径；沿用目录打开入口的设备前缀处理。
        let file = galgear_core::config::display_path(Path::new(command.get_program()));
        let directory =
            galgear_core::config::display_path(command.get_current_dir().unwrap_or(Path::new(".")));
        Ok(Self {
            file: wide(OsStr::new(&file))?,
            directory: wide(OsStr::new(&directory))?,
            parameters: parameters(command.get_args())?,
        })
    }

    fn request(&self) -> SHELLEXECUTEINFOW {
        SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: windows_sys::w!("runas"),
            lpFile: self.file.as_ptr(),
            lpParameters: self.parameters.as_ptr(),
            lpDirectory: self.directory.as_ptr(),
            nShow: SW_SHOWNORMAL,
            ..Default::default()
        }
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Launch value contains NUL",
        ));
    }
    value.push(0);
    Ok(value)
}

fn parameters<'a>(arguments: impl Iterator<Item = &'a OsStr>) -> io::Result<Vec<u16>> {
    let mut result = Vec::new();
    for argument in arguments {
        if !result.is_empty() {
            result.push(b' ' as u16);
        }
        result.push(b'"' as u16);
        let mut slashes = 0;
        // 与 Windows CRT 的 argv 规则一致：引号前的反斜杠加倍，再转义引号。
        // 不经过 cmd/PowerShell，参数中的 &、% 等字符没有 shell 执行语义。
        for unit in argument.encode_wide() {
            if unit == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Launch argument contains NUL",
                ));
            }
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            let count = if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            };
            result.extend(std::iter::repeat_n(b'\\' as u16, count));
            result.push(unit);
            slashes = 0;
        }
        result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        result.push(b'"' as u16);
    }
    result.push(0);
    Ok(result)
}

fn launch_error(error: io::Error) -> Message {
    if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
        Message::new("error.launch_cancelled")
    } else {
        Message::detail("error.launch", error)
    }
}

/// 以管理员权限启动已装配的游戏命令，保留程序、参数和工作目录。
/// 系统授权取消或启动失败时返回错误，不降级为普通权限重试。
pub(super) fn spawn(command: &Command) -> Result<Child, Message> {
    let command = ShellCommand::new(command).map_err(launch_error)?;
    // Shell 扩展需要 STA；使用新线程，避免改变 Tauri 后台线程已有的 COM 模式。
    std::thread::Builder::new()
        .name("game-elevation".into())
        .spawn(move || execute(&command))
        .map_err(launch_error)?
        .join()
        .map_err(|_| Message::detail("error.launch", "Game launch thread failed"))?
}

fn execute(command: &ShellCommand) -> Result<Child, Message> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: 此守卫仅在当前线程 COM 初始化成功后创建。
            unsafe { CoUninitialize() };
        }
    }
    // SAFETY: 在专用新线程上初始化 COM，不影响 UI 或其他线程。
    let initialized = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if initialized < 0 {
        return Err(Message::detail(
            "error.launch",
            format!("COM: {initialized:#x}"),
        ));
    }
    let _apartment = Apartment;
    let mut request = command.request();
    // SAFETY: 所有字符串以 NUL 结尾且在同步调用期间存活，授权由系统 UAC 处理。
    if unsafe { ShellExecuteExW(&mut request) } == 0 {
        return Err(launch_error(io::Error::last_os_error()));
    }
    if request.hProcess.is_null() {
        return Err(Message::detail(
            "error.launch",
            "Game process handle missing",
        ));
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS 将返回的进程句柄所有权交给本调用方。
    Ok(Child(unsafe {
        OwnedHandle::from_raw_handle(request.hProcess)
    }))
}

#[cfg(test)]
mod tests;
