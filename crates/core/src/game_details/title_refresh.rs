//! 改标题后的来源选择、唯一词条判断与覆盖规则；网络和持久化由调用方负责。
use crate::{
    Game, GameEdit, data_sources,
    game_details::{MetadataTarget, associate},
    providers::{Metadata, Resource},
};
use std::collections::BTreeMap;

/// 一次自动查询使用的来源和当前关联，标签与资料分别计算。
#[derive(Clone, Debug, PartialEq)]
pub struct Lookup {
    pub source: String,
    pub current_url: Option<String>,
}

/// 优先使用逐游戏关联；旧版独立标签未记录 URL 时回退到标签入口偏好。
pub fn lookup(
    game: &Game,
    edit: &GameEdit,
    target: MetadataTarget,
    preferences: &BTreeMap<String, String>,
    default_source: &str,
) -> Lookup {
    let metadata_url = game
        .metadata
        .as_ref()
        .map(|meta| meta.url.as_str())
        .filter(|url| !url.is_empty());
    let (context, url) = match target {
        MetadataTarget::Details => ("metadata", metadata_url),
        MetadataTarget::Tags => (
            "tags",
            edit.tag_source_url
                .as_deref()
                .filter(|url| !url.is_empty())
                .or_else(|| edit.source_tags.is_none().then_some(metadata_url).flatten()),
        ),
    };
    let source = url
        .and_then(|url| {
            data_sources::source_for_url(url)
                .ok()
                .map(|(source, _)| source)
        })
        .or_else(|| {
            matches!(target, MetadataTarget::Details)
                .then_some(edit.metadata_provider.as_str())
                .filter(|source| !source.is_empty())
        })
        .or_else(|| preferences.get(context).map(String::as_str))
        .unwrap_or(default_source);
    Lookup {
        source: source.into(),
        current_url: url.map(str::to_owned),
    }
}

fn identity(url: &str) -> Option<(String, String)> {
    let (source, id) = data_sources::source_for_url(url).ok()?;
    if source != "2dfan" {
        return Some((source.into(), id));
    }
    // 2DFan 支持自定义域名，按 subject ID 比较镜像站上的同一词条。
    let parsed = reqwest::Url::parse(url).ok()?;
    let id = parsed
        .path()
        .trim_end_matches('/')
        .strip_prefix("/subjects/")?;
    (!id.is_empty() && id.bytes().all(|c| c.is_ascii_digit())).then(|| (source.into(), id.into()))
}

/// 仅接受源一致、唯一且不同于当前词条的结果；域名别名不构成新词条。
pub fn unique_changed_url<'a>(lookup: &Lookup, results: &'a [Resource]) -> Option<&'a str> {
    let [result] = results else {
        return None;
    };
    let candidate = identity(&result.url)?;
    if candidate.0 != lookup.source
        || lookup.current_url.as_deref().and_then(identity).as_ref() == Some(&candidate)
    {
        return None;
    }
    Some(&result.url)
}

/// 查询期间标题、关联或目标内容改变时丢弃旧结果；无关配置变动不影响更新。
pub fn unchanged(
    before: &Game,
    previous: &GameEdit,
    current: &Game,
    edit: &GameEdit,
    target: MetadataTarget,
) -> bool {
    if before.title != current.title || previous.title != edit.title {
        return false;
    }
    match target {
        MetadataTarget::Details => {
            before.metadata == current.metadata
                && previous.metadata == edit.metadata
                && previous.metadata_overrides == edit.metadata_overrides
                && previous.description == edit.description
                && previous.metadata_provider == edit.metadata_provider
        }
        MetadataTarget::Tags => {
            before.tags == current.tags
                && previous.tags == edit.tags
                && previous.source_tags == edit.source_tags
                && previous.tag_source_url == edit.tag_source_url
                && previous.tag_rules == edit.tag_rules
        }
    }
}

