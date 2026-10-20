//! Reading view model: turns a note into a small block tree that the UI
//! draws with egui widgets.
//!
//! Obsidian syntax is located with the spans from `ors_markdown::parse`
//! (`ParsedNote`): wikilinks, embeds and tags are rewritten to standard
//! markdown links with an `ors:N` destination pointing into
//! [`Prepared::refs`], then the result goes through `pulldown-cmark`.
//! Comments (`%%…%%`) are removed and `==highlights==` styled lexically.

use ors_core::{LinkKind, LinkResolver, NotePath, ParsedNote, Subpath};
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
    /// (offset in `markdown`, offset in the source, copied verbatim?) at
    /// every edit boundary, to map rendered blocks back to source lines.
    anchors: Vec<(usize, usize, bool)>,
    /// Start offsets of the source lines.
    line_starts: Vec<usize>,
}

impl Prepared {
    /// Source byte offset of a byte offset in [`Prepared::markdown`].
    pub fn source_offset(&self, md: usize) -> usize {
        let i = self.anchors.partition_point(|a| a.0 <= md);
        match i.checked_sub(1).and_then(|i| self.anchors.get(i)) {
            Some(&(out, src, true)) => src + (md - out),
            Some(&(_, src, false)) => src,
            None => md,
        }
    }

    /// 0-based source line of a byte offset in [`Prepared::markdown`].
    pub fn source_line(&self, md: usize) -> usize {
        let off = self.source_offset(md);
        self.line_starts.partition_point(|s| *s <= off).saturating_sub(1)
    }
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
    let mut prepared = Prepared::default();
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
    let mut anchors = Vec::new();
    for (start, end, rep) in edits {
        if start < pos
            || end > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(end)
        {
            continue; // overlapping (e.g. link inside a comment) or invalid
        }
        anchors.push((out.len(), pos, true));
        out.push_str(&source[pos..start]);
        anchors.push((out.len(), start, false));
        out.push_str(&rep);
        pos = end;
    }
    anchors.push((out.len(), pos, true));
    out.push_str(&source[pos..]);
    prepared.markdown = out;
    prepared.anchors = anchors;
    prepared.line_starts = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    prepared
}

//PARSE//
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::naive_parse;
    use ors_core::NoResolver;

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
}
