//! YAML frontmatter: detection, YAML → JSON conversion, aliases, tags and
//! wikilink-valued properties.

use std::collections::HashMap;

use ors_core::{Frontmatter, Link, LinkKind, Span, Tag};
use serde_json::Value as Json;
use serde_norway::Value as Yaml;

use crate::scan::{LinkRec, parse_wiki_inner};
use crate::util::{LineIndex, find_in};

/// Byte layout of a frontmatter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontmatterBlock {
    /// Start of the opening `---` (after an optional UTF-8 BOM).
    pub start: usize,
    /// Start of the YAML text (after the opening fence line).
    pub yaml_start: usize,
    /// End of the YAML text (start of the closing fence line).
    pub yaml_end: usize,
    /// End of the block: just after the closing fence line (and its newline).
    pub end: usize,
}

/// Locate the frontmatter block: it must start on the first line (an optional
/// BOM is allowed) with a line that is exactly `---` and end at the next line
/// that is exactly `---` (trailing whitespace and `\r` are tolerated).
/// Without a closing fence there is no frontmatter.
pub fn split_frontmatter(source: &str) -> Option<FrontmatterBlock> {
    let start = if source.starts_with('\u{FEFF}') { 3 } else { 0 };
    let bytes = source.as_bytes();
    let first_end = find_in(bytes, start, bytes.len(), b"\n")?;
    if source[start..first_end].trim_end() != "---" {
        return None;
    }
    let yaml_start = first_end + 1;
    let mut line_start = yaml_start;
    while line_start <= bytes.len() {
        let line_end = find_in(bytes, line_start, bytes.len(), b"\n").unwrap_or(bytes.len());
        if source[line_start..line_end].trim_end() == "---" {
            let end = if line_end < bytes.len() {
                line_end + 1
            } else {
                line_end
            };
            return Some(FrontmatterBlock {
                start,
                yaml_start,
                yaml_end: line_start,
                end,
            });
        }
        if line_end >= bytes.len() {
            break;
        }
        line_start = line_end + 1;
    }
    None
}

pub(crate) struct FrontmatterResult {
    pub block: FrontmatterBlock,
    pub map: Option<Frontmatter>,
    pub aliases: Vec<String>,
    pub tags: Vec<Tag>,
    pub links: Vec<LinkRec>,
}

/// Convert a YAML value into JSON. Non-string keys are stringified, tags are
/// dropped, non-finite floats become `null`.
pub(crate) fn yaml_to_json(v: &Yaml) -> Json {
    match v {
        Yaml::Null => Json::Null,
        Yaml::Bool(b) => Json::Bool(*b),
        Yaml::Number(n) => {
            if let Some(i) = n.as_i64() {
                Json::from(i)
            } else if let Some(u) = n.as_u64() {
                Json::from(u)
            } else {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Json::Null, Json::Number)
            }
        }
        Yaml::String(s) => Json::String(s.clone()),
        Yaml::Sequence(seq) => Json::Array(seq.iter().map(yaml_to_json).collect()),
        Yaml::Mapping(m) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in m {
                obj.insert(yaml_key(k), yaml_to_json(v));
            }
            Json::Object(obj)
        }
        Yaml::Tagged(t) => yaml_to_json(&t.value),
    }
}

fn yaml_key(k: &Yaml) -> String {
    match k {
        Yaml::String(s) => s.clone(),
        Yaml::Null => "null".into(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(n) => n.to_string(),
        Yaml::Tagged(t) => yaml_key(&t.value),
        other => yaml_to_json(other).to_string(),
    }
}

/// Parse the YAML text of a frontmatter block into a JSON object.
/// Returns `None` for invalid YAML or a non-mapping document; an empty
/// document gives an empty object.
pub fn parse_yaml(yaml: &str) -> Option<Frontmatter> {
    if yaml.trim().is_empty() {
        return Some(Frontmatter::new());
    }
    match serde_norway::from_str::<Yaml>(yaml) {
        Ok(Yaml::Null) => Some(Frontmatter::new()),
        Ok(v) => match yaml_to_json(&v) {
            Json::Object(m) => Some(m),
            _ => None,
        },
        Err(_) => None,
    }
}

fn get_ci<'a>(map: &'a Frontmatter, keys: &[&str]) -> Option<(&'a str, &'a Json)> {
    for want in keys {
        for (k, v) in map {
            if k.eq_ignore_ascii_case(want) {
                return Some((k.as_str(), v));
            }
        }
    }
    None
}

