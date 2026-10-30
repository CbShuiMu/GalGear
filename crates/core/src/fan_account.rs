//! 2DFan sessions, read-only account inspection and confirmed download plans.
use crate::{credentials::FanCredentials, providers};
use galgear_i18n::Message;
use reqwest::{
    Url,
    blocking::Client,
    cookie::Jar,
    header::{AUTHORIZATION, HeaderValue, REFERER},
};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}
fn auth_error() -> Message {
    Message::new("auth.network")
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct Account {
    pub status: String,
    pub username: Option<String>,
    pub balance: Option<u64>,
    pub token_saved: bool,
    pub cookie_saved: bool,
    pub reason: Option<Message>,
    pub checked_at: u64,
}
pub struct Session {
    client: Client,
    download_client: Client,
    base: Url,
    credentials: FanCredentials,
}
impl Session {
    pub fn new(domain: &str, credentials: FanCredentials) -> Result<Self, Message> {
        credentials.validate()?;
        let base = providers::fan_base(domain)?;
        let jar = Arc::new(Jar::default());
        for pair in credentials
            .cookie
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let Some((name, value)) = pair.split_once('=') else {
                return Err(Message::new("auth.invalid"));
            };
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                || HeaderValue::from_str(value).is_err()
            {
                return Err(Message::new("auth.invalid"));
            }
            jar.add_cookie_str(&format!("{name}={value}; Path=/; Secure"), &base);
        }
        let builder = || {
            Client::builder()
                .user_agent("GalGear/0.1 (+local game library)")
                .cookie_provider(jar.clone())
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::custom({
                    let origin = base.origin();
                    move |attempt| {
                        if attempt.previous().len() >= 5 {
                            attempt.error("Too many redirects")
                        } else if attempt.url().origin() == origin {
                            attempt.follow()
                        } else {
                            attempt.stop()
                        }
                    }
                }))
        };
        let client = builder().build().map_err(|_| auth_error())?;
        let download_client = builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(12))
            .timeout(Duration::from_secs(1800))
            .build()
            .map_err(|_| Message::new("download.network"))?;
        Ok(Self {
            client,
            download_client,
            base,
            credentials,
        })
    }
    fn request(&self, url: Url) -> Result<reqwest::blocking::RequestBuilder, Message> {
        let mut request = self.client.get(url.clone());
        if url.origin() == self.base.origin() && !self.credentials.token.is_empty() {
            let (key, value) = if self.credentials.mode == "bearer" {
                (AUTHORIZATION, format!("Bearer {}", self.credentials.token))
            } else {
                (
                    reqwest::header::HeaderName::from_static("x-csrf-token"),
                    self.credentials.token.clone(),
                )
            };
            let mut value =
                HeaderValue::from_str(&value).map_err(|_| Message::new("auth.invalid"))?;
            value.set_sensitive(true);
            request = request.header(key, value);
        }
        Ok(request)
    }
    fn read(&self, url: Url) -> Result<String, Message> {
        let response = self
            .request(url)?
            .send()
            .map_err(|_| auth_error())?
            .error_for_status()
            .map_err(|_| auth_error())?;
        let mut data = Vec::new();
        response
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(|_| auth_error())?;
        if data.len() > 4 * 1024 * 1024 {
            return Err(Message::new("error.response_size"));
        }
        String::from_utf8(data).map_err(|_| auth_error())
    }
    pub fn check(&self) -> Result<Account, Message> {
        let mut account = parse_account(&self.read(self.base.clone())?);
        account.token_saved = !self.credentials.token.is_empty();
        account.cookie_saved = !self.credentials.cookie.is_empty();
        if account.status != "authenticated" {
            account.reason = Some(Message::new(if self.credentials.cookie.is_empty() {
                "auth.need_cookie"
            } else {
                "auth.session_invalid"
            }));
        }
        account.checked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(account)
    }
    pub fn inspect(&self, url: &str) -> Result<DownloadPlan, Message> {
        let url = self
            .base
            .join(url)
            .map_err(|_| Message::new("error.site_url"))?;
        let id = url.path().strip_prefix("/downloads/").unwrap_or_default();
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Message::new("error.site_url"));
        }
        parse_download(&self.read(url.clone())?, url)
    }

    /// Read-only diagnostics for changed site markup, excluding credentials and signed URL values.
    pub fn resource_structure(&self, url: &str) -> Result<serde_json::Value, Message> {
        let url = self
            .base
            .join(url)
            .map_err(|_| Message::new("error.site_url"))?;
        let id = url.path().strip_prefix("/downloads/").unwrap_or_default();
        if url.origin() != self.base.origin()
            || id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Message::new("error.site_url"));
        }
        let body = self.read(url.clone())?;
        let doc = Html::parse_document(&body);
        let links:Vec<_>=doc.select(&sel("main a, main button, main form")).filter_map(|e| {
            let label=e.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
            let class=e.value().attr("class").unwrap_or_default();
            if !class.contains("download-btn") && !(label.len()<80 && (label.contains("下载")||label.contains("购买"))) {return None;}
            let link=e.value().attr("href").or_else(||e.value().attr("action")).and_then(|s|url.join(s).ok());
            Some(serde_json::json!({"tag":e.value().name(),"label":label,"class":class,"attributes":e.value().attrs().map(|(k,_)|k).collect::<Vec<_>>(),"method":e.value().attr("method"),"host":link.as_ref().and_then(|l|l.host_str()),"same_origin":link.as_ref().is_some_and(|l|l.origin()==url.origin()),"local_path":link.as_ref().filter(|l|l.origin()==url.origin()).map(|l|l.path()),"file_extension":link.as_ref().and_then(|l|l.path().rsplit('.').next()).filter(|s|["zip","exe","rar","7z"].contains(s)),"query_present":link.as_ref().is_some_and(|l|l.query().is_some())}))
        }).take(30).collect();
        Ok(serde_json::json!({"preview":parse_download(&body,url)?.preview,"links":links}))
    }
    pub fn purchase(&self, plan: &DownloadPlan) -> Result<DownloadPlan, Message> {
        let fresh = self.inspect(&plan.preview.source_url)?;
        if !fresh.preview.requires_purchase {
            return Ok(fresh);
        }
        if fresh.preview.price != plan.preview.price {
            return Err(Message::new("download.price_changed"));
        }
        let price = fresh
            .preview
            .price
            .ok_or_else(|| Message::new("download.unknown_price"))?;
        if !fresh.preview.authenticated {
            return Err(Message::new("auth.need_cookie"));
        }
        let balance = fresh
            .preview
            .balance
            .ok_or_else(|| Message::new("auth.balance_unknown"))?;
        if balance < price {
            return Err(Message::new("download.insufficient"));
        }
        let action = fresh
            .purchase_action
            .as_ref()
            .ok_or_else(|| Message::new("download.site_required"))?;
        let csrf = fresh
            .csrf
            .as_ref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Message::new("download.site_required"))?;
        // Exactly one POST. A timeout must be resolved by re-inspecting the site, never automatic reposting.
        self.client
            .post(action.clone())
            .header(REFERER, &fresh.preview.source_url)
            .form(&[("authenticity_token", csrf)])
            .send()
            .map_err(|_| Message::new("download.purchase_uncertain"))?
            .error_for_status()
            .map_err(|_| Message::new("download.purchase_uncertain"))?;
        let after = self
            .inspect(&fresh.preview.source_url)
            .map_err(|_| Message::new("download.purchase_uncertain"))?;
        if after.preview.requires_purchase || after.file_url.is_none() {
            return Err(Message::new("download.purchase_uncertain"));
        }
        Ok(after)
    }

    /// The site sometimes exposes a zero-point acquisition form instead of a direct link.
    pub fn acquire_free(&self, plan: &DownloadPlan) -> Result<DownloadPlan, Message> {
        let fresh = self.inspect(&plan.preview.source_url)?;
        if fresh.file_url.is_some() {
            return Ok(fresh);
        }
        if !fresh.preview.free || fresh.preview.price != Some(0) || fresh.preview.requires_purchase
        {
            return Err(Message::new("download.price_changed"));
        }
        if !fresh.preview.authenticated {
            return Err(Message::new("auth.need_cookie"));
        }
        let action = fresh
            .purchase_action
            .as_ref()
            .ok_or_else(|| Message::new("download.site_required"))?;
        let csrf = fresh
            .csrf
            .as_ref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Message::new("download.site_required"))?;
        self.client
            .post(action.clone())
            .header(REFERER, &fresh.preview.source_url)
            .form(&[("authenticity_token", csrf)])
            .send()
            .map_err(|_| Message::new("download.free_uncertain"))?
            .error_for_status()
            .map_err(|_| Message::new("download.free_uncertain"))?;
        let after = self
            .inspect(&fresh.preview.source_url)
            .map_err(|_| Message::new("download.free_uncertain"))?;
        if after.file_url.is_none() {
            return Err(Message::new("download.free_uncertain"));
        }
        Ok(after)
    }

    pub fn download(
        &self,
        plan: &DownloadPlan,
        directory: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Transfer),
    ) -> Result<String, Message> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Message::new("download.cancelled"));
        }
        let url = plan
            .file_url
            .as_ref()
            .ok_or_else(|| Message::new("download.site_required"))?;
        if url.origin() != self.base.origin() {
            return download(plan, directory, cancel, progress);
        }
        // Login cookies stay on the site. CDN requests use a separate client without credentials.
        let response = self
            .download_client
            .get(url.clone())
            .header(REFERER, &plan.preview.source_url)
            .timeout(Duration::from_secs(1800))
            .send()
            .map_err(|_| Message::new("download.network"))?;
        if response.status().is_redirection()
            && let Some(target) = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| response.url().join(v).ok())
            && allowed_file(&target, &self.base)
            && target.origin() != self.base.origin()
        {
            let redirected = DownloadPlan {
                preview: plan.preview.clone(),
                file_url: Some(target),
                purchase_action: None,
                csrf: None,
            };
            return download(&redirected, directory, cancel, progress);
        }
        finish_download(response, plan, directory, cancel, progress)
    }
}

