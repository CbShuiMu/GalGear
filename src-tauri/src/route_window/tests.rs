//! 在原生 WebView 中验证桌面布局和导出 IPC。真实路线缓存只读。
use super::build_window;
use crate::state::AppState;
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
    let state = AppState::load(temp.path().to_owned()).unwrap();
    let mut game_id = "test".to_owned();
    let game_exe = std::env::var_os("GALGEAR_TEST_ROUTE_EXE");
    let fixture = game_exe.is_none();
    let document = if let Some(exe) = game_exe {
        let mut game = catalog::manual_game(Path::new(&exe), "Route test".into()).unwrap();
        // 扫描导入与手动添加使用不同的游戏 ID，以这份测试缓存的所有者为准。
        let cache = Path::new(&game.folder).join(".galgear-ai-summary/zh-CN/skill-analysis.json");
        let record: Value = serde_json::from_slice(&std::fs::read(cache).unwrap()).unwrap();
        game.id = record["game_id"].as_str().unwrap().into();
        let mut settings = Settings {
            language: Language::Chinese,
            ..Default::default()
        };
        let edit = settings.game_edits.entry(game.id.clone()).or_default();
        edit.ai_summary = true;
        edit.save_path = std::env::var("GALGEAR_TEST_ROUTE_SAVE_DIR").ok();
        let doc = route_view::load(&game, &settings, temp.path()).unwrap();
        game_id = game.id.clone();
        *state.settings.lock().unwrap() = settings;
        state.catalog.lock().unwrap().games.push(game);
        doc
    } else {
        route_view::Document::Interactive(json!({
            "nodes":[
                {"id":"s","type":"start","title":"Start"},
                {"id":"a","type":"choice","title":"First choice"},
                {"id":"b","type":"choice","title":"Second choice"},
                {"id":"c","type":"choice","title":"Third choice"},
                {"id":"e","type":"ending","title":"Early ending"},
                {"id":"ea","type":"ending","title":"First ending"},
                {"id":"eb","type":"ending","title":"Second ending"},
                {"id":"ec","type":"ending","title":"Third ending"}
            ],
            "edges":[
                {"from":"s","to":"a","label":"Continue on the main route and investigate the castle"},
                {"from":"s","to":"e","label":"Leave"},
                {"from":"a","to":"b","label":"Continue"}, {"from":"a","to":"ea","label":"Leave"},
                {"from":"b","to":"c","label":"Continue"}, {"from":"b","to":"eb","label":"Leave"},
                {"from":"c","to":"s","label":"Return to start"}, {"from":"c","to":"ec","label":"Leave"}
            ],"title":"Route test",
            "ui":{"next":"Next","draftNote":"Draft"},"validation":[],"uncertainties":[]
        }))
    };
    let route_view::Document::Interactive(graph) = &document else {
        panic!("interactive graph expected")
    };
    let expected = graph["nodes"].as_array().unwrap().len();
    let locate = std::env::var_os("GALGEAR_TEST_ROUTE_NODE").is_some();
    let check = include_str!("../../../scripts/check-route-map.js")
        .trim_end()
        .trim_end_matches(';');
    let script = format!(
        r#"(async () => {{
        const result = await ({check});
        const button = key => document.querySelector('[data-action="' + key + '"]');
        if ({fixture}) {{
            button('expand').click();
            const center = id => {{
                const node = document.querySelector(`[data-node="${{id}}"]`).parentElement;
                return node.offsetLeft + node.offsetWidth / 2;
            }};
            result.shortBranchOffset = Math.abs(center('e') - center('s'));
            if (result.shortBranchOffset > 120) throw Error('An early ending is too far from its branch point');
            button('collapse').click();
        }}
        const scale = document.querySelector('#toolbar span');
        button('fit').click(); const before = scale.textContent;
        button('in').click(); if (before === scale.textContent) throw Error('zoom');
        button('out').click(); const theme = document.documentElement.dataset.theme;
        button('theme').click(); if (theme === document.documentElement.dataset.theme) throw Error('theme');
        button('json').click(); button('draft').click();
        if ({locate}) {{
            const locate = button('locate');
            async function clickLocate() {{
                locate.click();
                while (locate.disabled) await new Promise(r => setTimeout(r, 15));
            }}
            await clickLocate();
            result.status = document.querySelector('.location-status').textContent;
            const marked = document.querySelector('[aria-current=location]');
            if (!marked || marked !== document.activeElement || marked.parentElement.hidden) throw Error(result.status);
            result.located = marked.dataset.node;
            for (let i = 0; i < 2; i++) {{
                const left = getComputedStyle(marked, '::before');
                const right = getComputedStyle(marked, '::after');
                const highlight = getComputedStyle(marked).borderLeftColor;
                if (left.content === 'none' || parseFloat(left.borderLeftWidth) <= 0 || left.borderLeftColor !== highlight ||
                    right.content === 'none' || parseFloat(right.borderRightWidth) <= 0 || right.borderRightColor !== highlight)
                    throw Error('Missing inward triangles on the located node');
                button('theme').click();
            }}
            window.GalGearRoute.gameId += '-another-game';
            await clickLocate();
            if (document.querySelector('[aria-current=location]') || result.status === document.querySelector('.location-status').textContent)
                throw Error('A failed lookup retained the old position');
            if (getComputedStyle(marked, '::before').content !== 'none' || getComputedStyle(marked, '::after').content !== 'none')
                throw Error('The old node retained its location triangles');
        }}
        return result;
    }})().catch(e => ({{error: String(e)}})).then(result => window.__TAURI__.core.invoke('route_probe', {{result}}));"#
    );
    let mut app = tauri::Builder::default()
        .any_thread()
        .manage(Probe::default())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            route_probe,
            capture_export,
            super::locate_route
        ])
        .on_page_load(move |view, event| {
            if event.event() == PageLoadEvent::Finished {
                view.eval(&script).unwrap();
            }
        })
        .build(context)
        .unwrap();
    let window = build_window(
        app.handle(),
        &game_id,
        "GalGear route test",
        Language::Chinese,
        document,
    )
    .unwrap();
    if fixture {
        window.set_size(tauri::LogicalSize::new(420, 760)).unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    loop {
        #[allow(deprecated)]
        app.run_iteration(|_, _| {});
        let probe = app.state::<Probe>();
        if let Some(result) = probe.results.lock().unwrap().first() {
            assert_eq!(result["nodes"].as_u64(), Some(expected as u64), "{result}");
            if let Ok(node) = std::env::var("GALGEAR_TEST_ROUTE_NODE") {
                assert_eq!(result["located"], node, "{result}");
                if let Ok(text) = std::env::var("GALGEAR_TEST_ROUTE_DETAIL") {
                    assert!(
                        result["status"].as_str().unwrap().contains(&text),
                        "{result}"
                    );
                }
            }
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
