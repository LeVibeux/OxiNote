//! Low-level filesystem helpers: sandboxed path resolution, directory walking,
//! atomic writes and unique-name generation.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use oxi_core::{Error, NotePath, Result};

use crate::Vault;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Map an I/O error on `path` to the contract error type.
pub(crate) fn map_io(e: io::Error, path: &NotePath) -> Error {
    if e.kind() == io::ErrorKind::NotFound {
        Error::NotFound(path.clone())
    } else {
        Error::Io(e)
    }
}

/// `true` for names Obsidian hides from the vault (`.obsidian`, `.trash`,
/// `.git`, any dotfile).
pub(crate) fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// `true` if any component of a vault-relative path is hidden.
pub(crate) fn is_hidden_rel(rel: &str) -> bool {
    rel.split('/').any(is_hidden_name)
}

/// A single path segment must be exactly one normal component (rejects drive
/// prefixes, root dirs, `..`, embedded separators, NUL bytes).
fn check_segment(seg: &str) -> Result<()> {
    if seg.is_empty() || seg.contains('\0') {
        return Err(Error::InvalidPath(seg.to_string()));
    }
    let mut comps = Path::new(seg).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(c)), None) if c == OsStr::new(seg) => Ok(()),
        _ => Err(Error::InvalidPath(seg.to_string())),
    }
}

/// Normalize a vault-relative folder string. `""` / `"/"` is the root.
pub(crate) fn normalize_folder(folder: &str) -> Result<String> {
    let trimmed = folder.replace('\\', "/");
    let trimmed = trimmed.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    Ok(NotePath::new(trimmed)?.as_str().to_string())
}

/// Split a file name into `(stem, extension-with-dot)`.
pub(crate) fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Does `dir` contain an entry named `name`, compared case-insensitively?
pub(crate) fn exists_ci(dir: &Path, name: &str) -> bool {
    if dir.join(name).symlink_metadata().is_ok() {
        return true;
    }
    let lower = name.to_lowercase();
    fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().to_lowercase() == lower)
    })
}

/// Does `dir` contain an entry with exactly this name (case-sensitive)?
pub(crate) fn exists_exact(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    if path.symlink_metadata().is_err() {
        return false;
    }
    fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| e.file_name() == name))
}

