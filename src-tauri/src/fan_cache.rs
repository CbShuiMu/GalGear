//! Status-only cache. Credentials remain in Windows Credential Manager.
use galgear_core::{credentials::FanCredentials, fan_account::Account};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub account: Account,
    pub updated_at: u64,
    pub error: Option<Message>,
    pub refreshing: bool,
    pub revision: u64,
}
#[derive(Serialize, Deserialize)]
struct Stored {
    scope: String,
    status: Snapshot,
    attempted_at: Option<u64>,
}
pub struct Cache {
    path: PathBuf,
    scope: String,
    status: Snapshot,
    attempted_at: Option<u64>,
    generation: u64,
}
pub fn scope(domain: &str, credentials: &FanCredentials) -> String {
    let mut hash = Sha256::new();
    for value in [
        domain.trim_end_matches('/'),
        &credentials.mode,
        &credentials.token,
        &credentials.cookie,
    ] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn next_minute(timestamp: u64) -> u64 {
    60 - timestamp % 60
}
impl Cache {
    pub fn load(path: PathBuf, scope: String) -> Self {
        let stored = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .filter(|r| r.scope == scope);
        let (mut status, attempted_at) = stored
            .map(|r| (r.status, r.attempted_at))
            .unwrap_or_default();
        status.refreshing = false;
        Self {
            path,
            scope,
            status,
            attempted_at,
            generation: 0,
        }
    }
    pub fn reset(&mut self, scope: String) {
        self.scope = scope;
        let revision = self.status.revision.wrapping_add(1);
        self.status = Snapshot {
            revision,
            ..Default::default()
        };
        self.attempted_at = None;
        self.generation = self.generation.wrapping_add(1);
        // A mismatched session also invalidates any file left behind if removal fails.
        let _ = std::fs::remove_file(&self.path);
    }
    pub fn ensure_scope(&mut self, scope: String) {
        if self.scope != scope {
            self.reset(scope);
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.status.clone()
    }
    pub fn initial(&self) -> bool {
        self.attempted_at.is_none()
    }
    pub fn due(&self, now: u64) -> bool {
        self.attempted_at.is_none_or(|t| t / 3600 != now / 3600)
    }
    pub fn begin(&mut self, now: u64, force: bool) -> Option<u64> {
        if !force && !self.due(now) {
            return None;
        }
        self.attempted_at = Some(now);
        self.status.refreshing = true;
        self.status.revision = self.status.revision.wrapping_add(1);
        Some(self.generation)
    }
    pub fn finish(&mut self, ticket: u64, result: Result<Snapshot, Message>) -> bool {
        if ticket != self.generation {
            return false;
        }
        let revision = self.status.revision.wrapping_add(1);
        match result {
            Ok(status) => self.status = status,
            Err(error) => self.status.error = Some(error),
        }
        self.status.refreshing = false;
        self.status.revision = revision;
        if self.persist().is_err() {
            self.status.error = Some(Message::new("auth.cache_write"));
        }
        true
    }
    fn persist(&self) -> Result<(), Box<dyn std::error::Error>> {
        let directory = self.path.parent().ok_or("Cache directory missing")?;
        std::fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&serde_json::to_vec(&Stored {
            scope: self.scope.clone(),
            status: self.status.clone(),
            attempted_at: self.attempted_at,
        })?)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hourly_boundary_and_error_backoff_survive_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("status.json");
        let mut cache = Cache::load(path.clone(), "session".into());
        assert!(cache.initial());
        let ticket = cache.begin(3599, false).unwrap();
        cache.finish(
            ticket,
            Ok(Snapshot {
                account: Account {
                    balance: Some(14),
                    ..Default::default()
                },
                updated_at: 3599,
                ..Default::default()
            }),
        );
        assert!(!cache.due(3599));
        assert!(cache.due(3600));
        let ticket = cache.begin(3600, false).unwrap();
        cache.finish(ticket, Err(Message::new("auth.network")));
        let cache = Cache::load(path, "session".into());
        assert_eq!(cache.snapshot().account.balance, Some(14));
        assert!(cache.snapshot().error.is_some());
        assert!(!cache.due(7199));
        assert!(cache.due(7200));
        assert_eq!(next_minute(3599), 1);
    }
    #[test]
    fn changed_credentials_reject_old_results_and_never_serialize_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("status.json");
        let credentials = FanCredentials {
            token: "fixture-secret-token".into(),
            cookie: "fixture-secret-cookie".into(),
            mode: "csrf".into(),
        };
        let old = scope("https://2dfmax.top", &credentials);
        let new = scope("https://2dfmax.top", &FanCredentials::default());
        assert_ne!(old, new);
        let mut cache = Cache::load(path.clone(), old.clone());
        let ticket = cache.begin(1, true).unwrap();
        cache.finish(ticket, Ok(Snapshot::default()));
        let data = std::fs::read_to_string(&path).unwrap();
        assert!(!data.contains("fixture-secret"));
        assert!(Cache::load(path.clone(), new.clone()).initial());
        cache.reset(new);
        assert!(!cache.finish(ticket, Ok(Snapshot::default())));
        assert!(!path.exists());
        cache.reset(old);
        let ticket = cache.begin(2, true).unwrap();
        assert!(cache.finish(ticket, Ok(Snapshot::default())));
    }
}
