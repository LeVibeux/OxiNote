//! Metadata cache (link resolution, backlinks, tags), full-text search and
//! vault-wide refactoring.
//!
//! - [`MetadataCache`] indexes every file of a vault: parsed notes
//!   ([`ParsedNote`](ors_core::ParsedNote) from `ors-markdown`) and attachments. It resolves links
//!   with Obsidian's rules ([`MetadataCache::resolve_target`]) and keeps
//!   backlinks, unresolved links and tags up to date under incremental
//!   updates ([`MetadataCache::update_file`], [`MetadataCache::insert_parsed`],
//!   [`MetadataCache::remove_file`], [`MetadataCache::rename_file`]).
//! - [`SearchIndex`] implements Obsidian's search syntax over raw note
//!   sources (see [`search::query`] for the grammar).
//! - [`fuzzy_find`] powers the quick switcher.
//! - [`rename_with_links`] / [`rename_folder_with_links`] move files and
//!   rewrite the links pointing to them.
//! - Properties and tags: [`MetadataCache::properties`],
//!   [`MetadataCache::property_values`], [`MetadataCache::tag_suggestions`],
//!   [`MetadataCache::tag_tree`]; note edits that keep the cache in sync
//!   ([`set_property_in`], [`rename_tag`], [`rename_property_everywhere`], ...).

mod cache;
mod fuzzy;
mod properties;
mod refactor;
mod resolve;
pub mod search;
mod text;

use ors_core::{Link, NotePath};

pub use cache::{CacheStats, MetadataCache};
pub use fuzzy::{FuzzyOptions, fuzzy_find, fuzzy_find_with, fuzzy_score};
pub use properties::{
    PropertyInfo, TagNode, add_tag_in, edit_note, plan_tag_rename, remove_property_in,
    remove_tag_in, rename_property_everywhere, rename_tag, set_property_in, set_property_in_with,
};
pub use refactor::{
    LinkEdit, RenamePlan, plan_rename, rename_folder_with_links, rename_with_links,
};
pub use search::{Operator, Query, QueryError, SearchIndex, parse_query};

/// A link from `source` pointing at the queried note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    /// Note containing the link.
    pub source: NotePath,
    /// The link itself (span/line locate it in `source`).
    pub link: Link,
}

/// An outgoing link with its resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLink {
    /// The link as parsed.
    pub link: Link,
    /// `None` if the target does not exist.
    pub target: Option<NotePath>,
}

/// Options for [`MetadataCache::graph`].
#[derive(Debug, Clone, Default)]
pub struct GraphOptions {
    /// Add `"#tag"` nodes with note → tag edges.
    pub include_tags: bool,
    /// Add attachment nodes (non-markdown files).
    pub include_attachments: bool,
    /// Add nodes for link targets that do not exist.
    pub include_unresolved: bool,
    /// Keep nodes without any edge.
    pub include_orphans: bool,
}

/// One match inside a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// 0-based line number.
    pub line: usize,
    /// Text of the line (trimmed to a reasonable length).
    pub line_text: String,
    /// Byte ranges of the matches inside `line_text`.
    pub ranges: Vec<(usize, usize)>,
}

/// One file matching a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Matching note.
    pub path: NotePath,
    /// Higher is better.
    pub score: u32,
    /// Empty when the file matched only on name/path/tag operators.
    pub matches: Vec<SearchMatch>,
}

#[cfg(test)]
mod tests;
