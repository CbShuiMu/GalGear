use galgear_i18n::{Language, Message, format_text, tr};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};
use wasm_bindgen::prelude::*;
mod choices;
mod context_menu;
mod detail;
mod detail_form;
mod features;
mod icons;
mod library;
mod preferences;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    fn render(html: &str);
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    fn reset_scroll();
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    fn request(id: u32, command: &str, payload: &str);
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    fn schedule_toast_expiry(token: u32, milliseconds: u32);
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    fn schedule_download_notice_expiry(token: u32, milliseconds: u32);
}

#[derive(Clone, Default, Deserialize)]
struct Settings {
    library_folders: Vec<String>,
    #[serde(default)]
    collections: Vec<library::Collection>,
    #[serde(default)]
    excluded_games: Vec<String>,
    card_size: u32,
    sort_by: String,
    #[serde(default)]
    language: Language,
    #[serde(default)]
    game_edits: BTreeMap<String, GameEdit>,
    #[serde(default)]
    launch_behavior: String,
    #[serde(default)]
    close_behavior: String,
    #[serde(default)]
    release_page: String,
    #[serde(default)]
    download_folder: String,
    #[serde(default)]
    fan_domain: String,
    #[serde(default = "true_value")]
    show_patches: bool,
    #[serde(default = "default_font")]
    font_size: u32,
    #[serde(default)]
    backup_folder: String,
    #[serde(default = "default_retention")]
    backup_retention: usize,
    #[serde(default)]
    locale_emulator: String,
    #[serde(default)]
    magpie: String,
    #[serde(default)]
    magpie_hotkey: String,
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default = "default_source")]
    metadata_provider: String,
}
fn default_source() -> String {
    "2dfan".into()
}
fn default_theme() -> String {
    "light".into()
}
fn default_font() -> u32 {
    16
}
fn default_retention() -> usize {
    10
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct FanCredentials {
    token: String,
    cookie: String,
    mode: String,
}
#[derive(Clone, Default, Deserialize)]
struct Account {
    status: String,
    username: Option<String>,
    balance: Option<u64>,
    reason: Option<Message>,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct FanStatus {
    account: Account,
    error: Option<Message>,
    refreshing: bool,
    revision: u64,
}
#[derive(Clone, Default, Deserialize)]
struct DownloadPreview {
    ticket: String,
    source_url: String,
    title: String,
    price: Option<u64>,
    balance: Option<u64>,
    requires_purchase: bool,
    can_download: bool,
    can_purchase: bool,
    #[serde(default)]
    free: bool,
    reason: Option<Message>,
}
#[derive(Clone, Default, Deserialize)]
struct Transfer {
    phase: String,
    title: String,
    received: u64,
    total: Option<u64>,
    path: Option<String>,
    error: Option<Message>,
}

#[derive(Clone, Deserialize)]
struct Game {
    id: String,
    title: String,
    folder: String,
    executable: Option<String>,
    engine: String,
    #[serde(default)]
    play_status: String,
    cover: Option<String>,
    modified: u64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "true_value")]
    available: bool,
    #[serde(default)]
    metadata: Option<Metadata>,
    #[serde(default)]
    metadata_warning: Option<Message>,
}
fn true_value() -> bool {
    true
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct GameEdit {
    title: Option<String>,
    tags: Vec<String>,
    executable: Option<String>,
    arguments: Vec<String>,
    working_directory: Option<String>,
    launcher: Option<String>,
    save_path: Option<String>,
    metadata: Option<Metadata>,
    description: Option<String>,
    metadata_overrides: BTreeMap<String, String>,
    tag_rules: BTreeMap<String, Option<String>>,
    save_paths: Vec<String>,
    auto_backup: bool,
    launch_mode: String,
    locale_profile: String,
    upscale: bool,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Metadata {
    url: String,
    title: String,
    description: String,
    developer: String,
    release_date: String,
    publisher: String,
    brand: String,
    artists: String,
    voices: String,
    writers: String,
    music: String,
    singers: String,
    tags: Vec<String>,
    links: Vec<MetadataLink>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct MetadataLink {
    field: String,
    name: String,
    url: String,
}

#[derive(Clone, Deserialize)]
struct SaveSnapshot {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    sequence: u64,
    created: u64,
    size: u64,
    note: String,
    locked: bool,
    files: Vec<SaveFile>,
}
#[derive(Clone, Deserialize)]
struct SaveFile {
    root: usize,
    path: String,
    size: u64,
}
#[derive(Clone, Deserialize)]
struct RestorePreview {
    snapshot: SaveSnapshot,
    targets: Vec<String>,
    changes: Vec<SaveChange>,
}
#[derive(Clone, Deserialize)]
struct SaveChange {
    root: usize,
    path: String,
    action: String,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Resource {
    title: String,
    url: String,
    source: String,
    description: String,
    game_title: String,
    tags: Vec<String>,
    author: String,
    uploaded_at: String,
    size: String,
}
#[derive(Clone, Default, Deserialize)]
struct SaveReport {
    locations: Vec<SaveLocation>,
    limited: bool,
}
#[derive(Clone, Default, Deserialize)]
struct SaveLocation {
    path: String,
    source: String,
    confidence: String,
    exists: bool,
}

#[derive(Clone, Deserialize)]
struct PatchPreview {
    title: String,
    game_id: String,
    target: String,
    files: Vec<PatchChange>,
    executable: Option<String>,
}
#[derive(Clone, Deserialize)]
struct PatchChange {
    path: String,
    before: Option<String>,
    size: u64,
}
#[derive(Clone, Deserialize)]
struct InstalledPatch {
    id: String,
    preview: PatchPreview,
    status: String,
}

#[derive(Clone, Default, Deserialize)]
struct Catalog {
    games: Vec<Game>,
    warnings: Vec<Message>,
}

#[derive(Deserialize)]
struct Bootstrap {
    settings: Settings,
    catalog: Catalog,
    config_path: String,
    version: String,
}

#[derive(Clone)]
enum Pending {
    Bootstrap,
    Scan,
    Folder,
    Add,
    Remove,
    Preferences,
    Display,
    Theme,
    Language,
    Cover,
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
    SearchMetadata(String),
    SelectMetadata,
    Resources,
    PatchPreview,
    PatchList,
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

struct App {
    page: String,
    collection: Option<String>,
    expanded: BTreeSet<String>,
    tree_open: bool,
    batch: bool,
    checked: BTreeSet<String>,
    checked_folders: BTreeSet<String>,
    picker_games: BTreeSet<String>,
    library_dialog: Option<library::Dialog>,
    batch_failures: Vec<library::Failure>,
    last_batch: Option<(String, Option<String>)>,
    excluded: Vec<Game>,
    settings: Settings,
    catalog: Catalog,
    search: String,
    path_input: String,
    selected: Option<String>,
    config_path: String,
    version: String,
    busy: bool,
    initialized: bool,
    toast: Option<(Message, bool)>,
    requests: BTreeMap<u32, Pending>,
    next_request: u32,
    toast_generation: u32,
    theme_request: u32,
    detail_editor: Option<detail::Editor>,
    detail_editing: bool,
    settings_section: String,
    context_menu: Option<context_menu::Menu>,
    history_open: bool,
    history: Vec<SaveSnapshot>,
    history_game: Option<String>,
    data_sources: Vec<preferences::SourceStatus>,
    metadata_source: String,
    restore_preview: Option<RestorePreview>,
    runtime: BTreeMap<String, String>,
    confirmation: Option<(String, String)>,
    saves: BTreeMap<String, SaveReport>,
    form: BTreeMap<String, String>,
    candidates: BTreeMap<String, Vec<Resource>>,
    resources: Vec<Resource>,
    resource_warnings: Vec<Message>,
    tag_filter: String,
    scanning: bool,
    scan_folders: usize,
    patch_game: String,
    patch_search_game: String,
    patch_preview: Option<PatchPreview>,
    installed_patches: Vec<InstalledPatch>,
    metadata_warnings: BTreeMap<String, Message>,
    update: Option<(String, String)>,
    visible_count: usize,
    patch_target: Option<String>,
    credentials: FanCredentials,
    account: Account,
    fan_refreshing: bool,
    fan_cache_error: Option<Message>,
    fan_revision: u64,
    download_preview: Option<DownloadPreview>,
    transfer: Option<Transfer>,
    download_notice: Option<Transfer>,
    download_notice_generation: u32,
    patch_source: String,
    resources_searched: bool,
    game_icons: BTreeMap<String, String>,
    choice: Option<String>,
    choice_query: String,
    choice_active: usize,
    enrichment_progress: Option<Message>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            page: "games".into(),
            collection: None,
            expanded: BTreeSet::new(),
            tree_open: true,
            batch: false,
            checked: BTreeSet::new(),
            checked_folders: BTreeSet::new(),
            picker_games: BTreeSet::new(),
            library_dialog: None,
            batch_failures: Vec::new(),
            last_batch: None,
            excluded: Vec::new(),
            settings: Settings {
                card_size: 200,
                font_size: 16,
                backup_retention: 10,
                theme: default_theme(),
                sort_by: "title".into(),
                metadata_provider: default_source(),
                ..Default::default()
            },
            catalog: Catalog::default(),
            search: String::new(),
            path_input: String::new(),
            selected: None,
            config_path: String::new(),
            version: "0.1.0".into(),
            busy: true,
            initialized: false,
            toast: None,
            requests: BTreeMap::new(),
            next_request: 0,
            toast_generation: 0,
            theme_request: 0,
            detail_editor: None,
            detail_editing: false,
            settings_section: "account".into(),
            context_menu: None,
            history_open: false,
            history: Vec::new(),
            history_game: None,
            data_sources: Vec::new(),
            metadata_source: default_source(),
            restore_preview: None,
            runtime: BTreeMap::new(),
            confirmation: None,
            saves: BTreeMap::new(),
            form: BTreeMap::new(),
            candidates: BTreeMap::new(),
            resources: Vec::new(),
            resource_warnings: Vec::new(),
            tag_filter: String::new(),
            scanning: false,
            scan_folders: 0,
            patch_game: String::new(),
            patch_search_game: String::new(),
            patch_preview: None,
            installed_patches: Vec::new(),
            metadata_warnings: BTreeMap::new(),
            update: None,
            visible_count: 120,
            patch_target: None,
            credentials: FanCredentials {
                mode: "csrf".into(),
                ..Default::default()
            },
            account: Account::default(),
            fan_refreshing: false,
            fan_cache_error: None,
            fan_revision: 0,
            download_preview: None,
            transfer: None,
            download_notice: None,
            download_notice_generation: 0,
            patch_source: "all".into(),
            resources_searched: false,
            game_icons: BTreeMap::new(),
            choice: None,
            choice_query: String::new(),
            choice_active: 0,
            enrichment_progress: None,
        }
    }
}

thread_local! { static APP: RefCell<App> = RefCell::new(App::default()); }

fn apply_settings(app: &mut App, mut settings: Settings, request_id: u32) {
    if request_id < app.theme_request {
        settings.theme.clone_from(&app.settings.theme);
    }
    app.settings = settings;
    if app.page == "patches" && !app.settings.show_patches {
        app.page = "games".into();
    }
}

fn show_toast(app: &mut App, message: Message, error: bool) {
    app.toast_generation = app.toast_generation.wrapping_add(1);
    app.toast = Some((message, error));
    schedule_toast_expiry(app.toast_generation, 4000);
}

#[wasm_bindgen]
pub fn runtime_updated(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(status) = serde_json::from_str(payload) {
            app.runtime = status;
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn tool_failed(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        show_toast(&mut app, Message::from_error(payload), true);
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn backup_completed(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(payload) {
            if !data["error"].is_null()
                && let Ok(error) = serde_json::from_value(data["error"].clone())
            {
                show_toast(&mut app, error, true);
            }
            if data["id"].as_str() == app.selected.as_deref() {
                let id = app.selected.clone();
                if let Some(id) = id {
                    issue(
                        &mut app,
                        Pending::History(id.clone()),
                        "save_history",
                        json!({"id":id}),
                    );
                }
            }
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn expire_toast(token: u32) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.toast_generation == token && app.toast.is_some() {
            app.toast = None;
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn expire_download_notice(token: u32) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.download_notice_generation == token && app.download_notice.is_some() {
            app.download_notice = None;
            paint(&app);
        }
    });
}

fn issue(app: &mut App, kind: Pending, command: &str, payload: serde_json::Value) {
    app.next_request += 1;
    let id = app.next_request;
    if matches!(kind, Pending::Scan | Pending::BackgroundScan) {
        app.scanning = true;
    }
    if matches!(kind, Pending::Enrich) {
        app.enrichment_progress = None;
    }
    if matches!(kind, Pending::Download) {
        app.download_notice = None;
    }
    app.requests.insert(id, kind);
    app.busy = app.requests.values().any(|p| {
        !matches!(
            p,
            Pending::Scan
                | Pending::BackgroundScan
                | Pending::Saves
                | Pending::SearchMetadata(_)
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
        )
    });
    request(id, command, &payload.to_string());
}

#[wasm_bindgen(start)]
pub fn start() {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        issue(&mut app, Pending::Bootstrap, "bootstrap", json!({}));
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn dispatch(action: &str, value: &str) {
    if action == "choice-pick" {
        if let Ok([field, selected]) = serde_json::from_str::<[String; 2]>(value) {
            let valid = APP.with(|state| {
                let mut app = state.borrow_mut();
                let valid = app.choice.as_deref() == Some(&field)
                    && choices::options(&app, &field)
                        .iter()
                        .any(|o| o.value == selected);
                app.choice = None;
                valid
            });
            if valid {
                dispatch(&field, &selected);
            }
        }
        return;
    }
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.busy
            && !matches!(
                action,
                "page"
                    | "close-detail"
                    | "dismiss-toast"
                    | "dismiss-download-notice"
                    | "game-detail"
                    | "load-more"
                    | "choice-close"
                    | "toggle-theme"
            )
        {
            return;
        }
        if library::dispatch(&mut app, action, value) || features::dispatch(&mut app, action, value)
        {
            paint(&app);
            return;
        }
        match action {
            "toggle-theme" if !app.requests.values().any(|p| matches!(p, Pending::Theme)) => {
                let theme = if app.settings.theme == "dark" {
                    "light"
                } else {
                    "dark"
                };
                app.theme_request = app.next_request + 1;
                issue(
                    &mut app,
                    Pending::Theme,
                    "set_theme",
                    json!({"theme":theme}),
                );
            }
            "choice-toggle"
                if matches!(
                    value,
                    "sort"
                        | "folder-parent"
                        | "tag-filter"
                        | "patch-game"
                        | "patch-search-game"
                        | "fan-mode"
                ) =>
            {
                if app.choice.as_deref() == Some(value) {
                    app.choice = None;
                } else {
                    app.choice_query.clear();
                    app.choice_active = choices::filtered(&app, value)
                        .iter()
                        .position(|o| o.value == choices::current(&app, value))
                        .unwrap_or(0);
                    app.choice = Some(value.into());
                }
            }
            "choice-close" => app.choice = None,
            "page"
                if matches!(
                    value,
                    "games" | "collections" | "patches" | "settings" | "about"
                ) =>
            {
                if app.page != value {
                    reset_scroll();
                }
                library::clear_selection(&mut app);
                app.batch = false;
                app.collection = None;
                app.library_dialog = None;
                app.page = value.into();
                app.choice = None;
                app.selected = None;
                app.context_menu = None;
                detail_form::cancel(&mut app);
                if value == "patches" {
                    issue(&mut app, Pending::PatchList, "list_patches", json!({}));
                }
                if value == "settings"
                    && !app.requests.values().any(|p| {
                        matches!(
                            p,
                            Pending::Credentials
                                | Pending::CredentialsSaved
                                | Pending::CredentialsCleared
                        )
                    })
                {
                    issue(
                        &mut app,
                        Pending::Credentials,
                        "load_fan_credentials",
                        json!({}),
                    );
                    issue(
                        &mut app,
                        Pending::FanCache,
                        "get_fan_status_cache",
                        json!({}),
                    );
                }
            }
            "load-more"
                if app.selected.is_none()
                    && matches!(app.page.as_str(), "games" | "collections") =>
            {
                app.visible_count += 120
            }
            "game-detail" => {
                app.selected = Some(value.into());
                app.form.clear();
                app.detail_editing = false;
                app.context_menu = None;
                app.detail_editor = None;
                app.history_open = false;
                app.history_game = None;
                app.history.clear();
                app.confirmation = None;
                app.restore_preview = None;
                if let Some(game) = app.catalog.games.iter().find(|g| g.id == value).cloned() {
                    let mut edit = app
                        .settings
                        .game_edits
                        .get(value)
                        .cloned()
                        .unwrap_or_default();
                    edit.title = Some(game.title.clone());
                    edit.tags = game.tags.clone();
                    if edit.executable.is_none() {
                        edit.executable.clone_from(&game.executable);
                    }
                    app.form
                        .insert("edit-title".into(), edit.title.unwrap_or_default());
                    app.form.insert("edit-tags".into(), edit.tags.join(", "));
                    app.form
                        .insert("edit-exe".into(), edit.executable.unwrap_or_default());
                    app.form
                        .insert("edit-args".into(), edit.arguments.join("\n"));
                    app.form.insert(
                        "edit-workdir".into(),
                        edit.working_directory.unwrap_or_default(),
                    );
                    app.form
                        .insert("edit-launcher".into(), edit.launcher.unwrap_or_default());
                    app.form.insert("metadata-query".into(), game.title.clone());
                }
                issue(
                    &mut app,
                    Pending::Saves,
                    "detect_saves",
                    json!({"id":value}),
                );
                issue(
                    &mut app,
                    Pending::Runtime(value.into()),
                    "game_runtime",
                    json!({"id":value}),
                );
                issue(
                    &mut app,
                    Pending::History(value.into()),
                    "save_history",
                    json!({"id":value}),
                );
            }
            "close-detail" => {
                detail_form::cancel(&mut app);
                app.selected = None;
                app.detail_editor = None;
                app.history_open = false;
                app.confirmation = None;
            }
            "dismiss-toast" => app.toast = None,
            "dismiss-download-notice" => app.download_notice = None,
            "refresh" => issue(
                &mut app,
                Pending::Scan,
                "scan_library",
                json!({"full":true}),
            ),
            "cancel-scan" => issue(&mut app, Pending::Open, "cancel_scan", json!({})),
            "detect-saves" => issue(
                &mut app,
                Pending::Saves,
                "detect_saves",
                json!({"id":value}),
            ),
            "choose-save" => issue(
                &mut app,
                Pending::ChooseSave,
                "choose_save_folder",
                json!({"id":value}),
            ),
            "open-save" => {
                if let Some(id) = app.selected.clone() {
                    issue(
                        &mut app,
                        Pending::Open,
                        "open_save_folder",
                        json!({"id":id,"path":value}),
                    );
                }
            }
            "tag-filter" => {
                library::clear_selection(&mut app);
                app.tag_filter = value.into();
                app.library_dialog = None;
                app.visible_count = 120;
            }
            "manual-game" => issue(&mut app, Pending::Manual, "add_manual_game", json!({})),
            "relocate" => issue(
                &mut app,
                Pending::Manual,
                "relocate_game",
                json!({"id":value}),
            ),
            "exclude-game" => issue(
                &mut app,
                Pending::Exclude(value.into()),
                "exclude_game",
                json!({"id":value}),
            ),
            "close-behavior" => issue(
                &mut app,
                Pending::Preferences,
                "set_close_behavior",
                json!({"behavior":value}),
            ),
            "save-release" => {
                let url = app
                    .form
                    .get("release-page")
                    .cloned()
                    .unwrap_or_else(|| app.settings.release_page.clone());
                issue(
                    &mut app,
                    Pending::Preferences,
                    "set_release_page",
                    json!({"url":url}),
                );
            }
            "check-update" => issue(&mut app, Pending::Update, "check_update", json!({})),
            "fan-mode" if matches!(value, "csrf" | "bearer") => app.credentials.mode = value.into(),
            "save-fan" if !account_busy(&app) && !download_busy(&app) => {
                let value = app.credentials.clone();
                issue(
                    &mut app,
                    Pending::CredentialsSaved,
                    "save_fan_credentials",
                    json!({"value":value}),
                );
            }
            "clear-fan" if !account_busy(&app) && !download_busy(&app) => {
                issue(
                    &mut app,
                    Pending::CredentialsCleared,
                    "clear_fan_credentials",
                    json!({}),
                );
            }
            "check-fan" if !account_busy(&app) => {
                issue(&mut app, Pending::Account, "refresh_fan_status", json!({}))
            }
            "download-folder" => issue(
                &mut app,
                Pending::Preferences,
                "choose_download_folder",
                json!({}),
            ),
            "open-downloads" => issue(&mut app, Pending::Open, "open_download_folder", json!({})),
            "patch-source" if matches!(value, "all" | "2DFan" | "kun") => {
                app.patch_source = value.into()
            }
            "inspect-download" if !download_busy(&app) => {
                app.download_preview = None;
                issue(
                    &mut app,
                    Pending::DownloadPreview,
                    "inspect_fan_resource",
                    json!({"url":value}),
                );
            }
            "close-download" if !download_busy(&app) => app.download_preview = None,
            "download-free" | "purchase-download" if !download_busy(&app) => {
                if let Some(preview) = app.download_preview.clone()
                    && ((action == "download-free" && preview.can_download)
                        || (action == "purchase-download" && preview.can_purchase))
                {
                    app.transfer = Some(Transfer {
                        phase: "preparing".into(),
                        title: preview.title.clone(),
                        ..Default::default()
                    });
                    let price = if action == "purchase-download" {
                        preview.price
                    } else {
                        None
                    };
                    issue(
                        &mut app,
                        Pending::Download,
                        "start_fan_download",
                        json!({"ticket":preview.ticket,"confirmPrice":price}),
                    );
                }
            }
            "cancel-download" => issue(&mut app, Pending::Open, "cancel_fan_download", json!({})),
            "use-download" if !app.patch_game.is_empty() => {
                let id = app.patch_game.clone();
                let target = app.patch_target.clone();
                issue(
                    &mut app,
                    Pending::PatchPreview,
                    "preview_downloaded_patch",
                    json!({"id":id,"target":target}),
                );
            }
            "exe-status" => issue(
                &mut app,
                Pending::PatchList,
                "set_exe_status",
                json!({"id":value,"installed":true}),
            ),
            "launch-behavior" => issue(
                &mut app,
                Pending::Preferences,
                "set_launch_behavior",
                json!({"behavior":value}),
            ),
            "edit-game" => {
                let get = |key: &str| app.form.get(key).cloned().unwrap_or_default();
                let optional = |key: &str| {
                    let v = get(key);
                    if v.trim().is_empty() { None } else { Some(v) }
                };
                let edit = GameEdit {
                    title: Some(get("edit-title")),
                    tags: get("edit-tags")
                        .split([',', '，'])
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    executable: optional("edit-exe"),
                    arguments: get("edit-args").lines().map(str::to_owned).collect(),
                    working_directory: optional("edit-workdir"),
                    launcher: optional("edit-launcher"),
                    ..Default::default()
                };
                let locale_profile = get("locale-profile");
                issue(
                    &mut app,
                    Pending::Edit,
                    "edit_game",
                    json!({"id":value,"edit":edit,"localeProfile":locale_profile}),
                );
            }
            "metadata-search" => {
                let query = app.form.get("metadata-query").cloned().unwrap_or_default();
                let source = app.metadata_source.clone();
                issue(
                    &mut app,
                    Pending::SearchMetadata(value.into()),
                    "search_metadata",
                    json!({"query":query,"source":source}),
                );
            }
            "metadata-select" => {
                if let Some(id) = app.selected.clone() {
                    issue(
                        &mut app,
                        Pending::SelectMetadata,
                        "select_metadata",
                        json!({"id":id,"url":value}),
                    );
                }
            }
            "open-resource" => issue(
                &mut app,
                Pending::Open,
                "open_resource",
                json!({"url":value}),
            ),
            "search-patches" => {
                app.resources_searched = true;
                let query = app.form.get("patch-query").cloned().unwrap_or_default();
                issue(
                    &mut app,
                    Pending::Resources,
                    "search_patches",
                    json!({"query":query}),
                );
            }
            "patch-search-game" => {
                app.patch_search_game = value.into();
                if let Some(title) = app
                    .catalog
                    .games
                    .iter()
                    .find(|g| g.id == value)
                    .map(|g| g.title.clone())
                {
                    app.form.insert("patch-query".into(), title.clone());
                    app.resources_searched = true;
                    issue(
                        &mut app,
                        Pending::Resources,
                        "search_patches",
                        json!({"query":title}),
                    );
                }
            }
            "patch-game" => {
                app.patch_game = value.into();
                app.patch_preview = None;
                app.patch_target = None;
            }
            "patch-target" => {
                let id = app.patch_game.clone();
                let target = app.patch_target.clone();
                issue(
                    &mut app,
                    Pending::PatchTarget,
                    "choose_patch_target",
                    json!({"id":id,"target":target}),
                );
            }
            "choose-patch" => {
                let id = app.patch_game.clone();
                let target = app.patch_target.clone();
                issue(
                    &mut app,
                    Pending::PatchPreview,
                    "preview_patch",
                    json!({"id":id,"target":target}),
                );
            }
            "install-patch" => issue(&mut app, Pending::PatchList, "install_patch", json!({})),
            "restore-patch" => issue(
                &mut app,
                Pending::PatchList,
                "restore_patch",
                json!({"id":value}),
            ),
            "run-patch" => issue(
                &mut app,
                Pending::PatchList,
                "run_patch_installer",
                json!({}),
            ),
            "choose-folder" => issue(
                &mut app,
                Pending::Folder,
                "choose_library_folder",
                json!({}),
            ),
            "add-folder" => {
                let path = app.path_input.trim().to_owned();
                if path.is_empty() {
                    show_toast(&mut app, Message::new("error.folder_required"), true);
                } else {
                    issue(
                        &mut app,
                        Pending::Add,
                        "add_library_folder",
                        json!({ "path": path }),
                    );
                }
            }
            "remove-folder" => issue(
                &mut app,
                Pending::Remove,
                "remove_library_folder",
                json!({"path": value}),
            ),
            "sort" => {
                let size = app.settings.card_size;
                issue(
                    &mut app,
                    Pending::Preferences,
                    "save_preferences",
                    json!({"cardSize":size,"sortBy":value}),
                );
            }
            "language" if Language::from_code(value).is_some() => issue(
                &mut app,
                Pending::Language,
                "set_language",
                json!({"language": value}),
            ),
            "choose-cover" => issue(
                &mut app,
                Pending::Cover,
                "choose_cover",
                json!({"id":value}),
            ),
            "unused-launch" => issue(
                &mut app,
                Pending::Launch,
                "launch_game",
                json!({"id":value}),
            ),
            "open-folder" => issue(
                &mut app,
                Pending::Open,
                "open_game_folder",
                json!({"id":value}),
            ),
            _ => {}
        }
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn game_context(id: &str, x: f64, y: f64) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.busy && app.selected.is_none() && app.catalog.games.iter().any(|g| g.id == id) {
            app.choice = None;
            app.context_menu = Some(context_menu::Menu {
                id: id.into(),
                x,
                y,
            });
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn library_marquee(payload: &str, additive: bool) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.batch || app.busy {
            return;
        }
        if let Ok(ids) = serde_json::from_str::<BTreeSet<String>>(payload) {
            let matching: BTreeSet<_> = library::matching(&app)
                .iter()
                .map(|g| g.id.clone())
                .collect();
            if !additive {
                app.checked.clear();
            }
            app.checked.extend(ids.intersection(&matching).cloned());
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn library_drag_ids(id: &str) -> String {
    APP.with(|state| {
        let app = state.borrow();
        let ids: Vec<_> = if app.batch && app.checked.contains(id) {
            app.checked.iter().map(String::as_str).collect()
        } else {
            vec![id]
        };
        serde_json::to_string(&ids).unwrap_or_default()
    })
}

#[wasm_bindgen]
pub fn library_drop(collection: &str, payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.busy {
            return;
        }
        if let Ok(ids) = serde_json::from_str::<Vec<String>>(payload) {
            if ids.len() > 100_000 {
                return;
            }
            app.library_dialog = Some(library::Dialog::Picker { ids });
            library::dispatch(&mut app, "collection-pick", collection);
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn choice_key(field: &str, key: &str) -> bool {
    let mut selected = None;
    let handled = APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.choice.is_none()
            && matches!(key, "ArrowDown" | "ArrowUp")
            && !app.busy
            && matches!(
                field,
                "sort" | "tag-filter" | "patch-game" | "patch-search-game" | "fan-mode"
            )
        {
            app.choice_query.clear();
            app.choice = Some(field.into());
            app.choice_active = choices::filtered(&app, field)
                .iter()
                .position(|o| o.value == choices::current(&app, field))
                .unwrap_or(0);
            paint(&app);
            return true;
        }
        let Some(open) = app.choice.clone() else {
            return false;
        };
        let options = choices::filtered(&app, &open);
        let last = options.len().saturating_sub(1);
        match key {
            "Escape" | "Tab" => {
                app.choice = None;
                paint(&app);
                return key == "Escape";
            }
            "ArrowDown" => app.choice_active = (app.choice_active + 1).min(last),
            "ArrowUp" => app.choice_active = app.choice_active.saturating_sub(1),
            "Home" => app.choice_active = 0,
            "End" => app.choice_active = last,
            "PageDown" => app.choice_active = (app.choice_active + 8).min(last),
            "PageUp" => app.choice_active = app.choice_active.saturating_sub(8),
            "Enter" => {
                if let Some(option) = options.get(app.choice_active) {
                    selected = Some(
                        serde_json::to_string(&[open.as_str(), option.value.as_str()]).unwrap(),
                    );
                }
            }
            _ => return false,
        }
        if selected.is_none() {
            paint(&app);
        }
        true
    });
    if let Some(value) = selected {
        dispatch("choice-pick", &value);
    }
    handled
}

#[wasm_bindgen]
pub fn input_changed(field: &str, value: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Some(metadata_field) = field.strip_prefix("draft-meta-") {
            app.form.remove(&format!("draft-reset-{metadata_field}"));
        }
        match field {
            "choice-query" if app.choice.is_some() => {
                app.choice_query = value.into();
                app.choice_active = 0;
                paint(&app);
                return;
            }
            "fan-token" => {
                app.credentials.token = value.into();
                return;
            }
            "fan-cookie" => {
                app.credentials.cookie = value.into();
                return;
            }
            "search" => {
                library::clear_selection(&mut app);
                app.search = value.into();
                app.visible_count = 120;
            }
            "folder-name" => {
                app.form.insert(field.into(), value.into());
                return;
            }
            "folder-query" | "tag-query" | "game-query" => {
                app.form.insert(field.into(), value.into());
            }
            "library-path" => app.path_input = value.into(),
            key if key.starts_with("draft-")
                || key.starts_with("edit-")
                || key.starts_with("meta-")
                || key.starts_with("tool-")
                || matches!(
                    key,
                    "title-draft"
                        | "backup-note"
                        | "tag-name"
                        | "tag-new"
                        | "locale-profile"
                        | "backup-retention"
                        | "backup-folder"
                        | "metadata-query"
                        | "patch-query"
                        | "release-page"
                ) =>
            {
                app.form.insert(key.into(), value.into());
                if key == "patch-query"
                    && !app
                        .catalog
                        .games
                        .iter()
                        .any(|g| g.id == app.patch_search_game && g.title == value)
                {
                    app.patch_search_game.clear();
                }
                return;
            }
            _ => return,
        }
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn resolve(id: u32, success: bool, payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        let Some(kind) = app.requests.remove(&id) else {
            return;
        };
        if matches!(kind, Pending::Enrich) {
            app.enrichment_progress = None;
        }
        if matches!(kind, Pending::Scan | Pending::BackgroundScan) {
            app.scanning = false;
        }
        app.busy = app.requests.values().any(|p| {
            !matches!(
                p,
                Pending::Scan
                    | Pending::BackgroundScan
                    | Pending::Saves
                    | Pending::SearchMetadata(_)
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
            )
        });
        if !success {
            if matches!(kind, Pending::Download) {
                if let Some(transfer) = &mut app.transfer {
                    transfer.phase = "error".into();
                    transfer.error = Some(Message::from_error(payload));
                }
                app.download_preview = None;
                issue(&mut app, Pending::Account, "refresh_fan_status", json!({}));
            }
            if matches!(kind, Pending::Account) {
                app.account = Account {
                    status: "error".into(),
                    reason: Some(Message::from_error(payload)),
                    ..Default::default()
                };
            }
            show_toast(&mut app, Message::from_error(payload), true);
        } else {
            let handled: Result<(), String> = (|| {
                match kind {
                    Pending::CollectionSave => {
                        let settings = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, settings, id);
                        if app
                            .collection
                            .as_ref()
                            .is_some_and(|id| !app.settings.collections.iter().any(|f| &f.id == id))
                        {
                            app.collection = None;
                            library::clear_selection(&mut app);
                        }
                        app.checked_folders.clear();
                        app.library_dialog = None;
                        app.choice = None;
                        show_toast(&mut app, Message::new("notice.saved"), false);
                    }
                    Pending::Excluded => {
                        app.excluded = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    }
                    Pending::Batch => {
                        let result: library::BatchResult =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, result.settings, id);
                        app.catalog = result.catalog;
                        for done in &result.succeeded {
                            app.checked.remove(done);
                        }
                        app.excluded
                            .retain(|game| !result.succeeded.contains(&game.id));
                        app.batch_failures = result.failed;
                        if app.batch {
                            let failed_ids: Vec<_> =
                                app.batch_failures.iter().map(|f| f.id.clone()).collect();
                            app.checked.extend(failed_ids);
                        }
                        app.library_dialog = if app.batch_failures.is_empty() {
                            None
                        } else {
                            Some(library::Dialog::Failures)
                        };
                        if app
                            .selected
                            .as_ref()
                            .is_some_and(|id| !app.catalog.games.iter().any(|g| &g.id == id))
                        {
                            app.selected = None;
                        }
                        let notice =
                            if result.unchanged > 0 && result.unchanged == result.succeeded.len() {
                                Message::new("folders.already_added")
                            } else {
                                Message::new("bulk.result")
                                    .arg("success", result.succeeded.len())
                                    .arg("failed", app.batch_failures.len())
                            };
                        let failed = !app.batch_failures.is_empty();
                        show_toast(&mut app, result.cache_warning.unwrap_or(notice), failed);
                    }
                    Pending::History(game_id) => {
                        if app.selected.as_deref() != Some(&game_id) {
                            return Ok(());
                        }
                        app.history = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.history_game = Some(game_id);
                        app.restore_preview = None;
                    }
                    Pending::DataSources(source) => {
                        app.data_sources =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if !source.is_empty() {
                            let prefix = format!("source-{source}-");
                            app.form.retain(|key, _| !key.starts_with(&prefix));
                        }
                    }
                    Pending::RestorePreview => {
                        app.restore_preview =
                            Some(serde_json::from_str(payload).map_err(|e| e.to_string())?);
                    }
                    Pending::Runtime(game_id) => {
                        app.runtime.insert(
                            game_id,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                        );
                    }
                    Pending::ConfirmSave(game_id) => {
                        issue(
                            &mut app,
                            Pending::Saves,
                            "detect_saves",
                            json!({"id":game_id}),
                        );
                    }
                    Pending::Upscale => {
                        show_toast(&mut app, Message::new("tools.triggered"), false)
                    }
                    Pending::GameIcons => {
                        app.game_icons = serde_json::from_str(payload).map_err(|e| e.to_string())?
                    }
                    Pending::Credentials
                    | Pending::CredentialsSaved
                    | Pending::CredentialsCleared => {
                        app.credentials =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.download_preview = None;
                        if matches!(kind, Pending::CredentialsCleared) {
                            app.account = Account::default();
                            app.fan_cache_error = None;
                            issue(
                                &mut app,
                                Pending::FanCache,
                                "get_fan_status_cache",
                                json!({}),
                            );
                        } else if matches!(kind, Pending::CredentialsSaved) {
                            app.account = Account::default();
                            app.fan_cache_error = None;
                            issue(&mut app, Pending::Account, "refresh_fan_status", json!({}));
                        }
                    }
                    Pending::Account | Pending::FanCache => {
                        apply_fan_status(
                            &mut app,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                        );
                    }
                    Pending::DownloadPreview => {
                        app.download_preview =
                            Some(serde_json::from_str(payload).map_err(|e| e.to_string())?)
                    }
                    Pending::Download => {
                        app.transfer =
                            Some(serde_json::from_str(payload).map_err(|e| e.to_string())?);
                        if app.transfer.as_ref().is_some_and(|t| t.phase == "complete") {
                            app.download_notice = app.transfer.clone();
                            app.download_notice_generation =
                                app.download_notice_generation.wrapping_add(1);
                            schedule_download_notice_expiry(app.download_notice_generation, 8000);
                        }
                        app.download_preview = None;
                        issue(&mut app, Pending::Account, "refresh_fan_status", json!({}));
                    }
                    Pending::Bootstrap | Pending::Add | Pending::Remove => {
                        let data: Bootstrap =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        apply_settings(&mut app, data.settings, id);
                        app.catalog = data.catalog;
                        app.config_path = data.config_path;
                        app.version = data.version;
                        app.initialized = true;
                        if matches!(kind, Pending::Bootstrap) {
                            issue(
                                &mut app,
                                Pending::DataSources(String::new()),
                                "get_data_sources",
                                json!({}),
                            );
                            issue(
                                &mut app,
                                Pending::FanCache,
                                "get_fan_status_cache",
                                json!({}),
                            );
                        }
                        if !app
                            .requests
                            .values()
                            .any(|p| matches!(p, Pending::GameIcons))
                        {
                            issue(&mut app, Pending::GameIcons, "load_game_icons", json!({}));
                        }
                        issue(&mut app, Pending::BackgroundScan, "scan_library", json!({}));
                        if !matches!(kind, Pending::Bootstrap) {
                            app.path_input.clear();
                            show_toast(&mut app, Message::new("notice.library_saved"), false);
                        }
                    }
                    Pending::Scan | Pending::BackgroundScan => {
                        app.catalog = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        let excluded = app.settings.excluded_games.clone();
                        app.catalog.games.retain(|g| !excluded.contains(&g.id));
                        if !app
                            .requests
                            .values()
                            .any(|p| matches!(p, Pending::GameIcons))
                        {
                            issue(&mut app, Pending::GameIcons, "load_game_icons", json!({}));
                        }
                        let count = app.catalog.games.len();
                        show_toast(
                            &mut app,
                            Message::new("notice.scan_complete").arg("count", count),
                            false,
                        );
                        if !app.requests.values().any(|p| matches!(p, Pending::Enrich)) {
                            issue(&mut app, Pending::Enrich, "enrich_library", json!({}));
                        }
                    }
                    Pending::Folder => {
                        let path: Option<String> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(path) = path {
                            issue(
                                &mut app,
                                Pending::Add,
                                "add_library_folder",
                                json!({"path":path}),
                            );
                        }
                    }
                    Pending::Theme => {
                        let settings: Settings =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.settings.theme = settings.theme;
                    }
                    Pending::Display => {
                        if !app
                            .requests
                            .iter()
                            .any(|(other, p)| *other > id && matches!(p, Pending::Display))
                        {
                            apply_settings(
                                &mut app,
                                serde_json::from_str(payload).map_err(|e| e.to_string())?,
                                id,
                            );
                        }
                    }
                    Pending::Preferences => {
                        apply_settings(
                            &mut app,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                            id,
                        );
                    }
                    Pending::Language => {
                        apply_settings(
                            &mut app,
                            serde_json::from_str(payload).map_err(|e| e.to_string())?,
                            id,
                        );
                        show_toast(&mut app, Message::new("notice.language_saved"), false);
                    }
                    Pending::Cover => {
                        let game: Option<Game> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(game) = game {
                            if let Some(entry) =
                                app.catalog.games.iter_mut().find(|g| g.id == game.id)
                            {
                                *entry = game;
                            }
                            show_toast(&mut app, Message::new("notice.cover_saved"), false);
                        }
                    }
                    Pending::Launch => show_toast(&mut app, Message::new("notice.launched"), false),
                    Pending::Open => {}
                    Pending::Saves | Pending::ChooseSave => {
                        let data: serde_json::Value =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if !data.is_null() {
                            let game_id = data["id"].as_str().unwrap_or_default().to_owned();
                            let report = serde_json::from_value(data["report"].clone())
                                .map_err(|e| e.to_string())?;
                            app.saves.insert(game_id, report);
                        }
                    }
                    Pending::Edit
                    | Pending::DetailSaved
                    | Pending::PlayStatus
                    | Pending::MetadataField(_)
                    | Pending::SelectMetadata => {
                        let game: Game =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(entry) = app.catalog.games.iter_mut().find(|g| g.id == game.id)
                        {
                            *entry = game.clone();
                        }
                        if let Pending::MetadataField(field) = &kind {
                            let empty = Metadata::default();
                            let meta = game.metadata.as_ref().unwrap_or(&empty);
                            app.form.insert(
                                format!("meta-{field}"),
                                features::meta_value(meta, field).into(),
                            );
                        }
                        if matches!(kind, Pending::DetailSaved) {
                            detail_form::cancel(&mut app);
                        }
                        if matches!(kind, Pending::Edit) {
                            app.detail_editor = None;
                        }
                        issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
                        if matches!(kind, Pending::SelectMetadata) {
                            app.settings
                                .game_edits
                                .entry(game.id.clone())
                                .or_default()
                                .metadata
                                .clone_from(&game.metadata);
                            app.candidates.remove(&game.id);
                            if matches!(app.detail_editor, Some(detail::Editor::Association)) {
                                app.detail_editor = None;
                            }
                        }
                        show_toast(&mut app, Message::new("notice.saved"), false);
                    }
                    Pending::Manual => {
                        issue(&mut app, Pending::Preferences, "get_preferences", json!({}));
                        issue(&mut app, Pending::Enrich, "enrich_library", json!({}));
                        let game: Option<Game> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if let Some(game) = game {
                            if let Some(entry) =
                                app.catalog.games.iter_mut().find(|g| g.id == game.id)
                            {
                                *entry = game;
                            } else {
                                app.catalog.games.push(game);
                            }
                        }
                    }
                    Pending::Exclude(game_id) => {
                        app.catalog.games.retain(|g| g.id != game_id);
                        app.checked.remove(&game_id);
                        if !app.settings.excluded_games.contains(&game_id) {
                            app.settings.excluded_games.push(game_id);
                        }
                        app.selected = None;
                    }
                    Pending::SearchMetadata(game_id) => {
                        let results = serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.candidates.insert(game_id, results);
                    }
                    Pending::Resources => {
                        let data: serde_json::Value =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.resources = serde_json::from_value(data["resources"].clone())
                            .map_err(|e| e.to_string())?;
                        app.resource_warnings = serde_json::from_value(data["warnings"].clone())
                            .map_err(|e| e.to_string())?;
                    }
                    Pending::PatchPreview => {
                        app.patch_preview =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                    }
                    Pending::PatchList => {
                        app.installed_patches =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.patch_preview = None;
                    }
                    Pending::PatchTarget => {
                        let path: Option<String> =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        if path.is_some() {
                            app.patch_target = path;
                            app.patch_preview = None;
                        }
                    }
                    Pending::Enrich => {}
                    Pending::Update => {
                        let data: serde_json::Value =
                            serde_json::from_str(payload).map_err(|e| e.to_string())?;
                        app.update = Some((
                            data["version"].as_str().unwrap_or_default().into(),
                            data["url"].as_str().unwrap_or_default().into(),
                        ));
                    }
                }
                Ok(())
            })();
            if let Err(error) = handled {
                show_toast(&mut app, Message::detail("error.read_data", error), true);
            }
        }
        paint(&app);
    });
}

#[wasm_bindgen]
pub fn download_progress(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app
            .requests
            .values()
            .any(|p| matches!(p, Pending::Download))
            && let Ok(transfer) = serde_json::from_str(payload)
        {
            app.transfer = Some(transfer);
            paint(&app);
        }
    });
}

fn account_busy(app: &App) -> bool {
    app.fan_refreshing
        || app.requests.values().any(|p| {
            matches!(
                p,
                Pending::Credentials
                    | Pending::CredentialsSaved
                    | Pending::CredentialsCleared
                    | Pending::Account
            )
        })
}
fn apply_fan_status(app: &mut App, status: FanStatus) {
    if status.revision < app.fan_revision {
        return;
    }
    app.fan_revision = status.revision;
    app.account = status.account;
    app.fan_cache_error = status.error;
    app.fan_refreshing = status.refreshing;
}

#[wasm_bindgen]
pub fn fan_status_updated(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(status) = serde_json::from_str::<FanStatus>(payload) {
            apply_fan_status(&mut app, status);
            paint(&app);
        }
    });
}
fn download_busy(app: &App) -> bool {
    app.requests
        .values()
        .any(|p| matches!(p, Pending::DownloadPreview | Pending::Download))
}

#[wasm_bindgen]
pub fn scan_progress(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if !app.scanning {
            return;
        }
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(payload) {
            app.scan_folders = data["folders_scanned"].as_u64().unwrap_or(0) as usize;
            if let Ok(games) = serde_json::from_value::<Vec<Game>>(data["games"].clone()) {
                for game in games {
                    if !app.settings.excluded_games.contains(&game.id)
                        && !app.catalog.games.iter().any(|g| g.id == game.id)
                    {
                        app.catalog.games.push(game);
                    }
                }
            }
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn game_enriched(payload: &str) {
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(payload) {
            if let Ok(game) = serde_json::from_value::<Game>(data["game"].clone()) {
                app.settings
                    .game_edits
                    .entry(game.id.clone())
                    .or_default()
                    .metadata
                    .clone_from(&game.metadata);
                if let Some(entry) = app.catalog.games.iter_mut().find(|g| g.id == game.id) {
                    *entry = game;
                }
            }
            if let Some(id) = data["id"].as_str() {
                if let Ok(candidates) =
                    serde_json::from_value::<Vec<Resource>>(data["candidates"].clone())
                {
                    app.candidates.insert(id.into(), candidates);
                }
                if let Ok(warning) = serde_json::from_value::<Message>(data["warning"].clone()) {
                    app.metadata_warnings.insert(id.into(), warning);
                }
            }
            paint(&app);
        }
    });
}

#[wasm_bindgen]
pub fn enrichment_progress(payload: &str) {
    #[derive(Deserialize)]
    struct Progress {
        title: String,
        current: usize,
        total: usize,
    }
    APP.with(|state| {
        let mut app = state.borrow_mut();
        if app.requests.values().any(|p| matches!(p, Pending::Enrich))
            && let Ok(progress) = serde_json::from_str::<Progress>(payload)
        {
            app.enrichment_progress = Some(
                Message::new("network.enrich")
                    .arg("title", progress.title)
                    .arg("current", progress.current)
                    .arg("total", progress.total),
            );
            paint(&app);
        }
    });
}

fn network_activities(app: &App) -> Vec<String> {
    let mut activities = Vec::new();
    for pending in app.requests.values() {
        let key = match pending {
            Pending::SearchMetadata(_) => "network.metadata_search",
            Pending::SelectMetadata => "network.metadata_fetch",
            Pending::Resources => "network.patches",
            Pending::Update => "network.update",
            Pending::Account => "network.account",
            Pending::DownloadPreview => "network.resource",
            Pending::Enrich => {
                if let Some(message) = &app.enrichment_progress {
                    let text = message.render(app.settings.language);
                    if !activities.contains(&text) {
                        activities.push(text);
                    }
                }
                continue;
            }
            _ => continue,
        };
        let text = t(app, key).to_owned();
        if !activities.contains(&text) {
            activities.push(text);
        }
    }
    activities
}

fn network_popup(html: &mut String, app: &App) {
    let activities = network_activities(app);
    if activities.is_empty() {
        return;
    }
    write!(html, r#"<aside class="network-popup" role="status" aria-live="polite" aria-atomic="true" aria-label="{}"><span class="network-spinner" aria-hidden="true"></span><div class="network-copy"><strong>{}</strong><ul>"#, t(app,"network.title"),t(app,"network.title")).unwrap();
    for text in activities {
        write!(html, "<li>{}</li>", esc(&text)).unwrap();
    }
    html.push_str("</ul></div></aside>");
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn icon(name: &str) -> String {
    if let Some(svg) = icons::google(name) {
        return svg
            .replacen(
                "<svg",
                r#"<svg class="icon google-icon" aria-hidden="true""#,
                1,
            )
            .replace("fill=\"#1f1f1f\"", "fill=\"currentColor\"");
    }
    let body = match name {
        "top" => r#"<path d="m6 14 6-6 6 6M6 4h12"/>"#,
        "trash" => r#"<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 10v7m4-7v7"/>"#,
        "star" => {
            r#"<path d="m12 3 2.8 5.7 6.3.9-4.6 4.5 1.1 6.3-5.6-3-5.6 3 1.1-6.3L3 9.6l6.2-.9Z"/>"#
        }
        "trophy" => {
            r#"<path d="M8 3h8v6a4 4 0 0 1-8 0ZM8 5H4v2a4 4 0 0 0 4 4m8-6h4v2a4 4 0 0 1-4 4m-4 2v5m-4 3h8m-7 0v-3h6v3"/>"#
        }
        "trophy-filled" => {
            r#"<path fill-rule="evenodd" d="M7 2h10v2h4v4a5 5 0 0 1-4.35 4.96A5 5 0 0 1 13 15.9V18h4v4H7v-4h4v-2.1a5 5 0 0 1-3.65-2.94A5 5 0 0 1 3 8V4h4ZM7 6H5v2a3 3 0 0 0 2 2.83Zm10 4.83A3 3 0 0 0 19 8V6h-2Z"/>"#
        }
        "chevron" => r#"<path d="m7 10 5 5 5-5"/>"#,
        "games" => {
            r#"<rect x="3" y="5" width="18" height="14" rx="4"/><path d="M7 10v4m-2-2h4m6-1h.01M18 14h.01"/>"#
        }
        "patches" => {
            r#"<path d="M12 3v4m0 10v4M3 12h4m10 0h4"/><rect x="7" y="7" width="10" height="10" rx="3"/><path d="m10 12 1.5 1.5L14 11"/>"#
        }
        "settings" => {
            r#"<path d="m9 3-1 3-3 1-2 3 2 2-1 3 3 2 3-1 2 2 3-1 1-3 3-1 1-3-2-2 1-3-3-2-3 1-2-2Z"/><circle cx="11" cy="11" r="3"/>"#
        }
        "about" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/>"#,
        "folder" => {
            r#"<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>"#
        }
        "search" => r#"<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4 4"/>"#,
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "refresh" => {
            r#"<path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 7a7 7 0 0 1 12-2l2 3M4 16l2 3a7 7 0 0 0 12-2"/>"#
        }
        "arrow" => r#"<path d="M5 12h14m-5-5 5 5-5 5"/>"#,
        "close" => r#"<path d="m6 6 12 12M6 18 18 6"/>"#,
        "play" => r#"<path d="m9 5 11 7-11 7Z"/>"#,
        "image" => {
            r#"<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8" cy="8" r="1.5"/><path d="m3 17 5-5 4 4 4-6 5 7"/>"#
        }
        "check" => r#"<path d="m5 12 4 4L19 6"/>"#,
        "disc" => {
            r#"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="2.5"/><path d="m7 5 3 4m4 6 3 4"/>"#
        }
        "language" => {
            r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c-5 5-5 13 0 18m0-18c5 5 5 13 0 18"/>"#
        }
        "heart" => r#"<path d="M20 5c-3-3-7-1-8 1-1-2-5-4-8-1-4 4 2 10 8 14 6-4 12-10 8-14Z"/>"#,
        _ => r#"<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5Z"/>"#,
    };
    format!(
        r#"<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{body}</svg>"#
    )
}

fn brand() -> &'static str {
    r#"<svg viewBox="0 0 48 48" fill="none" aria-hidden="true"><path d="M24 8c-12-12-22 5-11 11C-4 20 6 37 16 31c-3 17 17 17 16 4 13 9 24-7 12-14C50 8 33 1 28 14Z" fill="currentColor" transform="translate(-2 2) scale(.92)"/><circle cx="24" cy="24" r="5" fill="white"/><path d="m20 24 3 3 6-7" stroke="white" stroke-width="2"/></svg>"#
}

fn button(
    action: &str,
    value: &str,
    label: &str,
    symbol: &str,
    class: &str,
    disabled: bool,
) -> String {
    format!(
        r#"<button type="button" class="button {class}" data-action="{action}" data-value="{}" {}>{}<span>{label}</span></button>"#,
        esc(value),
        if disabled { "disabled" } else { "" },
        if symbol.is_empty() {
            String::new()
        } else {
            icon(symbol)
        }
    )
}

fn t(app: &App, key: &str) -> &'static str {
    tr(app.settings.language, key)
}

fn f(app: &App, key: &str, args: &[(&str, &str)]) -> String {
    esc(&format_text(app.settings.language, key, args))
}

fn engine<'a>(app: &App, game: &'a Game) -> &'a str {
    if matches!(game.engine.as_str(), "unknown" | "本地游戏") {
        t(app, "games.local_engine")
    } else {
        &game.engine
    }
}

fn paint(app: &App) {
    if !app.initialized && app.busy {
        render(
            r#"<div class="boot-screen" aria-busy="true"><span aria-hidden="true">✦</span><strong>GalGear</strong></div>"#,
        );
        return;
    }
    let mut html = String::with_capacity(24000);
    write!(html, r#"<div class="app-shell cards-240" data-cover-scale="1.2" data-font-size="{}" data-theme="{}" lang="{}"><aside class="sidebar"><a class="brand" href="./" data-action="page" data-value="games" aria-label="GalGear" title="GalGear"><span class="brand-mark">{}</span></a><nav class="navigation" aria-label="{}">"#,
        app.settings.font_size, if app.settings.theme=="dark" {"dark"} else {"light"}, app.settings.language.code(), brand(), t(app,"nav.main")).unwrap();
    for (value, key) in [
        ("games", "nav.games"),
        ("collections", "folders.title"),
        ("patches", "nav.patches"),
    ] {
        if value != "patches" || app.settings.show_patches {
            nav(&mut html, app, value, t(app, key));
        }
    }
    write!(
        html,
        r#"</nav><div class="sidebar-bottom"><nav class="navigation" aria-label="{}">"#,
        t(app, "nav.more")
    )
    .unwrap();
    let dark = app.settings.theme == "dark";
    write!(html,r#"<button type="button" id="theme-toggle" class="nav-item theme-toggle" data-action="toggle-theme" aria-label="{}" title="{}" aria-pressed="{}" {}>{}</button>"#,t(app,if dark {"theme.dark"} else {"theme.light"}),t(app,if dark {"theme.to_light"} else {"theme.to_dark"}),dark,if app.requests.values().any(|p|matches!(p,Pending::Theme)){"disabled"}else{""},icon(if dark {"dark_mode"} else {"light_mode"})).unwrap();
    nav(&mut html, app, "settings", t(app, "nav.settings"));
    nav(&mut html, app, "about", t(app, "nav.about"));
    write!(html, r#"</nav><div class="sidebar-version">GalGear <span>v{}</span></div></div></aside><main class="main-panel"><div class="page-scroll" id="page-scroll">"#, esc(&app.version)).unwrap();
    match app.page.as_str() {
        "settings" => settings_page(&mut html, app),
        "about" => about_page(&mut html, app),
        "patches" => patches_page(&mut html, app),
        _ => games_page(&mut html, app),
    }
    html.push_str("</div>");
    if let Some((message, error)) = &app.toast {
        write!(html, r#"<div class="toast {}" role="{}">{}<span>{}</span><button class="icon-button" data-action="dismiss-toast" aria-label="{}">{}</button></div>"#,
            if *error { "error" } else { "" }, if *error { "alert" } else { "status" },
            icon(if *error { "error" } else { "check" }), esc(&message.render(app.settings.language)), t(app,"common.close"), icon("close")).unwrap();
    }
    if let Some(game) = app
        .selected
        .as_ref()
        .and_then(|id| app.catalog.games.iter().find(|g| &g.id == id))
    {
        detail(&mut html, app, game);
    }
    library::dialog(&mut html, app);
    context_menu::render(&mut html, app);
    choices::popup(&mut html, app);
    html.push_str("</main>");
    network_popup(&mut html, app);
    download_notice(&mut html, app);
    html.push_str("</div>");
    render(&html);
}

fn nav(html: &mut String, app: &App, value: &str, label: &str) {
    let active = app.page == value && (value != "games" || app.collection.is_none());
    let _ = write!(
        html,
        r#"<button type="button" class="nav-item {}" data-action="page" data-value="{value}" aria-label="{label}" title="{label}" aria-current="{}">{}</button>"#,
        if active { "active" } else { "" },
        if active { "page" } else { "false" },
        icon(if value == "collections" {
            "folder_special"
        } else {
            value
        })
    );
}

fn games_page(html: &mut String, app: &App) {
    library::page(html, app);
}

fn poster(html: &mut String, app: &App, game: &Game) {
    if let Some(cover) = &game.cover {
        write!(
            html,
            r#"<img src="{}" alt="{}" loading="lazy" decoding="async" draggable="false"/>"#,
            esc(cover),
            f(app, "cover.alt", &[("title", &game.title)])
        )
        .unwrap();
    } else {
        let monogram: String = game
            .title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect();
        write!(html,r#"<div class="poster-fallback"><span class="poster-orbit"></span><span class="poster-flower">{}</span><span class="poster-monogram">{}</span><span class="cover-hint">{} {}</span></div>"#,brand(),esc(&monogram.to_uppercase()),icon("image"),t(app,"cover.add")).unwrap();
    }
}

fn settings_page(html: &mut String, app: &App) {
    preferences::render(html, app);
}

fn segment(html: &mut String, app: &App, action: &str, value: &str, key: &str, selected: bool) {
    let _ = write!(
        html,
        r#"<button type="button" class="segment {}" data-action="{}" data-value="{}" aria-pressed="{}" {}>{}</button>"#,
        if selected { "selected" } else { "" },
        esc(action),
        esc(value),
        selected,
        if app.busy { "disabled" } else { "" },
        t(app, key)
    );
}

fn account_label(app: &App) -> &'static str {
    t(
        app,
        if account_busy(app) {
            "auth.checking"
        } else if app.account.status == "authenticated" {
            "auth.connected"
        } else if app.account.status == "unauthenticated" {
            "auth.disconnected"
        } else if app.account.status == "error" {
            "auth.failed"
        } else {
            "auth.unchecked"
        },
    )
}
fn download_folder(app: &App) -> String {
    if !app.settings.download_folder.is_empty() {
        return app.settings.download_folder.clone();
    }
    app.config_path
        .rsplit_once(['\\', '/'])
        .map(|(dir, _)| format!("{dir}\\downloads"))
        .unwrap_or_default()
}

fn about_page(html: &mut String, app: &App) {
    write!(html, r#"<header class="page-heading"><h1>{}</h1></header><section class="about-hero"><span class="about-logo">{}</span><h2>GalGear</h2><p>{}</p><p>Rust + Tauri</p></section>"#, t(app,"about.title"),brand(),t(app,"about.description")).unwrap();
}

fn resource_row(html: &mut String, app: &App, result: &Resource) {
    let fan = result.source == "2DFan";
    let logo = if fan {
        "./assets/2dfan.png"
    } else {
        "./assets/kun.webp"
    };
    write!(html,r#"<article class="resource-row resource-entry"><span class="resource-logo"><img src="{logo}" alt="{}" width="28" height="28"/></span><div class="resource-copy"><div class="resource-origin"><span class="resource-source">{}</span>"#,esc(&result.source),esc(&result.source)).unwrap();
    if !result.game_title.is_empty() {
        write!(
            html,
            "<span class=\"resource-game\">{}</span>",
            esc(&result.game_title)
        )
        .unwrap();
    }
    write!(
        html,
        "</div><h3>{}</h3><div class=\"resource-meta\">",
        esc(&result.title)
    )
    .unwrap();
    for tag in &result.tags {
        let (label, class) = match tag.as_str() {
            "ai" | "AI翻译" => (t(app, "patches.tag_ai"), "ai"),
            "manual" => (t(app, "patches.tag_manual"), ""),
            "全CG存档" => (tag.as_str(), "save"),
            _ => (tag.as_str(), ""),
        };
        write!(
            html,
            "<span class=\"resource-tag {class}\">{}</span>",
            esc(label)
        )
        .unwrap();
    }
    if !result.author.is_empty() {
        write!(
            html,
            "<span class=\"resource-author\" title=\"{}\">{}</span>",
            f(app, "patches.uploaded_by", &[("name", &result.author)]),
            esc(&result.author)
        )
        .unwrap();
    }
    if !result.uploaded_at.is_empty() {
        write!(
            html,
            "<time datetime=\"{}\" title=\"{}\">{}</time>",
            esc(&result.uploaded_at),
            esc(&result.uploaded_at),
            esc(result.uploaded_at.split('T').next().unwrap_or_default())
        )
        .unwrap();
    }
    if !result.size.is_empty() {
        write!(html, "<span>{}</span>", esc(&result.size)).unwrap();
    }
    if result.tags.is_empty()
        && result.author.is_empty()
        && result.game_title.is_empty()
        && !result.description.is_empty()
    {
        write!(html, "<span>{}</span>", esc(&result.description)).unwrap();
    }
    let free = app
        .download_preview
        .as_ref()
        .is_some_and(|p| p.source_url == result.url && p.free);
    if free {
        write!(
            html,
            "<span class=\"resource-tag free\">{}</span>",
            t(app, "download.free_cost")
        )
        .unwrap();
    }
    write!(
        html,
        "</div></div><div class=\"resource-actions\">{}{}</div></article>",
        if fan && result.url.contains("/downloads/") {
            button(
                "inspect-download",
                &result.url,
                t(app, "download.inspect"),
                "arrow",
                "secondary",
                download_busy(app),
            )
        } else {
            String::new()
        },
        button(
            "open-resource",
            &result.url,
            t(app, "patches.open"),
            "arrow",
            "quiet",
            false
        )
    )
    .unwrap();
}

fn patches_page(html: &mut String, app: &App) {
    write!(html,r#"<header class="page-heading"><h1>{}</h1>{}</header><div class="patch-workspace"><div class="patch-browser"><section class="resource-search"><div class="patch-controls">"#,t(app,"patches.title"),button("page","settings",t(app,"auth.title"),"settings","quiet",false)).unwrap();
    form_field(html, app, "patch-query", "patches.query", false);
    html.push_str(&button(
        "search-patches",
        "",
        t(app, "patches.search"),
        "search",
        "primary",
        app.requests
            .values()
            .any(|p| matches!(p, Pending::Resources)),
    ));
    html.push_str("</div><div class=\"library-search-row\">");
    write!(html, "<span>{}</span>", t(app, "patches.library_search")).unwrap();
    choices::control(
        html,
        app,
        "patch-search-game",
        t(app, "patches.library_search"),
        app.requests
            .values()
            .any(|p| matches!(p, Pending::Resources)),
    );
    write!(
        html,
        "</div><div class=\"resource-toolbar\"><div class=\"source-filters\">"
    )
    .unwrap();
    for (source, label) in [
        ("all", t(app, "patches.all_sources")),
        ("2DFan", "2DFan"),
        ("kun", t(app, "patches.kun")),
    ] {
        let count = app
            .resources
            .iter()
            .filter(|r| {
                source == "all"
                    || (source == "2DFan" && r.source == "2DFan")
                    || (source == "kun" && r.source != "2DFan")
            })
            .count();
        write!(html,r#"<button class="source-filter {}" data-action="patch-source" data-value="{source}" aria-pressed="{}">{}<span>{count}</span></button>"#,if app.patch_source==source{"selected"}else{""},app.patch_source==source,label).unwrap();
    }
    html.push_str("</div></div></section>");
    for warning in &app.resource_warnings {
        write!(
            html,
            "<p class=\"account-notice\">{}</p>",
            esc(&warning.render(app.settings.language))
        )
        .unwrap();
    }
    download_panel(html, app);
    html.push_str("<div class=\"resource-list\">");
    let results: Vec<_> = app
        .resources
        .iter()
        .filter(|r| {
            app.patch_source == "all"
                || (app.patch_source == "2DFan" && r.source == "2DFan")
                || (app.patch_source == "kun" && r.source != "2DFan")
        })
        .collect();
    for result in &results {
        resource_row(html, app, result);
    }
    if results.is_empty() {
        let searching = app
            .requests
            .values()
            .any(|p| matches!(p, Pending::Resources));
        write!(html,r#"<div class="resource-empty"><span class="resource-empty-icon">{}</span><h3>{}</h3><p>{}</p></div>"#,icon("search"),t(app,if searching{"patches.searching"}else if app.resources_searched{"patches.no_results"}else{"patches.find_resources"}),t(app,if app.resources_searched && !searching{"patches.try_query"}else{"patches.resource_help"})).unwrap();
    }
    html.push_str("</div></div>");
    write!(html,"<aside class=\"patch-install\"><section class=\"install-section\"><h3>{}</h3><div class=\"patch-controls\"><div class=\"edit-field\"><span>{}</span>",t(app,"patches.local"),t(app,"patches.target")).unwrap();
    choices::control(html, app, "patch-game", t(app, "patches.target"), app.busy);
    write!(
        html,
        "</div>{}</div>",
        button(
            "choose-patch",
            "",
            t(app, "patches.choose"),
            "plus",
            "secondary",
            app.busy || app.patch_game.is_empty()
        )
    )
    .unwrap();
    if app.transfer.as_ref().is_some_and(|d| d.phase == "complete") {
        html.push_str(&button(
            "use-download",
            "",
            t(app, "download.associate"),
            "check",
            "secondary",
            app.busy || app.patch_game.is_empty(),
        ));
    }
    html.push_str(&button(
        "patch-target",
        "",
        t(app, "patches.choose_target"),
        "folder",
        "quiet",
        app.busy || app.patch_game.is_empty(),
    ));
    if let Some(path) = &app.patch_target {
        write!(html, "<p class=\"muted\">{}</p>", esc(path)).unwrap();
    }
    if let Some(preview) = &app.patch_preview {
        write!(
            html,
            "<h3>{} · {}</h3><p>{}</p><p>{}</p>",
            t(app, "patches.preview"),
            esc(&preview.title),
            esc(&preview.target),
            esc(&app
                .catalog
                .games
                .iter()
                .find(|g| g.id == preview.game_id)
                .map(|g| g.title.clone())
                .unwrap_or_default())
        )
        .unwrap();
        if preview.executable.is_some() {
            write!(
                html,
                "<p class=\"detail-note\">{}</p>{}",
                t(app, "patches.exe_note"),
                button(
                    "run-patch",
                    "",
                    t(app, "patches.run"),
                    "play",
                    "primary",
                    app.busy
                )
            )
            .unwrap();
        } else {
            html.push_str("<ul class=\"patch-preview\">");
            for file in &preview.files {
                write!(
                    html,
                    "<li><span>{} <small>({} B)</small></span><span>{}</span></li>",
                    esc(&file.path),
                    file.size,
                    t(
                        app,
                        if file.before.is_some() {
                            "patches.conflict"
                        } else {
                            "patches.new_file"
                        }
                    )
                )
                .unwrap();
            }
            html.push_str("</ul>");
            html.push_str(&button(
                "install-patch",
                "",
                t(app, "patches.install"),
                "check",
                "primary",
                app.busy,
            ));
        }
    }
    write!(
        html,
        "</section><section class=\"install-section patch-history\"><h3>{}</h3>",
        t(app, "patches.installed")
    )
    .unwrap();
    for patch in app.installed_patches.iter().filter(|p| {
        p.status == "installed" || p.status == "installing" || p.status.starts_with("manual_")
    }) {
        write!(
            html,
            "<div class=\"candidate-row\"><span>{}<small> · {}</small></span>{}</div>",
            esc(&patch.preview.title),
            esc(&patch.preview.target),
            if patch.preview.executable.is_some() {
                button(
                    "exe-status",
                    &patch.id,
                    t(
                        app,
                        if patch.status == "manual_installed" {
                            "patches.manual_installed"
                        } else {
                            "patches.mark_installed"
                        },
                    ),
                    "check",
                    "secondary",
                    app.busy || patch.status == "manual_installed",
                )
            } else {
                button(
                    "restore-patch",
                    &patch.id,
                    t(app, "patches.restore"),
                    "refresh",
                    "secondary",
                    app.busy,
                )
            }
        )
        .unwrap();
    }
    if app.installed_patches.iter().all(|p| p.status == "restored") {
        write!(
            html,
            "<p class=\"section-empty\">{}</p>",
            t(app, "patches.history_empty")
        )
        .unwrap();
    }
    html.push_str("</section></aside></div>");
}

fn download_notice(html: &mut String, app: &App) {
    let Some(transfer) = &app.download_notice else {
        return;
    };
    write!(html,r#"<aside class="download-notice" role="status" aria-live="polite"><div class="download-notice-heading"><span>{}<strong>{}</strong></span><button class="icon-button" data-action="dismiss-download-notice" aria-label="{}">{}</button></div><p title="{}">{}</p><div class="download-notice-actions">{}</div></aside>"#,icon("check"),t(app,"download.complete"),t(app,"common.close"),icon("close"),esc(&transfer.title),esc(&transfer.title),button("open-downloads","",t(app,"download.open_folder"),"folder","quiet",app.busy)).unwrap();
}

fn download_panel(html: &mut String, app: &App) {
    if let Some(preview) = &app.download_preview {
        write!(html, r#"<section class="download-inspection"><div class="panel-heading"><h2>{}</h2><button class="icon-button" data-action="close-download" aria-label="{}" {}>{}</button></div><h3>{}</h3><dl class="download-facts"><div><dt>{}</dt><dd>{}</dd></div><div><dt>{}</dt><dd>{}</dd></div></dl>"#,
            t(app,"download.preview"),t(app,"common.close"),if download_busy(app){"disabled"}else{""},icon("close"),esc(&preview.title),t(app,"download.cost"),if preview.free {t(app,"download.free_cost").into()}else if preview.can_download {t(app,"download.available").into()}else{preview.price.map(|n|f(app,"download.points",&[("count",&n.to_string())])).unwrap_or_else(||t(app,"auth.unknown").into())},t(app,"auth.points"),preview.balance.map(|n|f(app,"download.points",&[("count",&n.to_string())])).unwrap_or_else(||t(app,"auth.unknown").into())).unwrap();
        if let Some(reason) = &preview.reason {
            write!(
                html,
                "<p class=\"account-notice\">{}</p>",
                esc(&reason.render(app.settings.language))
            )
            .unwrap();
        }
        write!(
            html,
            "<p class=\"field-help directory-value\">{}: {}</p><div class=\"detail-actions\">",
            t(app, "download.folder"),
            esc(&download_folder(app))
        )
        .unwrap();
        if preview.requires_purchase {
            let label = preview
                .price
                .map(|n| f(app, "download.purchase", &[("count", &n.to_string())]))
                .unwrap_or_else(|| t(app, "download.confirm_required").into());
            html.push_str(&button(
                "purchase-download",
                "",
                &label,
                "check",
                "primary",
                !preview.can_purchase || download_busy(app),
            ));
        } else {
            html.push_str(&button(
                "download-free",
                "",
                t(
                    app,
                    if preview.free {
                        "download.free_start"
                    } else {
                        "download.start"
                    },
                ),
                "arrow",
                "primary",
                !preview.can_download || download_busy(app),
            ));
        }
        html.push_str(&button(
            "open-resource",
            &preview.source_url,
            t(app, "patches.open"),
            "arrow",
            "quiet",
            false,
        ));
        if !preview.can_download && !preview.can_purchase {
            html.push_str(&button(
                "page",
                "settings",
                t(app, "auth.configure"),
                "settings",
                "secondary",
                false,
            ));
        }
        html.push_str("</div></section>");
    }
    if let Some(transfer) = app.transfer.as_ref().filter(|t| t.phase != "complete") {
        let active = matches!(
            transfer.phase.as_str(),
            "preparing" | "downloading" | "purchasing"
        );
        write!(html,r#"<section class="transfer-panel" aria-live="polite"><div class="panel-heading"><h2>{}</h2>{}</div><p class="transfer-title">{}</p>"#,
            t(app,match transfer.phase.as_str(){"complete"=>"download.complete","error"=>"download.failed","purchasing"=>"download.purchasing","downloading"=>"download.downloading",_=>"download.preparing"}),
            if active {button("cancel-download","",t(app,"download.cancel"),"close","quiet",transfer.phase=="purchasing")}else{button("open-downloads","",t(app,"download.open_folder"),"folder","quiet",false)},esc(&transfer.title)).unwrap();
        if active {
            let progress = transfer
                .total
                .filter(|v| *v > 0)
                .map(|v| (transfer.received * 100 / v).min(100));
            write!(html,r#"<progress max="100" {} aria-label="{}"></progress><p class="field-help">{} / {}</p>"#,progress.map(|v|format!("value=\"{v}\"")).unwrap_or_default(),t(app,"download.downloading"),bytes(transfer.received),transfer.total.map(bytes).unwrap_or_else(||t(app,"auth.unknown").into())).unwrap();
        }
        if let Some(path) = &transfer.path {
            write!(
                html,
                "<p class=\"directory-value\">{}</p><p class=\"field-help\">{}</p>",
                esc(path),
                t(app, "download.install_help")
            )
            .unwrap();
        }
        if let Some(error) = &transfer.error {
            write!(
                html,
                "<p class=\"account-notice\" role=\"alert\">{}</p>",
                esc(&error.render(app.settings.language))
            )
            .unwrap();
        }
        html.push_str("</section>");
    }
}

fn bytes(value: u64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{:.1} KiB", value as f64 / 1024.)
    } else {
        format!("{:.1} MiB", value as f64 / (1024. * 1024.))
    }
}

fn form_field(html: &mut String, app: &App, field: &str, label: &str, multiline: bool) {
    let value = app.form.get(field).map(String::as_str).unwrap_or_default();
    write!(
        html,
        "<label class=\"edit-field\" for=\"{field}\"><span>{}</span>",
        t(app, label)
    )
    .unwrap();
    if multiline {
        write!(
            html,
            "<textarea id=\"{field}\" data-input=\"{field}\" rows=\"2\">{}</textarea>",
            esc(value)
        )
        .unwrap();
    } else {
        write!(
            html,
            "<input id=\"{field}\" data-input=\"{field}\" value=\"{}\"/>",
            esc(value)
        )
        .unwrap();
    }
    html.push_str("</label>");
}

fn detail(html: &mut String, app: &App, game: &Game) {
    detail::render(html, app, game);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_are_html_escaped() {
        assert_eq!(esc("游戏\"<img>&'"), "游戏&quot;&lt;img&gt;&amp;&#39;");
    }
    #[test]
    fn configuration_edits_preserve_cached_website_tags_schema_and_cover_fields() {
        let edit: GameEdit=serde_json::from_value(json!({"metadata":{"url":"https://example.invalid/game","tags":["ADV"],"schema_version":2,"cover_url":"https://example.invalid/cover.jpg"}})).unwrap();
        let roundtrip = serde_json::to_value(edit).unwrap();
        assert_eq!(
            (
                roundtrip["metadata"]["tags"].clone(),
                roundtrip["metadata"]["schema_version"].clone(),
                roundtrip["metadata"]["cover_url"].clone()
            ),
            (
                json!(["ADV"]),
                json!(2),
                json!("https://example.invalid/cover.jpg")
            )
        );
    }
}
