//! Find / replace in the current note.

use std::ops::Range;

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Non-overlapping matches of `query` in `text` (byte ranges), ignoring case
/// unless `case_sensitive`.
pub fn find_all(text: &str, query: &str, case_sensitive: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return text
            .match_indices(query)
            .map(|(i, m)| i..i + m.len())
            .collect();
    }
    let q: Vec<char> = query.chars().map(fold).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let mut chars = text[i..].char_indices();
        let mut end = None;
        let mut ok = true;
        for qc in &q {
            match chars.next() {
                Some((_, c)) if fold(c) == *qc => {}
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            end = Some(chars.next().map_or(text.len(), |(k, _)| i + k));
        }
        match end {
            Some(e) => {
                out.push(i..e);
                i = e;
            }
            None => i += text[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// Index of the match to select when searching forward/backward from byte
/// `from`.
pub fn next_match(matches: &[Range<usize>], from: usize, forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if forward {
        Some(
            matches
                .iter()
                .position(|m| m.start >= from)
                .unwrap_or(0),
        )
    } else {
        Some(
            matches
                .iter()
                .rposition(|m| m.end < from || (m.start < from && m.end <= from))
                .unwrap_or(matches.len() - 1),
        )
    }
}

/// Replace every match; returns the number of replacements.
pub fn replace_all(text: &mut String, query: &str, with: &str, case_sensitive: bool) -> usize {
    let matches = find_all(text, query, case_sensitive);
    for m in matches.iter().rev() {
        text.replace_range(m.clone(), with);
    }
    matches.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_insensitively() {
        let t = "Été, été et ÉTÉ";
        let m = find_all(t, "été", false);
        assert_eq!(m.len(), 3);
        assert_eq!(&t[m[2].clone()], "ÉTÉ");
        assert_eq!(find_all(t, "été", true).len(), 1);
        assert!(find_all(t, "", false).is_empty());
        assert_eq!(find_all("aaaa", "aa", false), vec![0..2, 2..4]);
    }

    #[test]
    fn navigation_and_replace() {
        let m = vec![0..2, 5..7, 10..12];
        assert_eq!(next_match(&m, 3, true), Some(1));
        assert_eq!(next_match(&m, 11, true), Some(0)); // wraps
        assert_eq!(next_match(&m, 5, false), Some(0));
        assert_eq!(next_match(&m, 0, false), Some(2)); // wraps
        assert_eq!(next_match(&[], 0, true), None);
        let mut t = "Rust rust RUST".to_string();
        assert_eq!(replace_all(&mut t, "rust", "Go", false), 3);
        assert_eq!(t, "Go Go Go");
    }
}
