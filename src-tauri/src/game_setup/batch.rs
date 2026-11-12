//! 批量准备资料与封面，所有网络工作完成后合并提交一次。
use super::{SetupInput, SetupResult};
use crate::{
    covers,
    state::{AppState, find_game, snapshot},
};
use galgear_core::{Game, GameEdit, catalog, game_setup, providers};
use galgear_i18n::Message;
use serde::Serialize;
use tauri::{Emitter, Manager};

#[derive(Serialize)]
pub(crate) struct BatchResult {
    pub(super) items: Vec<ItemResult>,
    warning: Option<Message>,
}
#[derive(Serialize)]
pub(super) struct ItemResult {
    id: String,
    result: Result<SetupResult, Message>,
}
struct Prepared {
    input: SetupInput,
    original: Game,
    previous: Option<GameEdit>,
    metadata: providers::Metadata,
    cover: Option<Result<PreparedCover, Message>>,
}

struct PreparedCover {
    data: String,
    path: String,
    warning: Option<Message>,
    artwork: Option<galgear_core::cover_art::Artwork>,
}

pub(super) async fn run(
    inputs: Vec<SetupInput>,
    request_id: u32,
    app: tauri::AppHandle,
) -> Result<BatchResult, Message> {
    let state = app.state::<AppState>();
    let _batch = state.setup_service.begin_batch()?;
    if inputs.is_empty() {
        return Ok(BatchResult {
            items: Vec::new(),
            warning: None,
        });
    }
    let mut seen = std::collections::BTreeSet::new();
    if inputs.iter().any(|input| !seen.insert(input.id.clone())) {
        return Err(Message::new("error.operation"));
    }
    let client = tauri::async_runtime::spawn_blocking(providers::client)
        .await
        .map_err(|error| Message::detail("error.network", error))??;
    let settings = std::sync::Arc::new(snapshot(&state)?);
    let total = inputs.len();
    let mut jobs = tokio::task::JoinSet::new();
    for input in inputs {
        let handle = app.clone();
        let client = client.clone();
        let settings = settings.clone();
        jobs.spawn(async move {
            let id = input.id.clone();
            (id, prepare(input, handle, client, settings).await)
        });
    }
    let mut ready = Vec::new();
    let mut items = Vec::new();
    while let Some(result) = jobs.join_next().await {
        let (id, prepared) = result.map_err(|error| Message::detail("error.operation", error))?;
        match prepared {
            Ok(value) => ready.push(value),
            Err(error) => items.push(ItemResult {
                id,
                result: Err(error),
            }),
        }
        let _ = app.emit(
            "setup-save-progress",
            serde_json::json!({
                "requestId": request_id, "done": ready.len() + items.len(), "total": total
            }),
        );
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || commit(&handle.state::<AppState>(), ready, items))
        .await
        .map_err(|error| Message::detail("error.operation", error))?
}

