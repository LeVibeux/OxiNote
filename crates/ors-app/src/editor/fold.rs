//! Folding of headings and list items: which lines a fold hides, and a
//! stable way to remember folds while the text is edited.
//!
//! Folds are stored as [`FoldKey`]s (text of the header line + rank among
//! identical header lines) so that they survive edits above them.

use std::collections::HashSet;

use super::format::{ListKind, parse_prefix};
use super::live::{LineInfo, LineKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldKind {
    Heading,
    List,
}

/// A foldable block: line `start` stays visible, lines `start + 1 ..= end`
/// are hidden when folded (0-based line numbers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldRange {
    pub start: usize,
    pub end: usize,
    pub kind: FoldKind,
}

/// Identity of a folded block.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoldKey {
    pub text: String,
    /// Rank among the foldable lines with the same text.
    pub nth: usize,
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// Foldable blocks of a note, sorted by start line. `lines` are the
/// decorated lines of `text` (headings inside code or YAML are not
/// headings).
pub fn fold_ranges(text: &str, lines: &[LineInfo]) -> Vec<FoldRange> {
    let src: Vec<&str> = lines
        .iter()
        .map(|l| text.get(l.range.clone()).unwrap_or(""))
        .collect();
    let blank = |i: usize| src[i].trim().is_empty();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        match &l.kind {
            LineKind::Heading(level) => {
                let mut end = lines.len() - 1;
                for (j, m) in lines.iter().enumerate().skip(i + 1) {
                    if matches!(&m.kind, LineKind::Heading(lv) if lv <= level) {
                        end = j - 1;
                        break;
                    }
                }
                while end > i && blank(end) {
                    end -= 1;
                }
                if end > i {
                    out.push(FoldRange {
                        start: i,
                        end,
                        kind: FoldKind::Heading,
                    });
                }
            }
            LineKind::Normal => {
                let Some(p) = parse_prefix(src[i]) else {
                    continue;
                };
                if !matches!(p.kind, ListKind::Bullet(_) | ListKind::Ordered { .. }) {
                    continue;
                }
                let base = indent_width(src[i]);
                let mut end = i;
                for j in i + 1..lines.len() {
                    if blank(j) {
                        continue;
                    }
                    if indent_width(src[j]) > base && !matches!(lines[j].kind, LineKind::Heading(_))
                    {
                        end = j;
                    } else {
                        break;
                    }
                }
                if end > i {
                    out.push(FoldRange {
                        start: i,
                        end,
                        kind: FoldKind::List,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn header_text<'a>(text: &'a str, lines: &[LineInfo], start: usize) -> &'a str {
    lines
        .get(start)
        .and_then(|l| text.get(l.range.clone()))
        .unwrap_or("")
}

/// Key of the block starting at `ranges[idx]`.
pub fn key_of(text: &str, lines: &[LineInfo], ranges: &[FoldRange], idx: usize) -> FoldKey {
    let t = header_text(text, lines, ranges[idx].start);
    let nth = ranges[..idx]
        .iter()
        .filter(|r| header_text(text, lines, r.start) == t)
        .count();
    FoldKey {
        text: t.to_string(),
        nth,
    }
}

/// Indices (in `ranges`) of the blocks that `keys` designate.
pub fn folded_indices(
    text: &str,
    lines: &[LineInfo],
    ranges: &[FoldRange],
    keys: &[FoldKey],
) -> Vec<usize> {
    let mut out = Vec::new();
    for k in keys {
        let found = ranges
            .iter()
            .enumerate()
            .filter(|(_, r)| header_text(text, lines, r.start) == k.text)
            .nth(k.nth);
        if let Some((i, _)) = found {
            out.push(i);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Lines hidden by the folded blocks `folded` (indices into `ranges`).
pub fn hidden_lines(ranges: &[FoldRange], folded: &[usize]) -> HashSet<usize> {
    let mut out = HashSet::new();
    for &i in folded {
        if let Some(r) = ranges.get(i) {
            out.extend(r.start + 1..=r.end);
        }
    }
    out
}

/// Keys of every foldable block (fold all).
pub fn all_keys(text: &str, lines: &[LineInfo], ranges: &[FoldRange]) -> Vec<FoldKey> {
    (0..ranges.len())
        .map(|i| key_of(text, lines, ranges, i))
        .collect()
}

/// Keys left after unfolding every block that hides `line`.
pub fn reveal_line(
    text: &str,
    lines: &[LineInfo],
    ranges: &[FoldRange],
    keys: &[FoldKey],
    line: usize,
) -> Vec<FoldKey> {
    let folded = folded_indices(text, lines, ranges, keys);
    let keep: Vec<usize> = folded
        .into_iter()
        .filter(|&i| !(ranges[i].start < line && line <= ranges[i].end))
        .collect();
    keep.into_iter()
        .map(|i| key_of(text, lines, ranges, i))
        .collect()
}

/// Toggle the block starting at line `start`.
pub fn toggle(
    text: &str,
    lines: &[LineInfo],
    ranges: &[FoldRange],
    keys: &[FoldKey],
    start: usize,
) -> Vec<FoldKey> {
    let mut folded = folded_indices(text, lines, ranges, keys);
    match ranges.iter().position(|r| r.start == start) {
        Some(i) if folded.contains(&i) => folded.retain(|x| *x != i),
        Some(i) => folded.push(i),
        None => {}
    }
    folded
        .into_iter()
        .map(|i| key_of(text, lines, ranges, i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::live::decorate;

    const T: &str = "# A\ntexte\n## B\nb1\nb2\n\n## C\nc1\n# D\n- p1\n  - p1a\n  - p1b\n- p2\n\n```\n# pas un titre\n```\n";

    fn ranges() -> (crate::editor::live::Decor, Vec<FoldRange>) {
        let d = decorate(T);
        let r = fold_ranges(T, &d.lines);
        (d, r)
    }

    fn pairs(r: &[FoldRange]) -> Vec<(usize, usize, FoldKind)> {
        r.iter().map(|r| (r.start, r.end, r.kind)).collect()
    }

    #[test]
    fn heading_and_list_ranges() {
        let (_, r) = ranges();
        use FoldKind::*;
        assert_eq!(
            pairs(&r),
            vec![
                (0, 7, Heading),  // # A : up to the line before `# D`
                (2, 4, Heading),  // ## B : trailing blank line trimmed
                (6, 7, Heading),  // ## C
                (8, 16, Heading), // # D : list and fenced code, `# pas un titre` is code
                (9, 11, List),    // - p1 and its two children
            ]
        );
    }

    #[test]
    fn keys_survive_edits_above() {
        let (d, r) = ranges();
        let keys = all_keys(T, &d.lines, &r);
        let folded = folded_indices(T, &d.lines, &r, &keys);
        assert_eq!(folded.len(), r.len());
        // Insert a line at the top: every fold is found again, shifted.
        let t2 = format!("avant\n{T}");
        let d2 = decorate(&t2);
        let r2 = fold_ranges(&t2, &d2.lines);
        let f2 = folded_indices(&t2, &d2.lines, &r2, &keys);
        assert_eq!(f2.len(), r.len());
        assert_eq!(r2[f2[0]].start, r[folded[0]].start + 1);
    }

    #[test]
    fn duplicate_titles_have_distinct_keys() {
        let t = "## Notes\nx\n## Notes\ny\n";
        let d = decorate(t);
        let r = fold_ranges(t, &d.lines);
        assert_eq!(r.len(), 2);
        let k0 = key_of(t, &d.lines, &r, 0);
        let k1 = key_of(t, &d.lines, &r, 1);
        assert_ne!(k0, k1);
        assert_eq!(folded_indices(t, &d.lines, &r, &[k1]), vec![1]);
    }

    #[test]
    fn hiding_toggle_and_reveal() {
        let (d, r) = ranges();
        let keys = toggle(T, &d.lines, &r, &[], 2);
        let f = folded_indices(T, &d.lines, &r, &keys);
        let hidden = hidden_lines(&r, &f);
        let mut h: Vec<_> = hidden.into_iter().collect();
        h.sort_unstable();
        assert_eq!(h, vec![3, 4]);
        // Toggling again unfolds.
        assert!(toggle(T, &d.lines, &r, &keys, 2).is_empty());
        // A caret inside the hidden lines reveals the block.
        assert!(reveal_line(T, &d.lines, &r, &keys, 3).is_empty());
        assert_eq!(reveal_line(T, &d.lines, &r, &keys, 1).len(), 1);
        // Toggling something that is not foldable does nothing.
        assert!(toggle(T, &d.lines, &r, &[], 1).is_empty());
    }
}
