use galgear_core::unlock;
use std::path::Path;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("preview") if args.len() == 4 => unlock::build(Path::new(&args[2])).and_then(|data| {
            std::fs::write(&args[3], data)
                .map_err(|e| galgear_i18n::Message::detail("unlock.io", e))
        }),
        Some("install") if args.len() == 3 => unlock::install(Path::new(&args[2])),
        Some("restore") if args.len() == 3 => unlock::restore(Path::new(&args[2])),
        _ => {
            eprintln!(
                "Usage: cg_unlock preview GAME_FOLDER OUTPUT | install GAME_FOLDER | restore GAME_FOLDER"
            );
            std::process::exit(2);
        }
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
