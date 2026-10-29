//! AI settings use the shared searchable model picker and independent provider requests.
use crate::{
    choices, requests::Pending, requests::issue, state::App, state::apply_settings,
    state::show_toast, view::t, widgets::button, widgets::esc,
};
use galgear_i18n::Message;
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, fmt::Write};

const PROVIDERS: [(&str, &str); 4] = [
    ("openai", "OpenAI"),
    ("deepseek", "DeepSeek"),
    ("qwen", "Qwen"),
    ("kimi", "Kimi"),
];

#[derive(Clone)]
pub(crate) enum Request {
    Statuses,
    Models(String),
    Save(String),
    Clear(String),
    Select(String),
}

#[derive(Clone, Deserialize)]
struct Model {
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct ProviderStatus {
    provider: String,
    configured: bool,
    model: String,
}

#[derive(Default)]
struct ProviderState {
    configured: bool,
    token: String,
    selected: String,
    saved_model: String,
    models: Vec<Model>,
    loaded: bool,
    error: Option<Message>,
}

pub(crate) struct State {
    providers: BTreeMap<String, ProviderState>,
    loaded: bool,
    error: Option<Message>,
    retry_provider: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            providers: PROVIDERS
                .iter()
                .map(|(id, _)| ((*id).into(), ProviderState::default()))
                .collect(),
            loaded: false,
            error: None,
            retry_provider: None,
        }
    }
}

fn busy(app: &App, provider: &str) -> bool {
    app.requests.values().any(|pending| match pending {
        Pending::Ai(Request::Statuses | Request::Select(_)) => true,
        Pending::Ai(Request::Models(id) | Request::Save(id) | Request::Clear(id)) => id == provider,
        _ => false,
    })
}

pub(crate) fn load(app: &mut App) {
    if !app.ai.loaded
        && !app
            .requests
            .values()
            .any(|pending| matches!(pending, Pending::Ai(Request::Statuses)))
    {
        app.ai.error = None;
        issue(
            app,
            Pending::Ai(Request::Statuses),
            "get_ai_providers",
            json!({}),
        );
    }
}

