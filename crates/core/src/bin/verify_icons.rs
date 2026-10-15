use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(std::env::args().nth(1).ok_or("Provide catalog path")?);
    let catalog = galgear_core::catalog::load(&path)?;
    let cache = path.with_file_name("icons");
    let mut count = 0;
    for game in catalog.games.iter().filter(|g| g.available) {
        if let Some(exe) = &game.executable
            && let Some(icon) =
                galgear_core::launcher_icons::cached_icon(std::path::Path::new(exe), &cache)
        {
            assert_eq!(image::open(&icon)?.width(), 32);
            count += 1;
        }
    }
    println!("Extracted and decoded {count} launcher icons without running games.");
    Ok(())
}
