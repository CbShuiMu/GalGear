use super::*;

#[test]
fn interrupted_restore_removes_only_the_file_that_was_originally_absent() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("SAVE.SAV");
    fs::write(&file, b"snapshot").unwrap();
    let roots = vec![display_path(&file)];
    let data = temp.path().join("backups");
    let snapshot = create(&data, "air", &roots, "", 10, Status::Stopped).unwrap();
    fs::remove_file(&file).unwrap();
    let plan = preview(&data, "air", &snapshot.id, &roots, Status::Stopped).unwrap();
    fs::write(&file, b"snapshot").unwrap();
    fs::write(temp.path().join("air.exe"), b"game").unwrap();
    write_journal(
        &journal_path(&data, "air").unwrap(),
        &Journal {
            plan,
            originals: vec![display_path(
                &temp.path().join(".galgear-restore-test.original"),
            )],
            absent: vec![0],
            committed: false,
        },
    )
    .unwrap();
    recover(&data, "air", &roots, Status::Stopped).unwrap();
    assert!(!file.exists());
    assert_eq!(fs::read(temp.path().join("air.exe")).unwrap(), b"game");
}

#[test]
fn loose_files_restore_deleted_save_and_preserve_game_resources() {
    let temp = tempfile::tempdir().unwrap();
    let game = temp.path().join("AIR");
    fs::create_dir(&game).unwrap();
    fs::write(game.join("air_cn.exe"), b"executable").unwrap();
    fs::write(game.join("SAVE.SAV"), b"original progress").unwrap();
    fs::write(game.join("READ.SAV"), b"original read flags").unwrap();
    let data = temp.path().join("backups");
    let roots = validate_roots(
        &[
            display_path(&game.join("READ.SAV")),
            display_path(&game.join("SAVE.SAV")),
        ],
        &game,
        &data,
    )
    .unwrap();
    assert!(validate_roots(&[display_path(&game)], &game, &data).is_err());
    let snapshot = create(&data, "air", &roots, "", 10, Status::Stopped).unwrap();
    assert_eq!(snapshot.files.len(), 2);
    assert_eq!(snapshot.file_roots, vec![0, 1]);
    fs::write(game.join("READ.SAV"), b"new read flags").unwrap();
    fs::remove_file(game.join("SAVE.SAV")).unwrap();
    fs::write(game.join("other.sav"), b"outside selection").unwrap();
    let roots = validate_roots(&roots, &game, &data).unwrap();
    let preview = preview(&data, "air", &snapshot.id, &roots, Status::Stopped).unwrap();
    assert!(
        preview
            .changes
            .iter()
            .any(|change| change.path == "SAVE.SAV" && change.action == "add")
    );
    restore(&data, &preview, 10, Status::Stopped).unwrap();
    assert_eq!(
        fs::read(game.join("SAVE.SAV")).unwrap(),
        b"original progress"
    );
    assert_eq!(
        fs::read(game.join("READ.SAV")).unwrap(),
        b"original read flags"
    );
    assert_eq!(fs::read(game.join("air_cn.exe")).unwrap(), b"executable");
    assert_eq!(
        fs::read(game.join("other.sav")).unwrap(),
        b"outside selection"
    );
    assert!(!recovery_pending(&data, "air").unwrap());
}

#[test]
fn interrupted_file_restore_rolls_back_exact_files() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("SAVE.SAV");
    fs::write(&file, b"snapshot").unwrap();
    let roots = vec![display_path(&file)];
    let data = temp.path().join("backups");
    let snapshot = create(&data, "air", &roots, "", 10, Status::Stopped).unwrap();
    fs::write(&file, b"before restore").unwrap();
    let plan = preview(&data, "air", &snapshot.id, &roots, Status::Stopped).unwrap();
    let old = temp.path().join(".galgear-restore-test.original");
    fs::rename(&file, &old).unwrap();
    fs::write(&file, b"snapshot").unwrap();
    write_journal(
        &journal_path(&data, "air").unwrap(),
        &Journal {
            plan,
            originals: vec![display_path(&old)],
            absent: vec![],
            committed: false,
        },
    )
    .unwrap();
    recover(&data, "air", &roots, Status::Stopped).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"before restore");
    assert!(!old.exists());
}

#[test]
fn file_restore_rejects_changed_contents_and_directory_target() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("SAVE.SAV");
    fs::write(&file, b"snapshot").unwrap();
    let roots = vec![display_path(&file)];
    let data = temp.path().join("backups");
    let snapshot = create(&data, "air", &roots, "", 10, Status::Stopped).unwrap();
    let plan = preview(&data, "air", &snapshot.id, &roots, Status::Stopped).unwrap();
    fs::write(&file, b"late write").unwrap();
    assert!(restore(&data, &plan, 10, Status::Stopped).is_err());
    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    assert!(preview(&data, "air", &snapshot.id, &roots, Status::Stopped).is_err());
}

#[test]
fn imported_file_snapshot_retains_exact_file_scope() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("SAVE.SAV");
    fs::write(&file, b"snapshot").unwrap();
    let roots = vec![display_path(&file)];
    let data = temp.path().join("backups");
    let snapshot = create(&data, "air", &roots, "", 10, Status::Stopped).unwrap();
    let extracted = content_directory(&data, "air", &snapshot.id).unwrap();
    let imported =
        import_verified(&temp.path().join("other"), "air", snapshot, &extracted).unwrap();
    assert_eq!(imported.file_roots, vec![0]);
    fs::write(&file, b"new").unwrap();
    let plan = preview(
        &temp.path().join("other"),
        "air",
        &imported.id,
        &roots,
        Status::Stopped,
    )
    .unwrap();
    restore(&temp.path().join("other"), &plan, 10, Status::Stopped).unwrap();
    assert_eq!(fs::read(file).unwrap(), b"snapshot");
}
