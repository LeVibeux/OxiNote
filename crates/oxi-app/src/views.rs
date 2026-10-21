//! State and logic of the "views" (graph settings, search pane options,
//! backlinks / outgoing links / outline panes, reading view options) that
//! does not depend on egui, plus the text edits they trigger (linking an
//! unlinked mention, toggling a task from the reading view).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use oxi_core::{NotePath, ParsedNote};
use oxi_index::{Operator, Query, SearchResult};

use crate::graph_model::{GRAPH_FILE, GraphSettings, GraphStore, GraphTab, LOCAL_GRAPH_FILE};
use crate::highlight::highlight;
use crate::state::{AppState, OpenIn, Tab};

/// Sort orders of the search and backlinks panes (Obsidian's menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    NameAsc,
    NameDesc,
    ModifiedNewest,
    ModifiedOldest,
    CreatedNewest,
    CreatedOldest,
}

impl SortOrder {
    pub const ALL: [SortOrder; 6] = [
        SortOrder::NameAsc,
        SortOrder::NameDesc,
        SortOrder::ModifiedNewest,
        SortOrder::ModifiedOldest,
        SortOrder::CreatedNewest,
        SortOrder::CreatedOldest,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::NameAsc => "File name (A to Z)",
            SortOrder::NameDesc => "File name (Z to A)",
            SortOrder::ModifiedNewest => "Modified time (new to old)",
            SortOrder::ModifiedOldest => "Modified time (old to new)",
            SortOrder::CreatedNewest => "Created time (new to old)",
            SortOrder::CreatedOldest => "Created time (old to new)",
        }
    }
}

/// Sort `items` by their path according to `order`; `meta` gives
/// (modified, created) times.
pub fn sort_by_path<T>(
    items: &mut [T],
    path: impl Fn(&T) -> &NotePath,
    order: SortOrder,
    meta: impl Fn(&NotePath) -> Option<(SystemTime, SystemTime)>,
) {
    let name = |p: &NotePath| p.stem().to_lowercase();
    match order {
        SortOrder::NameAsc => items.sort_by_cached_key(|t| (name(path(t)), path(t).clone())),
        SortOrder::NameDesc => {
            items.sort_by_cached_key(|t| std::cmp::Reverse((name(path(t)), path(t).clone())))
        }
        SortOrder::ModifiedNewest | SortOrder::ModifiedOldest => {
            items.sort_by_cached_key(|t| meta(path(t)).map(|m| m.0));
            if order == SortOrder::ModifiedNewest {
                items.reverse();
            }
        }
        SortOrder::CreatedNewest | SortOrder::CreatedOldest => {
            items.sort_by_cached_key(|t| meta(path(t)).map(|m| m.1));
            if order == SortOrder::CreatedNewest {
                items.reverse();
            }
        }
    }
}

/// Options of the search pane.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchOptions {
    pub match_case: bool,
    pub explain: bool,
    pub collapse: bool,
    pub more_context: bool,
    pub sort: SortOrder,
    pub show_settings: bool,
    pub show_help: bool,
}

/// The query actually run for `query` with the pane options.
pub fn effective_query(query: &str, opts: &SearchOptions) -> String {
    let q = query.trim();
    if q.is_empty() || !opts.match_case {
        return q.to_string();
    }
    format!("match-case:({q})")
}

/// Human description of a parsed query ("Explain search term"), one entry
/// per line with its indentation depth.
pub fn explain_query(q: &Query) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    explain_into(q, 0, &mut out);
    out
}

fn op_words(op: Operator) -> &'static str {
    match op {
        Operator::File => "File name must match",
        Operator::Path => "File path must match",
        Operator::Content => "File content must match",
        Operator::Line => "Matches on the same line",
        Operator::Section => "Matches in the same section",
        Operator::Block => "Matches in the same block",
        Operator::Task => "Matches in a task",
        Operator::TaskTodo => "Matches in an uncompleted task",
        Operator::TaskDone => "Matches in a completed task",
        Operator::MatchCase => "Case sensitive",
        Operator::IgnoreCase => "Case insensitive",
    }
}

