//! Cover selection only writes the explicit cover override, preserving metadata edits.
use crate::{
    data_sources, dialogs, preferences::update, state::AppState, state::find_game,
    state::persist_game, state::snapshot,
};
use galgear_core::{Game, data_sources::Cover, providers, scanner};
use galgear_i18n::{Message, tr};
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
pub async fn source_covers(
    id: String,
    source: String,
    query: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Cover>, Message> {
    galgear_core::data_sources::validate_source(&source)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let credentials = data_sources::credentials_for(&state, &source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let query = query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty());
        let linked = game.metadata.as_ref().filter(|metadata| {
            query.is_none()
                && galgear_core::data_sources::source_for_url(&metadata.url)
                    .is_ok_and(|(id, _)| id == source)
        });
        let url = if let Some(metadata) = linked {
            metadata.url.clone()
        } else {
            let matches = galgear_core::data_sources::search(
                &source,
                &settings.fan_domain,
                query.unwrap_or(&game.title),
                &credentials,
            )?;
            let Some(first) = matches.first() else {
                return Ok(Vec::new());
            };
            first.url.clone()
        };
        galgear_core::data_sources::covers(&settings.fan_domain, &url, &credentials)
    })
    .await
    .map_err(|error| Message::detail("error.network", error))?
}

#[tauri::command]
pub async fn select_source_cover(
    id: String,
    url: String,
    image_url: String,
    state: State<'_, AppState>,
) -> Result<Option<Game>, Message> {
    find_game(&state, &id)?;
    galgear_core::data_sources::source_for_url(&url)?;
    let path = state
        .store
        .path
        .with_file_name("covers")
        .join(format!("{id}.jpg"));
    let destination = path.clone();
    let data = tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let bytes = providers::cover_bytes(&image_url, &url)?;
        let parent = destination
            .parent()
            .ok_or_else(|| Message::new("error.cover_path"))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Message::detail("error.operation", error))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| Message::detail("error.operation", error))?;
        temp.write_all(&bytes)
            .map_err(|error| Message::detail("error.operation", error))?;
        temp.persist(&destination)
            .map_err(|error| Message::detail("error.operation", error))?;
        scanner::poster_data(&destination)
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))??;
    let mut game = find_game(&state, &id)?;
    super::preferences::update(&state, |settings| {
        settings
            .cover_overrides
            .insert(id, path.to_string_lossy().into_owned());
        Ok(())
    })?;
    game.cover = Some(data);
    persist_game(&state, game).map(Some)
}

#[tauri::command]
pub(crate) async fn choose_cover(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Game>, Message> {
    let mut game = find_game(&state, &id)?;
    let language = snapshot(&state)?.language;
    let role = format!("cover:{id}");
    let settings = snapshot(&state)?;
    let context = settings
        .dialog_directories
        .get(&role)
        .map(PathBuf::from)
        .into_iter()
        .chain([PathBuf::from(&game.folder)])
        .collect::<Vec<_>>();
    let start = dialogs::start(&state, &role, &context)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        dialogs::dialog(start)
            .set_title(tr(language, "dialog.cover"))
            .add_filter(
                tr(language, "dialog.images"),
                &["png", "jpg", "jpeg", "webp", "bmp"],
            )
            .pick_file()
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))?;
    let Some(path) = path else {
        return Ok(None);
    };
    dialogs::remember(&state, &role, &path)?;
    let cache_path = state
        .store
        .path
        .parent()
        .ok_or_else(|| Message::new("error.config_path"))?
        .join("covers")
        .join(format!("{id}.jpg"));
    let image_path = path.clone();
    let destination = cache_path.clone();
    let data = tauri::async_runtime::spawn_blocking(move || {
        scanner::cache_poster(&image_path, &destination)
    })
    .await
    .map_err(|e| Message::detail("error.operation", e))??;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| Message::new("error.config_state"))?;
        let mut next = settings.clone();
        next.cover_overrides
            .insert(id.clone(), cache_path.to_string_lossy().into_owned());
        state.store.save(&next)?;
        *settings = next;
    }
    game.cover = Some(data);
    persist_game(&state, game).map(Some)
}

pub(crate) fn prepare_cover(game: &mut Game, cache: &std::path::Path) -> Result<(), Message> {
    use base64::Engine;
    use sha2::Digest;
    use std::io::Write;
    if let Some(value) = &game.cover
        && let Some(encoded) = value.strip_prefix("data:image/jpeg;base64,")
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Message::detail("error.image", e))?;
        std::fs::create_dir_all(cache).map_err(|e| Message::detail("error.image", e))?;
        let key = format!("{:x}", sha2::Sha256::digest(&bytes));
        let path = cache.join(format!("{key}.jpg"));
        if !path.exists() {
            let mut file = tempfile::NamedTempFile::new_in(cache)
                .map_err(|e| Message::detail("error.image", e))?;
            file.write_all(&bytes)
                .map_err(|e| Message::detail("error.image", e))?;
            file.as_file()
                .sync_all()
                .map_err(|e| Message::detail("error.image", e))?;
            match file.persist_noclobber(&path) {
                Ok(_) => {}
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Message::detail("error.image", e)),
            }
        }
        game.cover = Some(format!("http://galgear-cover.localhost/{key}"));
    }
    galgear_core::cover_art::restore(game, cache);
    Ok(())
}

#[tauri::command]
pub(crate) async fn load_game_icons(
    state: State<'_, AppState>,
) -> Result<std::collections::BTreeMap<String, String>, Message> {
    let games = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?
        .games
        .clone();
    let cache = state.store.path.with_file_name("icons");
    tauri::async_runtime::spawn_blocking(move || {
        games
            .into_iter()
            .filter(|g| g.available)
            .filter_map(|g| {
                let path = galgear_core::launcher_icons::cached_icon(
                    std::path::Path::new(g.executable.as_deref()?),
                    &cache,
                )?;
                Some((
                    g.id,
                    format!(
                        "http://galgear-icon.localhost/{}",
                        path.file_stem()?.to_string_lossy()
                    ),
                ))
            })
            .collect()
    })
    .await
    .map_err(|_| Message::new("error.image"))
}

#[tauri::command]
pub(crate) async fn reset_cover(id: String, state: State<'_, AppState>) -> Result<Game, Message> {
    let mut game = find_game(&state, &id)?;
    update(&state, |s| {
        s.cover_overrides.remove(&id);
        Ok(())
    })?;
    game.cover = None;
    let url = game
        .metadata
        .as_ref()
        .map(|m| m.url.clone())
        .filter(|u| !u.is_empty());
    let game = persist_game(&state, game)?;
    if let Some(url) = url {
        crate::metadata::select_metadata(id, url, None, None, state).await
    } else {
        Ok(game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use galgear_core::catalog;
    #[test]
    fn replacing_a_cover_changes_its_url_and_preserves_previous_image() {
        use base64::Engine;
        let temp = tempfile::tempdir().unwrap();
        let mut game = catalog::manual_game(&temp.path().join("game.exe"), "Game".into()).unwrap();
        let data = |bytes: &[u8]| {
            format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        };
        game.cover = Some(data(b"first image"));
        prepare_cover(&mut game, temp.path()).unwrap();
        let first = game.cover.clone().unwrap();
        game.cover = Some(data(b"second image"));
        prepare_cover(&mut game, temp.path()).unwrap();
        let second = game.cover.clone().unwrap();
        assert_ne!(first, second);
        let file = temp
            .path()
            .join(format!("{}.jpg", first.rsplit('/').next().unwrap()));
        assert_eq!(std::fs::read(file).unwrap(), b"first image");
    }
}
