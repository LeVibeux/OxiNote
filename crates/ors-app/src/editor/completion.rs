//! Autocompletion contexts (what is being typed at the caret) and the text
//! inserted when a suggestion is accepted.
//!
//! - `[[query` → notes and aliases; `![[query` → files incl. attachments;
//! - `[[Note#query` → headings of `Note` (current note when empty);
//! - `[[Note#^query` → block ids of `Note`;
//! - `#query` → tags.

use super::{Sel, line_start};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Link,
    Heading { note: String },
    Block { note: String },
    Tag,
}

/// What is being completed at the caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub kind: Kind,
    /// `![[` rather than `[[`.
    pub embed: bool,
    /// Byte offset of the opening `[[` / `![[` / `#`.
    pub open: usize,
    /// Byte offset where the query starts.
    pub start: usize,
    /// Byte offset of the caret (query end).
    pub end: usize,
    pub query: String,
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

/// Completion context for a caret at byte `at`.
pub fn detect(text: &str, at: usize) -> Option<Context> {
    if at > text.len() || !text.is_char_boundary(at) {
        return None;
    }
    let ls = line_start(text, at);
    let before = &text[ls..at];
    if let Some(rel) = before.rfind("[[") {
        let open = ls + rel;
        let inner = &text[open + 2..at];
        if !inner.contains("]]") && !inner.contains('|') && inner.len() <= 120 {
            let embed = open > 0 && text.as_bytes()[open - 1] == b'!';
            let open = if embed { open - 1 } else { open };
            let (kind, start) = match inner.rfind('#') {
                Some(h) => {
                    let note = inner[..inner.find('#').unwrap_or(h)].to_string();
                    let after = &inner[h + 1..];
                    if after.starts_with('^') {
                        (Kind::Block { note }, at - after.len() + 1)
                    } else {
                        (Kind::Heading { note }, at - after.len())
                    }
                }
                None => match inner.rfind('^') {
                    // `[[Note^`: block of a note, without the `#`.
                    Some(c) if !inner[..c].is_empty() => (
                        Kind::Block {
                            note: inner[..c].to_string(),
                        },
                        at - (inner.len() - c - 1),
                    ),
                    _ => (Kind::Link, at - inner.len()),
                },
            };
            return Some(Context {
                kind,
                embed,
                open,
                start,
                end: at,
                query: text[start..at].to_string(),
            });
        }
    }
    // `#tag` (at least one tag character typed, so that headings don't trigger).
    let query_len: usize = before
        .chars()
        .rev()
        .take_while(|c| is_tag_char(*c))
        .map(char::len_utf8)
        .sum();
    if query_len == 0 {
        return None;
    }
    let hash = at - query_len - 1;
    if hash < ls || text.as_bytes().get(hash) != Some(&b'#') {
        return None;
    }
    let prev_ok = hash == ls || text[..hash].ends_with(char::is_whitespace);
    let query = &text[hash + 1..at];
    if !prev_ok || query.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(Context {
        kind: Kind::Tag,
        embed: false,
        open: hash,
        start: hash + 1,
        end: at,
        query: query.to_string(),
    })
}

/// A block id of a note with the text of its block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockItem {
    pub id: String,
    pub excerpt: String,
}

/// Text of the block carrying `^id` on `line` (0-based): its paragraph
/// without the id and the list / quote / heading markers, shortened.
pub fn block_excerpt(text: &str, line: usize, id: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(&at) = lines.get(line) else {
        return String::new();
    };
    let clean = |l: &str| -> String {
        let l = l.replace(&format!("^{id}"), "");
        let mut t = l.trim();
        loop {
            let before = t;
            t = t.trim_start_matches(['>', '#']).trim_start();
            for m in ["- [ ] ", "- [x] ", "- ", "* ", "+ "] {
                if let Some(r) = t.strip_prefix(m) {
                    t = r.trim_start();
                }
            }
            if t == before {
                break;
            }
        }
        t.to_string()
    };
    let mut first = line;
    let mut body = clean(at);
    // The id alone on its line (after a table, a list...) belongs to the
    // paragraph above.
    if body.is_empty() {
        while first > 0 && lines[first - 1].trim().is_empty() {
            first -= 1;
        }
        if first == 0 {
            return String::new();
        }
        first -= 1;
        body = clean(lines[first]);
    }
    // Earlier lines of the same paragraph (at most 2 more).
    let mut parts = vec![body];
    for k in 1..=2 {
        let Some(i) = first.checked_sub(k) else { break };
        let l = lines[i];
        if l.trim().is_empty()
            || l.trim_start().starts_with(['#', '>', '-', '*', '+'])
            || l.starts_with("---")
        {
            break;
        }
        parts.insert(0, clean(l));
    }
    let joined = parts.join(" ");
    let mut out: String = joined.chars().take(80).collect();
    if joined.chars().count() > 80 {
        out.push('…');
    }
    out
}

