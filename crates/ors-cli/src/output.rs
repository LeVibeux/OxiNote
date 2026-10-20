//! Output helpers: JSON and terminal highlighting.

use std::io::Write;

use anyhow::Result;
use serde::Serialize;

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Pretty-print `value` as JSON followed by a newline.
pub fn write_json<T: Serialize + ?Sized>(out: &mut dyn Write, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    writeln!(out, "{text}")?;
    Ok(())
}

/// Wrap the byte `ranges` of `text` in ANSI bold (or leave the text as is
/// when `color` is false). Ranges may be unsorted, overlapping or out of
/// bounds; ranges not on char boundaries are skipped.
pub fn highlight(text: &str, ranges: &[(usize, usize)], color: bool) -> String {
    if !color || ranges.is_empty() {
        return text.to_string();
    }
    let mut sorted: Vec<(usize, usize)> = ranges
        .iter()
        .map(|&(s, e)| (s.min(text.len()), e.min(text.len())))
        .filter(|&(s, e)| s < e && text.is_char_boundary(s) && text.is_char_boundary(e))
        .collect();
    sorted.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in sorted {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    let mut out = String::with_capacity(text.len() + merged.len() * 8);
    let mut pos = 0;
    for (s, e) in merged {
        out.push_str(&text[pos..s]);
        out.push_str(BOLD);
        out.push_str(&text[s..e]);
        out.push_str(RESET);
        pos = e;
    }
    out.push_str(&text[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights() {
        assert_eq!(highlight("hello world", &[(0, 5)], false), "hello world");
        assert_eq!(
            highlight("hello world", &[(6, 11), (0, 2), (1, 3)], true),
            "\x1b[1mhel\x1b[0mlo \x1b[1mworld\x1b[0m"
        );
        // Out of bounds and non-boundary ranges are ignored.
        assert_eq!(highlight("é", &[(0, 1), (5, 9)], true), "é");
    }

    #[test]
    fn json() {
        let mut buf = Vec::new();
        write_json(&mut buf, &vec![1, 2]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "[\n  1,\n  2\n]\n");
    }
}
