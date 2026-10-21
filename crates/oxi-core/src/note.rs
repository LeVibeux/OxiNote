use serde::{Deserialize, Serialize};

/// Byte range in the note source (`start..end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// YAML frontmatter converted to JSON (always an object when present).
pub type Frontmatter = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    /// `[[target#sub|alias]]`
    Wiki,
    /// `[alias](target#sub)` — only internal links (no URL scheme) are reported.
    Markdown,
    /// A wikilink-style reference written as a frontmatter property value,
    /// e.g. `related: "[[Other]]"`. Span covers the value in the source.
    Frontmatter,
}

/// Target sub-location of a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subpath {
    /// `#Heading` or nested `#H1#H2` (outermost first).
    Heading(Vec<String>),
    /// `#^block-id`
    Block(String),
}

/// An internal link or embed found in a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// Path part as written (percent-decoded for markdown links), without
    /// subpath or alias. Empty for same-note links like `[[#Heading]]`.
    pub target: String,
    pub subpath: Option<Subpath>,
    /// Alias / link text (`[[a|alias]]`, `[alias](a.md)`).
    pub display: Option<String>,
    pub kind: LinkKind,
    /// `![[...]]` or `![](...)`.
    pub is_embed: bool,
    /// Whole link including brackets and the `!` of embeds.
    pub span: Span,
    /// 0-based line number of `span.start`.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heading {
    /// 1..=6
    pub level: u8,
    /// Heading text, inline markup stripped.
    pub text: String,
    pub span: Span,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// Without the leading `#`, case preserved, nested with `/`: `"project/rust"`.
    pub name: String,
    /// For frontmatter tags this covers the value in the YAML.
    pub span: Span,
    pub line: usize,
    pub from_frontmatter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockId {
    /// Without the `^`.
    pub id: String,
    pub span: Span,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Character inside the brackets: `' '` (todo), `'x'` (done), or custom (`'/'`, `'-'`, ...).
    pub status: char,
    pub text: String,
    pub span: Span,
    pub line: usize,
}

impl Task {
    pub fn is_done(&self) -> bool {
        matches!(self.status, 'x' | 'X')
    }
}

/// Everything the metadata cache needs to know about one markdown note.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParsedNote {
    pub frontmatter: Option<Frontmatter>,
    /// Span of the whole frontmatter block including both `---` fences.
    pub frontmatter_span: Option<Span>,
    /// From frontmatter `aliases` / `alias` (string or list).
    pub aliases: Vec<String>,
    pub headings: Vec<Heading>,
    /// Links and embeds, in source order (frontmatter links first).
    pub links: Vec<Link>,
    /// Inline and frontmatter tags, in source order (frontmatter first).
    pub tags: Vec<Tag>,
    pub block_ids: Vec<BlockId>,
    pub tasks: Vec<Task>,
    /// Number of words in the body (excluding frontmatter and comments).
    pub word_count: usize,
}

impl ParsedNote {
    /// Links that are not embeds.
    pub fn plain_links(&self) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(|l| !l.is_embed)
    }

    pub fn embeds(&self) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(|l| l.is_embed)
    }
}
