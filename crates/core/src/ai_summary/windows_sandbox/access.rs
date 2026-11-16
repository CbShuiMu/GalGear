//! 临时 ACL 仅添加/撤销本次随机 SID，保留其他用户及并行任务的权限。
use super::child::wide;
use std::{
    ffi::OsString,
    io,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
            REVOKE_ACCESS, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW, SetNamedSecurityInfoW,
            TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, DeriveCapabilitySidsFromName, FreeSid,
        Isolation::{
            CreateAppContainerProfile, DeleteAppContainerProfile,
            DeriveAppContainerSidFromAppContainerName, GetAppContainerFolderPath,
        },
        PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    },
    Storage::FileSystem::{DELETE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE},
    System::{
        Com::CoTaskMemFree,
        Threading::{CreateMutexW, INFINITE, ReleaseMutex, WaitForSingleObject},
    },
};

// Get/Set ACL 无原子追加操作，同一桌面会话的应用实例共用锁，恢复也走同一入口。
struct AclLock(OwnedHandle);

impl AclLock {
    fn acquire() -> io::Result<Self> {
        let name = wide("Local\\GalGear.Analysis.ACL.v1".as_ref())?;
        // SAFETY: null attributes make a non-inheritable mutex; name is NUL terminated.
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: this call returned a new owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        // SAFETY: handle remains owned throughout this wait and until ReleaseMutex.
        match unsafe { WaitForSingleObject(handle.as_raw_handle(), INFINITE) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Drop for AclLock {
    fn drop(&mut self) {
        // SAFETY: acquire and this private guard's drop occur on the same thread.
        unsafe {
            ReleaseMutex(self.0.as_raw_handle());
        }
    }
}

pub(super) struct Sid {
    pointer: PSID,
    package: bool,
    name: Option<Vec<u16>>,
}

// SAFETY: Windows allocates immutable SID bytes; ownership and freeing remain exclusive to Sid.
unsafe impl Send for Sid {}

impl Sid {
    pub(super) fn existing(name: &str) -> io::Result<Self> {
        let name = wide(name.as_ref())?;
        let mut pointer = ptr::null_mut();
        // SAFETY: name is terminated, the API allocates a SID freed by Sid::drop.
        let status =
            unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut pointer) };
        if status < 0 {
            return Err(io::Error::other(format!(
                "AppContainer SID failed: {status:#x}"
            )));
        }
        Ok(Self {
            pointer,
            package: true,
            name: Some(name),
        })
    }

    pub(super) fn package(name: &str) -> io::Result<Self> {
        let name = wide(name.as_ref())?;
        let mut pointer = ptr::null_mut();
        // SAFETY: name is NUL terminated and pointer receives a newly allocated SID.
        let result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                ptr::null(),
                0,
                &mut pointer,
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "AppContainer identity failed: {result:#x}"
            )));
        }
        Ok(Self {
            pointer,
            package: true,
            name: Some(name),
        })
    }

    pub(super) fn capability(name: &str) -> io::Result<Self> {
        let name = wide(name.as_ref())?;
        let (mut groups, mut capabilities) = (ptr::null_mut(), ptr::null_mut());
        let (mut group_count, mut count) = (0, 0);
        // SAFETY: all outputs are initialized pointers/counts; the API owns allocation.
        if unsafe {
            DeriveCapabilitySidsFromName(
                name.as_ptr(),
                &mut groups,
                &mut group_count,
                &mut capabilities,
                &mut count,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the API returns arrays of the reported lengths allocated with LocalAlloc.
        unsafe {
            for index in 0..group_count {
                LocalFree(*groups.add(index as usize));
            }
            LocalFree(groups.cast());
            if count != 1 {
                for index in 0..count {
                    LocalFree(*capabilities.add(index as usize));
                }
                LocalFree(capabilities.cast());
                return Err(io::Error::other("Unexpected capability SID count"));
            }
            let pointer = *capabilities;
            LocalFree(capabilities.cast());
            Ok(Self {
                pointer,
                package: false,
                name: None,
            })
        }
    }

    pub(super) fn as_ptr(&self) -> PSID {
        self.pointer
    }

    pub(super) fn remove_profile(&mut self) -> io::Result<()> {
        if let Some(name) = &self.name {
            // SAFETY: name identifies this task's profile; deleting an absent profile is idempotent.
            let status = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
            if status < 0 {
                return Err(io::Error::other(format!(
                    "AppContainer cleanup failed: {status:#x}"
                )));
            }
            self.name = None;
        }
        Ok(())
    }

    pub(super) fn profile_directory(&self) -> io::Result<PathBuf> {
        let mut text = ptr::null_mut();
        let mut path = ptr::null_mut();
        // SAFETY: SID is live, output buffers are freed with their documented allocators.
        unsafe {
            if ConvertSidToStringSidW(self.pointer, &mut text) == 0 {
                return Err(io::Error::last_os_error());
            }
            let status = GetAppContainerFolderPath(text, &mut path);
            LocalFree(text.cast());
            if status < 0 {
                return Err(io::Error::other(format!(
                    "AppContainer folder failed: {status:#x}"
                )));
            }
            let mut length = 0;
            while *path.add(length) != 0 {
                length += 1;
            }
            let result = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
                path, length,
            )));
            CoTaskMemFree(path.cast());
            Ok(result)
        }
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: each SID is freed exactly once using its allocating API's required deleter.
        unsafe {
            if self.package {
                FreeSid(self.pointer);
                if let Some(name) = &self.name {
                    DeleteAppContainerProfile(name.as_ptr());
                }
            } else {
                LocalFree(self.pointer);
            }
        }
    }
}

