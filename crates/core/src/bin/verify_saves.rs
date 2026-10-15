use galgear_core::{Settings, saves, scanner};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = std::env::args().nth(1).ok_or("Provide a library folder")?;
    let games = scanner::scan(&Settings {
        library_folders: vec![folder],
        ..Default::default()
    });
    for game in games.games.iter().filter(|g| g.executable.is_some()) {
        println!(
            "{}: {}",
            game.title,
            serde_json::to_string(&saves::discover(game, None))?
        );
    }
    Ok(())
}
