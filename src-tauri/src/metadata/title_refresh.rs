//! 标题保存后的后台刮削；不持锁联网，提交前重新校验标题、来源和目标内容。
use crate::{
    covers, data_sources,
    state::{AppState, find_game, persist_game, snapshot},
};
use galgear_core::{
    Game, GameEdit, catalog,
    game_details::{MetadataTarget, title_refresh},
    providers::Metadata,
};
use galgear_i18n::Message;
use std::collections::BTreeMap;
use tauri::{Emitter, State};

struct CoverUpdate {
    target: MetadataTarget,
    lookup: title_refresh::Lookup,
    expected_override: Option<String>,
    image: covers::SourceCover,
}

#[derive(Default)]
struct Refresh {
    updates: Vec<(MetadataTarget, title_refresh::Lookup, Metadata)>,
    cover: Option<CoverUpdate>,
    warning: Option<Message>,
}

#[tauri::command]
pub(crate) async fn refresh_title_metadata(
    id: String,
    title: String,
    request_id: Option<u32>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Option<Game>, Message> {
    let before = find_game(&state, &id)?;
    if before.title != title {
        return Ok(None);
    }
    let initial = snapshot(&state)?;
    let previous = initial.game_edits.get(&id).cloned().unwrap_or_default();
    let mut searches = BTreeMap::new();
    let mut fetched: BTreeMap<String, Result<Metadata, Message>> = BTreeMap::new();
    let mut refresh = Refresh::default();
    let mut cover_work = None;
    let mut started = false;
    for target in [MetadataTarget::Details, MetadataTarget::Tags] {
        let lookup = title_refresh::lookup(
            &before,
            &previous,
            target,
            &initial.source_preferences,
            &initial.metadata_provider,
        );
        if !searches.contains_key(&lookup.source) {
            let result = state
                .metadata_search
                .search(
                    &state.setup_service,
                    state.store.path.clone(),
                    lookup.source.clone(),
                    initial.fan_domain.clone(),
                    title.clone(),
                )
                .await;
            searches.insert(lookup.source.clone(), result);
        }
        let results = match &searches[&lookup.source] {
            Ok(results) => results,
            Err(error) => {
                refresh.warning = Some(error.clone());
                continue;
            }
        };
        let Some(url) = title_refresh::unique_changed_url(&lookup, results) else {
            continue;
        };
        // 资料词条优先；仅标签源匹配新词条时才取标签词条的封面，不混用多个来源。
        if cover_work.is_none() {
            cover_work = Some((target, lookup.clone(), url.to_owned()));
        }
        // 搜索不等于同步；首次确认新词条后才通知，提示持续到整个请求完成。
        if !started {
            started = true;
            if let Some(request_id) = request_id
                && let Err(error) = app.emit("title-refresh-started", request_id)
            {
                log::warn!("title_refresh_notice_failed: {error}");
            }
        }
        if !fetched.contains_key(url) {
            let result = fetch(&state, &lookup.source, &initial.fan_domain, url).await;
            fetched.insert(url.into(), result);
        }
        match &fetched[url] {
            Ok(metadata) => refresh.updates.push((target, lookup, metadata.clone())),
            Err(error) => refresh.warning = Some(error.clone()),
        }
    }
    if let Some((target, lookup, url)) = cover_work {
        match fetch_cover(&state, &lookup.source, &initial.fan_domain, &url).await {
            Ok(Some(image)) => {
                refresh.cover = Some(CoverUpdate {
                    target,
                    lookup,
                    expected_override: initial.cover_overrides.get(&id).cloned(),
                    image,
                })
            }
            Ok(None) => {}
            Err(error) => refresh.warning = Some(error),
        }
    }
    commit(&state, &before, &previous, refresh)
}

fn commit(
    state: &AppState,
    before: &Game,
    previous: &GameEdit,
    refresh: Refresh,
) -> Result<Option<Game>, Message> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut game = find_game(state, &before.id)?;
    catalog::apply_edits(&mut game, &settings);
    if game.title != before.title {
        return Ok(None);
    }
    let current = settings
        .game_edits
        .get(&before.id)
        .cloned()
        .unwrap_or_default();
    let mut edit = current.clone();
    let mut changed = false;
    for (target, lookup, metadata) in refresh.updates {
        if title_refresh::unchanged(before, previous, &game, &current, target)
            && lookup
                == title_refresh::lookup(
                    &game,
                    &current,
                    target,
                    &settings.source_preferences,
                    &settings.metadata_provider,
                )
        {
            edit = title_refresh::apply(&game, &edit, &lookup.source, metadata, target);
            changed = true;
        }
    }
    let cover = refresh.cover.filter(|cover| {
        // 新标题、来源或手动选图已改变时，下载只留下未被引用的缓存，不覆盖现状。
        game.cover == before.cover
            && settings.cover_overrides.get(&before.id) == cover.expected_override.as_ref()
            && title_refresh::unchanged(before, previous, &game, &current, cover.target)
            && cover.lookup
                == title_refresh::lookup(
                    &game,
                    &current,
                    cover.target,
                    &settings.source_preferences,
                    &settings.metadata_provider,
                )
    });
    if !changed && cover.is_none() && refresh.warning.is_none() {
        return Ok(None);
    }
    if changed || cover.is_some() {
        let mut next = settings.clone();
        if changed {
            next.game_edits.insert(before.id.clone(), edit);
        }
        if let Some(cover) = cover {
            next.cover_overrides
                .insert(before.id.clone(), cover.image.path);
            game.cover = Some(cover.image.data);
            game.cover_artwork = None;
        }
        state.store.save(&next)?;
        catalog::apply_edits(&mut game, &next);
        *settings = next;
    }
    game.metadata_warning = refresh.warning;
    // 与详情编辑相同的 settings → catalog 锁顺序；持久化期间不允许新标题插入。
    persist_game(state, game).map(Some)
}

