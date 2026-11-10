//! Per-game association and executable confirmation.
use crate::{Game, GameEdit, Settings, config::display_path, data_sources, providers, tags};
use galgear_i18n::Message;
use std::{collections::BTreeSet, path::Path};

/// 列出游戏目录直属的普通 EXE 文件，不跟随链接或 Windows 重解析点。
pub fn executables(folder: &Path) -> Result<Vec<std::path::PathBuf>, Message> {
    let root =
        std::fs::canonicalize(folder).map_err(|error| Message::detail("error.operation", error))?;
    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(&root).map_err(|error| Message::detail("error.operation", error))?
    {
        let entry = entry.map_err(|error| Message::detail("error.operation", error))?;
        let metadata = entry
            .metadata()
            .map_err(|error| Message::detail("error.operation", error))?;
        if !metadata.is_file()
            || entry
                .file_type()
                .map_err(|error| Message::detail("error.operation", error))?
                .is_symlink()
        {
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
        }
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            files.push(entry.path());
        }
    }
    files.sort_by_cached_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    Ok(files)
}

pub fn mark_new(settings: &mut Settings, games: &[Game], _known: &BTreeSet<String>) -> bool {
    let mut changed = false;
    for game in games {
        if crate::catalog::is_excluded(game, settings) {
            continue;
        }
        let edit = settings
            .game_edits
            .entry(game.id.clone())
            .or_insert_with(|| GameEdit {
                tags: game.tags.clone(),
                play_status: game.play_status,
                ..Default::default()
            });
        if !edit.setup_confirmed && !edit.setup_pending {
            edit.setup_pending = true;
            changed = true;
        }
    }
    changed
}

pub fn validate(source: &str, url: &str, executable: &str, confirmed: bool) -> Result<(), Message> {
    if !confirmed {
        return Err(Message::new("setup.confirm_required"));
    }
    data_sources::validate_source(source)?;
    if url.trim().is_empty() || data_sources::source_for_url(url)?.0 != source {
        return Err(Message::new("setup.entry_required"));
    }
    if executable.trim().is_empty() {
        return Ok(());
    }
    let path = Path::new(executable.trim());
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Err(Message::new("setup.executable_required"));
    }
    Ok(())
}

pub fn prepare(
    game: &Game,
    previous: Option<&GameEdit>,
    source: &str,
    executable: &str,
    metadata: providers::Metadata,
) -> Result<GameEdit, Message> {
    validate(source, &metadata.url, executable, true)?;
    let mut edit = previous.cloned().unwrap_or_else(|| GameEdit {
        play_status: game.play_status,
        ..Default::default()
    });
    if executable.trim().is_empty() {
        edit.launcher_pending = edit.executable.is_none();
    } else {
        let executable = display_path(Path::new(executable.trim()));
        if game.executable.as_deref() != Some(&executable) {
            edit.working_directory = Path::new(&executable).parent().map(display_path);
        }
        edit.executable = Some(executable);
        edit.launcher_pending = false;
    }
    edit.metadata_provider = source.into();
    edit.setup_pending = false;
    edit.setup_confirmed = true;
    edit.tags = tags::merge(&game.tags, &metadata.tag_names(), &edit.tag_rules);
    edit.metadata = Some(metadata);
    Ok(edit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_without_launcher_defers_only_new_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let game =
            crate::catalog::manual_game(&dir.path().join("guess.exe"), "Game".into()).unwrap();
        let metadata = providers::Metadata {
            url: "https://vndb.org/v1".into(),
            ..Default::default()
        };
        let edit = prepare(&game, None, "vndb", "", metadata.clone()).unwrap();
        assert!(edit.setup_confirmed && edit.launcher_pending);
        assert!(edit.executable.is_none());
        let previous = GameEdit {
            executable: Some("chosen.exe".into()),
            ..Default::default()
        };
        let edit = prepare(&game, Some(&previous), "vndb", "", metadata).unwrap();
        assert!(!edit.launcher_pending);
        assert_eq!(edit.executable.as_deref(), Some("chosen.exe"));
    }

    #[test]
    fn executable_candidates_exclude_directories_and_non_executables() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("translation.EXE"), b"fixture").unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"fixture").unwrap();
        std::fs::create_dir(dir.path().join("folder.exe")).unwrap();
        let candidates = executables(dir.path()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].file_name().unwrap(), "translation.EXE");
        assert!(executables(&dir.path().join("missing")).is_err());
    }
    #[test]
    fn setup_validates_the_entry_and_real_executable_before_changing_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.exe");
        std::fs::write(&path, b"fixture").unwrap();
        let exe = display_path(&path);
        assert!(validate("vndb", "https://vndb.org/v1", &exe, false).is_err());
        assert!(validate("bangumi", "https://vndb.org/v1", &exe, true).is_err());
        assert!(validate("vndb", "https://vndb.org/v1", "missing.exe", true).is_err());
        let game = crate::catalog::manual_game(&path, "Game".into()).unwrap();
        let previous = GameEdit {
            setup_pending: true,
            arguments: vec!["--local".into()],
            play_status: crate::PlayStatus::Playing,
            ..Default::default()
        };
        let edit = prepare(
            &game,
            Some(&previous),
            "vndb",
            &exe,
            providers::Metadata {
                url: "https://vndb.org/v1".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(edit.setup_confirmed && !edit.setup_pending);
        assert_eq!(edit.arguments, previous.arguments);
        assert_eq!(edit.play_status, previous.play_status);
        assert_eq!(edit.metadata_provider, "vndb");
        let mut settings = Settings::default();
        settings.game_edits.insert(game.id.clone(), edit);
        settings.game_edits.insert(
            "other".into(),
            GameEdit {
                metadata_provider: "bangumi".into(),
                ..Default::default()
            },
        );
        let store = crate::config::ConfigStore::new(dir.path().join("settings.json"));
        store.save(&settings).unwrap();
        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.game_edits[&game.id].metadata_provider, "vndb");
        assert_eq!(reloaded.game_edits["other"].metadata_provider, "bangumi");
        assert_eq!(reloaded.metadata_provider, "nextmoe");
        assert!(reloaded.game_edits[&game.id].setup_confirmed);
    }

    #[test]
    fn unconfirmed_cached_games_are_pending_and_confirmed_games_are_preserved() {
        let games: Vec<_> = ["old", "new", "confirmed"]
            .into_iter()
            .map(|id| {
                let mut game =
                    crate::catalog::manual_game(Path::new("F:/game.exe"), id.into()).unwrap();
                game.id = id.into();
                game
            })
            .collect();
        let mut settings = Settings::default();
        settings.game_edits.insert(
            "confirmed".into(),
            GameEdit {
                setup_confirmed: true,
                ..Default::default()
            },
        );
        assert!(mark_new(
            &mut settings,
            &games,
            &BTreeSet::from(["old".into()])
        ));
        assert!(settings.game_edits["old"].setup_pending);
        assert!(settings.game_edits["new"].setup_pending);
        assert!(!settings.game_edits["confirmed"].setup_pending);
        assert!(!mark_new(
            &mut settings,
            &games,
            &BTreeSet::from(["old".into()])
        ));
    }
}
