use super::{Journal, recover};
use crate::ai_summary::windows_sandbox::{
    Sandbox,
    access::{Grant, Sid},
    child::wide,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION,
    },
};

fn name() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).unwrap();
    format!("GalGear.Analysis.{:032x}", u128::from_le_bytes(bytes))
}

fn dacl(path: &Path) -> String {
    let path = wide(path.as_os_str()).unwrap();
    let (mut descriptor, mut text) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: all outputs are initialized and both API allocated buffers are freed below.
    unsafe {
        assert_eq!(
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor
            ),
            0
        );
        assert_ne!(
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                ptr::null_mut()
            ),
            0
        );
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        LocalFree(descriptor);
        result
    }
}

fn setup(root: &Path) -> Value {
    for path in ["game/work", "saves", "skill"] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    for path in ["game/input", "saves/selected", "saves/other", "skill/input"] {
        fs::write(root.join(path), b"untouched fixture").unwrap();
    }
    json!({"game":root.join("game"), "skill":root.join("skill"), "save_candidates":[{"path":root.join("saves/selected")}]})
}

fn snapshot(root: &Path) -> Vec<String> {
    [
        "game",
        "game/input",
        "game/work",
        "saves",
        "saves/selected",
        "saves/other",
        "skill/input",
    ]
    .iter()
    .map(|path| dacl(&root.join(path)))
    .collect()
}