fn scalar_string(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        Json::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Aliases from `aliases`/`alias`: a string (comma separated) or a list.
pub(crate) fn aliases_of(map: &Frontmatter) -> Vec<String> {
    let mut out = Vec::new();
    if let Some((_, v)) = get_ci(map, &["aliases", "alias"]) {
        match v {
            Json::Array(items) => {
                for it in items {
                    if let Some(s) = scalar_string(it) {
                        let s = s.trim();
                        if !s.is_empty() {
                            out.push(s.to_string());
                        }
                    }
                }
            }
            other => {
                if let Some(s) = scalar_string(other) {
                    out.extend(
                        s.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                    );
                }
            }
        }
    }
    out
}

/// Tag names from `tags`/`tag`: a string (comma or space separated) or a
/// list. Leading `#` are stripped; purely numeric names are dropped.
pub(crate) fn tag_names_of(map: &Frontmatter) -> (Option<String>, Vec<String>) {
    let mut out = Vec::new();
    let Some((key, v)) = get_ci(map, &["tags", "tag"]) else {
        return (None, out);
    };
    let mut push_str = |s: &str| {
        for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
            let name = part.trim_start_matches('#');
            if crate::scan::is_valid_tag_name(name) {
                out.push(name.to_string());
            }
        }
    };
    match v {
        Json::Array(items) => {
            for it in items {
                if let Some(s) = scalar_string(it) {
                    push_str(&s);
                }
            }
        }
        other => {
            if let Some(s) = scalar_string(other) {
                push_str(&s);
            }
        }
    }
    (Some(key.to_string()), out)
}

