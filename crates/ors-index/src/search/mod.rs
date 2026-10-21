//! Full-text search with Obsidian's query syntax.
//!
//! The index keeps, for every note, its raw source (tokenized here, not via
//! the markdown parser), a case-folded copy, line / section / block / task
//! ranges, and a vocabulary of alphanumeric tokens used to prune candidate
//! documents before the exact evaluation of the query AST.

pub mod query;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use ors_core::{Error, NotePath, Result, VaultRead};
use regex::{Regex, RegexBuilder};

pub use query::{Operator, Query, QueryError, parse_query};

use crate::cache::tag_key;
use crate::text::{self, Folded, key};
use crate::{MetadataCache, SearchMatch, SearchResult};

/// Longest `line_text` returned in a [`SearchMatch`] (bytes, approximately).
const MAX_LINE_LEN: usize = 300;
/// Score bonus when a term matches the note's file name.
const TITLE_BONUS: u32 = 10;

static TASK_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d+[.)])\s+\[(.)\](?:\s+|$)").ok());

#[derive(Debug, Clone, Copy)]
struct TaskRange {
    /// Byte range of the task text (after the checkbox).
    start: usize,
    end: usize,
    status: char,
}

/// One indexed note.
#[derive(Debug, Clone)]
struct Doc {
    path: NotePath,
    source: String,
    folded: Folded,
    line_starts: Vec<usize>,
    file_name: String,
    file_name_folded: String,
    path_str: String,
    path_folded: String,
    sections: Vec<(usize, usize)>,
    blocks: Vec<(usize, usize)>,
    tasks: Vec<TaskRange>,
    tokens: Vec<String>,
}

impl Doc {
    fn new(path: NotePath, source: &str) -> Self {
        let folded = Folded::new(source);
        let line_starts = text::line_starts(source);
        let (sections, blocks, tasks) = structure(source, &line_starts);
        let mut tokens: Vec<String> = folded
            .text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        tokens.sort();
        tokens.dedup();
        Self {
            file_name: path.file_name().to_string(),
            file_name_folded: Folded::new(path.file_name()).text,
            path_str: path.as_str().to_string(),
            path_folded: Folded::new(path.as_str()).text,
            path,
            source: source.to_string(),
            folded,
            line_starts,
            sections,
            blocks,
            tasks,
            tokens,
        }
    }

    /// Byte range of line `i` without the line terminator.
    fn line_range(&self, i: usize) -> (usize, usize) {
        line_bounds(&self.source, &self.line_starts, i)
    }

    fn lines_in(&self, s: usize, e: usize) -> Vec<(usize, usize)> {
        let first = text::line_of(&self.line_starts, s);
        let mut out = Vec::new();
        for i in first..self.line_starts.len() {
            let (ls, le) = self.line_range(i);
            if ls > e || (ls == e && e != s) {
                break;
            }
            let (cs, ce) = (ls.max(s), le.min(e));
            if cs <= ce {
                out.push((cs, ce));
            }
        }
        out
    }
}

fn line_bounds(source: &str, starts: &[usize], i: usize) -> (usize, usize) {
    let s = starts.get(i).copied().unwrap_or(source.len());
    let mut e = starts.get(i + 1).map_or(source.len(), |n| n - 1);
    if e > s && source.as_bytes().get(e - 1) == Some(&b'\r') {
        e -= 1;
    }
    (s, e.max(s))
}

