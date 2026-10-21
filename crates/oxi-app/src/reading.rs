//! Reading view model: turns a note into a small block tree that the UI
//! draws with egui widgets.
//!
//! Obsidian syntax is located with the spans from `oxi_markdown::parse`
//! (`ParsedNote`): wikilinks, embeds and tags are rewritten to standard
//! markdown links with an `ors:N` destination pointing into
//! [`Prepared::refs`], then the result goes through `pulldown-cmark`.
//! Comments (`%%…%%`) are removed and `==highlights==` styled lexically.

use oxi_core::{LinkKind, LinkResolver, NotePath, ParsedNote, Subpath};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::highlight::highlight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRef {
    /// Internal link (wikilink or markdown) to a note/attachment.
    Internal {
        target: String,
        subpath: Option<Subpath>,
        resolved: Option<NotePath>,
    },
    /// `![[embed]]`
    Embed {
        target: String,
        subpath: Option<Subpath>,
        resolved: Option<NotePath>,
        /// Text after `|` (alt text or image size like `300x200`).
        display: Option<String>,
    },
    Tag(String),
}

#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub markdown: String,
    pub refs: Vec<LinkRef>,
    pub frontmatter: Vec<(String, String)>,
    /// Source line of every heading / task, in document order (the n-th
    /// heading or task block drawn is the n-th entry).
    pub heading_lines: Vec<usize>,
    pub task_lines: Vec<usize>,
}

fn escape_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '#' | '=' | '~' | '%'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn subpath_text(s: &Subpath) -> String {
    match s {
        Subpath::Heading(h) => h.join(" > "),
        Subpath::Block(b) => format!("^{b}"),
    }
}

