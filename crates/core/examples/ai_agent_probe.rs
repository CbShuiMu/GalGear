//! Read-only application-state probe; generated programs/resources remain in the game cache.
use galgear_core::{ai_summary, catalog, config::ConfigStore, saves};
use serde_json::json;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let config = PathBuf::from(args.next().ok_or("config path required")?);
    let id = args.next().ok_or("catalog game id required")?;
    let root = PathBuf::from(args.next().ok_or("actual game folder required")?);
    let save = args.next().ok_or("save folder required")?;
    let action = args.next().unwrap_or_else(|| "prepare".into());
    let mut settings = ConfigStore::new(config.clone()).load()?;
    let mut game = catalog::load(&config.with_file_name("catalog.json"))?
        .games
        .into_iter()
        .find(|game| game.id == id)
        .ok_or("game missing from catalog")?;
    let exe = game
        .executable
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .ok_or("game executable missing")?;
    game.executable = Some(root.join(exe).to_string_lossy().into_owned());
    game.folder = root.to_string_lossy().into_owned();
    let discovered = saves::discover(&game, None);
    let found_saved_games = discovered
        .locations
        .iter()
        .any(|l| l.source == "saved-games" && l.path.eq_ignore_ascii_case(&save));
    let edit = settings.game_edits.entry(id).or_default();
    edit.save_path = Some(save);
    edit.ai_summary = true;
    let report = saves::discover(&game, Some(edit));
    if report.locations.len() != 1 || !report.locations[0].exists {
        return Err("selected save location was not isolated".into());
    }
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let setup = ai_summary::prepare(&game, &settings, &config, &home)?;
    println!(
        "{}",
        serde_json::to_string(
            &json!({"setup":setup,"save_locations":report.locations.len(),"automatically_found_saved_games":found_saved_games})
        )?
    );
    if action == "all" {
        let first = ai_summary::summarize(&game, &settings, &config, &home)?;
        let second = ai_summary::summarize(&game, &settings, &config, &home)?;
        if !second.cached
            || first.save_hash != second.save_hash
            || first.generated_at != second.generated_at
        {
            return Err("unchanged save cache was not reused".into());
        }
        println!(
            "{}",
            serde_json::to_string(
                &json!({"cached_second":second.cached,"save_file":second.save_file,"same_generation_time":first.generated_at==second.generated_at,"recap_nonempty":!second.content.recap.is_empty(),"progress_nonempty":!second.content.current_progress.is_empty(),"provider":second.provider,"model":second.model})
            )?
        );
    }
    Ok(())
}
