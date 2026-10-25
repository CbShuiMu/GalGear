//! Retrieve bounded script candidates without claiming that code order is play history.
use super::resources::Dialogue;
use super::{invalid, io};
use galgear_i18n::Message;
use sha2::{Digest, Sha256};

const MAX_ROWS: usize = 512;
const MAX_BYTES: usize = 96_000;

pub(super) fn fingerprint() -> String {
    format!("{:x}", Sha256::digest(include_bytes!("context.rs")))
}

/// Host-signed restore labels are structural coordinates, never story messages.
pub(super) fn is_resume_marker(row: &serde_json::Value) -> bool {
    row["_native"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["kind"] == "kag_label_resume" && item["role"] == "resume_node")
    })
}

/// Keep the matched node and nearby earlier code references from its own source.
/// These are candidates for narrative reconstruction, never a verified route.
pub(super) fn candidates(lines: &[Dialogue], current: &Dialogue) -> Result<Vec<Dialogue>, Message> {
    if current.text.trim().is_empty() {
        return Err(invalid());
    }
    if lines.iter().any(|row| {
        row.source == current.source
            && row.position == current.position
            && (row.text != current.text || row.speaker != current.speaker)
    }) {
        return Err(Message::new("summary.ambiguous_anchor"));
    }
    let mut earlier: Vec<_> = lines
        .iter()
        .filter(|row| row.source == current.source && row.position < current.position)
        .collect();
    earlier.sort_unstable_by_key(|row| row.position);

    let mut selected = vec![current.clone()];
    let mut bytes = serde_json::to_vec(current).map_err(io)?.len() + 2;
    for group in earlier
        .chunk_by(|left, right| left.position == right.position)
        .rev()
    {
        let row = group[0];
        // Conflicting records at one coordinate cannot establish a candidate line.
        if group
            .iter()
            .any(|other| other.text != row.text || other.speaker != row.speaker)
        {
            continue;
        }
        let size = serde_json::to_vec(row).map_err(io)?.len() + 1;
        if selected.len() == MAX_ROWS || bytes.saturating_add(size) > MAX_BYTES {
            break;
        }
        selected.push(row.clone());
        bytes += size;
    }
    selected.reverse();
    Ok(selected)
}

/// Script context strictly before a verified restore label, without a current line.
pub(super) fn before_resume(
    lines: &[Dialogue],
    source: &str,
    position: u64,
) -> Result<Vec<Dialogue>, Message> {
    let mut earlier: Vec<_> = lines
        .iter()
        .filter(|row| row.source == source && row.position < position)
        .collect();
    earlier.sort_unstable_by_key(|row| row.position);
    let mut selected = Vec::new();
    let mut bytes = 2usize;
    for group in earlier
        .chunk_by(|left, right| left.position == right.position)
        .rev()
    {
        let row = group[0];
        if group
            .iter()
            .any(|other| other.text != row.text || other.speaker != row.speaker)
        {
            continue;
        }
        let size = serde_json::to_vec(row).map_err(io)?.len() + 1;
        if selected.len() == MAX_ROWS || bytes.saturating_add(size) > MAX_BYTES {
            break;
        }
        selected.push(row.clone());
        bytes += size;
    }
    selected.reverse();
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source: &str, position: u64, text: &str) -> Dialogue {
        Dialogue {
            source: source.into(),
            position,
            text: text.into(),
            speaker: None,
        }
    }

    #[test]
    fn restore_entry_context_excludes_the_entry_and_all_later_code() {
        let rows = [
            row("chapter", 10, "Before the entry"),
            row("chapter", 50, "Entry content"),
            row("chapter", 80, "Future branch"),
            row("other", 20, "Other route"),
        ];
        let selected = before_resume(&rows, "chapter", 50).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>(),
            ["Before the entry"]
        );
        assert!(before_resume(&rows, "chapter", 0).unwrap().is_empty());
    }

    #[test]
    fn candidates_are_sorted_and_exclude_other_sources_and_later_code() {
        let current = row("route", 200, "Current");
        let rows = [
            row("route", 300, "Later script"),
            row("other", 50, "Other route"),
            current.clone(),
            row("route", 100, "Earlier candidate"),
        ];
        let selected = candidates(&rows, &current).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>(),
            ["Earlier candidate", "Current"]
        );
    }

    #[test]
    fn candidates_keep_the_nearest_window_instead_of_an_entire_numeric_prefix() {
        let rows: Vec<_> = (0..2000).map(|p| row("source", p, "Script line")).collect();
        let selected = candidates(&rows, &rows[1000]).unwrap();
        assert_eq!(
            selected.iter().map(|row| row.position).collect::<Vec<_>>(),
            (1001 - MAX_ROWS as u64..=1000).collect::<Vec<_>>()
        );
    }

    #[test]
    fn candidates_bound_serialized_bytes_and_preserve_the_exact_current_node() {
        let text = "剧情🌙台词".repeat(400);
        let rows: Vec<_> = (0..100).map(|p| row("source", p, &text)).collect();
        let selected = candidates(&rows, &rows[99]).unwrap();
        assert!(selected.len() > 1 && selected.len() < MAX_ROWS);
        assert!(serde_json::to_vec(&selected).unwrap().len() <= MAX_BYTES);
        assert_eq!(selected.last().unwrap().text, text);
    }

    #[test]
    fn conflicting_candidates_are_omitted_but_conflicting_current_nodes_fail() {
        let current = row("source", 20, "Current");
        let mut rows = vec![
            row("source", 10, "Alternative one"),
            row("source", 10, "Alternative two"),
            current.clone(),
        ];
        assert_eq!(candidates(&rows, &current).unwrap().len(), 1);
        rows.push(row("source", 20, "Conflicting current"));
        assert_eq!(
            candidates(&rows, &current).unwrap_err().key,
            "summary.ambiguous_anchor"
        );
    }
}
