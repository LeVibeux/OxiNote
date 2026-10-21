//! Small shared helpers: line index, range sets, escaping, encoding, slugs.

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

/// Maps byte offsets to 0-based line numbers.
pub(crate) struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(source: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1)
    }
}

/// A set of half-open byte ranges supporting fast membership queries.
#[derive(Debug, Default, Clone)]
pub(crate) struct RangeSet {
    ranges: Vec<(usize, usize)>,
}

impl RangeSet {
    pub(crate) fn new(mut ranges: Vec<(usize, usize)>) -> Self {
        ranges.retain(|r| r.0 < r.1);
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match merged.last_mut() {
                Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
                _ => merged.push(r),
            }
        }
        Self { ranges: merged }
    }

    pub(crate) fn union(&self, other: &[(usize, usize)]) -> Self {
        let mut all = self.ranges.clone();
        all.extend_from_slice(other);
        Self::new(all)
    }

    pub(crate) fn contains(&self, pos: usize) -> bool {
        let i = self.ranges.partition_point(|r| r.1 <= pos);
        self.ranges.get(i).is_some_and(|r| r.0 <= pos)
    }

    /// End of the range containing `pos`, if any.
    pub(crate) fn end_containing(&self, pos: usize) -> Option<usize> {
        let i = self.ranges.partition_point(|r| r.1 <= pos);
        self.ranges.get(i).filter(|r| r.0 <= pos).map(|r| r.1)
    }

    /// True if any byte of `start..end` is in the set.
    pub(crate) fn overlaps(&self, start: usize, end: usize) -> bool {
        let i = self.ranges.partition_point(|r| r.1 <= start);
        self.ranges.get(i).is_some_and(|r| r.0 < end)
    }
}

/// Number of consecutive backslashes immediately before `pos`.
pub(crate) fn backslashes_before(bytes: &[u8], pos: usize, floor: usize) -> usize {
    let mut n = 0;
    let mut p = pos;
    while p > floor && bytes[p - 1] == b'\\' {
        n += 1;
        p -= 1;
    }
    n
}

pub(crate) fn is_escaped(bytes: &[u8], pos: usize, floor: usize) -> bool {
    backslashes_before(bytes, pos, floor) % 2 == 1
}

/// Find `needle` in `hay[from..to]`, returning an absolute index.
pub(crate) fn find_in(hay: &[u8], from: usize, to: usize, needle: &[u8]) -> Option<usize> {
    if from >= to || to > hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..to]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Find the last `needle` in `hay[from..to]`, returning an absolute index.
pub(crate) fn rfind_in(hay: &[u8], from: usize, to: usize, needle: &[u8]) -> Option<usize> {
    if from >= to || to > hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..to]
        .windows(needle.len())
        .rposition(|w| w == needle)
        .map(|p| p + from)
}

/// Escape text for HTML element content.
pub(crate) fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    push_escaped(&mut out, s);
    out
}

/// Append `s` HTML-escaped (safe for both text and quoted attribute values).
pub(crate) fn push_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

/// Characters percent-encoded in a URL path segment of an `href`.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'|')
    .add(b'\\')
    .add(b'^')
    .add(b'[')
    .add(b']')
    .add(b'\'')
    .add(b'/');

/// Percent-encode a vault path for use in `href`/`src` (each `/`-separated
/// segment is encoded, non-ASCII characters become UTF-8 percent escapes).
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| utf8_percent_encode(seg, PATH_SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Encode a path for a markdown link destination (`My Note.md` → `My%20Note.md`).
///
/// Idempotent: existing `%XX` escapes are kept (an already-encoded target is
/// returned unchanged, never `%2520`). Encoded: ASCII controls, space, `(`,
/// `)`, `<`, `>`, `#` and a `%` not starting a valid escape. Non-ASCII
/// characters are kept as-is (like Obsidian does).
pub fn encode_markdown_target(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    for (i, c) in path.char_indices() {
        match c {
            '%' => {
                let valid = b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                    && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
                out.push_str(if valid { "%" } else { "%25" });
            }
            ' ' | '(' | ')' | '<' | '>' | '#' => {
                out.push_str(&format!("%{:02X}", c as u32));
            }
            c if c.is_ascii_control() => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Percent-decode (lossy UTF-8).
pub(crate) fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// Remove CommonMark backslash escapes before ASCII punctuation.
pub(crate) fn unescape_backslashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&n) = chars.peek()
            && n.is_ascii_punctuation()
        {
            out.push(n);
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// True when a link destination is external (has a URL scheme such as
/// `https:`, `mailto:`, `obsidian:` or is protocol-relative `//host`).
pub fn is_external(dest: &str) -> bool {
    if dest.starts_with("//") {
        return true;
    }
    let mut chars = dest.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for (_, c) in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
            return false;
        }
    }
    false
}

/// Heading anchor slug used for `id` attributes and `#subpath` fragments.
///
/// Algorithm: lowercase; keep Unicode letters/digits, `_` and `-`; turn runs
/// of whitespace (and other separators) into a single `-`; drop all other
/// punctuation; trim leading/trailing `-`. Example: `"Hello, World!"` →
/// `"hello-world"`, `"Idées & projets"` → `"idées-projets"`.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_dash = false;
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            for l in c.to_lowercase() {
                out.push(l);
            }
        } else if c.is_whitespace() {
            pending_dash = true;
        }
    }
    out
}

