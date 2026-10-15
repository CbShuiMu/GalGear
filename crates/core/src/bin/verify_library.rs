//! Temporary 1,000-game acceptance fixture and read-only real save discovery.
use galgear_core::{Settings, catalog, saves, scanner};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    for i in 0..1000 {
        let dir = temp.path().join(format!("Game {i:04}"));
        std::fs::create_dir(&dir)?;
        std::fs::write(dir.join("story.exe"), b"fixture")?;
        std::fs::write(dir.join("data.xp3"), b"fixture")?;
    }
    let settings = Settings {
        library_folders: vec![temp.path().to_string_lossy().into_owned()],
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let first = scanner::scan(&settings);
    println!(
        "full scan: {} games / {} ms",
        first.games.len(),
        start.elapsed().as_millis()
    );
    assert_eq!(first.games.len(), 1000);
    catalog::save(&temp.path().join("catalog.json"), &first)?;
    let start = std::time::Instant::now();
    let cached = catalog::load(&temp.path().join("catalog.json"))?;
    println!(
        "cache load: {} games / {} ms",
        cached.games.len(),
        start.elapsed().as_millis()
    );
    let start = std::time::Instant::now();
    let incremental =
        scanner::scan_incremental(&settings, &first, &Default::default(), &mut |_| {});
    println!(
        "incremental scan: {} games / {} ms",
        incremental.games.len(),
        start.elapsed().as_millis()
    );
    if let Some(path) = std::env::args().nth(1) {
        let real = scanner::scan(&Settings {
            library_folders: vec![path],
            ..Default::default()
        });
        for game in real.games.iter().filter(|g| g.executable.is_some()) {
            let report = saves::discover(game, None);
            println!("{}: {}", game.title, serde_json::to_string(&report)?);
        }
    }
    Ok(())
}
