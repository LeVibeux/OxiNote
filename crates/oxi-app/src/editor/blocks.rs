//! Block-level structure used by Live Preview: fenced code blocks, markdown
//! tables, inline maths, footnote references and list indentation guides.
//!
//! Everything works on byte ranges of the note text and never changes it.

use std::collections::HashSet;
use std::ops::Range;

use super::format::{ListKind, parse_prefix};
use super::live::{LineInfo, LineKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// Opening fence line ..= closing fence line (or the end of the note).
    Code { lang: String },
    /// Header row, delimiter row, body rows.
    Table { aligns: Vec<Align> },
}

/// A block spanning the lines `start ..= end` (0-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSpan {
    pub kind: BlockKind,
    pub start: usize,
    pub end: usize,
}

/// Cell of a table row: byte ranges in the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Everything between the two bars.
    pub region: Range<usize>,
    /// The trimmed content.
    pub content: Range<usize>,
}

/// Byte positions of the cell separators `|` of `line` (starting at byte
/// `base` of the note). Escaped bars, bars in code spans and in `[[..]]`
/// are not separators.
pub fn bars(line: &str, base: usize) -> Vec<usize> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut in_code, mut in_wiki) = (0, false, false);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'`' => in_code = !in_code,
            b'[' if !in_code && b.get(i + 1) == Some(&b'[') => {
                in_wiki = true;
                i += 1;
            }
            b']' if in_wiki && b.get(i + 1) == Some(&b']') => {
                in_wiki = false;
                i += 1;
            }
            b'|' if !in_code && !in_wiki => out.push(base + i),
            _ => {}
        }
        i += 1;
    }
    out
}

/// Cells of a table row starting at byte `base`.
pub fn cells(text: &str, line: Range<usize>) -> Vec<Cell> {
    let src = &text[line.clone()];
    let mut seps = bars(src, line.start);
    let first_content = src.len() - src.trim_start().len() + line.start;
    // Rows without a leading / trailing bar get virtual ones.
    let mut bounds: Vec<(usize, usize)> = Vec::new(); // (region start, region end)
    let lead = seps.first().is_some_and(|b| *b == first_content);
    let mut start = if lead { seps.remove(0) + 1 } else { line.start };
    for s in &seps {
        bounds.push((start, *s));
        start = *s + 1;
    }
    if start < line.end && !text[start..line.end].trim().is_empty() {
        bounds.push((start, line.end));
    }
    bounds
        .into_iter()
        .map(|(a, b)| {
            let s = &text[a..b];
            let ts = a + (s.len() - s.trim_start().len());
            let te = (b - (s.len() - s.trim_end().len())).max(ts);
            Cell {
                region: a..b,
                content: ts..te,
            }
        })
        .collect()
}

