use super::{SessionLog, initialize_at, privacy::Redactor};
use log::Level;
use std::{fs, process::Command, sync::Arc};

#[test]
fn sessions_never_overwrite_existing_logs() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("logs");
    let first = SessionLog::create(&directory, "2026-11-06_120000_123").unwrap();
    first
        .write(Level::Info, "galgear::test", "original session")
        .unwrap();
    let original = fs::read(&first.path).unwrap();
    let second = SessionLog::create(&directory, "2026-11-06_120000_123").unwrap();
    assert_ne!(first.path, second.path);
    assert_eq!(fs::read(&first.path).unwrap(), original);
    assert_eq!(
        first.path.file_name().unwrap(),
        "galgear_2026-11-06_120000_123.log"
    );
}

#[test]
fn concurrent_records_remain_independent_json_lines() {
    let root = tempfile::tempdir().unwrap();
    let logger = Arc::new(SessionLog::create(root.path(), "parallel").unwrap());
    std::thread::scope(|scope| {
        for index in 0..8 {
            let logger = logger.clone();
            scope.spawn(move || {
                for item in 0..25 {
                    logger
                        .write(
                            Level::Info,
                            "galgear::test",
                            &format!("thread={index} item={item}\nnext line"),
                        )
                        .unwrap();
                }
            });
        }
    });
    let content = fs::read_to_string(&logger.path).unwrap();
    assert_eq!(content.lines().count(), 200);
    for line in content.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(record["message"].as_str().unwrap().ends_with("\nnext line"));
    }
}

#[test]
fn credentials_urls_and_personal_paths_are_redacted() {
    let redactor = Redactor::new().unwrap();
    let input = "Authorization: Bearer auth-value\nCookie: session=cookie-value\nurl=https://user:pass@example.com/api?sig=signed-value\naccess_token=token-value password=\"long secret\" api_key='api-value'\nC:\\Users\\Alice\\AppData\\Local user@example.com\n中文错误: os error 5";
    let output = redactor.clean(input);
    for secret in [
        "auth-value",
        "cookie-value",
        "signed-value",
        "token-value",
        "long secret",
        "api-value",
        "Alice",
        "user@example.com",
        "user:pass",
    ] {
        assert!(!output.contains(secret), "leaked {secret}: {output}");
    }
    assert!(output.contains("中文错误: os error 5"));
    assert_eq!(
        redactor.clean("at http://tauri.localhost/bridge.js:42:7?token=private"),
        "at [app]/bridge.js:42:7"
    );
    let long = redactor.clean(&"中".repeat(40_000));
    assert!(long.ends_with(" [truncated]"));
}

#[test]
fn unwritable_destination_reports_error() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("logs");
    fs::write(&file, "do not replace").unwrap();
    assert!(SessionLog::create(&file, "invalid").is_err());
    assert_eq!(fs::read_to_string(file).unwrap(), "do not replace");
}

fn run_crash_child(kind: &str) -> String {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "diagnostics::tests::crash_child",
            "--nocapture",
        ])
        .env("GALGEAR_TEST_LOG_DIR", directory.path())
        .env("GALGEAR_TEST_CRASH_KIND", kind)
        .output()
        .unwrap();
    assert!(!output.status.success(), "child must fail");
    let file = fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::read_to_string(file).unwrap()
}

#[test]
fn panic_records_location_backtrace_and_last_operation_before_exit() {
    let log = run_crash_child("panic");
    assert!(log.contains("last_operation"));
    assert!(log.contains("galgear::panic"));
    assert!(log.contains("diagnostic test panic"));
    assert!(log.contains("tests.rs"));
    assert!(log.contains("stack backtrace") || log.contains("0:"));
    assert!(!log.contains("session_end"));
}

#[cfg(windows)]
#[test]
fn native_exception_records_fault_code_and_address() {
    let log = run_crash_child("native");
    assert!(log.contains("last_operation"));
    assert!(log.contains("galgear::native_crash"), "{log}");
    assert!(log.contains("0xE0424747"));
    assert!(log.contains("module_base"));
    assert!(!log.contains("session_end"));
}

#[test]
#[ignore = "Only launched in an isolated child with a temporary log directory"]
fn crash_child() {
    let Ok(directory) = std::env::var("GALGEAR_TEST_LOG_DIR") else {
        return;
    };
    initialize_at(std::path::Path::new(&directory)).unwrap();
    log::info!("last_operation");
    if std::env::var("GALGEAR_TEST_CRASH_KIND").as_deref() == Ok("native") {
        #[cfg(windows)]
        // SAFETY: 仅在本测试派生的独立进程触发无参数、不可继续的测试异常；禁用系统错误弹窗。
        unsafe {
            use windows_sys::Win32::System::Diagnostics::Debug::{
                RaiseException, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
            };
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
            RaiseException(0xE0424747, 1, 0, std::ptr::null());
        }
    }
    panic!("diagnostic test panic");
}
