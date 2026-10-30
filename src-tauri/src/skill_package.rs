//! Check skills once per app launch without blocking the window or library.
use galgear_core::ai::{runtime, skills};
use galgear_i18n::Message;
use serde::Serialize;
use tokio::sync::OnceCell;

static UPDATE: OnceCell<Result<(), Message>> = OnceCell::const_new();

async fn ensure() -> &'static Result<(), Message> {
    UPDATE
        .get_or_init(|| async {
            tauri::async_runtime::spawn_blocking(|| {
                let home = runtime::home().ok_or_else(|| Message::new("summary.tools_missing"))?;
                skills::sync(&home)
            })
            .await
            .unwrap_or_else(|error| Err(Message::detail("skills.update_failed", error)))
        })
        .await
}

pub(crate) fn start() {
    tauri::async_runtime::spawn(async {
        ensure().await;
    });
}

pub(crate) fn available() -> bool {
    // Do not start an AI process while its resource directory is being replaced.
    UPDATE.get().is_some() && runtime::home().is_some_and(|home| runtime::available(&home))
}

#[derive(Serialize)]
pub(crate) struct Status {
    available: bool,
    error: Option<Message>,
}

#[tauri::command]
pub(crate) async fn get_ai_runtime() -> Status {
    let result = ensure().await;
    Status {
        available: available(),
        error: result.as_ref().err().cloned(),
    }
}
