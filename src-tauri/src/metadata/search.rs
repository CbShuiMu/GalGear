//! 限制资料搜索的等待和后台工作数量，复用连接而不缓存凭据或错误结果。
use galgear_core::{data_sources, providers};
use galgear_i18n::Message;
use reqwest::blocking::Client;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
pub(crate) struct Service {
    client: Arc<Mutex<Option<Client>>>,
}

impl Service {
    pub(crate) async fn search(
        &self,
        scheduler: &crate::game_setup::Service,
        config: PathBuf,
        source: String,
        domain: String,
        query: String,
    ) -> Result<Vec<providers::Resource>, Message> {
        let client = self.client.clone();
        tokio::time::timeout(
            Duration::from_secs(45),
            scheduler.run(source.clone(), false, move || {
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
            }),
        )
        .await
        .map_err(|_| Message::new("setup.search_timeout"))?
    }
}
