//! Cover selection only writes the explicit cover override, preserving metadata edits.
use crate::{
    data_sources, dialogs, preferences::update, state::AppState, state::find_game,
    state::persist_game, state::snapshot,
};
use galgear_core::{Game, data_sources::Cover, providers, scanner};
use galgear_i18n::{Message, tr};
use std::path::PathBuf;
use tauri::State;

/// 后台下载自动封面，并在落盘前确认原条目仍指向同一个游戏目录。
pub(crate) fn download_game_cover(
    state: &AppState,
    game: &Game,
    metadata: &providers::Metadata,
) -> Result<(String, String, Option<Message>), Message> {
    let bytes = providers::download_cover(metadata)?;
    let (path, warning) = store_game_cover(state, game, &bytes)?;
    Ok((
        scanner::poster_data(&path)?,
        galgear_core::config::display_path(&path),
        warning,
    ))
}

fn store_game_cover(
    state: &AppState,
    game: &Game,
    bytes: &[u8],
) -> Result<(PathBuf, Option<Message>), Message> {
    let list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let current = list
        .games
        .iter()
        .find(|current| current.id == game.id && current.folder == game.folder)
        .ok_or_else(|| Message::new("error.game_missing"))?;
    providers::store_downloaded_cover(current, bytes, &state.store.path.with_file_name("covers"))
}

/// 已下载并校验的封面缓存；内容寻址确保迟到请求不会覆盖当前封面文件。
pub(crate) struct SourceCover {
    pub(crate) data: String,
    pub(crate) path: String,
}

pub(crate) fn download_source_cover(
    image_url: &str,
    url: &str,
    cache: &std::path::Path,
) -> Result<SourceCover, Message> {
    cache_source_cover(&providers::cover_bytes(image_url, url)?, cache)
}

fn cache_source_cover(bytes: &[u8], cache: &std::path::Path) -> Result<SourceCover, Message> {
    use sha2::Digest;
    use std::io::Write;
    std::fs::create_dir_all(cache).map_err(|error| Message::detail("error.image", error))?;
    let path = cache.join(format!("source-{:x}.jpg", sha2::Sha256::digest(bytes)));
    let mut temp = tempfile::NamedTempFile::new_in(cache)
        .map_err(|error| Message::detail("error.image", error))?;
    temp.write_all(bytes)
        .map_err(|error| Message::detail("error.image", error))?;
    temp.as_file()
        .sync_all()
        .map_err(|error| Message::detail("error.image", error))?;
    let data = scanner::poster_data(temp.path())?;
    match temp.persist_noclobber(&path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(Message::detail("error.image", error)),
    }
    Ok(SourceCover {
        data,
        path: path.to_string_lossy().into_owned(),
    })
}

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
    let credentials = data_sources::credentials_for(&state, &source).await?;
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
    let cache = state.store.path.with_file_name("covers");
    let cover = tauri::async_runtime::spawn_blocking(move || {
        download_source_cover(&image_url, &url, &cache)
    })
    .await
    .map_err(|error| Message::detail("error.operation", error))??;
    let mut game = find_game(&state, &id)?;
    super::preferences::update(&state, |settings| {
        settings.cover_overrides.insert(id, cover.path);
        Ok(())
    })?;
    game.cover = Some(cover.data);
    game.cover_artwork = None;
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
pub(crate) async fn reset_cover(
    id: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Game, Message> {
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
        crate::metadata::select_metadata(id, url, None, None, app).await
    } else {
        Ok(game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_download_cannot_recreate_cover_after_entry_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState::load(temp.path().join("config")).unwrap();
        let game = galgear_core::catalog::manual_game(&temp.path().join("game.exe"), "Game".into())
            .unwrap();
        state.catalog.lock().unwrap().games.push(game.clone());
        let (cover, _) = store_game_cover(&state, &game, b"fixture").unwrap();
        crate::collections::change_games(&state, "exclude", vec![game.id.clone()], None).unwrap();

        let error = store_game_cover(&state, &game, b"late fixture").unwrap_err();
        assert_eq!(error.key, "error.game_missing");
        assert!(!cover.exists());
    }
    use galgear_core::catalog;
    #[test]
    fn downloaded_cover_cache_preserves_old_files_and_rejects_invalid_images() {
        let temp = tempfile::tempdir().unwrap();
        let first =
            cache_source_cover(include_bytes!("../../web/assets/2dfan.png"), temp.path()).unwrap();
        let original = std::fs::read(&first.path).unwrap();
        let second = cache_source_cover(
            include_bytes!("../../web/assets/erogamescape.png"),
            temp.path(),
        )
        .unwrap();
        assert_ne!(first.path, second.path);
        assert_ne!(first.data, second.data);
        assert_eq!(std::fs::read(&first.path).unwrap(), original);
        let repeat =
            cache_source_cover(include_bytes!("../../web/assets/2dfan.png"), temp.path()).unwrap();
        assert_eq!(repeat.path, first.path);
        assert!(cache_source_cover(b"not an image", temp.path()).is_err());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    }

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
