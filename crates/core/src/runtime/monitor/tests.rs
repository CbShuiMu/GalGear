use super::*;
use crate::{catalog, play_sessions, playtime};

fn identity(pid: u32, created: u64) -> Identity {
    Identity { pid, created }
}
fn process(pid: u32, created: u64, parent: u32, path: &str, visible: bool) -> (u32, Process) {
    (
        pid,
        Process {
            identity: Some(identity(pid, created)),
            parent,
            name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            image: Some(path.into()),
            visible,
            foreground: visible,
        },
    )
}
fn snapshot(now: Instant, processes: Vec<(u32, Process)>) -> Snapshot {
    Snapshot {
        captured: now,
        processes: processes.into_iter().collect(),
    }
}
fn launcher() -> (u32, Process) {
    let (id, mut process) = process(10, 100, 1, "C:/games/launcher.exe", true);
    process.foreground = false;
    (id, process)
}
fn game(visible: bool) -> (u32, Process) {
    process(20, 110, 10, "C:/temp/translated.tmp", visible)
}

#[test]
fn persistent_settings_process_is_replaced_and_never_selected_again() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    // 启动器退出，先出现的设置窗被接续。它随后隐藏但进程一直存活。
    let settings = || process(20, 110, 10, "C:/games/settings.exe", true);
    session.observe(path, &snapshot(now, vec![settings()]));
    assert_eq!(session.target.pid, 20);
    let mut game_entry = catalog::manual_game(path, "Game".into()).unwrap();
    let mut records = playtime::Records::new();
    let mut timing = playtime::Tracker::default();
    let mut finishes = play_sessions::Tracker::default();
    let statuses = BTreeMap::from([(game_entry.id.clone(), Status::Running)]);
    timing.observe(
        std::slice::from_mut(&mut game_entry),
        &statuses,
        &mut records,
        now,
        1000,
    );
    finishes.observe(&statuses);
    for second in 1..=15 {
        let settings = process(20, 110, 10, "C:/games/settings.exe", false);
        let actual = process(30, 120, 20, "C:/temp/random.tmp", true);
        let instant = now + Duration::from_secs(second);
        let status = session.observe(path, &snapshot(instant, vec![settings, actual]));
        assert_eq!(status, Status::Running);
        let statuses = BTreeMap::from([(game_entry.id.clone(), status)]);
        timing.observe(
            std::slice::from_mut(&mut game_entry),
            &statuses,
            &mut records,
            instant,
            1000 + second,
        );
        assert!(finishes.observe(&statuses).is_empty());
        if second <= 6 {
            assert_eq!(session.target.pid, 20, "短暂领先不能立即换绑");
        }
    }
    assert_eq!(session.target.pid, 30);
    assert_eq!(
        game_entry.playtime_seconds, 15,
        "换绑不丢失或重复累计游戏时间"
    );
    // 游戏最小化、用户切回旧设置窗，监控仍属于较新的游戏。
    for second in 16..=30 {
        let actual = process(30, 120, 20, "C:/temp/random.tmp", false);
        session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![settings(), actual]),
        );
        assert_eq!(session.target.pid, 30);
    }
    for second in 31..=34 {
        let status = session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![settings()]),
        );
        assert_eq!(
            status,
            if second < 34 {
                Status::Unknown
            } else {
                Status::Stopped
            }
        );
    }
}

#[test]
fn short_splash_and_intermittent_focus_do_not_replace_live_game() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in 0..=30 {
        let foreground = second <= 2 || second % 2 == 0;
        let mut candidate = game(true);
        candidate.1.foreground = foreground;
        let mut current = launcher();
        current.1.foreground = !foreground;
        session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![current, candidate]),
        );
        assert_eq!(session.target.pid, 10);
    }
}

#[test]
fn ambiguous_successors_stay_unknown_until_one_has_a_stable_lead() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in 0..=10 {
        let mut a = game(true);
        let mut b = process(30, 120, 10, "C:/temp/another.tmp", true);
        a.1.foreground = false;
        b.1.foreground = false;
        assert_eq!(
            session.observe(
                path,
                &snapshot(now + Duration::from_secs(second), vec![a, b])
            ),
            Status::Unknown
        );
        assert_eq!(session.target.pid, 10);
    }
    for second in 11..=15 {
        let mut b = process(30, 120, 10, "C:/temp/another.tmp", true);
        b.1.foreground = false;
        session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![game(true), b]),
        );
    }
    assert_eq!(session.target.pid, 20);
}