pub(super) struct Grant;

impl Grant {
    pub(super) fn revoke_path(path: &std::path::Path, sid: &Sid) -> io::Result<()> {
        use std::os::windows::fs::MetadataExt;
        let ancestors: Vec<_> = path.ancestors().collect();
        for (index, ancestor) in ancestors.iter().rev().enumerate() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Sandbox cleanup path contains a reparse point",
                    ));
                }
                Ok(_) => {}
                // 磁盘根失联时保留记录；已删除的子路径已无 ACL 可清理。
                Err(error) if index > 0 && error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        update(path, sid, 0, REVOKE_ACCESS)
    }

    pub(super) fn list_directory(path: PathBuf, sid: &Sid) -> io::Result<()> {
        update_with_inheritance(
            &path,
            sid,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            GRANT_ACCESS,
            0,
        )
    }
    pub(super) fn deny_profile_write(sid: &Sid) -> io::Result<()> {
        let path = sid.profile_directory()?;
        // 覆盖 Windows 为此包创建的默认完全控制权限，避免任务写入 AppData 下的 profile。
        update(
            &path,
            sid,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            SET_ACCESS,
        )
    }
    pub(super) fn allow(path: PathBuf, sid: &Sid, writable: bool) -> io::Result<()> {
        let rights = FILE_GENERIC_READ
            | FILE_GENERIC_EXECUTE
            | if writable {
                FILE_GENERIC_WRITE | DELETE
            } else {
                0
            };
        if let Err(error) = update(&path, sid, rights, GRANT_ACCESS) {
            // 继承传播可能部分成功，初始化失败也撤销已经写入的本轮 SID。
            update(&path, sid, 0, REVOKE_ACCESS)?;
            return Err(error);
        }
        Ok(())
    }
}

fn update(path: &std::path::Path, sid: &Sid, rights: u32, mode: i32) -> io::Result<()> {
    let inheritance = if path.is_dir() {
        SUB_CONTAINERS_AND_OBJECTS_INHERIT
    } else {
        0
    };
    update_with_inheritance(path, sid, rights, mode, inheritance)
}

fn update_with_inheritance(
    path: &std::path::Path,
    sid: &Sid,
    rights: u32,
    mode: i32,
    inheritance: u32,
) -> io::Result<()> {
    let _lock = AclLock::acquire()?;
    let name = wide(path.as_os_str())?;
    let (mut descriptor, mut old_acl, mut acl) =
        (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: rights,
        grfAccessMode: mode,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.as_ptr().cast(),
            ..Default::default()
        },
    };
    // SAFETY: paths are NUL terminated, output pointers live through each call; returned buffers
    // remain allocated until SetNamedSecurityInfoW finishes, and are freed on every path.
    let result = unsafe {
        let mut result = GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_acl,
            ptr::null_mut(),
            &mut descriptor,
        );
        if result == 0 {
            result = SetEntriesInAclW(1, &entry, old_acl, &mut acl);
        }
        if result == 0 {
            result = SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null(),
            );
        }
        LocalFree(acl.cast());
        LocalFree(descriptor);
        result
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "AI sandbox ACL {}: {}",
                path.display(),
                io::Error::from_raw_os_error(result as i32)
            ),
        ))
    }
}
