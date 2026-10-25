//! Library filters, selection and collection drafts.
use crate::{library::Dialog, library::Failure, models::Game};
use std::collections::BTreeSet;

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
            scanning: false,
            scan_folders: 0,
            visible_count: 120,
            dialog: None,
        }
    }
}