fn refresh(app: &mut App, provider: &str) {
    if busy(app, provider) {
        return;
    }
    let Some(state) = app.ai.providers.get_mut(provider) else {
        return;
    };
    if state.token.trim().is_empty() && !state.configured {
        return;
    }
    state.error = None;
    let token = state.token.clone();
    issue(
        app,
        Pending::Ai(Request::Models(provider.into())),
        "list_ai_models",
        json!({"provider":provider,"token":token}),
    );
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> bool {
    if action == "ai-select-provider" {
        if PROVIDERS.iter().any(|(id, _)| *id == value)
            && value != app.settings.ai_provider
            && !busy(app, value)
            && !app.busy()
        {
            app.ai.error = None;
            app.ai.retry_provider = None;
            issue(
                app,
                Pending::Ai(Request::Select(value.into())),
                "set_active_ai_provider",
                json!({"provider":value}),
            );
        }
        return true;
    }
    if action == "ai-retry" {
        if let Some(provider) = app.ai.retry_provider.clone() {
            dispatch(app, "ai-select-provider", &provider);
        } else {
            load(app);
        }
        return true;
    }
    if let Some(provider) = action.strip_prefix("ai-model:") {
        if choice_enabled(app, action)
            && let Some(state) = app.ai.providers.get_mut(provider)
            && state.models.iter().any(|model| model.id == value)
        {
            state.selected = value.into();
        }
        return true;
    }
    if !matches!(action, "ai-refresh" | "ai-save" | "ai-clear") {
        return false;
    }
    if busy(app, value) {
        return true;
    }
    if action == "ai-refresh" {
        refresh(app, value);
        return true;
    }
    let Some(state) = app.ai.providers.get_mut(value) else {
        return true;
    };
    state.error = None;
    if action == "ai-save" {
        if !state.loaded || !state.models.iter().any(|model| model.id == state.selected) {
            return true;
        }
        let payload = json!({"provider":value,"token":state.token,"model":state.selected});
        issue(
            app,
            Pending::Ai(Request::Save(value.into())),
            "save_ai_provider",
            payload,
        );
    } else {
        issue(
            app,
            Pending::Ai(Request::Clear(value.into())),
            "clear_ai_provider",
            json!({"provider":value}),
        );
    }
    true
}

pub(crate) fn input(app: &mut App, field: &str, value: &str) -> bool {
    let Some(provider) = field.strip_prefix("ai-token:") else {
        return false;
    };
    if busy(app, provider) {
        return true;
    }
    if let Some(state) = app.ai.providers.get_mut(provider) {
        state.token = value.into();
        state.models.clear();
        state.loaded = false;
        state.error = None;
        app.choices.open = None;
    }
    true
}

pub(crate) fn resolve(
    app: &mut App,
    request: &Request,
    request_id: u32,
    success: bool,
    payload: &str,
) {
    let result = if !success {
        Err(Message::from_error(payload))
    } else {
        match request {
            Request::Select(provider) => serde_json::from_str::<crate::models::Settings>(payload)
                .map_err(|_| Message::new("ai.invalid_response"))
                .and_then(|settings| {
                    if settings.ai_provider != *provider {
                        return Err(Message::new("ai.invalid_response"));
                    }
                    apply_settings(app, settings, request_id);
                    app.ai.error = None;
                    app.ai.retry_provider = None;
                    Ok(())
                }),
            Request::Statuses => serde_json::from_str::<Vec<ProviderStatus>>(payload)
                .map_err(|_| Message::new("ai.invalid_response"))
                .map(|statuses| {
                    app.ai.loaded = true;
                    app.ai.error = None;
                    let mut refresh_ids = Vec::new();
                    for status in statuses {
                        if let Some(state) = app.ai.providers.get_mut(&status.provider) {
                            state.configured = status.configured;
                            state.selected.clone_from(&status.model);
                            state.saved_model = status.model;
                            if state.configured && state.token.is_empty() {
                                refresh_ids.push(status.provider);
                            }
                        }
                    }
                    for provider in refresh_ids {
                        refresh(app, &provider);
                    }
                }),
            Request::Models(provider) => serde_json::from_str::<Vec<Model>>(payload)
                .map_err(|_| Message::new("ai.invalid_response"))
                .map(|models| {
                    if let Some(state) = app.ai.providers.get_mut(provider) {
                        if !state.selected.is_empty()
                            && !models.iter().any(|model| model.id == state.selected)
                        {
                            state.selected.clear();
                            state.error = Some(Message::new("ai.model_unavailable"));
                        }
                        state.models = models;
                        state.loaded = true;
                    }
                }),
            Request::Save(_) | Request::Clear(_) => serde_json::from_str::<ProviderStatus>(payload)
                .map_err(|_| Message::new("ai.invalid_response"))
                .map(|status| {
                    if let Some(state) = app.ai.providers.get_mut(&status.provider) {
                        if matches!(request, Request::Clear(_)) {
                            *state = ProviderState::default();
                        } else {
                            state.configured = status.configured;
                            state.selected.clone_from(&status.model);
                            state.saved_model = status.model;
                            state.token.clear();
                            state.error = None;
                        }
                    }
                    show_toast(
                        app,
                        Message::new(if matches!(request, Request::Clear(_)) {
                            "ai.cleared"
                        } else {
                            "notice.saved"
                        }),
                        false,
                    );
                }),
        }
    };
    if let Err(error) = result {
        match request {
            Request::Select(provider) => {
                app.ai.error = Some(error);
                app.ai.retry_provider = Some(provider.clone());
            }
            Request::Statuses => app.ai.error = Some(error),
            Request::Models(provider) | Request::Save(provider) | Request::Clear(provider) => {
                if let Some(state) = app.ai.providers.get_mut(provider) {
                    state.error = Some(error);
                }
            }
        }
    }
}

pub(crate) fn choice_enabled(app: &App, field: &str) -> bool {
    field.strip_prefix("ai-model:").is_some_and(|provider| {
        !busy(app, provider)
            && app
                .ai
                .providers
                .get(provider)
                .is_some_and(|state| state.loaded && !state.models.is_empty())
    })
}

pub(crate) fn choice_options(app: &App, field: &str) -> Vec<choices::OptionItem> {
    let Some(state) = field
        .strip_prefix("ai-model:")
        .and_then(|provider| app.ai.providers.get(provider))
    else {
        return Vec::new();
    };
    let mut options = vec![choices::item("", t(app, "ai.model_placeholder"))];
    if !state.loaded && !state.selected.is_empty() {
        options.push(choices::item(&state.selected, &state.selected));
    }
    options.extend(state.models.iter().map(|model| choices::OptionItem {
        value: model.id.clone(),
        label: model.id.clone(),
        subtitle: if model.name != model.id {
            model.name.clone()
        } else {
            String::new()
        },
        image: None,
    }));
    options
}

pub(crate) fn choice_current<'a>(app: &'a App, field: &str) -> &'a str {
    field
        .strip_prefix("ai-model:")
        .and_then(|provider| app.ai.providers.get(provider))
        .map_or("", |state| state.selected.as_str())
}

