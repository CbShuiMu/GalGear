use super::{Child, Sandbox};
use serde_json::json;
use std::{fs, io::Write, path::Path, process::Command};

#[test]
#[ignore = "native child fixture, launched by sandbox tests"]
fn access_fixture() {
    let Ok(root) = std::env::var("GALGEAR_SANDBOX_FIXTURE") else {
        return;
    };
    let root = Path::new(&root);
    assert_eq!(fs::read(root.join("game/input")).unwrap(), b"game");
    assert_eq!(fs::read(root.join("saves/selected")).unwrap(), b"save");
    assert_eq!(fs::read(root.join("skill/input")).unwrap(), b"skill");
    for path in [
        "unrelated/private",
        "saves/other-game",
        "game/work/link/private",
    ] {
        assert!(
            fs::read(root.join(path)).is_err(),
            "unexpected read: {path}"
        );
    }
    let profile = std::env::var_os("GALGEAR_SANDBOX_PROFILE").unwrap();
    for relative in [
        "forbidden",
        "AC/Temp/forbidden",
        "LocalCache/forbidden",
        "LocalState/forbidden",
        "RoamingState/forbidden",
        "TempState/forbidden",
        "Settings/forbidden",
    ] {
        assert!(
            fs::write(Path::new(&profile).join(relative), b"private").is_err(),
            "profile write escaped work: {relative}"
        );
    }
    for path in [
        "game/input",
        "game/new",
        "saves/selected",
        "skill/input",
        "unrelated/new",
        "game/work/link/new",
    ] {
        assert!(
            fs::write(root.join(path), b"changed").is_err(),
            "unexpected write: {path}"
        );
    }
    use std::os::windows::fs::OpenOptionsExt;
    for access in [0x10000, 0x40000, 0x80000] {
        // DELETE、WRITE_DAC、WRITE_OWNER
        assert!(
            fs::OpenOptions::new()
                .access_mode(access)
                .open(root.join("game/input"))
                .is_err()
        );
    }
    let output = root.join("game/work/output");
    fs::write(&output, b"result").unwrap();
    fs::rename(&output, root.join("game/work/renamed")).unwrap();
    fs::remove_file(root.join("game/work/renamed")).unwrap();
    let shell = Path::new(&std::env::var_os("SYSTEMROOT").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let result = Command::new(shell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::Write(6 * 7)",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "PowerShell: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42");
    if std::env::var_os("GALGEAR_SANDBOX_NETWORK_PROBE").is_some()
        && std::env::var_os("GALGEAR_SANDBOX_DESCENDANT").is_none()
    {
        use std::io::{BufRead, BufReader};
        use std::os::windows::io::FromRawHandle;
        let handle: usize = std::env::var("GALGEAR_AI_HTTP_HANDLE")
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: the fixture owns the client handle explicitly inherited from its parent.
        let mut pipe = unsafe { fs::File::from_raw_handle(handle as _) };
        writeln!(
            pipe,
            "{}",
            json!({"url":"https://api.openai.com/v1/models","method":"GET","timeout":15})
        )
        .unwrap();
        let mut response = String::new();
        BufReader::new(&mut pipe).read_line(&mut response).unwrap();
        pipe.write_all(b"ok\n").unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], 401, "{response}");
    }
    if std::env::var_os("GALGEAR_SANDBOX_DESCENDANT").is_none() {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ai_summary::windows_sandbox::tests::access_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("GALGEAR_SANDBOX_DESCENDANT", "1")
            .status()
            .unwrap();
        assert!(result.success(), "descendant lost sandbox access");
    }
}

#[test]
fn windows_enforces_per_task_access_and_descendant_isolation() {
    check_access(false);
}

#[test]
#[ignore = "requires Internet access; sends no credentials or game data"]
fn windows_sandbox_can_reach_model_service() {
    check_access(true);
}

