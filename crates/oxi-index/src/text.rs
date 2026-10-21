//! Small text helpers shared by the cache, the search engine and refactoring:
//! comparison keys, case folding with offset mapping, path arithmetic and
//! percent-encoding.

use unicode_normalization::UnicodeNormalization;

/// Comparison key used for link resolution and tag lookup: Unicode NFC, then
/// lowercased.
pub(crate) fn key(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    nfc.to_lowercase()
}

/// Strip a trailing `.md` (case-insensitive) from a key.
pub(crate) fn strip_md(s: &str) -> &str {
    let len = s.len();
    if len > 3 && s.is_char_boundary(len - 3) && s[len - 3..].eq_ignore_ascii_case(".md") {
        &s[..len - 3]
    } else {
        s
    }
}

/// Whether `s` ends with `.md` (case-insensitive).
pub(crate) fn has_md(s: &str) -> bool {
    strip_md(s).len() != s.len()
}

/// Last `/`-separated component of a path-like string.
pub(crate) fn last_component(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// The "name key" of a link target or of a file: last component, NFC +
/// lowercase, with `.md` removed. A link can only resolve to a file whose name
/// key equals the link's name key.
pub(crate) fn name_key(s: &str) -> String {
    key(strip_md(last_component(s)))
}

/// Parent folder of a path-like string, `""` for the root.
pub(crate) fn folder_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// Join `rel` onto `folder`, resolving `.` and `..` components. Returns `None`
/// if the path escapes the vault root or is empty.
pub(crate) fn join_normalize(folder: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Relative path from `folder` (vault-relative, `""` = root) to `target`
/// (vault-relative), using `../` as needed.
pub(crate) fn relative_path(folder: &str, target: &str) -> String {
    let from: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
    let to: Vec<&str> = target.split('/').filter(|p| !p.is_empty()).collect();
    // Never consume the file name of `to` as a common folder component.
    let max_common = from.len().min(to.len().saturating_sub(1));
    let common = from
        .iter()
        .zip(to.iter())
        .take(max_common)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = vec![".."; from.len() - common];
    out.extend(&to[common..]);
    out.join("/")
}

/// Percent-encode a path for a markdown link: ASCII characters that would
/// break the link syntax (space, parentheses, brackets, `#`, `%`, `?`, `<`,
/// `>`, `^`, `|`) and control characters. Non-ASCII characters are kept as-is
/// (Obsidian accepts them and they stay readable).
pub(crate) fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' | '(' | ')' | '[' | ']' | '#' | '%' | '?' | '<' | '>' | '^' | '|' | '"' | '`' => {
                out.push_str(&format!("%{:02X}", c as u32));
            }
            c if c.is_ascii_control() => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// A case-folded copy of a text with a mapping back to the original byte
/// offsets.
#[derive(Debug, Clone, Default)]
pub(crate) struct Folded {
    /// Lowercased text.
    pub text: String,
    /// `map[i]` = byte offset in the original for byte `i` of `text`
    /// (length `text.len() + 1`). `None` when folding preserved every byte
    /// offset (the common case), i.e. the mapping is the identity.
    map: Option<Vec<usize>>,
}

impl Folded {
    pub fn new(original: &str) -> Self {
        let mut text = String::with_capacity(original.len());
        let mut map: Option<Vec<usize>> = None;
        for (i, c) in original.char_indices() {
            let start = text.len();
            for l in c.to_lowercase() {
                text.push(l);
            }
            let produced = text.len() - start;
            if map.is_none() && (produced != c.len_utf8() || start != i) {
                // Switch to an explicit mapping, filling the identity so far.
                map = Some((0..start).collect());
            }
            if let Some(m) = map.as_mut() {
                // Every byte of the folded char maps to the start of the original
                // char, except we keep the first byte precise.
                for _ in 0..produced {
                    m.push(i);
                }
            }
        }
        if let Some(m) = map.as_mut() {
            m.push(original.len());
        }
        Self { text, map }
    }

    /// Original byte offset of folded byte offset `i`.
    pub fn to_original(&self, i: usize) -> usize {
        match &self.map {
            None => i,
            Some(m) => m.get(i).copied().unwrap_or(i),
        }
    }

    /// Folded byte offset corresponding to original byte offset `o` (the first
    /// folded byte whose original offset is `>= o`).
    pub fn to_folded(&self, o: usize) -> usize {
        match &self.map {
            None => o.min(self.text.len()),
            Some(m) => m.partition_point(|&x| x < o).min(self.text.len()),
        }
    }
}

/// Whether `c` counts as a word character for whole-word matching.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte offsets of whole-word occurrences of `needle` (already folded) inside
/// `hay` (already folded).
pub(crate) fn find_whole_words(hay: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let s = from + pos;
        let e = s + needle.len();
        let before_ok = hay[..s]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c));
        let after_ok = hay[e..].chars().next().is_none_or(|c| !is_word_char(c));
        if before_ok && after_ok {
            out.push((s, e));
        }
        // Advance by one char.
        let step = hay[s..].chars().next().map_or(1, char::len_utf8);
        from = s + step;
    }
    out
}

