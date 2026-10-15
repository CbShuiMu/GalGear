//! Logical collections reference stable game identities, never disk directories.
use crate::{Settings, tags};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub games: BTreeSet<String>,
}

pub fn descendants(settings: &Settings, id: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::from([id.to_owned()]);
    let mut pending = vec![id];
    while let Some(parent) = pending.pop() {
        for folder in &settings.collections {
            if folder.parent.as_deref() == Some(parent) && found.insert(folder.id.clone()) {
                pending.push(&folder.id);
            }
        }
    }
    found
}

/// Remove all selected subtrees together, including overlapping selections.
pub fn delete_many(settings: &mut Settings, ids: &[String]) -> Result<(), Message> {
    if ids.is_empty()
        || ids
            .iter()
            .any(|id| !settings.collections.iter().any(|f| &f.id == id))
    {
        return Err(Message::new("folders.missing"));
    }
    let removed: BTreeSet<_> = ids
        .iter()
        .flat_map(|id| descendants(settings, id))
        .collect();
    settings.collections.retain(|f| !removed.contains(&f.id));
    Ok(())
}

pub fn edit(
    settings: &mut Settings,
    operation: &str,
    id: &str,
    name: &str,
    parent: Option<&str>,
) -> Result<(), Message> {
    if operation == "delete" {
        if !settings.collections.iter().any(|f| f.id == id) {
            return Err(Message::new("folders.missing"));
        }
        let removed = descendants(settings, id);
        settings.collections.retain(|f| !removed.contains(&f.id));
        return Ok(());
    }
    if !matches!(operation, "create" | "update") {
        return Err(Message::new("error.operation"));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(Message::new("folders.name_required"));
    }
    if let Some(parent) = parent {
        if !settings.collections.iter().any(|f| f.id == parent) {
            return Err(Message::new("folders.missing"));
        }
        if operation == "update" && descendants(settings, id).contains(parent) {
            return Err(Message::new("folders.cycle"));
        }
    }
    if settings.collections.iter().any(|f| {
        (operation == "create" || f.id != id)
            && f.parent.as_deref() == parent
            && tags::key(&f.name) == tags::key(name)
    }) {
        return Err(Message::new("folders.duplicate_name"));
    }
    if operation == "create" {
        let id = loop {
            settings.next_collection_id = settings
                .next_collection_id
                .checked_add(1)
                .ok_or_else(|| Message::new("error.operation"))?;
            let candidate = format!("collection-{}", settings.next_collection_id);
            if !settings.collections.iter().any(|f| f.id == candidate) {
                break candidate;
            }
        };
        settings.collections.push(Collection {
            id,
            name: name.into(),
            parent: parent.map(str::to_owned),
            games: BTreeSet::new(),
        });
    } else {
        let folder = settings
            .collections
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| Message::new("folders.missing"))?;
        folder.name = name.into();
        folder.parent = parent.map(str::to_owned);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_delete_handles_overlapping_subtrees_and_rejects_missing_ids_atomically() {
        let mut settings = tree();
        edit(&mut settings, "create", "", "Other", None).unwrap();
        let before = settings.clone();
        assert!(delete_many(&mut settings, &["collection-1".into(), "missing".into()]).is_err());
        assert_eq!(settings, before);
        delete_many(
            &mut settings,
            &["collection-1".into(), "collection-2".into()],
        )
        .unwrap();
        assert_eq!(settings.collections.len(), 1);
        assert_eq!(settings.collections[0].name, "Other");
    }
    fn tree() -> Settings {
        let mut settings = Settings::default();
        edit(&mut settings, "create", "", "Series", None).unwrap();
        edit(
            &mut settings,
            "create",
            "",
            "Part one",
            Some("collection-1"),
        )
        .unwrap();
        settings
    }
    #[test]
    fn moving_parent_into_descendant_is_rejected_without_mutation() {
        let mut settings = tree();
        let before = settings.clone();
        assert!(
            edit(
                &mut settings,
                "update",
                "collection-1",
                "Series",
                Some("collection-2")
            )
            .is_err()
        );
        assert_eq!(settings, before);
    }
    #[test]
    fn sibling_name_comparison_is_case_insensitive_but_other_levels_can_share_names() {
        let mut settings = tree();
        assert!(edit(&mut settings, "create", "", " series ", None).is_err());
        assert!(edit(&mut settings, "create", "", "Series", Some("collection-1")).is_ok());
    }
    #[test]
    fn deleting_a_subtree_preserves_other_collections_and_game_records() {
        let mut settings = tree();
        edit(&mut settings, "create", "", "Other", None).unwrap();
        settings.collections[2].games.insert("game-a".into());
        edit(&mut settings, "delete", "collection-1", "", None).unwrap();
        assert_eq!(settings.collections.len(), 1);
        assert!(settings.collections[0].games.contains("game-a"));
    }
    #[test]
    fn old_settings_load_with_empty_collections() {
        let settings: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(settings.collections.is_empty());
    }
    #[test]
    fn collection_memberships_survive_settings_roundtrip() {
        let mut settings = tree();
        for folder in &mut settings.collections {
            folder.games.insert("same-game".into());
        }
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert_eq!(
            serde_json::from_slice::<Settings>(&bytes).unwrap(),
            settings
        );
    }
}