#[test]
fn sole_loading_process_is_confirmed_after_ten_observed_seconds() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in 0..=10 {
        let status = session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![game(false)]),
        );
        assert_eq!(
            status,
            if second < 10 {
                Status::Unknown
            } else {
                Status::Running
            }
        );
    }
    assert_eq!(session.target.pid, 20);
}

#[test]
fn stalled_sampling_cannot_complete_a_pending_handoff() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in 0..=5 {
        session.observe(
            path,
            &snapshot(
                now + Duration::from_secs(second),
                vec![launcher(), game(true)],
            ),
        );
    }
    assert!(session.challenge.is_some());
    session.observe(
        path,
        &snapshot(now + Duration::from_secs(100), vec![launcher(), game(true)]),
    );
    assert_eq!(session.target.pid, 10);
    assert!(session.challenge.is_none());
}

#[test]
fn confidence_uses_elapsed_time_at_different_polling_rates() {
    let now = Instant::now();
    let mut fast = Confidence::new(now);
    let mut slow = Confidence::new(now);
    for quarter in 1..=16 {
        fast.observe(1.0, now + Duration::from_millis(quarter * 250));
    }
    for second in 1..=4 {
        slow.observe(1.0, now + Duration::from_secs(second));
    }
    assert!((fast.score - slow.score).abs() < 1e-10);
    assert!(slow.score > 0.85 && slow.score < 0.87);
}

#[test]
fn configured_game_can_replace_a_visible_platform_without_parent_relation() {
    let now = Instant::now();
    let path = Path::new("C:/games/game.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in 0..=12 {
        let mut platform = process(10, 100, 1, "C:/Steam/steam.exe", true);
        platform.1.foreground = false;
        let actual = process(30, 120, 999, "C:/games/game.exe", true);
        session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![platform, actual]),
        );
    }
    assert_eq!(session.target.pid, 30);
}

#[test]
fn launcher_exit_tracks_renamed_child_and_minimized_game() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    assert_eq!(
        session.observe(path, &snapshot(now, vec![game(true)])),
        Status::Running
    );
    assert_eq!(session.target, identity(20, 110));
    assert_eq!(
        session.observe(
            path,
            &snapshot(now + Duration::from_secs(20), vec![game(false)])
        ),
        Status::Running
    );
}

#[test]
fn slow_child_initialization_does_not_end_the_session_before_its_window_appears() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in [0, 3, 10] {
        assert_eq!(
            session.observe(
                path,
                &snapshot(now + Duration::from_secs(second), vec![game(false)])
            ),
            Status::Unknown
        );
    }
    assert_eq!(
        session.observe(
            path,
            &snapshot(now + Duration::from_secs(15), vec![game(true)])
        ),
        Status::Running
    );
}

#[test]
fn stale_snapshot_cannot_restart_a_finished_session() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut sessions = BTreeMap::from([(path.to_path_buf(), Session::new(identity(10, 100), now))]);
    observe(&mut sessions, path, &snapshot(now, vec![]));
    assert_eq!(
        observe(&mut sessions, path, &snapshot(now + HANDOFF, vec![])),
        Status::Stopped
    );
    let earlier = snapshot(
        now + Duration::from_secs(1),
        vec![process(50, 500, 1, "C:/games/launcher.exe", true)],
    );
    assert_eq!(observe(&mut sessions, path, &earlier), Status::Stopped);
}

#[test]
fn inaccessible_new_process_after_session_end_is_not_reported_stopped() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut sessions = BTreeMap::from([(path.to_path_buf(), Session::new(identity(10, 100), now))]);
    observe(&mut sessions, path, &snapshot(now, vec![]));
    observe(&mut sessions, path, &snapshot(now + HANDOFF, vec![]));
    let (_, mut inaccessible) = launcher();
    inaccessible.image = None;
    inaccessible.identity = None;
    assert_eq!(
        observe(
            &mut sessions,
            path,
            &snapshot(now + HANDOFF, vec![(50, inaccessible)])
        ),
        Status::Unknown
    );
}

