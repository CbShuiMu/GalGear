//! 原生未处理异常只使用预打开句柄和栈上缓冲，避免损坏的堆或日志锁阻塞最后一条记录。
use std::{
    fmt::Write,
    fs::{File, OpenOptions},
    io,
    os::windows::io::AsRawHandle,
    path::Path,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::SYSTEMTIME,
    Storage::FileSystem::{FlushFileBuffers, WriteFile},
    System::{
        Diagnostics::Debug::{
            EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
        },
        LibraryLoader::GetModuleHandleW,
        SystemInformation::GetLocalTime,
        Threading::GetCurrentThreadId,
    },
};

static CRASH_FILE: OnceLock<File> = OnceLock::new();

pub(super) fn install(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().append(true).open(path)?;
    CRASH_FILE
        .set(file)
        .map_err(|_| io::Error::other("Crash logger already initialized"))?;
    // SAFETY: 回调为静态函数，文件句柄由 OnceLock 持有直到进程结束。
    unsafe {
        SetUnhandledExceptionFilter(Some(unhandled));
    }
    Ok(())
}

struct Buffer {
    bytes: [u8; 1024],
    len: usize,
}
impl std::fmt::Write for Buffer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len + text.len();
        let target = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;
        target.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

unsafe extern "system" fn unhandled(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: Windows 在回调期间提供有效异常结构；空指针时跳过，不尝试恢复故障线程。
    unsafe {
        if let Some(file) = CRASH_FILE.get()
            && let Some(info) = info.as_ref()
            && let Some(record) = info.ExceptionRecord.as_ref()
        {
            let mut time: SYSTEMTIME = std::mem::zeroed();
            GetLocalTime(&mut time);
            let mut buffer = Buffer {
                bytes: [0; 1024],
                len: 0,
            };
            let _ = writeln!(
                buffer,
                "{{\"time_local\":\"{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}\",\"level\":\"ERROR\",\"target\":\"galgear::native_crash\",\"code\":\"0x{:08X}\",\"address\":\"{:p}\",\"module_base\":\"{:p}\",\"thread\":{}}}",
                time.wYear,
                time.wMonth,
                time.wDay,
                time.wHour,
                time.wMinute,
                time.wSecond,
                time.wMilliseconds,
                record.ExceptionCode as u32,
                record.ExceptionAddress,
                GetModuleHandleW(std::ptr::null()),
                GetCurrentThreadId()
            );
            let mut written = 0;
            WriteFile(
                file.as_raw_handle(),
                buffer.bytes.as_ptr(),
                buffer.len as u32,
                &mut written,
                std::ptr::null_mut(),
            );
            FlushFileBuffers(file.as_raw_handle());
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}
