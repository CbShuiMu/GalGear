//! Shared persistence and exclusive write coordination.
use crate::{
    accounts, covers::prepare_cover, downloads, library, patch_commands, save_commands, tools,
};
use galgear_core::{Game, ScanResult, Settings, catalog, config::ConfigStore, playtime};
use galgear_i18n::Message;
use std::{path::PathBuf, sync::Mutex, sync::atomic::AtomicBool, sync::atomic::Ordering};

pub(crate) struct AppState {
    pub(crate) store: ConfigStore,
    pub(crate) settings: Mutex<Settings>,
    pub(crate) catalog: Mutex<ScanResult>,
    pub(crate) playtime: Mutex<playtime::Records>,
    pub(crate) scan: library::ScanState,
    pub(crate) account: accounts::AccountState,
    pub(crate) downloads: downloads::DownloadState,
    pub(crate) patches: patch_commands::PatchState,
    pub(crate) saves: save_commands::SaveState,
    pub(crate) tools: tools::RuntimeState,
    pub(crate) summaries: crate::ai_summary::Jobs,
    writing: AtomicBool,
}

impl AppState {
    pub(crate) fn load(data_dir: PathBuf) -> Result<Self, Message> {
        let store = ConfigStore::new(data_dir.join("settings.json"));
        let mut settings = store.load()?;
        let mut cached =
            catalog::load(&store.path.with_file_name("catalog.json")).unwrap_or_else(|error| {
                ScanResult {
                    warnings: vec![error],
                    ..Default::default()
                }
            });
        let play_records = playtime::load(&store.path.with_file_name("playtime.json"))?;
        let mut summary_disabled = false;
        for (id, edit) in &mut settings.game_edits {
            if !edit.ai_summary {
                continue;
            }
            let game = settings
                .manual_games
                .iter()
                .find(|game| &game.id == id)
                .or_else(|| cached.games.iter().find(|game| &game.id == id))
                .or_else(|| settings.excluded_entries.get(id));
            if let Some(game) = game
                && !PathBuf::from(&game.folder)
                    .join(galgear_core::ai_summary::FOLDER)
                    .is_dir()
            {
                edit.ai_summary = false;
                summary_disabled = true;
            }
        }
        if summary_disabled {
            store.save(&settings)?;
        }
        patch_commands::recover(
            &store.path.with_file_name("patches"),
            &settings,
            &mut cached,
        );
        for game in &mut cached.games {
            catalog::apply_edits(game, &settings);
            if let Some(record) = play_records.get(&game.id) {
                record.apply(game);
            }
        }
        cached.warnings.extend(save_commands::migrate_known_backups(
            &store.path,
            &settings,
            &cached.games,
        ));
        let account = accounts::AccountState::load(&store, &settings);
        Ok(Self {
            store,
            settings: Mutex::new(settings),
            catalog: Mutex::new(cached),
            playtime: Mutex::new(play_records),
            scan: library::ScanState::default(),
            account,
            downloads: downloads::DownloadState::default(),
            patches: patch_commands::PatchState::default(),
            saves: save_commands::SaveState::default(),
            tools: tools::RuntimeState::default(),
            summaries: crate::ai_summary::Jobs::default(),
            writing: AtomicBool::new(false),
        })
    }
}

pub(crate) fn snapshot(state: &AppState) -> Result<Settings, Message> {
    Ok(state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?
        .clone())
}

pub(crate) fn find_game(state: &AppState, id: &str) -> Result<Game, Message> {
    state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .games
        .iter()
        .find(|game| game.id == id)
        .cloned()
        .ok_or_else(|| Message::new("error.game_missing"))
}

pub(crate) fn persist_game(state: &AppState, mut game: Game) -> Result<Game, Message> {
    prepare_cover(
        &mut game,
        &state.store.path.with_file_name("covers").join("rendered"),
    )?;
    let mut list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    if let Some(entry) = list.games.iter_mut().find(|g| g.id == game.id) {
        game.playtime_seconds = entry.playtime_seconds;
        game.last_played = entry.last_played;
        *entry = game.clone();
    }
    catalog::save(&state.store.path.with_file_name("catalog.json"), &list)?;
    Ok(game)
}

pub(crate) struct WriteGuard<'a>(&'a AppState);
impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.0.writing.store(false, Ordering::SeqCst);
    }
}
pub(crate) fn begin_write(state: &AppState) -> Result<WriteGuard<'_>, Message> {
    if state.writing.swap(true, Ordering::SeqCst) {
        return Err(Message::new("backup.busy"));
    }
    Ok(WriteGuard(state))
}

#[cfg(test)]
mod tests {
    use super::{AppState, begin_write};
    use galgear_core::{
        Game, GameEdit, ScanResult, Settings, ai_summary, catalog, config::ConfigStore,
    };
    use galgear_i18n::Message;
    use std::{fs, path::Path};

    fn summary_game(directory: &Path, name: &str) -> Game {
        let folder = directory.join(name);
        fs::create_dir(&folder).unwrap();
        let executable = folder.join("game.exe");
        fs::write(&executable, []).unwrap();
        catalog::manual_game(&executable, name.into()).unwrap()
    }

