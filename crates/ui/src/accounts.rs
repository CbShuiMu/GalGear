//! Account status and credential drafts.
use crate::{
    downloads, models::Account, models::FanCredentials, models::FanStatus, requests::Pending,
    requests::RequestQueue, requests::issue, state::App,
};
use galgear_i18n::Message;
use serde_json::json;

pub(crate) struct State {
    pub(crate) credentials: FanCredentials,
    pub(crate) credentials_loaded: bool,
    pub(crate) status: Account,
    pub(crate) refreshing: bool,
    pub(crate) error: Option<Message>,
    pub(crate) revision: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            credentials: FanCredentials {
                mode: "csrf".into(),
                ..Default::default()
            },
            credentials_loaded: false,
            status: Account::default(),
            refreshing: false,
            error: None,
            revision: 0,
        }
    }
}

pub(crate) fn busy(state: &State, requests: &RequestQueue) -> bool {
    state.refreshing
        || requests.values().any(|p| {
            matches!(
                p,
                Pending::Credentials
                    | Pending::CredentialsSaved
                    | Pending::CredentialsCleared
                    | Pending::Account
            )
        })
}

pub(crate) fn apply_status(state: &mut State, status: FanStatus) {
    if status.revision < state.revision {
        return;
    }
    state.revision = status.revision;
    state.status = status.account;
    state.error = status.error;
    state.refreshing = status.refreshing;
}

pub(crate) fn dispatch(app: &mut App, action: &str, value: &str) -> Option<bool> {
    match action {
        "fan-mode" if matches!(value, "csrf" | "bearer") => {
            app.account.credentials.mode = value.into()
        }
        "save-fan" if !busy(&app.account, &app.requests) && !downloads::busy(&app.requests) => {
            let value = app.account.credentials.clone();
            issue(
                app,
                Pending::CredentialsSaved,
                "save_fan_credentials",
                json!({"value":value}),
            );
        }
        "clear-fan" if !busy(&app.account, &app.requests) && !downloads::busy(&app.requests) => {
            issue(
                app,
                Pending::CredentialsCleared,
                "clear_fan_credentials",
                json!({}),
            );
        }
        "check-fan" if !busy(&app.account, &app.requests) => {
            issue(app, Pending::Account, "refresh_fan_status", json!({}))
        }
        _ => return None,
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::{State, apply_status};
    use crate::models::{Account, FanStatus};
    use galgear_i18n::Message;

    #[test]
    fn stale_status_cannot_replace_a_newer_account_or_restart_its_refresh_indicator() {
        let mut state = State::default();
        apply_status(
            &mut state,
            FanStatus {
                revision: 8,
                account: Account {
                    status: "authenticated".into(),
                    username: Some("Current".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        apply_status(
            &mut state,
            FanStatus {
                revision: 7,
                refreshing: true,
                error: Some(Message::new("auth.network")),
                ..Default::default()
            },
        );
        assert_eq!(state.status.username.as_deref(), Some("Current"));
        assert_eq!(state.revision, 8);
        assert!(!state.refreshing && state.error.is_none());
    }
}
