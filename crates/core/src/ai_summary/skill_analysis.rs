//! 面向路线图和前情提要的 Skill 分析入口；与可复用解析器缓存分别持久化。
use super::{
    Setup, Summary, agent, cache, hash, invalid, io, now, read, resources, route_graph, route_html,
    write_json,
};
use crate::{Game, Settings};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Record {
    schema: u32,
    policy: String,
    game_id: String,
    inventory: Vec<resources::Stamp>,
    save_candidates: Value,
    generated_at: u64,
    result: Value,
}

fn fingerprint() -> String {
    let mut sha = Sha256::new();
    for bytes in [
        include_bytes!("analysis_agent.py").as_slice(),
        include_bytes!("analysis_reads.py").as_slice(),
        include_bytes!("analysis_recap.py").as_slice(),
        include_bytes!("route_graph.rs").as_slice(),
        include_bytes!("route_html.rs").as_slice(),
        include_bytes!("../../../../web/route-map.html").as_slice(),
        include_bytes!("../../../../web/route-map.js").as_slice(),
        include_bytes!("../../../../web/route-map.css").as_slice(),
    ] {
        sha.update(bytes);
    }
    format!("{:x}", sha.finalize())
}

fn record_path(game: &Game, settings: &Settings) -> PathBuf {
    cache(game, settings.language).join("skill-analysis.json")
}

pub(super) fn exists(game: &Game, settings: &Settings) -> bool {
    record_path(game, settings).is_file()
}

fn content(result: &Value) -> Result<super::report::Content, Message> {
    let value: super::report::Content = serde_json::from_value(json!({
        "recap":result["recap"], "current_progress":result["current_progress"]
    }))
    .map_err(io)?;
    value.validate()?;
    Ok(value)
}

fn validate(result: &Value) -> Result<route_graph::Graph, Message> {
    if result["schema"] != 1
        || result["mode"] != "skill_analysis"
        || result["save_position"]["runtime_verified"] != false
        || !matches!(
            result["save_position"]["level"].as_str(),
            Some("candidate" | "decoded_active")
        )
        || !result["limitations"].is_array()
    {
        return Err(invalid());
    }
    content(result)?;
    let graph: route_graph::Graph = serde_json::from_value(result["graph"].clone()).map_err(io)?;
    graph.validate()?;
    Ok(graph)
}

fn load(game: &Game, settings: &Settings, check_save: bool) -> Result<Record, Message> {
    let record: Record =
        serde_json::from_slice(&read(&record_path(game, settings), 4 * 1024 * 1024)?)
            .map_err(io)?;
    // 离线重绘只验证数据结构与输入身份；新版视图不要求重新付费分析旧图。
    // 准备/总结仍检查完整策略指纹，防止复用过时的分析方法。
    if record.schema != 1
        || (check_save && record.policy != fingerprint())
        || record.game_id != game.id
        || record.inventory != resources::source_inventory(Path::new(&game.folder))?
    {
        return Err(invalid());
    }
    validate(&record.result)?;
    if check_save {
        let candidates =
            serde_json::to_value(super::preparation_save_inputs(game, settings)?.candidates)
                .map_err(io)?;
        let position = &record.result["save_position"];
        if candidates != record.save_candidates
            || hash(Path::new(position["file"].as_str().ok_or_else(invalid)?))?
                != position["sha256"]
        {
            return Err(Message::new("summary.save_changed"));
        }
    }
    Ok(record)
}

fn setup(game: &Game, settings: &Settings, result: &Value, reused: bool) -> Setup {
    Setup {
        reused,
        directory: crate::config::display_path(&cache(game, settings.language)),
        script_language: result["script_language"]
            .as_str()
            .unwrap_or(settings.language.code())
            .into(),
        dialogue_count: result["scope"]["dialogue_count"].as_u64().unwrap_or(0) as usize,
    }
}

