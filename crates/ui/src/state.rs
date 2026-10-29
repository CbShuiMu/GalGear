//! Application state composition and shared notifications.
use crate::{
    accounts, ai, bridge::schedule_toast_expiry, choices, cloud, context_menu, cover_picker,
    detail, downloads, game_setup, library, local_patches, models::Catalog, models::Settings,
    models::default_color_theme, models::default_source, models::default_theme, patches,
    preferences, requests::RequestQueue,
};
use galgear_i18n::Message;
use std::{cell::RefCell, collections::BTreeMap};

pub(crate) struct App {
    pub(crate) ai_available: bool,
    pub(crate) ai: ai::State,
    pub(crate) cloud: cloud::State,
    pub(crate) summary: crate::ai_summary::State,
    pub(crate) setup: game_setup::State,
    pub(crate) page: String,
    pub(crate) home_selected: Option<String>,
    pub(crate) settings: Settings,
    pub(crate) catalog: Catalog,
    pub(crate) config_path: String,
    pub(crate) version: String,
    pub(crate) initialized: bool,
    pub(crate) toast: Option<(Message, bool)>,
    pub(crate) toast_generation: u32,
    pub(crate) theme_request: u32,
    pub(crate) background_request: u32,
    pub(crate) cover_picker: cover_picker::State,
    pub(crate) settings_section: String,
    pub(crate) context_menu: Option<context_menu::Menu>,
    pub(crate) data_sources: Vec<preferences::SourceStatus>,
    pub(crate) runtime: BTreeMap<String, String>,
    pub(crate) form: BTreeMap<String, String>,
    pub(crate) local_patches: local_patches::State,
    pub(crate) update: Option<(String, String)>,
    pub(crate) game_icons: BTreeMap<String, String>,
    pub(crate) enrichment_progress: Option<Message>,
    pub(crate) requests: RequestQueue,
    pub(crate) library: library::State,
    pub(crate) detail: detail::State,
    pub(crate) account: accounts::State,
    pub(crate) downloads: downloads::State,
    pub(crate) patches: patches::State,
    pub(crate) choices: choices::State,
}

impl Default for App {
    fn default() -> Self {
        Self {
            ai_available: false,
            ai: ai::State::default(),
            cloud: cloud::State::default(),
            summary: crate::ai_summary::State::default(),
            setup: game_setup::State::default(),
            page: "home".into(),
            home_selected: None,
            settings: Settings {
                card_size: 200,
                font_size: 16,
                backup_retention: 10,
                theme: default_theme(),
                color_theme: default_color_theme(),
                sort_by: "title".into(),
                ai_provider: crate::models::default_ai_provider(),
                metadata_provider: default_source(),
                ..Default::default()
            },
            catalog: Catalog::default(),
            config_path: String::new(),
            version: env!("CARGO_PKG_VERSION").into(),
            initialized: false,
            toast: None,
            toast_generation: 0,
            theme_request: 0,
            background_request: 0,
            cover_picker: cover_picker::State::default(),
            settings_section: "account".into(),
            context_menu: None,
            data_sources: Vec::new(),
            runtime: BTreeMap::new(),
            form: BTreeMap::new(),
            local_patches: local_patches::State::default(),
            update: None,
            game_icons: BTreeMap::new(),
            enrichment_progress: None,
            requests: RequestQueue::default(),
            library: library::State::default(),
            detail: detail::State::default(),
            account: accounts::State::default(),
            downloads: downloads::State::default(),
            patches: patches::State::default(),
            choices: choices::State::default(),
        }
    }
}

impl App {
    pub(crate) fn busy(&self) -> bool {
        self.requests.is_busy()
    }
}

thread_local! { pub(crate) static APP: RefCell<App> = RefCell::new(App::default()); }

pub(crate) fn apply_settings(app: &mut App, mut settings: Settings, request_id: u32) {
    crate::ai_summary::preserve_completed_setup(&app.summary, &mut settings, request_id);
    if request_id < app.theme_request {
        settings.theme.clone_from(&app.settings.theme);
        settings.color_theme.clone_from(&app.settings.color_theme);
    }
    if request_id < app.background_request {
        settings
            .background_image
            .clone_from(&app.settings.background_image);
    }
    app.settings = settings;
    if app.page == "patches" && !app.settings.show_patches {
        app.page = "games".into();
    }
}

pub(crate) fn show_toast(app: &mut App, message: Message, error: bool) {
    app.toast_generation = app.toast_generation.wrapping_add(1);
    app.toast = Some((message, error));
    schedule_toast_expiry(app.toast_generation, 4000);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BackgroundImage;

    #[test]
    fn delayed_preferences_cannot_restore_a_background_after_reset() {
        let mut app = App::default();
        let mut older = app.settings.clone();
        older.background_image = Some(BackgroundImage {
            id: "a".repeat(64),
            name: "old.png".into(),
        });
        older.font_size = 18;
        app.background_request = 7;

        apply_settings(&mut app, older, 6);

        assert!(app.settings.background_image.is_none());
        assert_eq!(app.settings.font_size, 18);
    }

    #[test]
    fn delayed_preferences_cannot_replace_newer_mode_or_color_theme() {
        let mut app = App::default();
        app.settings.theme = "dark".into();
        app.settings.color_theme = "white-pink".into();
        app.theme_request = 7;
        let mut older = app.settings.clone();
        older.theme = "light".into();
        older.color_theme = "pink".into();
        older.font_size = 18;
        apply_settings(&mut app, older, 6);
        assert_eq!(
            (
                app.settings.theme.as_str(),
                app.settings.color_theme.as_str(),
                app.settings.font_size
            ),
            ("dark", "white-pink", 18)
        );
    }
}