/// Compute heading sections, blocks (paragraphs) and tasks of a source.
#[allow(clippy::type_complexity)]
fn structure(
    source: &str,
    starts: &[usize],
) -> (Vec<(usize, usize)>, Vec<(usize, usize)>, Vec<TaskRange>) {
    let mut headings = Vec::new();
    let mut blocks = Vec::new();
    let mut tasks = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut block_start: Option<usize> = None;
    let mut block_end = 0;
    let close_block = |blocks: &mut Vec<(usize, usize)>, bs: &mut Option<usize>, be: usize| {
        if let Some(s) = bs.take() {
            blocks.push((s, be));
        }
    };
    for i in 0..starts.len() {
        let (ls, le) = line_bounds(source, starts, i);
        let line = &source[ls..le];
        let trimmed = line.trim_start();
        let fence_marker = ['`', '~'].into_iter().find_map(|ch| {
            let n = trimmed.chars().take_while(|&c| c == ch).count();
            (n >= 3).then_some((ch, n))
        });
        if let Some((ch, n)) = fence {
            if fence_marker.is_some_and(|(c, m)| c == ch && m >= n) {
                fence = None;
            }
            block_end = le;
            continue;
        }
        if let Some(m) = fence_marker {
            fence = Some(m);
            if block_start.is_none() {
                block_start = Some(ls);
            }
            block_end = le;
            continue;
        }
        if line.trim().is_empty() {
            close_block(&mut blocks, &mut block_start, block_end);
            continue;
        }
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        let is_heading = (1..=6).contains(&hashes)
            && trimmed[hashes..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
        if is_heading {
            close_block(&mut blocks, &mut block_start, block_end);
            headings.push(ls);
            blocks.push((ls, le));
            continue;
        }
        if let Some(re) = TASK_RE.as_ref()
            && let Some(caps) = re.captures(line)
            && let (Some(all), Some(st)) = (caps.get(0), caps.get(1))
        {
            let status = st.as_str().chars().next().unwrap_or(' ');
            tasks.push(TaskRange {
                start: ls + all.end(),
                end: le,
                status,
            });
        }
        if block_start.is_none() {
            block_start = Some(ls);
        }
        block_end = le;
    }
    close_block(&mut blocks, &mut block_start, block_end);
    let mut sections = Vec::new();
    let mut prev = 0;
    for &h in &headings {
        if h > prev {
            sections.push((prev, h));
        }
        prev = h;
    }
    if prev < source.len() || sections.is_empty() {
        sections.push((prev, source.len()));
    }
    (sections, blocks, tasks)
}

/// Compiled query node.
#[derive(Debug)]
enum Node {
    Text {
        raw: String,
        folded: String,
    },
    Regex {
        ci: Regex,
        cs: Regex,
    },
    Tag(String),
    Property {
        name: String,
        value: Option<Box<Node>>,
    },
    Scoped(Operator, Box<Node>),
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
}

fn compile(q: &Query) -> std::result::Result<Node, QueryError> {
    Ok(match q {
        Query::Term(s) | Query::Phrase(s) => Node::Text {
            raw: s.clone(),
            folded: Folded::new(s).text,
        },
        Query::Regex(r) => {
            let build = |ci: bool| {
                RegexBuilder::new(r)
                    .case_insensitive(ci)
                    .multi_line(true)
                    .build()
                    .map_err(|e| QueryError {
                        message: format!("invalid regex: {e}"),
                        position: 0,
                    })
            };
            Node::Regex {
                ci: build(true)?,
                cs: build(false)?,
            }
        }
        Query::Tag(t) => Node::Tag(tag_key(t)),
        Query::Property { name, value } => Node::Property {
            name: key(name.trim()),
            value: match value {
                Some(v) => Some(Box::new(compile(v)?)),
                None => None,
            },
        },
        Query::Scoped(op, q) => Node::Scoped(*op, Box::new(compile(q)?)),
        Query::And(v) => Node::And(
            v.iter()
                .map(compile)
                .collect::<std::result::Result<_, _>>()?,
        ),
        Query::Or(v) => Node::Or(
            v.iter()
                .map(compile)
                .collect::<std::result::Result<_, _>>()?,
        ),
        Query::Not(q) => Node::Not(Box::new(compile(q)?)),
    })
}

/// Where a node is evaluated.
#[derive(Clone, Copy)]
enum Scope<'a> {
    /// Whole note: file name + content.
    Doc,
    /// A byte range of the content.
    Range(usize, usize),
    /// A standalone string (file name, path, property value): no hits.
    Text { orig: &'a str, folded: &'a str },
}

struct Eval<'a> {
    doc: &'a Doc,
    cache: &'a MetadataCache,
}

/// Per-evaluation output: hit ranges (original byte offsets) and whether a
/// term matched the file name.
#[derive(Default)]
struct Out {
    hits: Vec<(usize, usize)>,
    title: bool,
}

