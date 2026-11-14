//! 导入步骤只在实际工作开始时上报；序号隔离并行工作和迟到事件。
use galgear_i18n::Message;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Metadata,
    Cover,
    Saving,
}

#[derive(Clone)]
pub(super) struct Reporter {
    app: tauri::AppHandle,
    request_id: u32,
    total: usize,
    sequence: Arc<Mutex<u64>>,
}

impl Reporter {
    pub(super) fn new(app: tauri::AppHandle, request_id: u32, total: usize) -> Self {
        Self {
            app,
            request_id,
            total,
            sequence: Arc::new(Mutex::new(0)),
        }
    }

    pub(super) fn stage(&self, stage: Stage, current: usize) {
        self.emit(Some(stage), current, None);
    }

    pub(super) fn failed(&self, error: &Message, current: usize) {
        self.emit(None, current, Some(error));
    }

    fn emit(&self, stage: Option<Stage>, current: usize, error: Option<&Message>) {
        let Ok(mut sequence) = self.sequence.lock() else {
            return;
        };
        *sequence += 1;
        // 排序与发送共用短锁，避免不同工作线程倒序发布；不持有业务状态锁。
        let _ = self.app.emit(
            "setup-save-progress",
            serde_json::json!({
                "requestId": self.request_id,
                "sequence": *sequence,
                "stage": stage,
                "current": current,
                "total": self.total,
                "error": error,
            }),
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::{Reporter, Stage};
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    #[test]
    #[ignore = "创建独立原生事件循环，不加载真实游戏库；单独运行"]
    fn native_import_progress_events_keep_order() {
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        let app = tauri::Builder::default()
            .any_thread()
            .build(context)
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let received = events.clone();
        app.listen("setup-save-progress", move |event| {
            received
                .lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });
        let reporter = Reporter::new(app.handle().clone(), 42, 4);
        std::thread::scope(|scope| {
            for current in 1..=4 {
                let reporter = reporter.clone();
                scope.spawn(move || {
                    reporter.stage(Stage::Metadata, current);
                    if current % 2 == 0 {
                        reporter.stage(Stage::Cover, current);
                    }
                });
            }
        });
        reporter.failed(
            &galgear_i18n::Message::detail("error.network", "fixture offline"),
            2,
        );
        reporter.stage(Stage::Saving, 4);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 8);
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence"], index + 1);
            assert_eq!(event["requestId"], 42);
            assert_eq!(event["total"], 4);
        }
        assert_eq!(events[6]["error"]["args"]["detail"], "fixture offline");
        assert_eq!(events[7]["stage"], "saving");
    }
}
