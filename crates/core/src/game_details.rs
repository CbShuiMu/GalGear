//! Validate and prepare a complete detail edit before persisting any fields.
use crate::{Game, GameEdit, PlayStatus, providers::Metadata, tags};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataTarget {
    Details,
    Tags,
}

pub fn associate(
    game: &Game,
    previous: Option<&GameEdit>,
    source: &str,
    metadata: Metadata,
    target: MetadataTarget,
) -> GameEdit {
    let mut edit = previous.cloned().unwrap_or_else(|| GameEdit {
        play_status: game.play_status,
        ..Default::default()
    });
    edit.title.get_or_insert_with(|| game.title.clone());
    match target {
        MetadataTarget::Details => {
            edit.tags.clone_from(&game.tags);
            edit.summary_brand.get_or_insert_with(|| {
                game.metadata
                    .as_ref()
                    .map(|meta| meta.brand.clone())
                    .unwrap_or_default()
            });
            edit.summary_release_date.get_or_insert_with(|| {
                game.metadata
                    .as_ref()
                    .map(|meta| meta.release_date.clone())
                    .unwrap_or_default()
            });
            edit.source_tags.get_or_insert_with(|| {
                game.metadata
                    .as_ref()
                    .map(|meta| meta.tag_names())
                    .unwrap_or_default()
            });
            if game
                .metadata
                .as_ref()
                .is_none_or(|current| current.url != metadata.url)
            {
                edit.metadata_overrides.clear();
                edit.description = None;
            }
            edit.metadata_provider = source.into();
            edit.metadata = Some(metadata);
        }
        MetadataTarget::Tags => {
            let names = metadata.tag_names();
            edit.tags.clone_from(&names);
            edit.source_tags = Some(names);
            edit.tag_rules.clear();
        }
    }
    edit
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Draft {
    pub title: String,
    pub tags: Vec<String>,
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub launcher: Option<String>,
    pub locale_profile: String,
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub metadata_resets: Vec<String>,
    #[serde(default)]
    pub reset_tag_rules: bool,
}

pub fn prepare(
    game: &Game,
    previous: Option<&GameEdit>,
    draft: Draft,
) -> Result<GameEdit, Message> {
    if draft.title.trim().is_empty() {
        return Err(Message::new("error.title_required"));
    }
    if !Path::new(draft.executable.trim())
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Err(Message::new("error.executable_required"));
    }
    let mut edit = previous.cloned().unwrap_or_else(|| GameEdit {
        play_status: game.play_status,
        ..Default::default()
    });
    if draft.title != game.title {
        edit.title = Some(draft.title.trim().into());
    }
    let names = tags::normalize(draft.tags);
    if draft.reset_tag_rules {
        edit.tag_rules.clear();
    }
    tags::remember(&game.tags, &names, &mut edit.tag_rules);
    edit.tags = names;
    edit.executable = Some(draft.executable.trim().into());
    edit.arguments = draft.arguments;
    edit.working_directory = draft.working_directory;
    edit.launcher = draft.launcher;
    edit.locale_profile = draft.locale_profile;
    let empty = Metadata::default();
    let current = game.metadata.as_ref().unwrap_or(&empty);
    for field in &draft.metadata_resets {
        if field == "title" || current.field(field).is_none() {
            return Err(Message::new("error.operation"));
        }
        edit.metadata_overrides.remove(field);
        if field == "description" {
            edit.description = None;
        }
    }
    for (field, text) in draft.metadata {
        let Some(old) = current.field(&field).filter(|_| field != "title") else {
            return Err(Message::new("error.operation"));
        };
        // Only changed fields receive protection; untouched fields can still refresh.
        if old != text && !draft.metadata_resets.contains(&field) {
            edit.metadata_overrides.insert(field, text);
            if edit.metadata.is_none() {
                edit.metadata = Some(current.clone());
            }
        }
    }
    Ok(edit)
}