impl Eval<'_> {
    fn eval(&self, node: &Node, scope: Scope<'_>, cs: bool, out: &mut Out) -> bool {
        match node {
            Node::Text { raw, folded } => self.text(raw, folded, scope, cs, out),
            Node::Regex { ci, cs: csr } => self.regex(if cs { csr } else { ci }, scope, out),
            Node::Tag(k) => self.tag(k, scope, out),
            Node::Property { name, value } => self.property(name, value.as_deref(), cs, out),
            Node::Scoped(op, child) => self.scoped(*op, child, scope, cs, out),
            Node::And(children) => {
                let mut local = Out::default();
                for c in children {
                    if !self.eval(c, scope, cs, &mut local) {
                        return false;
                    }
                }
                out.hits.append(&mut local.hits);
                out.title |= local.title;
                true
            }
            Node::Or(children) => {
                let mut any = false;
                for c in children {
                    any |= self.eval(c, scope, cs, out);
                }
                any
            }
            Node::Not(child) => !self.eval(child, scope, cs, &mut Out::default()),
        }
    }

    fn content_range(&self, scope: Scope<'_>) -> Option<(usize, usize)> {
        match scope {
            Scope::Doc => Some((0, self.doc.source.len())),
            Scope::Range(s, e) => Some((s, e)),
            Scope::Text { .. } => None,
        }
    }

    fn text(&self, raw: &str, folded: &str, scope: Scope<'_>, cs: bool, out: &mut Out) -> bool {
        if raw.is_empty() {
            return true;
        }
        let contains = |orig: &str, f: &str| {
            if cs {
                orig.contains(raw)
            } else {
                f.contains(folded)
            }
        };
        match scope {
            Scope::Text { orig, folded: f } => contains(orig, f),
            Scope::Doc | Scope::Range(..) => {
                let Some((s, e)) = self.content_range(scope) else {
                    return false;
                };
                let found = self.find_in_content(raw, folded, s, e, cs, out);
                let title = matches!(scope, Scope::Doc)
                    && contains(&self.doc.file_name, &self.doc.file_name_folded);
                out.title |= title;
                found || title
            }
        }
    }

    fn find_in_content(
        &self,
        raw: &str,
        folded: &str,
        s: usize,
        e: usize,
        cs: bool,
        out: &mut Out,
    ) -> bool {
        let mut found = false;
        if cs {
            let hay = &self.doc.source[s..e];
            for (i, m) in hay.match_indices(raw) {
                out.hits.push((s + i, s + i + m.len()));
                found = true;
            }
        } else {
            let f = &self.doc.folded;
            let (fs, fe) = (f.to_folded(s), f.to_folded(e));
            let Some(hay) = f.text.get(fs..fe) else {
                return false;
            };
            for (i, m) in hay.match_indices(folded) {
                let (a, b) = (fs + i, fs + i + m.len());
                out.hits.push((f.to_original(a), f.to_original(b)));
                found = true;
            }
        }
        found
    }

    fn regex(&self, re: &Regex, scope: Scope<'_>, out: &mut Out) -> bool {
        match scope {
            Scope::Text { orig, .. } => re.is_match(orig),
            Scope::Doc | Scope::Range(..) => {
                let Some((s, e)) = self.content_range(scope) else {
                    return false;
                };
                let mut found = false;
                if let Some(hay) = self.doc.source.get(s..e) {
                    for m in re.find_iter(hay) {
                        found = true;
                        if !m.is_empty() {
                            out.hits.push((s + m.start(), s + m.end()));
                        }
                    }
                }
                let title = matches!(scope, Scope::Doc) && re.is_match(&self.doc.file_name);
                out.title |= title;
                found || title
            }
        }
    }

    fn tag(&self, k: &str, scope: Scope<'_>, out: &mut Out) -> bool {
        let Some(note) = self.cache.get(&self.doc.path) else {
            return false;
        };
        let (lo, hi) = match scope {
            Scope::Range(s, e) => (s, e.max(s + 1)),
            _ => (0, usize::MAX),
        };
        let mut found = false;
        for t in &note.tags {
            let tk = tag_key(&t.name);
            let hit = tk == k || tk.starts_with(&format!("{k}/"));
            if hit && t.span.start >= lo && t.span.start < hi {
                found = true;
                if t.span.start < t.span.end && t.span.end <= self.doc.source.len() {
                    out.hits.push((t.span.start, t.span.end));
                }
            }
        }
        found
    }

    /// Byte range of the `name:` line in the frontmatter, if present.
    fn property_line(&self, name: &str) -> Option<(usize, usize)> {
        let span = self.cache.get(&self.doc.path)?.frontmatter_span?;
        let src = &self.doc.source;
        let first = text::line_of(&self.doc.line_starts, span.start);
        for i in first..self.doc.line_starts.len() {
            let (ls, le) = self.doc.line_range(i);
            if ls >= span.end {
                break;
            }
            let line = &src[ls..le];
            if line.starts_with(char::is_whitespace) || line.starts_with("---") {
                continue;
            }
            let Some((k, _)) = line.split_once(':') else {
                continue;
            };
            if key(k.trim().trim_matches(['"', '\''])) == name {
                return Some((ls, ls + line.trim_end().len()));
            }
        }
        None
    }

    fn property(&self, name: &str, value: Option<&Node>, cs: bool, out: &mut Out) -> bool {
        let Some(fm) = self
            .cache
            .get(&self.doc.path)
            .and_then(|n| n.frontmatter.as_ref())
        else {
            return false;
        };
        let Some(v) = fm
            .iter()
            .find(|(k, _)| key(k.trim()) == name)
            .map(|(_, v)| v)
        else {
            return false;
        };
        let mut matched = true;
        if let Some(value) = value {
            let mut strings = Vec::new();
            flatten_value(v, &mut strings);
            matched = strings.iter().any(|s| {
                let f = Folded::new(s).text;
                self.eval(
                    value,
                    Scope::Text {
                        orig: s,
                        folded: &f,
                    },
                    cs,
                    &mut Out::default(),
                )
            });
        }
        if matched && let Some(r) = self.property_line(name) {
            out.hits.push(r);
        }
        matched
    }

    fn sub_ranges(
        &self,
        scope: Scope<'_>,
        ranges: impl Iterator<Item = (usize, usize)>,
    ) -> Vec<(usize, usize)> {
        let Some((s, e)) = self.content_range(scope) else {
            return Vec::new();
        };
        ranges
            .filter_map(|(a, b)| {
                let (cs, ce) = (a.max(s), b.min(e));
                (cs < ce || (cs == ce && a == b && a >= s && a <= e)).then_some((cs, ce))
            })
            .collect()
    }

    fn any_range(
        &self,
        child: &Node,
        ranges: Vec<(usize, usize)>,
        cs: bool,
        out: &mut Out,
    ) -> bool {
        let mut any = false;
        for (s, e) in ranges {
            let mut local = Out::default();
            if self.eval(child, Scope::Range(s, e), cs, &mut local) {
                any = true;
                out.hits.append(&mut local.hits);
            }
        }
        any
    }

    fn scoped(
        &self,
        op: Operator,
        child: &Node,
        scope: Scope<'_>,
        cs: bool,
        out: &mut Out,
    ) -> bool {
        let d = self.doc;
        match op {
            Operator::File => self.eval(
                child,
                Scope::Text {
                    orig: &d.file_name,
                    folded: &d.file_name_folded,
                },
                cs,
                &mut Out::default(),
            ),
            Operator::Path => self.eval(
                child,
                Scope::Text {
                    orig: &d.path_str,
                    folded: &d.path_folded,
                },
                cs,
                &mut Out::default(),
            ),
            Operator::Content => match scope {
                Scope::Doc => self.eval(child, Scope::Range(0, d.source.len()), cs, out),
                other => self.eval(child, other, cs, out),
            },
            Operator::MatchCase => self.eval(child, scope, true, out),
            Operator::IgnoreCase => self.eval(child, scope, false, out),
            Operator::Line => {
                let Some((s, e)) = self.content_range(scope) else {
                    return false;
                };
                self.any_range(child, d.lines_in(s, e), cs, out)
            }
            Operator::Section => {
                let r = self.sub_ranges(scope, d.sections.iter().copied());
                self.any_range(child, r, cs, out)
            }
            Operator::Block => {
                let r = self.sub_ranges(scope, d.blocks.iter().copied());
                self.any_range(child, r, cs, out)
            }
            Operator::Task | Operator::TaskTodo | Operator::TaskDone => {
                let tasks = d.tasks.iter().filter(|t| match op {
                    Operator::TaskTodo => t.status == ' ',
                    Operator::TaskDone => t.status != ' ',
                    _ => true,
                });
                let r = self.sub_ranges(scope, tasks.map(|t| (t.start, t.end)));
                self.any_range(child, r, cs, out)
            }
        }
    }
}

