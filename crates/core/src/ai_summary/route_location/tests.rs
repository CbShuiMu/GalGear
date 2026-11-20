use super::locate;
use crate::{
    Game, Settings,
    ai_summary::{self, route_view},
    catalog,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn put(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn setup(root: &Path) -> (Game, Settings, String) {
    let exe = root.join("story.exe");
    fs::write(&exe, []).unwrap();
    let game = catalog::manual_game(&exe, "Test".into()).unwrap();
    let mut settings = Settings::default();
    let edit = settings.game_edits.entry(game.id.clone()).or_default();
    edit.ai_summary = true;
    let slots = root.join("saves");
    fs::create_dir(&slots).unwrap();
    edit.save_path = Some(crate::config::display_path(&slots));
    let base = root.join(ai_summary::FOLDER);
    let work = base.join(".skill-analysis-fixture");
    let input = work.join("script.ks.scn");
    let index = "reader-results/1/index.json";
    let output = "reader-results/1/script.json";
    let lines = |start: usize, texts: &[&str]| {
        texts
            .iter()
            .enumerate()
            .map(|(i, text)| json!({"record_index":start+i,"variants":[{"text":text}]}))
            .collect::<Vec<_>>()
    };
    put(
        &work.join(output),
        &json!({"source_sha256":"scriptDigest","scene_dialogue":[
            {"scene_index":0,"label":"*a","records":lines(4,&["First","Again"])},
            {"scene_index":1,"label":"*b","records":lines(18,&["Later","Again","Ending"])}
        ]}),
    );
    put(
        &work.join(index),
        &json!({"records":[{"input":input,"output":output,"reader":"structured","sha256":"scriptDigest"}]}),
    );
    let analyzed = slots.join("slot1.bmp");
    let nodes = [
        ("start", "start", "start"),
        ("a", "scene", "script.ks.scn *a"),
        ("b", "scene", "script.ks.scn *b"),
        ("old", "save", "analyzed slot"),
        ("end", "ending", "terminal"),
    ];
    let graph = json!({"nodes":nodes.map(|(id,kind,pc)|json!({"id":id,"kind":kind,"pc":pc,"label":id,"evidence":"fixture"})),
        "edges":([("start","a"),("start","b"),("a","old"),("b","end")].map(|(from,to)| json!({"from":from,"to":to,"label":"","inferred":false,"evidence":"fixture"}))),
        "uncertainties":[]});
    let result = json!({"schema":1,"mode":"skill_analysis","recap":"人物在港口商量次日的行程。","current_progress":"目前准备前往车站。",
        "limitations":[],"save_position":{"level":"candidate","runtime_verified":false,"file":analyzed,"sha256":format!("{:x}",Sha256::digest(b"analyzed slot"))},
        "inputs":{(input.to_str().unwrap()):{}},"reader_indexes":[index],"graph":graph});
    put(
        &ai_summary::cache(&game, settings.language).join("skill-analysis.json"),
        &json!({
        "game_id":game.id,"schema":1,"policy":"","inventory":[],"save_candidates":[],"generated_at":0,"result":result}),
    );
    let route_view::Document::Interactive(graph) =
        route_view::load(&game, &settings, root).unwrap()
    else {
        panic!("interactive map expected")
    };
    (game, settings, graph["revision"].as_str().unwrap().into())
}

fn slot(path: &Path, day: u64, data: &[u8]) {
    fs::write(path, data).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(day * 86_400))
        .unwrap();
}

fn bookmark(scene: &str, caption: &str, previous: &str) -> String {
    format!(
        r#"%["core"=>%["currentStorage"=>"script.ks","scenePlayer"=>%["scene"=>"{scene}"],"bookMarkInfo"=>%["lastText"=>"{caption}"],"historyData"=>%["data"=>[%["text"=>"{previous}"]]]]]"#
    )
}

#[test]
fn latest_slot_refreshes_the_open_graph_and_resolves_repeated_lines_only_with_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    // manual_game normalizes Windows extended paths before persisting input identities.
    let root = std::path::PathBuf::from(crate::config::display_path(tmp.path()));
    let (game, settings, revision) = setup(&root);
    let slots = root.join("saves");
    let locate = || locate(&game, &settings, &revision);
    assert_eq!(locate().err().unwrap().key, "summary.no_saves");
    slot(&slots.join("slot1.bmp"), 1, b"analyzed slot");
    assert_eq!(locate().unwrap().node, "a");
    slot(
        &slots.join("slot2.bmp"),
        2,
        bookmark("script.ks*a", "Again", "First").as_bytes(),
    );
    assert_eq!(locate().unwrap().node, "a");
    slot(
        &slots.join("slot3.bmp"),
        3,
        bookmark("", "Again", "Later").as_bytes(),
    );
    let found = locate().unwrap();
    assert_eq!(found.node, "start");
    assert!(found.detail.contains("19"));
    slot(
        &slots.join("slot3.bmp"),
        4,
        bookmark("", "Again", "Unknown").as_bytes(),
    );
    assert_eq!(locate().err().unwrap().key, "route.locate_ambiguous");
    slot(
        &slots.join("slot3.bmp"),
        5,
        bookmark("script.ks*a", "Ending", "Later").as_bytes(),
    );
    assert_eq!(locate().err().unwrap().key, "route.locate_unmatched");
    slot(&slots.join("quicksave.bmp"), 6, b"incomplete slot");
    assert_eq!(locate().err().unwrap().key, "route.locate_unsupported");
    slot(&slots.join("slot4.bmp"), 6, b"another slot");
    assert_eq!(locate().err().unwrap().key, "route.locate_ambiguous");
    assert_eq!(
        super::locate(&game, &settings, "earlier map")
            .err()
            .unwrap()
            .key,
        "route.locate_map_changed"
    );
}
