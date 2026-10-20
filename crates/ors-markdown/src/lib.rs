//! Obsidian-flavored markdown: parsing to [`ParsedNote`], HTML rendering, link rewriting.
//!
//! STUB — public signatures are the contract; the bodies are placeholders.

use ors_core::{Link, LinkResolver, NotePath, ParsedNote};

/// Extract all metadata (frontmatter, headings, links, embeds, tags, block ids,
/// tasks, word count) from a note's source.
pub fn parse(source: &str) -> ParsedNote {
    let _ = source;
    ParsedNote::default()
}

/// Options for [`render_html`].
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Path of the note being rendered (used to resolve relative links).
    pub note_path: NotePath,
    /// Prefix prepended to resolved vault paths in `href`/`src` attributes
    /// (e.g. `"/"` or `"../"`). Paths are percent-encoded.
    pub link_prefix: String,
    /// Replace the `.md` extension of resolved note links with this one
    /// (e.g. `Some("html")` for a static site export); `None` keeps `.md`.
    pub note_extension: Option<String>,
    /// Render the frontmatter as a properties table instead of dropping it.
    pub show_frontmatter: bool,
}

impl RenderOptions {
    pub fn new(note_path: NotePath) -> Self {
        Self {
            note_path,
            link_prefix: String::new(),
            note_extension: None,
            show_frontmatter: false,
        }
    }
}

/// Render a note to an HTML fragment (no `<html>`/`<body>`).
///
/// Internal links become `<a class="internal-link" href=...>`; unresolved ones get
/// the extra class `is-unresolved`. Embeds of images become `<img>`, embeds of
/// notes become `<div class="internal-embed" data-src=...>` placeholders (no
/// recursive transclusion here). Callouts become
/// `<div class="callout" data-callout="note">`, tags `<a class="tag" href="#tag">`.
pub fn render_html(source: &str, options: &RenderOptions, resolver: &dyn LinkResolver) -> String {
    let _ = (source, options, resolver);
    String::new()
}

/// Rewrite every link in `source`. For each link, `f` returns `Some(new_target)`
/// to replace the target path (subpath, alias, embed marker and link style are
/// preserved), or `None` to leave the link untouched. Used to update links when
/// a note is renamed.
pub fn rewrite_links(source: &str, f: impl FnMut(&Link) -> Option<String>) -> String {
    let _ = f;
    source.to_string()
}
