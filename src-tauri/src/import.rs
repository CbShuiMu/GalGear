//! Native file selection and confirmation of a read-only library import plan.
use crate::{
    dialogs,
    state::{AppState, begin_write, snapshot},
};
use galgear_core::{
    ScanResult, Settings, catalog,
    library_import::{self, Plan, Preview, Source},
    playtime,
};
use galgear_i18n::Message;
use serde::Serialize;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use tauri::Manager;

#[derive(Default)]
pub(crate) struct Imports {
    plan: Mutex<Option<(u64, Plan)>>,
    next: AtomicU64,
}
#[derive(Serialize)]
pub(crate) struct Prepared {
    token: u64,
    #[serde(flatten)]
    preview: Preview,
}
#[derive(Serialize)]
pub(crate) struct Imported {
    settings: Settings,
    catalog: ScanResult,
    added: usize,
    skipped: usize,
}

#[tauri::command]
pub(crate) async fn prepare_library_import(
    app: tauri::AppHandle,
    source: String,
) -> Result<Option<Prepared>, Message> {
    let source = Source::parse(&source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let imports = app.state::<Imports>();
        *imports
            .plan
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))? = None;
        let start = dialogs::start(&state, "library-import", &[])?;
        let extensions = if matches!(source, Source::Reina) {
            vec!["db", "sqlite", "sqlite3"]
        } else {
            vec!["zip"]
        };
        let Some(path) = dialogs::dialog(start)
            .add_filter(source.key(), &extensions)
            .pick_file()
        else {
            return Ok(None);
        };
        let settings = snapshot(&state)?;
        let existing = state
            .catalog
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?
            .games
            .clone();
        let plan = library_import::read(source, &path, &settings, &existing)?;
        dialogs::remember(&state, "library-import", &path)?;
        let token = imports.next.fetch_add(1, Ordering::Relaxed) + 1;
        let prepared = Prepared {
            token,
            preview: plan.preview.clone(),
        };
        *imports
            .plan
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))? = Some((token, plan));
        Ok(Some(prepared))
    })
    .await
    .map_err(|e| Message::detail("import.error", e))?
}

#[tauri::command]
pub(crate) async fn confirm_library_import(
    app: tauri::AppHandle,
    token: u64,
) -> Result<Imported, Message> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let imports = app.state::<Imports>();
        let _write = begin_write(&state)?;
        let mut pending = imports
            .plan
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?;
        let Some((stored, plan)) = pending.as_ref().filter(|(id, _)| *id == token) else {
            return Err(Message::new("import.expired"));
        };
        let _ = stored;
        let mut plan = plan.clone();
        for (game, _) in &mut plan.games {
            crate::covers::prepare_cover(
                game,
                &state.store.path.with_file_name("covers").join("rendered"),
            )?;
        }
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut records = state
            .playtime
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?;
        let mut list = state
            .catalog
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?;
        let mut next = settings.clone();
        let added = library_import::merge(&plan, &mut next, &list.games);
        // The single atomic settings write is authoritative. Imported manual records contain
        // metadata and time too, so cache failures are recoverable on the next startup.
        state.store.save(&next)?;
        *settings = next;
        for game in &added {
            records.insert(game.id.clone(), playtime::Record::from_game(game));
            list.games.push(game.clone());
        }
        if let Err(e) = catalog::save(&state.store.path.with_file_name("catalog.json"), &list) {
            list.warnings.push(e);
        }
        if let Err(e) = playtime::save(&state.store.path.with_file_name("playtime.json"), &records)
        {
            list.warnings.push(e);
        }
        let result = Imported {
            settings: settings.clone(),
            catalog: list.clone(),
            added: added.len(),
            skipped: plan.preview.skipped + plan.preview.added - added.len(),
        };
        *pending = None;
        Ok(result)
    })
    .await
    .map_err(|e| Message::detail("import.error", e))?
}
