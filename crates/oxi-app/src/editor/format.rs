//! Pure text transformations behind the editing shortcuts (Obsidian
//! behaviour): wrap/unwrap markers, links, checklists, list indentation and
//! continuation, auto-pairing, moving lines, smart paste.
//!
//! Each function edits `text` in place and returns the new selection, or
//! `None` when it does not apply (the caller then falls back to the default
//! behaviour and `text` is unchanged).

use std::ops::Range;

use super::{Sel, line_end, line_start};

/// Indentation unit inserted by Tab / Ctrl+] (Obsidian's default `useTab`).
pub const INDENT: &str = "\t";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// `-`, `*` or `+`.
    Bullet(char),
    /// `1.` or `1)`.
    Ordered { n: u64, delim: char },
    /// `>` (blockquote / callout continuation).
    Quote,
}

/// The list/quote prefix of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefix {
    /// Byte length of the leading whitespace.
    pub indent: usize,
    pub kind: ListKind,
    /// End of the marker including the space after it (`- `, `1. `, `> `).
    pub marker_end: usize,
    /// Task status char (`- [x] `).
    pub task: Option<char>,
    /// Byte offset of the status char inside the line (if a task).
    pub task_at: usize,
    /// End of the whole prefix (content starts here).
    pub end: usize,
}

/// Parse the list / task / quote prefix of a single line.
pub fn parse_prefix(line: &str) -> Option<Prefix> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    let b = rest.as_bytes();
    let (kind, marker_end) = match b.first()? {
        c @ (b'-' | b'*' | b'+') if matches!(b.get(1), Some(b' ' | b'\t')) => {
            (ListKind::Bullet(*c as char), indent + 2)
        }
        b'>' => {
            let sp = usize::from(b.get(1) == Some(&b' '));
            (ListKind::Quote, indent + 1 + sp)
        }
        c if c.is_ascii_digit() => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let delim = *b.get(digits)?;
            if digits > 9 || !matches!(delim, b'.' | b')') || b.get(digits + 1) != Some(&b' ') {
                return None;
            }
            let n = rest[..digits].parse().ok()?;
            (
                ListKind::Ordered {
                    n,
                    delim: delim as char,
                },
                indent + digits + 2,
            )
        }
        _ => return None,
    };
    let mut p = Prefix {
        indent,
        kind,
        marker_end,
        task: None,
        task_at: 0,
        end: marker_end,
    };
    if !matches!(kind, ListKind::Quote) {
        let after = &line[marker_end..];
        let mut chars = after.char_indices();
        if let (Some((_, '[')), Some((_, c)), Some((k, ']'))) =
            (chars.next(), chars.next(), chars.next())
            && c != '\n'
        {
            let close = marker_end + k + 1;
            match line[close..].chars().next() {
                None => {
                    p.task = Some(c);
                    p.task_at = marker_end + 1;
                    p.end = close;
                }
                Some(' ') => {
                    p.task = Some(c);
                    p.task_at = marker_end + 1;
                    p.end = close + 1;
                }
                _ => {}
            }
        }
    }
    Some(p)
}

/// Apply non-overlapping `edits` (original coordinates) and map `sel`
/// through them. Insertions at the caret push it right.
pub fn apply_edits(text: &mut String, mut edits: Vec<(Range<usize>, String)>, sel: Sel) -> Sel {
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    let map = |pos: usize| {
        let mut out = pos as isize;
        for (r, s) in &edits {
            let delta = s.len() as isize - r.len() as isize;
            if r.is_empty() {
                if r.start <= pos {
                    out += delta;
                }
            } else if r.end <= pos {
                out += delta;
            } else if r.start < pos {
                // Inside a replaced range: clamp to its new end.
                let inside = (pos - r.start).min(s.len());
                out += inside as isize - (pos - r.start) as isize;
            }
        }
        out.max(0) as usize
    };
    let new = Sel::new(map(sel.start), map(sel.end));
    for (r, s) in edits.iter().rev() {
        text.replace_range(r.clone(), s);
    }
    new
}

