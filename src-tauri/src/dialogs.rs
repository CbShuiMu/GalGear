use crate::{AppState, snapshot};
use galgear_i18n::Message;
use std::path::{Path, PathBuf};

pub fn nearest_directory(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf)
}

pub fn start(
    state: &AppState,
    role: &str,
    context: &[PathBuf],
) -> Result<Option<PathBuf>, Message> {
    let settings = snapshot(state)?;
    let previous = settings.dialog_directories.get(role).map(PathBuf::from);
    // Task context takes precedence over Windows' global last-used directory.
    let candidates = context.iter().chain(previous.iter()).collect::<Vec<_>>();
    Ok(candidates
        .iter()
        .find_map(|p| {
            if p.is_dir() {
                Some((*p).clone())
            } else if p.is_file() {
                p.parent().map(Path::to_path_buf)
            } else {
                None
            }
        })
        .or_else(|| candidates.iter().find_map(|p| nearest_directory(p))))
}

pub fn dialog(directory: Option<PathBuf>) -> rfd::FileDialog {
    match directory {
        Some(path) => rfd::FileDialog::new().set_directory(path),
        None => rfd::FileDialog::new(),
    }
}

pub fn remember(state: &AppState, role: &str, selection: &Path) -> Result<(), Message> {
    let directory = if selection.is_dir() {
        selection
    } else {
        selection.parent().unwrap_or(selection)
    };
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| Message::new("error.config_state"))?;
    let mut next = settings.clone();
    next.dialog_directories
        .insert(role.into(), galgear_core::config::display_path(directory));
    state.store.save(&next)?;
    *settings = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleted_folders_and_files_resolve_to_an_existing_parent() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            nearest_directory(&temp.path().join("removed/game.exe")),
            Some(temp.path().to_owned())
        );
        let file = temp.path().join("image.png");
        std::fs::write(&file, b"image").unwrap();
        assert_eq!(nearest_directory(&file), Some(temp.path().to_owned()));
    }
}
