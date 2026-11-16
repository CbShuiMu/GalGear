//! 媒体提取使用独立 Skill 任务；宿主只写工作目录，校验后由 core 发布到固定目录。
use super::{Stage, StageStatus, agent, hash, progress::Reporter, read};
use crate::{Game, Settings};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

/// 提取类别决定唯一的游戏目录直属输出位置，调用方不能指定任意路径。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Cg,
    Audio,
}

impl Kind {
    fn folder(self) -> &'static str {
        match self {
            Self::Cg => "CGs",
            Self::Audio => "audios",
        }
    }
}

/// 已经校验并发布的产物；warnings 非空表示仅完成了部分提取。
#[derive(Debug, Serialize)]
pub struct Extraction {
    pub game_id: String,
    pub kind: Kind,
    pub output: String,
    pub files: usize,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    kind: Kind,
    files: Vec<Member>,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct Member {
    path: String,
    size: u64,
    sha256: String,
}

fn invalid() -> Message {
    Message::new("media.invalid_output")
}
fn io(error: impl std::fmt::Display) -> Message {
    Message::detail("media.io", error)
}

fn checked(path: &Path) -> Result<fs::Metadata, Message> {
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        return Err(Message::new("media.unsafe_path"));
    }
    Ok(metadata)
}

fn directory(parent: &Path, name: &str) -> Result<PathBuf, Message> {
    checked(parent)?;
    let path = parent.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io(error)),
    }
    if !checked(&path)?.is_dir() || path.canonicalize().map_err(io)? != path {
        return Err(Message::new("media.unsafe_path"));
    }
    Ok(path)
}

fn members(
    directory: &Path,
    root: &Path,
    result: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), Message> {
    if depth > 64 || !checked(directory)?.is_dir() {
        return Err(invalid());
    }
    for entry in fs::read_dir(directory).map_err(io)? {
        let path = entry.map_err(io)?.path();
        let metadata = checked(&path)?;
        if metadata.is_dir() {
            members(&path, root, result, depth + 1)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(io)?
                .to_str()
                .ok_or_else(invalid)?
                .replace('\\', "/");
            if relative != "manifest.json" {
                result.insert(relative);
            }
            if result.len() > 200_000 {
                return Err(invalid());
            }
        } else {
            return Err(invalid());
        }
    }
    Ok(())
}

fn validate(delivery: &Path, kind: Kind) -> Result<Manifest, Message> {
    let mut actual = BTreeSet::new();
    members(delivery, delivery, &mut actual, 0)?;
    let manifest: Manifest =
        serde_json::from_slice(&read(&delivery.join("manifest.json"), 64 * 1024 * 1024)?)
            .map_err(io)?;
    if manifest.schema != 1
        || manifest.kind != kind
        || manifest.files.is_empty()
        || manifest.files.len() != actual.len()
        || manifest.warnings.len() > 128
    {
        return Err(invalid());
    }
    for member in &manifest.files {
        let relative = Path::new(&member.path);
        if member.path.contains(['\\', ':'])
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || !actual.remove(&member.path)
            || member.sha256.len() != 64
        {
            return Err(invalid());
        }
        let path = delivery.join(relative);
        let metadata = checked(&path)?;
        if metadata.len() != member.size || member.size == 0 || hash(&path)? != member.sha256 {
            return Err(invalid());
        }
    }
    Ok(manifest)
}

fn publish(root: &Path, work: &Path, kind: Kind) -> Result<(PathBuf, Manifest), Message> {
    // 宿主退出并撤销写权限后复核整棵目录；只重命名本次产物，不合并覆盖旧文件。
    checked(root)?;
    checked(work)?;
    let delivery = work.join("delivery");
    let manifest = validate(&delivery, kind)?;
    let output = directory(root, kind.folder())?.join(work.file_name().ok_or_else(invalid)?);
    if output.try_exists().map_err(io)? {
        return Err(invalid());
    }
    fs::rename(&delivery, &output).map_err(io)?;
    Ok((output, manifest))
}

