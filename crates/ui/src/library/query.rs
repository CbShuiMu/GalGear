//! 按目录代数缓存搜索文本与标签倒排索引；条目位置仅在该代数内有效。
use super::{filter_brands, filter_tags};
use crate::models::Catalog;
use std::{collections::HashMap, rc::Rc};

#[derive(Default)]
pub(super) struct SearchIndex {
    revision: Option<Rc<()>>,
    names: Vec<Vec<String>>,
    tags: HashMap<String, Vec<usize>>,
    brands: HashMap<String, Vec<usize>>,
}

impl SearchIndex {
    pub(super) fn matching(
        &mut self,
        catalog: &Catalog,
        query: &str,
        tags: &[String],
        brands: &[String],
    ) -> Vec<usize> {
        let revision = catalog.revision();
        if !self
            .revision
            .as_ref()
            .is_some_and(|old| Rc::ptr_eq(old, &revision))
        {
            self.rebuild(catalog, revision);
        }
        let mut lists = Vec::new();
        for (selected, index) in [(tags, &self.tags), (brands, &self.brands)] {
            for name in selected {
                let Some(postings) = index.get(&galgear_tags::key(name)) else {
                    return Vec::new();
                };
                lists.push(postings);
            }
        }
        lists.sort_by_key(|list| list.len());
        let matches = |position: usize| {
            self.names[position].iter().any(|name| name.contains(query))
                && lists
                    .iter()
                    .skip(1)
                    .all(|list| list.binary_search(&position).is_ok())
        };
        // 从最短的有序倒排表开始，保留目录顺序供稳定排序处理同名游戏。
        match lists.first() {
            Some(smallest) => smallest
                .iter()
                .copied()
                .filter(|&position| matches(position))
                .collect(),
            None => (0..self.names.len())
                .filter(|&position| matches(position))
                .collect(),
        }
    }

    fn rebuild(&mut self, catalog: &Catalog, revision: Rc<()>) {
        self.names.clear();
        self.tags.clear();
        self.brands.clear();
        for (position, game) in catalog.games.iter().enumerate() {
            self.names.push(
                std::iter::once(&game.title)
                    .chain(&game.aliases)
                    .map(|name| name.to_lowercase())
                    .collect(),
            );
            for (names, index) in [
                (filter_tags(game).collect::<Vec<_>>(), &mut self.tags),
                (filter_brands(game).collect::<Vec<_>>(), &mut self.brands),
            ] {
                for name in names {
                    let postings = index.entry(galgear_tags::key(&name)).or_default();
                    if postings.last() != Some(&position) {
                        postings.push(position);
                    }
                }
            }
        }
        self.revision = Some(revision);
    }
}

#[cfg(test)]
mod tests {
    use super::SearchIndex;
    use crate::models::Catalog;
    use serde_json::json;
    use std::rc::Rc;

    #[test]
    fn cached_index_tracks_mutation_clone_replacement_and_reordering() {
        let mut catalog: Catalog = serde_json::from_value(json!({"warnings":[],"games":[
            {"id":"a","title":"Alpha","aliases":["星之终途"],"folder":"A","engine":"unknown","modified":0,"tags":["ADV","ADV"],"metadata":{"brand":"Key"}},
            {"id":"b","title":"Beta","folder":"B","engine":"unknown","modified":0,"tags":["ADV"]}
        ]})).unwrap();
        let mut index = SearchIndex::default();
        assert_eq!(
            index.matching(&catalog, "星之", &["adv".into()], &["key".into()]),
            [0]
        );
        let revision = index.revision.clone().unwrap();
        assert_eq!(index.matching(&catalog, "", &["ADV".into()], &[]), [0, 1]);
        assert!(Rc::ptr_eq(&revision, index.revision.as_ref().unwrap()));
        let original = catalog.clone();
        catalog.games[0].aliases.clear();
        catalog.games[0].title = "Changed".into();
        catalog.games[0].tags.clear();
        catalog.games[0].metadata.as_mut().unwrap().brand = "Other".into();
        assert!(index.matching(&catalog, "星之", &[], &[]).is_empty());
        assert!(
            index
                .matching(&catalog, "", &[], &["key".into()])
                .is_empty()
        );
        assert_eq!(index.matching(&catalog, "", &["ADV".into()], &[]), [1]);
        catalog.games.reverse();
        assert_eq!(index.matching(&catalog, "", &["ADV".into()], &[]), [0]);
        assert_eq!(index.matching(&original, "alpha", &[], &[]), [0]);
        catalog.games.clear();
        assert!(index.matching(&catalog, "", &[], &[]).is_empty());
    }
}
