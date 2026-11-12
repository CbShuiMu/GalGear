//! 分别限制资料查询与图片工作，源站退避不会占用全局网络名额。
use galgear_i18n::Message;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Semaphore, time::Instant};

pub(crate) struct Service {
    lanes: Mutex<BTreeMap<String, Arc<Lane>>>,
    metadata: Arc<Semaphore>,
    covers: Arc<Semaphore>,
    batch: tokio::sync::Mutex<()>,
}
struct Lane {
    slots: Arc<Semaphore>,
    next: Mutex<Instant>,
    interval: Duration,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            lanes: Default::default(),
            metadata: Arc::new(Semaphore::new(6)),
            covers: Arc::new(Semaphore::new(4)),
            batch: Default::default(),
        }
    }
}
impl Service {
    pub(super) fn begin_batch(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, Message> {
        self.batch
            .try_lock()
            .map_err(|_| Message::new("backup.busy"))
    }

    pub(crate) async fn run<T: Send + 'static>(
        &self,
        key: String,
        image: bool,
        work: impl Fn() -> Result<T, Message> + Send + Sync + 'static,
    ) -> Result<T, Message> {
        let lane = {
            let mut lanes = self
                .lanes
                .lock()
                .map_err(|_| Message::new("error.operation"))?;
            // VNDB Kana 公布 200 次/5 分钟，1.6 秒留出余量；执行时间配额仍服从 429。
            // https://api.vndb.org/kana
            let interval = if key == "vndb" {
                Duration::from_millis(1600)
            } else {
                Duration::from_secs(1)
            };
            lanes
                .entry(key)
                .or_insert_with(|| {
                    Arc::new(Lane {
                        slots: Arc::new(Semaphore::new(2)),
                        next: Mutex::new(Instant::now()),
                        interval,
                    })
                })
                .clone()
        };
        let pool = if image { &self.covers } else { &self.metadata };
        let work = Arc::new(work);
        for attempt in 0..3 {
            let source_permit = lane
                .slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| Message::new("error.operation"))?;
            // 等待来源限速时不持有全局许可；取得许可后再次核对共享退避。
            let permit = loop {
                let deadline = *lane
                    .next
                    .lock()
                    .map_err(|_| Message::new("error.operation"))?;
                let now = Instant::now();
                if deadline > now {
                    let wait = deadline.duration_since(now);
                    if wait > Duration::from_secs(60) {
                        return Err(
                            Message::new("sources.rate_limited").arg("seconds", wait.as_secs() + 1)
                        );
                    }
                    tokio::time::sleep_until(deadline).await;
                }
                let permit = pool
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| Message::new("error.operation"))?;
                let mut next = lane
                    .next
                    .lock()
                    .map_err(|_| Message::new("error.operation"))?;
                let now = Instant::now();
                if *next <= now {
                    *next = now + lane.interval;
                    break permit;
                }
            };
            let operation = work.clone();
            let worker_lane = lane.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                // 超时不会终止线程，真实工作和退避更新结束前始终保留许可。
                let _permits = (source_permit, permit);
                let result = operation();
                if let Some(seconds) = result
                    .as_ref()
                    .err()
                    .and_then(galgear_core::rate_limit::retry_after)
                {
                    let delay = seconds.max(2u64.pow(attempt + 1));
                    let now = Instant::now();
                    let deadline = now
                        .checked_add(Duration::from_secs(delay))
                        .unwrap_or(now + Duration::from_secs(100 * 365 * 86400));
                    let mut next = worker_lane
                        .next
                        .lock()
                        .map_err(|_| Message::new("error.operation"))?;
                    *next = (*next).max(deadline);
                }
                result
            })
            .await
            .map_err(|error| Message::detail("error.operation", error))?;
            let Some(seconds) = result
                .as_ref()
                .err()
                .and_then(galgear_core::rate_limit::retry_after)
            else {
                return result;
            };
            let delay = seconds.max(2u64.pow(attempt + 1));
            // 较长的服务端等待交回用户重试，绝不提前重发。
            if attempt == 2 || delay > 60 {
                return result;
            }
        }
        Err(Message::new("error.operation"))
    }
}

#[cfg(test)]
mod tests {
    use super::Service;
    use galgear_i18n::Message;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn timeout_keeps_worker_permit_and_expired_queue_never_starts() {
        tauri::async_runtime::block_on(async {
            let service = Arc::new(Service {
                metadata: Arc::new(tokio::sync::Semaphore::new(1)),
                ..Default::default()
            });
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let started = Mutex::new(Some(started_tx));
            let (release_tx, release_rx) = mpsc::channel();
            let release = Mutex::new(release_rx);
            let worker_service = service.clone();
            let worker = tauri::async_runtime::spawn(async move {
                tokio::time::timeout(
                    Duration::from_millis(150),
                    worker_service.run("first".into(), false, move || {
                        started.lock().unwrap().take().unwrap().send(()).unwrap();
                        release
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        Ok(())
                    }),
                )
                .await
            });
            started_rx.await.unwrap();
            assert!(worker.await.unwrap().is_err());
            assert_eq!(service.metadata.available_permits(), 0);
            let ran = Arc::new(AtomicBool::new(false));
            let queued_ran = ran.clone();
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(20),
                    service.run("second".into(), false, move || {
                        queued_ran.store(true, Ordering::SeqCst);
                        Ok(())
                    })
                )
                .await
                .is_err()
            );
            // 图片不占用资料名额，资料服务器等待期间仍能处理图片。
            assert_eq!(
                service
                    .run("image:fixture".into(), true, || Ok(42))
                    .await
                    .unwrap(),
                42
            );
            release_tx.send(()).unwrap();
            assert_eq!(
                service.run("third".into(), false, || Ok(7)).await.unwrap(),
                7
            );
            assert!(!ran.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn rate_limit_cools_only_its_source_and_retries_are_bounded() {
        tauri::async_runtime::block_on(async {
            let service = Service::default();
            let count = Arc::new(AtomicUsize::new(0));
            let attempts = count.clone();
            let started = tokio::time::Instant::now();
            let result = service
                .run("limited".into(), false, move || {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err(Message::new("sources.rate_limited").arg("seconds", 1))
                    } else {
                        Ok(42)
                    }
                })
                .await;
            assert_eq!(result.unwrap(), 42);
            assert_eq!(count.load(Ordering::SeqCst), 2);
            assert!(started.elapsed() >= Duration::from_secs(2));
            let error = service
                .run("long".into(), false, || {
                    Err::<(), _>(Message::new("sources.rate_limited").arg("seconds", 120))
                })
                .await
                .unwrap_err();
            assert_eq!(error.key, "sources.rate_limited");
            let other = tokio::time::timeout(
                Duration::from_millis(500),
                service.run("healthy".into(), false, || Ok(1)),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(other, 1);
            assert_eq!(
                service
                    .run("long".into(), false, || Ok(1))
                    .await
                    .unwrap_err()
                    .key,
                "sources.rate_limited"
            );
        });
    }
}
