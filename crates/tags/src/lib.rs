//! Local tag names and edit rules. Names have no source identity.
use std::collections::{BTreeMap, BTreeSet};

/// Structured credits which also act as local library tags.
pub const CREDIT_FIELDS: &[&str] = &[
    "developer",
    "publisher",
    "brand",
    "artists",
    "voices",
    "writers",
    "music",
    "singers",
];

/// Split credit lists while preserving exact linked names, including punctuation.
pub fn credit_names<'a>(text: &str, linked: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let linked: Vec<_> = linked.into_iter().filter(|name| !name.is_empty()).collect();
    let mut names = Vec::new();
    let mut remaining = text;
    let split = |text: &str| {
        text.split(['、', '，', ',', '；', ';', '/', '／', '\n', '\r'])
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    while let Some((index, name)) = linked
        .iter()
        .filter_map(|name| remaining.find(name).map(|index| (index, *name)))
        .min_by_key(|(index, name)| (*index, std::cmp::Reverse(name.len())))
    {
        names.extend(split(&remaining[..index]));
        names.push(name.to_owned());
        remaining = &remaining[index + name.len()..];
    }
    names.extend(split(remaining));
    normalize(names)
}

/// ASCII case is ignored; other scripts retain their spelling.
pub fn key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Trim, discard empty names, and preserve the first spelling of each name.
pub fn normalize(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    names
        .into_iter()
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty() && seen.insert(key(n)))
        .collect()
}

/// Merge names while applying persistent local deletion/rename rules.
pub fn merge(
    local: &[String],
    incoming: &[String],
    rules: &BTreeMap<String, Option<String>>,
) -> Vec<String> {
    normalize(local.iter().cloned().chain(incoming.iter().filter_map(
        |n| match rules.get(&key(n)) {
            Some(value) => value.clone(),
            None => Some(n.clone()),
        },
    )))
}

/// Remember names removed or renamed by a local edit.
pub fn remember(
    previous: &[String],
    next: &[String],
    rules: &mut BTreeMap<String, Option<String>>,
) {
    for name in previous {
        if !next.iter().any(|n| key(n) == key(name)) {
            rules.insert(key(name), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credits_preserve_linked_multiword_names_and_split_unlinked_lists() {
        assert_eq!(
            credit_names(
                " Alice Smith、Circle/A, Bob；Carol / bob ",
                ["Bob", "Circle/A", "Alice Smith"]
            ),
            vec!["Alice Smith", "Circle/A", "Bob", "Carol"]
        );
    }

    #[test]
    fn adjacent_links_and_overlapping_names_do_not_create_combined_tags() {
        assert_eq!(
            credit_names("Alice Smith Bob", ["Alice", "Bob", "Alice Smith", ""]),
            vec!["Alice Smith", "Bob"]
        );
    }

    #[test]
    fn deleted_names_stay_deleted_but_new_names_are_imported() {
        let old = vec![" School ".into(), "school".into(), "恋愛".into()];
        let local = vec!["恋愛".into()];
        let mut rules = BTreeMap::new();
        remember(&old, &local, &mut rules);
        assert_eq!(
            merge(&local, &["SCHOOL".into(), "New".into()], &rules),
            vec!["恋愛", "New"]
        );
    }
}
