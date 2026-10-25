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
use tauri::{Manager, State};

#[derive(Default)]
pub(crate) struct Jobs(Arc<Mutex<BTreeSet<String>>>);
struct Guard(Arc<Mutex<BTreeSet<String>>>, String);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.0.lock() {
            jobs.remove(&self.1);
        }
    }
}
impl Jobs {
    fn begin(&self, id: &str) -> Result<Guard, Message> {
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
fn home(app: &tauri::AppHandle) -> Result<PathBuf, Message> {
    let bundled = app
        .path()
        .resource_dir()
        .map_err(|_| Message::new("summary.tools_missing"))?;
    if bundled
        .join("skills/GalgameResourceExtract/tools/GARbro-cli/GARbro.Console.exe")
        .is_file()
    {
        return Ok(bundled);
    }
    let exe = std::env::current_exe().map_err(|_| Message::new("summary.tools_missing"))?;
    if let Some(parent) = exe.parent()
        && parent.join("skills").is_dir()
    {
        return Ok(parent.into());
    }
    #[cfg(debug_assertions)]
    {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
    }
    #[cfg(not(debug_assertions))]
    {
        // Cached reports and completed setup do not need extraction tools.
        // The decoder reports missing tools only when it actually needs them.
        Ok(bundled)
    }
}

#[tauri::command]
pub async fn set_ai_summary(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Settings, Message> {
    let game = find_game(&state, &id)?;
    let guard = state.summaries.begin(&id)?;
    if enabled {
        let settings = snapshot(&state)?;
        let config = state.store.path.clone();
        let home = home(&app)?;
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = guard;
            ai_summary::prepare(&game, &settings, &config, &home)
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
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<ai_summary::Summary, Message> {
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let config = state.store.path.clone();
    let home = home(&app)?;
    let guard = state.summaries.begin(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        ai_summary::summarize(&game, &settings, &config, &home)
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

async fn generate_route_map(id: String, state: State<'_, AppState>) -> Result<PathBuf, Message> {
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
pub async fn open_ai_route_map(id: String, state: State<'_, AppState>) -> Result<(), Message> {
    let path = generate_route_map(id, state).await?;
    tauri::async_runtime::spawn_blocking(move || crate::default_browser::open_svg(&path))
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
