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
fn executable_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
}

#[cfg(windows)]
fn matches_executable(image: &Path, executable: &Path) -> bool {
    if image
        .to_string_lossy()
        .eq_ignore_ascii_case(&executable.to_string_lossy())
    {
        return true;
    }
    // Some launchers run a PE image with the same name and a different extension.
    image
        .parent()
        .zip(executable.parent())
        .is_some_and(|(a, b)| {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        })
        && executable_stem(image).is_some_and(|name| Some(name) == executable_stem(executable))
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
            let filename = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if let Some(name) = executable_stem(Path::new(&filename))
                && names.contains(&name)
            {
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
    let name = executable_stem(path).ok_or("Missing executable name")?;
    let ProcessSnapshot { found, uncertain } = processes(&[name.clone()].into())?;
    let ids: Vec<_> = found
        .iter()
        .filter(|(_, p)| matches_executable(p, &canonical))
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
        .filter_map(|(_, p)| executable_stem(p.as_ref()?))
        .collect();
    let results = processes(&names);
    paths
        .into_iter()
        .map(|(id, path)| {
            let status = match (path, &results) {
                (Some(path), Ok(ProcessSnapshot { found, uncertain })) => {
                    if found.iter().any(|(_, p)| matches_executable(p, &path)) {
                        Status::Running
                    } else if executable_stem(&path).is_some_and(|n| uncertain.contains(&n)) {
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
// Names shared by Magpie config decoding and synthesized keyboard input.
const HOTKEY_NAMES: &[(u16, &str)] = &[
    (0x03, "Break"), (0x08, "Backspace"), (0x09, "Tab"), (0x0d, "Enter"),
    (0x10, "Shift"), (0x11, "Ctrl"), (0x12, "Alt"), (0x13, "Pause"),
    (0x20, "Space"), (0x21, "PageUp"), (0x22, "PageDown"), (0x23, "End"),
    (0x24, "Home"), (0x25, "Left"), (0x26, "Up"), (0x27, "Right"), (0x28, "Down"),
    (0x2c, "PrintScreen"), (0x2d, "Insert"), (0x2e, "Delete"), (0x5b, "Win"),
    (0x6a, "Multiply"), (0x6b, "Add"), (0x6d, "Subtract"), (0x6e, "Decimal"), (0x6f, "Divide"),
    (0xba, "Semicolon"), (0xbb, "Equals"), (0xbc, "Comma"), (0xbd, "Minus"),
    (0xbe, "Period"), (0xbf, "Slash"), (0xc0, "Backtick"),
    (0xdb, "LeftBracket"), (0xdc, "Backslash"), (0xdd, "RightBracket"), (0xde, "Quote"),
];

pub fn hotkey_key_name(key: u16) -> Option<String> {
    match key {
        0x30..=0x39 | 0x41..=0x5a => Some(char::from(key as u8).to_string()),
        0x60..=0x69 => Some(format!("NumPad{}", key - 0x60)),
        0x70..=0x87 => Some(format!("F{}", key - 0x6f)),
        _ => HOTKEY_NAMES.iter().find(|(code, _)| *code == key).map(|(_, name)| (*name).into()),
    }
}

pub fn hotkey_keys(hotkey: &str) -> Result<Vec<u16>, String> {
    let mut keys = Vec::new();
    for token in hotkey.split('+') {
        let token = token.trim();
        let key = (0..=255).find(|key| hotkey_key_name(*key).is_some_and(|name| name.eq_ignore_ascii_case(token)))
            .ok_or("Use a shortcut such as Alt+Shift+A or Ctrl+F12")?;
        if keys.contains(&key) || keys.last().is_some_and(|key| !matches!(key, 0x10..=0x12 | 0x5b)) {
            return Err("Use distinct modifiers followed by one key".into());
        }
        keys.push(key);
    }
    if !(2..=5).contains(&keys.len()) || !matches!(keys[0], 0x10..=0x12 | 0x5b) {
        return Err("At least one modifier is required".into());
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
    #[cfg(windows)]
    #[test]
    fn alternate_image_extension_requires_the_same_directory_and_stem() {
        let launcher = Path::new(r"C:\Games\Title\game.exe");
        assert!(matches_executable(
            Path::new(r"c:\games\title\GAME.log"),
            launcher
        ));
        assert!(!matches_executable(
            Path::new(r"C:\Games\Other\game.log"),
            launcher
        ));
        assert!(!matches_executable(
            Path::new(r"C:\Games\Title\updater.log"),
            launcher
        ));
    }

    #[cfg(windows)]
    #[test]
    fn runtime_detection_finds_a_running_pe_image_with_a_log_extension() {
        use std::process::{Child, Command, Stdio};

        struct Probe(Child);
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let launcher = directory.path().join("game.exe");
        let image = directory.path().join("game.log");
        std::fs::write(&launcher, []).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &image).unwrap();
        let probe = Probe(
            Command::new(&image)
                .args([
                    "--ignored",
                    "--exact",
                    "runtime::tests::runtime_probe_waits_for_input",
                ])
                .env("GALGEAR_RUNTIME_PROBE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let other = directory.path().join("other").join("game.exe");
        std::fs::create_dir(other.parent().unwrap()).unwrap();
        std::fs::write(&other, []).unwrap();

        assert!(process_ids(&launcher).unwrap().contains(&probe.0.id()));
        assert_eq!(detect(Some(launcher.to_str().unwrap())), Status::Running);
        assert_eq!(
            detect_many(&[
                ("game".into(), Some(launcher.to_string_lossy().into_owned())),
                ("other".into(), Some(other.to_string_lossy().into_owned())),
            ]),
            [
                ("game".into(), Status::Running),
                ("other".into(), Status::Stopped),
            ]
            .into()
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Subprocess helper for the renamed PE image test"]
    fn runtime_probe_waits_for_input() {
        if std::env::var_os("GALGEAR_RUNTIME_PROBE").is_some() {
            let mut byte = [0];
            std::io::Read::read(&mut std::io::stdin(), &mut byte).unwrap();
        }
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