pub fn parse_account(body: &str) -> Account {
    let doc = Html::parse_document(body);
    let user = doc
        .select(&sel("header[data-island='NexusHeader'][data-props]"))
        .next()
        .and_then(|e| e.value().attr("data-props"))
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .map(|v| v["user"].clone())
        .unwrap_or(serde_json::Value::Null);
    let username = ["name", "username", "display_name"].iter().find_map(|k| {
        user[*k]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });
    let authenticated = user["id"].as_u64().is_some_and(|id| id > 0) || username.is_some();
    let balance = ["points", "point", "credits", "credit"]
        .iter()
        .find_map(|k| {
            user[*k].as_u64().or_else(|| {
                user[*k]
                    .as_str()
                    .and_then(|s| s.replace(',', "").parse().ok())
            })
        });
    Account {
        status: if authenticated {
            "authenticated"
        } else {
            "unauthenticated"
        }
        .into(),
        username,
        balance: if authenticated { balance } else { None },
        ..Default::default()
    }
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct DownloadPreview {
    pub ticket: String,
    pub source_url: String,
    pub title: String,
    pub price: Option<u64>,
    pub balance: Option<u64>,
    pub authenticated: bool,
    pub requires_purchase: bool,
    pub can_download: bool,
    pub can_purchase: bool,
    pub external: bool,
    pub free: bool,
    pub reason: Option<Message>,
}
pub struct DownloadPlan {
    pub preview: DownloadPreview,
    file_url: Option<Url>,
    purchase_action: Option<Url>,
    csrf: Option<String>,
}
pub fn parse_download(body: &str, url: Url) -> Result<DownloadPlan, Message> {
    let doc = Html::parse_document(body);
    let account = parse_account(body);
    let title = doc
        .select(&sel("h1"))
        .next()
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .unwrap_or_default();
    if title.is_empty() {
        return Err(Message::new("error.metadata"));
    }
    // Only the price field belonging to this resource, never comments or related resources.
    let mut price = doc
        .select(&sel("main .download-info-item"))
        .find_map(|row| {
            let label = row
                .select(&sel(".download-info-label"))
                .next()?
                .text()
                .collect::<String>();
            if label.trim() != "价格" {
                return None;
            }
            let value = row
                .select(&sel(".download-info-value"))
                .next()?
                .text()
                .collect::<String>();
            let number = value.trim().strip_suffix("积分")?.trim();
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            number.parse::<u64>().ok()
        });
    let csrf = doc
        .select(&sel("meta[name='csrf-token']"))
        .next()
        .and_then(|e| e.value().attr("content"))
        .map(str::to_owned);
    let mut file_url = None;
    let mut external = false;
    let mut purchase_action = None;
    for a in doc.select(&sel("main a[href]")) {
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        let Ok(link) = url.join(href) else {
            continue;
        };
        let label = a.text().collect::<String>();
        if label.contains("网盘载点") {
            external = true;
        }
        let file_label = ["直连下载", "附件下载", "免费下载", "海外直连", "国内直连"]
            .iter()
            .any(|text| label.contains(text));
        let cdn_button = a
            .value()
            .attr("class")
            .is_some_and(|s| s.split_whitespace().any(|c| c == "download-btn--primary"))
            && link.origin() != url.origin();
        if (file_label || cdn_button)
            && a.value()
                .attr("data-turbo-method")
                .is_none_or(|method| method.eq_ignore_ascii_case("get"))
            && allowed_file(&link, &url)
        {
            file_url = Some(link.clone());
        }
        if a.value().attr("data-turbo-method") == Some("post")
            && link.origin() == url.origin()
            && link.path() == format!("{}/purchase", url.path())
            && label.contains("购买")
        {
            purchase_action = Some(link);
        }
    }
    for form in doc.select(&sel("form[method='post'][action]")) {
        if let Some(action) = form.value().attr("action").and_then(|a| url.join(a).ok())
            && action.origin() == url.origin()
            && action.path() == format!("{}/purchase", url.path())
        {
            purchase_action = Some(action);
            let text = form
                .text()
                .collect::<String>()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            let form_price = text
                .strip_prefix("消耗")
                .and_then(|s| s.strip_suffix("积分下载"))
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u64>().ok());
            if price.is_none() {
                price = form_price;
            }
        }
    }
    let free =
        price == Some(0) || (file_url.is_some() && price.is_none() && purchase_action.is_none());
    let can_acquire_free = price == Some(0)
        && account.status == "authenticated"
        && purchase_action.is_some()
        && csrf.as_ref().is_some_and(|s| !s.is_empty());
    let requires_purchase = !free
        && file_url.is_none()
        && (purchase_action.is_some() || (price.is_some() && account.status != "authenticated"));
    let can_purchase = requires_purchase
        && account.status == "authenticated"
        && account
            .balance
            .zip(price)
            .is_some_and(|(b, p)| p > 0 && b >= p)
        && purchase_action.is_some()
        && csrf.as_ref().is_some_and(|s| !s.is_empty());
    let reason = if file_url.is_some() || can_acquire_free || can_purchase {
        None
    } else if account.status != "authenticated" && !external {
        Some(Message::new("auth.need_cookie"))
    } else if requires_purchase && price.is_none() {
        Some(Message::new("download.unknown_price"))
    } else if requires_purchase && account.balance.is_none() {
        Some(Message::new("auth.balance_unknown"))
    } else if requires_purchase
        && account
            .balance
            .zip(price)
            .is_some_and(|(balance, price)| balance < price)
    {
        Some(Message::new("download.insufficient"))
    } else {
        Some(Message::new("download.site_required"))
    };
    let preview = DownloadPreview {
        source_url: url.to_string(),
        title,
        price,
        balance: account.balance,
        authenticated: account.status == "authenticated",
        requires_purchase,
        can_download: file_url.is_some() || can_acquire_free,
        can_purchase,
        external,
        free,
        reason,
        ..Default::default()
    };
    Ok(DownloadPlan {
        preview,
        file_url,
        purchase_action,
        csrf,
    })
}
fn allowed_file(file: &Url, source: &Url) -> bool {
    file.scheme() == "https"
        && file.username().is_empty()
        && file.password().is_none()
        && file.path() != "/login"
        && file.path() != source.path()
        && !file.path().ends_with("/purchase")
        && (file.host_str() == source.host_str()
            || [
                "d1.acgxp.com",
                "d2.acgxp.com",
                "d3.acgxp.com",
                "file.achost.top",
            ]
            .contains(&file.host_str().unwrap_or_default()))
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct Transfer {
    pub phase: String,
    pub title: String,
    pub received: u64,
    pub total: Option<u64>,
    pub path: Option<String>,
    pub error: Option<Message>,
}
pub fn download(
    plan: &DownloadPlan,
    directory: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Transfer),
) -> Result<String, Message> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Message::new("download.cancelled"));
    }
    let url = plan
        .file_url
        .as_ref()
        .ok_or_else(|| Message::new("download.site_required"))?;
    let client = Client::builder()
        // Patch files bypass system and environment proxies.
        .no_proxy()
        .user_agent("GalGear/0.1")
        .timeout(Duration::from_secs(1800))
        .connect_timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| Message::new("download.network"))?;
    let response = client
        .get(url.clone())
        .header(REFERER, &plan.preview.source_url)
        .send()
        .map_err(|_| Message::new("download.network"))?
        .error_for_status()
        .map_err(|_| Message::new("download.network"))?;
    finish_download(response, plan, directory, cancel, progress)
}

