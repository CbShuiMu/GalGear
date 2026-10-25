//! Run production preparation with DeepSeek and report only safe validation metadata.
use galgear_core::{ai_summary, catalog, config::ConfigStore};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    time::Instant,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let config = PathBuf::from(arguments.next().ok_or("settings path required")?);
    let folder = PathBuf::from(arguments.next().ok_or("game folder required")?);
    let home = PathBuf::from(arguments.next().ok_or("application home required")?);
    let mut settings = ConfigStore::new(config.clone()).load()?;
    settings.ai_provider = "deepseek".into();
    let saved = catalog::load(&config.with_file_name("catalog.json"))?
        .games
        .into_iter()
        .find(|game| game.folder.eq_ignore_ascii_case(&folder.to_string_lossy()));
    let mut game = if let Some(saved) = saved {
        saved
    } else {
        let executable = arguments
            .next()
            .ok_or("manual game requires its actual executable path")?;
        let title = arguments
            .next()
            .ok_or("manual game requires its actual title")?;
        let executable = PathBuf::from(executable);
        if !executable.is_file() || executable.parent() != Some(folder.as_path()) {
            return Err(
                "manual executable must be an existing file in the selected game folder".into(),
            );
        }
        catalog::manual_game(&executable, title)?
    };
    let selected_save = arguments.next();
    if arguments.next().as_deref() == Some("--resume-existing-cache") {
        let installed = folder.join(".galgear-ai-summary/zh-CN");
        let manifest: Value = serde_json::from_slice(&fs::read(installed.join("manifest.json"))?)?;
        game.id = manifest["game_id"]
            .as_str()
            .ok_or("cached game id required")?
            .into();
        game.metadata = Some(serde_json::from_slice(&fs::read(
            installed.join("background.json"),
        )?)?);
        let prepared: Value = serde_json::from_slice(&fs::read(
            folder.join(".galgear-ai-summary/.agent-zh-CN-deepseek/result.json"),
        )?)?;
        settings.ai_models.insert(
            "deepseek".into(),
            prepared["model"]
                .as_str()
                .ok_or("prepared model required")?
                .into(),
        );
    }
    catalog::apply_edits(&mut game, &settings);
    if let Some(selected_save) = selected_save {
        if !PathBuf::from(&selected_save).is_dir() {
            return Err("selected save folder must exist".into());
        }
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .save_path = Some(selected_save);
    }
    let started = Instant::now();
    let setup = ai_summary::prepare(&game, &settings, &config, &home)?;
    let directory = PathBuf::from(&setup.directory);
    let mut total = 0;
    let mut named = 0;
    let mut speakers = BTreeSet::new();
    for line in BufReader::new(fs::File::open(directory.join("dialogue.jsonl"))?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        total += 1;
        if let Some(name) = row["speaker"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
        {
            named += 1;
            speakers.insert(name.to_owned());
        }
    }
    let result: Value = serde_json::from_slice(&fs::read(directory.join("result.json"))?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "provider":result["provider"],"model":result["model"],
            "reused":setup.reused,"directory":setup.directory,
            "elapsed_seconds":started.elapsed().as_secs_f64(),
            "dialogue_rows":total,"named_rows":named,"unnamed_rows":total-named,
            "distinct_displayed_names":speakers.len(),
            "speaker_resolution":result["speaker_resolution"],
            "speaker_verification":result["speaker_verification"],
            "validated_save_count":result["validated_saves"].as_object().map_or(0, |saves| saves.len()),
            "limited_save_count":result["limited_samples"].as_object().map_or(0, |saves| saves.len()),
            "script_language":setup.script_language
        }))?
    );
    Ok(())
}
