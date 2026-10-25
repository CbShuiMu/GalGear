//! Preview legacy save-backup transfers; add `--apply` to perform verified migration.
//! Usage: migrate_save_backups <config-dir> [--apply] [--game-folder <id> <folder>]
use galgear_core::{catalog, config::ConfigStore, snapshots};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

struct GameLocation {
    title: String,
    folder: PathBuf,
}

fn snapshot_count(folder: &Path) -> Result<usize, std::io::Error> {
    let mut count = 0;
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("manifest.json").is_file() {
            count += 1;
        }
    }
    Ok(count)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let config_dir = PathBuf::from(args.next().ok_or(
        "usage: migrate_save_backups <config-dir> [--apply] [--game-folder <id> <folder>]",
    )?);
    if !config_dir.is_absolute() || !config_dir.is_dir() {
        return Err("config directory must be an existing absolute directory".into());
    }
    let mut apply = false;
    let mut overrides = BTreeMap::new();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--apply" => apply = true,
            "--game-folder" => {
                let id = args.next().ok_or("--game-folder requires a game id")?;
                let folder = PathBuf::from(args.next().ok_or("--game-folder requires a folder")?);
                if id.is_empty()
                    || id.len() > 128
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || !folder.is_absolute()
                {
                    return Err(
                        "explicit game mapping requires a valid id and absolute folder".into(),
                    );
                }
                overrides.insert(id, folder);
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }

    let config_path = config_dir.join("settings.json");
    let settings = ConfigStore::new(config_path).load()?;
    let cached = catalog::load(&config_dir.join("catalog.json"))?;
    let mut games = BTreeMap::new();
    for game in cached.games.iter().chain(&settings.manual_games) {
        games.insert(
            game.id.clone(),
            GameLocation {
                title: game.title.clone(),
                folder: PathBuf::from(&game.folder),
            },
        );
    }
    for game in settings.excluded_entries.values() {
        games
            .entry(game.id.clone())
            .or_insert_with(|| GameLocation {
                title: game.title.clone(),
                folder: PathBuf::from(&game.folder),
            });
    }
    for (id, folder) in overrides {
        let title = folder
            .file_name()
            .map_or_else(|| id.clone(), |name| name.to_string_lossy().into_owned());
        games
            .entry(id)
            .and_modify(|game| game.folder.clone_from(&folder))
            .or_insert(GameLocation { title, folder });
    }

    let mut legacy_roots = vec![config_dir.join("save-backups")];
    if !settings.backup_folder.is_empty() {
        let custom = PathBuf::from(&settings.backup_folder);
        if !legacy_roots.iter().any(|root| {
            fs::canonicalize(root).unwrap_or_else(|_| root.clone())
                == fs::canonicalize(&custom).unwrap_or_else(|_| custom.clone())
        }) {
            legacy_roots.push(custom);
        }
    }

    println!("Mode: {}", if apply { "apply" } else { "preview" });
    let mut failures = 0;
    let mut total = 0;
    for legacy_root in legacy_roots {
        if !legacy_root.exists() {
            continue;
        }
        if fs::symlink_metadata(&legacy_root)?.file_type().is_symlink() {
            eprintln!(
                "UNCHANGED: legacy root is a link: {}",
                legacy_root.display()
            );
            failures += 1;
            continue;
        }
        let mut entries = fs::read_dir(&legacy_root)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            if !kind.is_dir() || kind.is_symlink() {
                if kind.is_symlink() {
                    eprintln!(
                        "UNCHANGED: legacy entry is a link: {}",
                        entry.path().display()
                    );
                    failures += 1;
                }
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let Some(game) = games.get(&id) else {
                eprintln!(
                    "UNCHANGED: no catalog game or explicit mapping for {id}: {}",
                    entry.path().display()
                );
                failures += 1;
                continue;
            };
            if !game.folder.is_dir() {
                eprintln!(
                    "UNCHANGED: game folder is missing for {}: {}",
                    game.title,
                    game.folder.display()
                );
                failures += 1;
                continue;
            }
            let data = snapshots::game_data(&game.folder);
            let count = snapshot_count(&entry.path())?;
            println!("Game: {} ({id})", game.title);
            println!("  From: {}", entry.path().display());
            println!("  To:   {}", data.join(&id).display());
            println!("  Snapshots: {count}");
            if apply {
                match snapshots::migrate(&legacy_root, &data, &id) {
                    Ok(moved) => {
                        println!("  Migrated: {moved}");
                        total += moved;
                    }
                    Err(error) => {
                        eprintln!("  Migration failed; unverified source data remains: {error}");
                        failures += 1;
                    }
                }
            } else {
                total += count;
            }
        }
    }
    println!(
        "Total {}: {total}",
        if apply { "migrated" } else { "planned" }
    );
    if failures > 0 {
        return Err(format!("{failures} game backup location(s) require attention").into());
    }
    Ok(())
}
