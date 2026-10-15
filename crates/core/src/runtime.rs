//! Detect real executable paths independently of the process that launched a game.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Stopped,
    Running,
    Unknown,
}

/// Returns unknown when the executable is unavailable or inspection fails.
pub fn detect(executable: Option<&str>) -> Status {
    let Some(path) = executable.filter(|p| Path::new(p).is_file()) else {
        return Status::Unknown;
    };
    match process_ids(Path::new(path)) {
        Ok(ids) if ids.is_empty() => Status::Stopped,
        Ok(_) => Status::Running,
        Err(_) => Status::Unknown,
    }
}

#[cfg(windows)]
struct ProcessSnapshot {
    found: Vec<(u32, std::path::PathBuf)>,
    uncertain: std::collections::BTreeSet<String>,
}

#[cfg(windows)]
fn processes(names: &std::collections::BTreeSet<String>) -> Result<ProcessSnapshot, String> {
    use windows_sys::Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let mut found = Vec::new();
    let mut uncertain = std::collections::BTreeSet::new();
    // Toolhelp calls are synchronous; every process handle has an RAII owner.
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let handle = Handle(raw);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(handle.0, &mut entry) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        loop {
            let length = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]).to_ascii_lowercase();
            if names.contains(&name) {
                let process =
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if process.is_null() {
                    if GetLastError() != ERROR_INVALID_PARAMETER {
                        uncertain.insert(name);
                    }
                } else {
                    let process = Handle(process);
                    let mut buffer = vec![0_u16; 32768];
                    let mut length = buffer.len() as u32;
                    if QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut length)
                        == 0
                    {
                        uncertain.insert(name);
                    } else {
                        let path = std::path::PathBuf::from(String::from_utf16_lossy(
                            &buffer[..length as usize],
                        ));
                        match std::fs::canonicalize(path) {
                            Ok(path) => found.push((entry.th32ProcessID, path)),
                            Err(_) => {
                                uncertain.insert(name);
                            }
                        }
                    }
                }
            }
            if Process32NextW(handle.0, &mut entry) == 0 {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                break;
            }
        }
    }
    Ok(ProcessSnapshot { found, uncertain })
}

#[cfg(windows)]
pub fn process_ids(path: &Path) -> Result<Vec<u32>, String> {
    let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .ok_or("Missing executable name")?
        .to_string_lossy()
        .to_ascii_lowercase();
    let ProcessSnapshot { found, uncertain } = processes(&[name.clone()].into())?;
    let ids: Vec<_> = found
        .iter()
        .filter(|(_, p)| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&canonical.to_string_lossy())
        })
        .map(|(id, _)| *id)
        .collect();
    if ids.is_empty() && uncertain.contains(&name) {
        Err("Cannot inspect a matching process".into())
    } else {
        Ok(ids)
    }
}

/// Inspect the entire library using one OS process snapshot per poll.
#[cfg(windows)]
pub fn detect_many(
    games: &[(String, Option<String>)],
) -> std::collections::BTreeMap<String, Status> {
    let paths: Vec<_> = games
        .iter()
        .map(|(id, exe)| (id, exe.as_ref().and_then(|p| std::fs::canonicalize(p).ok())))
        .collect();
    let names = paths
        .iter()
        .filter_map(|(_, p)| {
            p.as_ref()?
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
        })
        .collect();
    let results = processes(&names);
    paths
        .into_iter()
        .map(|(id, path)| {
            let status = match (path, &results) {
                (Some(path), Ok(ProcessSnapshot { found, uncertain })) => {
                    if found.iter().any(|(_, p)| {
                        p.to_string_lossy()
                            .eq_ignore_ascii_case(&path.to_string_lossy())
                    }) {
                        Status::Running
                    } else if path.file_name().is_some_and(|n| {
                        uncertain.contains(&n.to_string_lossy().to_ascii_lowercase())
                    }) {
                        Status::Unknown
                    } else {
                        Status::Stopped
                    }
                }
                _ => Status::Unknown,
            };
            (id.clone(), status)
        })
        .collect()
}

#[cfg(not(windows))]
pub fn detect_many(
    games: &[(String, Option<String>)],
) -> std::collections::BTreeMap<String, Status> {
    games
        .iter()
        .map(|(id, _)| (id.clone(), Status::Unknown))
        .collect()
}

#[cfg(not(windows))]
pub fn process_ids(_path: &Path) -> Result<Vec<u32>, String> {
    Err("Windows process inspection required".into())
}