/// Rewrite the Obsidian-specific syntax of `source` using the spans in `parsed`.
pub fn prepare(
    source: &str,
    parsed: &ParsedNote,
    note: &NotePath,
    resolver: &dyn LinkResolver,
) -> Prepared {
    let mut prepared = Prepared {
        heading_lines: parsed.headings.iter().map(|h| h.line).collect(),
        task_lines: parsed.tasks.iter().map(|t| t.line).collect(),
        ..Prepared::default()
    };
    if let Some(fm) = &parsed.frontmatter {
        for (k, v) in fm {
            let value = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(a) => a
                    .iter()
                    .map(|x| match x {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            prepared.frontmatter.push((k.clone(), value));
        }
    }

    // (start, end, replacement)
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    if let Some(fm) = parsed.frontmatter_span {
        edits.push((fm.start, fm.end.min(source.len()), String::new()));
    } else if let Some(rest) = source.strip_prefix("---\n") {
        // Fallback: hide an unparsed frontmatter block.
        if let Some(end) = rest.find("\n---") {
            let stop = (4 + end + 4).min(source.len());
            edits.push((0, stop, String::new()));
        }
    }
    // Comments.
    for (r, style) in highlight(source) {
        if style.comment {
            edits.push((r.start, r.end, String::new()));
        }
    }
    for link in &parsed.links {
        if link.kind == LinkKind::Frontmatter {
            continue;
        }
        let resolved = if link.target.is_empty() {
            Some(note.clone())
        } else {
            resolver.resolve(&link.target, note)
        };
        let id = prepared.refs.len();
        let text = match (&link.display, &link.subpath) {
            (Some(d), _) => d.clone(),
            (None, Some(s)) if link.target.is_empty() => subpath_text(s),
            (None, Some(s)) => format!("{} > {}", link.target, subpath_text(s)),
            (None, None) => link.target.clone(),
        };
        let replacement = if link.is_embed {
            prepared.refs.push(LinkRef::Embed {
                target: link.target.clone(),
                subpath: link.subpath.clone(),
                resolved,
                display: link.display.clone(),
            });
            format!("![{}](ors:{id})", escape_md(&text))
        } else {
            prepared.refs.push(LinkRef::Internal {
                target: link.target.clone(),
                subpath: link.subpath.clone(),
                resolved,
            });
            format!("[{}](ors:{id})", escape_md(&text))
        };
        edits.push((link.span.start, link.span.end, replacement));
    }
    // Block ids are hidden like in Obsidian's reading view.
    for b in &parsed.block_ids {
        edits.push((b.span.start, b.span.end, String::new()));
    }
    // Custom task statuses (`[/]`, `[-]`...) are drawn as checked boxes.
    for t in &parsed.tasks {
        if matches!(t.status, ' ' | 'x' | 'X') {
            continue;
        }
        let ls = source[..t.span.start.min(source.len())]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        let le = source[ls..].find('\n').map_or(source.len(), |i| ls + i);
        let pat = format!("[{}]", t.status);
        if let Some(i) = source[ls..le].find(&pat) {
            let at = ls + i + 1;
            edits.push((at, at + t.status.len_utf8(), "x".into()));
        }
    }
    for tag in &parsed.tags {
        if tag.from_frontmatter {
            continue;
        }
        let id = prepared.refs.len();
        prepared.refs.push(LinkRef::Tag(tag.name.clone()));
        edits.push((
            tag.span.start,
            tag.span.end,
            format!("[\\#{}](ors:{id})", escape_md(&tag.name)),
        ));
    }
    edits.sort_by_key(|e| (e.0, std::cmp::Reverse(e.1)));

    let mut out = String::with_capacity(source.len());
    let mut pos = 0;
    for (start, end, rep) in edits {
        if start < pos
            || end > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            continue; // overlapping (e.g. link inside a comment) or invalid
        }
        out.push_str(&source[pos..start]);
        out.push_str(&rep);
        pos = end;
    }
    out.push_str(&source[pos..]);
    prepared.markdown = out;
    prepared
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub highlight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineKind {
    Text,
    /// Link destination (`ors:N` for internal ones).
    Link(String),
    Image {
        url: String,
    },
    LineBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub style: InlineStyle,
    pub kind: InlineKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Code {
        lang: String,
        text: String,
    },
    Quote(Vec<Block>),
    Callout {
        kind: String,
        title: String,
        /// `[!note]-` (`Some(true)`, folded by default) or `[!note]+`
        /// (`Some(false)`, open but foldable); `None`: not foldable.
        fold: Option<bool>,
        blocks: Vec<Block>,
    },
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
}

/// Parse prepared markdown into blocks.
pub fn parse_blocks(markdown: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events: Vec<Event<'_>> = Parser::new_ext(markdown, opts).collect();
    let mut pos = 0;
    blocks_until(&events, &mut pos, None)
}

/// Is `e` the end event closing a block started by `start`?
fn closes(e: &Event<'_>, end: Option<TagEnd>) -> bool {
    matches!((e, end), (Event::End(t), Some(want)) if *t == want)
}

fn blocks_until(events: &[Event<'_>], pos: &mut usize, end: Option<TagEnd>) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut loose: Vec<Inline> = Vec::new();
    while *pos < events.len() {
        let e = &events[*pos];
        if closes(e, end) {
            *pos += 1;
            break;
        }
        match e {
            Event::Start(Tag::Paragraph) => {
                flush_loose(&mut loose, &mut blocks);
                *pos += 1;
                let inl = inlines_until(events, pos, TagEnd::Paragraph);
                blocks.push(Block::Paragraph(inl));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush_loose(&mut loose, &mut blocks);
                let lvl = *level as u8;
                let tag_end = TagEnd::Heading(*level);
                *pos += 1;
                let inlines = inlines_until(events, pos, tag_end);
                blocks.push(Block::Heading {
                    level: lvl,
                    inlines,
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_loose(&mut loose, &mut blocks);
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                *pos += 1;
                let mut text = String::new();
                while *pos < events.len() {
                    match &events[*pos] {
                        Event::End(TagEnd::CodeBlock) => {
                            *pos += 1;
                            break;
                        }
                        Event::Text(t) => text.push_str(t),
                        _ => {}
                    }
                    *pos += 1;
                }
                if text.ends_with('\n') {
                    text.pop();
                }
                blocks.push(Block::Code { lang, text });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush_loose(&mut loose, &mut blocks);
                *pos += 1;
                let inner = blocks_until(events, pos, Some(TagEnd::BlockQuote(None)));
                blocks.push(to_callout(inner));
            }
            Event::Start(Tag::List(start)) => {
                flush_loose(&mut loose, &mut blocks);
                let start = *start;
                let ordered = start.is_some();
                *pos += 1;
                let mut items = Vec::new();
                while *pos < events.len() {
                    match &events[*pos] {
                        Event::End(TagEnd::List(o)) if *o == ordered => {
                            *pos += 1;
                            break;
                        }
                        Event::Start(Tag::Item) => {
                            *pos += 1;
                            let mut task = None;
                            if let Some(Event::TaskListMarker(done)) = events.get(*pos) {
                                task = Some(*done);
                                *pos += 1;
                            }
                            let blocks = blocks_until(events, pos, Some(TagEnd::Item));
                            items.push(Item { task, blocks });
                        }
                        _ => *pos += 1,
                    }
                }
                blocks.push(Block::List { start, items });
            }
            Event::Start(Tag::Table(_)) => {
                flush_loose(&mut loose, &mut blocks);
                *pos += 1;
                blocks.push(table(events, pos));
            }
            Event::Rule => {
                flush_loose(&mut loose, &mut blocks);
                blocks.push(Block::Rule);
                *pos += 1;
            }
            Event::Start(Tag::HtmlBlock) | Event::Start(Tag::MetadataBlock(_)) => {
                skip_tag(events, pos);
            }
            Event::Start(_)
            | Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::InlineHtml(_)
            | Event::Html(_) => {
                // Inline content directly inside a tight list item.
                let mut style = Vec::new();
                inline_event(events, pos, &mut loose, &mut style);
            }
            _ => *pos += 1,
        }
    }
    flush_loose(&mut loose, &mut blocks);
    blocks
}

fn flush_loose(loose: &mut Vec<Inline>, blocks: &mut Vec<Block>) {
    if !loose.is_empty() {
        blocks.push(Block::Paragraph(apply_highlights(merge_text(
            std::mem::take(loose),
        ))));
    }
}

fn skip_tag(events: &[Event<'_>], pos: &mut usize) {
    let mut depth = 0usize;
    while *pos < events.len() {
        match &events[*pos] {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    *pos += 1;
                    return;
                }
            }
            _ => {}
        }
        *pos += 1;
    }
}

fn table(events: &[Event<'_>], pos: &mut usize) -> Block {
    let mut header = Vec::new();
    let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
    let mut in_head = false;
    while *pos < events.len() {
        match &events[*pos] {
            Event::End(TagEnd::Table) => {
                *pos += 1;
                break;
            }
            Event::Start(Tag::TableHead) => {
                in_head = true;
                *pos += 1;
            }
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                *pos += 1;
            }
            Event::Start(Tag::TableRow) => {
                rows.push(Vec::new());
                *pos += 1;
            }
            Event::Start(Tag::TableCell) => {
                *pos += 1;
                let cell = inlines_until(events, pos, TagEnd::TableCell);
                if in_head {
                    header.push(cell);
                } else if let Some(r) = rows.last_mut() {
                    r.push(cell);
                }
            }
            _ => *pos += 1,
        }
    }
    Block::Table { header, rows }
}

#[derive(Clone, Copy)]
enum StyleOp {
    Bold,
    Italic,
    Strike,
}

fn current_style(stack: &[StyleOp], highlight: bool) -> InlineStyle {
    let mut s = InlineStyle {
        highlight,
        ..InlineStyle::default()
    };
    for op in stack {
        match op {
            StyleOp::Bold => s.bold = true,
            StyleOp::Italic => s.italic = true,
            StyleOp::Strike => s.strike = true,
        }
    }
    s
}

fn inlines_until(events: &[Event<'_>], pos: &mut usize, end: TagEnd) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    while *pos < events.len() {
        if matches!(&events[*pos], Event::End(t) if *t == end) {
            *pos += 1;
            break;
        }
        inline_event(events, pos, &mut out, &mut stack);
    }
    apply_highlights(merge_text(out))
}

/// Consume one inline event (or a whole link/image) at `pos`.
fn inline_event(
    events: &[Event<'_>],
    pos: &mut usize,
    out: &mut Vec<Inline>,
    stack: &mut Vec<StyleOp>,
) {
    let style = current_style(stack, false);
    match &events[*pos] {
        Event::Text(t) => out.push(Inline {
            text: t.to_string(),
            style,
            kind: InlineKind::Text,
        }),
        Event::Code(t) => out.push(Inline {
            text: t.to_string(),
            style: InlineStyle {
                code: true,
                ..style
            },
            kind: InlineKind::Text,
        }),
        Event::InlineHtml(t) | Event::Html(t) => out.push(Inline {
            text: t.to_string(),
            style,
            kind: InlineKind::Text,
        }),
        // Kept as '\n' so callout titles can be split; drawn as a space.
        Event::SoftBreak => out.push(Inline {
            text: "\n".into(),
            style,
            kind: InlineKind::Text,
        }),
        Event::HardBreak => out.push(Inline {
            text: String::new(),
            style,
            kind: InlineKind::LineBreak,
        }),
        Event::Start(Tag::Strong) => stack.push(StyleOp::Bold),
        Event::Start(Tag::Emphasis) => stack.push(StyleOp::Italic),
        Event::Start(Tag::Strikethrough) => stack.push(StyleOp::Strike),
        Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => {
            stack.pop();
        }
        Event::Start(Tag::Link { dest_url, .. }) => {
            let url = dest_url.to_string();
            *pos += 1;
            let inner = inlines_until(events, pos, TagEnd::Link);
            let text: String = inner.iter().map(|i| i.text.as_str()).collect();
            let s = inner.first().map_or(style, |i| i.style);
            out.push(Inline {
                text,
                style: s,
                kind: InlineKind::Link(url),
            });
            return;
        }
        Event::Start(Tag::Image { dest_url, .. }) => {
            let url = dest_url.to_string();
            *pos += 1;
            let inner = inlines_until(events, pos, TagEnd::Image);
            out.push(Inline {
                text: inner.iter().map(|i| i.text.as_str()).collect(),
                style,
                kind: InlineKind::Image { url },
            });
            return;
        }
        Event::TaskListMarker(done) => out.push(Inline {
            text: if *done { "☑ " } else { "☐ " }.into(),
            style,
            kind: InlineKind::Text,
        }),
        _ => {}
    }
    *pos += 1;
}

fn merge_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for i in inlines {
        if let Some(last) = out.last_mut()
            && last.kind == InlineKind::Text
            && i.kind == InlineKind::Text
            && last.style == i.style
        {
            last.text.push_str(&i.text);
            continue;
        }
        out.push(i);
    }
    out
}

/// Split text runs on `==` and toggle the highlight style.
fn apply_highlights(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut on = false;
    let mut out = Vec::new();
    for i in inlines {
        if i.kind != InlineKind::Text || i.style.code {
            let mut i = i;
            i.style.highlight |= on;
            out.push(i);
            continue;
        }
        for (n, part) in i.text.split("==").enumerate() {
            if n > 0 {
                on = !on;
            }
            if !part.is_empty() {
                out.push(Inline {
                    text: part.to_string(),
                    style: InlineStyle {
                        highlight: on,
                        ..i.style
                    },
                    kind: InlineKind::Text,
                });
            }
        }
    }
    out
}

/// `> [!type] Title` blockquotes become callouts.
fn to_callout(blocks: Vec<Block>) -> Block {
    let Some(Block::Paragraph(first)) = blocks.first() else {
        return Block::Quote(blocks);
    };
    let Some(Inline {
        text,
        kind: InlineKind::Text,
        ..
    }) = first.first()
    else {
        return Block::Quote(blocks);
    };
    let Some(rest) = text.strip_prefix("[!") else {
        return Block::Quote(blocks);
    };
    let Some(close) = rest.find(']') else {
        return Block::Quote(blocks);
    };
    let kind = rest[..close].to_lowercase();
    let fold = match rest[close + 1..].chars().next() {
        Some('-') => Some(true),
        Some('+') => Some(false),
        _ => None,
    };
    let after = rest[close + 1..].trim_start_matches(['-', '+']);
    // Title runs until the first line break (soft breaks became " ", so
    // only the first text run is used).
    let (title, remainder) = match after.find('\n') {
        Some(n) => (after[..n].trim().to_string(), after[n + 1..].to_string()),
        None => (after.trim().to_string(), String::new()),
    };
    let mut first_rest: Vec<Inline> = Vec::new();
    if !remainder.is_empty() {
        first_rest.push(Inline {
            text: remainder,
            style: InlineStyle::default(),
            kind: InlineKind::Text,
        });
    }
    let mut blocks = blocks;
    let mut para = match blocks.remove(0) {
        Block::Paragraph(p) => p,
        _ => Vec::new(),
    };
    para.remove(0);
    first_rest.extend(para);
    let title = if title.is_empty() {
        let mut t = kind.clone();
        if let Some(c) = t.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        t
    } else {
        title
    };
    if !first_rest.is_empty() {
        blocks.insert(0, Block::Paragraph(first_rest));
    }
    Block::Callout {
        kind,
        title,
        fold,
        blocks,
    }
}

/// Deepest level of nested `![[Note]]` transclusions that is drawn.
pub const MAX_EMBED_DEPTH: usize = 3;

/// What to do with an embed of `target` met while drawing the notes in
/// `chain` (outermost first, the last one is the note being drawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedPlan {
    Render,
    /// `target` is already being drawn: embedding it would loop forever.
    Cycle,
    TooDeep,
}

/// One level of the transclusion chain: the note and the part of it drawn
/// (`None`: the whole note).
pub type EmbedKey = (NotePath, Option<Subpath>);

pub fn embed_plan(chain: &[EmbedKey], target: &NotePath, sub: Option<&Subpath>) -> EmbedPlan {
    if chain.iter().any(|(p, s)| p == target && s.as_ref() == sub) {
        EmbedPlan::Cycle
    } else if chain.len() > MAX_EMBED_DEPTH {
        EmbedPlan::TooDeep
    } else {
        EmbedPlan::Render
    }
}

/// Callout type with Obsidian's aliases resolved (`warning`, `error`...).
pub fn callout_family(kind: &str) -> &'static str {
    match kind {
        "note" => "note",
        "abstract" | "summary" | "tldr" => "abstract",
        "info" => "info",
        "todo" => "todo",
        "tip" | "hint" | "important" => "tip",
        "success" | "check" | "done" => "success",
        "question" | "help" | "faq" => "question",
        "warning" | "caution" | "attention" => "warning",
        "failure" | "fail" | "missing" => "failure",
        "danger" | "error" => "danger",
        "bug" => "bug",
        "example" => "example",
        "quote" | "cite" => "quote",
        _ => "note",
    }
}

/// Icon of a callout type.
pub fn callout_icon(kind: &str) -> &'static str {
    match callout_family(kind) {
        "abstract" => "≡",
        "info" => "ⓘ",
        "todo" => "☑",
        "tip" => "★",
        "success" => "✔",
        "question" => "?",
        "warning" => "⚠",
        "failure" => "✖",
        "danger" => "⚡",
        "bug" => "✱",
        "example" => "☰",
        "quote" => "❝",
        _ => "✎",
    }
}

/// Parse `ors:N` destinations.
pub fn ref_index(url: &str) -> Option<usize> {
    url.strip_prefix("ors:").and_then(|n| n.parse().ok())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::naive_parse;
    use oxi_core::NoResolver;

    #[test]
    fn prepare_rewrites_links_tags_and_comments() {
        let src = "---\na: 1\n---\nSee [[Note|alias]] and #tag %%secret [[x]]%% ![[img.png]]";
        let parsed = naive_parse(src);
        let p = prepare(src, &parsed, &NotePath::new("a.md").unwrap(), &NoResolver);
        assert!(!p.markdown.contains("secret"));
        assert!(!p.markdown.contains("a: 1"));
        assert!(p.markdown.contains("[alias](ors:0)"), "{}", p.markdown);
        // The link inside the comment is dropped (its ref is unused).
        assert!(!p.markdown.contains("(ors:1)"), "{}", p.markdown);
        assert!(p.markdown.contains("![img.png](ors:2)"));
        assert!(p.markdown.contains("[\\#tag](ors:3)"), "{}", p.markdown);
        assert!(matches!(&p.refs[3], LinkRef::Tag(t) if t == "tag"));
        assert!(matches!(&p.refs[2], LinkRef::Embed { target, .. } if target == "img.png"));
    }

    #[test]
    fn blocks_from_markdown() {
        let md = "# Title\n\nSome **bold** and ==hi== text [l](ors:0)\n\n- [ ] task\n- [x] done\n\n```rust\nfn x() {}\n```\n\n> [!tip] Astuce\n> Body\n\n> plain\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let blocks = parse_blocks(md);
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }));
        let Block::Paragraph(p) = &blocks[1] else {
            panic!("{:?}", blocks[1])
        };
        assert!(p.iter().any(|i| i.style.bold && i.text == "bold"));
        assert!(p.iter().any(|i| i.style.highlight && i.text == "hi"));
        assert!(p.iter().any(|i| i.kind == InlineKind::Link("ors:0".into())));
        let Block::List { items, .. } = &blocks[2] else {
            panic!("{:?}", blocks[2])
        };
        assert_eq!(items[0].task, Some(false));
        assert_eq!(items[1].task, Some(true));
        assert!(matches!(&blocks[3], Block::Code { lang, .. } if lang == "rust"));
        let Block::Callout {
            kind,
            title,
            blocks: inner,
            ..
        } = &blocks[4]
        else {
            panic!("{:?}", blocks[4])
        };
        assert_eq!(kind, "tip");
        assert_eq!(title, "Astuce");
        assert!(!inner.is_empty());
        assert!(matches!(&blocks[5], Block::Quote(_)));
        assert!(matches!(&blocks[6], Block::Rule));
        assert!(
            matches!(&blocks[7], Block::Table { header, rows } if header.len() == 2 && rows.len() == 1)
        );
        assert_eq!(ref_index("ors:12"), Some(12));
        assert_eq!(ref_index("https://x"), None);
    }

    #[test]
    fn embed_plan_guards_cycles_and_depth() {
        let p = |n: &str| NotePath::new(n).unwrap();
        let blk = Subpath::Block("x".into());
        let whole = |n: &str| (p(n), None);
        assert_eq!(
            embed_plan(&[whole("a.md")], &p("b.md"), None),
            EmbedPlan::Render
        );
        assert_eq!(
            embed_plan(&[whole("a.md")], &p("a.md"), None),
            EmbedPlan::Cycle
        );
        assert_eq!(
            embed_plan(&[whole("a.md"), whole("b.md")], &p("a.md"), None),
            EmbedPlan::Cycle
        );
        // A block of the note being drawn is fine, but not twice.
        assert_eq!(
            embed_plan(&[whole("a.md")], &p("a.md"), Some(&blk)),
            EmbedPlan::Render
        );
        let chain = [whole("a.md"), (p("a.md"), Some(blk.clone()))];
        assert_eq!(embed_plan(&chain, &p("a.md"), Some(&blk)), EmbedPlan::Cycle);
        let deep = [whole("a.md"), whole("b.md"), whole("c.md"), whole("d.md")];
        assert_eq!(embed_plan(&deep[..3], &p("e.md"), None), EmbedPlan::Render);
        assert_eq!(embed_plan(&deep, &p("e.md"), None), EmbedPlan::TooDeep);
    }

    #[test]
    fn callout_fold_markers_and_families() {
        let b = parse_blocks(
            "> [!warning]- Careful
> body

> [!note]+ Open
> x

> [!tip] Plain
> y
",
        );
        let folds: Vec<Option<bool>> = b
            .iter()
            .filter_map(|b| match b {
                Block::Callout { fold, .. } => Some(*fold),
                _ => None,
            })
            .collect();
        assert_eq!(folds, vec![Some(true), Some(false), None]);
        assert_eq!(callout_family("error"), "danger");
        assert_eq!(callout_family("unknown"), "note");
        assert_ne!(callout_icon("tip"), callout_icon("bug"));
    }

    #[test]
    fn prepare_lists_heading_and_task_lines() {
        let src = "# T

- [ ] a
- [x] b

## U
";
        let p = prepare(
            src,
            &oxi_markdown::parse(src),
            &NotePath::new("a.md").unwrap(),
            &NoResolver,
        );
        assert_eq!(p.heading_lines, vec![0, 5]);
        assert_eq!(p.task_lines, vec![2, 3]);
    }
}
