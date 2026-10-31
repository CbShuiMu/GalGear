//! PotatoVN 的 JSON 数据交换包；不导出凭据、游戏文件或存档内容。
use crate::{Game, GameEdit, PlayStatus, Settings, cover_art};
use base64::Engine;
use galgear_i18n::Message;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, path::Path};
use zip::{ZipWriter, write::SimpleFileOptions};

fn error(error: impl std::fmt::Display) -> Message {
    Message::detail("export.error", error)
}

// Stable UUIDs let the source/category references resolve to the same game.
fn uuid(kind: &str, id: &str) -> String {
    let mut hash = Sha256::digest(format!("GalGear/{kind}/{id}")).to_vec();
    hash[6] = (hash[6] & 15) | 0x50;
    hash[8] = (hash[8] & 63) | 0x80;
    let hex = hash[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
fn property(value: impl serde::Serialize) -> Value {
    json!({"Value":value,"IsLock":false})
}
fn timestamp(value: Option<u64>) -> String {
    value
        .and_then(|n| i64::try_from(n).ok())
        .and_then(|n| time::OffsetDateTime::from_unix_timestamp(n).ok())
        .filter(|d| (1..=9999).contains(&d.year()))
        .map(|d| {
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                d.year(),
                u8::from(d.month()),
                d.day(),
                d.hour(),
                d.minute(),
                d.second()
            )
        })
        .unwrap_or_else(|| "0001-01-01T00:00:00".into())
}
fn release_date(value: &str) -> String {
    let parts = value.split(['-', '/', '.']).collect::<Vec<_>>();
    if parts.len() == 3
        && let (Ok(year), Ok(month), Ok(day)) =
            (parts[0].parse(), parts[1].parse::<u8>(), parts[2].parse())
        && let Ok(month) = time::Month::try_from(month)
        && let Ok(date) = time::Date::from_calendar_date(year, month, day)
        && (1..=9999).contains(&year)
    {
        return format!("{date}T00:00:00");
    }
    "0001-01-01T00:00:00".into()
}

fn game_record(game: &Game, edit: Option<&GameEdit>) -> Value {
    let metadata = game.metadata.clone().unwrap_or_default();
    let mut ids = vec![Value::Null; 9];
    let mut rss = 3;
    if let Ok(url) = reqwest::Url::parse(&metadata.url) {
        let path = url.path().trim_matches('/');
        let provider = match url.host_str().unwrap_or("") {
            "vndb.org" | "www.vndb.org" => Some((0, path.trim_start_matches('v'))),
            "bgm.tv" | "bangumi.tv" | "chii.in" => path.strip_prefix("subject/").map(|id| (1, id)),
            "www.ymgal.games" | "ymgal.games" => path.strip_prefix("ga").map(|id| (5, id)),
            _ => None,
        };
        if let Some((index, id)) = provider
            && !id.is_empty()
            && id.bytes().all(|c| c.is_ascii_digit())
        {
            ids[index] = id.into();
            rss = index;
        }
    }
    let status = match game.play_status {
        PlayStatus::Unmarked => 0,
        PlayStatus::Playing => 1,
        PlayStatus::Finished => 2,
        PlayStatus::Shelved => 3,
        PlayStatus::Abandoned => 4,
        PlayStatus::Wanted => 5,
    };
    let config = installation(game, edit);
    json!({"Uuid":uuid("game",&game.id),"Name":property(&game.title),"OriginalName":property(if metadata.title.is_empty(){&game.title}else{&metadata.title}),
        "Description":property(&metadata.description),"Developer":property(if metadata.brand.is_empty(){&metadata.developer}else{&metadata.brand}),
        "Engine":property(&game.engine),"ReleaseDate":property(release_date(&metadata.release_date)),"Tags":property(&game.tags),
        "Ids":ids,"RssType":rss,"PlayType":status,"TotalPlayTime":(game.playtime_seconds/60).min(i32::MAX as u64),
        "PlayedTime":{},"LastPlayTime":timestamp(game.last_played),"AddTime":timestamp(Some(game.modified)),
        "Path":game.folder,"ExePath":config["ExePath"],"ExeArguments":config["ExeArguments"],"RunInLocaleEmulator":config["RunInLocaleEmulator"],
        "SavePath":config["SavePath"],"PreferredInstallationId":uuid("entry",&game.id),"EnableMagpie":edit.is_some_and(|e|e.upscale),
        "ImagePath":property("ms-appx:///Assets/WindowIcon.ico")})
}

// Quote each argument with Windows' backslash-before-quote rules.
fn arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            let mut result = String::from("\"");
            let mut slashes = 0;
            for c in arg.chars() {
                if c == '\\' {
                    slashes += 1;
                    continue;
                }
                result.push_str(&"\\".repeat(if c == '"' { slashes * 2 + 1 } else { slashes }));
                result.push(c);
                slashes = 0;
            }
            result.push_str(&"\\".repeat(slashes * 2));
            result.push('"');
            result
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn installation(game: &Game, edit: Option<&GameEdit>) -> Value {
    let locale = edit.is_some_and(|e| e.launch_mode == "locale");
    let launcher = edit.and_then(|e| e.launcher.as_ref()).filter(|_| !locale);
    let mut args = Vec::new();
    if launcher.is_some() {
        args.extend(game.executable.iter().cloned());
    }
    if let Some(edit) = edit {
        args.extend(edit.arguments.iter().cloned());
    }
    json!({"ExePath":launcher.or(game.executable.as_ref()),"ExeArguments":arguments(&args),
        "RunInLocaleEmulator":edit.is_some_and(|e|e.launch_mode=="locale"),
        "SavePath":edit.and_then(|e|e.save_path.as_ref()).filter(|_|edit.is_some_and(|e|!e.save_path_auto))})
}
fn write_entry(zip: &mut ZipWriter<&mut fs::File>, name: &str, data: &[u8]) -> Result<(), Message> {
    zip.start_file(
        name,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )
    .map_err(error)?;
    zip.write_all(data).map_err(error)
}
fn write_json(zip: &mut ZipWriter<&mut fs::File>, key: &str, value: &Value) -> Result<(), Message> {
    write_entry(
        zip,
        &format!("data.{key}.json"),
        &serde_json::to_vec_pretty(value).map_err(error)?,
    )
}
fn cover_bytes(value: &str, cache: &Path) -> Result<Option<Vec<u8>>, Message> {
    if let Some(key) = cover_art::cover_key(value) {
        return match fs::read(cover_art::image_path(cache, key)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(error(e)),
        };
    }
    if value.starts_with("data:image/")
        && let Some((header, data)) = value.split_once(",")
        && header.ends_with(";base64")
    {
        return base64::engine::general_purpose::STANDARD
            .decode(data)
            .map(Some)
            .map_err(error);
    }
    Ok(None)
}

/// 原子生成可由 PotatoVN「导入数据」读取的 .pvnExport.zip 包。
/// `cover_cache` 指向 GalGear 的 covers/rendered；不读取或复制游戏目录内容。
pub fn export(
    destination: &Path,
    games: &[Game],
    settings: &Settings,
    cover_cache: &Path,
) -> Result<(), Message> {
    let mut temporary = tempfile::NamedTempFile::new_in(
        destination
            .parent()
            .ok_or_else(|| error("Missing export directory"))?,
    )
    .map_err(error)?;
    let mut zip = ZipWriter::new(temporary.as_file_mut());
    let mut records = Vec::new();
    let mut sources: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for game in games {
        let edit = settings.game_edits.get(&game.id);
        let mut record = game_record(game, edit);
        if let Some(bytes) = game
            .cover
            .as_deref()
            .map(|v| cover_bytes(v, cover_cache))
            .transpose()?
            .flatten()
        {
            let format = image::guess_format(&bytes).map_err(error)?;
            let extension = format.extensions_str().first().copied().unwrap_or("jpg");
            let name = format!("{}.{}", uuid("game", &game.id), extension);
            write_entry(&mut zip, &format!("Images/{name}"), &bytes)?;
            record["ImagePath"] = property(format!(".\\Images\\{name}"));
        }
        records.push(record);
        let root = if game.library.is_empty() {
            &game.folder
        } else {
            &game.library
        };
        sources.entry(root.clone()).or_default().push(json!({"Galgame":uuid("game",&game.id),"EntryId":uuid("entry",&game.id),"Path":game.folder,"LocalConfig":installation(game,edit)}));
    }
    let sources = sources.into_iter().map(|(path,games)|json!({"Id":uuid("source",&path),"SourceType":if path.is_empty(){3}else{1},"Name":if path.is_empty(){"GalGear".to_owned()}else{Path::new(&path).file_name().map_or(path.clone(),|n|n.to_string_lossy().into_owned())},"Path":path,"Galgames":games,"CheckOnStart":false,"ScanOnStart":false,"Detect":false,"SaveMetaBackup":false})).collect::<Vec<_>>();
    let categories = settings
        .collections
        .iter()
        .map(|collection| {
            let name = collection
                .parent
                .as_ref()
                .and_then(|id| settings.collections.iter().find(|c| &c.id == id))
                .map_or(collection.name.clone(), |parent| {
                    format!("{} / {}", parent.name, collection.name)
                });
            let members = games
                .iter()
                .filter(|g| collection.games.contains(&g.id))
                .map(|g| uuid("game", &g.id))
                .collect::<Vec<_>>();
            json!({"Id":uuid("category",&collection.id),"Name":name,"GalgamesX":members})
        })
        .collect::<Vec<_>>();
    write_json(&mut zip, "galgames", &json!(records))?;
    write_json(&mut zip, "galgameSources", &json!(sources))?;
    write_json(
        &mut zip,
        "categoryGroups",
        &json!([{"Id":uuid("category-group","collections"),"Name":"GalGear","Type":2,"Categories":categories}]),
    )?;
    write_json(
        &mut zip,
        "dataStatus",
        &json!({"LargerFileSeparateUpgraded":true,"GalgameSourceFormatUpgrade":true,"MetaBackupPerSourceUpgrade":true,"GalgameMultiInstallUpgrade":false,"CategoryGameIndexUpgrade":true,"CategoryAddLastPlayed":true,"CategoryAddWantToPlay":false,"GalgameDetectedSavePath":true,"GameLiteDbUpgrade":false,"CategoryLiteDbUpgrade":false,"SourceLiteDbUpgrade":false,"MultiStreamPageLiteDbUpgrade":false,"ImportGalgame":false,"ImportGalgameSource":false,"ImportCategory":false,"ImportStaff":true,"ImportPageSettings":true}),
    )?;
    zip.finish().map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary.persist(destination).map_err(error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn fixture() -> Game {
        serde_json::from_value(json!({"id":"game-one","title":"测试 Game","folder":"F:/Games/Test","library":"F:/Games","executable":"F:/Games/Test/game.exe","engine":"krkr","modified":1700000000,"cg_unlock_status":"","auto_unlock":serde_json::to_value(crate::auto_unlock::Summary::default()).unwrap(),"playtime_seconds":3660,"last_played":1700000000,"play_status":"finished","tags":["中文"],"metadata":{"url":"https://vndb.org/v123","release_date":"2024-02-29","brand":"制作组"}})).unwrap()
    }
    fn read(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Value {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    #[test]
    fn export_resolves_game_source_collection_and_cover_references() {
        let temp = tempfile::tempdir().unwrap();
        let mut game = fixture();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        game.cover = Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png.into_inner())
        ));
        let settings = Settings {
            collections: vec![crate::collections::Collection {
                id: "favorites".into(),
                name: "收藏".into(),
                games: [game.id.clone()].into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let path = temp.path().join("test.pvnExport.zip");
        export(&path, &[game], &settings, temp.path()).unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        let games = read(&mut zip, "data.galgames.json");
        let sources = read(&mut zip, "data.galgameSources.json");
        let groups = read(&mut zip, "data.categoryGroups.json");
        let status = read(&mut zip, "data.dataStatus.json");
        assert_eq!(games[0]["Name"]["Value"], "测试 Game");
        assert_eq!(games[0]["Ids"][0], "123");
        assert_eq!(games[0]["PlayType"], 2);
        assert_eq!(games[0]["TotalPlayTime"], 61);
        assert_eq!(sources[0]["Galgames"][0]["Galgame"], games[0]["Uuid"]);
        assert_eq!(
            sources[0]["Galgames"][0]["EntryId"],
            games[0]["PreferredInstallationId"]
        );
        assert_eq!(
            sources[0]["Galgames"][0]["LocalConfig"]["ExePath"],
            "F:/Games/Test/game.exe"
        );
        assert_eq!(groups[0]["Type"], 2);
        assert_eq!(groups[0]["Categories"][0]["GalgamesX"][0], games[0]["Uuid"]);
        let image = games[0]["ImagePath"]["Value"]
            .as_str()
            .unwrap()
            .trim_start_matches(".\\")
            .replace('\\', "/");
        assert!(zip.by_name(&image).unwrap().size() > 0);
        assert_eq!(status["GameLiteDbUpgrade"], false);
        assert_eq!(status["ImportGalgame"], false);
        assert!(
            zip.file_names()
                .all(|name| name.starts_with("data.") || name.starts_with("Images/"))
        );
    }
    #[test]
    fn failed_export_preserves_previous_file_and_invalid_dates_are_safe() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("previous.pvnExport.zip");
        fs::write(&path, b"previous").unwrap();
        let mut game = fixture();
        game.cover = Some("data:image/png;base64,broken".into());
        assert!(export(&path, &[game], &Settings::default(), temp.path()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"previous");
        assert_eq!(release_date("2024-02-30"), "0001-01-01T00:00:00");
        assert_eq!(
            arguments(&["a b".into(), "a\"b".into(), "c\\".into()]),
            r#""a b" "a\"b" "c\\""#
        );
    }
}