/// Collect every string property value that is exactly a wikilink
/// (`"[[Target]]"`), including inside lists and nested objects. The unquoted
/// YAML form `key: [[Target]]` (a nested flow list) is recognised too.
fn collect_link_values(v: &Json, out: &mut Vec<String>) {
    match v {
        Json::String(s) => {
            let t = s.trim();
            if is_whole_wikilink(t) {
                out.push(t.to_string());
            }
        }
        Json::Array(items) => {
            if let [Json::Array(inner)] = items.as_slice()
                && let [Json::String(s)] = inner.as_slice()
                && !s.contains(['[', ']'])
                && !s.trim().is_empty()
            {
                out.push(format!("[[{s}]]"));
                return;
            }
            for it in items {
                collect_link_values(it, out);
            }
        }
        Json::Object(m) => {
            for v in m.values() {
                collect_link_values(v, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn is_whole_wikilink(t: &str) -> bool {
    t.len() > 4 && t.starts_with("[[") && t.ends_with("]]") && !t[2..t.len() - 2].contains("]]")
}

pub(crate) fn extract(source: &str, lines: &LineIndex) -> Option<FrontmatterResult> {
    let block = split_frontmatter(source)?;
    let yaml = &source[block.yaml_start..block.yaml_end];
    let map = parse_yaml(yaml);
    let mut res = FrontmatterResult {
        block,
        map: None,
        aliases: Vec::new(),
        tags: Vec::new(),
        links: Vec::new(),
    };
    let Some(map) = map else {
        return Some(res);
    };
    let fm_span = Span::new(block.start, block.end);
    let bytes = source.as_bytes();

    res.aliases = aliases_of(&map);

    // Tags, with spans located textually after the `tags:` key line.
    let (key, names) = tag_names_of(&map);
    if let Some(key) = key {
        let key_pos = find_key_line(source, block.yaml_start, block.yaml_end, &key);
        let key_line_span = key_pos.map_or(fm_span, |p| {
            let e = find_in(bytes, p, block.yaml_end, b"\n").unwrap_or(block.yaml_end);
            Span::new(p, e)
        });
        // Start after the key itself so `tags` cannot match a tag named `a`.
        let mut cursor = key_pos.map_or(block.yaml_start, |p| {
            find_in(bytes, p, key_line_span.end.max(p + 1), b":").map_or(p, |c| c + 1)
        });
        for name in names {
            let span = match find_tag_token(source, cursor, block.yaml_end, &name) {
                Some(p) => {
                    cursor = p + name.len();
                    Span::new(p, p + name.len())
                }
                None => key_line_span,
            };
            res.tags.push(Tag {
                name,
                span,
                line: lines.line_of(span.start),
                from_frontmatter: true,
            });
        }
    }

    // Wikilink-valued properties.
    let mut wanted = Vec::new();
    for v in map.values() {
        collect_link_values(v, &mut wanted);
    }
    if !wanted.is_empty() {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for w in &wanted {
            *counts.entry(w.clone()).or_default() += 1;
        }
        let mut found: Vec<(usize, usize)> = Vec::new();
        let mut p = block.yaml_start;
        while let Some(open) = find_in(bytes, p, block.yaml_end, b"[[") {
            let line_end = find_in(bytes, open, block.yaml_end, b"\n").unwrap_or(block.yaml_end);
            let Some(close) = find_in(bytes, open + 2, line_end, b"]]") else {
                p = open + 2;
                continue;
            };
            let end = close + 2;
            let text = &source[open..end];
            if let Some(c) = counts.get_mut(text)
                && *c > 0
            {
                *c -= 1;
                found.push((open, end));
            }
            p = end;
        }
        for (open, end) in found {
            if let Some(w) = parse_wiki_inner(source, open + 2, end - 2) {
                let span = Span::new(open, end);
                res.links.push(LinkRec {
                    link: Link {
                        target: w.target,
                        subpath: w.subpath,
                        display: w.display,
                        kind: LinkKind::Frontmatter,
                        is_embed: false,
                        span,
                        line: lines.line_of(open),
                    },
                    target_range: Some(w.target_range),
                    raw: w.raw,
                });
            }
        }
        // Values whose text could not be located (e.g. YAML escapes).
        for (text, n) in counts {
            for _ in 0..n {
                if let Some(w) = parse_wiki_inner(&text, 2, text.len() - 2) {
                    res.links.push(LinkRec {
                        link: Link {
                            target: w.target,
                            subpath: w.subpath,
                            display: w.display,
                            kind: LinkKind::Frontmatter,
                            is_embed: false,
                            span: fm_span,
                            line: lines.line_of(fm_span.start),
                        },
                        target_range: None,
                        raw: w.raw,
                    });
                }
            }
        }
        res.links.sort_by_key(|l| l.link.span.start);
    }

    res.map = Some(map);
    Some(res)
}

/// First occurrence of `name` in `source[from..to]` as a whole tag token (not
/// preceded or followed by a tag character).
fn find_tag_token(source: &str, from: usize, to: usize, name: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut p = from;
    while let Some(i) = find_in(bytes, p, to, name.as_bytes()) {
        let end = i + name.len();
        let before_ok = !source[..i]
            .chars()
            .next_back()
            .is_some_and(crate::scan::is_tag_char);
        let after_ok = !source[end..]
            .chars()
            .next()
            .is_some_and(crate::scan::is_tag_char);
        if before_ok && after_ok {
            return Some(i);
        }
        p = i + 1;
        while p < to && !source.is_char_boundary(p) {
            p += 1;
        }
    }
    None
}

/// Position of a top-level `key:` line in the YAML text.
fn find_key_line(source: &str, start: usize, end: usize, key: &str) -> Option<usize> {
    let mut line_start = start;
    while line_start < end {
        let line_end = find_in(source.as_bytes(), line_start, end, b"\n").unwrap_or(end);
        let line = &source[line_start..line_end];
        let unquoted = line.trim_start_matches(['"', '\'']);
        if unquoted.len() > key.len()
            && unquoted.is_char_boundary(key.len())
            && unquoted[..key.len()].eq_ignore_ascii_case(key)
        {
            let rest = unquoted[key.len()..].trim_start_matches(['"', '\'']);
            if rest.starts_with(':') {
                return Some(line_start);
            }
        }
        line_start = line_end + 1;
    }
    None
}