#[test]
fn newly_created_same_directory_window_bridges_missing_intermediate_launcher() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    for second in 0..=4 {
        let child = process(20, 110, 99, "C:/games/CHS.tmp", true);
        let status = session.observe(
            path,
            &snapshot(now + Duration::from_secs(second), vec![child]),
        );
        assert_eq!(
            status,
            if second < 3 {
                Status::Unknown
            } else {
                Status::Running
            }
        );
    }
    assert_eq!(session.target.pid, 20);
}

#[test]
fn unrelated_and_preexisting_windows_are_not_adopted() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    let processes = || {
        vec![
            process(20, 90, 1, "C:/games/tool.exe", true),
            process(30, 110, 1, "C:/other/game.exe", true),
        ]
    };
    assert_eq!(
        session.observe(path, &snapshot(now, processes())),
        Status::Unknown
    );
    assert_eq!(
        session.observe(path, &snapshot(now + HANDOFF, processes())),
        Status::Stopped
    );
}

#[test]
fn parent_pid_reuse_does_not_claim_an_unrelated_child() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    let processes = || {
        vec![
            process(10, 200, 1, "C:/other/browser.exe", true),
            process(20, 210, 10, "C:/other/helper.exe", true),
        ]
    };
    assert_eq!(
        session.observe(path, &snapshot(now, processes())),
        Status::Unknown
    );
    assert_eq!(
        session.observe(path, &snapshot(now + HANDOFF, processes())),
        Status::Stopped
    );
}

#[test]
fn handoff_grace_finds_successor_through_multiple_parents() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    assert_eq!(
        session.observe(path, &snapshot(now, vec![])),
        Status::Unknown
    );
    let processes = vec![
        process(20, 110, 10, "C:/temp/wrapper.tmp", false),
        process(30, 120, 20, "C:/temp/game.tmp", true),
    ];
    assert_eq!(
        session.observe(path, &snapshot(now + Duration::from_secs(2), processes)),
        Status::Running
    );
    assert_eq!(session.target.pid, 30);
}

#[test]
fn lingering_launcher_does_not_extend_playtime_or_reopen_the_session() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut sessions = BTreeMap::new();
    for second in 0..=10 {
        assert_eq!(
            observe(
                &mut sessions,
                path,
                &snapshot(
                    now + Duration::from_secs(second),
                    vec![launcher(), game(true)]
                )
            ),
            Status::Running
        );
    }
    assert_eq!(sessions[path].target.pid, 20);
    assert_eq!(
        observe(
            &mut sessions,
            path,
            &snapshot(now + Duration::from_secs(11), vec![launcher()])
        ),
        Status::Unknown
    );
    assert_eq!(
        observe(
            &mut sessions,
            path,
            &snapshot(now + Duration::from_secs(14), vec![launcher()])
        ),
        Status::Stopped
    );
    assert_eq!(
        observe(
            &mut sessions,
            path,
            &snapshot(now + Duration::from_secs(30), vec![launcher()])
        ),
        Status::Stopped
    );
    let restarted = process(50, 500, 1, "C:/games/launcher.exe", true);
    assert_eq!(
        observe(
            &mut sessions,
            path,
            &snapshot(now + Duration::from_secs(40), vec![launcher(), restarted])
        ),
        Status::Running
    );
    assert_eq!(sessions[path].target.pid, 50);
}

#[test]
fn inaccessible_target_is_unknown_and_stale_poll_cannot_rewind_launch() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    assert_eq!(
        session.observe(path, &snapshot(now - Duration::from_secs(1), vec![])),
        Status::Running
    );
    session.observe(path, &snapshot(now, vec![game(true)]));
    let (pid, mut inaccessible) = game(false);
    inaccessible.identity = None;
    inaccessible.image = None;
    assert_eq!(
        session.observe(path, &snapshot(now + HANDOFF, vec![(pid, inaccessible)])),
        Status::Unknown
    );
}

