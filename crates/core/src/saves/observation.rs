//! 按用户发起的保存前后对比提供文件候选，时间相关性只作为待确认线索。
use super::{SaveReport, loose, validate_components};
use crate::{Game, config::display_path};
use galgear_i18n::Message;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

/// 有界、只读的本次诊断基线，应用退出后自动丢弃。
pub struct Observation {
    folder: String,
    roots: Vec<PathBuf>,
    files: BTreeMap<String, (u64, SystemTime)>,
    started: Instant,
    limited: bool,
}

impl Observation {
    /// 记录游戏及常用用户目录中的文件元数据，不读取存档正文。
    pub fn begin(game: &Game) -> Result<Self, Message> {
        validate_components(Path::new(&game.folder))?;
        let mut roots = vec![PathBuf::from(&game.folder)];
        roots.extend(super::user_roots().into_iter().map(|(_, path)| path));
        let (files, limited) = collect(&roots);
        Ok(Self {
            folder: game.folder.clone(),
            roots,
            files,
            started: Instant::now(),
            limited,
        })
    }

    /// 比较一小时内的基线，返回实际发生变化的文件集合供用户确认。
    pub fn finish(&self, game: &Game) -> Result<SaveReport, Message> {
        if game.folder != self.folder || self.started.elapsed().as_secs() > 3600 {
            return Err(Message::new("save.observation_expired"));
        }
        let (current, limited) = collect(&self.roots);
        let changed: Vec<_> = current
            .iter()
            .filter(|(path, stamp)| self.files.get(*path) != Some(*stamp))
            .map(|(path, _)| path.clone())
            .take(100)
            .collect();
        let mut report = loose::selected(&changed, "observed", "detected");
        report.limited = self.limited || limited || changed.len() == 100;
        Ok(report)
    }
}

fn collect(roots: &[PathBuf]) -> (BTreeMap<String, (u64, SystemTime)>, bool) {
    let mut files = BTreeMap::new();
    let mut limited = false;
    for root in roots {
        let mut queue = std::collections::VecDeque::from([(root.clone(), 0)]);
        let mut visited = 0;
        while let Some((folder, depth)) = queue.pop_front() {
            if validate_components(&folder).is_err() {
                limited = true;
                continue;
            }
            let Ok(entries) = fs::read_dir(folder) else {
                limited = true;
                continue;
            };
            for entry in entries {
                visited += 1;
                if visited > 10000 {
                    limited = true;
                    break;
                }
                let Ok(entry) = entry else {
                    limited = true;
                    continue;
                };
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with('.')
                    || ["cache", "logs", "node_modules", "patch", "crashdumps"]
                        .contains(&name.as_str())
                {
                    continue;
                }
                let path = entry.path();
                if validate_components(&path).is_err() {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    limited = true;
                    continue;
                };
                if metadata.is_dir() {
                    if depth < 4 {
                        queue.push_back((path, depth + 1));
                    } else {
                        limited = true;
                    }
                } else if metadata.is_file() && metadata.len() <= 64 * 1024 * 1024 {
                    let extension = path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if matches!(
                        extension.as_str(),
                        "" | "sav"
                            | "save"
                            | "dat"
                            | "ksd"
                            | "asd"
                            | "rpgsave"
                            | "rmmzsave"
                            | "bin"
                            | "ini"
                            | "json"
                            | "bmp"
                    ) && let Ok(modified) = metadata.modified()
                    {
                        files.insert(display_path(&path), (metadata.len(), modified));
                    }
                }
            }
            if visited > 10000 {
                break;
            }
        }
    }
    (files, limited)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changes_include_new_and_updated_files_but_not_unchanged_resources() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("SAVE.SAV"), b"before").unwrap();
        fs::write(temp.path().join("data.dat"), b"resource").unwrap();
        let game =
            crate::catalog::manual_game(&temp.path().join("game.exe"), "Example".into()).unwrap();
        let roots = vec![temp.path().to_path_buf()];
        let (files, limited) = collect(&roots);
        let baseline = Observation {
            folder: game.folder.clone(),
            roots,
            files,
            limited,
            started: Instant::now(),
        };
        fs::write(temp.path().join("SAVE.SAV"), b"after save").unwrap();
        fs::write(temp.path().join("persistent"), b"new flags").unwrap();
        let report = baseline.finish(&game).unwrap();
        assert_eq!(report.locations[0].files.len(), 2);
        assert!(
            report.locations[0]
                .files
                .iter()
                .all(|file| !file.ends_with("data.dat"))
        );
        assert_eq!(report.locations[0].confidence, "detected");
    }
}
