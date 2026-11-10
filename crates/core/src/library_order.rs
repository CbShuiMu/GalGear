//! 自定顺序的初始化、增量补齐与整组移动，持久化由桌面层负责。
use crate::{Game, Settings};
use std::collections::BTreeSet;

fn name_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 保留已保存的顺序，新游戏按品牌、规范化名称排序后追加。
/// 首次调用时全部游戏参与排序，使同品牌、同名前缀的作品相邻。
pub fn reconcile(settings: &mut Settings, games: &[Game]) {
    let known: BTreeSet<_> = games.iter().map(|game| game.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    settings
        .custom_order
        .retain(|id| known.contains(id.as_str()) && seen.insert(id.clone()));
    let mut remaining: Vec<_> = games
        .iter()
        .filter(|game| !seen.contains(&game.id))
        .collect();
    remaining.sort_by_cached_key(|game| {
        (
            name_key(
                game.metadata
                    .as_ref()
                    .map_or("", |metadata| metadata.brand.as_str()),
            ),
            name_key(&game.title),
            game.id.clone(),
        )
    });
    settings
        .custom_order
        .extend(remaining.into_iter().map(|game| game.id.clone()));
}

/// 按当前自定顺序移动整组条目，保留未展示、未选中条目的相对位置。
pub fn reorder(settings: &mut Settings, games: &[Game], ids: &[String], before: Option<&str>) {
    reconcile(settings, games);
    let known: BTreeSet<_> = games.iter().map(|game| game.id.as_str()).collect();
    let selected: BTreeSet<_> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| known.contains(id))
        .collect();
    if selected.is_empty() || before.is_some_and(|id| selected.contains(id) || !known.contains(id))
    {
        return;
    }
    let mut order = settings.custom_order.clone();
    let moving: Vec<_> = order
        .iter()
        .filter(|id| selected.contains(id.as_str()))
        .cloned()
        .collect();
    order.retain(|id| !selected.contains(id.as_str()));
    let index = before
        .and_then(|id| order.iter().position(|item| item == id))
        .unwrap_or(order.len());
    order.splice(index..index, moving);
    settings.custom_order = order;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn reorder_preserves_group_order_and_hidden_games() {
        let games: Vec<_> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(|id| {
                let mut game =
                    crate::catalog::manual_game(Path::new("F:/game.exe"), id.into()).unwrap();
                game.id = id.into();
                game
            })
            .collect();
        let mut settings = Settings {
            custom_order: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            ..Default::default()
        };
        reorder(&mut settings, &games, &["d".into(), "b".into()], Some("a"));
        assert_eq!(settings.custom_order, ["b", "d", "a", "c", "e"]);
        reorder(&mut settings, &games, &["b".into()], Some("b"));
        assert_eq!(settings.custom_order, ["b", "d", "a", "c", "e"]);
        reorder(&mut settings, &games, &["b".into(), "d".into()], None);
        assert_eq!(settings.custom_order, ["a", "c", "e", "b", "d"]);
    }

    #[test]
    fn initial_brand_and_series_order_survives_metadata_changes_and_reload() {
        let games: Vec<_> = [
            ("b", "Zeta", "Muv Luv Alt"),
            ("c", "Alpha", "Other"),
            ("a", "Zeta", "Muv-Luv"),
        ]
        .into_iter()
        .map(|(id, brand, title)| {
            let mut game =
                crate::catalog::manual_game(Path::new("F:/game.exe"), title.into()).unwrap();
            game.id = id.into();
            game.metadata = Some(crate::providers::Metadata {
                brand: brand.into(),
                ..Default::default()
            });
            game
        })
        .collect();
        let mut settings = Settings::default();
        reconcile(&mut settings, &games);
        assert_eq!(settings.custom_order, ["c", "a", "b"]);
        reorder(&mut settings, &games, &["b".into()], Some("c"));
        let mut saved: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        reconcile(&mut saved, &games);
        assert_eq!(saved.custom_order, ["b", "c", "a"]);
    }
}
