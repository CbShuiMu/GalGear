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
    if !matches!(operation.as_str(), "add" | "detach" | "exclude" | "restore") {
        return Err(Message::new("error.operation"));
    }
    // Lock order matches scanning: settings before catalog. One save covers the batch.
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut list = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let mut next = settings.clone();
    let mut next_list = list.clone();
    let (succeeded, failed, unchanged) = apply_batch(
        &mut next,
        &mut next_list,
        &operation,
        ids,
        collection.as_deref(),
    );
    state.store.save(&next)?;
    *settings = next.clone();
    *list = next_list.clone();
    // Settings are authoritative; a cache failure must not misreport committed actions.
    let cache_warning =
        catalog::save(&state.store.path.with_file_name("catalog.json"), &next_list).err();
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
            "exclude" => {
                settings.excluded_entries.insert(id.clone(), game.clone());
                if !settings.excluded_games.contains(&id) {
                    settings.excluded_games.push(id.clone());
                }
                list.games.retain(|g| g.id != id);
                Ok(())
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
    fn exclusion_and_restore_preserve_identity_cover_membership_and_edits_across_restart() {
        let (mut settings, mut list) = fixture();
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