#[test]
fn translated_game_counts_time_and_only_finishes_after_actual_exit() {
    let now = Instant::now();
    let path = Path::new("C:/games/launcher.exe");
    let mut session = Session::new(identity(10, 100), now);
    let mut game_entry = catalog::manual_game(path, "Game".into()).unwrap();
    let mut records = playtime::Records::new();
    let mut timing = playtime::Tracker::default();
    let mut finishes = play_sessions::Tracker::default();
    for second in 0..=10 {
        let time = now + Duration::from_secs(second);
        let status = session.observe(path, &snapshot(time, vec![game(second == 0)]));
        let statuses = BTreeMap::from([(game_entry.id.clone(), status)]);
        timing.observe(
            std::slice::from_mut(&mut game_entry),
            &statuses,
            &mut records,
            time,
            1000 + second,
        );
        assert!(finishes.observe(&statuses).is_empty());
    }
    assert_eq!(game_entry.playtime_seconds, 10);
    for second in 11..=14 {
        let status = session.observe(path, &snapshot(now + Duration::from_secs(second), vec![]));
        let ended = finishes.observe(&BTreeMap::from([(game_entry.id.clone(), status)]));
        assert_eq!(ended.len(), usize::from(second == 14));
    }
}

#[test]
fn windows_snapshot_tracks_real_launcher_exiting_before_renamed_pe() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let temporary = tempfile::tempdir().unwrap();
    let launcher_path = temporary.path().join("launcher.exe");
    let translated = temporary.path().join("translated.tmp");
    let current = std::env::current_exe().unwrap();
    std::fs::copy(&current, &launcher_path).unwrap();
    std::fs::copy(&current, &translated).unwrap();
    struct Stop(std::path::PathBuf);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = std::fs::write(self.0.join("stop"), b"");
        }
    }
    let stop = Stop(temporary.path().to_path_buf());
    let mut child = Command::new(&launcher_path)
        .args([
            "--ignored",
            "--exact",
            "runtime::monitor::tests::native_translated_probe",
        ])
        .env("GALGEAR_TRANSLATED_PROBE", "launcher")
        .env("GALGEAR_PROBE_DIRECTORY", temporary.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .spawn()
        .unwrap();
    let monitor = Monitor::default();
    monitor.launched(&launcher_path, &child).unwrap();
    child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !temporary.path().join("ready").exists() {
        assert!(
            Instant::now() < deadline,
            "translated process did not create its window"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        super::super::detect(launcher_path.to_str()),
        Status::Stopped
    );
    assert_eq!(monitor.detect(launcher_path.to_str()), Status::Running);
    let canonical = std::fs::canonicalize(&launcher_path).unwrap();
    assert_ne!(
        monitor.sessions.lock().unwrap()[&canonical].target.pid,
        child.id()
    );
    drop(stop);
    while !temporary.path().join("exited").exists() {
        assert!(Instant::now() < deadline, "translated process did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
    loop {
        if monitor.detect(launcher_path.to_str()) == Status::Stopped {
            break;
        }
        assert!(Instant::now() < deadline, "session did not finish");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "仅由隔离的 Windows 进程跟踪测试启动"]
#[expect(
    clippy::zombie_processes,
    reason = "此测试必须让 Windows 启动器先退出；子进程通过 stop 文件和 15 秒超时结束"
)]
fn native_translated_probe() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WS_EX_NOACTIVATE, WS_POPUP, WS_VISIBLE,
    };
    let Ok(role) = std::env::var("GALGEAR_TRANSLATED_PROBE") else {
        return;
    };
    let directory = PathBuf::from(std::env::var_os("GALGEAR_PROBE_DIRECTORY").unwrap());
    if role == "launcher" {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(directory.join("translated.tmp"))
            .args([
                "--ignored",
                "--exact",
                "runtime::monitor::tests::native_translated_probe",
            ])
            .env("GALGEAR_TRANSLATED_PROBE", "game")
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        return;
    }
    // SAFETY: 使用系统 STATIC 类创建测试自有窗口，位于屏幕外且不激活；本线程负责销毁。
    let window = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE,
            windows_sys::w!("STATIC"),
            windows_sys::w!(""),
            WS_POPUP | WS_VISIBLE,
            -32000,
            -32000,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert!(!window.is_null());
    std::fs::write(directory.join("ready"), b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !directory.join("stop").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    // SAFETY: 窗口由当前线程创建且尚未销毁。
    unsafe {
        DestroyWindow(window);
    }
    std::fs::write(directory.join("exited"), b"").unwrap();
}
