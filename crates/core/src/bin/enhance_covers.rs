//! Offline deployment helper. Close GalGear before using it against its data directory.
use galgear_core::{catalog, cover_art, runtime};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let cache_only = args.len() == 3 && args[2] == "--cache-only";
    if args.len() != 2 && !cache_only {
        return Err(
            "Usage: enhance_covers <data-directory> <waifu2x-directory> [--cache-only] (close GalGear before catalog writes)"
                .into(),
        );
    }
    let data = PathBuf::from(&args[0]);
    let engine = std::fs::canonicalize(&args[1])?;
    let path = data.join("catalog.json");
    let cache = data.join("covers/rendered");
    let mut catalog = catalog::load(&path)?;
    let running: Vec<_> = catalog
        .games
        .iter()
        .map(|game| (game.id.clone(), game.executable.clone()))
        .collect();
    let paused = || {
        runtime::detect_many(&running)
            .values()
            .any(|s| *s == runtime::Status::Running)
    };
    if paused() {
        return Err("Close running games before offline enhancement".into());
    }
    let mut ready = 0;
    let mut reused = 0;
    let mut missing = 0;
    let mut failed = 0;
    let backup = data.join("catalog.before-cover-enhancement.json");
    if !cache_only && !backup.exists() {
        std::fs::copy(&path, &backup)?;
    }
    for index in 0..catalog.games.len() {
        let game = &mut catalog.games[index];
        let Some(source) = cover_art::source_key(game).map(str::to_owned) else {
            missing += 1;
            continue;
        };
        cover_art::restore(game, &cache);
        if game
            .cover_artwork
            .as_ref()
            .is_some_and(|record| record.available(&cache))
        {
            reused += 1;
            continue;
        }
        let source = cover_art::source_key(game).unwrap_or(&source).to_owned();
        let start = Instant::now();
        match cover_art::process(&engine, &cache, &source, paused) {
            Ok(artwork) => {
                artwork.apply(game);
                ready += 1;
                println!("ready {} {:.2}s", game.id, start.elapsed().as_secs_f64());
                if !cache_only {
                    catalog::save(&path, &catalog)?;
                }
            }
            Err(error) => {
                failed += 1;
                eprintln!("failed {}: {error}", game.id);
            }
        }
    }
    if !cache_only {
        catalog::save(&path, &catalog)?;
    }
    println!("generated={ready} reused={reused} without_cover={missing} failed={failed}");
    if failed > 0 {
        return Err("Some covers could not be enhanced; their originals remain active".into());
    }
    Ok(())
}
