//! HTML rendering.
//!
//! Obsidian constructs found by [`analyze`] are replaced by inert placeholder
//! tokens (`U+E000 <index> U+E001`), the result is parsed again by
//! pulldown-cmark, and the event stream is transformed: placeholders become
//! HTML, callouts / headings / internal links / math are rewritten, raw HTML is
//! sanitised.

use std::collections::HashMap;
use std::fmt::Write as _;

use ors_core::{LinkResolver, NotePath, Subpath};
use pulldown_cmark::{CowStr, Event, LinkType, Parser, Tag, TagEnd};
use serde_json::Value as Json;

use crate::RenderOptions;
use crate::frontmatter::is_whole_wikilink;
use crate::scan::{Analysis, ElemKind, LinkRec, analyze, default_display, md_options};
use crate::util::{
    encode_path, ends_with_md, escape_html, is_external, is_image_target, percent_decode,
    push_escaped, slugify,
};

const PH_OPEN: char = '\u{E000}';
const PH_CLOSE: char = '\u{E001}';

struct Ctx<'a> {
    source: &'a str,
    a: &'a Analysis,
    opts: &'a RenderOptions,
    resolver: &'a dyn LinkResolver,
}

impl Ctx<'_> {
    /// Path part of an href for `target` (with prefix and extension mapping)
    /// and whether it resolved. Empty target = the current note.
    fn href_path(&self, target: &str) -> (String, bool) {
        if target.is_empty() {
            return (String::new(), true);
        }
        let resolved = self
            .resolver
            .resolve(target, &self.opts.note_path)
            .or_else(|| {
                if ends_with_md(target) {
                    self.resolver
                        .resolve(&target[..target.len() - 3], &self.opts.note_path)
                } else {
                    None
                }
            });
        match resolved {
            Some(p) => (
                format!(
                    "{}{}",
                    self.opts.link_prefix,
                    encode_path(&self.apply_ext(&p))
                ),
                true,
            ),
            None => (
                format!("{}{}", self.opts.link_prefix, encode_path(target)),
                false,
            ),
        }
    }

    fn apply_ext(&self, p: &NotePath) -> String {
        match &self.opts.note_extension {
            Some(ext) if p.is_markdown() => {
                let ext = ext.trim_start_matches('.');
                if ext.is_empty() {
                    p.without_extension().to_string()
                } else {
                    format!("{}.{}", p.without_extension(), ext)
                }
            }
            _ => p.as_str().to_string(),
        }
    }

    fn href(&self, target: &str, subpath: Option<&Subpath>) -> (String, bool) {
        let (mut h, ok) = self.href_path(target);
        h.push_str(&fragment(subpath));
        if h.is_empty() {
            h.push('#');
        }
        (h, ok)
    }

    fn wiki_html(&self, rec: &LinkRec) -> String {
        let l = &rec.link;
        let mut out = String::new();
        if l.is_embed {
            if is_image_target(&l.target) {
                let (src, _) = self.href_path(&l.target);
                let (mut width, mut height, mut alt) = (None, None, None);
                if let Some(d) = &l.display {
                    if let Some((w, h)) = parse_size(d) {
                        width = Some(w);
                        height = h;
                    } else {
                        alt = Some(d.clone());
                    }
                }
                let alt = alt.unwrap_or_else(|| {
                    l.target.rsplit('/').next().unwrap_or(&l.target).to_string()
                });
                out.push_str("<img src=\"");
                push_escaped(&mut out, &src);
                out.push_str("\" alt=\"");
                push_escaped(&mut out, &alt);
                out.push('"');
                if let Some(w) = width {
                    let _ = write!(out, " width=\"{w}\"");
                }
                if let Some(h) = height {
                    let _ = write!(out, " height=\"{h}\"");
                }
                out.push('>');
            } else {
                out.push_str("<div class=\"internal-embed\" data-src=\"");
                push_escaped(&mut out, &rec.raw);
                out.push_str("\"></div>");
            }
            return out;
        }
        let (href, ok) = self.href(&l.target, l.subpath.as_ref());
        let text = l
            .display
            .clone()
            .unwrap_or_else(|| default_display(&l.target, l.subpath.as_ref()));
        out.push_str(if ok {
            "<a class=\"internal-link\" href=\""
        } else {
            "<a class=\"internal-link is-unresolved\" href=\""
        });
        push_escaped(&mut out, &href);
        out.push_str("\" data-href=\"");
        push_escaped(&mut out, &rec.raw);
        out.push_str("\">");
        push_escaped(&mut out, &text);
        out.push_str("</a>");
        out
    }

    /// HTML for element `idx`, plus how it affects the `<mark>` state.
    fn element_html(&self, idx: usize, mark_open: &mut bool) -> String {
        let Some(el) = self.a.elements.get(idx) else {
            return String::new();
        };
        match el.kind {
            ElemKind::Wiki(i) => self
                .a
                .links
                .get(i)
                .map(|r| self.wiki_html(r))
                .unwrap_or_default(),
            ElemKind::Tag(i) => self
                .a
                .tags
                .get(i)
                .map(|t| tag_html(&t.name))
                .unwrap_or_default(),
            ElemKind::BlockId(i) => self
                .a
                .block_ids
                .get(i)
                .map(|b| {
                    format!(
                        "<span class=\"block-id\" id=\"^{}\"></span>",
                        escape_html(&b.id)
                    )
                })
                .unwrap_or_default(),
            ElemKind::Task(c) => {
                let mut s = String::from(
                    "<input type=\"checkbox\" class=\"task-list-item-checkbox\" data-task=\"",
                );
                push_escaped(&mut s, &c.to_string());
                s.push_str("\" disabled");
                if c != ' ' {
                    s.push_str(" checked");
                }
                s.push('>');
                s
            }
            ElemKind::HlOpen => {
                if *mark_open {
                    "==".into()
                } else {
                    *mark_open = true;
                    "<mark>".into()
                }
            }
            ElemKind::HlClose => {
                if *mark_open {
                    *mark_open = false;
                    "</mark>".into()
                } else {
                    "==".into()
                }
            }
            ElemKind::Comment => String::new(),
        }
    }

    /// Plain text of element `idx` (used for heading slugs).
    fn element_text(&self, idx: usize) -> String {
        let Some(el) = self.a.elements.get(idx) else {
            return String::new();
        };
        match el.kind {
            ElemKind::Wiki(i) => {
                self.a
                    .links
                    .get(i)
                    .map(|r| {
                        r.link.display.clone().unwrap_or_else(|| {
                            default_display(&r.link.target, r.link.subpath.as_ref())
                        })
                    })
                    .unwrap_or_default()
            }
            ElemKind::Tag(i) => self
                .a
                .tags
                .get(i)
                .map(|t| format!("#{}", t.name))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Original source text of element `idx` (restores placeholders that end
    /// up in code).
    fn element_source(&self, idx: usize) -> &str {
        self.a
            .elements
            .get(idx)
            .and_then(|e| self.source.get(e.start..e.end))
            .unwrap_or("")
    }
}

fn tag_html(name: &str) -> String {
    let n = escape_html(name);
    format!("<a class=\"tag\" href=\"#{n}\">#{n}</a>")
}

fn fragment(subpath: Option<&Subpath>) -> String {
    match subpath {
        None => String::new(),
        Some(Subpath::Heading(parts)) => {
            format!("#{}", slugify(parts.last().map_or("", String::as_str)))
        }
        Some(Subpath::Block(id)) => format!("#^{}", encode_path(id)),
    }
}

/// `300` → (300, None); `300x200` → (300, Some(200)).
fn parse_size(s: &str) -> Option<(u32, Option<u32>)> {
    let s = s.trim();
    if let Some((w, h)) = s.split_once('x') {
        Some((w.trim().parse().ok()?, Some(h.trim().parse().ok()?)))
    } else {
        Some((s.parse().ok()?, None))
    }
}

enum Piece<'s> {
    Text(&'s str),
    Elem(usize),
}

fn split_placeholders(s: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find(PH_OPEN) {
        let after = &rest[p + PH_OPEN.len_utf8()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let idx = after[..digits].parse::<usize>().ok();
        if let (Some(idx), true) = (idx, after[digits..].starts_with(PH_CLOSE)) {
            if p > 0 {
                out.push(Piece::Text(&rest[..p]));
            }
            out.push(Piece::Elem(idx));
            rest = &after[digits + PH_CLOSE.len_utf8()..];
        } else {
            out.push(Piece::Text(&rest[..p + PH_OPEN.len_utf8()]));
            rest = after;
        }
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    out
}

fn push_clean(out: &mut String, s: &str) {
    for c in s.chars() {
        out.push(if c == PH_OPEN || c == PH_CLOSE {
            '\u{FFFD}'
        } else {
            c
        });
    }
}

/// Merge consecutive `Text` events.
fn merge_text(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    let mut out: Vec<Event<'_>> = Vec::with_capacity(events.len());
    for ev in events {
        if let Event::Text(t) = &ev
            && let Some(Event::Text(prev)) = out.last_mut()
        {
            let mut s = prev.to_string();
            s.push_str(t);
            *prev = CowStr::from(s);
            continue;
        }
        out.push(ev);
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Rewrite `> [!type]+ Title` blockquotes into callout divs.
fn transform_callouts(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    let mut out = Vec::with_capacity(events.len());
    let mut stack: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::BlockQuote(_)) => {
                let header = match (events.get(i + 1), events.get(i + 2)) {
                    (Some(Event::Start(Tag::Paragraph)), Some(Event::Text(t))) => {
                        parse_callout_header(t)
                    }
                    _ => None,
                };
                let Some((kind, fold, title_rest)) = header else {
                    stack.push(false);
                    out.push(events[i].clone());
                    i += 1;
                    continue;
                };
                stack.push(true);
                let mut open = String::from("<div class=\"callout\" data-callout=\"");
                push_escaped(&mut open, &kind.to_lowercase());
                open.push('"');
                if let Some(f) = fold {
                    let _ = write!(open, " data-callout-fold=\"{f}\"");
                }
                open.push_str(">\n<div class=\"callout-title\">");
                out.push(Event::Html(open.into()));
                let mut j = i + 3;
                let mut title_events: Vec<Event<'_>> = Vec::new();
                if !title_rest.is_empty() {
                    title_events.push(Event::Text(title_rest.into()));
                }
                let mut depth = 0usize;
                let mut ended_paragraph = false;
                while j < events.len() {
                    match &events[j] {
                        Event::SoftBreak | Event::HardBreak if depth == 0 => break,
                        Event::End(TagEnd::Paragraph) if depth == 0 => {
                            ended_paragraph = true;
                            break;
                        }
                        Event::Start(_) => depth += 1,
                        Event::End(_) => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                    title_events.push(events[j].clone());
                    j += 1;
                }
                if title_events.is_empty() {
                    title_events.push(Event::Text(capitalize(&kind).into()));
                }
                out.extend(title_events);
                out.push(Event::Html(
                    "</div>\n<div class=\"callout-content\">\n".into(),
                ));
                if !ended_paragraph && j < events.len() {
                    out.push(Event::Start(Tag::Paragraph));
                }
                i = j + 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if stack.pop() == Some(true) {
                    out.push(Event::Html("</div></div>\n".into()));
                } else {
                    out.push(events[i].clone());
                }
                i += 1;
            }
            _ => {
                out.push(events[i].clone());
                i += 1;
            }
        }
    }
    out
}

/// `[!type]+ rest` → (type, fold, rest).
fn parse_callout_header(t: &str) -> Option<(String, Option<char>, String)> {
    let rest = t.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = rest[..close].trim();
    if kind.is_empty() || kind.contains(char::is_whitespace) {
        return None;
    }
    let mut after = &rest[close + 1..];
    let fold = match after.chars().next() {
        Some(c @ ('+' | '-')) => {
            after = &after[1..];
            Some(c)
        }
        _ => None,
    };
    if !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return None;
    }
    Some((kind.to_string(), fold, after.trim_start().to_string()))
}

const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Is the raw tag `<...>` safe to pass through?
fn tag_is_safe(tag: &str) -> bool {
    let inner = tag.trim_start_matches('<').trim_start_matches('/');
    let name: String = inner
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if !ALLOWED_TAGS.contains(&name.as_str()) {
        return false;
    }
    let lower: String = tag
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    if lower.contains("javascript:")
        || lower.contains("vbscript:")
        || lower.contains("data:")
        || lower.contains("&#")
        || lower.contains("&colon")
        || lower.contains("srcdoc")
        || lower.contains("formaction")
    {
        return false;
    }
    // Event handler attributes: whitespace / quote / slash followed by `on…=`.
    let lb = tag.to_ascii_lowercase();
    let b = lb.as_bytes();
    for i in 1..b.len() {
        if (b[i - 1].is_ascii_whitespace()
            || b[i - 1] == b'/'
            || b[i - 1] == b'"'
            || b[i - 1] == b'\'')
            && b[i..].starts_with(b"on")
        {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j > i + 2 && b.get(j) == Some(&b'=') {
                return false;
            }
        }
    }
    true
}

impl Ctx<'_> {
    /// Sanitise raw HTML: comments dropped, unsafe tags escaped, placeholders
    /// (outside tags) replaced by their HTML.
    fn sanitize_html(&self, s: &str, mark_open: &mut bool) -> String {
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        let b = s.as_bytes();
        while i < s.len() {
            let rest = &s[i..];
            if let Some(body) = rest.strip_prefix("<!--") {
                let end = body.find("-->").map_or(s.len(), |p| i + 4 + p + 3);
                i = end;
                continue;
            }
            if b[i] == b'<' {
                let next = b.get(i + 1).copied().unwrap_or(b' ');
                if next.is_ascii_alphabetic() || next == b'/' {
                    // Find the end of the tag, honouring quotes.
                    let mut j = i + 1;
                    let mut quote: Option<u8> = None;
                    while j < b.len() {
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
                    if j < b.len() {
                        let tag = &s[i..=j];
                        if tag_is_safe(tag) && !tag.contains(PH_OPEN) {
                            out.push_str(tag);
                        } else {
                            out.push_str(&escape_html(tag));
                        }
                        i = j + 1;
                        continue;
                    }
                }
                out.push_str("&lt;");
                i += 1;
                continue;
            }
            // Text up to the next '<'.
            let end = rest.find('<').map_or(s.len(), |p| i + p);
            for piece in split_placeholders(&s[i..end]) {
                match piece {
                    Piece::Text(t) => out.push_str(t),
                    Piece::Elem(idx) => out.push_str(&self.element_html(idx, mark_open)),
                }
            }
            i = end;
        }
        out
    }

    fn restore(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for piece in split_placeholders(s) {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Elem(idx) => out.push_str(self.element_source(idx)),
            }
        }
        out
    }

    fn plain_text(&self, s: &str) -> String {
        let mut out = String::new();
        for piece in split_placeholders(s) {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Elem(idx) => out.push_str(&self.element_text(idx)),
            }
        }
        out
    }

    fn properties_table(&self) -> String {
        let Some(map) = self.a.fm.as_ref().and_then(|f| f.map.as_ref()) else {
            return String::new();
        };
        let mut out = String::from("<table class=\"properties\"><tbody>\n");
        for (k, v) in map {
            out.push_str("<tr><th>");
            push_escaped(&mut out, k);
            out.push_str("</th><td>");
            let is_tags = k.eq_ignore_ascii_case("tags") || k.eq_ignore_ascii_case("tag");
            if is_tags {
                let names: Vec<String> = self
                    .a
                    .tags
                    .iter()
                    .filter(|t| t.from_frontmatter)
                    .map(|t| tag_html(&t.name))
                    .collect();
                out.push_str(&names.join(" "));
            } else {
                self.property_value(&mut out, v);
            }
            out.push_str("</td></tr>\n");
        }
        out.push_str("</tbody></table>\n");
        out
    }

    fn property_value(&self, out: &mut String, v: &Json) {
        match v {
            Json::Null => {}
            Json::String(s) => {
                let t = s.trim();
                let rec = is_whole_wikilink(t)
                    .then(|| crate::scan::parse_wiki_inner(t, 2, t.len() - 2))
                    .flatten();
                match rec {
                    Some(w) => {
                        let rec = LinkRec {
                            link: ors_core::Link {
                                target: w.target,
                                subpath: w.subpath,
                                display: w.display,
                                kind: ors_core::LinkKind::Frontmatter,
                                is_embed: false,
                                span: Default::default(),
                                line: 0,
                            },
                            target_range: None,
                            raw: w.raw,
                        };
                        out.push_str(&self.wiki_html(&rec));
                    }
                    None => push_escaped(out, s),
                }
            }
            Json::Array(items) => {
                for (n, it) in items.iter().enumerate() {
                    if n > 0 {
                        out.push_str(", ");
                    }
                    self.property_value(out, it);
                }
            }
            Json::Object(_) => push_escaped(out, &v.to_string()),
            other => push_escaped(out, &other.to_string()),
        }
    }
}

pub(crate) fn render(source: &str, opts: &RenderOptions, resolver: &dyn LinkResolver) -> String {
    let a = analyze(source, true);
    let ctx = Ctx {
        source,
        a: &a,
        opts,
        resolver,
    };

    // Substituted body.
    let mut text = String::with_capacity(source.len() - a.body_start + 16);
    let mut pos = a.body_start;
    for (idx, el) in a.elements.iter().enumerate() {
        if el.start < pos {
            continue;
        }
        push_clean(&mut text, &source[pos..el.start]);
        if el.kind != ElemKind::Comment {
            let _ = write!(text, "{PH_OPEN}{idx}{PH_CLOSE}");
        }
        pos = el.end;
    }
    push_clean(&mut text, &source[pos..]);

    let events: Vec<Event<'_>> = Parser::new_ext(&text, md_options()).collect();
    let events = transform_callouts(merge_text(events));

    let mut out_ev: Vec<Event<'_>> = Vec::with_capacity(events.len());
    let mut link_stack: Vec<bool> = Vec::new();
    let mut mark_open = false;
    let mut in_code = false;
    let mut slugs: HashMap<String, usize> = HashMap::new();
    let mut i = 0;
    while i < events.len() {
        let ev = &events[i];
        i += 1;
        match ev {
            Event::Text(t) => {
                if in_code {
                    out_ev.push(Event::Text(ctx.restore(t).into()));
                    continue;
                }
                for piece in split_placeholders(t) {
                    match piece {
                        Piece::Text(s) => out_ev.push(Event::Text(s.to_string().into())),
                        Piece::Elem(idx) => {
                            out_ev.push(Event::Html(ctx.element_html(idx, &mut mark_open).into()))
                        }
                    }
                }
            }
            Event::Code(t) => out_ev.push(Event::Code(ctx.restore(t).into())),
            Event::InlineMath(t) => out_ev.push(Event::Html(
                format!(
                    "<span class=\"math\">{}</span>",
                    escape_html(&ctx.restore(t))
                )
                .into(),
            )),
            Event::DisplayMath(t) => out_ev.push(Event::Html(
                format!("<div class=\"math\">{}</div>", escape_html(&ctx.restore(t))).into(),
            )),
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                out_ev.push(ev.clone());
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out_ev.push(ev.clone());
            }
            Event::Start(Tag::HtmlBlock) => {
                let mut raw = String::new();
                while i < events.len() {
                    match &events[i] {
                        Event::End(TagEnd::HtmlBlock) => break,
                        Event::Html(h) | Event::InlineHtml(h) | Event::Text(h) => raw.push_str(h),
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                out_ev.push(Event::Html(ctx.sanitize_html(&raw, &mut mark_open).into()));
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                out_ev.push(Event::Html(ctx.sanitize_html(h, &mut mark_open).into()));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let mut raw = String::new();
                let mut j = i;
                while j < events.len() {
                    match &events[j] {
                        Event::End(TagEnd::Heading(_)) => break,
                        Event::Text(t) => raw.push_str(&ctx.plain_text(t)),
                        Event::Code(t) | Event::InlineMath(t) => raw.push_str(&ctx.restore(t)),
                        Event::SoftBreak | Event::HardBreak => raw.push(' '),
                        _ => {}
                    }
                    j += 1;
                }
                let base = slugify(&crate::scan::clean_heading_text(&raw));
                let n = slugs.entry(base.clone()).or_insert(0);
                let slug = if *n == 0 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                };
                *n += 1;
                out_ev.push(Event::Html(
                    format!("<{level} id=\"{}\">", escape_html(&slug)).into(),
                ));
            }
            Event::End(TagEnd::Heading(level)) => {
                close_mark(&mut out_ev, &mut mark_open);
                out_ev.push(Event::Html(format!("</{level}>\n").into()));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let dest = dest_url.as_ref();
                let external =
                    matches!(link_type, LinkType::Autolink | LinkType::Email) || is_external(dest);
                if dest.is_empty() && !external {
                    link_stack.push(false);
                    out_ev.push(ev.clone());
                    continue;
                }
                let mut a_tag = String::new();
                if external {
                    let href = if *link_type == LinkType::Email {
                        format!("mailto:{dest}")
                    } else {
                        dest.to_string()
                    };
                    let href = if url_is_safe(&href) { href } else { "#".into() };
                    a_tag.push_str("<a class=\"external-link\" href=\"");
                    push_escaped(&mut a_tag, &href);
                    a_tag.push_str("\" target=\"_blank\" rel=\"noopener nofollow\"");
                } else {
                    let (path_raw, sub_raw) = match dest.find('#') {
                        Some(h) => (&dest[..h], Some(&dest[h + 1..])),
                        None => (dest, None),
                    };
                    let target = percent_decode(path_raw);
                    let sub = sub_raw.and_then(|s| crate::scan::parse_subpath(&percent_decode(s)));
                    let (href, ok) = ctx.href(&target, sub.as_ref());
                    a_tag.push_str(if ok {
                        "<a class=\"internal-link\" href=\""
                    } else {
                        "<a class=\"internal-link is-unresolved\" href=\""
                    });
                    push_escaped(&mut a_tag, &href);
                    a_tag.push_str("\" data-href=\"");
                    push_escaped(&mut a_tag, &percent_decode(dest));
                    a_tag.push('"');
                }
                if !title.is_empty() {
                    a_tag.push_str(" title=\"");
                    push_escaped(&mut a_tag, title);
                    a_tag.push('"');
                }
                a_tag.push('>');
                link_stack.push(true);
                out_ev.push(Event::Html(a_tag.into()));
            }
            Event::End(TagEnd::Link) => {
                if link_stack.pop() == Some(true) {
                    out_ev.push(Event::Html("</a>".into()));
                } else {
                    out_ev.push(ev.clone());
                }
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = dest_url.as_ref();
                let new_dest = if is_external(dest) {
                    if url_is_safe(dest) || dest.starts_with("data:image/") {
                        dest.to_string()
                    } else {
                        String::new()
                    }
                } else if dest.is_empty() {
                    String::new()
                } else {
                    let path_raw = dest.split('#').next().unwrap_or("");
                    ctx.href_path(&percent_decode(path_raw)).0
                };
                out_ev.push(Event::Start(Tag::Image {
                    link_type: *link_type,
                    dest_url: new_dest.into(),
                    title: title.clone(),
                    id: id.clone(),
                }));
            }
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Item
                | TagEnd::TableCell
                | TagEnd::BlockQuote(_)
                | TagEnd::FootnoteDefinition,
            ) => {
                close_mark(&mut out_ev, &mut mark_open);
                out_ev.push(ev.clone());
            }
            _ => out_ev.push(ev.clone()),
        }
    }
    close_mark(&mut out_ev, &mut mark_open);

    let mut html = String::new();
    if opts.show_frontmatter {
        html.push_str(&ctx.properties_table());
    }
    pulldown_cmark::html::push_html(&mut html, out_ev.into_iter());
    html
}

fn close_mark(out: &mut Vec<Event<'_>>, mark_open: &mut bool) {
    if *mark_open {
        out.push(Event::Html("</mark>".into()));
        *mark_open = false;
    }
}

/// Reject `javascript:`, `vbscript:` and `data:` URLs.
fn url_is_safe(url: &str) -> bool {
    let cleaned: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    !(cleaned.starts_with("javascript:")
        || cleaned.starts_with("vbscript:")
        || cleaned.starts_with("data:"))
}