/// 覆盖对应目标及其手动保护；保留用户新标题、封面和其他游戏配置。
pub fn apply(
    game: &Game,
    edit: &GameEdit,
    source: &str,
    metadata: Metadata,
    target: MetadataTarget,
) -> GameEdit {
    let release_date = metadata.release_date.clone();
    let mut updated = associate(game, Some(edit), source, metadata, target);
    if matches!(target, MetadataTarget::Details) {
        updated.metadata_overrides.clear();
        updated.description = None;
        updated.summary_release_date = Some(release_date);
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Settings, catalog};
    use std::path::Path;

    fn game() -> Game {
        let mut game =
            catalog::manual_game(Path::new("F:/Game/game.exe"), "New title".into()).unwrap();
        game.metadata = Some(Metadata {
            url: "https://vndb.org/v1".into(),
            description: "Old story".into(),
            ..Default::default()
        });
        game.tags = vec!["Old tag".into()];
        game.cover = Some("Local cover".into());
        game
    }

    #[test]
    fn uses_independent_game_associations_before_global_preferences() {
        let game = game();
        let edit = GameEdit {
            tag_source_url: Some("https://bgm.tv/subject/2".into()),
            ..Default::default()
        };
        let preferences = BTreeMap::from([
            ("tags".into(), "nextmoe".into()),
            ("metadata".into(), "ymgal".into()),
        ]);
        assert_eq!(
            lookup(&game, &edit, MetadataTarget::Details, &preferences, "2dfan").source,
            "vndb"
        );
        assert_eq!(
            lookup(&game, &edit, MetadataTarget::Tags, &preferences, "2dfan").source,
            "bangumi"
        );
        let old: GameEdit = serde_json::from_str(r#"{"source_tags":["Old tag"]}"#).unwrap();
        assert!(old.tag_source_url.is_none());
        assert_eq!(
            lookup(&game, &old, MetadataTarget::Tags, &preferences, "2dfan"),
            Lookup {
                source: "nextmoe".into(),
                current_url: None
            }
        );
        assert_eq!(
            lookup(
                &game,
                &GameEdit::default(),
                MetadataTarget::Tags,
                &preferences,
                "2dfan"
            )
            .source,
            "vndb"
        );
    }

    #[test]
    fn only_a_single_different_work_can_replace_content() {
        let lookup = Lookup {
            source: "vndb".into(),
            current_url: Some("https://vndb.org/v1/".into()),
        };
        let resource = |url: &str| Resource {
            url: url.into(),
            ..Default::default()
        };
        assert!(unique_changed_url(&lookup, &[]).is_none());
        assert!(unique_changed_url(&lookup, &[resource("https://vndb.org/v1")]).is_none());
        assert!(
            unique_changed_url(
                &lookup,
                &[
                    resource("https://vndb.org/v2"),
                    resource("https://vndb.org/v3")
                ]
            )
            .is_none()
        );
        assert!(unique_changed_url(&lookup, &[resource("https://bgm.tv/subject/2")]).is_none());
        assert!(unique_changed_url(&lookup, &[resource("invalid")]).is_none());
        assert_eq!(
            unique_changed_url(&lookup, &[resource("https://vndb.org/v2")]),
            Some("https://vndb.org/v2")
        );
    }

    #[test]
    fn alternate_urls_for_same_work_do_not_trigger_refresh() {
        for (source, old, new) in [
            (
                "bangumi",
                "https://bgm.tv/subject/1",
                "https://bangumi.tv/subject/1/",
            ),
            (
                "nextmoe",
                "https://api.nextmoe.dev/v2/catalog/works/1",
                "https://developer.nextmoe.dev/explore/work/1?nsfw=1",
            ),
            (
                "2dfan",
                "https://2dfan.com/subjects/1",
                "https://fan.example/subjects/1/",
            ),
        ] {
            assert!(
                unique_changed_url(
                    &Lookup {
                        source: source.into(),
                        current_url: Some(old.into())
                    },
                    &[Resource {
                        url: new.into(),
                        ..Default::default()
                    }]
                )
                .is_none()
            );
        }
    }

    #[test]
    fn overwrites_both_targets_and_manual_protection_but_preserves_title_and_configuration() {
        let mut game = game();
        let previous = GameEdit {
            title: Some(game.title.clone()),
            tags: game.tags.clone(),
            tag_rules: BTreeMap::from([("new tag".into(), None)]),
            metadata_overrides: BTreeMap::from([("description".into(), "Manual story".into())]),
            description: Some("Legacy story".into()),
            summary_release_date: Some("Old date".into()),
            save_paths: vec!["save".into()],
            auto_backup: true,
            ..Default::default()
        };
        let metadata = Metadata {
            url: "https://vndb.org/v2".into(),
            title: "Source title".into(),
            description: "New story".into(),
            release_date: "2026-01-01".into(),
            ..Default::default()
        };
        let details = apply(
            &game,
            &previous,
            "vndb",
            metadata.clone(),
            MetadataTarget::Details,
        );
        let edit = apply(
            &game,
            &details,
            "bangumi",
            Metadata {
                url: "https://bgm.tv/subject/3".into(),
                tags: vec!["New tag".into()],
                ..Default::default()
            },
            MetadataTarget::Tags,
        );
        let settings = Settings {
            game_edits: BTreeMap::from([(game.id.clone(), edit)]),
            ..Default::default()
        };
        let reloaded: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        catalog::apply_edits(&mut game, &reloaded);
        catalog::apply_edits(&mut game, &reloaded);
        assert_eq!(game.title, "New title");
        assert_eq!(game.tags, ["New tag"]);
        assert_eq!(game.metadata, Some(metadata));
        assert_eq!(game.cover.as_deref(), Some("Local cover"));
        let edit = &reloaded.game_edits[&game.id];
        assert_eq!(
            edit.tag_source_url.as_deref(),
            Some("https://bgm.tv/subject/3")
        );
        assert_eq!(edit.summary_release_date.as_deref(), Some("2026-01-01"));
        assert_eq!(edit.save_paths, ["save"]);
        assert!(
            edit.auto_backup && edit.tag_rules.is_empty() && edit.metadata_overrides.is_empty()
        );
    }

    #[test]
    fn details_only_update_keeps_original_tag_association_and_tags() {
        let mut game = game();
        let edit = apply(
            &game,
            &GameEdit::default(),
            "bangumi",
            Metadata {
                url: "https://bgm.tv/subject/2".into(),
                tags: vec!["Unwanted".into()],
                ..Default::default()
            },
            MetadataTarget::Details,
        );
        assert_eq!(edit.tag_source_url.as_deref(), Some("https://vndb.org/v1"));
        let id = game.id.clone();
        catalog::apply_edits(
            &mut game,
            &Settings {
                game_edits: BTreeMap::from([(id, edit)]),
                ..Default::default()
            },
        );
        assert_eq!(game.tags, ["Old tag"]);
    }

    #[test]
    fn late_result_cannot_overwrite_new_title_or_new_target_edits() {
        let before = game();
        let previous = GameEdit::default();
        let mut current = before.clone();
        current.title = "Even newer title".into();
        for target in [MetadataTarget::Details, MetadataTarget::Tags] {
            assert!(!unchanged(&before, &previous, &current, &previous, target));
        }
        current = before.clone();
        current.tags.push("Manual".into());
        assert!(!unchanged(
            &before,
            &previous,
            &current,
            &previous,
            MetadataTarget::Tags
        ));
        assert!(unchanged(
            &before,
            &previous,
            &current,
            &previous,
            MetadataTarget::Details
        ));
        let edit = GameEdit {
            metadata_overrides: BTreeMap::from([("brand".into(), "New brand".into())]),
            ..Default::default()
        };
        assert!(!unchanged(
            &before,
            &previous,
            &before,
            &edit,
            MetadataTarget::Details
        ));
        let unrelated = GameEdit {
            auto_backup: true,
            ..Default::default()
        };
        assert!(unchanged(
            &before,
            &previous,
            &before,
            &unrelated,
            MetadataTarget::Details
        ));
    }
}