/// Focus the real game window and send the user's configured Magpie hotkey.
/// This confirms only delivery, never the scaling result.
#[cfg(windows)]
pub fn trigger_magpie(executable: &Path, hotkey: &str) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::*,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };
    struct Search {
        ids: Vec<u32>,
        window: HWND,
    }
    unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> windows_sys::core::BOOL {
        // EnumWindows invokes this callback synchronously with our live Search.
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        if search.ids.contains(&pid)
            && unsafe { IsWindowVisible(window) } != 0
            && unsafe { GetWindow(window, GW_OWNER) }.is_null()
        {
            search.window = window;
            return 0;
        }
        1
    }
    let mut search = Search {
        ids: process_ids(executable)?,
        window: std::ptr::null_mut(),
    };
    let keys = hotkey_keys(hotkey)?;
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
        if search.window.is_null() {
            return Err("No visible game window".into());
        }
        if IsIconic(search.window) != 0 {
            ShowWindow(search.window, SW_RESTORE);
        }
        if SetForegroundWindow(search.window) == 0 || GetForegroundWindow() != search.window {
            return Err("Cannot focus game window".into());
        }
        let events: Vec<INPUT> = keys
            .iter()
            .map(|key| (*key, 0))
            .chain(keys.iter().rev().map(|key| (*key, KEYEVENTF_KEYUP)))
            .map(|(key, flags)| {
                let mut input: INPUT = std::mem::zeroed();
                input.r#type = INPUT_KEYBOARD;
                input.Anonymous.ki = KEYBDINPUT {
                    wVk: key,
                    dwFlags: flags,
                    ..std::mem::zeroed()
                };
                input
            })
            .collect();
        if SendInput(
            events.len() as u32,
            events.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        ) != events.len() as u32
        {
            return Err("Cannot deliver Magpie hotkey".into());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn trigger_magpie(_executable: &Path, _hotkey: &str) -> Result<(), String> {
    Err("Windows required".into())
}

/// Validate a chord before sending any keyboard input.
pub fn hotkey_keys(hotkey: &str) -> Result<Vec<u16>, String> {
    let mut keys = Vec::new();
    for token in hotkey.split('+') {
        let upper = token.trim().to_ascii_uppercase();
        let key = match upper.as_str() {
            "CTRL" => 0x11,
            "ALT" => 0x12,
            "SHIFT" => 0x10,
            s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
                u16::from(s.as_bytes()[0])
            }
            _ => return Err("Use a chord such as Ctrl+Alt+A".into()),
        };
        if keys.contains(&key) {
            return Err("Duplicate hotkey key".into());
        }
        keys.push(key);
    }
    if !(2..=4).contains(&keys.len()) || keys.last().is_none_or(|k| *k < 0x30) {
        return Err("A modifier and a letter or digit are required".into());
    }
    Ok(keys)
}

#[derive(Clone, Debug, Serialize)]
pub struct LocaleProfile {
    pub guid: String,
    pub name: String,
    pub locale: String,
    pub admin: bool,
}

/// Read global profiles without rewriting Locale Emulator configuration.
pub fn locale_profiles(tool: &Path) -> Result<Vec<LocaleProfile>, String> {
    let path = tool
        .parent()
        .ok_or("Missing tool directory")?
        .join("LEConfig.xml");
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = scraper::Html::parse_document(&text);
    let selector = scraper::Selector::parse("profiles > profile").map_err(|e| e.to_string())?;
    Ok(doc
        .select(&selector)
        .filter_map(|element| {
            let field = |name: &str| {
                element
                    .children()
                    .filter_map(scraper::ElementRef::wrap)
                    .find(|c| c.value().name() == name)
                    .map(|e| e.text().collect::<String>())
                    .unwrap_or_default()
            };
            Some(LocaleProfile {
                guid: element.value().attr("guid")?.into(),
                name: element.value().attr("name").unwrap_or_default().into(),
                locale: field("location"),
                admin: field("runasadmin").eq_ignore_ascii_case("true"),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_hotkeys_fail_before_input_and_missing_executable_is_unknown() {
        assert!(hotkey_keys("Ctrl+Ctrl+A").is_err());
        assert!(hotkey_keys("A").is_err());
        assert_eq!(detect(None), Status::Unknown);
    }
    #[cfg(windows)]
    #[test]
    fn real_executable_is_detected_independently_of_a_launch_handle() {
        let executable = std::env::current_exe().unwrap();
        let path = executable.to_string_lossy().into_owned();
        assert!(
            process_ids(&executable)
                .unwrap()
                .contains(&std::process::id())
        );
        assert_eq!(
            detect_many(&[("game".into(), Some(path))])["game"],
            Status::Running
        );
    }
    #[test]
    fn japanese_profile_can_be_selected_without_changing_tool_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let xml = "<LEConfig><Profiles><Profile Guid=\"japanese\" Name=\"Japanese\"><Location>ja-JP</Location><RunAsAdmin>false</RunAsAdmin></Profile></Profiles></LEConfig>";
        std::fs::write(temp.path().join("LEConfig.xml"), xml).unwrap();
        let profiles = locale_profiles(&temp.path().join("LEProc.exe")).unwrap();
        assert_eq!(profiles[0].locale, "ja-JP");
        assert_eq!(
            std::fs::read_to_string(temp.path().join("LEConfig.xml")).unwrap(),
            xml
        );
    }
}