/// Byte offset of the start of each line (`0` first).
pub(crate) fn line_starts(s: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(s.match_indices('\n').map(|(i, _)| i + 1));
    v
}

/// Index of the line containing byte `offset`.
pub(crate) fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset).saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_nfc_lowercase() {
        // "é" decomposed (e + combining acute) vs precomposed.
        assert_eq!(key("Ide\u{301}es"), key("Idées"));
        assert_eq!(key("NOTE"), "note");
    }

    #[test]
    fn md_helpers() {
        assert_eq!(strip_md("Note.MD"), "Note");
        assert_eq!(strip_md("a.png"), "a.png");
        assert_eq!(strip_md(".md"), ".md");
        assert!(has_md("x.md"));
        assert_eq!(name_key("Folder/My Note.md"), "my note");
        assert_eq!(name_key("img.PNG"), "img.png");
    }

    #[test]
    fn join_and_relative() {
        assert_eq!(join_normalize("a/b", "../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(join_normalize("", "./x.md").as_deref(), Some("x.md"));
        assert_eq!(join_normalize("", "../x.md"), None);
        assert_eq!(relative_path("a/b", "a/c/d.md"), "../c/d.md");
        assert_eq!(relative_path("", "a/d.md"), "a/d.md");
        assert_eq!(relative_path("a", "a/d.md"), "d.md");
        assert_eq!(relative_path("a/b", "x.md"), "../../x.md");
        assert_eq!(relative_path("a", "a"), "../a");
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode_path("Rust notes.md"), "Rust%20notes.md");
        assert_eq!(percent_encode_path("a(b)#c"), "a%28b%29%23c");
        assert_eq!(percent_encode_path("Idées.md"), "Idées.md");
    }

    #[test]
    fn folding_maps_offsets() {
        let f = Folded::new("Hello WORLD");
        assert_eq!(f.text, "hello world");
        assert_eq!(f.to_original(6), 6);
        // 'İ' (2 bytes) lowercases to "i̇" (3 bytes).
        let src = "aİb";
        let f = Folded::new(src);
        assert_eq!(f.text, "ai\u{307}b");
        let pos = f.text.find('b').unwrap();
        assert_eq!(&src[f.to_original(pos)..], "b");
        assert_eq!(f.to_folded(src.find('b').unwrap()), pos);
    }

    #[test]
    fn whole_words() {
        let hay = "rust rusty trust rust_x (rust)";
        assert_eq!(find_whole_words(hay, "rust"), vec![(0, 4), (25, 29)]);
        assert!(find_whole_words(hay, "").is_empty());
    }

    #[test]
    fn lines() {
        let s = "a\nbc\n\nd";
        let st = line_starts(s);
        assert_eq!(st, vec![0, 2, 5, 6]);
        assert_eq!(line_of(&st, 0), 0);
        assert_eq!(line_of(&st, 3), 1);
        assert_eq!(line_of(&st, 6), 3);
    }
}
