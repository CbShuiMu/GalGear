use galgear_core::auto_unlock;
use std::path::Path;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("analyze") if args.len() == 3 => auto_unlock::analyze(Path::new(&args[2])),
        Some("install") if args.len() == 3 => auto_unlock::install(Path::new(&args[2])),
        Some("restore") if args.len() == 3 => auto_unlock::restore(Path::new(&args[2])),
        _ => {
            eprintln!("Usage: auto_unlock analyze|install|restore GAME_FOLDER");
            std::process::exit(2)
        }
    };
    match result {
        Ok(report) => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1)
        }
    }
}
