//! Native destination picker and an immutable snapshot for library export.
use crate::{
    dialogs,
    state::{AppState, snapshot},
};
use galgear_i18n::Message;
use tauri::Manager;

#[tauri::command]
pub(crate) async fn export_potatovn(app: tauri::AppHandle) -> Result<Option<String>, Message> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let start = dialogs::start(&state, "potatovn-export", &[])?;
        let Some(path) = dialogs::dialog(start)
            .add_filter("PotatoVN", &["zip"])
            .set_file_name("GalGear.pvnExport.zip")
            .save_file()
        else {
            return Ok(None);
        };
        if !path
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".pvnexport.zip")
        {
            return Err(Message::new("export.extension"));
        }
        let settings = snapshot(&state)?;
        let games = state
            .catalog
            .lock()
            .map_err(|_| Message::new("error.catalog_state"))?
            .games
            .clone();
        galgear_core::potatovn::export(
            &path,
            &games,
            &settings,
            &state.store.path.with_file_name("covers").join("rendered"),
        )?;
        dialogs::remember(&state, "potatovn-export", &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| Message::detail("export.error", e))?
}
