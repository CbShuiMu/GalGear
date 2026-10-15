//! Import an explicitly supplied historical token without copying it into commands or logs.
use galgear_core::{
    credentials,
    fan_account::{Session, Transfer},
};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    sync::atomic::AtomicBool,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let config = PathBuf::from(args.get(1).ok_or("Provide settings path")?);
    let target = credentials::target(&config);
    if let Some(rollout) = args.get(2).filter(|s| !s.starts_with("--")) {
        let mut found = None;
        for line in BufReader::new(std::fs::File::open(rollout)?).lines() {
            let value: serde_json::Value = serde_json::from_str(&line?)?;
            for payload in [&value["payload"], &value["payload"]["item"]] {
                if payload["role"] != "user" {
                    continue;
                }
                for content in payload["content"].as_array().into_iter().flatten() {
                    let text = content["text"].as_str().unwrap_or_default();
                    if text.contains("token刚刚更新") {
                        found = text
                            .lines()
                            .map(str::trim)
                            .find(|s| {
                                s.len() >= 32
                                    && s.len() <= 1024
                                    && s.bytes()
                                        .all(|b| b.is_ascii_alphanumeric() || b"_-/+=".contains(&b))
                            })
                            .map(str::to_owned);
                    }
                }
            }
        }
        let token = found.ok_or("No explicitly updated user token found")?;
        let mut value = credentials::load(&target)?;
        value.token = token;
        value.mode = "csrf".into();
        credentials::save(&target, &value)?;
        println!(
            "Previously supplied token imported into Windows Credential Manager (value redacted)."
        );
    }
    let value = credentials::load(&target)?;
    for mode in ["csrf", "bearer"] {
        let mut test = value.clone();
        test.mode = mode.into();
        let session = Session::new("https://2dfmax.top", test)?;
        let account = session.check()?;
        println!(
            "Read-only {mode} test: status={}, balance={:?}, token_saved={}, cookie_saved={}",
            account.status, account.balance, account.token_saved, account.cookie_saved
        );
    }
    let session = Session::new("https://2dfmax.top", value)?;
    if args.iter().any(|s| s == "--download-free") {
        let url = args.get(3).ok_or("Provide a free resource path")?;
        let before = session.check()?.balance;
        let plan = session.inspect(url)?;
        if !plan.preview.free || !plan.preview.can_download || plan.preview.requires_purchase {
            return Err("Resource not confirmed free; no acquisition will be submitted".into());
        }
        let plan = session.acquire_free(&plan)?;
        let temp = tempfile::tempdir()?;
        let path = session.download(&plan, temp.path(), &AtomicBool::new(false), &mut |_| {})?;
        let data = std::fs::read(path)?;
        println!(
            "Free download verified: {} bytes, RAR={}, balance before={:?}, after={:?}; temporary file only.",
            data.len(),
            data.starts_with(b"Rar!\x1a\x07"),
            before,
            session.check()?.balance
        );
        return Ok(());
    }
    if args.iter().any(|s| s == "--structure") {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &session.resource_structure(
                    args.get(3)
                        .map(String::as_str)
                        .unwrap_or("/downloads/23924")
                )?
            )?
        );
        println!("Read-only structure inspection; no purchase submitted.");
        return Ok(());
    }
    if args.iter().any(|s| s == "--download-owned") {
        let before = session.check()?.balance;
        let plan = session.inspect("/downloads/23924")?;
        if plan.preview.requires_purchase || !plan.preview.can_download {
            return Err("Owned file unavailable; no purchase will be attempted".into());
        }
        let temp = tempfile::tempdir()?;
        let mut last = std::time::Instant::now();
        let path = session.download(
            &plan,
            temp.path(),
            &AtomicBool::new(false),
            &mut |t: Transfer| {
                if last.elapsed().as_secs() >= 2 {
                    println!("Download progress: {} bytes / {:?}", t.received, t.total);
                    last = std::time::Instant::now();
                }
            },
        )?;
        let file = std::fs::File::open(&path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let count = archive.len();
        for index in 0..count {
            std::io::copy(&mut archive.by_index(index)?, &mut std::io::sink())?;
        }
        println!(
            "Owned ZIP downloaded and CRC-checked: {} bytes, {} entries; balance before={:?}, after={:?}; no purchase request submitted.",
            std::fs::metadata(path)?.len(),
            count,
            before,
            session.check()?.balance
        );
        return Ok(());
    }
    let paid = session.inspect("https://2dfmax.top/downloads/23924")?;
    println!(
        "Paid patch inspection: price={:?}, can_purchase={}, requires_purchase={}",
        paid.preview.price, paid.preview.can_purchase, paid.preview.requires_purchase
    );
    let free = session.inspect("https://2dfmax.top/downloads/38576")?;
    println!(
        "Public file inspection: can_download={}, requires_purchase={}",
        free.preview.can_download, free.preview.requires_purchase
    );
    if free.preview.can_download {
        let temp = tempfile::tempdir()?;
        let path = galgear_core::fan_account::download(
            &free,
            temp.path(),
            &AtomicBool::new(false),
            &mut |_t: Transfer| {},
        )?;
        println!(
            "Public direct download passed: {} bytes (temporary folder; not installed).",
            std::fs::metadata(path)?.len()
        );
    }
    println!("No purchase request submitted.");
    Ok(())
}
