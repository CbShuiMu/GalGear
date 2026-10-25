//! Compact hour and calendar-date presentation for recorded game sessions.
use crate::models::{Catalog, Game};
use std::collections::BTreeMap;

pub(crate) fn merge_record(game: &mut Game, seconds: u64, last_played: Option<u64>) {
    game.playtime_seconds = game.playtime_seconds.max(seconds);
    game.last_played = game.last_played.max(last_played);
}

pub(crate) fn replace_game(current: &mut Game, mut incoming: Game) {
    merge_record(&mut incoming, current.playtime_seconds, current.last_played);
    *current = incoming;
}

pub(crate) fn replace_catalog(current: &mut Catalog, mut incoming: Catalog) {
    let records: BTreeMap<_, _> = current.games.iter().map(|game| (&game.id, game)).collect();
    for game in &mut incoming.games {
        if let Some(previous) = records.get(&game.id) {
            merge_record(game, previous.playtime_seconds, previous.last_played);
        }
    }
    *current = incoming;
}

pub(crate) fn hours(seconds: u64) -> String {
    let tenths = seconds / 360 + u64::from(seconds % 360 >= 180);
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

pub(crate) fn date(seconds: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        crate::bridge::local_date(seconds as f64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(date) = i64::try_from(seconds)
            .ok()
            .and_then(|seconds| time::OffsetDateTime::from_unix_timestamp(seconds).ok())
        else {
            return String::new();
        };
        format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            u8::from(date.month()),
            date.day()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn game(title: &str, seconds: u64, last_played: Option<u64>) -> Game {
        serde_json::from_value(json!({
            "id": "game", "title": title, "folder": "X:/Game", "engine": "unknown",
            "modified": 0, "playtime_seconds": seconds, "last_played": last_played
        }))
        .unwrap()
    }

    #[test]
    fn stale_scan_catalog_keeps_the_final_playtime_checkpoint() {
        let mut current = Catalog {
            games: vec![game("Original", 7200, Some(200))],
            ..Default::default()
        };
        let incoming = Catalog {
            games: vec![game("Rescanned", 3600, Some(100))],
            ..Default::default()
        };

        replace_catalog(&mut current, incoming);

        assert_eq!(
            (
                current.games[0].title.as_str(),
                current.games[0].playtime_seconds,
                current.games[0].last_played
            ),
            ("Rescanned", 7200, Some(200))
        );
    }

    #[test]
    fn stale_enrichment_keeps_the_final_playtime_checkpoint() {
        let mut current = game("Original", 7200, Some(200));
        let incoming = game("Enriched", 3600, None);

        replace_game(&mut current, incoming);

        assert_eq!(
            (
                current.title.as_str(),
                current.playtime_seconds,
                current.last_played
            ),
            ("Enriched", 7200, Some(200))
        );
    }

    #[test]
    fn newer_catalog_playtime_is_not_replaced_by_older_local_values() {
        let mut current = Catalog {
            games: vec![game("Original", 3600, Some(100))],
            ..Default::default()
        };
        let incoming = Catalog {
            games: vec![game("Refreshed", 7200, Some(200))],
            ..Default::default()
        };

        replace_catalog(&mut current, incoming);

        assert_eq!(
            (
                current.games[0].playtime_seconds,
                current.games[0].last_played
            ),
            (7200, Some(200))
        );
    }

    #[test]
    fn hours_round_to_one_decimal_without_trailing_zeroes() {
        for (seconds, expected) in [
            (0, "0"),
            (179, "0"),
            (180, "0.1"),
            (3600, "1"),
            (3959, "1.1"),
            (7019, "1.9"),
            (7020, "2"),
        ] {
            assert_eq!(hours(seconds), expected, "seconds={seconds}");
        }
    }

    #[test]
    fn hours_format_large_recordings_without_overflow() {
        assert_eq!(hours(u64::MAX), "5124095576030431");
    }
}
