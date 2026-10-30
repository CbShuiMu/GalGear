//! Collection editing and batch membership/exclusion commands.
use crate::{preferences, state::AppState, state::snapshot};
use galgear_core::{Game, ScanResult, Settings, catalog, collections};
use galgear_i18n::Message;
use serde::Serialize;
use std::collections::BTreeSet;
use tauri::State;

#[tauri::command]
pub fn delete_collections(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    preferences::update(&state, |settings| collections::delete_many(settings, &ids))
}

#[tauri::command]
pub fn edit_collection(
    operation: String,
    id: String,
    name: String,
    parent: Option<String>,
    state: State<'_, AppState>,
) -> Result<Settings, Message> {
    preferences::update(&state, |settings| {
        collections::edit(settings, &operation, &id, &name, parent.as_deref())
    })
}

#[derive(Serialize)]
pub struct Failure {
    id: String,
    title: String,
    error: Message,
}
#[derive(Serialize)]
pub struct BatchResult {
    settings: Settings,
    catalog: ScanResult,
    succeeded: Vec<String>,
    failed: Vec<Failure>,
    unchanged: usize,
    cache_warning: Option<Message>,
}

#[tauri::command]
pub fn batch_games(
    operation: String,
    ids: Vec<String>,
    collection: Option<String>,
    state: State<'_, AppState>,
) -> Result<BatchResult, Message> {
    change_games(&state, &operation, ids, collection.as_deref())
}

pub(crate) fn change_games(
    state: &AppState,
    operation: &str,
    ids: Vec<String>,
    collection: Option<&str>,
) -> Result<BatchResult, Message> {
    if !matches!(operation, "add" | "detach" | "exclude" | "restore") {
        return Err(Message::new("error.operation"));
    }
    // Scanning takes settings before catalog; the playtime tracker takes records
    // before catalog. Keep both orders when forgetting a game's play records.
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut records = if matches!(operation, "exclude" | "restore") {
        Some(state.playtime.lock().map_err(|_| Message::new("error.catalog_state"))?)
    } else {
        None
    };
    let mut list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let mut next = settings.clone();
    let mut next_list = list.clone();
    let (succeeded, failed, unchanged) = apply_batch(
        &mut next,
        &mut next_list,
        operation,
        ids,
        collection,
    );
    let forgotten: BTreeSet<_> = list.games.iter().chain(settings.excluded_entries.values())
        .filter(|game| !next_list.games.iter().any(|other| other.id == game.id)
            && !next.excluded_entries.contains_key(&game.id))
        .map(|game| game.id.clone()).collect();
    state.store.save(&next)?;
    *settings = next.clone();
    *list = next_list.clone();
    let mut playtime_warning = None;
    if matches!(operation, "exclude" | "restore") {
        state.scan.forget(&forgotten);
        if !forgotten.is_empty() && let Some(records) = records.as_mut() {
            records.retain(|id, _| !forgotten.contains(id));
            playtime_warning = galgear_core::playtime::save(
                &state.store.path.with_file_name("playtime.json"), records,
            ).err();
        }
    }
    // Settings are authoritative; a cache failure must not misreport committed actions.
    let cache_warning =
        catalog::save(&state.store.path.with_file_name("catalog.json"), &next_list).err().or(playtime_warning);
    Ok(BatchResult {
        settings: next,
        catalog: next_list,
        succeeded,
        failed,
        unchanged,
        cache_warning,
    })
}

fn apply_batch(
    settings: &mut Settings,
    list: &mut ScanResult,
    operation: &str,
    ids: Vec<String>,
    collection: Option<&str>,
) -> (Vec<String>, Vec<Failure>, usize) {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    let mut unchanged = 0;
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let game = if operation == "restore" {
            settings.excluded_entries.get(&id)
        } else {
            list.games.iter().find(|g| g.id == id)
        }
        .cloned();
        let Some(game) = game else {
            failed.push(Failure {
                title: id.clone(),
                id,
                error: Message::new("error.game_missing"),
            });
            continue;
        };
        let result = match operation {
            "add" if settings.collections.iter().any(|folder| {
                collection.is_some() && folder.parent.as_deref() == collection
            }) => Err(Message::new("folders.leaf_only")),
            "add" | "detach" => {
                if let Some(folder) = settings
                    .collections
                    .iter_mut()
                    .find(|f| Some(f.id.as_str()) == collection)
                {
                    let changed = if operation == "add" {
                        folder.games.insert(id.clone())
                    } else {
                        folder.games.remove(&id)
                    };
                    if !changed {
                        unchanged += 1;
                    }
                    Ok(())
                } else {
                    Err(Message::new("folders.missing"))
                }
            }
            "exclude" => catalog::remove_entry(settings, list, &id),
            "restore" if catalog::directory_missing(&game) => {
                catalog::forget_entry(settings, &id);
                list.games.retain(|game| game.id != id);
                Err(Message::new("error.game_missing"))
            }
            "restore" => {
                let mut restored = game.clone();
                settings.excluded_games.retain(|g| g != &id);
                catalog::apply_edits(&mut restored, settings);
                // Retain an authoritative record even if the derived catalog cannot be saved.
                settings
                    .excluded_entries
                    .insert(id.clone(), restored.clone());
                if !list.games.iter().any(|g| g.id == id) {
                    list.games.push(restored);
                }
                Ok(())
            }
            _ => Err(Message::new("error.operation")),
        };
        match result {
            Ok(()) => succeeded.push(id),
            Err(error) => failed.push(Failure {
                id,
                title: game.title,
                error,
            }),
        }
    }
    (succeeded, failed, unchanged)
}

