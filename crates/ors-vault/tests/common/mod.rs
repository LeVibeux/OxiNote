#![allow(dead_code, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use ors_core::NotePath;
use ors_vault::Vault;
use tempfile::TempDir;

pub fn sample_vault_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-vault")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// A temp copy of `examples/sample-vault`, opened.
pub fn sample() -> (TempDir, Vault) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sample-vault");
    copy_dir(&sample_vault_dir(), &root);
    let vault = Vault::open(&root).unwrap();
    (tmp, vault)
}

/// An empty vault in a temp dir.
pub fn empty() -> (TempDir, Vault) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    fs::create_dir(&root).unwrap();
    let vault = Vault::open(&root).unwrap();
    (tmp, vault)
}

pub fn p(s: &str) -> NotePath {
    NotePath::new(s).unwrap()
}
