//! Small helpers shared by the state and the UI.

/// Subsequence fuzzy score used to filter *commands* in the palette (files
/// go through `ors_index::fuzzy_find`). `None` if `query` is not a
/// subsequence of `candidate` (case-insensitive). Consecutive matches and
/// matches at word starts score higher; an empty query matches everything.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let q: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut score = 0i64;
    let mut prev_match: Option<usize> = None;
    for (i, c) in cand.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            score += 1;
            if prev_match.is_some_and(|p| p + 1 == i) {
                score += 5;
            }
            if i == 0 || matches!(cand[i - 1], ' ' | '/' | ':' | '-' | '_') {
                score += 3;
            }
            prev_match = Some(i);
            qi += 1;
        }
    }
    if qi == q.len() {
        Some(score * 100 - cand.len() as i64)
    } else {
        None
    }
}

/// Today's local date.
pub fn today() -> ors_vault::Date {
    use chrono::Datelike;
    let now = chrono::Local::now().date_naive();
    ors_vault::Date {
        year: now.year(),
        month: now.month(),
        day: now.day(),
    }
}

/// Byte offset of the start of the 0-based `line` in `text` (clamped).
pub fn line_start_byte(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 1)
        .map_or(text.len(), |(i, _)| i + 1)
}

/// Is `name` usable as a file or folder name?
pub fn valid_file_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n != "."
        && n != ".."
        && !n.contains([
            '/', '\\', ':', '*', '?', '"', '<', '>', '|', '#', '^', '[', ']',
        ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scores() {
        assert!(fuzzy_score("onn", "Open new note").is_some());
        assert!(fuzzy_score("xyz", "Open new note").is_none());
        let a = fuzzy_score("tog", "Toggle reading view").unwrap();
        let b = fuzzy_score("tog", "Open today's daily note: go").unwrap();
        assert!(a > b);
        assert_eq!(fuzzy_score("", "x"), Some(0));
    }

    #[test]
    fn line_offsets() {
        let t = "a\nbc\nd";
        assert_eq!(line_start_byte(t, 0), 0);
        assert_eq!(line_start_byte(t, 1), 2);
        assert_eq!(line_start_byte(t, 2), 5);
        assert_eq!(line_start_byte(t, 9), 6);
    }

    #[test]
    fn file_names() {
        assert!(valid_file_name("My note"));
        assert!(!valid_file_name("a/b"));
        assert!(!valid_file_name("  "));
        assert!(!valid_file_name("x[1]"));
    }
}
