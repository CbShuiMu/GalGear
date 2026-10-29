//! Detect completed play sessions independently of the desktop polling and backup transport.
use crate::runtime::Status;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Tracker {
    running: BTreeSet<String>,
}

impl Tracker {
    /// Emits one completion after a confirmed stop. Unknown observations keep a session open.
    pub fn observe(&mut self, statuses: &BTreeMap<String, Status>) -> Vec<String> {
        self.running.retain(|id| statuses.contains_key(id));
        let mut completed = Vec::new();
        for (id, status) in statuses {
            match status {
                Status::Running => {
                    self.running.insert(id.clone());
                }
                Status::Stopped if self.running.remove(id) => completed.push(id.clone()),
                _ => {}
            }
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(value: Status) -> BTreeMap<String, Status> {
        BTreeMap::from([("game".into(), value)])
    }
    #[test]
    fn startup_with_stopped_games_does_not_complete_a_session() {
        assert!(
            Tracker::default()
                .observe(&status(Status::Stopped))
                .is_empty()
        );
    }
    #[test]
    fn uncertain_runtime_does_not_lose_a_session_or_trigger_a_backup() {
        let mut tracker = Tracker::default();
        tracker.observe(&status(Status::Running));
        assert!(tracker.observe(&status(Status::Unknown)).is_empty());
        assert_eq!(tracker.observe(&status(Status::Stopped)), vec!["game"]);
    }
    #[test]
    fn each_session_completes_only_once() {
        let mut tracker = Tracker::default();
        for _ in 0..2 {
            tracker.observe(&status(Status::Running));
            assert_eq!(tracker.observe(&status(Status::Stopped)), vec!["game"]);
            assert!(tracker.observe(&status(Status::Stopped)).is_empty());
        }
    }
}
