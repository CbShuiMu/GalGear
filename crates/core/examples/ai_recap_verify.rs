//! Exercise the installed preparation and actual summary path without printing story text.
use galgear_core::{ai_summary, catalog, config::ConfigStore};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let config = PathBuf::from(arguments.next().ok_or("settings path required")?);
    let folder = PathBuf::from(arguments.next().ok_or("game folder required")?);
    let home = PathBuf::from(arguments.next().ok_or("application home required")?);
    let executable = PathBuf::from(arguments.next().ok_or("game executable required")?);
    let title = arguments.next().ok_or("game title required")?;
    let saves = arguments.next().ok_or("selected save directory required")?;
    let directory = folder.join(".galgear-ai-summary/zh-CN");
    let manifest: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json"))?)?;
    let result: Value = serde_json::from_slice(&fs::read(directory.join("result.json"))?)?;
    let mut game = catalog::manual_game(&executable, title)?;
    game.id = manifest["game_id"]
        .as_str()
        .ok_or("missing game id")?
        .into();
    let mut settings = ConfigStore::new(config.clone()).load()?;
    settings.ai_provider = result["provider"]
        .as_str()
        .ok_or("missing provider")?
        .into();
    settings.ai_models.insert(
        settings.ai_provider.clone(),
        result["model"].as_str().ok_or("missing model")?.into(),
    );
    let edit = settings.game_edits.entry(game.id.clone()).or_default();
    edit.save_path = Some(saves);
    edit.ai_summary = true;
    let started = Instant::now();
    let setup = ai_summary::prepare(&game, &settings, &config, &home)?;
    let first = ai_summary::summarize(&game, &settings, &config, &home)?;
    let second = ai_summary::summarize(&game, &settings, &config, &home)?;
    if !setup.reused
        || first.content.recap.trim().is_empty()
        || first.content.current_progress.trim().is_empty()
        || !second.cached
        || first.generated_at != second.generated_at
        || first.save_hash != second.save_hash
        || first.content.recap != second.content.recap
        || first.content.current_progress != second.content.current_progress
    {
        return Err("actual recap or unchanged-save reuse failed".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "preparation_reused":setup.reused,"dialogue_count":setup.dialogue_count,
            "first_summary_cached":first.cached,"second_summary_cached":second.cached,
            "recap_characters":first.content.recap.chars().count(),
            "progress_characters":first.content.current_progress.chars().count(),
            "provider":first.provider,"model":first.model,"save_file":first.save_file,
            "save_hash":first.save_hash,"same_generated_at":first.generated_at==second.generated_at,
            "elapsed_seconds":started.elapsed().as_secs_f64()
        }))?
    );
    Ok(())
}
