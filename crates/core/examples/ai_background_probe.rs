//! Read-only production background lookup; no AI requests or configuration writes.
use galgear_core::{ai_summary, catalog, config::ConfigStore};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let config = PathBuf::from(args.next().ok_or("settings path required")?);
    let id = args.next().ok_or("catalog game id required")?;
    let settings = ConfigStore::new(config.clone()).load()?;
    let mut game = catalog::load(&config.with_file_name("catalog.json"))?
        .games
        .into_iter()
        .find(|game| game.id == id)
        .ok_or("catalog game not found")?;
    catalog::apply_edits(&mut game, &settings);
    let background = ai_summary::background(&game, &settings, &config)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "game_id":game.id,"local_title":game.title,"executable":game.executable,
            "background_title":background.title,"source":background.source,"url":background.url,
            "description_chars":background.description.chars().count(),
            "description_sha256":format!("{:x}",Sha256::digest(background.description.as_bytes())),
            "settings_written":false,"ai_requests":0,
        }))?
    );
    Ok(())
}
