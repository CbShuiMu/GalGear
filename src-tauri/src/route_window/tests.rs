//! 在原生 WebView 中验证桌面布局和导出 IPC。真实路线缓存只读。
use super::{build_window, window_label};
use galgear_core::{Settings, ai_summary::route_view, catalog};
use galgear_i18n::Language;
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex, time::Duration};
use tauri::{Manager, State, webview::PageLoadEvent};

#[derive(Default)]
struct Probe {
    exports: Mutex<Vec<(String, String)>>,
    results: Mutex<Vec<Value>>,
}

#[tauri::command(rename = "export_route")]
async fn capture_export(
    kind: String,
    contents: String,
    state: State<'_, Probe>,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    if std::env::var_os("GALGEAR_TEST_ROUTE_DIALOGS").is_some() {
        let format = serde_json::from_value(json!(kind)).map_err(|e| e.to_string())?;
        super::export_route(format, contents.clone(), window).await?;
    }
    state.exports.lock().unwrap().push((kind, contents));
    Ok(())
}

#[tauri::command]
fn route_probe(result: Value, app: tauri::AppHandle) {
    app.state::<Probe>().results.lock().unwrap().push(result);
}

#[test]
#[ignore = "需要 Windows WebView2 桌面；可用 GALGEAR_TEST_ROUTE_EXE 指定已有 Skill 路线图的游戏"]
fn native_route_view_preserves_controls_and_exports() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let document = if let Some(exe) = std::env::var_os("GALGEAR_TEST_ROUTE_EXE") {
        let mut game = catalog::manual_game(Path::new(&exe), "Route test".into()).unwrap();
        // 扫描导入与手动添加使用不同的游戏 ID，以这份测试缓存的所有者为准。
        let cache = Path::new(&game.folder).join(".galgear-ai-summary/zh-CN/skill-analysis.json");
        let record: Value = serde_json::from_slice(&std::fs::read(cache).unwrap()).unwrap();
        game.id = record["game_id"].as_str().unwrap().into();
        let mut settings = Settings {
            language: Language::Chinese,
            ..Default::default()
        };
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .ai_summary = true;
        route_view::load(&game, &settings, temp.path()).unwrap()
    } else {
        route_view::Document::Interactive(json!({
            "nodes":[{"id":"s","type":"start","title":"Start"},{"id":"e","type":"ending","title":"Ending"}],
            "edges":[{"from":"s","to":"e","label":"Option"}],"title":"Route test",
            "ui":{"next":"Next","draftNote":"Draft"},"validation":[],"uncertainties":[]
        }))
    };
    let route_view::Document::Interactive(graph) = &document else {
        panic!("interactive graph expected")
    };
    let expected = graph["nodes"].as_array().unwrap().len();
    let check = include_str!("../../../scripts/check-route-map.js")
        .trim_end()
        .trim_end_matches(';');
    let script = format!(
        r#"(async () => {{
        const result = await ({check});
        const button = key => document.querySelector('[data-action="' + key + '"]');
        const scale = document.querySelector('#toolbar span');
        button('fit').click(); const before = scale.textContent;
        button('in').click(); if (before === scale.textContent) throw Error('zoom');
        button('out').click(); const theme = document.documentElement.dataset.theme;
        button('theme').click(); if (theme === document.documentElement.dataset.theme) throw Error('theme');
        button('json').click(); button('draft').click();
        return result;
    }})().catch(e => ({{error: String(e)}})).then(result => window.__TAURI__.core.invoke('route_probe', {{result}}));"#
    );
    let mut app = tauri::Builder::default()
        .any_thread()
        .manage(Probe::default())
        .invoke_handler(tauri::generate_handler![route_probe, capture_export])
        .on_page_load(move |view, event| {
            if event.event() == PageLoadEvent::Finished {
                view.eval(&script).unwrap();
            }
        })
        .build(context)
        .unwrap();
    let window = build_window(
        app.handle(),
        &window_label("test"),
        "GalGear route test",
        Language::Chinese,
        document,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    loop {
        #[allow(deprecated)]
        app.run_iteration(|_, _| {});
        let probe = app.state::<Probe>();
        if let Some(result) = probe.results.lock().unwrap().first() {
            assert_eq!(result["nodes"].as_u64(), Some(expected as u64), "{result}");
            let exports = probe.exports.lock().unwrap();
            if exports.len() == 2 {
                let (_, data) = exports.iter().find(|(kind, _)| kind == "json").unwrap();
                let data: Value = serde_json::from_str(data).unwrap();
                assert_eq!(data["nodes"].as_array().unwrap().len(), expected);
                assert!(
                    exports
                        .iter()
                        .any(|(kind, text)| kind == "draft" && text.contains('['))
                );
                eprintln!("Native route result: {result}");
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "route window or export IPC timed out"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    window.close().unwrap();
}