/// Byte ranges of the lines touched by `sel` (a selection ending right at a
/// line start does not include that line).
pub fn touched_lines(text: &str, sel: Sel) -> Vec<Range<usize>> {
    let first = line_start(text, sel.start);
    let mut last_at = sel.end;
    if sel.end > sel.start && sel.end == line_start(text, sel.end) && sel.end > first {
        last_at = sel.end - 1;
    }
    let mut out = Vec::new();
    let mut s = first;
    loop {
        let e = line_end(text, s);
        out.push(s..e);
        if e >= last_at.max(s) || e >= text.len() {
            break;
        }
        s = e + 1;
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Word around byte `at` (empty range if none).
pub fn word_at(text: &str, at: usize) -> Range<usize> {
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_word_char(*c))
        .map_or(text.len(), |(i, _)| at + i);
    start..end
}

/// Ctrl+B / Ctrl+I / ...: wrap the selection (or the word at the caret) in
/// `marker`, or unwrap it if already wrapped.
pub fn toggle_wrap(text: &mut String, sel: Sel, marker: &str) -> Sel {
    let m = marker.len();
    let mut sel = sel;
    if sel.is_empty() {
        let at = sel.start;
        if text[..at].ends_with(marker) && text[at..].starts_with(marker) {
            text.replace_range(at..at + m, "");
            text.replace_range(at - m..at, "");
            return Sel::caret(at - m);
        }
        let w = word_at(text, at);
        if w.is_empty() {
            text.insert_str(at, &marker.repeat(2));
            return Sel::caret(at + m);
        }
        sel = Sel::new(w.start, w.end);
    }
    let (s, e) = (sel.start, sel.end);
    if s >= m && text[..s].ends_with(marker) && text[e..].starts_with(marker) {
        text.replace_range(e..e + m, "");
        text.replace_range(s - m..s, "");
        return Sel::new(s - m, e - m);
    }
    let inner = &text[s..e];
    if inner.len() >= 2 * m && inner.starts_with(marker) && inner.ends_with(marker) {
        text.replace_range(e - m..e, "");
        text.replace_range(s..s + m, "");
        return Sel::new(s, e - 2 * m);
    }
    text.insert_str(e, marker);
    text.insert_str(s, marker);
    Sel::new(s + m, e + m)
}

/// Does `s` look like a URL?
pub fn is_url(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && [
            "http://",
            "https://",
            "ftp://",
            "mailto:",
            "file://",
            "obsidian://",
        ]
        .iter()
        .any(|p| s.to_ascii_lowercase().starts_with(p))
}

/// Ctrl+K: insert a markdown link around the selection.
pub fn insert_link(text: &mut String, sel: Sel) -> Sel {
    let selected = text[sel.start..sel.end].to_string();
    if is_url(&selected) {
        text.replace_range(sel.start..sel.end, &format!("[]({})", selected.trim()));
        return Sel::caret(sel.start + 1);
    }
    let ins = format!("[{selected}]()");
    text.replace_range(sel.start..sel.end, &ins);
    if selected.is_empty() {
        Sel::caret(sel.start + 1)
    } else {
        Sel::caret(sel.start + ins.len() - 1)
    }
}

/// Insert `[[selection]]`, caret inside.
pub fn insert_wikilink(text: &mut String, sel: Sel) -> Sel {
    let selected = text[sel.start..sel.end].to_string();
    text.replace_range(sel.start..sel.end, &format!("[[{selected}]]"));
    Sel::caret(sel.start + 2 + selected.len())
}

/// Replace the selection with `s`, caret after it.
pub fn insert(text: &mut String, sel: Sel, s: &str) -> Sel {
    text.replace_range(sel.start..sel.end, s);
    Sel::caret(sel.start + s.len())
}

/// Ctrl+L: cycle each touched line plain → `- [ ] ` → `- [x] ` → `- [ ] `.
pub fn toggle_checklist(text: &mut String, sel: Sel) -> Sel {
    let mut edits = Vec::new();
    for r in touched_lines(text, sel) {
        let line = &text[r.clone()];
        let s = r.start;
        match parse_prefix(line) {
            Some(p) if p.task.is_some() => {
                let c = p.task.unwrap_or(' ');
                let next = if c == ' ' { "x" } else { " " };
                edits.push((s + p.task_at..s + p.task_at + c.len_utf8(), next.into()));
            }
            Some(p) if matches!(p.kind, ListKind::Quote) => {
                edits.push((s + p.end..s + p.end, "- [ ] ".into()));
            }
            Some(p) => edits.push((s + p.marker_end..s + p.marker_end, "[ ] ".into())),
            None => {
                let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                edits.push((s + indent..s + indent, "- [ ] ".into()));
            }
        }
    }
    apply_edits(text, edits, sel)
}

/// Ctrl+Enter: toggle the checkbox of task lines (`[ ]` ↔ `[x]`); other
/// lines become tasks.
pub fn toggle_checkbox(text: &mut String, sel: Sel) -> Sel {
    toggle_checklist(text, sel)
}

/// Click on a rendered checkbox: toggle the status char of the task on
/// 0-based `line` (`[ ]` becomes `[x]`, any other status becomes `[ ]`).
/// Returns `text` unchanged when the line is not a task.
pub fn toggle_task_at(text: &str, line: usize) -> String {
    let mut start = 0;
    for (n, l) in text.split('\n').enumerate() {
        if n == line {
            let content = l.trim_end_matches('\r');
            if let Some(p) = parse_prefix(content)
                && let Some(c) = p.task
            {
                let at = start + p.task_at;
                let next = if c == ' ' { "x" } else { " " };
                let mut out = text.to_string();
                out.replace_range(at..at + c.len_utf8(), next);
                return out;
            }
            break;
        }
        start += l.len() + 1;
    }
    text.to_string()
}

/// Toggle a bullet (`ordered = false`) or numbered list on touched lines.
pub fn toggle_list(text: &mut String, sel: Sel, ordered: bool) -> Sel {
    let lines = touched_lines(text, sel);
    let all_on = lines.iter().all(|r| {
        matches!(
            (parse_prefix(&text[r.clone()]).map(|p| p.kind), ordered),
            (Some(ListKind::Bullet(_)), false) | (Some(ListKind::Ordered { .. }), true)
        )
    });
    let mut edits = Vec::new();
    for (n, r) in lines.into_iter().enumerate() {
        let line = &text[r.clone()];
        let s = r.start;
        let marker = if ordered {
            format!("{}. ", n + 1)
        } else {
            "- ".to_string()
        };
        match parse_prefix(line) {
            Some(p) if !matches!(p.kind, ListKind::Quote) => {
                let new = if all_on { String::new() } else { marker };
                edits.push((s + p.indent..s + p.marker_end, new));
            }
            Some(p) => edits.push((s + p.end..s + p.end, marker)),
            None => {
                let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                edits.push((s + indent..s + indent, marker));
            }
        }
    }
    apply_edits(text, edits, sel)
}

/// Ctrl+] / Tab: indent every touched line.
pub fn indent_lines(text: &mut String, sel: Sel) -> Sel {
    let edits = touched_lines(text, sel)
        .into_iter()
        .map(|r| (r.start..r.start, INDENT.to_string()))
        .collect();
    apply_edits(text, edits, sel)
}

/// Ctrl+[ / Shift+Tab: remove one indentation level from touched lines.
pub fn outdent_lines(text: &mut String, sel: Sel) -> Option<Sel> {
    let mut edits = Vec::new();
    for r in touched_lines(text, sel) {
        let line = &text[r.clone()];
        let n = if line.starts_with('\t') {
            1
        } else {
            line.bytes().take(4).take_while(|b| *b == b' ').count()
        };
        if n > 0 {
            edits.push((r.start..r.start + n, String::new()));
        }
    }
    if edits.is_empty() {
        return None;
    }
    Some(apply_edits(text, edits, sel))
}

/// Is the caret line a list item (Tab then indents instead of inserting a
/// tab character)?
pub fn in_list(text: &str, sel: Sel) -> bool {
    let s = line_start(text, sel.start);
    parse_prefix(&text[s..line_end(text, s)]).is_some_and(|p| !matches!(p.kind, ListKind::Quote))
}

/// Enter: continue a list / task / blockquote, or end it on an empty item.
pub fn on_enter(text: &mut String, sel: Sel) -> Option<Sel> {
    let ls = line_start(text, sel.start);
    let le = line_end(text, sel.start);
    if sel.end > le {
        return None;
    }
    let line = &text[ls..le];
    let p = parse_prefix(line)?;
    if sel.start < ls + p.marker_end {
        return None;
    }
    if line[p.end..].trim().is_empty() && sel.is_empty() {
        // Empty item: outdent if nested, else remove the marker.
        if p.indent > 0 && !matches!(p.kind, ListKind::Quote) {
            return outdent_lines(text, sel);
        }
        text.replace_range(ls..le, "");
        return Some(Sel::caret(ls));
    }
    let indent = &line[..p.indent];
    let marker = match p.kind {
        ListKind::Bullet(c) => format!("{c} "),
        ListKind::Ordered { n, delim } => format!("{}{delim} ", n + 1),
        ListKind::Quote => line[p.indent..p.marker_end].to_string(),
    };
    let task = if p.task.is_some() { "[ ] " } else { "" };
    let ins = format!("\n{indent}{marker}{task}");
    text.replace_range(sel.start..sel.end, &ins);
    let caret = sel.start + ins.len();
    if let ListKind::Ordered { n, .. } = p.kind {
        let next_line = line_end(text, caret);
        if next_line < text.len() {
            renumber(text, next_line + 1, p.indent, n + 2);
        }
    }
    Some(Sel::caret(caret))
}

/// Renumber consecutive ordered items at `indent` starting at byte `from`
/// (a line start), skipping nested lines.
fn renumber(text: &mut String, from: usize, indent: usize, mut n: u64) {
    let mut s = from;
    while s < text.len() {
        let e = line_end(text, s);
        let line = &text[s..e];
        let Some(p) = parse_prefix(line) else { break };
        if p.indent > indent {
            s = e + 1;
            continue;
        }
        let ListKind::Ordered { n: old, delim } = p.kind else {
            break;
        };
        if p.indent < indent || old == n {
            break;
        }
        let new = format!("{n}{delim} ");
        text.replace_range(s + p.indent..s + p.marker_end, &new);
        n += 1;
        s = line_end(text, s) + 1;
    }
}

const PAIRS: &[(char, char)] = &[('[', ']'), ('(', ')'), ('"', '"'), ('`', '`')];
/// Markers that only wrap a selection (never auto-inserted on their own).
const WRAP_ONLY: &[char] = &['*', '_', '=', '~'];

/// Typing `ch`: auto-pair brackets/quotes, wrap a selection, or skip over
/// an existing closing character.
pub fn auto_pair(text: &mut String, sel: Sel, ch: char) -> Option<Sel> {
    let close = PAIRS.iter().find(|(o, _)| *o == ch).map(|(_, c)| *c);
    if !sel.is_empty() {
        let close = close.or(WRAP_ONLY.contains(&ch).then_some(ch))?;
        text.insert(sel.end, close);
        text.insert(sel.start, ch);
        let l = ch.len_utf8();
        return Some(Sel::new(sel.start + l, sel.end + l));
    }
    let at = sel.start;
    let next = text[at..].chars().next();
    let prev = text[..at].chars().next_back();
    // Skip over a closing char that is already there.
    if matches!(ch, ']' | ')' | '"' | '`') && next == Some(ch) {
        return Some(Sel::caret(at + 1));
    }
    let close = close?;
    let next_ok = next.is_none_or(|c| c.is_whitespace() || ")]}\"'`,.;:!?".contains(c));
    if !next_ok {
        return None;
    }
    if ch == close && prev.is_some_and(|p| p.is_alphanumeric() || p == ch) {
        return None;
    }
    text.insert(at, close);
    text.insert(at, ch);
    Some(Sel::caret(at + ch.len_utf8()))
}

/// Backspace between an empty pair (`[|]`) deletes both characters.
pub fn backspace_pair(text: &mut String, sel: Sel) -> Option<Sel> {
    if !sel.is_empty() || sel.start == 0 {
        return None;
    }
    let at = sel.start;
    let prev = text[..at].chars().next_back()?;
    let next = text[at..].chars().next()?;
    if !PAIRS.iter().any(|&(o, c)| o == prev && c == next) {
        return None;
    }
    text.replace_range(at - prev.len_utf8()..at + next.len_utf8(), "");
    Some(Sel::caret(at - prev.len_utf8()))
}

/// Alt+↑ / Alt+↓: swap the touched lines with the line above/below.
pub fn move_lines(text: &mut String, sel: Sel, up: bool) -> Option<Sel> {
    let lines = touched_lines(text, sel);
    let block = lines.first()?.start..lines.last()?.end;
    if up {
        if block.start == 0 {
            return None;
        }
        let prev = line_start(text, block.start - 1)..block.start - 1;
        let moved = format!("{}\n{}", &text[block.clone()], &text[prev.clone()]);
        text.replace_range(prev.start..block.end, &moved);
        let d = prev.len() + 1;
        Some(Sel::new(sel.start - d, sel.end - d))
    } else {
        if block.end >= text.len() {
            return None;
        }
        let next = block.end + 1..line_end(text, block.end + 1);
        let moved = format!("{}\n{}", &text[next.clone()], &text[block.clone()]);
        text.replace_range(block.start..next.end, &moved);
        let d = next.len() + 1;
        Some(Sel::new(sel.start + d, sel.end + d))
    }
}

/// Paste: a URL pasted over a (non-URL) selection becomes `[sel](url)`.
pub fn smart_paste(text: &mut String, sel: Sel, pasted: &str) -> Option<Sel> {
    if sel.is_empty() || !is_url(pasted) {
        return None;
    }
    let selected = &text[sel.start..sel.end];
    if is_url(selected) || selected.contains('\n') {
        return None;
    }
    let link = format!("[{selected}]({})", pasted.trim());
    Some(insert(text, sel, &link))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `f` on a text where `|` marks the caret (or two `|` a selection),
    /// returning the result with the selection marked the same way.
    fn run(input: &str, f: impl FnOnce(&mut String, Sel) -> Option<Sel>) -> String {
        let a = input.find('|').expect("caret");
        let mut text = input.replacen('|', "", 1);
        let b = text.find('|');
        if let Some(b) = b {
            text.replace_range(b..b + 1, "");
        }
        let sel = Sel::new(a, b.unwrap_or(a));
        let original = text.clone();
        match f(&mut text, sel) {
            Some(s) => {
                let mut out = text.clone();
                if s.is_empty() {
                    out.insert(s.start, '|');
                } else {
                    out.insert(s.end, '|');
                    out.insert(s.start, '|');
                }
                out
            }
            None => {
                assert_eq!(text, original, "unchanged when not applicable");
                "NONE".into()
            }
        }
    }

    #[test]
    fn prefix_parsing() {
        let p = parse_prefix("\t- [x] done").unwrap();
        assert_eq!(p.indent, 1);
        assert_eq!(p.task, Some('x'));
        assert_eq!(p.end, 7);
        let p = parse_prefix("12. item").unwrap();
        assert_eq!(p.kind, ListKind::Ordered { n: 12, delim: '.' });
        assert_eq!(p.end, 4);
        assert_eq!(parse_prefix("> quote").unwrap().kind, ListKind::Quote);
        assert!(parse_prefix("-no").is_none());
        assert!(parse_prefix("plain").is_none());
        assert!(parse_prefix("- [link](x)").unwrap().task.is_none());
        assert_eq!(parse_prefix("- [ ]").unwrap().task, Some(' '));
        assert_eq!(parse_prefix("- [é] x").unwrap().end, 7);
    }

    #[test]
    fn wrap_and_unwrap() {
        assert_eq!(
            run("a |bc| d", |t, s| Some(toggle_wrap(t, s, "**"))),
            "a **|bc|** d"
        );
        assert_eq!(
            run("a **|bc|** d", |t, s| Some(toggle_wrap(t, s, "**"))),
            "a |bc| d"
        );
        assert_eq!(
            run("a |**bc**| d", |t, s| Some(toggle_wrap(t, s, "**"))),
            "a |bc| d"
        );
        // Caret in a word wraps the word (UTF-8).
        assert_eq!(
            run("un mo|té x", |t, s| Some(toggle_wrap(t, s, "*"))),
            "un *|moté|* x"
        );
        // Caret on a blank inserts an empty pair, toggling again removes it.
        assert_eq!(
            run("a | b", |t, s| Some(toggle_wrap(t, s, "=="))),
            "a ==|== b"
        );
        assert_eq!(
            run("a ==|== b", |t, s| Some(toggle_wrap(t, s, "=="))),
            "a | b"
        );
        assert_eq!(run("|", |t, s| Some(toggle_wrap(t, s, "%%"))), "%%|%%");
    }

    #[test]
    fn links() {
        assert_eq!(run("|text|", |t, s| Some(insert_link(t, s))), "[text](|)");
        assert_eq!(run("x |", |t, s| Some(insert_link(t, s))), "x [|]()");
        assert_eq!(
            run("|https://a.b|", |t, s| Some(insert_link(t, s))),
            "[|](https://a.b)"
        );
        assert_eq!(
            run("|Note|", |t, s| Some(insert_wikilink(t, s))),
            "[[Note|]]"
        );
        assert_eq!(
            run("see |docs|", |t, s| smart_paste(t, s, "https://docs.rs")),
            "see [docs](https://docs.rs)|"
        );
        assert_eq!(
            run("see |docs|", |t, s| smart_paste(t, s, "not a url")),
            "NONE"
        );
        assert_eq!(
            run("see |", |t, s| smart_paste(t, s, "https://x.y")),
            "NONE"
        );
    }

    #[test]
    fn checklist_cycle() {
        let f = |t: &mut String, s| Some(toggle_checklist(t, s));
        assert_eq!(run("tâche|", f), "- [ ] tâche|");
        assert_eq!(run("- [ ] tâche|", f), "- [x] tâche|");
        assert_eq!(run("- [x] tâche|", f), "- [ ] tâche|");
        assert_eq!(run("- item|", f), "- [ ] item|");
        assert_eq!(run("\t1. item|", f), "\t1. [ ] item|");
        // Several lines at once.
        assert_eq!(run("|a\n- [ ] b|", f), "- [ ] |a\n- [x] b|");
        assert_eq!(run("|", f), "- [ ] |");
    }

    #[test]
    fn task_toggle_at_line() {
        let t = "# Idées\n- [ ] é\n  - [x] fait\r\n- [/] mi\nrien";
        let a = toggle_task_at(t, 1);
        assert_eq!(a, "# Idées\n- [x] é\n  - [x] fait\r\n- [/] mi\nrien");
        assert_eq!(toggle_task_at(&a, 1), t);
        assert!(toggle_task_at(t, 2).contains("  - [ ] fait"));
        assert!(toggle_task_at(t, 3).contains("- [ ] mi"));
        assert_eq!(toggle_task_at(t, 0), t);
        assert_eq!(toggle_task_at(t, 4), t);
        assert_eq!(toggle_task_at(t, 99), t);
    }

    #[test]
    fn bullet_and_numbered_lists() {
        let bullets = |t: &mut String, s| Some(toggle_list(t, s, false));
        let numbers = |t: &mut String, s| Some(toggle_list(t, s, true));
        assert_eq!(run("|a\nb|", bullets), "- |a\n- b|");
        assert_eq!(run("|- a\n- b|", bullets), "|a\nb|");
        assert_eq!(run("|a\n- b|", numbers), "1. |a\n2. b|");
    }

    #[test]
    fn indentation() {
        assert_eq!(
            run("- a|\n- b", |t, s| Some(indent_lines(t, s))),
            "\t- a|\n- b"
        );
        assert_eq!(
            run("|- a\n- b|\nc", |t, s| Some(indent_lines(t, s))),
            "\t|- a\n\t- b|\nc"
        );
        assert_eq!(run("\t- a|", outdent_lines), "- a|");
        assert_eq!(run("    - a|", outdent_lines), "- a|");
        assert_eq!(run("- a|", outdent_lines), "NONE");
        // Selection ending at a line start does not touch that line.
        assert_eq!(run("|a\n|b", |t, s| Some(indent_lines(t, s))), "\t|a\n|b");
        assert!(in_list("- x", Sel::caret(3)));
        assert!(!in_list("x", Sel::caret(1)));
    }

    #[test]
    fn enter_continues_lists() {
        assert_eq!(run("- é|", on_enter), "- é\n- |");
        assert_eq!(run("* a|", on_enter), "* a\n* |");
        assert_eq!(run("- [x] done|", on_enter), "- [x] done\n- [ ] |");
        assert_eq!(run("\t9. a|", on_enter), "\t9. a\n\t10. |");
        assert_eq!(run("> quoted|", on_enter), "> quoted\n> |");
        // Splitting an item moves the rest to the new item.
        assert_eq!(run("- ab|cd", on_enter), "- ab\n- |cd");
        // Empty item ends the list; nested empty item outdents first.
        assert_eq!(run("- a\n- |", on_enter), "- a\n|");
        assert_eq!(run("- [ ] |", on_enter), "|");
        assert_eq!(run("- a\n\t- |", on_enter), "- a\n- |");
        // Not a list, or caret inside the marker: default behaviour.
        assert_eq!(run("text|", on_enter), "NONE");
        assert_eq!(run("-| a", on_enter), "NONE");
        // Following numbers are renumbered.
        assert_eq!(
            run("1. a|\n2. b\n\t- sub\n3. c\n\nx", on_enter),
            "1. a\n2. |\n3. b\n\t- sub\n4. c\n\nx"
        );
    }

    #[test]
    fn pairs() {
        let pair = |c: char| move |t: &mut String, s| auto_pair(t, s, c);
        assert_eq!(run("a |", pair('[')), "a [|]");
        assert_eq!(run("a [|]", pair('[')), "a [[|]]");
        assert_eq!(run("a [[x|]]", pair(']')), "a [[x]|]");
        assert_eq!(run("a |", pair('(')), "a (|)");
        assert_eq!(run("a |b", pair('(')), "NONE");
        assert_eq!(run("say |", pair('"')), "say \"|\"");
        assert_eq!(run("don|", pair('"')), "NONE");
        assert_eq!(run("|", pair('`')), "`|`");
        assert_eq!(run("`|`", pair('`')), "``|");
        assert_eq!(run("``|", pair('`')), "NONE");
        assert_eq!(run("|mot|", pair('*')), "*|mot|*");
        assert_eq!(run("|mot|", pair('=')), "=|mot|=");
        assert_eq!(run("a |", pair('*')), "NONE");
        assert_eq!(run("|été|", pair('[')), "[|été|]");
        assert_eq!(run("a [|]", backspace_pair), "a |");
        assert_eq!(run("a [x|]", backspace_pair), "NONE");
    }

    #[test]
    fn moving_lines() {
        assert_eq!(run("a\nb|\nc", |t, s| move_lines(t, s, true)), "b|\na\nc");
        assert_eq!(run("a\nb|\nc", |t, s| move_lines(t, s, false)), "a\nc\nb|");
        assert_eq!(run("a|\nb", |t, s| move_lines(t, s, true)), "NONE");
        assert_eq!(run("a\nb|", |t, s| move_lines(t, s, false)), "NONE");
        assert_eq!(
            run("x\n|é\nb|\nc", |t, s| move_lines(t, s, true)),
            "|é\nb|\nx\nc"
        );
    }

    #[test]
    fn edits_map_selection() {
        let mut t = "abc".to_string();
        let s = apply_edits(&mut t, vec![(1..2, "XYZ".into())], Sel::new(0, 3));
        assert_eq!(t, "aXYZc");
        assert_eq!(s, Sel::new(0, 5));
        let mut t = "abc".to_string();
        let s = apply_edits(&mut t, vec![(0..2, String::new())], Sel::caret(1));
        assert_eq!(t, "c");
        assert_eq!(s, Sel::caret(0));
    }
}