    #[test]
    fn startup_restores_independent_play_records_even_if_the_catalog_is_stale() {
        let directory = tempfile::tempdir().unwrap();
        let game = summary_game(directory.path(), "game");
        catalog::save(
            &directory.path().join("catalog.json"),
            &ScanResult {
                games: vec![game.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        let record = galgear_core::playtime::Record {
            playtime_seconds: 5400,
            last_played: Some(1730000000),
        };
        galgear_core::playtime::save(
            &directory.path().join("playtime.json"),
            &std::collections::BTreeMap::from([(game.id, record)]),
        )
        .unwrap();
        let state = AppState::load(directory.path().to_owned()).unwrap();
        assert_eq!(
            galgear_core::playtime::Record::from_game(&state.catalog.lock().unwrap().games[0]),
            record
        );
    }

    #[test]
    fn saving_stale_metadata_keeps_the_latest_play_record() {
        let directory = tempfile::tempdir().unwrap();
        let mut game = summary_game(directory.path(), "game");
        let mut stale = game.clone();
        stale.title = "Edited title".into();
        game.playtime_seconds = 7200;
        game.last_played = Some(1730000000);
        let state = AppState::load(directory.path().to_owned()).unwrap();
        state.catalog.lock().unwrap().games.push(game.clone());
        let updated = super::persist_game(&state, stale).unwrap();
        assert_eq!(
            galgear_core::playtime::Record::from_game(&updated),
            galgear_core::playtime::Record::from_game(&game)
        );
    }

    #[test]
    fn startup_disables_missing_summary_setup_and_persists_other_game_options() {
        let directory = tempfile::tempdir().unwrap();
        let game = summary_game(directory.path(), "game");
        let store = ConfigStore::new(directory.path().join("settings.json"));
        let mut settings = Settings::default();
        settings.game_edits.insert(
            game.id.clone(),
            GameEdit {
                ai_summary: true,
                upscale: true,
                title: Some("Custom title".into()),
                ..Default::default()
            },
        );
        store.save(&settings).unwrap();
        catalog::save(
            &directory.path().join("catalog.json"),
            &ScanResult {
                games: vec![game.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        settings.game_edits.get_mut(&game.id).unwrap().ai_summary = false;

        let state = AppState::load(directory.path().to_owned()).unwrap();

        assert_eq!(*state.settings.lock().unwrap(), settings);
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn startup_keeps_existing_summary_directory_and_rechecks_it_on_every_launch() {
        let directory = tempfile::tempdir().unwrap();
        let game = summary_game(directory.path(), "game");
        let setup = Path::new(&game.folder).join(ai_summary::FOLDER);
        fs::create_dir(&setup).unwrap();
        let store = ConfigStore::new(directory.path().join("settings.json"));
        let mut settings = Settings::default();
        settings.manual_games.push(game.clone());
        settings.game_edits.insert(
            game.id.clone(),
            GameEdit {
                ai_summary: true,
                ..Default::default()
            },
        );
        store.save(&settings).unwrap();

        let first = AppState::load(directory.path().to_owned()).unwrap();
        assert!(first.settings.lock().unwrap().game_edits[&game.id].ai_summary);
        drop(first);
        fs::remove_dir(&setup).unwrap();
        let second = AppState::load(directory.path().to_owned()).unwrap();

        assert!(!second.settings.lock().unwrap().game_edits[&game.id].ai_summary);
        assert!(!store.load().unwrap().game_edits[&game.id].ai_summary);
        assert!(!setup.exists());
    }

    #[test]
    fn startup_disables_summary_when_setup_path_is_a_regular_file() {
        let directory = tempfile::tempdir().unwrap();
        let game = summary_game(directory.path(), "game");
        fs::write(Path::new(&game.folder).join(ai_summary::FOLDER), []).unwrap();
        let store = ConfigStore::new(directory.path().join("settings.json"));
        let mut settings = Settings::default();
        settings.manual_games.push(game.clone());
        settings.game_edits.insert(
            game.id.clone(),
            GameEdit {
                ai_summary: true,
                ..Default::default()
            },
        );
        store.save(&settings).unwrap();

        let state = AppState::load(directory.path().to_owned()).unwrap();

        assert!(!state.settings.lock().unwrap().game_edits[&game.id].ai_summary);
    }

    #[test]
    fn startup_checks_manual_and_excluded_games_without_a_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let manual = summary_game(directory.path(), "manual");
        let excluded = summary_game(directory.path(), "excluded");
        let mut settings = Settings::default();
        settings.manual_games.push(manual.clone());
        settings.excluded_games.push(excluded.id.clone());
        settings
            .excluded_entries
            .insert(excluded.id.clone(), excluded.clone());
        for game in [&manual, &excluded] {
            settings.game_edits.insert(
                game.id.clone(),
                GameEdit {
                    ai_summary: true,
                    ..Default::default()
                },
            );
        }
        let store = ConfigStore::new(directory.path().join("settings.json"));
        store.save(&settings).unwrap();

        let state = AppState::load(directory.path().to_owned()).unwrap();

        assert!(
            state
                .settings
                .lock()
                .unwrap()
                .game_edits
                .values()
                .all(|edit| !edit.ai_summary)
        );
        assert!(
            store
                .load()
                .unwrap()
                .game_edits
                .values()
                .all(|edit| !edit.ai_summary)
        );
    }

    #[test]
    fn shared_write_coordination_rejects_overlap_and_releases_after_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().to_owned()).unwrap();
        let guard = begin_write(&state).unwrap();
        assert!(matches!(begin_write(&state), Err(error) if error.key == "backup.busy"));
        drop(guard);

        let failed = (|| -> Result<(), Message> {
            let _guard = begin_write(&state)?;
            Err(Message::new("error.operation"))
        })();
        assert!(failed.is_err());
        assert!(begin_write(&state).is_ok());
    }
}