/// Blocks of a note matching `query` (id or text, case-insensitive).
/// `blocks` are `(id, 0-based line)`.
pub fn block_items(
    text: &str,
    blocks: &[(String, usize)],
    query: &str,
    limit: usize,
) -> Vec<BlockItem> {
    let q = query.trim().trim_start_matches('^').to_lowercase();
    blocks
        .iter()
        .map(|(id, line)| BlockItem {
            id: id.clone(),
            excerpt: block_excerpt(text, *line, id),
        })
        .filter(|b| {
            q.is_empty()
                || b.id.to_lowercase().contains(&q)
                || b.excerpt.to_lowercase().contains(&q)
        })
        .take(limit)
        .collect()
}

/// Heading text as usable in a link subpath (Obsidian drops `#|^[]`).
pub fn heading_for_link(h: &str) -> String {
    h.chars()
        .map(|c| if "#|^[]".contains(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replace the query with `insert` inside `[[...]]`, adding the closing
/// `]]` if missing. Caret goes after `]]`.
pub fn accept_in_link(text: &mut String, ctx: &Context, insert: &str) -> Sel {
    let end = ctx.end.min(text.len());
    text.replace_range(ctx.start..end, insert);
    let after = ctx.start + insert.len();
    if !text[after..].starts_with("]]") {
        text.insert_str(after, "]]");
    }
    Sel::caret(after + 2)
}

/// Replace the whole `[[query]]` (or `![[query]]`) with `markup` (used for
/// markdown links). Caret goes after it.
pub fn accept_replacing_link(text: &mut String, ctx: &Context, markup: &str) -> Sel {
    let mut end = ctx.end.min(text.len());
    if text[end..].starts_with("]]") {
        end += 2;
    }
    text.replace_range(ctx.open..end, markup);
    Sel::caret(ctx.open + markup.len())
}

/// Replace a `#query` with `#tag ` (a space is added unless one follows).
pub fn accept_tag(text: &mut String, ctx: &Context, tag: &str) -> Sel {
    let end = ctx.end.min(text.len());
    let space = if text[end..].starts_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let ins = format!("{tag}{space}");
    text.replace_range(ctx.start..end, &ins);
    Sel::caret(ctx.start + ins.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(text: &str) -> Option<Context> {
        detect(text, text.len())
    }

    #[test]
    fn detects_links() {
        let c = ctx("see [[Ab").unwrap();
        assert_eq!(
            (c.kind, c.embed, c.open, c.start),
            (Kind::Link, false, 4, 6)
        );
        assert_eq!(c.query, "Ab");
        let c = ctx("x ![[ima").unwrap();
        assert!(c.embed);
        assert_eq!(c.open, 2);
        assert_eq!(ctx("see [[x]] y"), None);
        assert_eq!(ctx("[[a|b"), None);
        assert!(ctx("[[a\nb").is_none_or(|c| c.kind == Kind::Tag));
        // UTF-8.
        let c = ctx("é [[Idé").unwrap();
        assert_eq!(c.query, "Idé");
        assert_eq!(c.start, 5);
        // Caret in the middle of a text.
        let t = "a [[No]] b";
        assert_eq!(detect(t, 6).unwrap().query, "No");
    }

    #[test]
    fn detects_headings_and_blocks() {
        let c = ctx("[[Idées#Fonc").unwrap();
        assert_eq!(
            c.kind,
            Kind::Heading {
                note: "Idées".into()
            }
        );
        assert_eq!(c.query, "Fonc");
        let c = ctx("[[#").unwrap();
        assert_eq!(
            c.kind,
            Kind::Heading {
                note: String::new()
            }
        );
        assert_eq!(c.query, "");
        let c = ctx("![[Idées#^gra").unwrap();
        assert_eq!(
            c.kind,
            Kind::Block {
                note: "Idées".into()
            }
        );
        assert_eq!(c.query, "gra");
        assert!(c.embed);
    }

    #[test]
    fn detects_blocks_without_hash() {
        let c = ctx("[[Idées^gr").unwrap();
        assert_eq!(
            c.kind,
            Kind::Block {
                note: "Idées".into()
            }
        );
        assert_eq!(c.query, "gr");
        assert_eq!(c.start, "[[Idées^".len());
        // `^` first (no note) stays a plain link query.
        assert_eq!(ctx("[[^x").unwrap().kind, Kind::Link);
        let mut t = "voir [[Idées#^gr".to_string();
        let c = ctx(&t).unwrap();
        let s = accept_in_link(&mut t, &c, "graphe");
        assert_eq!(t, "voir [[Idées#^graphe]]");
        assert_eq!(s, Sel::caret(t.len()));
    }

    #[test]
    fn block_candidates() {
        let t = "# T\n\nPremière ligne\nsuite du bloc ^abc\n\n- [ ] une tâche ^t-1\n\n> citation\n\n^seul\n\nlibre\n";
        let blocks = vec![
            ("abc".to_string(), 3),
            ("t-1".to_string(), 5),
            ("seul".to_string(), 9),
        ];
        let all = block_items(t, &blocks, "", 10);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].excerpt, "Première ligne suite du bloc");
        assert_eq!(all[1].excerpt, "une tâche");
        // Id alone on its line: the paragraph above.
        assert_eq!(all[2].excerpt, "citation");
        // Filter on the id or on the text.
        let f = block_items(t, &blocks, "T-1", 10);
        assert_eq!(f.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["t-1"]);
        let f = block_items(t, &blocks, "^citat", 10);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "seul");
        assert_eq!(block_items(t, &blocks, "", 2).len(), 2);
        // Long blocks are shortened.
        let long = format!("{} ^l", "mot ".repeat(40));
        let e = block_excerpt(&long, 0, "l");
        assert!(e.ends_with('…') && e.chars().count() == 81);
        assert_eq!(block_excerpt("x", 5, "l"), "");
    }

    #[test]
    fn detects_tags() {
        let c = ctx("text #pro").unwrap();
        assert_eq!(c.kind, Kind::Tag);
        assert_eq!((c.open, c.start, c.query.as_str()), (5, 6, "pro"));
        assert_eq!(ctx("#idé").unwrap().query, "idé");
        assert_eq!(ctx("# ").map(|c| c.kind), None);
        assert_eq!(ctx("a#b"), None);
        assert_eq!(ctx("#12"), None);
        assert_eq!(ctx("#"), None);
        assert_eq!(ctx("x #a/b").unwrap().query, "a/b");
    }

    #[test]
    fn accepting() {
        let mut t = "see [[Ru".to_string();
        let c = ctx(&t).unwrap();
        let s = accept_in_link(&mut t, &c, "Rust notes");
        assert_eq!(t, "see [[Rust notes]]");
        assert_eq!(s, Sel::caret(t.len()));

        let mut t = "a [[Ru]] b".to_string();
        let c = detect(&t, 6).unwrap();
        let s = accept_in_link(&mut t, &c, "Rust|alias");
        assert_eq!(t, "a [[Rust|alias]] b");
        assert_eq!(s, Sel::caret(16));

        let mut t = "x ![[dia]]".to_string();
        let c = detect(&t, 8).unwrap();
        let s = accept_replacing_link(&mut t, &c, "![diagramme.png](attachments/diagramme.png)");
        assert_eq!(t, "x ![diagramme.png](attachments/diagramme.png)");
        assert_eq!(s, Sel::caret(t.len()));

        let mut t = "x #pr".to_string();
        let c = ctx(&t).unwrap();
        let s = accept_tag(&mut t, &c, "projet/obsirust");
        assert_eq!(t, "x #projet/obsirust ");
        assert_eq!(s, Sel::caret(t.len()));

        assert_eq!(heading_for_link("A #b | c"), "A b c");
    }
}
