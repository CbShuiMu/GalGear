//! Print production save selection without credentials, model requests or cache reuse.
use galgear_core::{ai_summary, catalog, config::ConfigStore};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let config = PathBuf::from(args.next().ok_or("settings path required")?);
    let root = PathBuf::from(args.next().ok_or("game folder required")?).canonicalize()?;
    let executable = args.next().map(PathBuf::from);
    let settings = ConfigStore::new(config.clone()).load()?;
    let saved = catalog::load(&config.with_file_name("catalog.json"))?;
    let game = if let Some(game) = saved.games.into_iter().find(|game| {
        PathBuf::from(&game.folder)
            .canonicalize()
            .is_ok_and(|folder| folder == root)
    }) {
        game
    } else {
        let executable = if let Some(executable) = executable {
            executable
        } else {
            let mut executables = fs::read_dir(&root)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                })
                .collect::<Vec<_>>();
            executables.sort();
            if executables.len() != 1 {
                return Err(
                    "provide an explicit executable when the catalog has no unique game".into(),
                );
            }
            executables.remove(0)
        };
        let title = root
            .file_name()
            .ok_or("game folder has no title")?
            .to_string_lossy()
            .into_owned();
        catalog::manual_game(&executable, title)?
    };
    let inputs = ai_summary::preparation_save_inputs(&game, &settings)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "game_id":game.id, "game":game.folder, "title":game.title,
            "language":settings.language.code(), "save_inputs":inputs,
        }))?
    );
    Ok(())
}