fn explain_into(q: &Query, depth: usize, out: &mut Vec<(usize, String)>) {
    match q {
        Query::Term(t) => out.push((depth, format!("Match text \"{t}\""))),
        Query::Phrase(p) => out.push((depth, format!("Match exact phrase \"{p}\""))),
        Query::Regex(r) => out.push((depth, format!("Match regular expression /{r}/"))),
        Query::Tag(t) => out.push((depth, format!("Has tag #{t}"))),
        Query::Property { name, value: None } => {
            out.push((depth, format!("Has property \"{name}\"")))
        }
        Query::Property {
            name,
            value: Some(v),
        } => {
            out.push((depth, format!("Property \"{name}\" must match:")));
            explain_into(v, depth + 1, out);
        }
        Query::Scoped(op, inner) => {
            out.push((depth, format!("{}:", op_words(*op))));
            explain_into(inner, depth + 1, out);
        }
        Query::And(v) if v.len() == 1 => explain_into(&v[0], depth, out),
        Query::And(v) => {
            out.push((depth, "Match all of:".to_string()));
            for c in v {
                explain_into(c, depth + 1, out);
            }
        }
        Query::Or(v) => {
            out.push((depth, "Match any of:".to_string()));
            for c in v {
                explain_into(c, depth + 1, out);
            }
        }
        Query::Not(inner) => {
            out.push((depth, "Does not match:".to_string()));
            explain_into(inner, depth + 1, out);
        }
    }
}

/// Search operators shown in the help popover (operator, description).
pub const SEARCH_OPERATORS: &[(&str, &str)] = &[
    ("file:", "match file name"),
    ("path:", "match file path"),
    ("content:", "match file content"),
    ("tag:", "search for tags"),
    ("line:", "search keywords on the same line"),
    ("block:", "search keywords in the same block"),
    ("section:", "search keywords under the same heading"),
    ("task:", "search keywords in tasks"),
    ("task-todo:", "search keywords in uncompleted tasks"),
    ("task-done:", "search keywords in completed tasks"),
    ("match-case:", "case-sensitive match"),
    ("ignore-case:", "case-insensitive match"),
    ("[property]", "match property"),
    ("[property:value]", "match property value"),
    ("\"phrase\"", "match exact phrase"),
    ("-term", "exclude term"),
    ("a OR b", "match either term"),
    ("/regex/", "regular expression"),
];

/// Options of the backlinks pane.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BacklinkOptions {
    pub collapse: bool,
    pub more_context: bool,
    pub sort: SortOrder,
    pub filter: String,
    pub show_filter: bool,
    /// The "Unlinked mentions" section is expanded (computed lazily).
    pub show_unlinked: bool,
}

/// One line (or paragraph with "show more context") shown under a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    /// Line to reveal when clicked.
    pub line: usize,
    pub text: String,
    /// Byte ranges to highlight in `text`.
    pub ranges: Vec<(usize, usize)>,
    /// For unlinked mentions: the mention, to be turned into a link.
    pub mention: Option<Mention>,
}

/// Text around `line` of `source`: the line itself, or the surrounding
/// paragraph (until blank lines, at most 3 lines on each side) when `more`.
/// `highlights` are byte ranges in `source`.
pub fn context_for(
    source: &str,
    line: usize,
    highlights: &[(usize, usize)],
    more: bool,
) -> ContextItem {
    let starts = line_starts(source);
    let n = starts.len();
    let line = line.min(n.saturating_sub(1));
    let line_text = |i: usize| -> &str {
        let s = starts[i];
        let e = starts.get(i + 1).map_or(source.len(), |e| *e);
        source[s..e].trim_end_matches(['\n', '\r'])
    };
    let (first, last) = if more {
        let mut a = line;
        while a > 0 && line - a < 3 && !line_text(a - 1).trim().is_empty() {
            a -= 1;
        }
        let mut b = line;
        while b + 1 < n && b < line + 3 && !line_text(b + 1).trim().is_empty() {
            b += 1;
        }
        (a, b)
    } else {
        (line, line)
    };
    let start = starts[first];
    let end = starts
        .get(last + 1)
        .map_or(source.len(), |e| *e)
        .min(source.len());
    let raw = &source[start..end];
    let trimmed_end = raw.trim_end();
    // Leading whitespace of the first line is dropped.
    let lead = trimmed_end.len() - trimmed_end.trim_start().len();
    let text = trimmed_end.trim_start().to_string();
    let base = start + lead;
    let ranges = highlights
        .iter()
        .filter_map(|&(s, e)| {
            let s2 = s.max(base);
            let e2 = e.min(base + text.len());
            (s2 < e2).then(|| (s2 - base, e2 - base))
        })
        .filter(|(s, e)| text.is_char_boundary(*s) && text.is_char_boundary(*e))
        .collect();
    ContextItem {
        line,
        text,
        ranges,
        mention: None,
    }
}

