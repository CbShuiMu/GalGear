//! Read-only verification: never purchases or executes downloads.
use galgear_core::{catalog, data_sources, providers};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credentials = galgear_core::data_sources::Credentials {
        patch_key: std::env::var("GALGEAR_NEXTMOE_PATCH_KEY").unwrap_or_default(),
        ..Default::default()
    };
    let query = std::env::args().nth(1).unwrap_or_else(|| "千恋".into());
    let subjects = providers::search_fan("https://2dfmax.top", &query, None)?;
    println!("2DFan subjects: {}", subjects.len());
    if let Some(subject) = subjects.first() {
        let metadata = providers::fan_metadata("https://2dfmax.top", &subject.url, None)?;
        println!(
            "2DFan metadata: {} / {} / {} / cover={}",
            metadata.title,
            metadata.developer,
            metadata.release_date,
            metadata.cover_url.is_some()
        );
        if metadata.cover_url.is_some() {
            let temp = tempfile::tempdir()?;
            let game = catalog::manual_game(&temp.path().join("game.exe"), "Verification".into())?;
            let (_, path, _) = providers::save_cover(&game, &metadata, &temp.path().join("cache"))?;
            println!(
                "Cover downloaded and decoded: {} bytes",
                std::fs::metadata(path)?.len()
            );
        }
        let patches = providers::fan_patches(
            "https://2dfmax.top",
            &format!("{}/downloads", subject.url),
            None,
        )?;
        println!("2DFan resource pages: {}", patches.len());
    }
    let combined = providers::search_patch_sites("https://2dfmax.top", &query, &credentials, None)?;
    let fan = combined
        .resources
        .iter()
        .filter(|r| r.source == "2DFan")
        .collect::<Vec<_>>();
    assert!(!fan.is_empty());
    assert!(
        fan.iter()
            .all(|r| !["汉化", "存档", "AI", "其他"].contains(&r.title.as_str()))
    );
    println!("2DFan named search resources: {}", fan.len());
    for resource in fan.iter().take(2) {
        println!("2DFan patch: {} / {}", resource.title, resource.description);
    }
    let kun = data_sources::moyu::search(&query, &credentials)?;
    println!("NextMoe patch resources: {}", kun.len());
    for resource in kun.iter().take(2) {
        println!("NextMoe: {} / {}", resource.title, resource.url);
    }
    if subjects.is_empty() || kun.is_empty() {
        return Err("Expected nonempty site results".into());
    }
    Ok(())
}
