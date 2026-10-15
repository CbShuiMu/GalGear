use galgear_core::{Settings, scanner};
fn main() {
    let folders: Vec<String> = std::env::args().skip(1).collect();
    if folders.is_empty() {
        eprintln!("Usage: cargo run -p galgear-core --bin scan -- F:\\GALGame");
        std::process::exit(2);
    }
    let settings = Settings {
        library_folders: folders,
        ..Default::default()
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&scanner::scan(&settings)).unwrap()
    );
}
