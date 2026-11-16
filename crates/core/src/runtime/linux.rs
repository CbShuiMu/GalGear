//! 观察 umu 启动树继承的游戏标识。没有可验证进程时保留 Unknown，禁止不安全的存档写入。
use super::Status;
use std::{collections::BTreeMap, fs, path::Path};

fn running() -> std::io::Result<Vec<(u32, String)>> {
    let mut games = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        // 其他用户的环境不可读，退出中的进程也可能消失。这些情况不能证明游戏已停止。
        let Ok(environment) = fs::read(entry.path().join("environ")) else {
            continue;
        };
        if let Some(path) = environment
            .split(|byte| *byte == 0)
            .find_map(|item| item.strip_prefix(b"GALGEAR_GAME_EXECUTABLE="))
        {
            let status = fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
            if status
                .rsplit_once(") ")
                .is_some_and(|(_, rest)| !rest.starts_with('Z'))
            {
                games.push((pid, String::from_utf8_lossy(path).into_owned()));
            }
        }
    }
    Ok(games)
}

pub(super) fn process_ids(path: &Path) -> Result<Vec<u32>, String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let ids: Vec<_> = running()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|(_, game)| Path::new(game) == path)
        .map(|(pid, _)| pid)
        .collect();
    if ids.is_empty() {
        Err("No verified Proton process; external launches remain unknown".into())
    } else {
        Ok(ids)
    }
}

pub(super) fn detect_many(games: &[(String, Option<String>)]) -> BTreeMap<String, Status> {
    let running = running().unwrap_or_default();
    games
        .iter()
        .map(|(id, executable)| {
            let path = executable.as_deref().and_then(|p| fs::canonicalize(p).ok());
            let active = path.is_some_and(|p| running.iter().any(|(_, game)| Path::new(game) == p));
            (
                id.clone(),
                if active {
                    Status::Running
                } else {
                    Status::Unknown
                },
            )
        })
        .collect()
}
