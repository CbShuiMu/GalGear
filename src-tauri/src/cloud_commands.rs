//! WebDAV transfers publish local snapshots; restoring still uses the save preview flow.
use crate::{
    preferences, save_commands,
    state::{AppState, begin_write, find_game, snapshot},
};
use galgear_core::{
    Game,
    cloud::{CloudClient, CloudSettings, CloudSnapshot, validate_password},
    credentials::{self, FanCredentials},
    runtime, snapshots,
};
use galgear_i18n::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use tauri::State;

#[derive(Serialize)]
pub(crate) struct CloudStatus {
    settings: CloudSettings,
    password_saved: bool,
}

fn credential_target(config: &Path) -> String {
    let path = config.to_string_lossy().replace('/', "\\").to_lowercase();
    format!("GalGear/WebDAV/{:x}", Sha256::digest(path.as_bytes()))
}

fn stored_credentials(state: &AppState) -> Result<FanCredentials, Message> {
    credentials::load(&credential_target(&state.store.path))
}

fn status(state: &AppState) -> Result<CloudStatus, Message> {
    Ok(CloudStatus {
        settings: snapshot(state)?.cloud,
        password_saved: !stored_credentials(state)?.token.is_empty(),
    })
}

fn password_for(
    state: &AppState,
    settings: &CloudSettings,
    draft: String,
) -> Result<String, Message> {
    if !draft.is_empty() {
        validate_password(&draft)?;
        return Ok(draft);
    }
    let saved = snapshot(state)?.cloud;
    // A blank draft may only reuse the credential for the same account and server.
    if saved.url != settings.url || saved.username != settings.username {
        return Err(Message::new("cloud.password_required"));
    }
    let password = stored_credentials(state)?.token;
    validate_password(&password)?;
    Ok(password)
}

/// Titles are independent of installation paths, so another device finds the same backups.
fn cloud_key(game: &Game) -> String {
    format!(
        "{:x}",
        Sha256::digest(game.title.trim().to_lowercase().as_bytes())
    )
}

#[tauri::command]
pub(crate) fn cloud_status(state: State<'_, AppState>) -> Result<CloudStatus, Message> {
    status(&state)
}

#[tauri::command]
pub(crate) async fn save_cloud_settings(
    settings: CloudSettings,
    password: String,
    state: State<'_, AppState>,
) -> Result<CloudStatus, Message> {
    let _guard = begin_write(&state)?;
    let settings = settings.normalized()?;
    let password = password_for(&state, &settings, password)?;
    // Construct the client before writing either configuration or credentials.
    let check = settings.clone();
    let secret = password.clone();
    tauri::async_runtime::spawn_blocking(move || CloudClient::new(&check, &secret).map(|_| ()))
        .await
        .map_err(|_| Message::new("error.operation"))??;
    let previous = stored_credentials(&state)?;
    let target = credential_target(&state.store.path);
    credentials::save(
        &target,
        &FanCredentials {
            token: password,
            mode: "bearer".into(),
            ..Default::default()
        },
    )?;
    if let Err(error) = preferences::update(&state, |value| {
        value.cloud = settings;
        Ok(())
    }) {
        credentials::save(&target, &previous)?;
        return Err(error);
    }
    status(&state)
}

#[tauri::command]
pub(crate) fn clear_cloud_settings(state: State<'_, AppState>) -> Result<CloudStatus, Message> {
    let _guard = begin_write(&state)?;
    let previous = stored_credentials(&state)?;
    let target = credential_target(&state.store.path);
    credentials::clear(&target)?;
    if let Err(error) = preferences::update(&state, |settings| {
        settings.cloud = CloudSettings::default();
        Ok(())
    }) {
        credentials::save(&target, &previous)?;
        return Err(error);
    }
    status(&state)
}

#[tauri::command]
pub(crate) async fn test_cloud_connection(
    settings: CloudSettings,
    password: String,
    state: State<'_, AppState>,
) -> Result<(), Message> {
    let settings = settings.normalized()?;
    let password = password_for(&state, &settings, password)?;
    tauri::async_runtime::spawn_blocking(move || {
        CloudClient::new(&settings, &password)?.test_connection()
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[tauri::command]
pub(crate) async fn cloud_list(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<CloudSnapshot>, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?.cloud;
    let password = password_for(&state, &settings, String::new())?;
    tauri::async_runtime::spawn_blocking(move || {
        CloudClient::new(&settings, &password)?.list(&cloud_key(&game))
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[tauri::command]
pub(crate) async fn cloud_upload(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<CloudSnapshot>, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let password = password_for(&state, &settings.cloud, String::new())?;
    let config_path = state.store.path.clone();
    // This also rejects unconfirmed, overlapping or unsafe save directories.
    let roots = save_commands::confirmed_roots(&state, &game, &settings)?;
    tauri::async_runtime::spawn_blocking(move || {
        let data = save_commands::prepare_backup_data(&config_path, &settings, &game)?;
        let client = CloudClient::new(&settings.cloud, &password)?;
        snapshots::recover(
            &data,
            &id,
            &roots,
            runtime::detect(game.executable.as_deref()),
        )?;
        let backup = snapshots::create_named_checked(
            &data,
            (&id, &game.title),
            &roots,
            "",
            settings.backup_retention,
            || runtime::detect(game.executable.as_deref()),
        )?;
        let key = cloud_key(&game);
        // Do not turn a committed upload into an apparent failure if a follow-up
        // listing is rate-limited or unavailable.
        Ok(vec![client.upload(&data, &backup, &key)?])
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[tauri::command]
pub(crate) async fn cloud_download(
    id: String,
    snapshot_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<snapshots::Snapshot>, Message> {
    let _guard = begin_write(&state)?;
    let game = find_game(&state, &id)?;
    let settings = snapshot(&state)?;
    let password = password_for(&state, &settings.cloud, String::new())?;
    let config_path = state.store.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = save_commands::prepare_backup_data(&config_path, &settings, &game)?;
        CloudClient::new(&settings.cloud, &password)?.download(
            &data,
            &id,
            &cloud_key(&game),
            &snapshot_id,
        )?;
        snapshots::list(&data, &id)
    })
    .await
    .map_err(|_| Message::new("error.operation"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_identity_is_case_and_separator_independent() {
        assert_eq!(
            credential_target(Path::new("C:/Data/settings.json")),
            credential_target(Path::new("c:\\data\\settings.json"))
        );
        assert_ne!(
            credential_target(Path::new("C:/other/settings.json")),
            credential_target(Path::new("C:/Data/settings.json"))
        );
    }

    #[test]
    fn passwords_reject_control_characters_and_preserve_spaces_and_unicode() {
        assert!(validate_password("应用 密码 ").is_ok());
        for invalid in ["", "secret\r\nheader", "secret\0", &"x".repeat(1025)] {
            assert!(validate_password(invalid).is_err());
        }
    }

    #[test]
    fn cloud_identity_ignores_install_location_and_title_case() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("game.exe");
        std::fs::write(&executable, []).unwrap();
        let first = galgear_core::catalog::manual_game(&executable, "Game Title".into()).unwrap();
        let mut second = first.clone();
        second.id = "local-two".into();
        second.folder = "D:/two".into();
        second.title = " game title ".into();
        assert_eq!(cloud_key(&first), cloud_key(&second));
    }
}