/// 按当前 AI 配置调用内置 Skill 提取媒体，成功后保存到 CGs/audios 下的独立任务目录。
/// 无需开启游戏剧情分析，先校验 AI 配置；不读取存档、不执行游戏。
/// 同一游戏的任务互斥由桌面编排层负责。
pub fn extract(
    game: &Game,
    settings: &Settings,
    config: &Path,
    home: &Path,
    kind: Kind,
    notify: &dyn Fn(&super::Progress),
) -> Result<Extraction, Message> {
    let mut progress = Reporter::new(notify);
    progress.stage(Stage::Checking, StageStatus::Running);
    let (provider, model, token) = super::provider(settings, config)?;
    // 在解析前拒绝所有可见的链接祖先，避免配置路径把写入引向另一个目录。
    let original = Path::new(&game.folder);
    if !original.is_absolute()
        || original
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Message::new("media.unsafe_path"));
    }
    for path in original.ancestors() {
        checked(path)?;
    }
    let root = original.canonicalize().map_err(io)?;
    let base = directory(&root, super::FOLDER)?;
    // 保留失败工作目录与诊断，让失败不会伪装成一次空的成功导出。
    let work = tempfile::Builder::new()
        .prefix("media-")
        .tempdir_in(base)
        .map_err(io)?
        .keep();
    progress.stage(Stage::Checking, StageStatus::Completed);
    run_workers(
        home,
        &work,
        json!({
            "mode":"media_extract", "kind":kind, "provider":provider,"model":model,"token":token,
            "title":game.title,"language":settings.language.code(),"configured_executable":game.executable,
            "game":crate::config::display_path(&root),"save":crate::config::display_path(&root),
            "skill":crate::config::display_path(&home.join(crate::ai::runtime::SKILL_DIRECTORY)),
            "work":crate::config::display_path(&work),"runtime_limit_seconds":2400,"max_turns":100
        }),
        &mut progress,
    )?;
    progress.stage(Stage::Publishing, StageStatus::Running);
    let (output, manifest) = publish(&root, &work, kind)?;
    progress.stage(Stage::Publishing, StageStatus::Completed);
    Ok(Extraction {
        game_id: game.id.clone(),
        kind,
        output: crate::config::display_path(&output),
        files: manifest.files.len(),
        warnings: manifest.warnings,
    })
}

