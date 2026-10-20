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
                None => (Kind::Link, at - inner.len()),
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
