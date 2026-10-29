//! Required AI resources are resolved relative to the application executable.
use std::path::{Path, PathBuf};

pub const SKILL_DIRECTORY: &str = "Galgear-GalgameReverse-Skill/GalgameResourceExtract";

pub fn host(home: &Path) -> Option<PathBuf> {
    [
        "tools/ai-agent/GalGear.AiAgent.exe",
        "ai-agent/GalGear.AiAgent.exe",
    ]
    .iter()
    .map(|relative| home.join(relative))
    .find(|path| path.is_file())
}

pub fn available(home: &Path) -> bool {
    host(home).is_some() && home.join(SKILL_DIRECTORY).join("SKILL.md").is_file()
}

pub fn home() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn requires_both_packaged_host_and_skill_document() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        assert!(!available(home));
        fs::create_dir_all(home.join("tools/ai-agent")).unwrap();
        let executable = home.join("tools/ai-agent/GalGear.AiAgent.exe");
        fs::write(&executable, []).unwrap();
        assert!(!available(home));
        fs::create_dir_all(home.join(SKILL_DIRECTORY)).unwrap();
        assert!(!available(home));
        fs::write(home.join(SKILL_DIRECTORY).join("SKILL.md"), "skill").unwrap();
        assert!(available(home));
        fs::remove_file(executable).unwrap();
        assert!(!available(home));
    }
}
