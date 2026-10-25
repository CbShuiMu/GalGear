//! Manual end-to-end probe; reuses the vault key without printing it or changing settings.
use galgear_core::{ai_summary, catalog, config::ConfigStore};
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let action = args.next().unwrap_or_else(|| "prepare".into());
    let config = PathBuf::from(args.next().ok_or("config path required")?);
    let root = PathBuf::from(args.next().ok_or("game directory required")?);
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut settings = ConfigStore::new(config.clone()).load()?;
    let mut game = catalog::manual_game(&root.join("angeltear.exe"), "Angel Tear".into())?;
    if let Ok(saved) = catalog::load(&config.with_file_name("catalog.json"))
        && let Some(entry) = saved
            .games
            .into_iter()
            .find(|g| g.folder.eq_ignore_ascii_case(&root.to_string_lossy()))
    {
        game = entry;
    }
    if action == "install-verified" {
        // Reuse the verified program author's provider locally, without model discovery
        // or changing the application's settings. prepare still rechecks all artifacts.
        let result: serde_json::Value = serde_json::from_slice(&std::fs::read(
            root.join(".galgear-ai-summary/.agent-zh-CN-deepseek/result.json"),
        )?)?;
        settings.ai_provider = result["provider"]
            .as_str()
            .ok_or("missing verified provider")?
            .into();
        settings.ai_models.insert(
            settings.ai_provider.clone(),
            result["model"]
                .as_str()
                .ok_or("missing verified model")?
                .into(),
        );
    }
    if matches!(
        action.as_str(),
        "prepare" | "reuse" | "all" | "install-verified"
    ) {
        println!(
            "{}",
            serde_json::to_string_pretty(&ai_summary::prepare(&game, &settings, &config, &home)?)?
        );
    }
    if matches!(action.as_str(), "summarize" | "all") {
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .ai_summary = true;
        let result = ai_summary::summarize(&game, &settings, &config, &home)?;
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    if action == "verify-cache" {
        let setup = ai_summary::prepare(&game, &settings, &config, &home)?;
        settings
            .game_edits
            .entry(game.id.clone())
            .or_default()
            .ai_summary = true;
        let first = ai_summary::summarize(&game, &settings, &config, &home)?;
        let second = ai_summary::summarize(&game, &settings, &config, &home)?;
        if !second.cached
            || first.generated_at != second.generated_at
            || first.content.recap != second.content.recap
            || first.save_hash != second.save_hash
        {
            return Err("unchanged save was not reused".into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "preparation_reused":setup.reused,"dialogue_count":setup.dialogue_count,
                "script_language":setup.script_language,"first_cached":first.cached,
                "second_cached":second.cached,"same_generation_time":first.generated_at==second.generated_at,
                "save_file":second.save_file,"save_hash":second.save_hash,
                "provider":second.provider,"model":second.model,
                "recap_nonempty":!second.content.recap.is_empty(),"progress_nonempty":!second.content.current_progress.is_empty()
            }))?
        );
    }
    Ok(())
}
