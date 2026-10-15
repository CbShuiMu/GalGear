//! Read-only check of the same public API and transport used by GalGear.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "千恋".into());
    let resources = galgear_core::providers::search_kun(&query)?;
    println!("Kun Gal returned {} resources", resources.len());
    for resource in resources.iter().take(5) {
        println!("{}", serde_json::to_string(resource)?);
    }
    Ok(())
}
