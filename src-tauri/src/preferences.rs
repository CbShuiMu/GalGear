//! Application preferences and release checks.
use crate::{state::AppState, state::snapshot};
use galgear_core::Settings;
use galgear_i18n::{Language, Message};
use tauri::State;

pub(crate) fn update(
    state: &AppState,
    change: impl FnOnce(&mut Settings) -> Result<(), Message>,
) -> Result<Settings, Message> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    change(&mut next)?;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub(crate) fn get_preferences(state: State<'_, AppState>) -> Result<Settings, Message> {
    let _ = crate::tools::sync_magpie_hotkey(&state);
    snapshot(&state)
}

#[tauri::command]
pub(crate) fn set_patch_tab(
    visible: bool,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    update(&state, |settings| {
        settings.show_patches = visible;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn set_collection_tab(
    visible: bool,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    update(&state, |settings| {
        settings.show_collections = visible;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn set_theme(
    theme: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Settings, Message> {
    if !matches!(theme.as_str(), "light" | "dark") {
        return Err(Message::new("error.operation"));
    }
    let settings = update(&state, |settings| {
        settings.theme = theme;
        Ok(())
    })?;
    app.set_theme(Some(if settings.theme == "dark" {
        tauri::Theme::Dark
    } else {
        tauri::Theme::Light
    }));
    Ok(settings)
}

#[tauri::command]
pub(crate) fn set_color_theme(
    color_theme: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !matches!(color_theme.as_str(), "pink" | "white-pink" | "white") {
        return Err(Message::new("error.operation"));
    }
    update(&state, |settings| {
        settings.color_theme = color_theme;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn set_display(
    cover_scale: u32,
    font_size: u32,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !(50..=200).contains(&cover_scale) || ![14, 16, 18].contains(&font_size) {
        return Err(Message::new("error.operation"));
    }
    update(&state, |s| {
        s.cover_scale = cover_scale;
        s.font_size = font_size;
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn save_preferences(
    card_size: u32,
    sort_by: String,
    sort_descending: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !matches!(sort_by.as_str(), "title" | "recent" | "release") {
        return Err(Message::new("error.sort"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.card_size = card_size.clamp(160, 280);
    next.sort_by = sort_by;
    if let Some(descending) = sort_descending {
        next.sort_descending = Some(descending);
    }
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_language(
    language: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    let language = Language::from_code(&language).ok_or_else(|| Message::new("error.language"))?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.language = language;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_launch_behavior(
    behavior: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !["keep", "minimize", "exit"].contains(&behavior.as_str()) {
        return Err(Message::new("error.operation"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.launch_behavior = behavior;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_close_behavior(
    behavior: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !["exit", "tray"].contains(&behavior.as_str()) {
        return Err(Message::new("error.operation"));
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.close_behavior = behavior;
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub(crate) fn set_release_page(
    url: String,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    if !url.trim().is_empty() {
        let parsed = reqwest::Url::parse(url.trim()).map_err(|_| Message::new("error.site_url"))?;
        let parts: Vec<_> = parsed.path_segments().into_iter().flatten().collect();
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("github.com")
            || parts.len() != 3
            || parts[2] != "releases"
        {
            return Err(Message::new("error.site_url"));
        }
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.release_page = url.trim().into();
    state.store.save(&next)?;
    *settings = next.clone();
    Ok(next)
}