fn flatten_value(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Null => out.push(String::new()),
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| flatten_value(x, out)),
        other => out.push(other.to_string()),
    }
}

/// `(line, line start, line end, ranges relative to the line)`.
type LineGroup = (usize, usize, usize, Vec<(usize, usize)>);

/// Merge `ranges` (original byte offsets into `source`) and group them by line.
pub(crate) fn group_ranges_by_line(
    source: &str,
    mut ranges: Vec<(usize, usize)>,
) -> Vec<SearchMatch> {
    ranges.retain(|&(s, e)| s < e && e <= source.len());
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    let starts = text::line_starts(source);
    let mut out: Vec<SearchMatch> = Vec::new();
    let mut current: Option<LineGroup> = None;
    let flush = |cur: Option<LineGroup>, out: &mut Vec<SearchMatch>| {
        if let Some((line, ls, le, rs)) = cur {
            out.push(make_match(source, line, ls, le, rs));
        }
    };
    for (s, e) in merged {
        let line = text::line_of(&starts, s);
        let (ls, le) = line_bounds(source, &starts, line);
        let e = e.min(le);
        if s >= e {
            continue;
        }
        match current.as_mut() {
            Some((l, _, _, rs)) if *l == line => rs.push((s - ls, e - ls)),
            _ => {
                flush(current.take(), &mut out);
                current = Some((line, ls, le, vec![(s - ls, e - ls)]));
            }
        }
    }
    flush(current, &mut out);
    out
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn make_match(
    source: &str,
    line: usize,
    ls: usize,
    le: usize,
    ranges: Vec<(usize, usize)>,
) -> SearchMatch {
    let text = &source[ls..le];
    if text.len() <= MAX_LINE_LEN {
        return SearchMatch {
            line,
            line_text: text.to_string(),
            ranges,
        };
    }
    let first = ranges.first().map_or(0, |r| r.0);
    let ws = floor_boundary(text, first.saturating_sub(MAX_LINE_LEN / 5));
    let we = floor_boundary(text, ws + MAX_LINE_LEN);
    let ranges = ranges
        .into_iter()
        .filter(|&(s, e)| e > ws && s < we)
        .map(|(s, e)| (s.max(ws) - ws, e.min(we) - ws))
        .collect();
    SearchMatch {
        line,
        line_text: text[ws..we].to_string(),
        ranges,
    }
}

/// Full-text index of note contents supporting Obsidian's search syntax:
/// words (AND), `"exact phrase"`, `OR`, `-negation`, `( )` grouping,
/// `file:`, `path:`, `content:`, `tag:`, `line:( )`, `section:( )`,
/// `block:( )`, `task:`, `task-todo:`, `task-done:`, `match-case:`,
/// `ignore-case:`, `[property]`, `[property:value]`, `/regex/`.
/// Case-insensitive (accent-sensitive) by default; terms match substrings.
#[derive(Debug, Default, Clone)]
pub struct SearchIndex {
    docs: Vec<Option<Doc>>,
    ids: HashMap<NotePath, usize>,
    free: Vec<usize>,
    vocab: HashMap<String, HashSet<usize>>,
}

impl SearchIndex {
    /// Empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Index every markdown note of the vault (read from `vault`; `cache` is
    /// not needed for indexing and only kept for API symmetry).
    pub fn build(vault: &dyn VaultRead, cache: &MetadataCache) -> Result<Self> {
        let _ = cache;
        let mut index = Self::default();
        for path in vault.list_notes()? {
            let src = vault.read_to_string(&path)?;
            index.update(&path, &src);
        }
        Ok(index)
    }

    /// Number of indexed notes.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether no note is indexed.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Whether `path` is indexed.
    pub fn contains(&self, path: &NotePath) -> bool {
        self.ids.contains_key(path)
    }

    /// Index (or re-index) a note from its source.
    pub fn update(&mut self, path: &NotePath, source: &str) {
        self.remove(path);
        let doc = Doc::new(path.clone(), source);
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                self.docs.push(None);
                self.docs.len() - 1
            }
        };
        for t in &doc.tokens {
            self.vocab.entry(t.clone()).or_default().insert(id);
        }
        if let Some(slot) = self.docs.get_mut(id) {
            *slot = Some(doc);
        }
        self.ids.insert(path.clone(), id);
    }

    /// Drop a note from the index.
    pub fn remove(&mut self, path: &NotePath) {
        let Some(id) = self.ids.remove(path) else {
            return;
        };
        if let Some(doc) = self.docs.get_mut(id).and_then(Option::take) {
            for t in &doc.tokens {
                if let Some(set) = self.vocab.get_mut(t) {
                    set.remove(&id);
                    if set.is_empty() {
                        self.vocab.remove(t);
                    }
                }
            }
        }
        self.free.push(id);
    }

    /// Move a note to a new path (contents unchanged).
    pub fn rename(&mut self, from: &NotePath, to: &NotePath) {
        let Some(source) = self
            .ids
            .get(from)
            .and_then(|&id| self.docs.get(id))
            .and_then(Option::as_ref)
            .map(|d| d.source.clone())
        else {
            return;
        };
        self.remove(from);
        self.update(to, &source);
    }

    /// Results sorted by score (desc) then path. Invalid queries return
    /// [`Error::Parse`]; an empty query returns no result.
    pub fn search(&self, query: &str, cache: &MetadataCache) -> Result<Vec<SearchResult>> {
        let q = parse_query(query).map_err(|e| Error::Parse(e.to_string()))?;
        self.search_query(&q, cache)
    }

    /// Run an already-parsed query.
    pub fn search_query(&self, q: &Query, cache: &MetadataCache) -> Result<Vec<SearchResult>> {
        if matches!(q, Query::And(v) if v.is_empty()) {
            return Ok(Vec::new());
        }
        let node = compile(q).map_err(|e| Error::Parse(e.to_string()))?;
        let candidates: Box<dyn Iterator<Item = &Doc>> = match self.candidates(&node, cache) {
            Some(ids) => Box::new(
                ids.into_iter()
                    .filter_map(|id| self.docs.get(id).and_then(Option::as_ref)),
            ),
            None => Box::new(self.docs.iter().flatten()),
        };
        let mut results = Vec::new();
        for doc in candidates {
            let ev = Eval { doc, cache };
            let mut out = Out::default();
            if ev.eval(&node, Scope::Doc, false, &mut out) {
                let matches = group_ranges_by_line(&doc.source, out.hits);
                let hits: usize = matches.iter().map(|m| m.ranges.len()).sum();
                let base = u32::try_from(hits).unwrap_or(u32::MAX).max(1);
                let score = base.saturating_add(if out.title { TITLE_BONUS } else { 0 });
                results.push(SearchResult {
                    path: doc.path.clone(),
                    score,
                    matches,
                });
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        Ok(results)
    }

    /// Conservative candidate set (`None` = every document).
    fn candidates(&self, node: &Node, cache: &MetadataCache) -> Option<BTreeSet<usize>> {
        match node {
            Node::Text { folded, .. } => {
                let runs: Vec<&str> = folded
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|r| !r.is_empty())
                    .collect();
                if runs.is_empty() {
                    return None;
                }
                let mut acc: Option<BTreeSet<usize>> = None;
                for run in runs {
                    let mut docs = BTreeSet::new();
                    for (tok, ids) in &self.vocab {
                        if tok.contains(run) {
                            docs.extend(ids.iter().copied());
                        }
                    }
                    acc = Some(match acc {
                        None => docs,
                        Some(a) => a.intersection(&docs).copied().collect(),
                    });
                }
                let mut set = acc.unwrap_or_default();
                // File-name matches.
                for (id, doc) in self.docs.iter().enumerate() {
                    if doc
                        .as_ref()
                        .is_some_and(|d| d.file_name_folded.contains(folded.as_str()))
                    {
                        set.insert(id);
                    }
                }
                Some(set)
            }
            Node::Tag(k) => Some(
                cache
                    .files_with_tag(k)
                    .iter()
                    .filter_map(|p| self.ids.get(p).copied())
                    .collect(),
            ),
            Node::And(children) => {
                let mut acc: Option<BTreeSet<usize>> = None;
                for c in children {
                    if let Some(s) = self.candidates(c, cache) {
                        acc = Some(match acc {
                            None => s,
                            Some(a) => a.intersection(&s).copied().collect(),
                        });
                    }
                }
                acc
            }
            Node::Or(children) => {
                let mut acc = BTreeSet::new();
                for c in children {
                    acc.extend(self.candidates(c, cache)?);
                }
                Some(acc)
            }
            Node::Scoped(op, child) => match op {
                Operator::File | Operator::Path => None,
                _ => self.candidates(child, cache),
            },
            Node::Regex { .. } | Node::Not(_) | Node::Property { .. } => None,
        }
    }
}
