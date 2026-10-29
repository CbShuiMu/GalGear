//! Library filters, selection and collection drafts.
use crate::{library::Dialog, library::Failure, models::Game};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct StatusFilter {
    pub(crate) favorite: Option<bool>,
    pub(crate) finished: Option<bool>,
}

impl StatusFilter {
    pub(crate) fn is_empty(self) -> bool {
        self.favorite.is_none() && self.finished.is_none()
    }

    pub(crate) fn options(self) -> [(&'static str, &'static str, bool); 4] {
        [
            ("favorite", "tags.favorited", self.favorite == Some(true)),
            (
                "not-favorite",
                "tags.not_favorited",
                self.favorite == Some(false),
            ),
            ("finished", "tags.finished", self.finished == Some(true)),
            (
                "not-finished",
                "tags.not_finished",
                self.finished == Some(false),
            ),
        ]
    }

    pub(crate) fn toggle(&mut self, value: &str) {
        let (field, enabled) = match value {
            "favorite" => (&mut self.favorite, true),
            "not-favorite" => (&mut self.favorite, false),
            "finished" => (&mut self.finished, true),
            "not-finished" => (&mut self.finished, false),
            _ => return,
        };
        *field = if *field == Some(enabled) {
            None
        } else {
            Some(enabled)
        };
    }
}

pub(crate) struct State {
    pub(crate) collection: Option<String>,
    pub(crate) expanded: BTreeSet<String>,
    pub(crate) tree_open: bool,
    pub(crate) batch: bool,
    pub(crate) checked: BTreeSet<String>,
    pub(crate) checked_folders: BTreeSet<String>,
    pub(crate) picker_games: BTreeSet<String>,
    pub(crate) batch_failures: Vec<Failure>,
    pub(crate) last_batch: Option<(String, Option<String>)>,
    pub(crate) excluded: Vec<Game>,
    pub(crate) search: String,
    pub(crate) path_input: String,
    pub(crate) tag_filter: Vec<String>,
    pub(crate) status_filter: StatusFilter,
    pub(crate) scanning: bool,
    pub(crate) scan_folders: usize,
    pub(crate) visible_count: usize,
    pub(crate) dialog: Option<Dialog>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            collection: None,
            expanded: BTreeSet::new(),
            tree_open: true,
            batch: false,
            checked: BTreeSet::new(),
            checked_folders: BTreeSet::new(),
            picker_games: BTreeSet::new(),
            batch_failures: Vec::new(),
            last_batch: None,
            excluded: Vec::new(),
            search: String::new(),
            path_input: String::new(),
            tag_filter: Vec::new(),
            status_filter: StatusFilter::default(),
            scanning: false,
            scan_folders: 0,
            visible_count: 120,
            dialog: None,
        }
    }
}

impl State {
    pub(crate) fn has_filters(&self) -> bool {
        !self.tag_filter.is_empty() || !self.status_filter.is_empty()
    }

    pub(crate) fn clear_filters(&mut self) {
        self.tag_filter.clear();
        self.status_filter = StatusFilter::default();
    }
}