async fn fetch_cover(
    state: &AppState,
    source: &str,
    domain: &str,
    url: &str,
) -> Result<Option<covers::SourceCover>, Message> {
    let credentials = data_sources::credentials_for(state, source).await?;
    let domain = domain.to_owned();
    let url = url.to_owned();
    let cache = state.store.path.with_file_name("covers");
    tauri::async_runtime::spawn_blocking(move || {
        let choices = galgear_core::data_sources::covers(&domain, &url, &credentials)?;
        // 与封面选择器使用相同列表及顺序；首张下载失败也不悄悄改选下一张。
        choices
            .first()
            .map(|cover| covers::download_source_cover(&cover.image_url, &cover.url, &cache))
            .transpose()
    })
    .await
    .map_err(|error| Message::detail("error.network", error))?
}

async fn fetch(
    state: &AppState,
    source: &str,
    domain: &str,
    url: &str,
) -> Result<Metadata, Message> {
    let credentials = data_sources::credentials_for(state, source).await?;
    let domain = domain.to_owned();
    let url = url.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        galgear_core::data_sources::metadata(&domain, &url, &credentials)
    })
    .await
    .map_err(|error| Message::detail("error.network", error))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, AppState, Game, GameEdit) {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::load(directory.path().join("data")).unwrap();
        let mut game =
            catalog::manual_game(&directory.path().join("game.exe"), "New title".into()).unwrap();
        game.tags = vec!["Old tag".into()];
        game.metadata = Some(Metadata {
            url: "https://vndb.org/v1".into(),
            description: "Old story".into(),
            ..Default::default()
        });
        let edit = GameEdit {
            title: Some(game.title.clone()),
            tags: game.tags.clone(),
            metadata: game.metadata.clone(),
            tag_source_url: Some("https://bgm.tv/subject/1".into()),
            setup_confirmed: true,
            ..Default::default()
        };
        state
            .settings
            .lock()
            .unwrap()
            .game_edits
            .insert(game.id.clone(), edit.clone());
        state.catalog.lock().unwrap().games.push(game.clone());
        (directory, state, game, edit)
    }

    fn with_cover(state: &AppState, game: &Game, edit: &GameEdit) -> Refresh {
        let path = state.store.path.with_file_name("downloaded.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, include_bytes!("../../../web/assets/2dfan.png")).unwrap();
        let lookup = title_refresh::lookup(
            game,
            edit,
            MetadataTarget::Details,
            &BTreeMap::new(),
            "nextmoe",
        );
        Refresh {
            updates: vec![(
                MetadataTarget::Details,
                lookup.clone(),
                Metadata {
                    url: "https://vndb.org/v2".into(),
                    description: "New story".into(),
                    ..Default::default()
                },
            )],
            cover: Some(CoverUpdate {
                target: MetadataTarget::Details,
                lookup,
                expected_override: None,
                image: covers::SourceCover {
                    data: galgear_core::scanner::poster_data(&path).unwrap(),
                    path: path.to_string_lossy().into_owned(),
                },
            }),
            warning: None,
        }
    }

    #[test]
    fn new_cover_and_metadata_are_saved_together_and_survive_reload() {
        let (_directory, state, mut game, edit) = fixture();
        game.cover = Some("old-cover".into());
        state.catalog.lock().unwrap().games[0] = game.clone();
        let refresh = with_cover(&state, &game, &edit);
        let path = refresh.cover.as_ref().unwrap().image.path.clone();
        let updated = commit(&state, &game, &edit, refresh).unwrap().unwrap();
        assert_ne!(updated.cover, game.cover);
        assert_eq!(updated.metadata.as_ref().unwrap().description, "New story");
        assert_eq!(updated.title, game.title);
        assert_eq!(state.store.load().unwrap().cover_overrides[&game.id], path);
        let reloaded = AppState::load(state.store.path.parent().unwrap().to_path_buf()).unwrap();
        let current = find_game(&reloaded, &game.id).unwrap();
        assert_eq!(current.cover, updated.cover);
        assert_eq!(current.metadata, updated.metadata);
    }

    #[test]
    fn manual_cover_change_during_download_is_preserved_while_details_update() {
        let (_directory, state, game, edit) = fixture();
        let refresh = with_cover(&state, &game, &edit);
        state.catalog.lock().unwrap().games[0].cover = Some("manual-cover".into());
        state
            .settings
            .lock()
            .unwrap()
            .cover_overrides
            .insert(game.id.clone(), "manual-path".into());
        let updated = commit(&state, &game, &edit, refresh).unwrap().unwrap();
        assert_eq!(updated.cover.as_deref(), Some("manual-cover"));
        assert_eq!(updated.metadata.as_ref().unwrap().description, "New story");
        assert_eq!(
            state.store.load().unwrap().cover_overrides[&game.id],
            "manual-path"
        );
    }

    #[test]
    fn failed_cover_download_keeps_old_cover_and_does_not_block_details() {
        let (_directory, state, mut game, edit) = fixture();
        game.cover = Some("old-cover".into());
        state.catalog.lock().unwrap().games[0] = game.clone();
        let mut refresh = with_cover(&state, &game, &edit);
        refresh.cover = None;
        refresh.warning = Some(Message::new("error.image"));
        let updated = commit(&state, &game, &edit, refresh).unwrap().unwrap();
        assert_eq!(updated.cover, game.cover);
        assert_eq!(updated.metadata.as_ref().unwrap().description, "New story");
        assert_eq!(updated.metadata_warning.unwrap().key, "error.image");
        assert!(state.store.load().unwrap().cover_overrides.is_empty());
    }

    #[test]
    fn new_title_or_association_discards_downloaded_cover() {
        for rename in [true, false] {
            let (_directory, state, game, edit) = fixture();
            let refresh = with_cover(&state, &game, &edit);
            {
                let mut settings = state.settings.lock().unwrap();
                let current = settings.game_edits.get_mut(&game.id).unwrap();
                if rename {
                    current.title = Some("Newest title".into());
                } else {
                    current.metadata.as_mut().unwrap().url = "https://vndb.org/v3".into();
                }
            }
            assert!(commit(&state, &game, &edit, refresh).unwrap().is_none());
            assert!(state.settings.lock().unwrap().cover_overrides.is_empty());
            assert_eq!(find_game(&state, &game.id).unwrap().cover, game.cover);
        }
    }

    #[test]
    fn successful_target_persists_even_when_other_source_fails() {
        let (_directory, state, game, edit) = fixture();
        let lookup = title_refresh::lookup(
            &game,
            &edit,
            MetadataTarget::Tags,
            &BTreeMap::new(),
            "nextmoe",
        );
        let updated = commit(
            &state,
            &game,
            &edit,
            Refresh {
                updates: vec![(
                    MetadataTarget::Tags,
                    lookup,
                    Metadata {
                        url: "https://bgm.tv/subject/2".into(),
                        tags: vec!["New tag".into()],
                        ..Default::default()
                    },
                )],
                warning: Some(Message::new("error.network")),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(updated.title, "New title");
        assert_eq!(updated.tags, ["New tag"]);
        assert_eq!(updated.metadata, game.metadata);
        assert_eq!(updated.metadata_warning.unwrap().key, "error.network");
        let saved = state.store.load().unwrap();
        assert_eq!(
            saved.game_edits[&game.id].tag_source_url.as_deref(),
            Some("https://bgm.tv/subject/2")
        );
        let cached = catalog::load(&state.store.path.with_file_name("catalog.json")).unwrap();
        assert_eq!(cached.games[0].tags, ["New tag"]);
    }

    #[test]
    fn title_or_association_changed_while_querying_discards_update() {
        let (_directory, state, game, edit) = fixture();
        let lookup = title_refresh::lookup(
            &game,
            &edit,
            MetadataTarget::Tags,
            &BTreeMap::new(),
            "nextmoe",
        );
        let updates = vec![(
            MetadataTarget::Tags,
            lookup,
            Metadata {
                url: "https://bgm.tv/subject/2".into(),
                tags: vec!["New tag".into()],
                ..Default::default()
            },
        )];
        state
            .settings
            .lock()
            .unwrap()
            .game_edits
            .get_mut(&game.id)
            .unwrap()
            .title = Some("Newest title".into());
        assert!(
            commit(
                &state,
                &game,
                &edit,
                Refresh {
                    updates: updates.clone(),
                    ..Default::default()
                }
            )
            .unwrap()
            .is_none()
        );
        {
            let mut settings = state.settings.lock().unwrap();
            let current = settings.game_edits.get_mut(&game.id).unwrap();
            current.title = Some(game.title.clone());
            current.tag_source_url = Some("https://bgm.tv/subject/3".into());
        }
        assert!(
            commit(
                &state,
                &game,
                &edit,
                Refresh {
                    updates,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(find_game(&state, &game.id).unwrap().tags, ["Old tag"]);
    }
}
