//! Validate and prepare a complete detail edit before persisting any fields.
use crate::{Game, GameEdit, PlayStatus, providers::Metadata, tags};
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

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
        assert_eq!(game.tags, vec!["Imported"]);
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