#[test]
#[ignore = "requires development Python and Internet access; sends no credentials"]
fn python_transport_reuses_sandbox_channel_with_custom_openers() {
    let work = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ai_summary/sandbox_http.py");
    let mut command = Command::new("python");
    command.args(["-I", "-S", "-c", r#"
import runpy, sys, urllib.request, urllib.error
runpy.run_path(sys.argv[1])['install']()
for _ in range(2):
    try:
        urllib.request.build_opener(urllib.request.HTTPRedirectHandler()).open('https://api.openai.com/v1/models', timeout=15)
        raise AssertionError('Expected unauthenticated HTTP response')
    except urllib.error.HTTPError as error:
        assert error.code == 401, error.code
        assert error.read()
        error.close()
"#]).arg(source);
    let config = json!({"game":work.path(),"save":work.path(),"skill":work.path()});
    let mut output = tempfile::tempfile().unwrap();
    let mut errors = tempfile::tempfile().unwrap();
    let mut child = super::super::process::spawn(
        &mut command,
        work.path(),
        false,
        &config,
        (&output, &errors),
    )
    .unwrap();
    let status = child.wait().unwrap();
    child.finish("completed").unwrap();
    use std::io::{Read, Seek};
    let mut diagnostic = String::new();
    output.rewind().unwrap();
    output.read_to_string(&mut diagnostic).unwrap();
    errors.rewind().unwrap();
    errors.read_to_string(&mut diagnostic).unwrap();
    assert!(status.success(), "{diagnostic}");
}

fn check_access(network: bool) {
    let root = tempfile::tempdir().unwrap();
    for path in ["game/work", "saves", "skill", "unrelated"] {
        fs::create_dir_all(root.path().join(path)).unwrap();
    }
    for (path, data) in [
        ("game/input", "game"),
        ("saves/selected", "save"),
        ("saves/other-game", "private"),
        ("skill/input", "skill"),
        ("unrelated/private", "private"),
    ] {
        fs::write(root.path().join(path), data).unwrap();
    }
    let config = json!({ "game": root.path().join("game"), "skill": root.path().join("skill"), "save": root.path().join("saves"), "save_candidates": [{"path":root.path().join("saves/selected")}] });
    let work = root.path().join("game/work");
    let junction = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/j"])
        .arg(work.join("link").to_string_lossy().replace('/', "\\"))
        .arg(root.path().join("unrelated"))
        .output()
        .unwrap();
    assert!(
        junction.status.success(),
        "{}",
        String::from_utf8_lossy(&junction.stderr)
    );
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "ai_summary::windows_sandbox::tests::access_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("GALGEAR_SANDBOX_FIXTURE", root.path());
    if network {
        command.env("GALGEAR_SANDBOX_NETWORK_PROBE", "1");
    }
    let mut sandbox = Sandbox::new(&mut command, &work, &config, false).unwrap();
    command.env(
        "GALGEAR_SANDBOX_PROFILE",
        sandbox.sid.profile_directory().unwrap(),
    );
    let stdout = tempfile::tempfile().unwrap();
    let stderr = tempfile::tempfile().unwrap();
    let mut child =
        Child::spawn(&command, &work, Some(&sandbox), Some((&stdout, &stderr))).unwrap();
    child.resume().unwrap();
    child.stdin.take().unwrap().write_all(b"").unwrap();
    let status = child.wait().unwrap();
    use std::io::{Read, Seek};
    let mut text = String::new();
    let mut output = stdout;
    output.rewind().unwrap();
    output.read_to_string(&mut text).unwrap();
    let mut errors = stderr;
    errors.rewind().unwrap();
    errors.read_to_string(&mut text).unwrap();
    assert!(status.success(), "{status}: {text}");
    sandbox.revoke().unwrap();
    // 同一 SID 撤销后不能再次启动该测试程序，验证 ACL 已回收。
    if let Ok(mut revoked) = Child::spawn(&command, &work, Some(&sandbox), None) {
        revoked.resume().unwrap();
        assert!(!revoked.wait().unwrap().success());
    }
    assert_eq!(fs::read(root.path().join("game/input")).unwrap(), b"game");
    assert_eq!(
        fs::read(root.path().join("saves/selected")).unwrap(),
        b"save"
    );
}
