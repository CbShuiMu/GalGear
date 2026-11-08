//! 游戏会话的进程交接规则。系统采集在锁外进行，锁内只更新显式会话状态。
use super::{
    Status, executable_stem, matches_executable,
    processes::{Identity, Process, Snapshot, child_identity},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

mod confidence;
use confidence::{Confidence, MAX_SAMPLE_GAP};

const VERIFY_HOLD: Duration = Duration::from_secs(5);
const SINGLE_PROCESS_WAIT: Duration = Duration::from_secs(10);
const MAX_VERIFIED_RETARGETS: u8 = 3;
const HANDOFF: Duration = Duration::from_secs(3);

/// 应用实例共享的游戏运行观察器；用 PID 和创建时间区分进程，跟踪汉化启动器交接。
#[derive(Default)]
pub struct Monitor {
    sessions: Mutex<BTreeMap<PathBuf, Session>>,
}

impl Monitor {
    /// 在 spawn 后立即登记启动句柄。即使启动器已退出，也保留其身份以寻找后继进程。
    pub fn launched(&self, executable: &Path, child: &std::process::Child) -> Result<(), String> {
        let path = std::fs::canonicalize(executable).map_err(|e| e.to_string())?;
        let identity = child_identity(child)?;
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(path, Session::new(identity, Instant::now()));
        Ok(())
    }

    /// 检查一个游戏。访问失败返回 Unknown，不能作为计时或允许写入的依据。
    pub fn detect(&self, executable: Option<&str>) -> Status {
        self.detect_many(&[(String::new(), executable.map(str::to_owned))])
            .get("")
            .copied()
            .unwrap_or(Status::Unknown)
    }

    /// 整个游戏库共用一份进程及窗口快照；与单个游戏检查使用同一组会话。
    pub fn detect_many(&self, games: &[(String, Option<String>)]) -> BTreeMap<String, Status> {
        let paths: Vec<_> = games
            .iter()
            .map(|(id, path)| {
                (
                    id,
                    path.as_ref().and_then(|p| std::fs::canonicalize(p).ok()),
                )
            })
            .collect();
        let snapshot = Snapshot::capture();
        let mut sessions = self.sessions.lock();
        paths
            .into_iter()
            .map(|(id, path)| {
                let status = match (path, &snapshot, &mut sessions) {
                    (Some(path), Ok(snapshot), Ok(sessions)) => observe(sessions, &path, snapshot),
                    _ => Status::Unknown,
                };
                (id.clone(), status)
            })
            .collect()
    }
}

struct Session {
    root: Identity,
    target: Identity,
    family: BTreeSet<Identity>,
    retired: BTreeSet<Identity>,
    confidence: BTreeMap<Identity, Confidence>,
    challenge: Option<(Identity, Instant)>,
    verified_retargets: u8,
    missing_since: Option<Instant>,
    observed: Instant,
    status: Status,
}

impl Session {
    fn new(root: Identity, now: Instant) -> Self {
        Self {
            root,
            target: root,
            family: BTreeSet::from([root]),
            retired: BTreeSet::new(),
            confidence: BTreeMap::new(),
            challenge: None,
            verified_retargets: 0,
            missing_since: None,
            observed: now,
            status: Status::Running,
        }
    }

    fn related(&self, process: &Process, snapshot: &Snapshot) -> bool {
        let Some(identity) = process.identity else {
            return false;
        };
        self.family.iter().any(|parent| {
            parent.pid == process.parent && parent.created <= identity.created
                // 父 PID 已重用时，不把新进程的孩子纳入旧游戏会话。
                && snapshot.processes.get(&parent.pid).is_none_or(|p| p.identity == Some(*parent))
        })
    }

    fn observe(&mut self, path: &Path, snapshot: &Snapshot) -> Status {
        // 并发检查可能后完成较旧的快照；不能覆盖刚登记的启动或较新的观察。
        if snapshot.captured < self.observed {
            return self.status;
        }
        if snapshot.captured.saturating_duration_since(self.observed) > MAX_SAMPLE_GAP {
            self.challenge = None;
        }
        self.observed = snapshot.captured;
        if self.status == Status::Stopped {
            return Status::Stopped;
        }
        loop {
            let descendants: Vec<_> = snapshot
                .processes
                .values()
                .filter(|p| self.related(p, snapshot))
                .filter_map(|p| p.identity)
                .filter(|id| !self.family.contains(id))
                .collect();
            if descendants.is_empty() {
                break;
            }
            self.family.extend(descendants);
        }
        self.update_confidence(path, snapshot);
        let current = snapshot.processes.get(&self.target.pid);
        let target_alive = current.is_some_and(|p| p.identity == Some(self.target));
        // 身份无法查询时不换绑，防止一次权限失败让当前目标失去保护。
        if current.is_none_or(|p| p.identity.is_some()) {
            let candidates = self.candidates(snapshot);
            let successor = if target_alive {
                self.verify_challenger(&candidates, snapshot.captured)
            } else {
                self.challenge = None;
                self.successor(&candidates, snapshot.captured)
            };
            if let Some(identity) = successor {
                self.retired.insert(self.target);
                self.target = identity;
                self.family.insert(identity);
                self.challenge = None;
                if target_alive {
                    self.verified_retargets += 1;
                }
            }
        }
        if let Some(process) = snapshot.processes.get(&self.target.pid) {
            if process.identity == Some(self.target) {
                self.missing_since = None;
                self.status = Status::Running;
                return self.status;
            }
            if process.identity.is_none() {
                self.status = Status::Unknown;
                return self.status;
            }
        }
        // 不可访问的后继进程保留未知状态，不能提前备份或允许覆盖游戏文件。
        if snapshot.processes.values().any(|p| {
            p.identity.is_none() && self.family.iter().any(|parent| parent.pid == p.parent)
        }) {
            self.status = Status::Unknown;
            return self.status;
        }
        // 多个窗口尚无明确领先者，或加载进程还没有窗口时，保持未知并继续验证。
        if !self.candidates(snapshot).is_empty() {
            self.missing_since = None;
            self.status = Status::Unknown;
            return self.status;
        }
        let missing = self.missing_since.get_or_insert(snapshot.captured);
        self.status = if snapshot.captured.saturating_duration_since(*missing) < HANDOFF {
            Status::Unknown
        } else {
            Status::Stopped
        };
        self.status
    }

    fn update_confidence(&mut self, path: &Path, snapshot: &Snapshot) {
        self.confidence.retain(|id, _| {
            snapshot
                .processes
                .get(&id.pid)
                .is_some_and(|p| p.identity == Some(*id))
        });
        for process in snapshot.processes.values() {
            let Some(id) = process.identity else {
                continue;
            };
            // Steam 等中间层可能不保留父子链，配置的游戏映像本身仍是强关联。
            let matches_game = id.created >= self.root.created
                && process
                    .image
                    .as_ref()
                    .is_some_and(|image| matches_executable(image, path));
            if matches_game {
                self.family.insert(id);
            }
            let related = self.family.contains(&id);
            if id != self.target
                && (id.created < self.root.created || (!related && !same_directory(process, path)))
            {
                continue;
            }
            let relation = if related { 1.0 } else { 0.5 };
            let evidence = 0.35 * f64::from(process.visible)
                + 0.40 * f64::from(process.foreground)
                + 0.25 * relation;
            self.confidence
                .entry(id)
                .or_insert_with(|| Confidence::new(snapshot.captured))
                .observe(evidence, snapshot.captured);
        }
    }

    fn candidates(&self, snapshot: &Snapshot) -> Vec<Candidate> {
        let mut candidates: Vec<_> = snapshot
            .processes
            .values()
            .filter_map(|process| {
                let id = process.identity?;
                let evidence = self.confidence.get(&id)?;
                if id == self.root
                    || id == self.target
                    || id.created < self.target.created
                    || self.retired.contains(&id)
                {
                    return None;
                }
                // 当前目标被观察得更晚时，不返回先前已存在的设置窗。
                if self
                    .confidence
                    .get(&self.target)
                    .is_some_and(|target| evidence.first_seen < target.first_seen)
                {
                    return None;
                }
                let related = self.family.contains(&id);
                let loading = self.target == self.root && related && process.image.is_some();
                if !(process.visible || loading) {
                    return None;
                }
                Some(Candidate {
                    id,
                    score: evidence.score,
                    first_seen: evidence.first_seen,
                    visible: process.visible,
                    related,
                })
            })
            .collect();
        candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        candidates
    }

    fn successor(&self, candidates: &[Candidate], now: Instant) -> Option<Identity> {
        let mut windows = candidates.iter().filter(|candidate| candidate.visible);
        if let Some(window) = windows.next()
            && windows.next().is_none()
            && window.related
        {
            // 退出后的唯一关联窗口允许快速接力，无窗口的中间层不与它竞争。
            return Some(window.id);
        }
        let first = candidates.first()?;
        if candidates.len() == 1 && !first.visible {
            return self
                .confidence
                .get(&first.id)?
                .stable_for(now, SINGLE_PROCESS_WAIT)
                .then_some(first.id);
        }
        let margin = candidates
            .get(1)
            .map_or(first.score, |second| first.score - second.score);
        (first.visible
            && first.score >= 0.6
            && margin >= 0.25
            && now.saturating_duration_since(first.first_seen) >= HANDOFF)
            .then_some(first.id)
    }

    fn verify_challenger(&mut self, candidates: &[Candidate], now: Instant) -> Option<Identity> {
        let target_score = self
            .confidence
            .get(&self.target)
            .map_or(0.0, |evidence| evidence.score);
        let qualified = candidates.first().filter(|candidate| {
            candidate.visible
                && candidate.score >= 0.6
                && candidate.score - target_score >= 0.3
                && candidates
                    .get(1)
                    .is_none_or(|second| candidate.score - second.score >= 0.25)
                && self.verified_retargets < MAX_VERIFIED_RETARGETS
        });
        let Some(candidate) = qualified else {
            self.challenge = None;
            return None;
        };
        match self.challenge {
            Some((id, since)) if id == candidate.id => {
                (now.saturating_duration_since(since) >= VERIFY_HOLD).then_some(id)
            }
            _ => {
                self.challenge = Some((candidate.id, now));
                None
            }
        }
    }
}

struct Candidate {
    id: Identity,
    score: f64,
    first_seen: Instant,
    visible: bool,
    related: bool,
}

fn same_directory(process: &Process, executable: &Path) -> bool {
    process
        .image
        .as_ref()
        .and_then(|p| p.parent())
        .zip(executable.parent())
        .is_some_and(|(a, b)| {
            a.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .eq_ignore_ascii_case(b.to_string_lossy().trim_start_matches(r"\\?\"))
        })
}

fn observe(sessions: &mut BTreeMap<PathBuf, Session>, path: &Path, snapshot: &Snapshot) -> Status {
    if let Some(session) = sessions.get(path)
        && snapshot.captured < session.observed
    {
        return session.status;
    }
    let exact = snapshot.processes.values().filter(|p| {
        p.image
            .as_ref()
            .is_some_and(|image| matches_executable(image, path))
    });
    let next = exact.clone().filter_map(|p| p.identity).find(|id| {
        sessions
            .get(path)
            .is_none_or(|session| session.status == Status::Stopped && !session.family.contains(id))
    });
    if let Some(root) = next {
        sessions.insert(path.to_path_buf(), Session::new(root, snapshot.captured));
    }
    let name = executable_stem(path);
    let uncertain = snapshot
        .processes
        .values()
        .any(|p| p.image.is_none() && executable_stem(Path::new(&p.name)) == name);
    if let Some(session) = sessions.get_mut(path) {
        let status = session.observe(path, snapshot);
        return if status == Status::Stopped && uncertain {
            Status::Unknown
        } else {
            status
        };
    }
    if exact.count() > 0 || uncertain {
        Status::Unknown
    } else {
        Status::Stopped
    }
}

#[cfg(test)]
mod tests;
