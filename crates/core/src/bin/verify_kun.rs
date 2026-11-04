//! Read-only check of the same public API and transport used by GalGear.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credentials = galgear_core::data_sources::Credentials {
        patch_key: std::env::var("GALGEAR_NEXTMOE_PATCH_KEY").unwrap_or_default(),
        ..Default::default()
    };
    let query = std::env::args().nth(1).unwrap_or_else(|| "千恋".into());
    let resources = galgear_core::data_sources::moyu::search(&query, &credentials)?;
    println!("NextMoe returned {} resources", resources.len());
    for resource in resources.iter().take(5) {
        println!("{}", serde_json::to_string(resource)?);
    }
    Ok(())
}