struct Owner(Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn owner(root: &Path, mode: &str) -> (Owner, Value) {
    use std::os::windows::process::CommandExt;
    let mut child = Owner(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ai_summary::windows_sandbox::journal::tests::crash_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("GALGEAR_RECOVERY_FIXTURE", root)
            .env("GALGEAR_RECOVERY_MODE", mode)
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(bytes) = fs::read(root.join("ready.json"))
            && let Ok(ready) = serde_json::from_slice(&bytes)
        {
            return (child, ready);
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Crash fixture exited early"
        );
        assert!(
            Instant::now() < deadline,
            "Crash fixture did not become ready"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "subprocess fixture, forcibly killed by recovery tests"]
fn crash_fixture() {
    let Some(root) = std::env::var_os("GALGEAR_RECOVERY_FIXTURE") else {
        return;
    };
    let root = Path::new(&root);
    let mode = std::env::var("GALGEAR_RECOVERY_MODE").unwrap();
    let journal_dir = root.join("journal");
    let work = root.join("game/work").canonicalize().unwrap();
    let config = json!({"game":root.join("game"),"skill":root.join("skill"),"save_candidates":[{"path":root.join("saves/selected")}]});
    let (profile, identity, _sandbox, _journal, _sid) = if mode == "full" {
        let sandbox = Sandbox::new_with_journal(
            &mut Command::new(std::env::current_exe().unwrap()),
            &work,
            &config,
            false,
            &journal_dir,
        )
        .unwrap();
        let file = fs::read_dir(&journal_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name();
        let identity = file
            .to_str()
            .unwrap()
            .strip_suffix(".jsonl")
            .unwrap()
            .to_owned();
        (
            sandbox.sid.profile_directory().unwrap(),
            identity,
            Some(sandbox),
            None,
            None,
        )
    } else {
        let identity = name();
        let mut journal = Journal::create(&journal_dir, &identity, &work).unwrap();
        let sid = Sid::package(&identity).unwrap();
        let profile = sid.profile_directory().unwrap();
        if mode == "partial" {
            let input = root.join("game").canonicalize().unwrap();
            journal.record(&input).unwrap();
            Grant::allow(input, &sid, false).unwrap();
            // 覆盖落盘后尚未授权，以及追加被中断的两个恢复边界。
            journal
                .record(&root.join("skill").canonicalize().unwrap())
                .unwrap();
            journal.file.write_all(b"\"truncated").unwrap();
            journal.file.sync_all().unwrap();
        }
        (profile, identity, None, Some(journal), Some(sid))
    };
    fs::write(
        root.join("ready.json"),
        serde_json::to_vec(&json!({"profile":profile,"name":identity})).unwrap(),
    )
    .unwrap();
    thread::sleep(Duration::from_secs(30));
}

#[test]
fn forced_owner_exit_is_recovered_without_touching_live_permissions() {
    let root = tempfile::tempdir().unwrap();
    let config = setup(root.path());
    let mut command = Command::new(std::env::current_exe().unwrap());
    let mut live = Sandbox::new_with_journal(
        &mut command,
        &root.path().join("game/work"),
        &config,
        false,
        &root.path().join("live-journal"),
    )
    .unwrap();
    let baseline = snapshot(root.path());
    let (mut owner, ready) = owner(root.path(), "full");
    let profile = PathBuf::from(ready["profile"].as_str().unwrap());
    assert!(profile.is_dir());
    let granted = snapshot(root.path());
    assert_ne!(baseline, granted);
    assert!(recover(&root.path().join("journal")).is_empty());
    assert_eq!(snapshot(root.path()), granted, "活跃实例不能被回收");
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    assert!(profile.is_dir(), "强制终止应跳过析构清理");
    assert_eq!(snapshot(root.path()), granted);
    assert!(recover(&root.path().join("journal")).is_empty());
    assert_eq!(
        snapshot(root.path()),
        baseline,
        "只移除死亡任务的 SID，保留活跃身份和用户 ACL"
    );
    assert!(!profile.exists());
    assert_eq!(
        fs::read_dir(root.path().join("journal")).unwrap().count(),
        0
    );
    // 同名创建成功证明 Windows 身份注册也已删除，而不仅是 profile 文件目录。
    drop(Sid::package(ready["name"].as_str().unwrap()).unwrap());
    assert!(recover(&root.path().join("journal")).is_empty());
    for path in ["game/input", "saves/selected", "saves/other", "skill/input"] {
        assert_eq!(
            fs::read(root.path().join(path)).unwrap(),
            b"untouched fixture"
        );
    }
    live.revoke().unwrap();
    drop(live);
    assert_eq!(
        fs::read_dir(root.path().join("live-journal"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn interrupted_initialization_and_torn_last_record_are_recoverable() {
    for mode in ["partial", "profile"] {
        let root = tempfile::tempdir().unwrap();
        setup(root.path());
        let baseline = snapshot(root.path());
        let (mut owner, ready) = owner(root.path(), mode);
        owner.0.kill().unwrap();
        owner.0.wait().unwrap();
        assert!(recover(&root.path().join("journal")).is_empty(), "{mode}");
        assert_eq!(snapshot(root.path()), baseline);
        assert!(!Path::new(ready["profile"].as_str().unwrap()).exists());
        drop(Sid::package(ready["name"].as_str().unwrap()).unwrap());
        assert_eq!(
            fs::read_dir(root.path().join("journal")).unwrap().count(),
            0
        );
    }
}

#[test]
fn empty_record_and_deleted_input_are_idempotently_reclaimed() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    let mut journal = Journal::create(&directory, &name(), &work.canonicalize().unwrap()).unwrap();
    journal.record(&work.canonicalize().unwrap()).unwrap();
    drop(journal);
    fs::remove_dir(&work).unwrap();
    drop(Journal::create(&directory, &name(), &work).unwrap());
    assert!(recover(&directory).is_empty());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    assert!(recover(&directory).is_empty());
}

#[test]
fn failed_recovery_retains_record_and_retries_after_reparse_point_is_removed() {
    let root = tempfile::tempdir().unwrap();
    setup(root.path());
    let (mut owner, ready) = owner(root.path(), "partial");
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    let original = root.path().join("game");
    let moved = root.path().join("moved");
    fs::rename(&original, &moved).unwrap();
    let before = dacl(&moved);
    let result = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/j"])
        .arg(&original)
        .arg(&moved)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(recover(&root.path().join("journal")).len(), 1);
    assert_eq!(dacl(&moved), before, "恢复不能跟随新目录联接");
    assert_eq!(
        fs::read_dir(root.path().join("journal")).unwrap().count(),
        1
    );
    fs::remove_dir(&original).unwrap();
    fs::rename(&moved, &original).unwrap();
    assert!(recover(&root.path().join("journal")).is_empty());
    assert_ne!(dacl(&original), before);
    assert_eq!(
        fs::read_dir(root.path().join("journal")).unwrap().count(),
        0
    );
    drop(Sid::package(ready["name"].as_str().unwrap()).unwrap());
}

#[test]
fn corrupt_complete_records_are_retained_and_unrelated_files_are_ignored() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::create(root.path(), &name(), &root.path().join("work")).unwrap();
    let mut file = journal.file;
    file.write_all(b"not-json\n").unwrap();
    drop(file);
    fs::write(root.path().join("unrelated.jsonl"), b"untouched").unwrap();
    assert_eq!(recover(root.path()).len(), 1);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    assert_eq!(
        fs::read(root.path().join("unrelated.jsonl")).unwrap(),
        b"untouched"
    );
    assert!(Journal::create(root.path(), &name(), &root.path().canonicalize().unwrap()).is_err());
}