async fn prepare(
    input: SetupInput,
    app: tauri::AppHandle,
    client: reqwest::blocking::Client,
    settings: std::sync::Arc<galgear_core::Settings>,
) -> Result<Prepared, Message> {
    let state = app.state::<AppState>();
    let original = find_game(&state, &input.id)?;
    let previous = settings.game_edits.get(&input.id).cloned();
    let domain = settings.fan_domain.clone();
    let path = state.store.path.clone();
    let source = input.source.clone();
    let url = input.url.clone();
    let executable = input.executable.clone();
    let confirmed = input.confirmed;
    let metadata = state
        .setup_service
        .run(source.clone(), false, move || {
            game_setup::validate(&source, &url, &executable, confirmed)?;
            let credentials = crate::data_sources::load_credentials(&path, &source)?;
            galgear_core::data_sources::metadata_with_client(&client, &domain, &url, &credentials)
        })
        .await?;
    // 图片使用独立名额，下载与解码期间继续推进其他游戏的资料查询。
    let cover = if original.cover.is_none()
        && let Some(image) = &metadata.cover_url
    {
        let image = image.clone();
        let host = reqwest::Url::parse(&image)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        let handle = app.clone();
        let game = original.clone();
        let details = metadata.clone();
        Some(
            state
                .setup_service
                .run(format!("image:{host}"), true, move || {
                    let downloaded =
                        covers::download_game_cover(&handle.state::<AppState>(), &game, &details)?;
                    let mut rendered = game.clone();
                    rendered.cover = Some(downloaded.0);
                    covers::prepare_cover(
                        &mut rendered,
                        &handle
                            .state::<AppState>()
                            .store
                            .path
                            .with_file_name("covers")
                            .join("rendered"),
                    )?;
                    Ok(PreparedCover {
                        data: rendered.cover.ok_or_else(|| Message::new("error.image"))?,
                        path: downloaded.1,
                        warning: downloaded.2,
                        artwork: rendered.cover_artwork,
                    })
                })
                .await,
        )
    } else {
        None
    };
    Ok(Prepared {
        input,
        original,
        previous,
        metadata,
        cover,
    })
}

fn commit(
    state: &AppState,
    ready: Vec<Prepared>,
    mut items: Vec<ItemResult>,
) -> Result<BatchResult, Message> {
    // 与扫描及移除保持 settings → catalog 的锁顺序。锁内只做最终校验和提交。
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut catalog = state
        .catalog
        .lock()
        .map_err(|_| Message::new("error.catalog_state"))?;
    let mut next = settings.clone();
    let mut updated = catalog.clone();
    let mut successes = Vec::new();
    for prepared in ready {
        let id = prepared.input.id.clone();
        let result = apply(&mut next, &mut updated, prepared);
        match result {
            Ok(saved) => successes.push(ItemResult {
                id,
                result: Ok(saved),
            }),
            Err(error) => items.push(ItemResult {
                id,
                result: Err(error),
            }),
        }
    }
    let mut warning = None;
    if !successes.is_empty() {
        // 设置保存完整资料和封面路径，是可恢复的权威记录；目录是派生缓存。
        // 原子设置替换失败时不发布任何成功结果。缓存失败仍保留已提交资料并明确提示。
        state.store.save(&next)?;
        *settings = next;
        if let Err(error) =
            galgear_core::catalog::save(&state.store.path.with_file_name("catalog.json"), &updated)
        {
            log::warn!("setup_catalog_cache_failed key={}", error.key);
            warning = Some(error);
        }
        *catalog = updated;
    }
    items.extend(successes);
    Ok(BatchResult { items, warning })
}

fn apply(
    settings: &mut galgear_core::Settings,
    catalog: &mut galgear_core::ScanResult,
    prepared: Prepared,
) -> Result<SetupResult, Message> {
    let Prepared {
        input,
        original,
        previous,
        metadata,
        cover,
    } = prepared;
    let current = catalog
        .games
        .iter_mut()
        .find(|game| game.id == input.id)
        .ok_or_else(|| Message::new("error.game_missing"))?;
    if current.folder != original.folder
        || settings.game_edits.get(&input.id) != previous.as_ref()
        || galgear_core::catalog::is_excluded(current, settings)
    {
        return Err(Message::new("setup.stale"));
    }
    let edit = game_setup::prepare(
        current,
        previous.as_ref(),
        &input.source,
        &input.executable,
        metadata,
    )?;
    if current.cover == original.cover {
        match cover {
            Some(Ok(cover)) => {
                settings
                    .cover_overrides
                    .insert(input.id.clone(), cover.path);
                current.cover = Some(cover.data);
                current.cover_artwork = cover.artwork;
                current.metadata_warning = cover.warning;
            }
            Some(Err(error)) => current.metadata_warning = Some(error),
            None => {}
        }
    }
    settings.game_edits.insert(input.id, edit.clone());
    catalog::apply_edits(current, settings);
    Ok(SetupResult {
        game: current.clone(),
        edit,
    })
}

