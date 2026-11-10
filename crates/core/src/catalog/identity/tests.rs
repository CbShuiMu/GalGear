use super::empty_scan_duplicates;
use crate::{GameEdit, ScanResult, Settings, catalog, config::display_path, scanner};
use std::fs;

fn fixture() -> (tempfile::TempDir, Settings, ScanResult) {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Nightmare×Sisters");
    fs::create_dir(&folder).unwrap();
    for name in ["AdvHD.exe", "AdvHDDD.exe", "data.arc"] {
        fs::write(folder.join(name), b"fixture").unwrap();
    }
    let mut settings = Settings {
        library_folders: vec![display_path(temp.path())],
        ..Default::default()
    };
    let scanned = scanner::scan(&settings);
    settings
        .manual_games
        .push(catalog::manual_game(&folder.join("AdvHDDD.exe"), "Custom title".into()).unwrap());
    (temp, settings, scanned)
}

#[test]
fn repeated_full_and_incremental_scans_keep_manual_identity_and_launcher() {
    let (_temp, mut settings, _) = fixture();
    let manual = settings.manual_games[0].clone();
    // 手动配置写入后、目录缓存尚未写入时，也应识别原身份。
    let first = scanner::scan(&settings);
    assert_eq!(first.games.len(), 1);
    assert_eq!(first.games[0].id, manual.id);
    settings.game_edits.insert(
        manual.id.clone(),
        GameEdit {
            setup_confirmed: true,
            title: Some("My game".into()),
            tags: vec!["Favorite".into()],
            ..Default::default()
        },
    );
    crate::collections::edit(&mut settings, "create", "", "Favorites", None).unwrap();
    settings.collections[0].games.insert(manual.id.clone());
    let mut previous = ScanResult {
        games: vec![manual.clone()],
        ..Default::default()
    };
    previous.games[0].playtime_seconds = 123;
    previous.games[0].last_played = Some(100);
    previous.games[0].metadata = Some(crate::providers::Metadata {
        title: "Original metadata".into(),
        ..Default::default()
    });
    for full in [true, false, true, false] {
        let fresh = if full {
            scanner::scan_refresh(&settings, &previous, &Default::default(), &mut |_| {})
        } else {
            scanner::scan_incremental(&settings, &previous, &Default::default(), &mut |_| {})
        };
        assert_eq!(fresh.games[0].id, manual.id);
        crate::game_setup::mark_new(&mut settings, &fresh.games, &Default::default());
        previous = catalog::merge(fresh, &previous, &settings);
        assert_eq!(previous.games.len(), 1);
        let game = &previous.games[0];
        assert_eq!(game.executable, manual.executable);
        assert_eq!(game.title, "My game");
        assert_eq!(game.tags, ["Favorite"]);
        assert_eq!(game.metadata.as_ref().unwrap().title, "Original metadata");
        assert_eq!((game.playtime_seconds, game.last_played), (123, Some(100)));
        assert!(!settings.game_edits[&manual.id].setup_pending);
        assert!(settings.collections[0].games.contains(&game.id));
    }
}

#[test]
fn full_refresh_reloads_files_without_changing_relocated_identity() {
    let (_temp, mut settings, mut previous) = fixture();
    settings.manual_games.clear();
    previous.games[0].id = "relocated-id".into();
    previous.games[0].cover = Some("stale-cache".into());
    let incremental =
        scanner::scan_incremental(&settings, &previous, &Default::default(), &mut |_| {});
    assert_eq!(incremental.games[0].cover.as_deref(), Some("stale-cache"));
    let full = scanner::scan_refresh(&settings, &previous, &Default::default(), &mut |_| {});
    assert_eq!(full.games[0].id, "relocated-id");
    assert!(full.games[0].cover.is_none());
}

#[test]
fn distinct_manual_executables_in_one_folder_survive_both_scan_modes() {
    let (_temp, mut settings, scanned) = fixture();
    let path = std::path::Path::new(scanned.games[0].executable.as_ref().unwrap());
    let mut manual = catalog::manual_game(path, "Other version".into()).unwrap();
    manual.folder = manual.folder.replace('\\', "/") + "/";
    settings.manual_games.push(manual.clone());
    for full in [true, false] {
        let previous = ScanResult {
            games: settings.manual_games.clone(),
            ..Default::default()
        };
        let fresh = if full {
            scanner::scan_refresh(&settings, &previous, &Default::default(), &mut |_| {})
        } else {
            scanner::scan_incremental(&settings, &previous, &Default::default(), &mut |_| {})
        };
        assert_eq!(fresh.games[0].id, manual.id);
        let merged = catalog::merge(fresh, &previous, &settings);
        assert_eq!(merged.games.len(), 2);
        for manual in &settings.manual_games {
            assert!(
                merged
                    .games
                    .iter()
                    .any(|game| game.id == manual.id && game.executable == manual.executable)
            );
        }
    }
}

#[test]
fn only_untouched_pending_scan_duplicates_are_candidates() {
    let (_temp, mut settings, scanned) = fixture();
    let duplicate = &scanned.games[0];
    settings.game_edits.insert(
        duplicate.id.clone(),
        GameEdit {
            setup_pending: true,
            ..Default::default()
        },
    );
    assert_eq!(
        empty_scan_duplicates(&scanned.games, &settings).as_slice(),
        std::slice::from_ref(&duplicate.id)
    );
    let mut played = scanned.clone();
    played.games[0].playtime_seconds = 1;
    assert!(empty_scan_duplicates(&played.games, &settings).is_empty());
    settings
        .game_edits
        .get_mut(&duplicate.id)
        .unwrap()
        .executable = duplicate.executable.clone();
    assert!(empty_scan_duplicates(&scanned.games, &settings).is_empty());
    settings
        .game_edits
        .get_mut(&duplicate.id)
        .unwrap()
        .executable = None;
    let mut second = settings.manual_games[0].clone();
    second.id = "second-manual".into();
    settings.manual_games.push(second);
    assert!(empty_scan_duplicates(&scanned.games, &settings).is_empty());
}
