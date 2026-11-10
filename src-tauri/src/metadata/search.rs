//! 限制资料搜索的等待和后台工作数量，复用连接而不缓存凭据或错误结果。
use galgear_core::{data_sources, providers};
use galgear_i18n::Message;
use reqwest::blocking::Client;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

pub(crate) struct Service {
    slots: Arc<Semaphore>,
    client: Arc<Mutex<Option<Client>>>,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(3)),
            client: Default::default(),
        }
    }
}

impl Service {
    pub(crate) async fn search(
        &self,
        config: PathBuf,
        source: String,
        domain: String,
        query: String,
    ) -> Result<Vec<providers::Resource>, Message> {
        let client = self.client.clone();
        self.run(Duration::from_secs(45), move || {
            let credentials = crate::data_sources::load_credentials(&config, &source)?;
            let client = {
                let mut cached = client.lock().map_err(|_| Message::new("error.operation"))?;
                if cached.is_none() {
                    *cached = Some(providers::client()?);
                }
                cached
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| Message::new("error.operation"))?
            };
            data_sources::search_with_client(&client, &source, &domain, &query, &credentials)
        })
        .await
    }

    async fn run<T: Send + 'static>(
        &self,
        timeout: Duration,
        work: impl FnOnce() -> Result<T, Message> + Send + 'static,
    ) -> Result<T, Message> {
        let operation = async {
            let permit = self
                .slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| Message::new("error.operation"))?;
            tauri::async_runtime::spawn_blocking(move || {
                // 超时不会终止阻塞线程，许可必须保留到真实工作结束，防止重试堆积线程。
                let _permit = permit;
                work()
            })
            .await
            .map_err(|error| Message::detail("error.network", error))?
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| Message::new("setup.search_timeout"))?
    }
}

#[cfg(test)]
mod tests {
    use super::Service;
    use galgear_i18n::Message;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::Duration;
    use tokio::sync::{Semaphore, oneshot};

    #[test]
    fn timeout_retains_worker_limit_and_expired_queue_does_not_start_work() {
        tauri::async_runtime::block_on(async {
            let service = Arc::new(Service {
                slots: Arc::new(Semaphore::new(1)),
                client: Default::default(),
            });
            let (started_tx, started_rx) = oneshot::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let worker_service = service.clone();
            let worker = tauri::async_runtime::spawn(async move {
                worker_service
                    .run(Duration::from_millis(150), move || {
                        started_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(())
                    })
                    .await
            });
            started_rx.await.unwrap();
            assert_eq!(
                worker.await.unwrap().unwrap_err().key,
                "setup.search_timeout"
            );
            assert_eq!(service.slots.available_permits(), 0);
            let ran = Arc::new(AtomicBool::new(false));
            let queued_ran = ran.clone();
            let expired = service
                .run(Duration::from_millis(20), move || {
                    queued_ran.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .await;
            assert_eq!(expired.unwrap_err().key, "setup.search_timeout");
            release_tx.send(()).unwrap();
            let failure = service
                .run(Duration::from_secs(2), || {
                    Err::<(), _>(Message::new("error.network"))
                })
                .await;
            assert_eq!(failure.unwrap_err().key, "error.network");
            assert_eq!(
                service
                    .run(Duration::from_secs(2), || Ok(42))
                    .await
                    .unwrap(),
                42
            );
            assert!(!ran.load(Ordering::SeqCst));
        });
    }
}
