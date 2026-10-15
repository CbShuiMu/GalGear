//! Credentials never enter settings.json, catalog caches or log output.
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FanCredentials {
    pub token: String,
    pub cookie: String,
    pub mode: String,
}
impl std::fmt::Debug for FanCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FanCredentials")
            .field("token", &"[redacted]")
            .field("cookie", &"[redacted]")
            .field("mode", &self.mode)
            .finish()
    }
}
impl FanCredentials {
    pub fn validate(&self) -> Result<(), Message> {
        if !["csrf", "bearer"].contains(&self.mode.as_str())
            || self.token.contains(['\r', '\n'])
            || self.cookie.contains(['\r', '\n'])
            || self.token.len() > 1024
            || self.cookie.len() > 2200
        {
            return Err(Message::new("auth.invalid"));
        }
        Ok(())
    }
}
pub fn target(config: &Path) -> String {
    format!(
        "GalGear/2DFan/{:x}",
        Sha256::digest(config.to_string_lossy().to_lowercase().as_bytes())
    )
}
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
pub fn save(target: &str, value: &FanCredentials) -> Result<(), Message> {
    use windows_sys::Win32::Security::Credentials::*;
    value.validate()?;
    let mut bytes = serde_json::to_vec(value).map_err(|_| Message::new("auth.store_error"))?;
    if bytes.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(Message::new("auth.too_long"));
    }
    let mut name = wide(target);
    let mut user = wide("2DFan");
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: name.as_mut_ptr(),
        UserName: user.as_mut_ptr(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    let ok = unsafe { CredWriteW(&credential, 0) } != 0;
    bytes.fill(0);
    if ok {
        Ok(())
    } else {
        Err(Message::new("auth.store_error"))
    }
}
#[cfg(windows)]
pub fn load(target: &str) -> Result<FanCredentials, Message> {
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, GetLastError},
        Security::Credentials::*,
    };
    let name = wide(target);
    let mut pointer = std::ptr::null_mut();
    unsafe {
        if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) == 0 {
            return if GetLastError() == ERROR_NOT_FOUND {
                Ok(FanCredentials {
                    mode: "csrf".into(),
                    ..Default::default()
                })
            } else {
                Err(Message::new("auth.store_error"))
            };
        }
        let credential = &*pointer;
        let result = if credential.CredentialBlobSize == 0 {
            Ok(FanCredentials {
                mode: "csrf".into(),
                ..Default::default()
            })
        } else {
            serde_json::from_slice(std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            ))
            .map_err(|_| Message::new("auth.store_error"))
        };
        CredFree(pointer.cast());
        result
    }
}
#[cfg(windows)]
pub fn clear(target: &str) -> Result<(), Message> {
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, GetLastError},
        Security::Credentials::*,
    };
    let name = wide(target);
    unsafe {
        if CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) != 0
            || GetLastError() == ERROR_NOT_FOUND
        {
            Ok(())
        } else {
            Err(Message::new("auth.store_error"))
        }
    }
}
#[cfg(not(windows))]
pub fn save(_: &str, _: &FanCredentials) -> Result<(), Message> {
    Err(Message::new("auth.platform"))
}
#[cfg(not(windows))]
pub fn load(_: &str) -> Result<FanCredentials, Message> {
    Err(Message::new("auth.platform"))
}
#[cfg(not(windows))]
pub fn clear(_: &str) -> Result<(), Message> {
    Err(Message::new("auth.platform"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_redacts_secrets_and_headers_reject_line_breaks() {
        let value = FanCredentials {
            token: "private-token".into(),
            cookie: "private-cookie".into(),
            mode: "csrf".into(),
        };
        let debug = format!("{value:?}");
        assert!(!debug.contains("private"));
        assert!(
            FanCredentials {
                token: "secret\r\nInjected: yes".into(),
                ..value
            }
            .validate()
            .is_err()
        );
    }
}
