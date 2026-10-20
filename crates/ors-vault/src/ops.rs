//! File and folder operations of [`Vault`]: writing, creating, renaming,
//! deleting (with trash handling).

use std::fs;
use std::path::Path;

use ors_core::{Error, NotePath, Result};

use crate::fsutil::{self, atomic_write, exists_exact, split_ext, temp_sibling, unique_name};
use crate::naming::sanitize_file_name;
use crate::{LOCAL_TRASH_DIR, TrashMode, Vault};

/// Rename `from` to `to` on disk. `case_only` triggers a two-step rename via a
/// temporary name so it also works on case-insensitive filesystems.
fn do_rename(from: &Path, to: &Path, case_only: bool) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if case_only {
        let tmp = temp_sibling(from);
        fs::rename(from, &tmp)?;
        if let Err(e) = fs::rename(&tmp, to) {
            let _ = fs::rename(&tmp, from);
            return Err(e);
        }
        Ok(())
    } else {
        fs::rename(from, to)
    }
}

/// Decide whether renaming `from` → `to` is allowed. Returns `Ok(case_only)`.
fn check_rename_target(
    from_rel: &str,
    to_rel: &str,
    to_abs: &Path,
    to_err: impl FnOnce() -> Error,
) -> Result<bool> {
    let case_only = from_rel != to_rel && from_rel.to_lowercase() == to_rel.to_lowercase();
    if to_abs.symlink_metadata().is_ok() {
        // On a case-insensitive FS `to` "exists" when it only differs in case
        // from `from`; that is fine as long as no entry has exactly that name.
        if !(case_only && !exists_exact(to_abs)) {
            return Err(to_err());
        }
    }
    Ok(case_only)
}

impl Vault {
    /// Atomically write raw bytes (attachments), creating parent folders.
    pub fn write_bytes(&self, path: &NotePath, bytes: &[u8]) -> Result<()> {
        let abs = self.resolve(path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
            // Re-check now that the parents exist (a parent may be a symlink).
            self.check_inside(parent)?;
        }
        atomic_write(&abs, bytes)?;
        Ok(())
    }

    /// `true` if `folder` is an existing folder of the vault (`""` = root).
    pub fn folder_exists(&self, folder: &str) -> bool {
        self.resolve_folder(folder).is_ok_and(|(_, p)| p.is_dir())
    }

    pub(crate) fn create_folder_impl(&self, folder: &str) -> Result<()> {
        let (norm, abs) = self.resolve_folder(folder)?;
        if norm.is_empty() {
            return Ok(());
        }
        if abs.exists() && !abs.is_dir() {
            return Err(Error::AlreadyExists(NotePath::new(&norm)?));
        }
        fs::create_dir_all(&abs)?;
        self.check_inside(&abs)?;
        Ok(())
    }

    /// Create a new folder named `Untitled`, `Untitled 1`, ... inside
    /// `parent` (`""` = root) like Obsidian's "New folder". Returns its path.
    pub fn create_untitled_folder(&self, parent: &str) -> Result<String> {
        self.create_unique_folder(parent, "Untitled")
    }

    /// Create a folder `name` in `parent`, appending ` 1`, ` 2`, ... if taken.
    /// `name` is sanitized. Returns the vault path of the new folder.
    pub fn create_unique_folder(&self, parent: &str, name: &str) -> Result<String> {
        let (norm, abs) = self.resolve_folder(parent)?;
        fs::create_dir_all(&abs)?;
        let unique = unique_name(&abs, &sanitize_file_name(name), "");
        let rel = if norm.is_empty() {
            unique
        } else {
            format!("{norm}/{unique}")
        };
        self.create_folder_impl(&rel)?;
        Ok(rel)
    }

