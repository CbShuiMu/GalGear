//! 查询本地和远端 Skill 版本；资源下载与替换仅由更新卡片显式触发。
use crate::state::AppState;
use galgear_core::ai::{runtime, skills};
use galgear_i18n::Message;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::State;

#[derive(Default)]
struct Usage {
    readers: usize,
    updating: bool,
}

/// 资源替换与 AI 任务互斥；锁仅用于登记，后台工作由租约保证生命周期。
#[derive(Default)]
pub(crate) struct Access(Arc<Mutex<Usage>>);
pub(crate) struct Lease {
    access: Arc<Mutex<Usage>>,
    update: bool,
}
impl Access {
    pub(crate) fn read(&self) -> Result<Lease, Message> {
        self.acquire(false)
    }
    fn acquire(&self, update: bool) -> Result<Lease, Message> {
        let mut usage = self.0.lock().map_err(|_| Message::new("error.operation"))?;
        if usage.updating || update && usage.readers > 0 {
            return Err(Message::new("skills.busy"));
        }
        if update {
            usage.updating = true;
        } else {
            usage.readers += 1;
        }
        Ok(Lease {
            access: self.0.clone(),
            update,
        })
    }
    fn updating(&self) -> bool {
        self.0.lock().map_or(true, |usage| usage.updating)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut usage) = self.access.lock() {
            if self.update {
                usage.updating = false;
            } else {
                usage.readers -= 1;
            }
        }
    }
}

pub(crate) fn available(state: &AppState) -> bool {
    !state.skills.updating() && runtime::home().is_some_and(|home| runtime::available(&home))
}

#[derive(Serialize)]
pub(crate) struct Status {
    available: bool,
    skill: skills::Installed,
    error: Option<Message>,
}

#[tauri::command]
pub(crate) async fn get_ai_runtime(state: State<'_, AppState>) -> Result<Status, Message> {
    let updating = state.skills.updating();
    tauri::async_runtime::spawn_blocking(move || {
        let home = runtime::home();
        let (skill, error) = match home.as_deref().map(skills::status).transpose() {
            Ok(local) => (local.unwrap_or_default(), None),
            Err(error) => (skills::Installed::default(), Some(error)),
        };
        Status {
            available: !updating && home.is_some_and(|home| runtime::available(&home)),
            skill,
            error,
        }
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))
}

#[derive(Serialize)]
pub(crate) struct Update {
    #[serde(flatten)]
    version: skills::Updated,
    available: bool,
}

#[tauri::command]
pub(crate) async fn check_skill_version() -> Result<String, Message> {
    // 只读远端 commit，无需独占资源租约，后台检查不影响正在运行的 AI 任务。
    tauri::async_runtime::spawn_blocking(skills::latest_commit)
        .await
        .map_err(|e| Message::detail("skills.update_failed", e))?
}

#[tauri::command]
pub(crate) async fn check_skill_update(state: State<'_, AppState>) -> Result<Update, Message> {
    let lease = state.skills.acquire(true)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let home = runtime::home().ok_or_else(|| Message::new("summary.tools_missing"))?;
        let version = skills::sync(&home)?;
        Ok(Update {
            version,
            available: runtime::available(&home),
        })
    })
    .await
    .map_err(|e| Message::detail("skills.update_failed", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_protect_running_jobs_and_release_after_failure() {
        let access = Access::default();
        let first = access.read().unwrap();
        let second = access.read().unwrap();
        assert!(access.acquire(true).is_err());
        drop(first);
        assert!(access.acquire(true).is_err());
        drop(second);
        let update = access.acquire(true).unwrap();
        assert!(access.read().is_err());
        assert!(access.acquire(true).is_err());
        drop(update);
        assert!(access.read().is_ok());
        assert!(access.acquire(true).is_ok());
    }
}
