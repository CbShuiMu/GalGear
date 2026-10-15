//! Read public subject metadata and optionally keep a reproducible parser fixture.
use galgear_core::providers;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://2dfmax.top/subjects/2111".into());
    let parsed = reqwest::Url::parse(&url)?;
    let body = providers::page(&providers::client()?, parsed.clone())?;
    if let Some(path) = std::env::args().nth(2) {
        std::fs::write(path, &body)?;
    }
    let metadata = providers::parse_metadata(&body, &parsed)?;
    println!("{}", serde_json::to_string_pretty(&metadata)?);
    Ok(())
}