#[tauri::command]
pub fn excluded_entries(state: State<'_, AppState>) -> Result<Vec<Game>, Message> {
    crate::state::purge_missing(&state)?;
    let settings = snapshot(&state)?;
    Ok(settings
        .excluded_entries
        .into_values()
        .filter(|g| settings.excluded_games.contains(&g.id))
        .collect())
}

#[tauri::command]
pub fn set_library_bounds(
    width: f64,
    height: f64,
    window: tauri::WebviewWindow,
) -> Result<(), Message> {
    if !width.is_finite()
        || !height.is_finite()
        || !(1000.0..=6000.0).contains(&width)
        || !(640.0..=2160.0).contains(&height)
    {
        return Err(Message::new("error.operation"));
    }
    window
        .set_min_size(Some(tauri::LogicalSize::new(width, height)))
        .map_err(|e| Message::detail("error.operation", e))?;
    let size = window
        .inner_size()
        .map_err(|e| Message::detail("error.operation", e))?;
    let scale = window
        .scale_factor()
        .map_err(|e| Message::detail("error.operation", e))?;
    let logical = size.to_logical::<f64>(scale);
    if logical.width < width || logical.height < height {
        window
            .set_size(tauri::LogicalSize::new(
                logical.width.max(width),
                logical.height.max(height),
            ))
            .map_err(|e| Message::detail("error.operation", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Settings, ScanResult) {
        let game = catalog::manual_game(std::path::Path::new("X:/example/game.exe"), "Game".into())
            .unwrap();
        let mut settings = Settings::default();
        collections::edit(&mut settings, "create", "", "A", None).unwrap();
        collections::edit(&mut settings, "create", "", "B", None).unwrap();
        (
            settings,
            ScanResult {
                games: vec![game],
                ..Default::default()
            },
        )
    }
    #[test]
    fn deleting_a_hidden_directory_while_restore_is_open_cannot_restore_or_keep_playtime() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState::load(temp.path().join("config")).unwrap();
        let folder = temp.path().join("Game");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("game.exe"), b"game").unwrap();
        let game = catalog::manual_game(&folder.join("game.exe"), "Game".into()).unwrap();
        state.catalog.lock().unwrap().games.push(game.clone());
        state.playtime.lock().unwrap().insert(game.id.clone(), Default::default());
        change_games(&state, "exclude", vec![game.id.clone()], None).unwrap();
        std::fs::remove_dir_all(folder).unwrap();
        let result = change_games(&state, "restore", vec![game.id], None).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(result.catalog.games.is_empty());
        assert!(result.settings.excluded_entries.is_empty());
        assert!(state.playtime.lock().unwrap().is_empty());
    }

    #[test]
    fn adding_duplicate_and_invalid_ids_reports_each_unique_result_and_keeps_multiple_memberships()
    {
        let (mut settings, mut list) = fixture();
        let id = list.games[0].id.clone();
        apply_batch(
            &mut settings,
            &mut list,
            "add",
            vec![id.clone()],
            Some("collection-1"),
        );
        let (done, fail, unchanged) = apply_batch(
            &mut settings,
            &mut list,
            "add",
            vec![id.clone(), id.clone(), "missing".into()],
            Some("collection-2"),
        );
        assert_eq!(done, vec![id.clone()]);
        assert_eq!(fail.len(), 1);
        assert_eq!(unchanged, 0);
        assert!(settings.collections.iter().all(|f| f.games.contains(&id)));
        let (_, _, unchanged) = apply_batch(
            &mut settings,
            &mut list,
            "add",
            vec![id],
            Some("collection-2"),
        );
        assert_eq!(unchanged, 1);
    }
    #[test]
    fn detach_affects_only_the_requested_collection() {
        let (mut settings, mut list) = fixture();
        let id = list.games[0].id.clone();
        for folder in &mut settings.collections {
            folder.games.insert(id.clone());
        }
        apply_batch(
            &mut settings,
            &mut list,
            "detach",
            vec![id.clone()],
            Some("collection-1"),
        );
        assert!(!settings.collections[0].games.contains(&id));
        assert!(settings.collections[1].games.contains(&id));
        assert_eq!(list.games.len(), 1);
    }
    #[test]
    fn adding_requires_a_leaf_collection_and_updates_after_children_are_deleted() {
        let (mut settings, mut list) = fixture();
        let id = list.games[0].id.clone();
        collections::edit(&mut settings, "create", "", "Child", Some("collection-1")).unwrap();
        let before = settings.clone();
        let (done, failed, _) = apply_batch(
            &mut settings, &mut list, "add", vec![id.clone()], Some("collection-1"),
        );
        assert!(done.is_empty());
        assert_eq!(failed[0].error.key, "folders.leaf_only");
        assert_eq!(settings, before);
        let (done, failed, _) = apply_batch(
            &mut settings, &mut list, "add", vec![id.clone()], Some("collection-3"),
        );
        assert_eq!(done, vec![id.clone()]);
        assert!(failed.is_empty());
        assert!(settings.collections[2].games.contains(&id));
        collections::edit(&mut settings, "delete", "collection-3", "", None).unwrap();
        let (done, failed, _) = apply_batch(
            &mut settings, &mut list, "add", vec![id.clone()], Some("collection-1"),
        );
        assert_eq!(done, vec![id.clone()]);
        assert!(failed.is_empty());
        assert!(settings.collections[0].games.contains(&id));
    }
    #[test]
    fn exclusion_and_restore_preserve_identity_cover_membership_and_edits_across_restart() {
        let (mut settings, mut list) = fixture();
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("game.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        list.games[0] = catalog::manual_game(&executable, "Game".into()).unwrap();
        let id = list.games[0].id.clone();
        list.games[0].cover = Some("cached-cover".into());
        settings.collections[0].games.insert(id.clone());
        settings.game_edits.entry(id.clone()).or_default().title = Some("Edited".into());
        apply_batch(&mut settings, &mut list, "exclude", vec![id.clone()], None);
        assert!(list.games.is_empty());
        let data = serde_json::to_vec(&settings).unwrap();
        let mut settings: Settings = serde_json::from_slice(&data).unwrap();
        collections::edit(&mut settings, "delete", "collection-2", "", None).unwrap();
        apply_batch(&mut settings, &mut list, "restore", vec![id.clone()], None);
        assert_eq!(list.games[0].id, id);
        assert_eq!(list.games[0].title, "Edited");
        assert_eq!(list.games[0].cover.as_deref(), Some("cached-cover"));
        assert!(settings.collections[0].games.contains(&id));
        assert!(settings.excluded_games.is_empty());
        assert_eq!(settings.collections.len(), 1);
        let recovered = catalog::merge(ScanResult::default(), &ScanResult::default(), &settings);
        assert_eq!(recovered.games[0].id, id);
        assert_eq!(recovered.games[0].cover.as_deref(), Some("cached-cover"));
    }
    #[test]
    fn forgetting_missing_games_clears_persistent_playtime_and_in_flight_scan_results() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState::load(temp.path().join("config")).unwrap();
        let game = catalog::manual_game(&temp.path().join("missing-folder").join("game.exe"), "Missing".into()).unwrap();
        state.catalog.lock().unwrap().games.push(game.clone());
        state.settings.lock().unwrap().game_edits.entry(game.id.clone()).or_default().setup_confirmed = true;
        state.playtime.lock().unwrap().insert(game.id.clone(), galgear_core::playtime::Record {
            playtime_seconds: 600, last_played: Some(1),
        });
        let result = change_games(&state, "exclude", vec![game.id.clone()], None).unwrap();
        assert_eq!(result.succeeded, vec![game.id.clone()]);
        assert!(result.failed.is_empty());
        assert!(result.settings.excluded_games.is_empty());
        assert!(result.catalog.games.is_empty());
        assert!(!result.settings.game_edits.contains_key(&game.id));
        assert!(state.playtime.lock().unwrap().is_empty());
        let mut stale_scan = vec![game.clone()];
        state.scan.retain_current(&mut stale_scan);
        assert!(stale_scan.is_empty());
        assert!(galgear_core::playtime::load(&state.store.path.with_file_name("playtime.json")).unwrap().is_empty());
        let reloaded = AppState::load(temp.path().join("config")).unwrap();
        assert!(reloaded.catalog.lock().unwrap().games.is_empty());
        assert!(reloaded.settings.lock().unwrap().excluded_entries.is_empty());
    }
    #[test]
    fn stale_collection_returns_failures_without_mutating_memberships() {
        let (mut settings, mut list) = fixture();
        let id = list.games[0].id.clone();
        let before = settings.clone();
        let (done, failed, _) =
            apply_batch(&mut settings, &mut list, "add", vec![id], Some("deleted"));
        assert!(done.is_empty());
        assert_eq!(failed[0].error.key, "folders.missing");
        assert_eq!(settings, before);
    }
}