pub fn with_status(game: &Game, previous: Option<&GameEdit>, status: PlayStatus) -> GameEdit {
    let mut edit = previous.cloned().unwrap_or_else(|| GameEdit {
        tags: game.tags.clone(),
        ..Default::default()
    });
    edit.play_status = status;
    edit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_metadata_preserves_title_tags_and_cover_after_reload() {
        let mut game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Local title".into())
                .unwrap();
        game.tags = vec!["Local tag".into(), "Old tag".into()];
        game.cover = Some("current-cover".into());
        game.metadata = Some(Metadata {
            url: "https://vndb.org/v1".into(),
            tags: vec!["Old tag".into()],
            ..Default::default()
        });
        let previous = GameEdit {
            description: Some("Manual story".into()),
            metadata_overrides: BTreeMap::from([("brand".into(), "Manual brand".into())]),
            ..Default::default()
        };
        let incoming = Metadata {
            url: "https://bangumi.tv/subject/1".into(),
            title: "Different title".into(),
            brand: "New brand".into(),
            tags: vec!["New tag".into()],
            description: "New story".into(),
            ..Default::default()
        };
        let edit = associate(
            &game,
            Some(&previous),
            "bangumi",
            incoming.clone(),
            MetadataTarget::Details,
        );
        let settings = crate::Settings {
            game_edits: BTreeMap::from([(game.id.clone(), edit)]),
            ..Default::default()
        };
        let reloaded = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        for _ in 0..2 {
            crate::catalog::apply_edits(&mut game, &reloaded);
        }
        assert_eq!(
            (game.title.as_str(), game.tags, game.cover, game.metadata),
            (
                "Local title",
                vec!["Local tag".into(), "Old tag".into()],
                Some("current-cover".into()),
                Some(incoming)
            )
        );
    }

    #[test]
    fn changing_tag_source_overwrites_manual_tags_and_rules_without_changing_metadata() {
        let mut game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Local title".into())
                .unwrap();
        game.tags = vec!["Local tag".into(), "Old tag".into()];
        game.metadata = Some(Metadata {
            title: "Original source title".into(),
            brand: "Original brand".into(),
            ..Default::default()
        });
        let original = game.metadata.clone();
        let previous = GameEdit {
            tag_rules: BTreeMap::from([("new tag".into(), None)]),
            ..Default::default()
        };
        let edit = associate(
            &game,
            Some(&previous),
            "vndb",
            Metadata {
                tags: vec!["New tag".into()],
                brand: "New brand".into(),
                ..Default::default()
            },
            MetadataTarget::Tags,
        );
        let settings = crate::Settings {
            game_edits: BTreeMap::from([(game.id.clone(), edit)]),
            ..Default::default()
        };
        let reloaded = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        for _ in 0..2 {
            crate::catalog::apply_edits(&mut game, &reloaded);
        }
        assert_eq!(
            (game.title.as_str(), game.tags, game.metadata),
            (
                "Local title",
                vec!["New tag".into(), "New brand".into()],
                original
            )
        );
    }

    #[test]
    fn empty_tag_source_stays_empty_when_metadata_is_refreshed() {
        let mut game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Local title".into())
                .unwrap();
        game.tags = vec!["Old tag".into()];
        game.metadata = Some(Metadata {
            brand: "Original brand".into(),
            ..Default::default()
        });
        let edit = associate(
            &game,
            None,
            "vndb",
            Metadata::default(),
            MetadataTarget::Tags,
        );
        let mut settings = crate::Settings {
            game_edits: BTreeMap::from([(game.id.clone(), edit)]),
            ..Default::default()
        };
        crate::catalog::apply_edits(&mut game, &settings);
        let edit = settings.game_edits.get(&game.id).unwrap();
        let edit = associate(
            &game,
            Some(edit),
            "bangumi",
            Metadata {
                brand: "Another brand".into(),
                ..Default::default()
            },
            MetadataTarget::Details,
        );
        settings.game_edits.insert(game.id.clone(), edit);
        crate::catalog::apply_edits(&mut game, &settings);
        assert!(game.tags.is_empty());
    }

    fn draft() -> Draft {
        Draft {
            title: "Title".into(),
            tags: vec!["Local".into()],
            executable: "F:/Title/game.exe".into(),
            arguments: vec![],
            working_directory: None,
            launcher: None,
            locale_profile: "ja-JP".into(),
            metadata: BTreeMap::new(),
            metadata_resets: Vec::new(),
            reset_tag_rules: false,
        }
    }
    #[test]
    fn invalid_field_cannot_partially_mutate_existing_edits() {
        let game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Title".into()).unwrap();
        let previous = GameEdit {
            locale_profile: "original".into(),
            ..Default::default()
        };
        let mut input = draft();
        input.metadata.insert("unknown".into(), "value".into());
        assert!(prepare(&game, Some(&previous), input).is_err());
        assert_eq!(previous.locale_profile, "original");
    }
    #[test]
    fn first_status_change_preserves_imported_tags_and_cached_metadata() {
        let mut game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Title".into()).unwrap();
        game.tags = vec!["Imported".into()];
        game.metadata = Some(Metadata {
            brand: "Brand".into(),
            ..Default::default()
        });
        let edit = with_status(&game, None, PlayStatus::Wanted);
        let settings = crate::Settings {
            game_edits: BTreeMap::from([(game.id.clone(), edit)]),
            ..Default::default()
        };
        crate::catalog::apply_edits(&mut game, &settings);
        assert_eq!(game.tags, vec!["Imported", "Brand"]);
        assert_eq!(game.metadata.as_ref().unwrap().brand, "Brand");
        assert_eq!(game.play_status, PlayStatus::Wanted);
    }
    #[test]
    fn reset_removes_manual_protection_without_reprotecting_the_draft_value() {
        let mut game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Title".into()).unwrap();
        game.metadata = Some(Metadata {
            description: "Manual story".into(),
            ..Default::default()
        });
        let previous = GameEdit {
            metadata: Some(Metadata {
                description: "Cached story".into(),
                ..Default::default()
            }),
            metadata_overrides: BTreeMap::from([("description".into(), "Manual story".into())]),
            ..Default::default()
        };
        let mut input = draft();
        input
            .metadata
            .insert("description".into(), "Cached story".into());
        input.metadata_resets.push("description".into());
        let edit = prepare(&game, Some(&previous), input).unwrap();
        assert!(!edit.metadata_overrides.contains_key("description"));
    }
    #[test]
    fn complete_edit_preserves_status_saves_and_protects_only_changed_metadata() {
        let mut game =
            crate::catalog::manual_game(Path::new("F:/Title/game.exe"), "Title".into()).unwrap();
        game.metadata = Some(Metadata {
            brand: "Brand".into(),
            description: "Story".into(),
            ..Default::default()
        });
        let previous = GameEdit {
            play_status: PlayStatus::Finished,
            save_paths: vec!["save".into()],
            ..Default::default()
        };
        let mut input = draft();
        input.metadata.insert("brand".into(), "Brand".into());
        input.metadata.insert("description".into(), String::new());
        let edit = prepare(&game, Some(&previous), input).unwrap();
        assert_eq!(edit.play_status, PlayStatus::Finished);
        assert_eq!(edit.save_paths, vec!["save"]);
        assert!(!edit.metadata_overrides.contains_key("brand"));
        assert_eq!(edit.metadata_overrides["description"], "");
    }
}
