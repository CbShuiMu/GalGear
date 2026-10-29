//! Request lifecycle and the single UI blocking policy.
use crate::{ai, bridge::request, cloud, state::App};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(crate) enum Pending {
    Ai(ai::Request),
    Cloud(cloud::Request),
    SummarySetup(String, bool),
    Summary(String),
    RouteMapPrepare(String),
    RouteMap(String),
    SetupSearch(String),
    SetupExecutable(String),
    SetupSave(String),
    Bootstrap,
    Scan,
    Folder,
    Add,
    Remove,
    Preferences,
    Tools,
    Display,
    Background,
    Theme,
    Language,
    Cover,
    CoverOptions,
    Launch,
    Open,
    History(String),
    DataSources(String),
    RestorePreview,
    Runtime(String),
    CollectionSave,
    Batch,
    Excluded,
    ConfirmSave(String),
    Upscale,
    BackgroundScan,
    Saves,
    ChooseSave,
    Edit,
    DetailSaved,
    PlayStatus,
    MetadataField(String),
    Manual,
    Exclude(String),
    SearchMetadata(String, String),
    SelectMetadata,
    Resources,
    PatchPreview,
    PatchList,
    LocalPatchList(String),
    LocalPatchApply(String, bool),
    Enrich,
    Update,
    PatchTarget,
    Credentials,
    CredentialsSaved,
    CredentialsCleared,
    Account,
    FanCache,
    DownloadPreview,
    Download,
    GameIcons,
}

impl Pending {
    fn blocks_ui(&self) -> bool {
        !matches!(
            self,
            Pending::Ai(_)
                | Pending::Cloud(_)
                | Pending::SummarySetup(..)
                | Pending::Summary(_)
                | Pending::RouteMapPrepare(_)
                | Pending::RouteMap(_)
                | Pending::Scan
                | Pending::BackgroundScan
                | Pending::Saves
                | Pending::LocalPatchList(_)
                | Pending::SearchMetadata(..)
                | Pending::CoverOptions
                | Pending::Resources
                | Pending::SelectMetadata
                | Pending::Enrich
                | Pending::GameIcons
                | Pending::Credentials
                | Pending::CredentialsSaved
                | Pending::CredentialsCleared
                | Pending::Account
                | Pending::FanCache
                | Pending::DownloadPreview
                | Pending::Download
                | Pending::History(_)
                | Pending::RestorePreview
                | Pending::Runtime(_)
                | Pending::ConfirmSave(_)
                | Pending::Display
                | Pending::Theme
                | Pending::SetupSearch(_)
                | Pending::SetupExecutable(_)
                | Pending::SetupSave(_)
        )
    }
}

/// Owns request identity and derives blocking state from the outstanding work.
#[derive(Default)]
pub(crate) struct RequestQueue {
    pending: BTreeMap<u32, Pending>,
    last_id: u32,
}

impl RequestQueue {
    pub(crate) fn next_id(&self) -> u32 {
        self.last_id + 1
    }

    pub(crate) fn last_id(&self) -> u32 {
        self.last_id
    }

    fn enqueue(&mut self, kind: Pending) -> u32 {
        self.last_id = self.next_id();
        self.pending.insert(self.last_id, kind);
        self.last_id
    }

    pub(crate) fn complete(&mut self, id: u32) -> Option<Pending> {
        self.pending.remove(&id)
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &Pending> {
        self.pending.values()
    }

    pub(crate) fn has_newer_display(&self, id: u32) -> bool {
        self.pending
            .iter()
            .any(|(other, kind)| *other > id && matches!(kind, Pending::Display))
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.values().any(Pending::blocks_ui)
    }
}

pub(crate) fn issue(app: &mut App, kind: Pending, command: &str, payload: serde_json::Value) {
    if matches!(kind, Pending::Scan | Pending::BackgroundScan) {
        app.library.scanning = true;
    }
    if matches!(kind, Pending::Enrich) {
        app.enrichment_progress = None;
    }
    if matches!(kind, Pending::Download) {
        app.downloads.notice = None;
    }
    let id = app.requests.enqueue(kind);
    request(id, command, &payload.to_string());
}

#[cfg(test)]
mod tests {
    use super::{Pending, RequestQueue};

    #[test]
    fn out_of_order_completion_keeps_the_ui_blocked_until_all_blocking_work_finishes() {
        let mut requests = RequestQueue::default();
        let scan = requests.enqueue(Pending::Scan);
        let first = requests.enqueue(Pending::Bootstrap);
        let second = requests.enqueue(Pending::Cover);
        assert!(requests.is_busy());

        requests.complete(second);
        assert!(requests.is_busy());
        assert!(requests.complete(second).is_none());
        assert!(requests.is_busy());

        requests.complete(first);
        assert!(!requests.is_busy());
        assert!(matches!(requests.complete(scan), Some(Pending::Scan)));
        assert!(requests.enqueue(Pending::Open) > second);
    }

    #[test]
    fn only_pending_newer_display_requests_supersede_a_display_response() {
        let mut requests = RequestQueue::default();
        let old = requests.enqueue(Pending::Display);
        let unrelated = requests.enqueue(Pending::Theme);
        assert!(!requests.has_newer_display(old));
        let newer = requests.enqueue(Pending::Display);
        assert!(requests.has_newer_display(old));
        assert!(!requests.has_newer_display(newer));
        requests.complete(newer);
        assert!(!requests.has_newer_display(old));
        assert!(matches!(requests.complete(unrelated), Some(Pending::Theme)));
        assert!(!requests.has_newer_display(u32::MAX));
    }

    #[test]
    fn cloud_transfers_disable_duplicate_cloud_actions_without_blocking_navigation() {
        let mut app = crate::state::App::default();
        let id = app
            .requests
            .enqueue(Pending::Cloud(crate::cloud::Request::Test));
        assert!(!app.busy());
        assert!(crate::cloud::busy(&app));
        assert!(crate::cloud::dispatch(&mut app, "cloud-test", ""));
        assert_eq!(app.requests.last_id(), id);
        assert!(!crate::cloud::choice_enabled(&app, "cloud-game"));
        app.requests.complete(id);
        assert!(crate::cloud::choice_enabled(&app, "cloud-game"));
    }
}
