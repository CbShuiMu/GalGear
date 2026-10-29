//! Serial background enhancement, with compare-before-publish and durable failure backoff.
use crate::state::AppState;
use galgear_core::{
    catalog,
    cover_art::{self, Artwork},
    runtime,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{Emitter, Manager};

#[derive(Serialize, Deserialize)]
struct Failure {
    retry_after: u64,
    error: String,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn failure_path(cache: &Path, source: &str) -> PathBuf {
    cache
        .join("enhanced")
        .join(format!("{source}-{}.failure.json", cover_art::PIPELINE))
}
fn cooling_down(cache: &Path, source: &str) -> bool {
    std::fs::read(failure_path(cache, source))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Failure>(&bytes).ok())
        .is_some_and(|failure| failure.retry_after > now())
}
fn remember_failure(cache: &Path, source: &str, error: String) {
    let path = failure_path(cache, source);
    let _ = std::fs::create_dir_all(cache.join("enhanced"));
    if let Ok(mut file) = tempfile::NamedTempFile::new_in(cache.join("enhanced")) {
        let failure = Failure {
            retry_after: now() + 6 * 3600,
            error,
        };
        if serde_json::to_writer(&mut file, &failure).is_ok() && file.as_file().sync_all().is_ok() {
            let _ = file.persist(path);
        }
    }
}
fn engine(app: &tauri::AppHandle) -> Option<PathBuf> {
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| root.join("tools/waifu2x"));
    bundled
        .into_iter()
        .chain([PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tools/waifu2x")])
        .find(|path| path.join("waifu2x-ncnn-vulkan.exe").is_file())
}

fn commit(state: &AppState, id: &str, expected: &str, artwork: &Artwork) -> Result<bool, String> {
    let mut list = state.catalog.lock().map_err(|e| e.to_string())?;
    let Some(index) = list
        .games
        .iter()
        .position(|game| game.id == id && game.cover.as_deref() == Some(expected))
    else {
        return Ok(false);
    };
    let mut next = list.clone();
    artwork.apply(&mut next.games[index]);
    catalog::save(&state.store.path.with_file_name("catalog.json"), &next)
        .map_err(|e| e.to_string())?;
    *list = next;
    Ok(true)
}

fn tick(app: &tauri::AppHandle, engine: &Path) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.scan.is_scanning() {
        return Ok(());
    }
    let mut games = state
        .catalog
        .lock()
        .map_err(|e| e.to_string())?
        .games
        .clone();
    let running: Vec<_> = games
        .iter()
        .map(|g| (g.id.clone(), g.executable.clone()))
        .collect();
    let paused = || {
        state.scan.is_scanning()
            || runtime::detect_many(&running)
                .values()
                .any(|status| *status == runtime::Status::Running)
    };
    if paused() {
        return Ok(());
    }
    games.sort_by_key(|game| std::cmp::Reverse(game.last_played));
    let cache = state.store.path.with_file_name("covers").join("rendered");
    for mut game in games {
        let Some(expected) = game.cover.clone() else {
            continue;
        };
        if game
            .cover_artwork
            .as_ref()
            .is_some_and(|art| art.available(&cache) && expected == cover_art::url(&art.cover))
        {
            continue;
        }
        cover_art::restore(&mut game, &cache);
        let Some(source) = cover_art::source_key(&game).map(str::to_owned) else {
            continue;
        };
        if cooling_down(&cache, &source) {
            continue;
        }
        let result = match game.cover_artwork {
            Some(record) if record.available(&cache) => Ok(record),
            _ => cover_art::process(engine, &cache, &source, paused),
        };
        match result {
            Ok(artwork) => {
                if commit(&state, &game.id, &expected, &artwork)? {
                    let _ = app.emit(
                        "cover-enhanced",
                        serde_json::json!({
                            "id": game.id, "previous_cover": expected,
                            "cover": cover_art::url(&artwork.cover), "cover_artwork": artwork,
                        }),
                    );
                }
            }
            Err(error) if !paused() => {
                eprintln!("Cover enhancement: {error}");
                remember_failure(&cache, &source, error);
            }
            Err(_) => {}
        }
        break;
    }
    Ok(())
}

pub(crate) fn scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        let Some(engine) = engine(&app) else {
            eprintln!("Cover enhancement unavailable: install tools/waifu2x");
            return;
        };
        loop {
            let handle = app.clone();
            let engine = engine.clone();
            match tauri::async_runtime::spawn_blocking(move || tick(&handle, &engine)).await {
                Ok(Ok(())) => {}
                other => eprintln!("Cover worker: {other:?}"),
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_worker_cannot_replace_a_new_cover_or_resurrect_removed_game() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::load(root.path().to_owned()).unwrap();
        let mut game = catalog::manual_game(Path::new("game.exe"), "New title".into()).unwrap();
        game.cover = Some("new-cover".into());
        let id = game.id.clone();
        state.catalog.lock().unwrap().games.push(game);
        let art = Artwork {
            pipeline: cover_art::PIPELINE.into(),
            source: "s".into(),
            cover: "c".into(),
            background: "b".into(),
        };
        assert!(!commit(&state, &id, "old-cover", &art).unwrap());
        assert_eq!(
            state.catalog.lock().unwrap().games[0].cover.as_deref(),
            Some("new-cover")
        );
        assert!(commit(&state, &id, "new-cover", &art).unwrap());
        assert_eq!(state.catalog.lock().unwrap().games[0].title, "New title");
        state.catalog.lock().unwrap().games.clear();
        assert!(!commit(&state, &id, "new-cover", &art).unwrap());
    }
}
