//! Local tag names and edit rules. Names have no source identity.
use std::collections::{BTreeMap, BTreeSet};

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
