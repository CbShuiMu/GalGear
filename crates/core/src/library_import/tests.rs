use super::*;
use std::{fs, io::Write};

fn game(dir: &Path) -> Game {
    let folder = dir.join("游戏");
    fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("game.exe");
    fs::write(&exe, b"fixture, never execute").unwrap();
    catalog::manual_game(&exe, "测试游戏".into()).unwrap()
}
fn zip(path: &Path, files: &[(&str, Vec<u8>)]) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}
fn csv(headers: &[&str], rows: &[Vec<String>]) -> Vec<u8> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(headers).unwrap();
    for row in rows {
        w.write_record(row).unwrap();
    }
    w.into_inner().unwrap()
}
#[test]
fn potato_archive_round_trip_retains_metadata_time_cover_and_collection() {
    let temp = tempfile::tempdir().unwrap();
    let mut g = game(temp.path());
    g.playtime_seconds = 3660;
    g.last_played = Some(1700000000);
    g.play_status = PlayStatus::Finished;
    g.tags = vec!["中文".into()];
    g.metadata = Some(Metadata {
        url: "https://vndb.org/v123".into(),
        description: "正文".into(),
        brand: "开发组".into(),
        ..Default::default()
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 4)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    g.cover = Some(archive::image_data(png.get_ref()).unwrap());
    let mut settings = Settings::default();
    settings.collections.push(Collection {
        id: "c".into(),
        name: "喜欢".into(),
        games: BTreeSet::from([g.id.clone()]),
        parent: None,
    });
    let path = temp.path().join("test.pvnExport.zip");
    crate::potatovn::export(&path, std::slice::from_ref(&g), &settings, temp.path()).unwrap();
    let plan = read(Source::Potato, &path, &Settings::default(), &[]).unwrap();
    assert_eq!((plan.preview.added, plan.preview.skipped), (1, 0));
    let imported = &plan.games[0].0;
    assert_eq!(imported.playtime_seconds, 3660);
    assert_eq!(imported.last_played, g.last_played);
    assert_eq!(imported.play_status, PlayStatus::Finished);
    assert_eq!(imported.tags, g.tags);
    assert_eq!(
        imported.metadata.as_ref().unwrap().url,
        "https://vndb.org/v123"
    );
    assert!(
        imported
            .cover
            .as_ref()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    assert_eq!(
        plan.collections[0].games,
        BTreeSet::from([imported.id.clone()])
    );
}
#[test]
fn reina_current_database_is_read_only_and_custom_metadata_takes_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let g = game(temp.path());
    let path = temp.path().join("reina.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE games(id INTEGER,id_type TEXT,localpath TEXT,executable TEXT,custom_data TEXT,clear INTEGER,le_launch INTEGER,magpie INTEGER);CREATE TABLE game_sources(game_id INTEGER,source TEXT,external_id TEXT,data TEXT);CREATE TABLE game_statistics(game_id INTEGER,total_time INTEGER,last_played INTEGER);CREATE TABLE seaql_migrations(version TEXT);INSERT INTO seaql_migrations VALUES('m20260131_000006_migrate_clear_to_play_status');CREATE TABLE collections(id INTEGER,name TEXT);CREATE TABLE game_collection_link(game_id INTEGER,collection_id INTEGER);").unwrap();
    db.execute(
        "INSERT INTO games VALUES(1,'vndb',?1,'game.exe',?2,3,1,0)",
        rusqlite::params![g.folder, r#"{"name":"自定义名字","tags":["中文"]}"#],
    )
    .unwrap();
    db.execute(
        "INSERT INTO game_sources VALUES(1,'vndb','v42',?1)",
        [r#"{"name":"原名","summary":"内容","developer":"开发者"}"#],
    )
    .unwrap();
    db.execute_batch("INSERT INTO game_statistics VALUES(1,120,1700000000);INSERT INTO collections VALUES(2,'收藏');INSERT INTO game_collection_link VALUES(1,2);").unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let plan = read(Source::Reina, &path, &Settings::default(), &[]).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(plan.games[0].0.title, "自定义名字");
    assert_eq!(plan.games[0].0.playtime_seconds, 7200);
    assert_eq!(plan.games[0].0.play_status, PlayStatus::Finished);
    assert_eq!(
        plan.games[0].0.metadata.as_ref().unwrap().url,
        "https://vndb.org/v42"
    );
    assert_eq!(plan.games[0].1.launch_mode, "locale");
    assert!(!plan.games[0].1.upscale);
    assert_eq!(plan.collections.len(), 1);
}
#[test]
fn reina_legacy_executable_and_old_boolean_status_are_supported() {
    let temp = tempfile::tempdir().unwrap();
    let g = game(temp.path());
    let path = temp.path().join("old.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE games(id INTEGER,id_type TEXT,localpath TEXT,name TEXT,clear INTEGER,bgm_id TEXT);").unwrap();
    db.execute(
        "INSERT INTO games VALUES(1,'bgm',?1,'旧游戏',1,'42')",
        [g.executable.as_ref().unwrap()],
    )
    .unwrap();
    drop(db);
    let plan = read(Source::Reina, &path, &Settings::default(), &[]).unwrap();
    assert_eq!(plan.games[0].0.play_status, PlayStatus::Finished);
    assert_eq!(
        plan.games[0].0.metadata.as_ref().unwrap().url,
        "https://bgm.tv/subject/42"
    );
}
#[test]
fn luna_backup_parses_quoted_csv_sessions_tags_and_categories_without_executing_sql() {
    let temp = tempfile::tempdir().unwrap();
    let g = game(temp.path());
    let path = temp.path().join("luna.zip");
    zip(
        &path,
        &[
            (
                "database/games.csv",
                csv(
                    &[
                        "id",
                        "name",
                        "path",
                        "summary",
                        "source_type",
                        "source_id",
                        "status",
                        "use_magpie",
                    ],
                    &[vec![
                        "one".into(),
                        "标题,含逗号".into(),
                        g.executable.unwrap(),
                        "第一行\n第二行\"引号\"".into(),
                        "bangumi".into(),
                        "42".into(),
                        "playing".into(),
                        "true".into(),
                    ]],
                ),
            ),
            (
                "database/play_sessions.csv",
                csv(
                    &["game_id", "start_time", "duration"],
                    &[
                        vec![
                            "one".into(),
                            "2024-01-02 12:00:00+08:00".into(),
                            "120".into(),
                        ],
                        vec!["one".into(), "2024-01-03 12:00:00+08".into(), "60".into()],
                    ],
                ),
            ),
            (
                "database/categories.csv",
                b"id,name,is_system\nc1,Favorites,false\n".to_vec(),
            ),
            (
                "database/game_categories.csv",
                b"game_id,category_id\none,c1\n".to_vec(),
            ),
            (
                "database/game_tags.csv",
                b"game_id,name\none,Tag\n".to_vec(),
            ),
            (
                "database/load.sql",
                b"INSTALL untrusted; DROP TABLE games;".to_vec(),
            ),
        ],
    );
    let plan = read(Source::Luna, &path, &Settings::default(), &[]).unwrap();
    let g = &plan.games[0].0;
    assert_eq!(g.title, "标题,含逗号");
    assert_eq!(g.playtime_seconds, 180);
    assert_eq!(g.last_played, timestamp("2024-01-03T04:00:00Z"));
    assert_eq!(g.tags, vec!["Tag"]);
    assert_eq!(plan.collections.len(), 1);
    assert!(plan.games[0].1.upscale);
}
#[test]
fn duplicate_excluded_and_missing_games_are_reported_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let g = game(temp.path());
    let path = temp.path().join("games.zip");
    crate::potatovn::export(
        &path,
        std::slice::from_ref(&g),
        &Settings::default(),
        temp.path(),
    )
    .unwrap();
    let mut settings = Settings::default();
    settings.excluded_entries.insert(g.id.clone(), g.clone());
    settings.excluded_games.push(g.id.clone());
    let plan = read(Source::Potato, &path, &settings, &[]).unwrap();
    assert_eq!(
        plan.preview.rows[0].reason.as_ref().unwrap().key,
        "import.duplicate"
    );
    assert!(plan.games.is_empty());
    let plan = read(Source::Potato, &path, &Settings::default(), &[]).unwrap();
    let mut next = Settings::default();
    assert_eq!(merge(&plan, &mut next, &[]).len(), 1);
    assert!(merge(&plan, &mut next, &[]).is_empty());
    fs::remove_file(g.executable.unwrap()).unwrap();
    assert!(merge(&plan, &mut Settings::default(), &[]).is_empty());
    let missing = read(Source::Potato, &path, &Settings::default(), &[]).unwrap();
    assert_eq!(
        missing.preview.rows[0].reason.as_ref().unwrap().key,
        "import.missing_path"
    );
}
#[test]
fn malformed_backups_and_archive_traversal_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bad.zip");
    zip(&path, &[("../escape.json", b"[]".to_vec())]);
    assert_eq!(
        read(Source::Potato, &path, &Settings::default(), &[])
            .err()
            .unwrap()
            .key,
        "import.unsafe_path"
    );
    assert!(!temp.path().join("escape.json").exists());
    zip(
        &path,
        &[(
            "database/games.csv",
            b"id,name,path,source_type\n1,short\n".to_vec(),
        )],
    );
    assert!(read(Source::Luna, &path, &Settings::default(), &[]).is_err());
    zip(&path, &[("data.galgames.json", b"{}".to_vec())]);
    assert!(read(Source::Potato, &path, &Settings::default(), &[]).is_err());
    assert_eq!(timestamp("无效无效无效无效无效无效无效"), None);
}
#[test]
fn sqlite_views_cannot_impersonate_a_games_table() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("view.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE VIEW games AS SELECT 1 AS id, 'vndb' AS id_type;")
        .unwrap();
    drop(db);
    assert_eq!(
        read(Source::Reina, &path, &Settings::default(), &[])
            .err()
            .unwrap()
            .key,
        "import.format"
    );
}

#[test]
fn scanning_an_imported_folder_reuses_its_identity() {
    let temp = tempfile::tempdir().unwrap();
    let game = game(temp.path());
    fs::write(Path::new(&game.folder).join("data.xp3"), b"fixture").unwrap();
    let path = temp.path().join("library.zip");
    crate::potatovn::export(
        &path,
        std::slice::from_ref(&game),
        &Settings::default(),
        temp.path(),
    )
    .unwrap();
    let plan = read(Source::Potato, &path, &Settings::default(), &[]).unwrap();
    let mut settings = Settings {
        library_folders: vec![game.folder.clone()],
        ..Default::default()
    };
    let imported = merge(&plan, &mut settings, &[]);
    let scanned = crate::scanner::scan(&settings);
    assert_eq!(scanned.games[0].id, imported[0].id);
    assert_eq!(
        catalog::merge(scanned, &crate::ScanResult::default(), &settings)
            .games
            .len(),
        1
    );
}