pub fn line_starts(source: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    if v.len() > 1 && v.last() == Some(&source.len()) {
        v.pop();
    }
    v
}

pub fn line_of(source: &str, byte: usize) -> usize {
    source[..byte.min(source.len())].matches('\n').count()
}

/// A plain-text occurrence of a note name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub line: usize,
    /// Byte offsets in the note source.
    pub start: usize,
    pub end: usize,
    /// Text as written.
    pub text: String,
}

/// Byte ranges where mentions are not looked for: frontmatter, links,
/// tags, code and comments.
fn excluded_ranges(source: &str, parsed: &ParsedNote) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = Vec::new();
    if let Some(fm) = parsed.frontmatter_span {
        v.push((fm.start, fm.end));
    }
    v.extend(parsed.links.iter().map(|l| (l.span.start, l.span.end)));
    v.extend(
        parsed
            .tags
            .iter()
            .filter(|t| !t.from_frontmatter)
            .map(|t| (t.span.start, t.span.end)),
    );
    for (r, s) in highlight(source) {
        if s.code || s.comment || s.frontmatter || s.link {
            v.push((r.start, r.end));
        }
    }
    v.sort_unstable();
    v
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Case-insensitive prefix match of `needle` at the start of `hay`; returns
/// the matched byte length in `hay`.
fn match_ci(hay: &str, needle: &str) -> Option<usize> {
    let mut h = hay.char_indices();
    for nc in needle.chars() {
        let (_, hc) = h.next()?;
        if hc != nc && !hc.to_lowercase().eq(nc.to_lowercase()) {
            return None;
        }
    }
    Some(h.next().map_or(hay.len(), |(i, _)| i))
}

/// Whole-word, case-insensitive occurrences of any of `needles` in
/// `source`, outside links, tags, code, comments and frontmatter. Longer
/// needles win when several match at the same place.
pub fn find_mentions(source: &str, parsed: &ParsedNote, needles: &[String]) -> Vec<Mention> {
    let mut needles: Vec<&str> = needles
        .iter()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .collect();
    needles.sort_by_key(|n| std::cmp::Reverse(n.len()));
    needles.dedup();
    if needles.is_empty() {
        return Vec::new();
    }
    let excluded = excluded_ranges(source, parsed);
    let in_excluded = |s: usize, e: usize| excluded.iter().any(|&(a, b)| s < b && e > a);
    let firsts: Vec<Vec<char>> = needles
        .iter()
        .map(|n| {
            n.chars()
                .next()
                .map(|c| c.to_lowercase().collect())
                .unwrap_or_default()
        })
        .collect();
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut skip_until = 0;
    for (i, c) in source.char_indices() {
        let boundary = prev.is_none_or(|p| !is_word_char(p));
        prev = Some(c);
        if i < skip_until || !boundary {
            continue;
        }
        let lc: Vec<char> = c.to_lowercase().collect();
        for (k, needle) in needles.iter().enumerate() {
            if firsts[k] != lc {
                continue;
            }
            let Some(len) = match_ci(&source[i..], needle) else {
                continue;
            };
            let end = i + len;
            let after_ok = source[end..]
                .chars()
                .next()
                .is_none_or(|a| !is_word_char(a));
            if !after_ok || in_excluded(i, end) {
                continue;
            }
            out.push(Mention {
                line: line_of(source, i),
                start: i,
                end,
                text: source[i..end].to_string(),
            });
            skip_until = end;
            break;
        }
    }
    out
}

/// Replace a mention by a wikilink to the note whose link text is
/// `link_text`. Returns `None` if the source changed since the mention was
/// found.
pub fn link_mention(source: &str, m: &Mention, link_text: &str) -> Option<String> {
    if source.get(m.start..m.end)? != m.text {
        return None;
    }
    let link = if m.text == link_text
        || (m.text.to_lowercase() == link_text.to_lowercase() && !link_text.contains('/'))
    {
        format!("[[{}]]", m.text)
    } else {
        format!("[[{link_text}|{}]]", m.text)
    };
    let mut out = String::with_capacity(source.len() + link.len());
    out.push_str(&source[..m.start]);
    out.push_str(&link);
    out.push_str(&source[m.end..]);
    Some(out)
}