#[cfg(test)]
mod tests {
    use super::{Prepared, commit};
    use crate::{game_setup::SetupInput, state::AppState};
    use galgear_core::{GameEdit, providers::Metadata};

    fn fixture(state: &AppState, id: &str) -> Prepared {
        let folder = state.store.path.parent().unwrap().join(id);
        std::fs::create_dir_all(&folder).unwrap();
        let executable = folder.join("game.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let mut original = galgear_core::catalog::manual_game(&executable, id.into()).unwrap();
        original.id = id.into();
        let edit = GameEdit {
            setup_pending: true,
            ..Default::default()
        };
        state.catalog.lock().unwrap().games.push(original.clone());
        state
            .settings
            .lock()
            .unwrap()
            .game_edits
            .insert(id.into(), edit.clone());
        Prepared {
            input: SetupInput {
                id: id.into(),
                source: "vndb".into(),
                url: "https://vndb.org/v1".into(),
                executable: String::new(),
                confirmed: true,
                rematch: false,
            },
            original,
            previous: Some(edit),
            metadata: Metadata {
                title: "Fetched".into(),
                url: "https://vndb.org/v1".into(),
                ..Default::default()
            },
            cover: None,
        }
    }
    #[test]
    fn batch_merges_successes_preserves_newer_edits_and_reports_deleted_rows() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState::load(temp.path().into()).unwrap();
        let ready: Vec<_> = ["ok", "changed", "deleted", "other"]
            .map(|id| fixture(&state, id))
            .into();
        state
            .settings
            .lock()
            .unwrap()
            .game_edits
            .get_mut("changed")
            .unwrap()
            .title = Some("Manual".into());
        state
            .catalog
            .lock()
            .unwrap()
            .games
            .retain(|game| game.id != "deleted");
        state.catalog.lock().unwrap().games[0].playtime_seconds = 123;
        let result = commit(&state, ready, Vec::new()).unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .filter(|item| item.result.is_ok())
                .count(),
            2
        );
        assert_eq!(
            result
                .items
                .iter()
                .find(|item| item.id == "changed")
                .unwrap()
                .result
                .as_ref()
                .err()
                .unwrap()
                .key,
            "setup.stale"
        );
        assert_eq!(
            result
                .items
                .iter()
                .find(|item| item.id == "deleted")
                .unwrap()
                .result
                .as_ref()
                .err()
                .unwrap()
                .key,
            "error.game_missing"
        );
        let settings = state.store.load().unwrap();
        assert!(settings.game_edits["ok"].setup_confirmed);
        assert_eq!(
            settings.game_edits["changed"].title.as_deref(),
            Some("Manual")
        );
        let catalog = galgear_core::catalog::load(&temp.path().join("catalog.json")).unwrap();
        assert_eq!(catalog.games[0].playtime_seconds, 123);
        assert_eq!(catalog.games[0].title, "Fetched");
    }
    #[test]
    fn settings_failure_publishes_nothing_and_cache_failure_keeps_authoritative_data() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState::load(temp.path().into()).unwrap();
        let prepared = fixture(&state, "ok");
        std::fs::create_dir(&state.store.path).unwrap();
        assert!(commit(&state, vec![prepared], Vec::new()).is_err());
        assert!(!state.settings.lock().unwrap().game_edits["ok"].setup_confirmed);
        assert_eq!(state.catalog.lock().unwrap().games[0].title, "ok");
        std::fs::remove_dir(&state.store.path).unwrap();
        let prepared = fixture(&state, "second");
        std::fs::create_dir(temp.path().join("catalog.json")).unwrap();
        let result = commit(&state, vec![prepared], Vec::new()).unwrap();
        assert!(result.warning.is_some());
        assert!(result.items[0].result.is_ok());
        assert!(state.store.load().unwrap().game_edits["second"].setup_confirmed);
    }
}
