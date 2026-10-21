//! Shared types and traits for Oxinote.
//!
//! This crate is the contract between all other crates: it holds the data model
//! produced by `oxi-markdown`, consumed by `oxi-index`, and displayed by the
//! CLI and the GUI. Keep it dependency-light and stable.

mod error;
mod graph;
mod memory;
mod note;
mod path;

pub use error::{Error, Result};
pub use graph::{GraphData, GraphEdge, GraphNode, GraphNodeKind};
pub use memory::MemoryVault;
pub use note::{
    BlockId, Frontmatter, Heading, Link, LinkKind, ParsedNote, Span, Subpath, Tag, Task,
};
pub use path::NotePath;

use std::time::SystemTime;

/// File metadata exposed by a vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    pub size: u64,
    pub modified: SystemTime,
    pub created: SystemTime,
}

/// Change notification emitted by a vault watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultEvent {
    Created(NotePath),
    Modified(NotePath),
    Deleted(NotePath),
    Renamed { from: NotePath, to: NotePath },
}

/// Read access to a vault. Implemented by `oxi_vault::Vault` (disk) and
/// [`MemoryVault`] (tests).
pub trait VaultRead {
    /// All files of the vault (markdown and attachments), excluding the config
    /// folder (`.obsidian`), the local trash (`.trash`) and hidden entries.
    /// Sorted by path.
    fn list_files(&self) -> Result<Vec<NotePath>>;
    fn read_to_string(&self, path: &NotePath) -> Result<String>;
    fn read_bytes(&self, path: &NotePath) -> Result<Vec<u8>>;
    fn metadata(&self, path: &NotePath) -> Result<FileMeta>;
    fn exists(&self, path: &NotePath) -> bool;

    /// Markdown notes only.
    fn list_notes(&self) -> Result<Vec<NotePath>> {
        Ok(self
            .list_files()?
            .into_iter()
            .filter(NotePath::is_markdown)
            .collect())
    }
}

/// Write access to a vault.
pub trait VaultWrite: VaultRead {
    /// Write (create or overwrite) a file, creating parent folders.
    fn write(&self, path: &NotePath, contents: &str) -> Result<()>;
    /// Rename/move a file. Fails with [`Error::AlreadyExists`] if `to` exists.
    fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()>;
    /// Remove a file (implementation decides between trash and permanent delete).
    fn delete(&self, path: &NotePath) -> Result<()>;
}

/// Resolves a link target (as written in a note) to a vault file.
/// Implemented by `oxi_index::MetadataCache`.
pub trait LinkResolver {
    /// `target` is the path part of a link (no `#subpath`, no alias), e.g.
    /// `"Note"`, `"folder/Note"`, `"image.png"`. `from` is the note containing the link.
    fn resolve(&self, target: &str, from: &NotePath) -> Option<NotePath>;
}

/// A resolver that never resolves anything.
pub struct NoResolver;

impl LinkResolver for NoResolver {
    fn resolve(&self, _target: &str, _from: &NotePath) -> Option<NotePath> {
        None
    }
}