/// Image extensions rendered as `<img>` when embedded.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif"];

/// Lowercased extension of the last path component, if any.
pub(crate) fn extension_of(target: &str) -> Option<String> {
    let name = target.rsplit('/').next().unwrap_or(target);
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => Some(name[i + 1..].to_ascii_lowercase()),
        _ => None,
    }
}

/// True if `target` names an image file (by extension).
pub fn is_image_target(target: &str) -> bool {
    extension_of(target).is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

pub(crate) fn ends_with_md(s: &str) -> bool {
    s.len() >= 3 && s.is_char_boundary(s.len() - 3) && s[s.len() - 3..].eq_ignore_ascii_case(".md")
}

/// Count words: maximal runs of letters/digits (`_`, `'`, `’` and `-`
/// allowed between two such characters); each CJK ideograph / kana / hangul
/// syllable counts as one word.
pub(crate) fn count_words(text: &str, mut skip: impl FnMut(usize) -> Option<usize>) -> usize {
    let mut count = 0;
    let mut in_word = false;
    let mut pending_joiner = false;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if let Some(end) = skip(i) {
            // Skip a hidden range: acts as a separator.
            in_word = false;
            pending_joiner = false;
            while let Some(&(j, _)) = iter.peek() {
                if j >= end {
                    break;
                }
                iter.next();
            }
            continue;
        }
        if is_cjk(c) {
            count += 1;
            in_word = false;
            pending_joiner = false;
        } else if c.is_alphanumeric() || c == '_' {
            if !in_word {
                count += 1;
                in_word = true;
            }
            pending_joiner = false;
        } else if in_word && !pending_joiner && matches!(c, '\'' | '’' | '-') {
            pending_joiner = true;
            // Stay "in word" only if the next char continues it.
            match iter.peek() {
                Some(&(_, n)) if n.is_alphanumeric() && !is_cjk(n) => {}
                _ => {
                    in_word = false;
                    pending_joiner = false;
                }
            }
        } else {
            in_word = false;
            pending_joiner = false;
        }
    }
    count
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility
        | 0x20000..=0x2FA1F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_set() {
        let s = RangeSet::new(vec![(5, 10), (0, 2), (9, 12)]);
        assert!(s.contains(0) && s.contains(1) && !s.contains(2));
        assert!(s.contains(5) && s.contains(11) && !s.contains(12));
        assert!(s.overlaps(3, 6) && !s.overlaps(2, 5) && !s.overlaps(12, 20));
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  Idées & projets "), "idées-projets");
        assert_eq!(slugify("a_b-c"), "a_b-c");
        assert_eq!(slugify("Étape 1.2"), "étape-12");
    }

    #[test]
    fn external() {
        assert!(is_external("https://x.y"));
        assert!(is_external("mailto:a@b.c"));
        assert!(is_external("obsidian://open?x"));
        assert!(is_external("//cdn"));
        assert!(!is_external("Note.md"));
        assert!(!is_external("folder/a:b.md"));
        assert!(!is_external("#heading"));
        assert!(!is_external(""));
    }

    #[test]
    fn encoding() {
        assert_eq!(encode_path("a b/c#d.md"), "a%20b/c%23d.md");
        assert_eq!(encode_path("Idées.md"), "Id%C3%A9es.md");
        assert_eq!(
            encode_markdown_target("My Note (1).md"),
            "My%20Note%20%281%29.md"
        );
        assert_eq!(encode_markdown_target("Idées.md"), "Idées.md");
        assert_eq!(encode_markdown_target("a%20b.md"), "a%20b.md");
        assert_eq!(encode_markdown_target("100% sure"), "100%25%20sure");
        assert_eq!(encode_markdown_target("x%2"), "x%252");
        let once = encode_markdown_target("A b (c) <d>.md");
        assert_eq!(encode_markdown_target(&once), once);
    }

    #[test]
    fn words() {
        assert_eq!(count_words("Hello world", |_| None), 2);
        assert_eq!(count_words("d'exemple oxi-core 2026-10-20", |_| None), 3);
        assert_eq!(count_words("- [ ] **bold** -- x", |_| None), 2);
        assert_eq!(count_words("日本語", |_| None), 3);
        assert_eq!(count_words("", |_| None), 0);
        assert_eq!(count_words("end-", |_| None), 1);
    }

    #[test]
    fn lines() {
        let l = LineIndex::new("a\nbc\n\nd");
        assert_eq!(l.line_of(0), 0);
        assert_eq!(l.line_of(1), 0);
        assert_eq!(l.line_of(2), 1);
        assert_eq!(l.line_of(5), 2);
        assert_eq!(l.line_of(6), 3);
        assert_eq!(l.line_of(100), 3);
    }
}
