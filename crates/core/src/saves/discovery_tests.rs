use super::{discover_in, paths, select_recent_ai};
use crate::{GameEdit, catalog::manual_game, config::display_path};
use std::{
    fs,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

fn write_at(folder: &Path, name: &str, seconds: u64) {
    fs::create_dir_all(folder).unwrap();
    let path = folder.join(name);
    fs::write(&path, b"fixture").unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)))
        .unwrap();
}

#[test]
fn fhd_edition_finds_player_saves_and_recovers_an_old_ai_placeholder_selection() {
    let temp = tempfile::tempdir().unwrap();
    let game_dir = temp.path().join("狙われた女神天使エンゼルティアーFHD版");
    let roaming = temp.path().join("Roaming");
    let saves = roaming
        .join("SYRUP")
        .join("狙われた女神天使エンゼルティアーFHD");
    let bundled = game_dir
        .join("$(APPDATA)")
        .join("SYRUP")
        .join("狙われた女神天使エンゼルティアーFHD");
    write_at(&bundled, "sysenv.dat", 9000);
    write_at(&saves, "save0000.bmp", 2000);
    write_at(
        &roaming.join("SYRUP/狙われた女神天使エンゼルティアー"),
        "save0000.bmp",
        10000,
    );
    let game = manual_game(
        &game_dir.join("angeltear.exe"),
        "狙われた女神天使エンゼルティアー 〜守った人間達に裏切られて〜 FHD版".into(),
    )
    .unwrap();
    let roots = [("roaming".into(), roaming)];
    let report = discover_in(&game, None, &roots);
    assert_eq!(report.locations.len(), 1);
    assert_eq!(report.locations[0].path, display_path(&saves));
    let mut edit = GameEdit {
        save_path: Some(display_path(&bundled)),
        save_path_auto: true,
        ..Default::default()
    };
    assert_eq!(
        discover_in(&game, Some(&edit), &roots).locations[0].path,
        display_path(&saves)
    );
    edit.save_path_auto = false;
    assert_eq!(
        discover_in(&game, Some(&edit), &roots).locations[0].path,
        display_path(&bundled)
    );
}

#[test]
fn unrelated_recent_files_do_not_promote_old_saves_or_parent_directories() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("savedata");
    let current = temp.path().join("save_chs");
    write_at(&old, "slot.sav", 1000);
    write_at(&old, "readme.txt", 9000);
    write_at(&current, "slot.sav", 2000);
    let game = manual_game(&temp.path().join("game.exe"), "Example".into()).unwrap();
    let mut report = discover_in(&game, None, &[]);
    select_recent_ai(&mut report, &old, &game);
    assert_eq!(report.locations[0].path, display_path(&current));
    let mut report = discover_in(&game, None, &[]);
    select_recent_ai(&mut report, temp.path(), &game);
    assert_eq!(report.locations[0].path, display_path(&current));
}

#[test]
fn configured_appdata_is_expanded_without_using_bundled_directory() {
    let temp = tempfile::tempdir().unwrap();
    let game_dir = temp.path().join("Installation");
    let roaming = temp.path().join("Roaming");
    fs::create_dir_all(&game_dir).unwrap();
    let saves = roaming.join("Vendor").join("Product");
    write_at(&saves, "slot.sav", 2000);
    let game = manual_game(&game_dir.join("game.exe"), "Localized title".into()).unwrap();
    for value in ["$(APPDATA)/Vendor/Product", "%APPDATA%/Vendor/Product"] {
        fs::write(game_dir.join("config.ini"), format!("SavePath={value}")).unwrap();
        let report = discover_in(&game, None, &[("roaming".into(), roaming.clone())]);
        assert_eq!(report.locations.len(), 1);
        assert_eq!(report.locations[0].path, display_path(&saves));
        assert!(!paths::has_placeholder(Path::new(
            &report.locations[0].path
        )));
    }
}

#[test]
fn loose_air_files_are_one_candidate_and_manual_selection_is_exact() {
    let temp = tempfile::tempdir().unwrap();
    for name in [
        "air_cn.exe",
        "SAVE.SAV",
        "READ.SAV",
        "resource.dat",
        "cover.jpg",
    ] {
        std::fs::write(temp.path().join(name), name.as_bytes()).unwrap();
    }
    let game = crate::catalog::manual_game(&temp.path().join("air_cn.exe"), "AIR".into()).unwrap();
    let report = super::discover_in(&game, None, &[]);
    let candidate = report
        .locations
        .iter()
        .find(|location| !location.files.is_empty())
        .unwrap();
    assert_eq!(candidate.files.len(), 2);
    assert_eq!(candidate.confidence, "detected");
    assert!(candidate.files.iter().all(|file| file.ends_with(".SAV")));
    let edit = crate::GameEdit {
        save_files: candidate.files.clone(),
        ..Default::default()
    };
    let confirmed = super::discover_in(&game, Some(&edit), &[]);
    assert_eq!(confirmed.locations.len(), 1);
    assert_eq!(confirmed.locations[0].confidence, "confirmed");
    assert_eq!(
        super::location_roots(&confirmed.locations[0]),
        candidate.files
    );
}