fn finish_download(
    mut response: reqwest::blocking::Response,
    plan: &DownloadPlan,
    directory: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Transfer),
) -> Result<String, Message> {
    if !response.status().is_success() {
        return Err(Message::new("download.network"));
    }
    let url = response.url().clone();
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/html"))
    {
        return Err(Message::new("download.not_file"));
    }
    let total = response.content_length();
    if total.is_some_and(|n| n > 8 * 1024 * 1024 * 1024) {
        return Err(Message::new("download.too_large"));
    }
    std::fs::create_dir_all(directory).map_err(|_| Message::new("download.write"))?;
    let mut file =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| Message::new("download.write"))?;
    let mut received = 0;
    let mut buf = [0u8; 64 * 1024];
    let mut last = std::time::Instant::now() - Duration::from_secs(1);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Message::new("download.cancelled"));
        }
        let n = response
            .read(&mut buf)
            .map_err(|_| Message::new("download.network"))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > 8 * 1024 * 1024 * 1024 {
            return Err(Message::new("download.too_large"));
        }
        file.write_all(&buf[..n])
            .map_err(|_| Message::new("download.write"))?;
        if last.elapsed() > Duration::from_millis(180) {
            progress(Transfer {
                phase: "downloading".into(),
                title: plan.preview.title.clone(),
                received,
                total,
                ..Default::default()
            });
            last = std::time::Instant::now();
        }
    }
    if received == 0 || total.is_some_and(|n| n != received) {
        return Err(Message::new("download.incomplete"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Message::new("download.cancelled"));
    }
    file.as_file()
        .sync_all()
        .map_err(|_| Message::new("download.write"))?;
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .filter(|s| ["zip", "rar", "7z", "exe"].contains(s))
        .unwrap_or("bin");
    let name: String = plan
        .preview
        .title
        .chars()
        .filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control())
        .take(100)
        .collect();
    let name = name.trim_matches([' ', '.']);
    let name = if name.is_empty() {
        "2DFan".to_owned()
    } else {
        let root = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&root.as_str())
        {
            format!("_{name}")
        } else {
            name.to_owned()
        }
    };
    let mut path = directory.join(format!("{name}.{extension}"));
    for index in 1..1000 {
        if !path.exists() {
            break;
        }
        path = directory.join(format!("{name} ({index}).{extension}"));
    }
    file.persist_noclobber(&path)
        .map_err(|_| Message::new("download.write"))?;
    let path = crate::config::display_path(&path);
    progress(Transfer {
        phase: "complete".into(),
        title: plan.preview.title.clone(),
        received,
        total,
        path: Some(path.clone()),
        ..Default::default()
    });
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_zero_point_form_is_free_and_requires_an_authenticated_same_origin_action() {
        let body = r#"<header data-island='NexusHeader' data-props='{"user":{"id":1,"points":14}}'></header><meta name='csrf-token' content='fixture'><h1>Free save</h1><main><form method='post' action='/downloads/2682/purchase'><button type='submit'>消耗 0 积分下载</button></form></main>"#;
        let url = Url::parse("https://2dfmax.top/downloads/2682").unwrap();
        let plan = parse_download(body, url.clone()).unwrap();
        assert_eq!(plan.preview.price, Some(0));
        assert!(plan.preview.free && plan.preview.can_download);
        assert!(!plan.preview.requires_purchase && !plan.preview.can_purchase);
        for invalid in [
            body.replace("\"user\":{\"id\":1,\"points\":14}", "\"user\":null"),
            body.replace("content='fixture'", "content=''"),
            body.replace(
                "action='/downloads/2682/purchase'",
                "action='https://other.example/downloads/2682/purchase'",
            ),
        ] {
            assert!(
                !parse_download(&invalid, url.clone())
                    .unwrap()
                    .preview
                    .can_download
            );
        }
        let paid = body.replace("消耗 0 积分下载", "消耗 2 积分下载");
        let paid = parse_download(&paid, url).unwrap().preview;
        assert!(!paid.free && paid.requires_purchase && paid.can_purchase);
    }

    #[test]
    fn free_acquisition_stops_when_fresh_cost_changes() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let body = r#"<header data-island='NexusHeader' data-props='{"user":{"id":1,"points":14}}'></header><meta name='csrf-token' content='fixture'><h1>Changed cost</h1><main><form method='post' action='/downloads/2682/purchase'><button>消耗 2 积分下载</button></form></main>"#;
        let old = parse_download(
            &body.replace("消耗 2", "消耗 0"),
            base.join("/downloads/2682").unwrap(),
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            assert!(String::from_utf8_lossy(&buffer[..n]).starts_with("GET /downloads/2682 "));
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let session = Session {
            download_client: Client::builder().no_proxy().build().unwrap(),
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            base,
            credentials: FanCredentials::default(),
        };
        assert_eq!(
            session.acquire_free(&old).err().unwrap().key,
            "download.price_changed"
        );
        server.join().unwrap();
    }
    #[test]
    fn balance_is_taken_only_from_authenticated_user_props() {
        let body = r#"<header data-island='NexusHeader' data-props='{"user":{"id":1,"name":"Tester","points":42}}'></header><main>有人拥有9999积分</main>"#;
        assert_eq!(parse_account(body).balance, Some(42));
        assert_eq!(parse_account("<main>积分9999</main>").balance, None);
    }
    #[test]
    fn zero_blank_and_missing_prices_do_not_enable_purchase() {
        let url = Url::parse("https://2dfmax.top/downloads/1").unwrap();
        for price in ["0", "", "未知"] {
            let body = format!(
                "<h1>Patch</h1><main><div class='download-info-item'><span class='download-info-label'>价格</span><span class='download-info-value'>{price} 积分</span></div><a href='/downloads/1/purchase' data-turbo-method='post'>购买</a></main>"
            );
            assert!(
                !parse_download(&body, url.clone())
                    .unwrap()
                    .preview
                    .can_purchase
            );
        }
    }
    #[test]
    fn paid_resources_require_balance_and_explicit_purchase_endpoint() {
        let body = r#"<header data-island='NexusHeader' data-props='{"user":{"id":1,"points":5}}'></header><meta name='csrf-token' content='example'><h1>Patch</h1><main><div class='download-info-item'><span class='download-info-label'>价格</span><span class='download-info-value'>2 积分</span></div><form method='post' action='/downloads/1/purchase'></form></main>"#;
        let plan =
            parse_download(body, Url::parse("https://2dfmax.top/downloads/1").unwrap()).unwrap();
        assert_eq!(plan.preview.price, Some(2));
        assert!(plan.preview.can_purchase);
        let no_balance = body.replace("\"points\":5", "\"name\":\"Tester\"");
        assert!(
            !parse_download(
                &no_balance,
                Url::parse("https://2dfmax.top/downloads/1").unwrap()
            )
            .unwrap()
            .preview
            .can_purchase
        );
        for invalid in [
            body.replace("\"points\":5", "\"points\":1"),
            body.replace("content='example'", "content=''"),
            body.replace(
                "action='/downloads/1/purchase'",
                "action='http://2dfmax.top/downloads/1/purchase'",
            ),
            body.replace(
                "action='/downloads/1/purchase'",
                "action='https://other.example/downloads/1/purchase'",
            ),
        ] {
            assert!(
                !parse_download(
                    &invalid,
                    Url::parse("https://2dfmax.top/downloads/1").unwrap()
                )
                .unwrap()
                .preview
                .can_purchase
            );
        }
    }

    #[test]
    fn cancelled_download_does_not_request_or_create_files() {
        let plan = parse_download(
            "<h1>Patch</h1><main><a href='https://d1.acgxp.com/patch.zip'>直连下载</a></main>",
            Url::parse("https://2dfmax.top/downloads/1").unwrap(),
        )
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            download(&plan, temp.path(), &AtomicBool::new(true), &mut |_| panic!(
                "No progress expected"
            ))
            .unwrap_err()
            .key,
            "download.cancelled"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn purchased_overseas_link_does_not_request_another_purchase() {
        let body = r#"<header data-island='NexusHeader' data-props='{"user":{"id":1,"points":14}}'></header><h1>Patch</h1><main><div class='download-info-item'><span class='download-info-label'>价格</span><span class='download-info-value'>2 积分</span></div><a class='download-btn download-btn--primary' href='https://d1.acgxp.com/uploads/patch.zip?st=example'>海外直连</a></main>"#;
        let url = Url::parse("https://2dfmax.top/downloads/23924").unwrap();
        for label in ["海外直连", "国内直连", "Direct download"] {
            let plan = parse_download(&body.replace("海外直连", label), url.clone()).unwrap();
            assert!(plan.preview.can_download);
            assert!(!plan.preview.requires_purchase);
            assert!(!plan.preview.can_purchase);
            assert!(plan.preview.reason.is_none());
        }
        for blocked in [
            body.replace("d1.acgxp.com", "other.example"),
            body.replace(
                "href='https://d1.acgxp.com/uploads/patch.zip?st=example'",
                "data-turbo-method='post' href='/downloads/23924/purchase'",
            ),
        ] {
            assert!(
                !parse_download(&blocked, url.clone())
                    .unwrap()
                    .preview
                    .can_download
            );
        }
    }

    fn response(bytes: &[u8], content_type: &str) -> reqwest::blocking::Response {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = bytes.to_vec();
        let content_type = content_type.to_owned();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer);
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",body.len(),content_type).unwrap();
            socket.write_all(&body).unwrap();
        });
        Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/patch.zip"))
            .send()
            .unwrap()
    }

    #[test]
    fn download_preserves_existing_files_and_discards_html() {
        let plan = parse_download(
            "<h1>Patch</h1><main><a href='https://d1.acgxp.com/patch.zip'>直连下载</a></main>",
            Url::parse("https://2dfmax.top/downloads/1").unwrap(),
        )
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("Patch.zip"), b"original").unwrap();
        let path = finish_download(
            response(b"new patch", "application/zip"),
            &plan,
            temp.path(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("Patch.zip")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"new patch");
        assert_eq!(
            finish_download(
                response(b"<html>login</html>", "text/html"),
                &plan,
                temp.path(),
                &AtomicBool::new(false),
                &mut |_| {}
            )
            .unwrap_err()
            .key,
            "download.not_file"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    }
}
