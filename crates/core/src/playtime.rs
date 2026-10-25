//! Persistent play records and elapsed time from confirmed runtime observations.
use crate::{Game, runtime::Status};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Record {
    pub playtime_seconds: u64,
    pub last_played: Option<u64>,
}

impl Record {
    pub fn from_game(game: &Game) -> Self {
        Self {
            playtime_seconds: game.playtime_seconds,
            last_played: game.last_played,
        }
    }

    pub fn apply(self, game: &mut Game) {
        game.playtime_seconds = self.playtime_seconds;
        game.last_played = self.last_played;
    }
}

pub type Records = BTreeMap<String, Record>;

pub fn load(path: &Path) -> Result<Records, Message> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| Message::detail("error.cache", e)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Records::new()),
        Err(error) => Err(Message::detail("error.cache", error)),
    }
}

pub fn save(path: &Path, records: &Records) -> Result<(), Message> {
    let parent = path
        .parent()
        .ok_or_else(|| Message::new("error.config_path"))?;
    fs::create_dir_all(parent).map_err(|e| Message::detail("error.cache", e))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| Message::detail("error.cache", e))?;
    serde_json::to_writer(&mut temporary, records)
        .map_err(|e| Message::detail("error.cache", e))?;
    temporary
        .flush()
        .map_err(|e| Message::detail("error.cache", e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| Message::detail("error.cache", e))?;
    temporary
        .persist(path)
        .map_err(|e| Message::detail("error.cache", e))?;
    Ok(())
}

/// Checkpoints use a monotonic clock; wall time is used only for the displayed date.
#[derive(Default)]
pub struct Tracker {
    checkpoints: BTreeMap<String, Instant>,
}

impl Tracker {
    pub fn observe(
        &mut self,
        games: &mut [Game],
        statuses: &BTreeMap<String, Status>,
        records: &mut Records,
        now: Instant,
        timestamp: u64,
    ) -> Records {
        let mut changed = Records::new();
        self.checkpoints.retain(|id, _| statuses.contains_key(id));
        for game in games {
            let record = records
                .entry(game.id.clone())
                .or_insert_with(|| Record::from_game(game));
            let before = *record;
            let status = statuses.get(&game.id).copied().unwrap_or(Status::Unknown);
            match status {
                Status::Running | Status::Stopped => {
                    if let Some(checkpoint) = self.checkpoints.get_mut(&game.id) {
                        let elapsed = now.saturating_duration_since(*checkpoint);
                        // Suspend or a stalled observer leaves the interval unconfirmed.
                        let seconds = if elapsed <= Duration::from_secs(30) {
                            elapsed.as_secs()
                        } else {
                            *checkpoint = now;
                            0
                        };
                        record.playtime_seconds = record.playtime_seconds.saturating_add(seconds);
                        *checkpoint += Duration::from_secs(seconds);
                        if seconds > 0 {
                            record.last_played = Some(timestamp);
                        }
                    }
                    if status == Status::Running {
                        self.checkpoints.entry(game.id.clone()).or_insert(now);
                        record.last_played = Some(timestamp);
                    } else {
                        self.checkpoints.remove(&game.id);
                    }
                }
                Status::Unknown => {
                    // An inaccessible process must not turn into unbounded phantom playtime.
                    self.checkpoints.remove(&game.id);
                }
            }
            if *record != before || *record != Record::from_game(game) {
                changed.insert(game.id.clone(), *record);
            }
            record.apply(game);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    fn game() -> Game {
        catalog::manual_game(Path::new("F:/games/game.exe"), "Game".into()).unwrap()
    }

    fn observe(
        tracker: &mut Tracker,
        game: &mut Game,
        records: &mut Records,
        status: Status,
        instant: Instant,
        timestamp: u64,
    ) {
        let statuses = BTreeMap::from([(game.id.clone(), status)]);
        tracker.observe(
            std::slice::from_mut(game),
            &statuses,
            records,
            instant,
            timestamp,
        );
    }

    #[test]
    fn running_checkpoints_preserve_fractional_seconds_without_double_counting() {
        let mut game = game();
        let mut records = Records::new();
        let mut tracker = Tracker::default();
        let start = Instant::now();
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Running,
            start,
            100,
        );
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Running,
            start + Duration::from_millis(1500),
            101,
        );
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Running,
            start + Duration::from_millis(2200),
            102,
        );
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Stopped,
            start + Duration::from_millis(3000),
            103,
        );
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Stopped,
            start + Duration::from_secs(50),
            150,
        );
        assert_eq!(
            Record::from_game(&game),
            Record {
                playtime_seconds: 3,
                last_played: Some(103)
            }
        );
    }

    #[test]
    fn uncertain_detection_discards_the_unconfirmed_interval() {
        let mut game = game();
        let mut records = Records::new();
        let mut tracker = Tracker::default();
        let start = Instant::now();
        for (seconds, status) in [
            (0, Status::Running),
            (3, Status::Running),
            (6, Status::Unknown),
            (90, Status::Running),
            (93, Status::Running),
        ] {
            observe(
                &mut tracker,
                &mut game,
                &mut records,
                status,
                start + Duration::from_secs(seconds),
                100 + seconds,
            );
        }
        assert_eq!(game.playtime_seconds, 6);
    }

    #[test]
    fn reopening_preserves_records_without_counting_time_while_the_app_was_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("playtime.json");
        let mut game = game();
        let mut records = Records::new();
        let mut tracker = Tracker::default();
        let start = Instant::now();
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Running,
            start,
            100,
        );
        observe(
            &mut tracker,
            &mut game,
            &mut records,
            Status::Running,
            start + Duration::from_secs(3),
            103,
        );
        save(&path, &records).unwrap();
        let mut reopened = load(&path).unwrap();
        let mut stale_game =
            catalog::manual_game(Path::new("F:/games/game.exe"), "Game".into()).unwrap();
        observe(
            &mut Tracker::default(),
            &mut stale_game,
            &mut reopened,
            Status::Running,
            start + Duration::from_secs(86400),
            86500,
        );
        assert_eq!(
            Record::from_game(&stale_game),
            Record {
                playtime_seconds: 3,
                last_played: Some(86500)
            }
        );
    }

    #[test]
    fn suspended_or_stalled_observer_does_not_charge_the_unobserved_gap() {
        let mut game = game();
        let mut records = Records::new();
        let mut tracker = Tracker::default();
        let start = Instant::now();
        for seconds in [0, 3, 3600, 3603] {
            observe(
                &mut tracker,
                &mut game,
                &mut records,
                Status::Running,
                start + Duration::from_secs(seconds),
                100 + seconds,
            );
        }
        assert_eq!(game.playtime_seconds, 6);
    }

    #[test]
    fn repeated_checkpoints_replace_the_existing_record_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("playtime.json");
        let mut records = Records::from([(
            "game".into(),
            Record {
                playtime_seconds: 3,
                last_played: Some(100),
            },
        )]);
        save(&path, &records).unwrap();
        records.insert(
            "game".into(),
            Record {
                playtime_seconds: 6,
                last_played: Some(103),
            },
        );
        save(&path, &records).unwrap();
        assert_eq!(load(&path).unwrap(), records);
    }
}
