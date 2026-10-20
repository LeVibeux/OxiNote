//! Obsidian-flavored markdown: parsing to [`ParsedNote`], HTML rendering, link rewriting.
//!
//! # Supported syntax
//!
//! - YAML frontmatter (`---` fences on the first line, LF or CRLF), converted
//!   to JSON; `aliases`/`alias`, `tags`/`tag` and wikilink-valued properties
//!   (`related: "[[Note]]"`).
//! - Wikilinks `[[target#sub|alias]]` (also `\|` inside tables), embeds
//!   `![[...]]`, markdown links/images to internal targets.
//! - Tags `#tag/nested`, block ids `^id`, tasks `- [c] text`, headings,
//!   `==highlights==`, `%%comments%%`, callouts `> [!type]± Title`, math.
//!
//! Nothing is reported inside code spans, code blocks, math, HTML comments or
//! raw HTML tags, or `%%comments%%`.
//!
//! # Conventions other crates rely on
//!
//! - Byte spans index the original source; `line` values are 0-based.
//! - Heading anchors use [`slugify`]; duplicate headings in one note get
//!   `-1`, `-2`, … suffixes in rendered HTML only.
//! - [`ParsedNote::word_count`] counts the body (frontmatter and comments
//!   excluded, code included): words are runs of letters/digits/`_` (with
//!   `'`, `’`, `-` allowed between letters), each CJK character counts as one.
//! - [`ors_core::Task::span`] covers the first line of the list item (from the
//!   list marker to the end of the line); [`ors_core::Task::text`] is the text
//!   after the checkbox, trimmed.
//! - Inline [`ors_core::Tag::span`]s include the `#`; frontmatter tag spans
//!   cover the tag text in the YAML (without `#`).
//! - Frontmatter link spans cover the `[[...]]` text (without YAML quotes).
//!
//! # Properties
//!
//! [`properties`] edits the frontmatter in a source-preserving way (set /
//! remove / rename properties, add / remove / rename tags and aliases).

mod frontmatter;
pub mod properties;
mod render;
mod rewrite;
mod scan;
mod util;

use ors_core::{Link, LinkResolver, NotePath, ParsedNote};

pub use frontmatter::{FrontmatterBlock, parse_yaml, split_frontmatter};
pub use scan::is_valid_tag_name;
pub use util::{
    IMAGE_EXTENSIONS, encode_markdown_target, encode_path, is_external, is_image_target, slugify,
};

/// Extract all metadata (frontmatter, headings, links, embeds, tags, block ids,
/// tasks, word count) from a note's source.
///
/// Never panics; invalid YAML yields `frontmatter: None` (the block is still
/// skipped for body parsing).
pub fn parse(source: &str) -> ParsedNote {
    let a = scan::analyze(source, false);
    let (frontmatter, frontmatter_span, aliases) = match a.fm {
        Some(f) => (
            f.map,
            Some(ors_core::Span::new(f.block.start, f.block.end)),
            f.aliases,
        ),
        None => (None, None, Vec::new()),
    };
    ParsedNote {
        frontmatter,
        frontmatter_span,
        aliases,
        headings: a.headings,
        links: a.links.into_iter().map(|r| r.link).collect(),
        tags: a.tags,
        block_ids: a.block_ids,
        tasks: a.tasks,
        word_count: a.word_count,
    }
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
    /// Default options for `note_path`: no prefix, `.md` kept, frontmatter dropped.
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
/// Internal links become `<a class="internal-link" href=... data-href=...>`;
/// unresolved ones get the extra class `is-unresolved`. Embeds of images become
/// `<img>` (`![[a.png|300]]` / `|300x200` set the size), embeds of notes become
/// `<div class="internal-embed" data-src=...>` placeholders (no recursive
/// transclusion here). Callouts become
/// `<div class="callout" data-callout="note">` (plus `data-callout-fold="+"/"-"`
/// for foldable ones) containing `<div class="callout-title">` and
/// `<div class="callout-content">`. Tags become `<a class="tag" href="#tag">`,
/// highlights `<mark>`, block ids `<span class="block-id" id="^id">`, tasks
/// `<input type="checkbox" class="task-list-item-checkbox" data-task="x">`,
/// math `<span class="math">`/`<div class="math">` (escaped TeX). Headings get
/// `id` = [`slugify`] of their text. External links get
/// `class="external-link"`. Raw HTML is sanitised (allow-listed tags without
/// event handlers or `javascript:` URLs; others are escaped).
pub fn render_html(source: &str, options: &RenderOptions, resolver: &dyn LinkResolver) -> String {
    render::render(source, options, resolver)
}

/// Rewrite every link in `source`. For each link, `f` returns `Some(new_target)`
/// to replace the target path (subpath, alias, embed marker and link style are
/// preserved), or `None` to leave the link untouched. Used to update links when
/// a note is renamed.
///
/// The `.md` extension follows the original: if the original target ended in
/// `.md` and `new_target` does not, `.md` is appended; if the original did not
/// and `new_target` does, it is removed. Markdown link targets are
/// percent-encoded ([`encode_markdown_target`]). Text outside the replaced
/// targets is preserved byte for byte.
pub fn rewrite_links(source: &str, f: impl FnMut(&Link) -> Option<String>) -> String {
    rewrite::rewrite(source, f)
}