/// Toggle the checkbox of the task on `line` (`- [ ]` ↔ `- [x]`; any other
/// status becomes unchecked). Works in blockquotes / callouts too.
pub fn toggle_task(source: &str, line: usize) -> Option<String> {
    let starts = line_starts(source);
    let s = *starts.get(line)?;
    let e = starts.get(line + 1).map_or(source.len(), |e| *e);
    let text = &source[s..e];
    let bytes = text.as_bytes();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < bytes.len() && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
    };
    skip_ws(&mut i);
    while i < bytes.len() && bytes[i] == b'>' {
        i += 1;
        skip_ws(&mut i);
    }
    if i < bytes.len() && matches!(bytes[i], b'-' | b'*' | b'+') {
        i += 1;
    } else {
        let d = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == d || i >= bytes.len() || !matches!(bytes[i], b'.' | b')') {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b' ' {
        return None;
    }
    skip_ws(&mut i);
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let status_start = i + 1;
    let status = text[status_start..].chars().next()?;
    let status_end = status_start + status.len_utf8();
    if text.as_bytes().get(status_end) != Some(&b']') {
        return None;
    }
    let new = if status == ' ' { "x" } else { " " };
    let mut out = String::with_capacity(source.len());
    out.push_str(&source[..s + status_start]);
    out.push_str(new);
    out.push_str(&source[s + status_end..]);
    Some(out)
}

/// Byte ranges of the case-insensitive occurrences of `terms` in `text`
/// (non-overlapping, sorted).
pub fn highlight_terms(text: &str, terms: &[&str]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let hit = terms
            .iter()
            .filter(|t| !t.is_empty())
            .filter_map(|t| match_ci(&text[i..], t).map(|e| i + e))
            .max();
        match hit {
            Some(end) => {
                out.push((i, end));
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

/// Has the search box been quiet for `delay` seconds (or never edited)?
pub fn debounce_ready(edited: Option<f64>, now: f64, delay: f64) -> bool {
    edited.is_none_or(|t| now - t >= delay)
}

/// Node of the outline tree: index in the heading list and children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineNode {
    pub index: usize,
    pub children: Vec<OutlineNode>,
}

/// Nest headings by level: a heading's children are the following deeper
/// headings (levels may skip, like `#` then `###`).
pub fn outline_tree(levels: &[u8]) -> Vec<OutlineNode> {
    fn build(levels: &[u8], pos: &mut usize, parent_level: u8) -> Vec<OutlineNode> {
        let mut out = Vec::new();
        while *pos < levels.len() && levels[*pos] > parent_level {
            let index = *pos;
            let level = levels[index];
            *pos += 1;
            let children = build(levels, pos, level);
            out.push(OutlineNode { index, children });
        }
        out
    }
    let mut pos = 0;
    build(levels, &mut pos, 0)
}

/// Does heading `index` or one of its descendants contain `filter`?
pub fn outline_matches(node: &OutlineNode, texts: &[&str], filter: &str) -> bool {
    texts
        .get(node.index)
        .is_some_and(|t| t.to_lowercase().contains(filter))
        || node
            .children
            .iter()
            .any(|c| outline_matches(c, texts, filter))
}

/// Cached results of a pane, recomputed when the key changes.
#[derive(Debug, Clone)]
pub struct Cached<K, V> {
    pub key: Option<K>,
    pub value: V,
    pub time: f64,
}

impl<K, V: Default> Default for Cached<K, V> {
    fn default() -> Self {
        Self {
            key: None,
            value: V::default(),
            time: 0.0,
        }
    }
}

/// Groups of context items per source note.
pub type Grouped = Vec<(NotePath, Vec<ContextItem>)>;

/// Everything the views keep between frames.
pub struct ViewsState {
    pub graph: GraphStore,
    pub local_graph: GraphStore,
    /// Local graph shown in the right sidebar.
    pub local_pane: Option<Box<GraphTab>>,
    pub search: SearchOptions,
    /// Time of the last keystroke in the search box (debounce).
    pub search_edited: Option<f64>,
    pub backlinks: BacklinkOptions,
    pub linked: Cached<(NotePath, u64, bool), Grouped>,
    pub unlinked: Cached<(NotePath, u64, bool), Grouped>,
    /// Outgoing unlinked mentions of the active note: (mention, target).
    pub outgoing_mentions: Cached<(NotePath, u64), Vec<(Mention, NotePath)>>,
    pub outline_filter: String,
    /// Search results sorted for display.
    pub sorted: Cached<(String, u64, SortOrder), Vec<SearchResult>>,
    /// Line of the cursor (editor) or of the top of the viewport (reading
    /// view) of the active note, for the outline highlight.
    pub current_line: Option<(NotePath, usize)>,
    /// Show the frontmatter table at the top of the reading view (until the
    /// Properties panel exists).
    pub show_properties_table: bool,
}

impl ViewsState {
    pub fn new(root: Option<&Path>) -> Self {
        Self {
            graph: GraphStore::load(root, GRAPH_FILE, GraphSettings::default()),
            local_graph: GraphStore::load(root, LOCAL_GRAPH_FILE, GraphSettings::local_default()),
            local_pane: None,
            search: SearchOptions::default(),
            search_edited: None,
            backlinks: BacklinkOptions {
                show_unlinked: true,
                ..BacklinkOptions::default()
            },
            linked: Cached::default(),
            unlinked: Cached::default(),
            outgoing_mentions: Cached::default(),
            outline_filter: String::new(),
            sorted: Cached::default(),
            current_line: None,
            show_properties_table: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Derived data
// ---------------------------------------------------------------------------

/// Names a note can be mentioned by: stem and aliases.
pub fn note_names(state: &AppState, path: &NotePath) -> Vec<String> {
    let mut v = vec![path.stem().to_string()];
    if let Some(n) = state.index.get(path) {
        v.extend(n.aliases.iter().cloned());
    }
    v
}

/// Linked mentions of `path` grouped by source note, with context.
pub fn linked_mentions(state: &AppState, path: &NotePath, more: bool) -> Grouped {
    let mut grouped: Grouped = Vec::new();
    let mut texts: HashMap<NotePath, Option<String>> = HashMap::new();
    for bl in state.index.backlinks(path) {
        let Some(source) = texts
            .entry(bl.source.clone())
            .or_insert_with(|| state.note_text(&bl.source))
            .clone()
        else {
            continue;
        };
        let span = (bl.link.span.start, bl.link.span.end);
        let item = context_for(&source, bl.link.line, &[span], more);
        match grouped.last_mut() {
            Some((p, v)) if *p == bl.source => {
                // Several links in the same context: merge highlights.
                if let Some(last) = v.last_mut().filter(|l| l.text == item.text) {
                    last.ranges.extend(item.ranges);
                } else {
                    v.push(item);
                }
            }
            _ => grouped.push((bl.source.clone(), vec![item])),
        }
    }
    grouped
}

/// Unlinked mentions of `path` grouped by source note.
pub fn unlinked_mentions(state: &AppState, path: &NotePath, more: bool) -> Grouped {
    let names = note_names(state, path);
    let mut out = Vec::new();
    for src in state.index.unlinked_mentions(state.vault.as_read(), path) {
        let Some(source) = state.note_text(&src) else {
            continue;
        };
        let parsed = state.parsed_with_text(&src, &source);
        let items: Vec<ContextItem> = find_mentions(&source, &parsed, &names)
            .into_iter()
            .map(|m| {
                let mut item = context_for(&source, m.line, &[(m.start, m.end)], more);
                item.mention = Some(m);
                item
            })
            .collect();
        if !items.is_empty() {
            out.push((src, items));
        }
    }
    out
}

/// Plain-text mentions, in `path`, of other notes' names.
pub fn outgoing_mentions(state: &AppState, path: &NotePath) -> Vec<(Mention, NotePath)> {
    let Some(source) = state.note_text(path) else {
        return Vec::new();
    };
    let parsed = state.parsed_with_text(path, &source);
    let words: HashSet<String> = source
        .split(|c: char| !is_word_char(c))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut by_name: HashMap<String, NotePath> = HashMap::new();
    let mut needles: Vec<String> = Vec::new();
    for f in state.index.files() {
        if !f.is_markdown() || &f == path {
            continue;
        }
        for name in note_names(state, &f) {
            let lower = name.to_lowercase();
            let first = lower
                .split(|c: char| !is_word_char(c))
                .find(|w| !w.is_empty());
            if lower.chars().count() < 2 || !first.is_some_and(|w| words.contains(w)) {
                continue;
            }
            if by_name.insert(lower, f.clone()).is_none() {
                needles.push(name);
            }
        }
    }
    let linked: HashSet<NotePath> = state
        .index
        .outgoing(path)
        .into_iter()
        .filter_map(|l| l.target)
        .collect();
    find_mentions(&source, &parsed, &needles)
        .into_iter()
        .filter_map(|m| {
            let target = by_name.get(&m.text.to_lowercase())?.clone();
            (!linked.contains(&target)).then_some((m, target))
        })
        .collect()
}

/// (modified, created) of a file.
pub fn file_times(state: &AppState, path: &NotePath) -> Option<(SystemTime, SystemTime)> {
    state
        .vault
        .metadata(path)
        .ok()
        .map(|m| (m.modified, m.created))
}

/// Sort search results according to the pane options.
pub fn sort_results(state: &AppState, results: &mut [SearchResult], order: SortOrder) {
    sort_by_path(results, |r| &r.path, order, |p| file_times(state, p));
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// Requests from the view panes, applied after the frame.
#[derive(Debug, Clone)]
pub enum ViewAction {
    /// Scroll the note (in its current mode) to a line; opens it if needed.
    RevealLine {
        path: NotePath,
        line: usize,
        open_in: OpenIn,
    },
    /// Turn a plain-text mention in `source` into a link to `target`.
    LinkMention {
        source: NotePath,
        mention: Mention,
        target: NotePath,
    },
    ToggleTask {
        path: NotePath,
        line: usize,
    },
    /// The reading view scrolled: line of the current heading.
    SetCurrentLine(NotePath, usize),
}

pub fn apply(state: &mut AppState, action: ViewAction) {
    match action {
        ViewAction::RevealLine {
            path,
            line,
            open_in,
        } => reveal_line(state, &path, line, open_in),
        ViewAction::LinkMention {
            source,
            mention,
            target,
        } => {
            let link_text = state.index.link_text_for(&target, &source);
            if !edit_note(state, &source, |t| link_mention(t, &mention, &link_text)) {
                state.error("The note changed; mention not linked");
            }
        }
        ViewAction::ToggleTask { path, line } => {
            if !edit_note(state, &path, |t| toggle_task(t, line)) {
                state.error("Cannot toggle this task");
            }
        }
        ViewAction::SetCurrentLine(path, line) => state.views.current_line = Some((path, line)),
    }
}

/// Reveal `line` of `path` keeping the reading/editing mode of the tab.
pub fn reveal_line(state: &mut AppState, path: &NotePath, line: usize, open_in: OpenIn) {
    let is_active = state.active_note() == Some(path);
    if !is_active || open_in == OpenIn::NewTab {
        state.open(path, open_in);
    }
    if let Some(tab) = state.active_tab_mut().and_then(Tab::note_mut)
        && &tab.path == path
    {
        tab.pending_line = Some(line);
    }
}

/// Take the line the active note tab must reveal (reading view).
pub fn take_pending_line(state: &mut AppState) -> Option<usize> {
    state
        .active_tab_mut()
        .and_then(Tab::note_mut)
        .and_then(|n| n.pending_line.take())
}

/// Apply `f` to the text of `path` (open buffer or file on disk) and keep
/// the index up to date. Returns false if `f` refused or writing failed.
pub fn edit_note(
    state: &mut AppState,
    path: &NotePath,
    f: impl FnOnce(&str) -> Option<String>,
) -> bool {
    if let Some(b) = state.buffers.get_mut(path) {
        let Some(new) = f(&b.text) else {
            return false;
        };
        b.text = new;
        state.buffer_edited(path);
        return true;
    }
    let Ok(old) = state.vault.read_to_string(path) else {
        return false;
    };
    let Some(new) = f(&old) else {
        return false;
    };
    match state.vault.write(path, &new) {
        Ok(()) => {
            state.index.update_from_source(path, &new);
            state.generation += 1;
            true
        }
        Err(e) => {
            state.error(format!("Cannot save {path}: {e}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::naive_parse;

    #[test]
    fn terms_debounce_and_outline_tree() {
        assert_eq!(
            highlight_terms("Rust and RUST", &["rust"]),
            vec![(0, 4), (9, 13)]
        );
        assert_eq!(highlight_terms("été", &["É"]), vec![(0, 2), (3, 5)]);
        assert!(highlight_terms("abc", &[""]).is_empty());
        assert!(debounce_ready(None, 5.0, 0.3));
        assert!(!debounce_ready(Some(5.0), 5.1, 0.3));
        assert!(debounce_ready(Some(5.0), 5.4, 0.3));
        // # A / ## B / ### C / ## D / # E / ### F
        let t = outline_tree(&[1, 2, 3, 2, 1, 3]);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].children.len(), 2);
        assert_eq!(t[0].children[0].children[0].index, 2);
        assert_eq!(t[1].children[0].index, 5);
        let texts = ["A", "B", "C", "D", "E", "Foo"];
        assert!(outline_matches(&t[1], &texts, "foo"));
        assert!(!outline_matches(&t[0], &texts, "foo"));
    }

    #[test]
    fn effective_query_and_explain() {
        let mut o = SearchOptions::default();
        assert_eq!(effective_query("  rust ", &o), "rust");
        o.match_case = true;
        assert_eq!(effective_query("rust", &o), "match-case:(rust)");
        assert_eq!(effective_query("  ", &o), "");
        let q = oxi_index::parse_query(&effective_query("rust OR \"a b\" -tag:#x", &o)).unwrap();
        let lines = explain_query(&q);
        assert_eq!(lines[0], (0, "Case sensitive:".to_string()));
        let all: Vec<&str> = lines.iter().map(|l| l.1.as_str()).collect();
        assert!(all.contains(&"Match any of:"), "{all:?}");
        assert!(all.contains(&"Match exact phrase \"a b\""), "{all:?}");
        assert!(all.contains(&"Does not match:"), "{all:?}");
        assert!(all.contains(&"Has tag #x"), "{all:?}");
        // The match-case wrapper is accepted by the real search engine.
        let v = crate::backend::MemVault::with_files("t", [("a.md", "Rust\nrust")]);
        let idx = crate::backend::RealIndex::build(&v).unwrap();
        use crate::backend::IndexBackend;
        let r = idx.search("match-case:(Rust)").unwrap();
        assert_eq!(r[0].matches.len(), 1);
        let r = idx.search("rust").unwrap();
        assert_eq!(r[0].matches.len(), 2);
    }

    #[test]
    fn sorting() {
        let p = |s: &str| NotePath::new(s).unwrap();
        let mut v = vec![p("b.md"), p("A.md"), p("c.md")];
        sort_by_path(&mut v, |x| x, SortOrder::NameAsc, |_| None);
        assert_eq!(v, vec![p("A.md"), p("b.md"), p("c.md")]);
        sort_by_path(&mut v, |x| x, SortOrder::NameDesc, |_| None);
        assert_eq!(v[0], p("c.md"));
        let t = |s: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let meta = |x: &NotePath| match x.stem() {
            "A" => Some((t(30), t(1))),
            "b" => Some((t(10), t(3))),
            _ => Some((t(20), t(2))),
        };
        sort_by_path(&mut v, |x| x, SortOrder::ModifiedNewest, meta);
        assert_eq!(v, vec![p("A.md"), p("c.md"), p("b.md")]);
        sort_by_path(&mut v, |x| x, SortOrder::CreatedOldest, meta);
        assert_eq!(v, vec![p("A.md"), p("c.md"), p("b.md")]);
        sort_by_path(&mut v, |x| x, SortOrder::CreatedNewest, meta);
        assert_eq!(v[0], p("b.md"));
    }

    #[test]
    fn context_lines_and_paragraphs() {
        let src = "# T\n\nfirst line\nsecond [[X]] here\nthird\n\nother";
        let s = src.find("[[X]]").unwrap();
        let c = context_for(src, 3, &[(s, s + 5)], false);
        assert_eq!(c.text, "second [[X]] here");
        assert_eq!(c.ranges, vec![(7, 12)]);
        let c = context_for(src, 3, &[(s, s + 5)], true);
        assert_eq!(c.text, "first line\nsecond [[X]] here\nthird");
        assert_eq!(&c.text[c.ranges[0].0..c.ranges[0].1], "[[X]]");
        // Out of range line is clamped.
        let c = context_for("a", 10, &[], false);
        assert_eq!(c.text, "a");
    }

    #[test]
    fn mentions_skip_links_code_and_frontmatter() {
        let src = "---\ntitle: Rust\n---\nI like rust and Rustacean.\n`rust` [[Rust]] #rust\nRUST!";
        let parsed = naive_parse(src);
        let m = find_mentions(src, &parsed, &["Rust".to_string()]);
        let texts: Vec<&str> = m.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, vec!["rust", "RUST"], "{m:?}");
        assert_eq!(m[0].line, 3);
        assert_eq!(m[1].line, 5);
        // Longer names win; accents and case folding.
        let src = "Les notes Rust notes et ÉTÉ.";
        let parsed = naive_parse(src);
        let m = find_mentions(
            src,
            &parsed,
            &["Rust".into(), "Rust notes".into(), "été".into()],
        );
        let texts: Vec<&str> = m.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, vec!["Rust notes", "ÉTÉ"]);
    }

    #[test]
    fn linking_a_mention() {
        let src = "Voir rust ici.\nEt Oxinote.";
        let parsed = naive_parse(src);
        let m = find_mentions(src, &parsed, &["Rust".into()]);
        assert_eq!(m.len(), 1);
        assert_eq!(
            link_mention(src, &m[0], "Rust").unwrap(),
            "Voir [[rust]] ici.\nEt Oxinote."
        );
        assert_eq!(
            link_mention(src, &m[0], "Projets/Rust").unwrap(),
            "Voir [[Projets/Rust|rust]] ici.\nEt Oxinote."
        );
        let alias = find_mentions(src, &parsed, &["ici".into()]);
        assert_eq!(
            link_mention(src, &alias[0], "Accueil").unwrap(),
            "Voir rust [[Accueil|ici]].\nEt Oxinote."
        );
        // Stale mention is refused.
        assert!(link_mention("changed", &m[0], "Rust").is_none());
    }

    #[test]
    fn toggling_tasks() {
        let src = "- [ ] a\n- [x] b\n  * [/] c\n> - [ ] quoted\n1. [ ] num\nnot a task [ ]";
        assert_eq!(toggle_task(src, 0).unwrap().lines().next(), Some("- [x] a"));
        assert_eq!(toggle_task(src, 1).unwrap().lines().nth(1), Some("- [ ] b"));
        assert_eq!(
            toggle_task(src, 2).unwrap().lines().nth(2),
            Some("  * [ ] c")
        );
        assert_eq!(
            toggle_task(src, 3).unwrap().lines().nth(3),
            Some("> - [x] quoted")
        );
        assert_eq!(
            toggle_task(src, 4).unwrap().lines().nth(4),
            Some("1. [x] num")
        );
        assert!(toggle_task(src, 5).is_none());
        assert!(toggle_task(src, 99).is_none());
    }

    #[test]
    fn state_links_mention_and_toggles_task() {
        use crate::backend::MemVault;
        use crate::settings::Settings;
        let v = MemVault::with_files(
            "t",
            [
                ("Rust.md", "# Rust\n- [ ] learn"),
                ("B.md", "I love rust a lot"),
            ],
        );
        let mut s = AppState::new(
            Box::new(v),
            |v| Box::new(crate::fake::FakeIndex::build(v)),
            Settings::default(),
        );
        let rust = NotePath::new("Rust.md").unwrap();
        let b = NotePath::new("B.md").unwrap();
        let groups = unlinked_mentions(&s, &rust, false);
        assert_eq!(groups.len(), 1);
        let m = groups[0].1[0].mention.clone().unwrap();
        let g0 = s.generation;
        apply(
            &mut s,
            ViewAction::LinkMention {
                source: b.clone(),
                mention: m,
                target: rust.clone(),
            },
        );
        assert_eq!(s.vault.read_to_string(&b).unwrap(), "I love [[rust]] a lot");
        assert!(s.generation > g0);
        assert_eq!(s.index.backlinks(&rust).len(), 1);
        assert_eq!(
            linked_mentions(&s, &rust, false)[0].1[0].text,
            "I love [[rust]] a lot"
        );
        assert!(unlinked_mentions(&s, &rust, false).is_empty());
        // Task toggling goes through the open buffer.
        s.open(&rust, OpenIn::CurrentTab);
        apply(
            &mut s,
            ViewAction::ToggleTask {
                path: rust.clone(),
                line: 1,
            },
        );
        assert_eq!(s.buffers[&rust].text, "# Rust\n- [x] learn");
        // Outgoing mentions: B mentions nothing else now; Rust mentions none.
        assert!(outgoing_mentions(&s, &b).is_empty());
        s.open(&b, OpenIn::CurrentTab);
        reveal_line(&mut s, &b, 0, OpenIn::CurrentTab);
        assert_eq!(
            s.active_tab()
                .and_then(Tab::note)
                .and_then(|n| n.pending_line),
            Some(0)
        );
    }
}