pub(crate) fn render(html: &mut String, app: &App) {
    write!(
        html,
        "<div class=\"preference-page-heading\"><h1>{}</h1><p class=\"field-help\">{}</p></div>",
        t(app, "settings.section_ai"),
        t(app, "ai.help")
    )
    .unwrap();
    write!(html,r#"<div class="ai-active-provider"><span class="ai-active-label">{}</span><div class="segmented-control ai-provider-switch" role="group" aria-label="{}"><span class="segmented-thumb" data-render-key="selected-background" aria-hidden="true"></span>"#,t(app,"ai.active_provider"),t(app,"ai.active_provider")).unwrap();
    for (provider, name) in PROVIDERS {
        let selected = provider == app.settings.ai_provider;
        write!(html,r#"<button type="button" class="segment" data-action="ai-select-provider" data-value="{provider}" aria-pressed="{selected}" tabindex="{}" {}><img class="ai-switch-logo ai-logo-{provider}" src="assets/{provider}.svg" alt=""/><span>{name}</span></button>"#,if selected {"0"}else{"-1"},if app.busy() || busy(app,provider) {"disabled"} else {""}).unwrap();
    }
    html.push_str("</div></div>");
    if let Some(error) = &app.ai.error {
        write!(html, "<div class=\"ai-load-error\"><p class=\"account-notice\" role=\"alert\">{}</p>{}</div>", esc(&error.render(app.settings.language)), button("ai-retry", "", t(app, "ai.retry"), "refresh", "quiet", false)).unwrap();
    }
    html.push_str("<div class=\"ai-provider-grid\">");
    for (provider, name) in PROVIDERS {
        let Some(state) = app.ai.providers.get(provider) else {
            continue;
        };
        let pending = busy(app, provider);
        let disabled = pending || app.busy() || !app.ai.loaded;
        let model_field = format!("ai-model:{provider}");
        let mut picker = String::new();
        choices::control(
            &mut picker,
            app,
            &model_field,
            t(app, "ai.model"),
            disabled || !choice_enabled(app, &model_field),
        );
        let has_token = state.configured || !state.token.trim().is_empty();
        let can_save = state.loaded && state.models.iter().any(|model| model.id == state.selected);
        let status = if pending {
            "ai.loading"
        } else if state.configured {
            "ai.configured"
        } else {
            "ai.not_configured"
        };
        write!(html, r#"<section class="settings-card ai-provider-card" data-render-key="ai-{provider}" aria-labelledby="ai-title-{provider}" aria-busy="{pending}"><div class="ai-provider-heading"><img class="ai-provider-logo ai-logo-{provider}" src="assets/{provider}.svg" alt=""/><div class="ai-provider-name"><h2 id="ai-title-{provider}">{name}</h2><span class="connection-status {}" role="status">{}</span></div></div><label class="edit-field" for="ai-token-{provider}"><span>Token <small>API Key</small></span><input type="password" id="ai-token-{provider}" data-input="ai-token:{provider}" value="{}" placeholder="{}" autocomplete="off" autocapitalize="none" spellcheck="false" maxlength="1024" {} /></label><div class="ai-model-heading"><label for="{}">{}</label>{}</div>{picker}"#,
            if state.configured { "connected" } else { "" }, t(app, status), esc(&state.token),
            t(app, if state.configured { "ai.token_saved" } else { "ai.token_placeholder" }),
            if disabled { "disabled" } else { "" }, esc(&model_field), t(app, "ai.model"),
            button("ai-refresh", provider, t(app, "ai.refresh"), "refresh", "quiet", disabled || !has_token)).unwrap();
        let hint = if pending {
            Message::new("ai.loading")
        } else if state.loaded && state.models.is_empty() {
            Message::new("ai.empty")
        } else if state.loaded {
            Message::new("ai.model_count").arg("count", state.models.len())
        } else {
            Message::new("ai.models_hint")
        };
        write!(
            html,
            "<p class=\"field-help ai-model-help\" role=\"status\">{}</p>",
            esc(&hint.render(app.settings.language))
        )
        .unwrap();
        if let Some(error) = &state.error {
            write!(
                html,
                "<p class=\"account-notice ai-provider-error\" role=\"alert\">{}</p>",
                esc(&error.render(app.settings.language))
            )
            .unwrap();
        }
        write!(
            html,
            "<div class=\"account-actions ai-provider-actions\">{}{}</div></section>",
            button(
                "ai-save",
                provider,
                t(app, "ai.save"),
                "save",
                "primary",
                disabled || !can_save
            ),
            button(
                "ai-clear",
                provider,
                t(app, "auth.clear"),
                "close",
                "quiet",
                disabled || !state.configured
            )
        )
        .unwrap();
    }
    write!(
        html,
        "</div><p class=\"field-help ai-vault-note\">{}</p>",
        t(app, "ai.vault")
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domestic_providers_support_discovery_selection_save_and_clear() {
        let mut app = App::default();
        resolve(
            &mut app,
            &Request::Statuses,
            0,
            true,
            r#"[{"provider":"qwen","configured":false,"model":""},{"provider":"kimi","configured":false,"model":""}]"#,
        );
        for (provider, model) in [("qwen", "qwen-plus"), ("kimi", "kimi-k2.6")] {
            let field = format!("ai-model:{provider}");
            input(&mut app, &format!("ai-token:{provider}"), "test-token");
            dispatch(&mut app, "ai-refresh", provider);
            let requests = crate::bridge::take_requests();
            assert_eq!(requests.len(), 1);
            let (id, command, payload) = &requests[0];
            assert_eq!(command, "list_ai_models");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(payload).unwrap(),
                json!({"provider":provider,"token":"test-token"})
            );
            let Pending::Ai(request) = app.requests.complete(*id).unwrap() else {
                panic!("AI request expected")
            };
            resolve(
                &mut app,
                &request,
                *id,
                true,
                &json!([{"id":model}]).to_string(),
            );
            assert!(choice_enabled(&app, &field));
            dispatch(&mut app, &field, model);
            dispatch(&mut app, "ai-save", provider);
            let requests = crate::bridge::take_requests();
            assert_eq!(requests.len(), 1);
            let (id, command, payload) = &requests[0];
            assert_eq!(command, "save_ai_provider");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(payload).unwrap(),
                json!({"provider":provider,"token":"test-token","model":model})
            );
            let Pending::Ai(request) = app.requests.complete(*id).unwrap() else {
                panic!("AI request expected")
            };
            resolve(
                &mut app,
                &request,
                *id,
                true,
                &json!({"provider":provider,"configured":true,"model":model}).to_string(),
            );
            assert!(app.ai.providers[provider].token.is_empty());
            assert_eq!(choice_current(&app, &field), model);
            dispatch(&mut app, "ai-select-provider", provider);
            let requests = crate::bridge::take_requests();
            assert_eq!(requests.len(), 1);
            let (id, command, payload) = &requests[0];
            assert_eq!(command, "set_active_ai_provider");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(payload).unwrap(),
                json!({"provider":provider})
            );
            let Pending::Ai(request) = app.requests.complete(*id).unwrap() else {
                panic!("AI request expected")
            };
            resolve(&mut app, &request, *id, true, &json!({"library_folders":[],"card_size":200,"sort_by":"title","ai_provider":provider}).to_string());
            assert_eq!(app.settings.ai_provider, provider);
            dispatch(&mut app, "ai-clear", provider);
            let requests = crate::bridge::take_requests();
            assert_eq!(requests.len(), 1);
            let (id, command, _) = &requests[0];
            assert_eq!(command, "clear_ai_provider");
            let Pending::Ai(request) = app.requests.complete(*id).unwrap() else {
                panic!("AI request expected")
            };
            resolve(
                &mut app,
                &request,
                *id,
                true,
                &json!({"provider":provider,"configured":false,"model":""}).to_string(),
            );
            assert!(!app.ai.providers[provider].configured);
            assert!(choice_current(&app, &field).is_empty());
        }
        let mut html = String::new();
        render(&mut html, &app);
        for provider in ["qwen", "kimi"] {
            assert!(html.contains(&format!("id=\"ai-token-{provider}\"")));
            assert!(html.contains(&format!("src=\"assets/{provider}.svg\"")));
        }
    }

    #[test]
    fn active_provider_only_changes_after_successful_persistence() {
        let mut app = App::default();
        app.ai.loaded = true;
        assert_eq!(app.settings.ai_provider, "deepseek");
        let request = Request::Select("openai".into());
        resolve(
            &mut app,
            &request,
            1,
            false,
            r#"{"key":"error.config_write"}"#,
        );
        assert_eq!(app.settings.ai_provider, "deepseek");
        assert!(app.ai.error.is_some());
        assert_eq!(app.ai.retry_provider.as_deref(), Some("openai"));
        resolve(
            &mut app,
            &request,
            2,
            true,
            r#"{"library_folders":[],"card_size":200,"sort_by":"title","ai_provider":"openai"}"#,
        );
        assert_eq!(app.settings.ai_provider, "openai");
        assert!(app.ai.error.is_none());
        let before = app.requests.last_id();
        dispatch(&mut app, "ai-select-provider", "unknown-provider");
        assert_eq!(app.requests.last_id(), before);
    }

    #[test]
    fn model_choices_support_new_ids_search_and_invalidate_on_token_changes() {
        let mut app = App::default();
        resolve(
            &mut app,
            &Request::Models("openai".into()),
            0,
            true,
            r#"[{"id":"future-model","name":"New Model"},{"id":"another-model"}]"#,
        );
        assert!(choice_enabled(&app, "ai-model:openai"));
        app.choices.query = "future".into();
        assert_eq!(choices::filtered(&app, "ai-model:openai").len(), 1);
        assert!(dispatch(&mut app, "ai-model:openai", "future-model"));
        assert_eq!(choice_current(&app, "ai-model:openai"), "future-model");
        input(&mut app, "ai-token:openai", "replacement-token");
        assert!(!choice_enabled(&app, "ai-model:openai"));
        assert!(app.ai.providers["openai"].models.is_empty());
    }

    #[test]
    fn refreshed_models_do_not_silently_replace_a_removed_selection() {
        let mut app = App::default();
        app.ai.providers.get_mut("deepseek").unwrap().selected = "retired-model".into();
        resolve(
            &mut app,
            &Request::Models("deepseek".into()),
            0,
            true,
            r#"[{"id":"future-model"}]"#,
        );
        assert!(choice_current(&app, "ai-model:deepseek").is_empty());
        assert!(app.ai.providers["deepseek"].error.is_some());
        resolve(
            &mut app,
            &Request::Models("openai".into()),
            0,
            false,
            r#"{"key":"ai.unauthorized"}"#,
        );
        assert!(app.ai.providers["openai"].error.is_some());
        assert_eq!(app.ai.providers["deepseek"].models.len(), 1);
    }
}
