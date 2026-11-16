//! Linux 的 Proton 启动边界。umu 负责 Steam Runtime 与 Proton 的版本匹配。
use crate::GameEdit;
use galgear_i18n::Message;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

/// 为一款游戏构建 umu 命令。独立 prefix 与日志保存在应用数据目录，参数不经过 shell。
pub fn command(
    executable: &Path,
    edit: &GameEdit,
    data: &Path,
    id: &str,
) -> Result<Command, Message> {
    let executable =
        fs::canonicalize(executable).map_err(|e| Message::detail("error.launch", e))?;
    let root = data.join("compatdata");
    fs::create_dir_all(&root).map_err(|e| Message::detail("error.launch", e))?;
    let root = fs::canonicalize(root).map_err(|e| Message::detail("error.launch", e))?;
    let directory = root.join(format!("{:x}", Sha256::digest(id.as_bytes())));
    if directory
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(Message::new("proton.unsafe_prefix"));
    }
    fs::create_dir_all(&directory).map_err(|e| Message::detail("error.launch", e))?;
    let prefix = directory.join("pfx");
    if prefix
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(Message::new("proton.unsafe_prefix"));
    }
    let runner = edit
        .launcher
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or("umu-run");
    let mut command = Command::new(runner);
    if let Some(proton) = edit.proton_path.as_deref().filter(|p| !p.trim().is_empty()) {
        let proton = Path::new(proton);
        if !proton.is_absolute() || !proton.join("proton").is_file() {
            return Err(Message::new("proton.invalid_path"));
        }
        command.env("PROTONPATH", proton);
    } else {
        command.env_remove("PROTONPATH");
    }
    let log_path = directory.join("launch.log");
    if log_path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(Message::new("proton.unsafe_prefix"));
    }
    let log = fs::File::create(log_path).map_err(|e| Message::detail("error.launch", e))?;
    let errors = log
        .try_clone()
        .map_err(|e| Message::detail("error.launch", e))?;
    command
        .arg(&executable)
        .args(&edit.arguments)
        .current_dir(
            edit.working_directory
                .as_deref()
                .map(Path::new)
                .or_else(|| executable.parent())
                .ok_or_else(|| Message::new("error.game_folder"))?,
        )
        .env("WINEPREFIX", prefix)
        .env("GAMEID", "umu-default")
        .env("STORE", "none")
        .env("GALGEAR_GAME_EXECUTABLE", &executable)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(errors);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::command;
    use crate::GameEdit;
    use std::fs;

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "需要 umu-run、网络与 GALGEAR_TEST_PE 指向只写 proton-ok.txt 的隔离 Windows 夹具"]
    fn real_proton_runs_windows_fixture() {
        let source = std::env::var_os("GALGEAR_TEST_PE").expect("set GALGEAR_TEST_PE");
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("日本語 game.exe");
        fs::copy(source, &exe).unwrap();
        let edit = GameEdit {
            arguments: vec!["two words".into(), "日本語".into()],
            proton_path: std::env::var("GALGEAR_TEST_PROTON").ok(),
            ..Default::default()
        };
        let mut child = command(&exe, &edit, temp.path(), "fixture")
            .unwrap()
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Proton timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        if !status.success() {
            let directory = fs::read_dir(temp.path().join("compatdata"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            panic!(
                "Proton failed: {}",
                fs::read_to_string(directory.join("launch.log")).unwrap()
            );
        }
        assert_eq!(
            fs::read_to_string(temp.path().join("proton-ok.txt")).unwrap(),
            "two words\n日本語"
        );
    }

    #[test]
    fn preserves_arguments_and_separates_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("日本語 game.exe");
        fs::write(&exe, b"fixture").unwrap();
        let edit = GameEdit {
            arguments: vec!["two words".into(), "$(touch never)".into()],
            ..Default::default()
        };
        let a = command(&exe, &edit, temp.path(), "../a").unwrap();
        let b = command(&exe, &edit, temp.path(), "b").unwrap();
        assert_eq!(
            a.get_args().skip(1).collect::<Vec<_>>(),
            edit.arguments
                .iter()
                .map(std::ffi::OsStr::new)
                .collect::<Vec<_>>()
        );
        let prefix = |c: &std::process::Command| {
            c.get_envs()
                .find(|(k, _)| *k == "WINEPREFIX")
                .unwrap()
                .1
                .unwrap()
                .to_owned()
        };
        assert_ne!(prefix(&a), prefix(&b));
        assert!(
            std::path::Path::new(&prefix(&a))
                .starts_with(fs::canonicalize(temp.path().join("compatdata")).unwrap())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launches_runner_with_real_environment_and_exit_status() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let runner = temp.path().join("umu fixture");
        fs::write(&runner, "#!/bin/sh\nprintf '%s\\n' \"$WINEPREFIX\" \"$GALGEAR_GAME_EXECUTABLE\" \"$1\" \"$2\" > result\nexit 7\n").unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
        let exe = temp.path().join("ゲーム.exe");
        fs::write(&exe, b"fixture").unwrap();
        let edit = GameEdit {
            launcher: Some(runner.to_string_lossy().into()),
            arguments: vec!["space ; argument".into()],
            ..Default::default()
        };
        assert_eq!(
            command(&exe, &edit, temp.path(), "game")
                .unwrap()
                .status()
                .unwrap()
                .code(),
            Some(7)
        );
        let result = fs::read_to_string(temp.path().join("result")).unwrap();
        assert!(result.contains("space ; argument"));
        assert_eq!(result.lines().nth(1), exe.to_str());
        assert_eq!(result.lines().nth(2), exe.to_str());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_invalid_proton_and_redirected_prefix_or_log() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("game.exe");
        fs::write(&exe, b"fixture").unwrap();
        let invalid = GameEdit {
            proton_path: Some("relative/Proton".into()),
            ..Default::default()
        };
        assert!(command(&exe, &invalid, temp.path(), "game").is_err());
        let edit = GameEdit::default();
        let prepared = command(&exe, &edit, temp.path(), "game").unwrap();
        let prefix = prepared
            .get_envs()
            .find(|(key, _)| *key == "WINEPREFIX")
            .unwrap()
            .1
            .unwrap();
        let prefix = std::path::Path::new(prefix);
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        symlink(&external, prefix).unwrap();
        assert!(command(&exe, &edit, temp.path(), "game").is_err());
        fs::remove_file(prefix).unwrap();
        let log = prefix.parent().unwrap().join("launch.log");
        fs::remove_file(&log).unwrap();
        let protected = external.join("keep.txt");
        fs::write(&protected, b"keep").unwrap();
        symlink(&protected, &log).unwrap();
        assert!(command(&exe, &edit, temp.path(), "game").is_err());
        assert_eq!(fs::read(&protected).unwrap(), b"keep");
    }
}
