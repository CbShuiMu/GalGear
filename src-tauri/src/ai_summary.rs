//! Long AI work runs outside the UI thread, with one job per game.
use crate::{
    preferences,
    state::{AppState, find_game, snapshot},
};
use galgear_core::{Settings, ai_summary};
use galgear_i18n::Message;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, State};

fn report(app: &AppHandle, request_id: u32, game_id: &str, progress: &ai_summary::Progress) {
    // 窗口可能已关闭；进度通知失败不能中断正在保存的分析结果。
    let _ = app.emit(
        "ai-summary-progress",
        serde_json::json!({
            "request_id": request_id, "game_id": game_id, "progress": progress
        }),
    );
}

#[derive(Default)]
pub(crate) struct Jobs(Arc<Mutex<BTreeSet<String>>>);
pub(crate) struct Guard(Arc<Mutex<BTreeSet<String>>>, String);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.0.lock() {
            jobs.remove(&self.1);
        }
    }
}
impl Jobs {
    pub(crate) fn begin(&self, id: &str) -> Result<Guard, Message> {
        if !self
            .0
            .lock()
            .map_err(|_| Message::new("error.operation"))?
            .insert(id.into())
        {
            return Err(Message::new("summary.busy"));
        }
        Ok(Guard(self.0.clone(), id.into()))
    }
}
fn runtime_home(state: &AppState) -> Result<(PathBuf, crate::skill_package::Lease), Message> {
    let lease = state.skills.read()?;
    let home = galgear_core::ai::runtime::home()
        .filter(|home| galgear_core::ai::runtime::available(home))
        .ok_or_else(|| Message::new("summary.tools_missing"))?;
    Ok((home, lease))
}

#[tauri::command]
pub async fn set_ai_summary(
    id: String,
    enabled: bool,
    request_id: Option<u32>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    let game = find_game(&state, &id)?;
    let guard = state.summaries.begin(&id)?;
    if enabled {
        let settings = snapshot(&state)?;
        let config = state.store.path.clone();
        let (home, resources) = runtime_home(&state)?;
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = guard;
            let _resources = resources;
            ai_summary::prepare_with_progress(&game, &settings, &config, &home, &|progress| {
                report(&app, request_id.unwrap_or(0), &game.id, progress)
            })
        })
        .await
        .map_err(|_| Message::new("error.operation"))??;
    }
    preferences::update(&state, |settings| {
        settings.game_edits.entry(id).or_default().ai_summary = enabled;
        Ok(())
    })
}

#[tauri::command]
pub async fn summarize_game(
    id: String,
    request_id: Option<u32>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ai_summary::Summary, Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let config = state.store.path.clone();
    let (home, resources) = runtime_home(&state)?;
    let guard = state.summaries.begin(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let _resources = resources;
        ai_summary::summarize_with_progress(&game, &settings, &config, &home, &|progress| {
            report(&app, request_id.unwrap_or(0), &game.id, progress)
        })
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

async fn generate_route_map(id: String, state: State<'_, AppState>) -> Result<PathBuf, Message> {
    let (_home, resources) = runtime_home(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    if !settings
        .game_edits
        .get(&id)
        .is_some_and(|edit| edit.ai_summary)
    {
        return Err(Message::new("summary.disabled"));
    }
    let config = state.store.path.clone();
    let guard = state.summaries.begin(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let _resources = resources;
        ai_summary::route_map(&game, &settings, &config)
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[tauri::command]
pub async fn prepare_ai_route_map(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    generate_route_map(id, state).await.map(|_| ())
}

#[tauri::command]
pub async fn extract_game_media(
    id: String,
    kind: ai_summary::media::Kind,
    request_id: u32,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ai_summary::media::Extraction, Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let config = state.store.path.clone();
    let (home, resources) = runtime_home(&state)?;
    let guard = state.summaries.begin(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let _resources = resources;
        ai_summary::media::extract(&game, &settings, &config, &home, kind, &|progress| {
            report(&app, request_id, &game.id, progress)
        })
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_game_is_serialized_and_guard_release_allows_retry() {
        let jobs = Jobs::default();
        let first = jobs.begin("one").unwrap();
        assert!(jobs.begin("one").is_err());
        assert!(jobs.begin("two").is_ok());
        drop(first);
        assert!(jobs.begin("one").is_ok());
    }
}
