//! Source analysis shared by [`crate::parse`], [`crate::render_html`] and
//! [`crate::rewrite_links`].
//!
//! A first pulldown-cmark pass over the body gives the block structure and the
//! regions where Obsidian syntax is inert (code, math, raw HTML tags); a
//! hand-written scanner then finds comments, wikilinks, tags, block ids,
//! tasks and highlights with exact byte spans.

use ors_core::{BlockId, Heading, Link, LinkKind, Span, Subpath, Tag, Task};
use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag as MdTag, TagEnd};

use crate::frontmatter::{self, FrontmatterResult};
use crate::util::{
    LineIndex, RangeSet, count_words, find_in, is_escaped, is_external, percent_decode, rfind_in,
    unescape_backslashes,
};

/// Markdown extensions used by both pulldown-cmark passes.
pub(crate) fn md_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
}

/// A link plus the information needed to rewrite / render it.
#[derive(Debug, Clone)]
pub(crate) struct LinkRec {
    pub link: Link,
    /// Byte range of the path part of the target in the source, if known.
    pub target_range: Option<(usize, usize)>,
    /// `target#subpath` as written (markdown: percent-decoded).
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElemKind {
    /// Index into [`Analysis::links`].
    Wiki(usize),
    /// Index into [`Analysis::tags`].
    Tag(usize),
    /// Index into [`Analysis::block_ids`].
    BlockId(usize),
    /// Task checkbox `[c]`.
    Task(char),
    HlOpen,
    HlClose,
    Comment,
}

/// An Obsidian inline construct with its byte range (used by the renderer).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Element {
    pub start: usize,
    pub end: usize,
    pub kind: ElemKind,
}

pub(crate) struct Analysis {
    pub fm: Option<FrontmatterResult>,
    pub links: Vec<LinkRec>,
    pub tags: Vec<Tag>,
    pub headings: Vec<Heading>,
    pub block_ids: Vec<BlockId>,
    pub tasks: Vec<Task>,
    pub word_count: usize,
    pub body_start: usize,
    /// Sorted, non-overlapping.
    pub elements: Vec<Element>,
}

/// Parsed content of `[[...]]`.
pub(crate) struct WikiInner {
    pub target: String,
    pub target_range: (usize, usize),
    pub subpath: Option<Subpath>,
    pub display: Option<String>,
    pub raw: String,
}

/// Parse `src[cs..ce]`, the text between `[[` and `]]`.
pub(crate) fn parse_wiki_inner(src: &str, cs: usize, ce: usize) -> Option<WikiInner> {
    let content = src.get(cs..ce)?;
    let (tpart_end, alias) = match content.find('|') {
        Some(q) => {
            let te = if q > 0 && content.as_bytes()[q - 1] == b'\\' {
                q - 1
            } else {
                q
            };
            (te, Some(&content[q + 1..]))
        }
        None => (content.len(), None),
    };
    let tpart = &content[..tpart_end];
    let (path, sub) = match tpart.find('#') {
        Some(h) => (&tpart[..h], Some(&tpart[h + 1..])),
        None => (tpart, None),
    };
    let lead = path.len() - path.trim_start().len();
    let target = path.trim();
    let ts = cs + lead;
    let subpath = sub.and_then(parse_subpath);
    if target.is_empty() && subpath.is_none() {
        return None;
    }
    let display = alias
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(String::from);
    Some(WikiInner {
        target: target.to_string(),
        target_range: (ts, ts + target.len()),
        subpath,
        display,
        raw: tpart.trim().to_string(),
    })
}

/// Parse a subpath (the text after the first `#`).
pub(crate) fn parse_subpath(s: &str) -> Option<Subpath> {
    let s = s.trim();
    if let Some(id) = s.strip_prefix('^') {
        let id = id.trim();
        return (!id.is_empty()).then(|| Subpath::Block(id.to_string()));
    }
    let parts: Vec<String> = s
        .split('#')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    (!parts.is_empty()).then_some(Subpath::Heading(parts))
}

