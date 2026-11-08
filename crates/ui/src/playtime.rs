//! Compact hour and calendar-date presentation for recorded game sessions.
use crate::models::{Catalog, Game};
use galgear_i18n::{Language, format_text, tr};
use std::collections::BTreeMap;
use time::Date;

pub(crate) fn merge_record(game: &mut Game, seconds: u64, last_played: Option<u64>) {
    game.playtime_seconds = game.playtime_seconds.max(seconds);
    game.last_played = game.last_played.max(last_played);
}

pub(crate) fn replace_game(current: &mut Game, mut incoming: Game) {
    merge_record(&mut incoming, current.playtime_seconds, current.last_played);
    if incoming.aliases.is_empty() {
        incoming.aliases.clone_from(&current.aliases);
    }
    *current = incoming;
}

pub(crate) fn replace_catalog(current: &mut Catalog, mut incoming: Catalog) {
    let records: BTreeMap<_, _> = current.games.iter().map(|game| (&game.id, game)).collect();
    for game in &mut incoming.games {
        if let Some(previous) = records.get(&game.id) {
            merge_record(game, previous.playtime_seconds, previous.last_played);
            if game.aliases.is_empty() {
                game.aliases.clone_from(&previous.aliases);
            }
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

pub(crate) fn date_time(seconds: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        crate::bridge::local_date_time(seconds as f64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(time) = i64::try_from(seconds)
            .ok()
            .and_then(|seconds| time::OffsetDateTime::from_unix_timestamp(seconds).ok())
        else {
            return String::new();
        };
        format!("{} {:02}:{:02}", date(seconds), time.hour(), time.minute())
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

/// 按本地日历显示最近游玩日期；无效时间返回空字符串，未来日期保留绝对日期。
pub(crate) fn recent_date(language: Language, seconds: u64) -> String {
    let text = date(seconds);
    let Some(played) = calendar_date(&text) else {
        return text;
    };
    #[cfg(target_arch = "wasm32")]
    let today = calendar_date(&crate::bridge::local_today());
    #[cfg(not(target_arch = "wasm32"))]
    let today = Some(time::OffsetDateTime::now_utc().date());
    today.map_or(text, |today| relative_date(language, played, today))
}

fn calendar_date(text: &str) -> Option<Date> {
    let (year, rest) = text.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    Date::from_calendar_date(
        year.parse().ok()?,
        month.parse::<u8>().ok()?.try_into().ok()?,
        day.parse().ok()?,
    )
    .ok()
}

fn relative_date(language: Language, played: Date, today: Date) -> String {
    // 使用日历日差与周一边界，避免跨午夜或夏令时被当成不足 24 小时的同一天。
    let days = (today - played).whole_days();
    let weekday = i64::from(today.weekday().number_days_from_monday());
    match days {
        0 => tr(language, "date.today").into(),
        1 => tr(language, "date.yesterday").into(),
        2 => tr(language, "date.day_before_yesterday").into(),
        3.. if days <= weekday + 7 => {
            let name = tr(
                language,
                match played.weekday() {
                    time::Weekday::Monday => "date.monday",
                    time::Weekday::Tuesday => "date.tuesday",
                    time::Weekday::Wednesday => "date.wednesday",
                    time::Weekday::Thursday => "date.thursday",
                    time::Weekday::Friday => "date.friday",
                    time::Weekday::Saturday => "date.saturday",
                    time::Weekday::Sunday => "date.sunday",
                },
            );
            if days <= weekday {
                name.into()
            } else {
                format_text(language, "date.last_week", &[("weekday", name)])
            }
        }
        _ => played.to_string(),
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

    #[test]
    fn recent_dates_use_calendar_weeks_and_prioritize_the_last_three_days() {
        let today = calendar_date("2026-11-09").unwrap(); // 周一
        for (played, expected) in [
            ("2026-11-09", "今天"),
            ("2026-11-08", "昨天"),
            ("2026-11-07", "前天"),
            ("2026-11-06", "上周五"),
            ("2026-11-02", "上周一"),
            ("2026-11-01", "2026-11-01"), // 不足两周，但已是上上周。
            ("2026-10-26", "2026-10-26"),
            ("2026-11-10", "2026-11-10"),
        ] {
            assert_eq!(
                relative_date(Language::Chinese, calendar_date(played).unwrap(), today),
                expected,
                "played={played}"
            );
        }
    }

    #[test]
    fn recent_dates_cover_every_weekday_and_the_previous_week_boundary() {
        let today = calendar_date("2026-11-15").unwrap(); // 周日
        for (played, expected) in [
            ("2026-11-09", "周一"),
            ("2026-11-10", "周二"),
            ("2026-11-11", "周三"),
            ("2026-11-12", "周四"),
            ("2026-11-02", "上周一"),
            ("2026-11-03", "上周二"),
            ("2026-11-04", "上周三"),
            ("2026-11-05", "上周四"),
            ("2026-11-06", "上周五"),
            ("2026-11-07", "上周六"),
            ("2026-11-08", "上周日"),
            ("2026-11-01", "2026-11-01"),
        ] {
            assert_eq!(
                relative_date(Language::Chinese, calendar_date(played).unwrap(), today),
                expected,
                "played={played}"
            );
        }
    }

    #[test]
    fn recent_dates_handle_year_boundaries_and_all_languages() {
        let today = calendar_date("2027-01-04").unwrap();
        for (language, yesterday, previous_week) in [
            (Language::Chinese, "昨天", "上周四"),
            (Language::Japanese, "昨日", "先週の木曜日"),
            (Language::English, "Yesterday", "Last Thursday"),
        ] {
            assert_eq!(
                relative_date(language, calendar_date("2027-01-03").unwrap(), today),
                yesterday
            );
            assert_eq!(
                relative_date(language, calendar_date("2026-12-31").unwrap(), today),
                previous_week
            );
            assert_eq!(
                relative_date(language, calendar_date("2026-12-27").unwrap(), today),
                "2026-12-27"
            );
        }
    }

    #[test]
    fn invalid_calendar_dates_and_timestamps_are_not_relative_labels() {
        for text in ["", "invalid", "2026-13-01", "2026-02-29", "2026-11-00"] {
            assert!(calendar_date(text).is_none(), "{text}");
        }
        assert_eq!(recent_date(Language::Chinese, u64::MAX), "");
    }
}