/// Alignments of a delimiter row (`|:--|:-:|--:|`), `None` if it is not one.
pub fn delimiter_row(line: &str) -> Option<Vec<Align>> {
    let t = line.trim();
    if !t.contains('-') || !t.contains('|') && !t.starts_with(':') && !t.starts_with('-') {
        return None;
    }
    let inner = t.trim_start_matches('|').trim_end_matches('|');
    let mut out = Vec::new();
    for c in inner.split('|') {
        let c = c.trim();
        let (l, r) = (c.starts_with(':'), c.ends_with(':'));
        let dashes = c.trim_matches(':');
        if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
            return None;
        }
        out.push(match (l, r) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    (!out.is_empty()).then_some(out)
}

/// Code blocks and tables of the note.
pub fn find_blocks(text: &str, lines: &[LineInfo]) -> Vec<BlockSpan> {
    let src = |i: usize| text.get(lines[i].range.clone()).unwrap_or("");
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            LineKind::CodeFence => {
                let lang = src(i)
                    .trim_start()
                    .trim_start_matches(['`', '~'])
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let mut end = i;
                let mut j = i + 1;
                while j < lines.len() {
                    end = j;
                    if lines[j].kind == LineKind::CodeFence {
                        break;
                    }
                    j += 1;
                }
                out.push(BlockSpan {
                    kind: BlockKind::Code { lang },
                    start: i,
                    end,
                });
                i = end + 1;
            }
            LineKind::Normal
                if src(i).contains('|')
                    && i + 1 < lines.len()
                    && lines[i + 1].kind == LineKind::Normal
                    && delimiter_row(src(i + 1)).is_some()
                    && !src(i).trim().is_empty() =>
            {
                let aligns = delimiter_row(src(i + 1)).unwrap_or_default();
                let mut end = i + 1;
                while end + 1 < lines.len()
                    && lines[end + 1].kind == LineKind::Normal
                    && src(end + 1).contains('|')
                {
                    end += 1;
                }
                out.push(BlockSpan {
                    kind: BlockKind::Table { aligns },
                    start: i,
                    end,
                });
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// Widen `active` (the lines holding the caret) to the whole code blocks and
/// tables it touches: those show their raw markdown as a whole.
pub fn expand_active(active: Range<usize>, blocks: &[BlockSpan]) -> Range<usize> {
    if active.is_empty() {
        return active;
    }
    let (mut a, mut b) = (active.start, active.end);
    for bl in blocks {
        if bl.start < b && a <= bl.end {
            a = a.min(bl.start);
            b = b.max(bl.end + 1);
        }
    }
    a..b
}

/// Lines that take no room while the caret is elsewhere: fence lines of code
/// blocks and delimiter rows of tables.
pub fn collapsed_lines(blocks: &[BlockSpan], active: &Range<usize>) -> HashSet<usize> {
    let mut out = HashSet::new();
    for bl in blocks {
        if active.contains(&bl.start) || active.contains(&bl.end) {
            continue;
        }
        match bl.kind {
            BlockKind::Code { .. } => {
                out.insert(bl.start);
                out.insert(bl.end);
            }
            BlockKind::Table { .. } => {
                out.insert(bl.start + 1);
            }
        }
    }
    out
}

/// Inline maths `$..$` / `$$..$$` ranges of one line (not prices like `$5`).
pub fn math_spans(line: &str, base: usize) -> Vec<Range<usize>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_code = false;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'`' => in_code = !in_code,
            b'$' if !in_code => {
                let dbl = b.get(i + 1) == Some(&b'$');
                let open = if dbl { 2 } else { 1 };
                let from = i + open;
                let close = line[from..].find(if dbl { "$$" } else { "$" });
                if let Some(rel) = close {
                    let inner = &line[from..from + rel];
                    let ok = !inner.is_empty()
                        && (dbl
                            || (!inner.starts_with(char::is_whitespace)
                                && !inner.ends_with(char::is_whitespace)));
                    let after = from + rel + open;
                    let price = !dbl && b.get(after).is_some_and(u8::is_ascii_digit);
                    if ok && !price {
                        out.push(base + i..base + after);
                        i = after;
                        continue;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Footnote references `[^id]` (not definitions at the line start).
pub fn footnote_refs(line: &str, base: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = line[from..].find("[^") {
        let s = from + rel;
        let Some(e) = line[s..].find(']') else { break };
        let end = s + e + 1;
        let id = &line[s + 2..end - 1];
        let is_def = s == 0 && line[end..].starts_with(':');
        if !id.is_empty() && !id.contains(char::is_whitespace) && !is_def {
            out.push(base + s..base + end);
        }
        from = end;
    }
    out
}

/// Indentation guide: on `line`, a vertical guide below the marker of an
/// ancestor item, whose marker is at byte `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub line: usize,
    pub at: usize,
}

/// Guides and nesting depth of every list item line.
pub fn list_guides(text: &str, lines: &[LineInfo]) -> (Vec<Guide>, Vec<(usize, usize)>) {
    let mut guides = Vec::new();
    let mut depths = Vec::new();
    // (indent width, byte of the marker)
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (n, l) in lines.iter().enumerate() {
        let src = text.get(l.range.clone()).unwrap_or("");
        let item = (l.kind == LineKind::Normal)
            .then(|| parse_prefix(src))
            .flatten()
            .filter(|p| matches!(p.kind, ListKind::Bullet(_) | ListKind::Ordered { .. }));
        let Some(p) = item else {
            if !src.trim().is_empty() && !src.starts_with([' ', '\t']) {
                stack.clear();
            }
            continue;
        };
        let width = src
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum::<usize>();
        while stack.last().is_some_and(|(w, _)| *w >= width) {
            stack.pop();
        }
        for (_, at) in &stack {
            guides.push(Guide { line: n, at: *at });
        }
        depths.push((n, stack.len()));
        stack.push((width, l.range.start + p.indent));
    }
    (guides, depths)
}

/// Numbered lists: for every item whose number has fewer digits than the
/// longest of its list, `(delimiter byte, missing digits)`, so that the item
/// texts line up.
pub fn ordered_runs(text: &str, lines: &[LineInfo]) -> Vec<(usize, usize)> {
    // (delimiter byte, digits) of the current list at each indent.
    let mut out = Vec::new();
    let mut run: Vec<(usize, usize, usize)> = Vec::new(); // (indent, byte, digits)
    let flush = |run: &mut Vec<(usize, usize, usize)>, out: &mut Vec<(usize, usize)>| {
        let max = run.iter().map(|r| r.2).max().unwrap_or(0);
        for (_, at, d) in run.drain(..) {
            if d < max {
                out.push((at + 1, max - d));
            }
        }
    };
    let mut cur_indent = None;
    for l in lines {
        let src = text.get(l.range.clone()).unwrap_or("");
        let item = (l.kind == LineKind::Normal)
            .then(|| parse_prefix(src))
            .flatten();
        match item {
            Some(p) if matches!(p.kind, ListKind::Ordered { .. }) => {
                let digits = src[p.indent..]
                    .bytes()
                    .take_while(u8::is_ascii_digit)
                    .count();
                if cur_indent.is_some_and(|i| i != p.indent) {
                    // Nested list of another indent: keep the outer run.
                    if cur_indent.is_some_and(|i| i > p.indent) {
                        flush(&mut run, &mut out);
                    } else {
                        continue;
                    }
                }
                cur_indent = Some(p.indent);
                run.push((p.indent, l.range.start + p.indent + digits, digits));
            }
            Some(p) if cur_indent.is_some_and(|i| p.indent > i) => {}
            _ => {
                if !src.trim().is_empty() || cur_indent.is_some() && src.is_empty() {
                    flush(&mut run, &mut out);
                    cur_indent = None;
                }
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

/// One padding request: add `amount` points after the char at byte `at`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pad {
    pub at: usize,
    pub amount: f32,
}

/// Padding that aligns the columns of a table. `rows` are the cells of every
/// row except the delimiter row, `widths[r][c]` the measured width of the
/// content of cell `c` of row `r`.
pub fn table_pads(
    rows: &[Vec<Cell>],
    widths: &[Vec<f32>],
    aligns: &[Align],
    text: &str,
) -> Vec<Pad> {
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut col_w = vec![0.0f32; ncols];
    for w in widths {
        for (c, x) in w.iter().enumerate() {
            col_w[c] = col_w[c].max(*x);
        }
    }
    let mut out = Vec::new();
    let before = |at: usize| -> usize {
        // Start byte of the char ending just before `at`.
        text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
    };
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let extra = col_w[c] - widths[r].get(c).copied().unwrap_or(0.0);
            if extra < 0.5 || cell.region.is_empty() {
                continue;
            }
            let align = aligns.get(c).copied().unwrap_or(Align::Left);
            let lead = before(cell.content.start.max(cell.region.start + 1));
            let trail = before(cell.region.end);
            match align {
                Align::Left => out.push(Pad {
                    at: trail,
                    amount: extra,
                }),
                Align::Right => out.push(Pad {
                    at: lead,
                    amount: extra,
                }),
                Align::Center => {
                    out.push(Pad {
                        at: lead,
                        amount: extra / 2.0,
                    });
                    out.push(Pad {
                        at: trail,
                        amount: extra / 2.0,
                    });
                }
            }
        }
    }
    out
}

/// Text of the cell content with inline markers removed (what is shown).
pub fn shown_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("[[") {
            let end = r.find("]]").unwrap_or(r.len());
            let inner = &r[..end];
            out.push_str(inner.rsplit('|').next().unwrap_or(inner));
            rest = r.get(end + 2..).unwrap_or("");
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        if !matches!(c, '*' | '`' | '=' | '~') {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::live::decorate;

    const TABLE: &str = "| Name | Role | Score |\n|------|:----:|------:|\n| Ada  | Maths | 10 |\n";

    #[test]
    fn detects_code_blocks_and_tables() {
        let t = format!("intro\n```rust\nlet x = 1;\n```\n{TABLE}fin\n");
        let d = decorate(&t);
        let b = find_blocks(&t, &d.lines);
        assert_eq!(b.len(), 2);
        assert_eq!(
            b[0],
            BlockSpan {
                kind: BlockKind::Code {
                    lang: "rust".into()
                },
                start: 1,
                end: 3
            }
        );
        assert_eq!((b[1].start, b[1].end), (4, 6));
        assert!(matches!(&b[1].kind, BlockKind::Table { aligns }
            if aligns == &[Align::Left, Align::Center, Align::Right]));
    }

    #[test]
    fn unclosed_fence_runs_to_the_end_and_pipes_alone_are_not_tables() {
        let t = "```py\nx\n";
        let d = decorate(t);
        let b = find_blocks(t, &d.lines);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].end, d.lines.len() - 1);
        let t = "a | b\nc | d\n";
        assert!(find_blocks(t, &decorate(t).lines).is_empty());
    }

    #[test]
    fn bars_ignore_escapes_code_and_wikilinks() {
        let l = r"| a \| b | `c|d` | [[x|y]] |";
        assert_eq!(bars(l, 0).len(), 4);
    }

    #[test]
    fn cells_and_delimiter() {
        let t = "| Ada  | Maths |";
        let c = cells(t, 0..t.len());
        assert_eq!(c.len(), 2);
        assert_eq!(&t[c[0].content.clone()], "Ada");
        assert_eq!(&t[c[1].content.clone()], "Maths");
        assert!(delimiter_row("|---|:-:|").is_some());
        assert!(delimiter_row("| a | b |").is_none());
        assert!(delimiter_row("---").is_none() || delimiter_row("---").is_some());
    }

    #[test]
    fn expand_and_collapse() {
        let t = format!("x\n```rust\nlet x = 1;\n```\n{TABLE}");
        let d = decorate(&t);
        let b = find_blocks(&t, &d.lines);
        assert_eq!(expand_active(2..3, &b), 1..4);
        assert_eq!(expand_active(0..1, &b), 0..1);
        assert_eq!(expand_active(0..0, &b), 0..0);
        let hidden = collapsed_lines(&b, &(0..1));
        assert!(hidden.contains(&1) && hidden.contains(&3) && hidden.contains(&5));
        let hidden = collapsed_lines(&b, &(1..4));
        assert!(!hidden.contains(&1) && hidden.contains(&5));
    }

    #[test]
    fn math_and_footnotes() {
        assert_eq!(math_spans("a $x^2$ b", 10), vec![12..17]);
        assert!(math_spans("costs $5 and $6", 0).is_empty());
        assert_eq!(math_spans("$$a+b$$", 0), vec![0..7]);
        assert!(math_spans("`$x$`", 0).is_empty());
        assert_eq!(footnote_refs("see[^1] and[^n]", 0), vec![3..7, 11..15]);
        assert!(footnote_refs("[^1]: definition", 0).is_empty());
    }

    #[test]
    fn list_guides_and_depth() {
        let t = "- a\n  - b\n    - c\n- d\n";
        let d = decorate(t);
        let (g, depth) = list_guides(t, &d.lines);
        assert_eq!(depth, vec![(0, 0), (1, 1), (2, 2), (3, 0)]);
        assert_eq!(
            g,
            vec![
                Guide { line: 1, at: 0 },
                Guide { line: 2, at: 0 },
                Guide { line: 2, at: 6 }
            ]
        );
    }

    #[test]
    fn table_padding_aligns_columns() {
        let t = "| a | long |\n| bbb | c |";
        let l0 = 0..t.find('\n').unwrap();
        let l1 = t.find('\n').unwrap() + 1..t.len();
        let rows = vec![cells(t, l0), cells(t, l1)];
        let widths = vec![vec![10.0, 40.0], vec![30.0, 10.0]];
        let pads = table_pads(&rows, &widths, &[Align::Left, Align::Right], t);
        // Row 0, column 0 gets 20 after its trailing space; row 1, column 1
        // is right-aligned: 30 before its content.
        assert!(
            pads.contains(&Pad {
                at: 3,
                amount: 20.0
            }),
            "{pads:?}"
        );
        assert!(
            pads.iter()
                .any(|p| p.amount == 30.0 && t.is_char_boundary(p.at))
        );
    }

    #[test]
    fn ordered_lists_line_up() {
        let t = "1. a\n2. b\n10. c\n\ntext\n9. x\n";
        let d = decorate(t);
        // `1.` and `2.` miss one digit; the second list has no padding.
        assert_eq!(ordered_runs(t, &d.lines), vec![(2, 1), (7, 1)]);
    }

    #[test]
    fn shown_text_strips_markers() {
        assert_eq!(shown_text("**b** `c` [[x|y]]"), "b c y");
    }
}
