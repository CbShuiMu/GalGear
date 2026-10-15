//! Exercise a local archive exclusively against a temporary target folder.
use galgear_core::patches;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source =
        std::path::PathBuf::from(std::env::args().nth(1).ok_or("Provide a patch archive")?);
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("game");
    std::fs::create_dir(&root)?;
    let plan = patches::preview(&source, &root, "verification".into())?;
    println!("Archive preview: {} files", plan.preview.files.len());
    let data = temp.path().join("backup");
    let record = patches::install(plan, &data)?;
    patches::restore(&data, &record.id)?;
    println!("Install and restore passed in temporary folder");
    Ok(())
}