/// Text Obsidian shows for a link without alias: `Note`, `Note > Heading`,
/// `Note > ^block`, or just the subpath for same-note links.
pub(crate) fn default_display(target: &str, subpath: Option<&Subpath>) -> String {
    let sub = match subpath {
        None => return target.to_string(),
        Some(Subpath::Heading(p)) => p.join(" > "),
        Some(Subpath::Block(id)) => format!("^{id}"),
    };
    if target.is_empty() {
        sub
    } else {
        format!("{target} > {sub}")
    }
}

pub(crate) fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '-' | '/' | '\u{200D}' | '\u{FE0F}')
        || matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF)
}

/// A tag name (without `#`) is valid when non-empty, made of tag characters
/// and not purely numeric.
pub fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(is_tag_char)
        && name.chars().any(|c| !c.is_ascii_digit() && c != '/')
}

struct MdLink {
    range: (usize, usize),
    image: bool,
    inline: bool,
    dest: String,
    text: String,
}

struct P1Heading {
    level: u8,
    range: (usize, usize),
    pieces: Vec<(String, (usize, usize))>,
}

#[derive(Default)]
struct Pass1 {
    code: Vec<(usize, usize)>,
    math: Vec<(usize, usize)>,
    html: Vec<(usize, usize)>,
    mdlinks: Vec<MdLink>,
    headings: Vec<P1Heading>,
    items: Vec<usize>,
}

fn heading_level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn pass1(source: &str, body_start: usize) -> Pass1 {
    let body = &source[body_start..];
    let mut p = Pass1::default();
    let mut open_links: Vec<Option<usize>> = Vec::new();
    let mut heading: Option<usize> = None;
    for (ev, r) in Parser::new_ext(body, md_options()).into_offset_iter() {
        let r = (r.start + body_start, r.end + body_start);
        let mut text: Option<String> = None;
        match ev {
            Event::Start(MdTag::CodeBlock(_)) => p.code.push(r),
            Event::Code(t) => {
                p.code.push(r);
                text = Some(t.to_string());
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                p.math.push(r);
                text = Some(t.to_string());
            }
            Event::InlineHtml(_) => p.html.push(r),
            Event::Start(MdTag::HtmlBlock) => p.html.extend(html_tag_ranges(source, r.0, r.1)),
            Event::Start(MdTag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                open_links.push(Some(p.mdlinks.len()));
                p.mdlinks.push(MdLink {
                    range: r,
                    image: false,
                    inline: link_type == LinkType::Inline,
                    dest: dest_url.to_string(),
                    text: String::new(),
                });
            }
            Event::Start(MdTag::Image {
                link_type,
                dest_url,
                ..
            }) => {
                open_links.push(Some(p.mdlinks.len()));
                p.mdlinks.push(MdLink {
                    range: r,
                    image: true,
                    inline: link_type == LinkType::Inline,
                    dest: dest_url.to_string(),
                    text: String::new(),
                });
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                open_links.pop();
            }
            Event::Start(MdTag::Heading { level, .. }) => {
                heading = Some(p.headings.len());
                p.headings.push(P1Heading {
                    level: heading_level(level),
                    range: r,
                    pieces: Vec::new(),
                });
            }
            Event::End(TagEnd::Heading(_)) => heading = None,
            Event::Start(MdTag::Item) => p.items.push(r.0),
            Event::Text(t) => text = Some(t.to_string()),
            Event::SoftBreak | Event::HardBreak => text = Some(" ".into()),
            _ => {}
        }
        if let Some(t) = text {
            for l in open_links.iter().flatten() {
                if let Some(m) = p.mdlinks.get_mut(*l) {
                    m.text.push_str(&t);
                }
            }
            if let Some(h) = heading.and_then(|h| p.headings.get_mut(h)) {
                h.pieces.push((t, r));
            }
        }
    }
    p
}

/// Ranges of tags and comments inside a raw HTML block.
fn html_tag_ranges(source: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = start;
    while let Some(lt) = find_in(b, i, end, b"<") {
        if b[lt..end].starts_with(b"<!--") {
            let e = find_in(b, lt + 4, end, b"-->").map_or(end, |p| p + 3);
            out.push((lt, e));
            i = e;
            continue;
        }
        let next = b.get(lt + 1).copied().unwrap_or(b' ');
        if next.is_ascii_alphabetic() || next == b'/' || next == b'!' || next == b'?' {
            let mut j = lt + 1;
            let mut quote: Option<u8> = None;
            while j < end {
                let c = b[j];
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => break,
                    None => {}
                }
                j += 1;
            }
            let e = (j + 1).min(end);
            out.push((lt, e));
            i = e;
        } else {
            i = lt + 1;
        }
    }
    out
}

