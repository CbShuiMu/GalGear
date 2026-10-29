//! Native file selection and persistence for custom application backgrounds.
use crate::{dialogs, preferences::update, state::AppState, state::snapshot};
use galgear_core::{Settings, background};
use galgear_i18n::{Message, tr};
use tauri::State;

#[tauri::command]
pub(crate) async fn choose_background_image(
    state: State<'_, AppState>,
) -> Result<Option<Settings>, Message> {
    let language = snapshot(&state)?.language;
    let start = dialogs::start(&state, "background", &[])?;
    let selection = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .set_title(tr(language, "background.select"))
            .add_filter(
                tr(language, "dialog.images"),
                &["png", "jpg", "jpeg", "webp", "bmp"],
            )
            .pick_file()
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))?;
    let Some(selection) = selection else {
        return Ok(None);
    };
    let source = selection.clone();
    let cache = state.store.path.with_file_name("backgrounds");
    let image = tauri::async_runtime::spawn_blocking(move || background::import(&source, &cache))
        .await
        .map_err(|error| Message::detail("error.operation", error))??;
    dialogs::remember(&state, "background", &selection)?;
    update(&state, |settings| {
        settings.background_image = Some(image);
        Ok(())
    })
    .map(Some)
}

#[tauri::command]
pub(crate) fn clear_background_image(state: State<'_, AppState>) -> Result<Settings, Message> {
    update(&state, |settings| {
        settings.background_image = None;
        Ok(())
    })
}