pub(super) fn prepare(
    game: &Game,
    settings: &Settings,
    config: &Path,
    home: &Path,
) -> Result<Setup, Message> {
    if let Ok(record) = load(game, settings, true) {
        return Ok(setup(game, settings, &record.result, true));
    }
    let (provider, model, token) = super::provider(settings, config)?;
    let root = Path::new(&game.folder).canonicalize().map_err(io)?;
    let base = root.join(super::FOLDER);
    if base.exists() && base.canonicalize().map_err(io)? != base {
        return Err(invalid());
    }
    fs::create_dir_all(&base).map_err(io)?;
    let work = tempfile::Builder::new()
        .prefix(".skill-analysis-")
        .tempdir_in(&base)
        .map_err(io)?
        .keep();
    let inventory = resources::source_inventory(&root)?;
    let saves = super::preparation_save_inputs(game, settings)?;
    let candidates = serde_json::to_value(&saves.candidates).map_err(io)?;
    let result = agent::analyze(
        home,
        &work,
        json!({
            "mode":"skill_analysis", "provider":provider,"model":model,"token":token,
            "title":game.title,"language":settings.language.code(),
            "game":crate::config::display_path(&root), "save":saves.save,
            "skill":crate::config::display_path(&home.join("Galgear-GalgameReverse-Skill/GalgameResourceExtract")),
            "work":crate::config::display_path(&work),"inventory":inventory,
            "required_saves":saves.required_saves,"save_candidates":candidates,
            "runtime_limit_seconds":900,"max_turns":60,"graph_contract":route_graph::RULES
        }),
    )?;
    let graph = validate(&result)?;
    if inventory != resources::source_inventory(&root)?
        || candidates
            != serde_json::to_value(super::preparation_save_inputs(game, settings)?.candidates)
                .map_err(io)?
    {
        return Err(Message::new("summary.save_changed"));
    }
    let position = &result["save_position"];
    let selected = Path::new(position["file"].as_str().ok_or_else(invalid)?)
        .canonicalize()
        .map_err(io)?;
    if !saves.candidates.iter().any(|c| {
        Path::new(&c.path)
            .canonicalize()
            .is_ok_and(|p| p == selected)
    }) || hash(&selected)? != position["sha256"]
    {
        return Err(invalid());
    }
    let directory = cache(game, settings.language);
    fs::create_dir_all(&directory).map_err(io)?;
    if directory.canonicalize().map_err(io)? != base.join(settings.language.code()) {
        return Err(invalid());
    }
    let record = Record {
        schema: 1,
        policy: fingerprint(),
        game_id: game.id.clone(),
        inventory,
        save_candidates: candidates,
        generated_at: now(),
        result,
    };
    // 发布以原子 JSON 为准；单文件 HTML 始终可以由同一图结构离线重建。
    write_json(&record_path(game, settings), &record)?;
    fs::write(
        directory.join("route-map.html"),
        route_html::render(&graph, &game.title, settings.language)?,
    )
    .map_err(io)?;
    Ok(setup(game, settings, &record.result, false))
}

pub(super) fn summarize(
    game: &Game,
    settings: &Settings,
    config: &Path,
    home: &Path,
) -> Result<Summary, Message> {
    let (record, cached) = match load(game, settings, true) {
        Ok(record) => (record, true),
        Err(_) => {
            prepare(game, settings, config, home)?;
            (load(game, settings, true)?, false)
        }
    };
    let result = &record.result;
    let mut warnings: Vec<String> =
        serde_json::from_value(result["limitations"].clone()).map_err(io)?;
    if let Some(explanation) = result["save_position"]["explanation"].as_str() {
        warnings.push(explanation.into());
    }
    let summary = Summary {
        game_id: game.id.clone(),
        content: content(result)?,
        provider: result["provider"].as_str().ok_or_else(invalid)?.into(),
        model: result["model"].as_str().ok_or_else(invalid)?.into(),
        generated_at: record.generated_at,
        save_file: result["save_position"]["file"]
            .as_str()
            .ok_or_else(invalid)?
            .into(),
        source_url: String::new(),
        script_language: result["script_language"]
            .as_str()
            .ok_or_else(invalid)?
            .into(),
        warnings,
        cached,
        save_hash: result["save_position"]["sha256"]
            .as_str()
            .ok_or_else(invalid)?
            .into(),
    };
    write_json(
        &cache(game, settings.language).join("latest-summary.json"),
        &summary,
    )?;
    Ok(summary)
}

pub(super) fn route(game: &Game, settings: &Settings) -> Result<PathBuf, Message> {
    let record = load(game, settings, false)?;
    let graph = validate(&record.result)?;
    let path = cache(game, settings.language).join("route-map.html");
    fs::write(
        &path,
        route_html::render(&graph, &game.title, settings.language)?,
    )
    .map_err(io)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::validate;
    use serde_json::json;

    #[test]
    fn static_analysis_cannot_claim_runtime_validation_or_skip_deliverables() {
        let mut value = json!({"schema":1,"mode":"skill_analysis","save_position":{"level":"candidate","runtime_verified":false},
            "recap":"人物在港口商量次日的行程。","current_progress":"目前准备前往车站。","limitations":["仅静态候选"],
            "graph":{"nodes":[{"id":"s","label":"开始","kind":"start","evidence":"O1"}],"edges":[],"uncertainties":["后续未知"]}});
        assert!(validate(&value).is_ok());
        value["save_position"]["runtime_verified"] = json!(true);
        assert!(validate(&value).is_err());
        value["save_position"]["runtime_verified"] = json!(false);
        value["recap"] = json!("无法总结");
        assert!(validate(&value).is_err());
    }
}
