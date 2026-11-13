//! Update handoff: terminate application tasks, then replace files in a separate process.
use crate::state::{AppState, snapshot};
use galgear_core::updates::{self, Release};
use galgear_i18n::{Language, Message};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::{Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
use tauri::{Manager, State};

#[derive(Default)]
pub(crate) struct Updates { pub active: AtomicBool, checked: Mutex<Option<Release>> }

#[tauri::command]
pub(crate) async fn check_update(state: State<'_, AppState>) -> Result<Release, Message> {
    let page = snapshot(&state)?.release_page;
    let release = tauri::async_runtime::spawn_blocking(move || updates::check(&page, env!("CARGO_PKG_VERSION")))
        .await.map_err(|e| Message::detail("update.failed", e))??;
    *state.updates.checked.lock().map_err(|_| Message::new("error.operation"))? = Some(release.clone());
    Ok(release)
}

#[derive(Serialize, Deserialize)]
struct Handoff { parent: u32, executable: PathBuf, release: Release, language: Language }

#[tauri::command]
pub(crate) async fn install_update(app: tauri::AppHandle, version: String, state: State<'_, AppState>) -> Result<(), Message> {
    let release = state.updates.checked.lock().map_err(|_| Message::new("error.operation"))?
        .as_ref().filter(|release| release.version == version && release.available && release.asset.is_some()).cloned()
        .ok_or_else(|| Message::new("update.unavailable"))?;
    if state.updates.active.swap(true, Ordering::SeqCst) { return Err(Message::new("update.busy")); }
    let result = async {
        crate::library::cancel_scan(state.clone());
        crate::downloads::cancel_fan_download(state.clone());
        // New write transactions are blocked; let the current atomic operation reach its boundary.
        for _ in 0..600 {
            if !state.is_writing() { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if state.is_writing() { return Err(Message::new("update.write_busy")); }
        let executable = std::env::current_exe().map_err(|e| Message::detail("update.failed", e))?;
        let parent = executable.parent().ok_or_else(|| Message::new("error.operation"))?;
        let work = tempfile::Builder::new().prefix(".galgear-update-").tempdir_in(parent)
            .map_err(|e| Message::detail("update.failed", e))?;
        let helper = work.path().join("updater.exe");
        fs::copy(&executable, &helper).map_err(|e| Message::detail("update.failed", e))?;
        let handoff = Handoff { parent: std::process::id(), executable, release, language: snapshot(&state)?.language };
        let manifest = work.path().join("handoff.json");
        fs::write(&manifest, serde_json::to_vec(&handoff).map_err(|e| Message::detail("update.failed", e))?)
            .map_err(|e| Message::detail("update.failed", e))?;
        let mut command = std::process::Command::new(&helper);
        command.arg("--galgear-update").arg(&manifest);
        #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
        command.spawn().map_err(|e| Message::detail("update.failed", e))?;
        let _ = work.keep();
        app.exit(0);
        Ok(())
    }.await;
    if result.is_err() { state.updates.active.store(false, Ordering::SeqCst); }
    result
}

fn wait_parent(parent: u32) -> Result<(), Message> {
    #[cfg(windows)] unsafe {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE}};
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, parent);
        if !handle.is_null() {
            let result = WaitForSingleObject(handle, 30000);
            CloseHandle(handle);
            if result != 0 { return Err(Message::new("update.write_busy")); }
        }
    }
    Ok(())
}

fn run_job(manifest: &std::path::Path, mut status: impl FnMut(String)) -> Result<(), Message> {
    let handoff: Handoff = serde_json::from_slice(&fs::read(manifest).map_err(|e| Message::detail("update.failed", e))?)
        .map_err(|e| Message::detail("update.failed", e))?;
    let work = manifest.parent().ok_or_else(|| Message::new("error.operation"))?;
    wait_parent(handoff.parent)?;
    let asset = handoff.release.asset.ok_or_else(|| Message::new("update.unavailable"))?;
    let archive = work.join("download");
    let mut percent = u64::MAX;
    updates::download(&asset, &archive, |received, total| {
        let next = received * 100 / total.max(1);
        if next != percent {
            percent = next;
            status(format!("{} {next}%", Message::new("update.downloading").render(handoff.language)));
        }
    })?;
    status(Message::new("update.replacing").render(handoff.language));
    let payload = updates::unpack(&archive, &work.join("payload"), asset.name.ends_with(".zip"))?;
    let transaction = updates::replace(&payload, &handoff.executable, &work.join("backup"))?;
    match std::process::Command::new(&handoff.executable).arg("--galgear-update-complete").spawn() {
        Ok(_) => Ok(()),
        Err(error) => { transaction.rollback()?; Err(Message::detail("update.failed", error)) }
    }
}

pub(crate) fn run_helper() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--galgear-update")) {
        return false;
    }
    let Some(manifest) = args.next().map(PathBuf::from) else {
        return true;
    };
    crate::diagnostics::initialize();
    log::info!(
        "updater_start webview_version={:?}",
        tauri::webview_version()
    );
    let recovery_manifest = manifest.clone();
    // 独立 profile 避免更新窗口与待退出的主程序共享故障浏览器进程。
    let profile = manifest.with_extension("webview");
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.security.csp = None;
    let result = helper_builder(move |app| {
        complete_job(&manifest, |text| {
            log::info!("updater_progress {text}");
            if let Some(window) = app.get_webview_window("updater") {
                // 标题属于原生窗口，即使网页尚未加载或已经崩溃也能显示进度。
                let _ = window.set_title(&format!("GalGear — {text}"));
                if let Ok(text) = serde_json::to_string(&text) {
                    let _ = window.eval(format!("{{const status=document.getElementById('status');if(status)status.textContent={text};}}"));
                }
            }
        });
        app.exit(0);
    }, profile).run(context);
    if let Err(error) = result {
        log::error!("updater_window_failed {error}");
        let _ = rfd::MessageDialog::new()
            .set_title("GalGear 更新失败")
            .set_description(error.to_string())
            .show();
        restart_original(&recovery_manifest);
    }
    crate::diagnostics::finish();
    true
}

