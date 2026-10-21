//! Fuzzy file matching for the quick switcher.

use oxi_core::NotePath;

use crate::MetadataCache;

/// Options for [`fuzzy_find_with`].
#[derive(Debug, Clone, Copy, Default)]
pub struct FuzzyOptions {
    /// Also match attachments (non-markdown files) by file name and path.
    pub include_attachments: bool,
}

const BONUS_STEM: i64 = 60;
const BONUS_ALIAS: i64 = 40;

/// Subsequence fuzzy score of `query` in `candidate` (both compared
/// case-insensitively; whitespace in the query is ignored). `None` if the
/// query is not a subsequence. Higher is better.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let c: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    let orig: Vec<char> = candidate.chars().collect();
    let boundary = |i: usize| -> bool {
        if i == 0 {
            return true;
        }
        let prev = c.get(i - 1).copied().unwrap_or(' ');
        if !prev.is_alphanumeric() {
            return true;
        }
        // camelCase boundary (only meaningful when lowercasing kept the length).
        orig.len() == c.len()
            && orig.get(i).is_some_and(|ch| ch.is_uppercase())
            && orig.get(i - 1).is_some_and(|ch| ch.is_lowercase())
    };
    // Greedy forward match followed by a backward tightening pass.
    let mut positions = Vec::with_capacity(q.len());
    let mut j = 0;
    for &qc in &q {
        while j < c.len() && c[j] != qc {
            j += 1;
        }
        if j == c.len() {
            return None;
        }
        positions.push(j);
        j += 1;
    }
    // Tighten: walk backwards from the last match to pull earlier matches as
    // late as possible (shorter span, more consecutive runs).
    let mut end = positions.last().copied().unwrap_or(0);
    for k in (0..q.len()).rev() {
        let mut p = end;
        while c.get(p) != Some(&q[k]) {
            if p == 0 {
                break;
            }
            p -= 1;
        }
        if c.get(p) == Some(&q[k]) && p >= positions[k] {
            positions[k] = p;
        }
        end = positions[k].saturating_sub(1);
    }
    let mut score: i64 = 0;
    let mut prev: Option<usize> = None;
    for (k, &p) in positions.iter().enumerate() {
        score += 16;
        if boundary(p) {
            score += 24;
        }
        if k == 0 && p == 0 {
            score += 20;
        }
        match prev {
            Some(pp) if pp + 1 == p => score += 18,
            Some(pp) => score -= i64::try_from((p - pp - 1).min(20)).unwrap_or(20),
            None => score -= i64::try_from(p.min(15)).unwrap_or(15),
        }
        prev = Some(p);
    }
    // Prefer shorter candidates and exact / prefix matches.
    score -= i64::try_from(c.len().saturating_sub(q.len()).min(40)).unwrap_or(40) / 2;
    let full: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    if c == full {
        score += 100;
    } else if c.starts_with(&full) {
        score += 40;
    }
    Some(score)
}

/// Fuzzy file matching for the quick switcher: matches against note stems
/// (bonus), vault paths and aliases. Returns `(path, matched alias if any,
/// score)` best first. An empty query returns the first `limit` notes in
/// alphabetical order with score 0.
pub fn fuzzy_find(
    cache: &MetadataCache,
    query: &str,
    limit: usize,
) -> Vec<(NotePath, Option<String>, i64)> {
    fuzzy_find_with(cache, query, limit, FuzzyOptions::default())
}

/// [`fuzzy_find`] with options.
pub fn fuzzy_find_with(
    cache: &MetadataCache,
    query: &str,
    limit: usize,
    options: FuzzyOptions,
) -> Vec<(NotePath, Option<String>, i64)> {
    let files = cache
        .files
        .iter()
        .filter(|(_, d)| d.is_some() || options.include_attachments);
    if query.trim().is_empty() {
        return files
            .take(limit)
            .map(|(p, _)| (p.clone(), None, 0))
            .collect();
    }
    let mut out: Vec<(NotePath, Option<String>, i64)> = Vec::new();
    for (path, data) in files {
        let name = if data.is_some() {
            path.stem()
        } else {
            path.file_name()
        };
        let mut best: Option<(i64, Option<String>)> = None;
        let mut consider = |score: Option<i64>, alias: Option<&str>| {
            if let Some(s) = score
                && best.as_ref().is_none_or(|(b, _)| s > *b)
            {
                best = Some((s, alias.map(str::to_string)));
            }
        };
        consider(fuzzy_score(query, name).map(|s| s + BONUS_STEM), None);
        consider(fuzzy_score(query, path.without_extension()), None);
        if let Some(d) = data {
            for alias in &d.parsed.aliases {
                consider(
                    fuzzy_score(query, alias).map(|s| s + BONUS_ALIAS),
                    Some(alias),
                );
            }
        }
        if let Some((score, alias)) = best {
            out.push((path.clone(), alias, score));
        }
    }
    out.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.as_str().len().cmp(&b.0.as_str().len()))
            .then_with(|| a.0.cmp(&b.0))
    });
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_score("abc", "a-b-c").is_some());
        assert!(fuzzy_score("acb", "abc").is_none());
        assert_eq!(fuzzy_score("", "x"), Some(0));
    }

    #[test]
    fn ranking_prefers_exact_prefix_and_consecutive() {
        let exact = fuzzy_score("rust", "rust").unwrap();
        let prefix = fuzzy_score("rust", "rust notes").unwrap();
        let scattered = fuzzy_score("rust", "r u s t").unwrap();
        let inner = fuzzy_score("rust", "trusty").unwrap();
        assert!(exact > prefix, "{exact} {prefix}");
        assert!(prefix > inner, "{prefix} {inner}");
        assert!(prefix > scattered, "{prefix} {scattered}");
    }

    #[test]
    fn case_and_spaces_ignored() {
        assert!(fuzzy_score("Rust Notes", "rustnotes").is_some());
        assert!(fuzzy_score("RN", "rust notes").is_some());
        let boundary = fuzzy_score("rn", "rust notes").unwrap();
        let mid = fuzzy_score("rn", "ironman").unwrap();
        assert!(boundary > mid);
    }
}
