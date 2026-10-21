//! Data left by the app under its former name, ObsiRust (up to 0.2.x):
//! app settings and recent vaults, per-vault config files, hotkey ids and
//! the `.gitignore` rule of the workspace file. Read as a fallback (or
//! copied once) so that renaming the app loses nothing.

use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::VaultBackend;

/// The app name before Oxinote.
pub const OLD_NAME: &str = "ObsiRust";
/// Prefix of the app's own hotkey ids before Oxinote.
pub const OLD_HOTKEY_PREFIX: &str = "obsirust:";
/// Per-vault config files (in `.obsidian/`): (new name, old name).
pub const CONFIG_FILES: [(&str, &str); 2] = [
    ("oxinote-git.json", "obsirust-git.json"),
    ("workspace-oxinote.json", "workspace-obsirust.json"),
];

/// Copy `old` to `new` (recursively) when only `old` exists. The old copy
/// is kept, so that an older version still finds its data.
pub fn copy_dir_once(old: &Path, new: &Path) -> std::io::Result<bool> {
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    copy_dir(old, new)?;
    Ok(true)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// Folders of the former app: (old, new) pairs for the egui state (settings,
/// window) and the config folder (recent vaults).
fn app_dirs() -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    if let (Some(old), Some(new)) = (
        eframe::storage_dir(OLD_NAME),
        eframe::storage_dir(crate::APP_NAME),
    ) {
        out.push((old, new));
    }
    if let (Some(old), Some(new)) = (
        directories::ProjectDirs::from("", "", OLD_NAME),
        directories::ProjectDirs::from("", "", crate::APP_NAME),
    ) {
        out.push((
            old.config_dir().to_path_buf(),
            new.config_dir().to_path_buf(),
        ));
    }
    out
}

/// On the first start under the new name, take over the former settings and
/// recent vaults. Errors only cost the old settings: reported, not fatal.
pub fn migrate_app_dirs() {
    for (old, new) in app_dirs() {
        if let Err(e) = copy_dir_once(&old, &new) {
            eprintln!(
                "cannot copy the settings of {OLD_NAME} ({}): {e}",
                old.display()
            );
        }
    }
}

/// A per-vault config file, or its file under the former name.
pub fn read_config_file(vault: &dyn VaultBackend, name: &str) -> Option<String> {
    vault.read_config_file(name).or_else(|| {
        CONFIG_FILES
            .iter()
            .find(|(new, _)| *new == name)
            .and_then(|(_, old)| vault.read_config_file(old))
    })
}

/// A hotkey id of the app under its former name, if `id` is one of ours.
pub fn old_hotkey_id(id: &str) -> Option<String> {
    id.strip_prefix("oxinote:")
        .map(|rest| format!("{OLD_HOTKEY_PREFIX}{rest}"))
}

/// Add the new workspace file next to the old one in a `.gitignore` that
/// ignores the old one (else the renamed file would get committed).
pub fn gitignore_with_new_rules(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 40);
    for line in text.lines() {
        out.push_str(line);
        out.push('\n');
        for (new, old) in CONFIG_FILES {
            let old_rule = format!(".obsidian/{old}");
            let new_rule = format!(".obsidian/{new}");
            if line.trim() == old_rule && !text.lines().any(|l| l.trim() == new_rule) {
                out.push_str(&new_rule);
                out.push('\n');
            }
        }
    }
    if !text.ends_with('\n') && out.ends_with('\n') && !text.is_empty() {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_the_old_folder_once() {
        let tmp = tempfile::tempdir().expect("tmp");
        let old = tmp.path().join("obsirust");
        let new = tmp.path().join("oxinote");
        fs::create_dir_all(old.join("sub")).expect("mkdir");
        fs::write(old.join("app.ron"), b"settings").expect("write");
        fs::write(old.join("sub/recent_vaults.json"), b"[]").expect("write");
        assert!(copy_dir_once(&old, &new).expect("copy"));
        assert_eq!(fs::read(new.join("app.ron")).expect("read"), b"settings");
        assert!(new.join("sub/recent_vaults.json").is_file());
        // The old data stays; a second run changes nothing.
        assert!(old.join("app.ron").is_file());
        fs::write(new.join("app.ron"), b"newer").expect("write");
        assert!(!copy_dir_once(&old, &new).expect("copy"));
        assert_eq!(fs::read(new.join("app.ron")).expect("read"), b"newer");
        // Nothing to take over.
        assert!(!copy_dir_once(&tmp.path().join("none"), &tmp.path().join("x")).expect("copy"));
    }

    #[test]
    fn maps_hotkey_ids() {
        assert_eq!(
            old_hotkey_id("oxinote:close-vault").as_deref(),
            Some("obsirust:close-vault")
        );
        assert_eq!(old_hotkey_id("editor:toggle-bold"), None);
    }

    #[test]
    fn adds_the_new_workspace_rule() {
        let old = "# local\n.obsidian/workspace.json\n.obsidian/workspace-obsirust.json\n.trash/\n";
        let new = gitignore_with_new_rules(old);
        assert_eq!(
            new,
            "# local\n.obsidian/workspace.json\n.obsidian/workspace-obsirust.json\n\
             .obsidian/workspace-oxinote.json\n.trash/\n"
        );
        // Idempotent, and nothing to add without the old rule.
        assert_eq!(gitignore_with_new_rules(&new), new);
        assert_eq!(gitignore_with_new_rules(".trash/"), ".trash/");
        assert_eq!(gitignore_with_new_rules(""), "");
    }
}
