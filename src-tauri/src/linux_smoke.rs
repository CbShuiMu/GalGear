//! 在真实 WebKitGTK 中通过生产 IPC 验证 Linux 导入、资料与启动链。
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Listener, Manager};

#[test]
#[ignore = "需要 Linux 图形会话及已构建前端；GALGEAR_LINUX_LIVE=1 额外访问 VNDB"]
fn linux_native_import_scrape_launch() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let library = temp.path().join("日本語 Library");
    let folder = library.join("Kanon");
    fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("Game.exe");
    fs::write(&exe, b"isolated fixture").unwrap();
    let runner = temp.path().join("umu fixture");
    fs::write(
        &runner,
        "#!/bin/sh\nprintf '%s\\n' \"$1\" \"$WINEPREFIX\" > launched.txt\nsleep 3\n",
    )
    .unwrap();
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
    let settings = galgear_core::Settings {
        launch_behavior: "none".into(),
        initial_setup_pending: false,
        ..Default::default()
    };
    galgear_core::config::ConfigStore::new(data.join("settings.json"))
        .save(&settings)
        .unwrap();
    let live = std::env::var_os("GALGEAR_LINUX_LIVE").is_some();
    let script = format!(
        r#"
      window.addEventListener('DOMContentLoaded', async () => {{
        const invoke = window.__TAURI__.core.invoke;
        const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
        try {{
          if (sessionStorage.getItem('linux-smoke-complete')) {{
            for(let i=0;i<200 && !document.querySelector('.home-game');i++) await pause(50);
            if (!document.querySelector('.home-game')) throw Error('Imported game did not render after reload');
            if ({live}) {{
              const images = [...document.querySelectorAll('img')].filter(image => image.src.startsWith('galgear-cover:'));
              if (!images.length) throw Error('Rendered cover missing');
              await Promise.all(images.map(image => image.decode()));
            }}
            await pause(300);
            await window.__TAURI__.event.emit('linux-smoke-result', 'ok');
            return;
          }}
          await invoke('bootstrap');
          await invoke('add_library_folder', {{path:{library}}});
          const catalog = await invoke('scan_library');
          const game = catalog.games.find(g => g.executable === {exe});
          if (!game) throw Error('Imported game missing');
          let url = '';
          if ({live}) {{
            const rows = await invoke('search_metadata', {{query:'Kanon',source:'vndb'}});
            if (!rows.length) throw Error('Search returned no metadata');
            url = rows[0].url;
          }}
          await invoke('complete_game_setup', {{id:game.id,source:'vndb',url,executable:game.executable,confirmed:true}});
          await invoke('save_game_details', {{id:game.id,draft:{{title:game.title,tags:[],executable:game.executable,
            arguments:['two words'],working_directory:null,launcher:{runner},proton_path:null,locale_profile:'',metadata:{{}}}}}});
          // 重新加载真实 WASM 状态，再确认页面渲染以及本地封面协议。
          const loaded = await invoke('bootstrap');
          if ({live} && !loaded.catalog.games.find(g=>g.id===game.id)?.metadata?.title) throw Error('Metadata was not persisted');
          const cover = loaded.catalog.games.find(g=>g.id===game.id)?.cover;
          if ({live} && !cover) throw Error('Scraped cover missing');
          if (cover) {{
            const key = cover.split('/').pop();
            await new Promise((resolve,reject)=>{{ const image=new Image(); image.onload=resolve; image.onerror=()=>reject(Error('Cover protocol failed')); image.src=window.__TAURI__.core.convertFileSrc(key,'galgear-cover'); }});
          }}
          await invoke('launch_game', {{id:game.id}});
          await pause(500);
          const status = await invoke('game_runtime', {{id:game.id}});
          if (status !== 'running') throw Error('Runner not observed: '+status);
          sessionStorage.setItem('linux-smoke-complete', '1');
          location.reload();
        }} catch(error) {{ await window.__TAURI__.event.emit('linux-smoke-result', JSON.stringify(error, Object.getOwnPropertyNames(error)) || String(error)); }}
      }});
    "#,
        library = serde_json::to_string(&library).unwrap(),
        exe = serde_json::to_string(&exe).unwrap(),
        runner = serde_json::to_string(&runner).unwrap()
    );
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let outcome = Arc::new(Mutex::new(None));
    let result = outcome.clone();
    let profile = temp.path().join("profile");
    let app = super::builder()
        .any_thread()
        .setup(move |app| {
            crate::startup::start(app.handle(), data, temp.path().to_owned());
            let handle = app.handle().clone();
            app.listen("linux-smoke-result", move |event| {
                if let Some(path) = std::env::var_os("GALGEAR_LINUX_SCREENSHOT") {
                    let _ = std::process::Command::new("import")
                        .args(["-window", "root"])
                        .arg(path)
                        .status();
                }
                *result.lock().unwrap() = Some(event.payload().to_owned());
                handle.exit(0);
            });
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .data_directory(profile)
            .inner_size(1280., 800.)
            .initialization_script(script)
            .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(120));
                handle.exit(1);
            });
            // 保持整个隔离目录直到事件循环退出。
            app.manage(temp);
            Ok(())
        })
        .build(context)
        .unwrap();
    app.run_return(|_, _| {});
    assert_eq!(outcome.lock().unwrap().as_deref(), Some("\"ok\""));
}
