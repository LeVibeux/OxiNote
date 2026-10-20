//! Unit tests. `ParsedNote`s are built by hand (or by the tiny test-only
//! extractor below) so the tests do not depend on `ors-markdown`.

mod cache_tests;
mod fuzzy_tests;
mod graph_tests;
mod perf_tests;
mod refactor_tests;
mod resolve_tests;
mod search_tests;

use ors_core::{Link, LinkKind, MemoryVault, NotePath, ParsedNote, Span, Subpath, Tag};
use regex::Regex;

use crate::MetadataCache;

pub(crate) fn p(s: &str) -> NotePath {
    NotePath::new(s).unwrap()
}

/// A link with only a target (wikilink, not an embed).
pub(crate) fn wiki(target: &str) -> Link {
    Link {
        target: target.into(),
        subpath: None,
        display: None,
        kind: LinkKind::Wiki,
        is_embed: false,
        span: Span::default(),
        line: 0,
    }
}

pub(crate) fn md_link(target: &str) -> Link {
    Link {
        kind: LinkKind::Markdown,
        ..wiki(target)
    }
}

pub(crate) fn tag(name: &str) -> Tag {
    Tag {
        name: name.into(),
        span: Span::default(),
        line: 0,
        from_frontmatter: false,
    }
}

/// Builder for hand-made notes.
#[derive(Default)]
pub(crate) struct NoteBuilder(ParsedNote);

impl NoteBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn link(mut self, l: Link) -> Self {
        self.0.links.push(l);
        self
    }
    pub fn links(mut self, targets: &[&str]) -> Self {
        self.0.links.extend(targets.iter().map(|t| wiki(t)));
        self
    }
    pub fn tags(mut self, tags: &[&str]) -> Self {
        self.0.tags.extend(tags.iter().map(|t| tag(t)));
        self
    }
    pub fn alias(mut self, a: &str) -> Self {
        self.0.aliases.push(a.into());
        self
    }
    pub fn prop(mut self, k: &str, v: serde_json::Value) -> Self {
        self.0
            .frontmatter
            .get_or_insert_with(Default::default)
            .insert(k.into(), v);
        self
    }
    pub fn build(self) -> ParsedNote {
        self.0
    }
}

fn line_of(src: &str, offset: usize) -> usize {
    src[..offset].matches('\n').count()
}

/// Minimal Obsidian-ish extractor for tests: wikilinks/embeds, internal
/// markdown links (with `%20` decoding), inline `#tags`. Spans are exact.
pub(crate) fn mini_parse(src: &str) -> ParsedNote {
    let wiki_re = Regex::new(r"(!?)\[\[([^\]|#]*)(#[^\]|]*)?(\|[^\]]*)?\]\]").unwrap();
    let md_re = Regex::new(r"(!?)\[([^\]]*)\]\(([^)\s#]*)(#[^)]*)?\)").unwrap();
    let tag_re = Regex::new(r"(?m)(?:^|\s)#([\p{L}\p{N}_/-]+)").unwrap();
    let mut links = Vec::new();
    let subpath = |s: Option<&str>| {
        s.map(|s| {
            let s = &s[1..];
            match s.strip_prefix('^') {
                Some(b) => Subpath::Block(b.into()),
                None => Subpath::Heading(s.split('#').map(str::to_string).collect()),
            }
        })
    };
    for c in wiki_re.captures_iter(src) {
        let m = c.get(0).unwrap();
        links.push(Link {
            target: c[2].trim().to_string(),
            subpath: subpath(c.get(3).map(|m| m.as_str())),
            display: c.get(4).map(|m| m.as_str()[1..].to_string()),
            kind: LinkKind::Wiki,
            is_embed: !c[1].is_empty(),
            span: Span::new(m.start(), m.end()),
            line: line_of(src, m.start()),
        });
    }
    for c in md_re.captures_iter(src) {
        let target = &c[3];
        if target.contains("://") || target.is_empty() {
            continue;
        }
        let m = c.get(0).unwrap();
        links.push(Link {
            target: target.replace("%20", " "),
            subpath: subpath(c.get(4).map(|m| m.as_str())),
            display: Some(c[2].to_string()),
            kind: LinkKind::Markdown,
            is_embed: !c[1].is_empty(),
            span: Span::new(m.start(), m.end()),
            line: line_of(src, m.start()),
        });
    }
    links.sort_by_key(|l| l.span.start);
    let tags = tag_re
        .captures_iter(src)
        .map(|c| {
            let m = c.get(1).unwrap();
            Tag {
                name: m.as_str().to_string(),
                span: Span::new(m.start() - 1, m.end()),
                line: line_of(src, m.start()),
                from_frontmatter: false,
            }
        })
        .collect();
    ParsedNote {
        links,
        tags,
        word_count: src.split_whitespace().count(),
        ..ParsedNote::default()
    }
}

/// Cache over `files`: `.md` files are parsed with [`mini_parse`], others
/// are registered as attachments.
pub(crate) fn cache_of(files: &[(&str, &str)]) -> MetadataCache {
    let mut c = MetadataCache::new();
    for (path, src) in files {
        let path = p(path);
        if path.is_markdown() {
            c.insert_parsed(path, mini_parse(src));
        } else {
            c.insert_file(path);
        }
    }
    c
}

pub(crate) fn vault_of(files: &[(&str, &str)]) -> MemoryVault {
    MemoryVault::from_files(files.iter().copied())
}

pub(crate) fn paths(v: &[NotePath]) -> Vec<&str> {
    v.iter().map(NotePath::as_str).collect()
}

#[test]
fn mini_parse_extracts_spans() {
    let src = "a [[B#h|x]] ![](img%20a.png)\n#tag/sub";
    let n = mini_parse(src);
    assert_eq!(n.links.len(), 2);
    assert_eq!(
        &src[n.links[0].span.start..n.links[0].span.end],
        "[[B#h|x]]"
    );
    assert_eq!(n.links[1].target, "img a.png");
    assert!(n.links[1].is_embed);
    assert_eq!(n.tags[0].name, "tag/sub");
    assert_eq!(n.tags[0].line, 1);
}