/// First free name among `stem.ext`, `stem 1.ext`, `stem 2.ext`, ... in `dir`
/// (case-insensitive check, like Obsidian on every platform).
pub(crate) fn unique_name(dir: &Path, stem: &str, ext: &str) -> String {
    let first = format!("{stem}{ext}");
    if !exists_ci(dir, &first) {
        return first;
    }
    let mut n: u64 = 1;
    loop {
        let candidate = format!("{stem} {n}{ext}");
        if !exists_ci(dir, &candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Atomically write `bytes` to `target`: write a hidden temp file in the same
/// folder, fsync it and rename it over the target. Parent folders are created.
pub(crate) fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".~ors-{}-{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Ok(meta) = fs::metadata(target) {
            if meta.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::IsADirectory,
                    "target is a directory",
                ));
            }
            // Keep the permissions of the file we replace.
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Temporary sibling name used for two-step (case-only) renames.
pub(crate) fn temp_sibling(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(path);
    dir.join(format!(
        ".~oxi-rename-{}-{}",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

impl Vault {
    /// Resolve a vault path to an absolute path, rejecting anything that would
    /// end up outside the vault (including through symlinks).
    pub(crate) fn resolve(&self, path: &NotePath) -> Result<PathBuf> {
        let mut abs = self.root.clone();
        for seg in path.as_str().split('/') {
            check_segment(seg)?;
            abs.push(seg);
        }
        self.check_inside(&abs)?;
        Ok(abs)
    }

    /// Resolve a vault-relative folder (`""` = root).
    pub(crate) fn resolve_folder(&self, folder: &str) -> Result<(String, PathBuf)> {
        let norm = normalize_folder(folder)?;
        if norm.is_empty() {
            return Ok((norm, self.root.clone()));
        }
        let abs = self.resolve(&NotePath::new(&norm)?)?;
        Ok((norm, abs))
    }

    /// Check that the deepest existing ancestor of `abs` (after resolving
    /// symlinks) lies inside the canonical root.
    pub(crate) fn check_inside(&self, abs: &Path) -> Result<()> {
        for ancestor in abs.ancestors() {
            if let Ok(canon) = fs::canonicalize(ancestor) {
                if canon.starts_with(&self.canonical_root) {
                    return Ok(());
                }
                return Err(Error::InvalidPath(format!(
                    "{} is outside the vault",
                    abs.display()
                )));
            }
        }
        Err(Error::InvalidPath(abs.display().to_string()))
    }

    /// Convert an absolute path (as given or canonical) to a vault path.
    pub(crate) fn rel_path(&self, abs: &Path) -> Option<NotePath> {
        let rel = abs
            .strip_prefix(&self.root)
            .or_else(|_| abs.strip_prefix(&self.canonical_root))
            .ok()?;
        let mut parts = Vec::new();
        for c in rel.components() {
            match c {
                Component::Normal(s) => parts.push(s.to_str()?),
                Component::CurDir => {}
                _ => return None,
            }
        }
        NotePath::new(parts.join("/")).ok()
    }

    /// Recursively collect `(files, folders)` (vault-relative, unsorted),
    /// skipping hidden entries, following symlinks only when they stay inside
    /// the vault, and never visiting a directory twice (no symlink loops).
    pub(crate) fn walk(&self) -> Result<(Vec<NotePath>, Vec<String>)> {
        self.walk_from(&self.root, "")
    }

    /// Like [`Vault::walk`] but starting at `start` whose vault path is `prefix`.
    pub(crate) fn walk_from(
        &self,
        start: &Path,
        prefix: &str,
    ) -> Result<(Vec<NotePath>, Vec<String>)> {
        let mut files = Vec::new();
        let mut folders = Vec::new();
        let mut visited = HashSet::new();
        if let Ok(c) = fs::canonicalize(start) {
            visited.insert(c);
        }
        let mut stack = vec![(start.to_path_buf(), prefix.to_string())];
        while let Some((dir, rel)) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                // The root itself must be readable; unreadable subfolders are skipped.
                Err(e) if rel == prefix => return Err(Error::Io(e)),
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let os_name = entry.file_name();
                let Some(name) = os_name.to_str() else {
                    continue; // not representable as a NotePath
                };
                if is_hidden_name(name) {
                    continue;
                }
                let child_rel = if rel.is_empty() {
                    name.to_string()
                } else {
                    format!("{rel}/{name}")
                };
                let path = entry.path();
                let Ok(ftype) = entry.file_type() else {
                    continue;
                };
                let (is_dir, is_file) = if ftype.is_symlink() {
                    let Ok(canon) = fs::canonicalize(&path) else {
                        continue; // dangling
                    };
                    if !canon.starts_with(&self.canonical_root) {
                        continue; // escapes the vault
                    }
                    (canon.is_dir(), canon.is_file())
                } else {
                    (ftype.is_dir(), ftype.is_file())
                };
                if is_dir {
                    let canon = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    if !visited.insert(canon) {
                        continue; // loop or already listed through another link
                    }
                    folders.push(child_rel.clone());
                    stack.push((path, child_rel));
                } else if is_file && let Ok(p) = NotePath::new(&child_rel) {
                    files.push(p);
                }
            }
        }
        Ok((files, folders))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments() {
        assert!(check_segment("a.md").is_ok());
        assert!(check_segment("..").is_err());
        assert!(check_segment(".").is_err());
        assert!(check_segment("").is_err());
        assert!(check_segment("a\0b").is_err());
    }

    #[test]
    fn split() {
        assert_eq!(split_ext("a.md"), ("a", ".md"));
        assert_eq!(split_ext("a.tar.gz"), ("a.tar", ".gz"));
        assert_eq!(split_ext("noext"), ("noext", ""));
        assert_eq!(split_ext(".hidden"), (".hidden", ""));
    }

    #[test]
    fn folders() {
        assert_eq!(normalize_folder("/").unwrap(), "");
        assert_eq!(normalize_folder("a/b/").unwrap(), "a/b");
        assert_eq!(normalize_folder("a\\b").unwrap(), "a/b");
        assert!(normalize_folder("a/../b").is_err());
    }

    #[test]
    fn hidden() {
        assert!(is_hidden_rel(".obsidian/app.json"));
        assert!(is_hidden_rel("a/.git/x"));
        assert!(!is_hidden_rel("a/b.md"));
    }
}
