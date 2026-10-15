//! Read-only provider verification, with optional credentials from standard input.
use galgear_core::data_sources::{self, Credentials};
use std::{io::Read, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", error.render(galgear_i18n::Language::Chinese));
        std::process::exit(1);
    }
}
fn run() -> Result<(), galgear_i18n::Message> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let config = args.get(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
            .join("com.galgear.desktop")
            .join("settings.json")
    });
    if args.first().is_some_and(|arg| arg == "--configure") {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|_| galgear_i18n::Message::new("auth.invalid"))?;
        let values: std::collections::BTreeMap<String, Credentials> =
            serde_json::from_str(&input).map_err(|_| galgear_i18n::Message::new("auth.invalid"))?;
        for (source, value) in values {
            data_sources::check(&source, &value)?;
            data_sources::save(&config, &source, &value)?;
            println!("{source}: credentials verified and saved to Windows vault");
        }
        return Ok(());
    }
    let source = args.first().map(String::as_str).unwrap_or("ymgal");
    let query = args.get(2).map(String::as_str).unwrap_or("千恋万花");
    let credentials = data_sources::load(&config, source)?;
    let resources = data_sources::search(source, "https://2dfmax.top", query, &credentials)?;
    println!("{source}: {} candidates", resources.len());
    if let Some(resource) = resources.first() {
        let metadata = data_sources::metadata("https://2dfmax.top", &resource.url, &credentials)?;
        println!(
            "{} | {} | {} | story: {} chars | {} tags",
            metadata.title,
            metadata.brand,
            metadata.release_date,
            metadata.description.chars().count(),
            metadata.tags.len()
        );
    }
    Ok(())
}
