//! 标题保存后的后台刮削；不持锁联网，提交前重新校验标题、来源和目标内容。
use crate::{
    data_sources,
    state::{AppState, find_game, persist_game, snapshot},
};
use galgear_core::{
    Game, GameEdit, catalog,
    game_details::{MetadataTarget, title_refresh},
    providers::Metadata,
};
use galgear_i18n::Message;
use std::collections::BTreeMap;
use tauri::State;

#[tauri::command]
pub(crate) async fn refresh_title_metadata(
    id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<Option<Game>, Message> {
    let before = find_game(&state, &id)?;
    if before.title != title {
        return Ok(None);
    }
    let initial = snapshot(&state)?;
    let previous = initial.game_edits.get(&id).cloned().unwrap_or_default();
    let mut searches = BTreeMap::new();
    let mut fetched: BTreeMap<String, Result<Metadata, Message>> = BTreeMap::new();
    let mut updates = Vec::new();
    let mut warning = None;
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
                warning = Some(error.clone());
                continue;
            }
        };
        let Some(url) = title_refresh::unique_changed_url(&lookup, results) else {
            continue;
        };
        if !fetched.contains_key(url) {
            let result = fetch(&state, &lookup.source, &initial.fan_domain, url).await;
            fetched.insert(url.into(), result);
        }
        match &fetched[url] {
            Ok(metadata) => updates.push((target, lookup, metadata.clone())),
            Err(error) => warning = Some(error.clone()),
        }
    }
    commit(&state, &before, &previous, updates, warning)
}

fn commit(
    state: &AppState,
    before: &Game,
    previous: &GameEdit,
    updates: Vec<(MetadataTarget, title_refresh::Lookup, Metadata)>,
    warning: Option<Message>,
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
    for (target, lookup, metadata) in updates {
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
    if !changed && warning.is_none() {
        return Ok(None);
    }
    if changed {
        let mut next = settings.clone();
        next.game_edits.insert(before.id.clone(), edit);
        state.store.save(&next)?;
        catalog::apply_edits(&mut game, &next);
        *settings = next;
    }
    game.metadata_warning = warning;
    // 与详情编辑相同的 settings → catalog 锁顺序；持久化期间不允许新标题插入。
    persist_game(state, game).map(Some)
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
            vec![(
                MetadataTarget::Tags,
                lookup,
                Metadata {
                    url: "https://bgm.tv/subject/2".into(),
                    tags: vec!["New tag".into()],
                    ..Default::default()
                },
            )],
            Some(Message::new("error.network")),
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
            commit(&state, &game, &edit, updates.clone(), None)
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
            commit(&state, &game, &edit, updates, None)
                .unwrap()
                .is_none()
        );
        assert_eq!(find_game(&state, &game.id).unwrap().tags, ["Old tag"]);
    }
}