// setup 只派发一次后台任务。页面加载、重载和进程故障都不参与更新任务的启动。
fn helper_builder(
    job: impl FnOnce(tauri::AppHandle) + Send + 'static,
    profile: PathBuf,
) -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default().setup(move |app| {
        let builder = tauri::WebviewWindowBuilder::new(
            app,
            "updater",
            tauri::WebviewUrl::App("updater.html".into()),
        )
        .title("GalGear 更新")
        .inner_size(440.0, 190.0)
        .resizable(false)
        .closable(false)
        .center();
        let window = builder.data_directory(profile).build()?;
        #[cfg(windows)]
        crate::diagnostics::observe_webview(&window, || {});
        let handle = app.handle().clone();
        std::thread::Builder::new()
            .name("updater-job".into())
            .spawn(move || job(handle))?;
        Ok(())
    })
}

fn complete_job(manifest: &std::path::Path, status: impl FnMut(String)) {
    log::info!("updater_job_start");
    if let Err(error) = run_job(manifest, status) {
        log::error!("updater_job_failed {error:?}");
        let _ = rfd::MessageDialog::new()
            .set_title("GalGear 更新失败")
            .set_description(error.render(Language::Chinese))
            .set_level(rfd::MessageLevel::Error)
            .show();
        restart_original(manifest);
    } else {
        log::info!("updater_job_complete");
    }
}

fn restart_original(manifest: &std::path::Path) {
    if let Ok(bytes) = fs::read(manifest)
        && let Ok(handoff) = serde_json::from_slice::<Handoff>(&bytes)
        && let Err(error) = std::process::Command::new(handoff.executable).spawn()
    {
        log::error!("updater_restart_failed {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    #[ignore = "需要 Windows 桌面和 WebView2，使用临时配置且不下载或替换文件"]
    fn native_updater_starts_job_without_page_load() {
        use std::sync::Arc;
        let root = tempfile::tempdir().unwrap();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        // 新窗口使用隔离 profile；不读取真实 WebView 用户数据。
        let profile = root.path().join("webview");
        let completed = Arc::new(AtomicBool::new(false));
        let loaded = Arc::new(AtomicBool::new(false));
        let app = helper_builder(
            {
                let completed = completed.clone();
                let loaded = loaded.clone();
                move |app| {
                    std::thread::sleep(Duration::from_millis(500));
                    completed.store(!loaded.load(Ordering::SeqCst), Ordering::SeqCst);
                    app.exit(0);
                }
            },
            profile,
        )
        .any_thread()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("blocked-page")
                .js_init_script("while (true) {}")
                .build(),
        )
        .on_page_load(move |_, event| {
            if event.event() == tauri::webview::PageLoadEvent::Finished {
                loaded.store(true, Ordering::SeqCst);
            }
        })
        .build(context)
        .unwrap();
        assert_eq!(app.run_return(|_, _| {}), 0);
        assert!(
            completed.load(Ordering::SeqCst),
            "更新任务必须在页面无法完成加载时运行"
        );
    }
    #[test]
    fn update_priority_blocks_new_writes_without_interrupting_the_current_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState::load(temp.path().to_path_buf()).unwrap();
        let guard = crate::state::begin_write(&state).unwrap();
        state.updates.active.store(true, Ordering::SeqCst);
        assert!(state.is_writing());
        assert!(crate::state::begin_write(&state).is_err());
        drop(guard);
        assert!(!state.is_writing());
        assert!(crate::state::begin_write(&state).is_err());
        state.updates.active.store(false, Ordering::SeqCst);
        assert!(crate::state::begin_write(&state).is_ok());
    }
}