fn run_workers(
    home: &Path,
    work: &Path,
    config: serde_json::Value,
    progress: &mut Reporter<'_>,
) -> Result<(), Message> {
    let batch = json!({
        "mode":"media_batch","kind":config["kind"],"title":config["title"],"language":config["language"],
        "game":config["game"],"save":config["save"],"skill":config["skill"],"work":config["work"],"runtime_limit_seconds":21600
    });
    let plan = agent::analyze(home, work, config, progress)?;
    if plan["batch_plan"] != "media-plan.json" || plan["tested_samples"].as_u64().unwrap_or(0) == 0
    {
        return Err(invalid());
    }
    // 规划进程已经退出；批量进程只接收目录与已验证计划，完全不传模型配置或凭据。
    progress.stage(Stage::Sampling, StageStatus::Completed);
    progress.stage(Stage::Extracting, StageStatus::Running);
    agent::analyze(home, work, batch, progress)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_story_analysis_still_checks_ai_before_creating_work() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let game: Game = serde_json::from_value(json!({
            "id":"game", "title":"Fixture", "folder":root, "library":"",
            "engine":"unknown", "modified":0, "cg_unlock_status":"unknown",
            "auto_unlock":{"status":"unknown","engine":"unknown","cg_groups":0,"cg_variants":0,"scenes":0}
        }))
        .unwrap();
        let mut settings = Settings {
            ai_provider: "openai".into(),
            ..Settings::default()
        };
        let config = root.join("settings.json");
        for (model, expected) in [
            ("", "summary.ai_missing"),
            ("fixture-model", "ai.invalid_token"),
        ] {
            settings.ai_models.insert("openai".into(), model.into());
            for kind in [Kind::Cg, Kind::Audio] {
                let error = extract(&game, &settings, &config, &root, kind, &|_| {}).unwrap_err();
                assert_eq!(error.key, expected);
                assert!(!root.join(super::super::FOLDER).exists());
                assert!(!root.join("CGs").exists());
                assert!(!root.join("audios").exists());
            }
        }
    }

    /// 仅在用户明确要求真实游戏验证时显式运行；不写配置、不覆盖游戏、存档或旧输出。
    #[test]
    #[ignore = "requires explicitly selected local game, configured AI and bundled Windows runtime"]
    fn real_game_sample_and_background_batch() {
        let config =
            PathBuf::from(std::env::var_os("GALGEAR_MEDIA_TEST_CONFIG").expect("explicit config"));
        let id = std::env::var("GALGEAR_MEDIA_TEST_ID").expect("explicit game");
        let scope = std::env::var("GALGEAR_MEDIA_TEST_SCOPE").expect("explicit bounded test scope");
        let kind = match std::env::var("GALGEAR_MEDIA_TEST_KIND").as_deref() {
            Ok("cg") => Kind::Cg,
            Ok("audio") => Kind::Audio,
            _ => panic!("explicit kind"),
        };
        let home = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let settings = crate::config::ConfigStore::new(config.clone())
            .load()
            .unwrap();
        let game = crate::catalog::load(&config.with_file_name("catalog.json"))
            .unwrap()
            .games
            .into_iter()
            .find(|game| game.id == id)
            .unwrap();
        let root = Path::new(&game.folder).canonicalize().unwrap();
        let base = directory(&root, super::super::FOLDER).unwrap();
        let work = tempfile::Builder::new()
            .prefix("media-validation-")
            .tempdir_in(base)
            .unwrap()
            .keep();
        let (provider, model, token) = super::super::provider(&settings, &config).unwrap();
        let notify = |progress: &super::super::Progress| {
            println!(
                "MEDIA_PROGRESS {}",
                serde_json::to_string(progress).unwrap()
            )
        };
        let mut progress = Reporter::new(&notify);
        run_workers(&home,&work,json!({"mode":"media_extract","kind":kind,"title":game.title,
            "language":settings.language.code(),"game":crate::config::display_path(&root),"save":crate::config::display_path(&root),
            "work":crate::config::display_path(&work),"skill":crate::config::display_path(&home.join(crate::ai::runtime::SKILL_DIRECTORY)),
            "provider":provider,"model":model,"token":token,"request_scope":scope,"runtime_limit_seconds":1200,"max_turns":45}),&mut progress).unwrap();
        let (output, manifest) = publish(&root, &work, kind).unwrap();
        println!(
            "MEDIA_RESULT {}",
            json!({"title":game.title,"kind":kind,"files":manifest.files.len(),"output":crate::config::display_path(&output),"warnings":manifest.warnings,"work":crate::config::display_path(&work)})
        );
    }
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().canonicalize().unwrap().join("media-test");
        fs::create_dir_all(work.join("delivery/archive")).unwrap();
        let path = work.join("delivery/archive/cg.png");
        fs::write(&path, b"fixture").unwrap();
        super::super::write_json(&work.join("delivery/manifest.json"), &json!({"schema":1,"kind":"cg","warnings":[],"files":[{"path":"archive/cg.png","size":7,"sha256":hash(&path).unwrap()}]})).unwrap();
        (temp, work)
    }
    #[test]
    fn publishes_in_fixed_folder_and_preserves_existing_files() {
        let (temp, work) = fixture();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("CGs")).unwrap();
        fs::write(root.join("CGs/previous.png"), b"old").unwrap();
        let (path, manifest) = publish(&root, &work, Kind::Cg).unwrap();
        assert_eq!(path, root.join("CGs/media-test"));
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(fs::read(root.join("CGs/previous.png")).unwrap(), b"old");
        assert!(path.join("archive/cg.png").is_file());
    }
    #[test]
    fn rejects_changed_unlisted_and_wrong_kind_outputs() {
        let (_temp, work) = fixture();
        let delivery = work.join("delivery");
        assert!(validate(&delivery, Kind::Audio).is_err());
        fs::write(delivery.join("extra.exe"), b"extra").unwrap();
        assert!(validate(&delivery, Kind::Cg).is_err());
        fs::remove_file(delivery.join("extra.exe")).unwrap();
        fs::write(delivery.join("archive/cg.png"), b"changed").unwrap();
        assert!(validate(&delivery, Kind::Cg).is_err());
    }
    #[test]
    fn refuses_existing_run_directory() {
        let (temp, work) = fixture();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("CGs/media-test")).unwrap();
        assert!(publish(&root, &work, Kind::Cg).is_err());
        assert!(work.join("delivery/archive/cg.png").is_file());
    }
}
