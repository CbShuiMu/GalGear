//! Local identity queries and exact, unambiguous background association.
use crate::{Game, providers::Resource};
use std::{collections::BTreeSet, path::Path};

pub(super) fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn queries(game: &Game) -> Vec<String> {
    let mut queries = Vec::new();
    let mut push = |value: &str| {
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if !value.is_empty() && !queries.contains(&value) {
            queries.push(value);
        }
    };
    if let Some(metadata) = &game.metadata {
        push(&metadata.title);
    }
    push(&game.title);
    if let Some(identity) = executable_identity(game) {
        push(&identity);
        let spaced = spaced_identity(&identity);
        // Search presentation may differ, but a query variant cannot add or
        // remove identity characters. Keep raw and spaced strings separately.
        if normalize(&spaced) == normalize(&identity) {
            push(&spaced);
        }
    }
    queries
}

fn executable_identity(game: &Game) -> Option<String> {
    let path = Path::new(game.executable.as_deref()?);
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let mut identity = path.file_stem()?.to_str()?.trim();
    loop {
        let previous = identity;
        identity = without_architecture_suffix(identity);
        if identity == previous {
            break;
        }
    }
    let normalized = normalize(identity);
    let entry = normalized.trim_end_matches(|character: char| character.is_ascii_digit());
    if normalized.is_empty()
        || matches!(
            normalized.as_str(),
            "x86" | "x64" | "amd64" | "win32" | "win64" | "32" | "64"
        )
        || matches!(
            entry,
            "game"
                | "main"
                | "start"
                | "startup"
                | "launcher"
                | "launch"
                | "krkr"
                | "krkrz"
                | "kirikiri"
                | "kirikiriz"
                | "kag"
                | "siglusengine"
                | "advhd"
                | "advhdnew"
                | "engine"
                | "system"
                | "boot"
                | "run"
                | "player"
                | "application"
        )
    {
        return None;
    }
    Some(identity.to_owned())
}

fn without_architecture_suffix(identity: &str) -> &str {
    const ARCHITECTURES: &[&str] = &[
        "x86_64", "amd64", "win64", "win32", "x64", "x86", "64-bit", "32-bit", "64_bit", "32_bit",
        "64bit", "32bit", "64", "32",
    ];
    let identity = identity.trim();
    for (opening, closing) in [('(', ')'), ('[', ']')] {
        if let Some(inner) = identity.strip_suffix(closing)
            && let Some((prefix, architecture)) = inner.rsplit_once(opening)
            && ARCHITECTURES
                .iter()
                .any(|known| architecture.trim().eq_ignore_ascii_case(known))
        {
            return prefix.trim_end_matches([' ', '_', '-', '.']);
        }
    }
    let lower = identity.to_ascii_lowercase();
    for architecture in ARCHITECTURES {
        if lower.ends_with(architecture) {
            let prefix = &identity[..identity.len() - architecture.len()];
            if prefix.ends_with([' ', '_', '-', '.']) {
                return prefix.trim_end_matches([' ', '_', '-', '.']);
            }
        }
    }
    identity
}

fn spaced_identity(identity: &str) -> String {
    let characters = identity.chars().collect::<Vec<_>>();
    let mut spaced = String::new();
    for (index, &character) in characters.iter().enumerate() {
        if matches!(character, '-' | '_' | '.') {
            spaced.push(' ');
            continue;
        }
        if character.is_uppercase() && index > 0 {
            let previous = characters[index - 1];
            let next_lowercase = characters
                .get(index + 1)
                .is_some_and(|next| next.is_lowercase());
            if previous.is_lowercase()
                || previous.is_ascii_digit()
                || previous.is_uppercase() && next_lowercase
            {
                spaced.push(' ');
            }
        }
        spaced.push(character);
    }
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn unique_match<'a>(results: &'a [Resource], identity: &str) -> Option<&'a Resource> {
    let expected = normalize(identity);
    if expected.is_empty() {
        return None;
    }
    let mut urls = BTreeSet::new();
    let mut selected = None;
    for result in results {
        if normalize(&result.title) != expected || result.url.trim().is_empty() {
            continue;
        }
        if urls.insert(result.url.trim().trim_end_matches('/')) {
            if selected.is_some() {
                return None;
            }
            selected = Some(result);
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_local_executable_identities_keep_raw_and_spaced_query_variants() {
        for (filename, raw, spaced) in [
            (
                "SilverGarden-AfterStory-_x64.exe",
                "SilverGarden-AfterStory",
                "Silver Garden After Story",
            ),
            (
                "BlueStarChronicle[win32].exe",
                "BlueStarChronicle",
                "Blue Star Chronicle",
            ),
        ] {
            let game = crate::catalog::manual_game(
                &Path::new("games").join(filename),
                "本地显示名".into(),
            )
            .unwrap();
            let values = queries(&game);
            assert!(values.iter().any(|value| value == raw));
            assert!(values.iter().any(|value| value == spaced));
            assert_eq!(normalize(raw), normalize(spaced));
        }
    }

    #[test]
    fn generic_entry_executables_never_become_identity_searches() {
        for filename in [
            "game.exe",
            "Main_x64.exe",
            "start[win32].exe",
            "Launcher(64-bit).exe",
            "krkrz.exe",
            "AdvHD_New64.exe",
        ] {
            let game = crate::catalog::manual_game(
                &Path::new("games").join(filename),
                "Chosen title".into(),
            )
            .unwrap();
            assert_eq!(queries(&game), ["Chosen title"]);
        }
    }

    #[test]
    fn metadata_and_user_titles_keep_priority_without_losing_distinct_presentations() {
        let mut game =
            crate::catalog::manual_game(Path::new("games/SilverGarden.exe"), "Local name".into())
                .unwrap();
        game.metadata = Some(crate::providers::Metadata {
            title: "Official title".into(),
            ..Default::default()
        });
        let values = queries(&game);
        assert_eq!(
            values,
            [
                "Official title",
                "Local name",
                "SilverGarden",
                "Silver Garden"
            ]
        );
    }

    #[test]
    fn exact_identity_matches_deduplicate_urls_and_reject_different_same_title_records() {
        let result = |title: &str, url: &str| Resource {
            title: title.into(),
            url: url.into(),
            ..Default::default()
        };
        let rows = vec![
            result("Silver Garden -After Story-", "https://example.test/work/1"),
            result("SilverGardenAfterStory", "https://example.test/work/1/"),
            result("Silver Garden After Story 2", "https://example.test/work/2"),
        ];
        assert!(unique_match(&rows, "SilverGarden-AfterStory-").is_some());
        let mut ambiguous = rows;
        ambiguous.push(result(
            "Silver Garden After Story",
            "https://example.test/work/3",
        ));
        assert!(unique_match(&ambiguous, "SilverGarden-AfterStory-").is_none());
        assert!(unique_match(&ambiguous, "Silver Garden").is_none());
    }
}