    pub(crate) fn create_note_impl(
        &self,
        folder: &str,
        name: &str,
        contents: &str,
    ) -> Result<NotePath> {
        let (norm, dir) = self.resolve_folder(folder)?;
        let name = name.trim();
        let base = name
            .strip_suffix(".md")
            .or_else(|| name.strip_suffix(".MD"))
            .unwrap_or(name);
        let stem = sanitize_file_name(base);
        fs::create_dir_all(&dir)?;
        self.check_inside(&dir)?;
        // Retry on the (unlikely) race where another process grabs the name.
        for _ in 0..16 {
            let file_name = unique_name(&dir, &stem, ".md");
            let path = NotePath::join(&norm, &file_name)?;
            let abs = self.resolve(&path)?;
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&abs)
            {
                Ok(f) => {
                    drop(f);
                    if !contents.is_empty() {
                        atomic_write(&abs, contents.as_bytes())?;
                    }
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::Io(e)),
            }
        }
        Err(Error::Other("could not find a free note name".into()))
    }

    /// Create an `Untitled` note in the folder chosen by `newFileLocation`
    /// (see [`Vault::new_note_folder`]). `current_folder` is the folder of the
    /// active note, if any.
    pub fn create_untitled_note(
        &self,
        current_folder: Option<&str>,
        contents: &str,
    ) -> Result<NotePath> {
        let folder = self.new_note_folder(current_folder)?;
        self.create_note(&folder, "Untitled", contents)
    }

    pub(crate) fn rename_file_impl(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        let from_abs = self.resolve(from)?;
        let to_abs = self.resolve(to)?;
        if !from_abs.is_file() {
            return Err(Error::NotFound(from.clone()));
        }
        if from == to {
            return Ok(());
        }
        let case_only = check_rename_target(from.as_str(), to.as_str(), &to_abs, || {
            Error::AlreadyExists(to.clone())
        })?;
        if let Some(parent) = to_abs.parent() {
            fs::create_dir_all(parent)?;
            self.check_inside(parent)?;
        }
        do_rename(&from_abs, &to_abs, case_only)?;
        Ok(())
    }

    pub(crate) fn rename_folder_impl(&self, from: &str, to: &str) -> Result<()> {
        let (from_norm, from_abs) = self.resolve_folder(from)?;
        let (to_norm, to_abs) = self.resolve_folder(to)?;
        if from_norm.is_empty() || to_norm.is_empty() {
            return Err(Error::InvalidPath("cannot rename the vault root".into()));
        }
        if !from_abs.is_dir() {
            return Err(Error::NotFound(NotePath::new(&from_norm)?));
        }
        if from_norm == to_norm {
            return Ok(());
        }
        if to_norm.starts_with(&format!("{from_norm}/")) {
            return Err(Error::InvalidPath(format!(
                "cannot move {from_norm} into itself ({to_norm})"
            )));
        }
        let to_path = NotePath::new(&to_norm)?;
        let case_only = check_rename_target(&from_norm, &to_norm, &to_abs, || {
            Error::AlreadyExists(to_path.clone())
        })?;
        if let Some(parent) = to_abs.parent() {
            fs::create_dir_all(parent)?;
            self.check_inside(parent)?;
        }
        do_rename(&from_abs, &to_abs, case_only)?;
        Ok(())
    }

    pub(crate) fn delete_file_impl(&self, path: &NotePath, mode: TrashMode) -> Result<()> {
        let abs = self.resolve(path)?;
        let meta = abs
            .symlink_metadata()
            .map_err(|e| fsutil::map_io(e, path))?;
        if meta.is_dir() {
            return Err(Error::InvalidPath(format!(
                "{path} is a folder, use delete_folder"
            )));
        }
        self.trash_entry(&abs, mode, false)
    }

    /// Delete a folder and its contents using the trash mode from
    /// `app.json`.
    pub fn delete_folder(&self, folder: &str) -> Result<()> {
        let mode = self.app_config()?.trash_mode;
        self.delete_folder_with(folder, mode)
    }

    /// Delete a folder and its contents honoring `mode`. The vault root
    /// cannot be deleted.
    pub fn delete_folder_with(&self, folder: &str, mode: TrashMode) -> Result<()> {
        let (norm, abs) = self.resolve_folder(folder)?;
        if norm.is_empty() {
            return Err(Error::InvalidPath("cannot delete the vault root".into()));
        }
        if !abs.is_dir() {
            return Err(Error::NotFound(NotePath::new(&norm)?));
        }
        let is_link = abs.symlink_metadata().is_ok_and(|m| m.is_symlink());
        self.trash_entry(&abs, mode, !is_link)
    }

    /// Move `abs` to the trash according to `mode`.
    fn trash_entry(&self, abs: &Path, mode: TrashMode, is_dir: bool) -> Result<()> {
        match mode {
            TrashMode::System => match trash::delete(abs) {
                Ok(()) => Ok(()),
                Err(_) => self.local_trash(abs, is_dir),
            },
            TrashMode::Local => self.local_trash(abs, is_dir),
            TrashMode::Permanent => {
                if is_dir {
                    fs::remove_dir_all(abs)?;
                } else {
                    fs::remove_file(abs)?;
                }
                Ok(())
            }
        }
    }

    /// Move `abs` into `<vault>/.trash/`, picking a unique name.
    fn local_trash(&self, abs: &Path, is_dir: bool) -> Result<()> {
        let trash_dir = self.root.join(LOCAL_TRASH_DIR);
        fs::create_dir_all(&trash_dir)?;
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error::InvalidPath(abs.display().to_string()))?;
        let (stem, ext) = if is_dir {
            (name.as_str(), "")
        } else {
            split_ext(&name)
        };
        let unique = unique_name(&trash_dir, stem, ext);
        fs::rename(abs, trash_dir.join(unique))?;
        Ok(())
    }

    /// Folder (vault-relative) where a new note should be created, honoring
    /// `newFileLocation` from `app.json`. See [`crate::AppConfig::new_note_folder`].
    pub fn new_note_folder(&self, current_folder: Option<&str>) -> Result<String> {
        Ok(self.app_config()?.new_note_folder(current_folder))
    }

    /// Folder (vault-relative) where an attachment pasted into `note` should
    /// go, honoring `attachmentFolderPath`. See
    /// [`crate::AppConfig::attachment_folder`].
    pub fn attachment_folder_for(&self, note: Option<&NotePath>) -> Result<String> {
        Ok(self.app_config()?.attachment_folder(note))
    }

    /// A free vault path for a new attachment named `file_name` (sanitized)
    /// for `note`: `<attachment folder>/<name>`, or `<name> 1.<ext>`, ... if
    /// taken. Nothing is created.
    pub fn available_attachment_path(
        &self,
        note: Option<&NotePath>,
        file_name: &str,
    ) -> Result<NotePath> {
        let folder = self.attachment_folder_for(note)?;
        let (norm, dir) = self.resolve_folder(&folder)?;
        let (stem, ext) = split_ext(file_name);
        let ext: String = ext
            .chars()
            .filter(|c| !crate::FORBIDDEN_NAME_CHARS.contains(c))
            .collect();
        let name = unique_name(&dir, &sanitize_file_name(stem), &ext);
        NotePath::join(&norm, &name)
    }
}