/// `%%...%%` comments (outside code); an unclosed `%%` runs to the end.
fn find_comments(source: &str, from: usize, inert: &RangeSet) -> Vec<(usize, usize)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = from;
    while let Some(p) = find_in(b, i, b.len(), b"%%") {
        if inert.contains(p) {
            i = p + 1;
            continue;
        }
        let end = find_in(b, p + 2, b.len(), b"%%").map_or(b.len(), |c| c + 2);
        out.push((p, end));
        i = end;
    }
    out
}

/// Iterate `(line_start, line_end)` (end excludes `\n`) over `start..len`.
fn lines_from(source: &str, start: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let b = source.as_bytes();
    let mut pos = Some(start);
    std::iter::from_fn(move || {
        let ls = pos?;
        if ls > b.len() {
            return None;
        }
        match find_in(b, ls, b.len(), b"\n") {
            Some(nl) => {
                pos = Some(nl + 1);
                Some((ls, nl))
            }
            None => {
                pos = None;
                Some((ls, b.len()))
            }
        }
    })
}

/// Find the destination of an inline markdown link `[text](dest "title")`
/// spanning `s..e`. Returns `(dest_start, dest_end)` without angle brackets.
fn parse_md_dest(source: &str, s: usize, e: usize) -> Option<(usize, usize)> {
    let b = source.as_bytes();
    let e = e.min(b.len());
    let mut i = s;
    if b.get(i) == Some(&b'!') {
        i += 1;
    }
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    while i < e {
        match b[i] {
            b'\\' if b.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                i += 2;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            b'`' => {
                let run_start = i;
                while i < e && b[i] == b'`' {
                    i += 1;
                }
                let n = i - run_start;
                // Find the closing run of exactly n backticks.
                let mut j = i;
                let mut closed = None;
                while j < e {
                    if b[j] == b'`' {
                        let rs = j;
                        while j < e && b[j] == b'`' {
                            j += 1;
                        }
                        if j - rs == n {
                            closed = Some(j);
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                if let Some(c) = closed {
                    i = c;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if i >= e {
        return None;
    }
    i += 1;
    if b.get(i) != Some(&b'(') {
        return None;
    }
    i += 1;
    while i < e && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    if i >= e {
        return None;
    }
    if b[i] == b'<' {
        let st = i + 1;
        let mut j = st;
        while j < e {
            match b[j] {
                b'\\' if b.get(j + 1).is_some_and(u8::is_ascii_punctuation) => j += 2,
                b'>' => return Some((st, j)),
                b'\n' | b'<' => return None,
                _ => j += 1,
            }
        }
        return None;
    }
    let st = i;
    let mut paren = 0usize;
    while i < e {
        let c = b[i];
        if c == b'\\' && b.get(i + 1).is_some_and(u8::is_ascii_punctuation) {
            i += 2;
            continue;
        }
        if c.is_ascii_whitespace() {
            break;
        }
        if c == b'(' {
            paren += 1;
        } else if c == b')' {
            if paren == 0 {
                break;
            }
            paren -= 1;
        }
        i += 1;
    }
    Some((st, i.min(e)))
}

/// Pair `==` highlight markers on each line. Returns `(open, close)` byte
/// positions of the two-character markers.
pub(crate) fn highlight_pairs(
    source: &str,
    from: usize,
    to: usize,
    excl: &RangeSet,
) -> Vec<(usize, usize)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    let mut line_end = find_in(b, from, to, b"\n").unwrap_or(to);
    let mut i = from;
    while let Some(p) = find_in(b, i, to, b"==") {
        if p >= line_end {
            open = None;
            line_end = find_in(b, p, to, b"\n").unwrap_or(to);
        }
        // Must be a run of exactly two '='.
        let mut run_end = p;
        while run_end < to && b[run_end] == b'=' {
            run_end += 1;
        }
        i = run_end;
        if run_end - p != 2
            || (p > from && b[p - 1] == b'=')
            || excl.overlaps(p, p + 2)
            || is_escaped(b, p, from)
        {
            continue;
        }
        let next_ws = source[p + 2..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace);
        let prev_ws = source[..p]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        match open {
            Some(o) if !prev_ws && p > o + 2 => {
                out.push((o, p));
                open = None;
            }
            _ if !next_ws => open = Some(p),
            _ => {}
        }
    }
    out
}

/// Normalise heading text: wikilinks → their display text, `==` markers and
/// a trailing `^block-id` removed, whitespace collapsed.
pub(crate) fn clean_heading_text(s: &str) -> String {
    let b = s.as_bytes();
    let mut t = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"[[") {
            let line_end = find_in(b, i, b.len(), b"\n").unwrap_or(b.len());
            if let Some(close) = find_in(b, i + 2, line_end, b"]]")
                && let Some(w) = parse_wiki_inner(s, i + 2, close)
            {
                let d = w
                    .display
                    .unwrap_or_else(|| default_display(&w.target, w.subpath.as_ref()));
                t.push_str(&d);
                i = close + 2;
                continue;
            }
        }
        let Some(c) = s[i..].chars().next() else {
            break;
        };
        t.push(c);
        i += c.len_utf8();
    }
    let pairs = highlight_pairs(&t, 0, t.len(), &RangeSet::default());
    let mut u = String::with_capacity(t.len());
    let mut last = 0;
    for (o, c) in pairs {
        u.push_str(&t[last..o]);
        u.push_str(&t[o + 2..c]);
        last = c + 2;
    }
    u.push_str(&t[last..]);
    let mut words: Vec<&str> = u.split_whitespace().collect();
    if words.len() > 1
        && let Some(last) = words.last()
        && let Some(id) = last.strip_prefix('^')
        && !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        words.pop();
    }
    words.join(" ")
}

fn parse_task(source: &str, start: usize) -> Option<(char, String, Span, (usize, usize))> {
    let b = source.as_bytes();
    let line_end = find_in(b, start, b.len(), b"\n").unwrap_or(b.len());
    let mut i = start;
    while i < line_end && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    match b.get(i)? {
        b'-' | b'*' | b'+' => i += 1,
        c if c.is_ascii_digit() => {
            let ds = i;
            while i < line_end && b[i].is_ascii_digit() {
                i += 1;
            }
            if i - ds > 9 || !matches!(b.get(i), Some(b'.' | b')')) {
                return None;
            }
            i += 1;
        }
        _ => return None,
    }
    let ws = i;
    while i < line_end && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    if i == ws || b.get(i) != Some(&b'[') {
        return None;
    }
    let status = source.get(i + 1..line_end)?.chars().next()?;
    if status == '\r' || status == '\t' {
        return None;
    }
    let close = i + 1 + status.len_utf8();
    if b.get(close) != Some(&b']') {
        return None;
    }
    let after = close + 1;
    if after < line_end && !matches!(b[after], b' ' | b'\t' | b'\r') {
        return None;
    }
    let text_end = source[..line_end].trim_end().len().max(after.min(line_end));
    let text = source
        .get(after.min(text_end)..text_end)?
        .trim()
        .to_string();
    Some((status, text, Span::new(start, text_end), (i, close + 1)))
}

/// Analyse a note. `with_highlights` additionally computes `==` markers.
pub(crate) fn analyze(source: &str, with_highlights: bool) -> Analysis {
    let lines = LineIndex::new(source);
    let fm = frontmatter::extract(source, &lines);
    let body_start = match &fm {
        Some(f) => f.block.end,
        None if source.starts_with('\u{FEFF}') => 3,
        None => 0,
    };
    let bytes = source.as_bytes();
    let p1 = pass1(source, body_start);

    let mut inert_v = p1.code.clone();
    inert_v.extend_from_slice(&p1.math);
    inert_v.extend_from_slice(&p1.html);
    let inert = RangeSet::new(inert_v);
    let comments = find_comments(source, body_start, &inert);
    let comment_set = RangeSet::new(comments.clone());
    let excluded = inert.union(&comments);

    // Markdown links / images.
    let mut body_links: Vec<LinkRec> = Vec::new();
    let mut link_ranges: Vec<(usize, usize)> = Vec::new();
    for m in &p1.mdlinks {
        if excluded.contains(m.range.0) {
            continue;
        }
        link_ranges.push(m.range);
        if !m.inline {
            continue;
        }
        let found = parse_md_dest(source, m.range.0, m.range.1);
        let (dest, raw_src) = match found {
            Some((ds, de)) => (unescape_backslashes(&source[ds..de]), Some((ds, de))),
            None => (m.dest.clone(), None),
        };
        if dest.is_empty() || is_external(&dest) {
            continue;
        }
        let (path_raw, sub_raw) = match dest.find('#') {
            Some(h) => (&dest[..h], Some(&dest[h + 1..])),
            None => (dest.as_str(), None),
        };
        let target = percent_decode(path_raw);
        let subpath = sub_raw.and_then(|s| parse_subpath(&percent_decode(s)));
        if target.is_empty() && subpath.is_none() {
            continue;
        }
        let target_range = raw_src.map(|(ds, de)| {
            let pe = source[ds..de].find('#').map_or(de, |h| ds + h);
            (ds, pe)
        });
        let text = m.text.trim();
        body_links.push(LinkRec {
            link: Link {
                target,
                subpath,
                display: (!text.is_empty()).then(|| text.to_string()),
                kind: LinkKind::Markdown,
                is_embed: m.image,
                span: Span::new(m.range.0, m.range.1),
                line: lines.line_of(m.range.0),
            },
            target_range,
            raw: percent_decode(&dest),
        });
    }
    let excl_links = excluded.union(&link_ranges);

    // Wikilinks and embeds.
    let mut wiki_ranges = Vec::new();
    for (ls, le) in lines_from(source, body_start) {
        let mut p = ls;
        while let Some(j) = find_in(bytes, p, le, b"]]") {
            let k = rfind_in(bytes, p, j, b"[[");
            p = j + 2;
            let Some(k) = k else { continue };
            if excl_links.contains(k) || excl_links.contains(j) || is_escaped(bytes, k, ls) {
                continue;
            }
            let Some(w) = parse_wiki_inner(source, k + 2, j) else {
                continue;
            };
            let embed = k > ls
                && bytes[k - 1] == b'!'
                && !is_escaped(bytes, k - 1, ls)
                && !excl_links.contains(k - 1);
            let start = if embed { k - 1 } else { k };
            wiki_ranges.push((start, j + 2));
            body_links.push(LinkRec {
                link: Link {
                    target: w.target,
                    subpath: w.subpath,
                    display: w.display,
                    kind: LinkKind::Wiki,
                    is_embed: embed,
                    span: Span::new(start, j + 2),
                    line: lines.line_of(start),
                },
                target_range: Some(w.target_range),
                raw: w.raw,
            });
        }
    }
    body_links.sort_by_key(|l| l.link.span.start);
    let excl_tags = excl_links.union(&wiki_ranges);

    // Tags.
    let mut body_tags = Vec::new();
    let mut i = body_start;
    while let Some(h) = find_in(bytes, i, bytes.len(), b"#") {
        i = h + 1;
        if excl_tags.contains(h) {
            continue;
        }
        let prev_ok = h == body_start
            || source[..h]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if !prev_ok {
            continue;
        }
        let mut end = h + 1;
        for c in source[h + 1..].chars() {
            if is_tag_char(c) {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        let name = &source[h + 1..end];
        if is_valid_tag_name(name) {
            body_tags.push(Tag {
                name: name.to_string(),
                span: Span::new(h, end),
                line: lines.line_of(h),
                from_frontmatter: false,
            });
            i = end;
        }
    }

    // Block ids.
    let mut block_ids = Vec::new();
    for (ls, le) in lines_from(source, body_start) {
        let line = source[ls..le].trim_end();
        let lend = ls + line.len();
        let Some(rel) = line.rfind('^') else { continue };
        let caret = ls + rel;
        let id = &source[caret + 1..lend];
        if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            continue;
        }
        let prev_ok = caret == ls
            || source[..caret]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if !prev_ok || excl_tags.contains(caret) {
            continue;
        }
        block_ids.push(BlockId {
            id: id.to_string(),
            span: Span::new(caret, lend),
            line: lines.line_of(caret),
        });
    }

    // Tasks.
    let mut tasks = Vec::new();
    let mut task_markers = Vec::new();
    for &start in &p1.items {
        if excluded.contains(start) {
            continue;
        }
        if let Some((status, text, span, marker)) = parse_task(source, start) {
            if excl_tags.contains(marker.0) {
                continue;
            }
            task_markers.push((marker, status));
            tasks.push(Task {
                status,
                text,
                span,
                line: lines.line_of(start),
            });
        }
    }

    // Headings.
    let mut headings = Vec::new();
    for h in &p1.headings {
        if excluded.contains(h.range.0) {
            continue;
        }
        let mut text = String::new();
        for (t, r) in &h.pieces {
            if !comment_set.overlaps(r.0, r.1) {
                text.push_str(t);
            } else if source.get(r.0..r.1) == Some(t.as_str()) {
                for (off, c) in t.char_indices() {
                    if !comment_set.contains(r.0 + off) {
                        text.push(c);
                    }
                }
            }
        }
        let end = h.range.0 + source[h.range.0..h.range.1].trim_end().len();
        headings.push(Heading {
            level: h.level,
            text: clean_heading_text(&text),
            span: Span::new(h.range.0, end),
            line: lines.line_of(h.range.0),
        });
    }

    let word_count = count_words(&source[body_start..], |off| {
        comment_set
            .end_containing(off + body_start)
            .map(|e| e - body_start)
    });

    // Assemble links and tags (frontmatter first).
    let mut links: Vec<LinkRec> = Vec::new();
    let mut tags: Vec<Tag> = Vec::new();
    if let Some(f) = &fm {
        links.extend(f.links.iter().cloned());
        tags.extend(f.tags.iter().cloned());
    }
    let link_offset = links.len();
    let tag_offset = tags.len();
    links.extend(body_links);
    tags.extend(body_tags);

    // Renderer elements.
    let mut elements = Vec::new();
    for (idx, l) in links.iter().enumerate().skip(link_offset) {
        if l.link.kind == LinkKind::Wiki {
            elements.push(Element {
                start: l.link.span.start,
                end: l.link.span.end,
                kind: ElemKind::Wiki(idx),
            });
        }
    }
    for (idx, t) in tags.iter().enumerate().skip(tag_offset) {
        elements.push(Element {
            start: t.span.start,
            end: t.span.end,
            kind: ElemKind::Tag(idx),
        });
    }
    for (idx, b) in block_ids.iter().enumerate() {
        elements.push(Element {
            start: b.span.start,
            end: b.span.end,
            kind: ElemKind::BlockId(idx),
        });
    }
    for &((s, e), status) in &task_markers {
        elements.push(Element {
            start: s,
            end: e,
            kind: ElemKind::Task(status),
        });
    }
    for &(s, e) in &comments {
        elements.push(Element {
            start: s,
            end: e,
            kind: ElemKind::Comment,
        });
    }
    if with_highlights {
        let mut taken: Vec<(usize, usize)> = elements.iter().map(|e| (e.start, e.end)).collect();
        taken.extend_from_slice(&link_ranges);
        let excl_hl = excluded.union(&taken);
        for (o, c) in highlight_pairs(source, body_start, source.len(), &excl_hl) {
            elements.push(Element {
                start: o,
                end: o + 2,
                kind: ElemKind::HlOpen,
            });
            elements.push(Element {
                start: c,
                end: c + 2,
                kind: ElemKind::HlClose,
            });
        }
    }
    elements.sort_by_key(|e| (e.start, e.end));
    let mut cleaned: Vec<Element> = Vec::with_capacity(elements.len());
    for e in elements {
        if cleaned.last().is_none_or(|l| e.start >= l.end) {
            cleaned.push(e);
        }
    }

    Analysis {
        fm,
        links,
        tags,
        headings,
        block_ids,
        tasks,
        word_count,
        body_start,
        elements: cleaned,
    }
}
