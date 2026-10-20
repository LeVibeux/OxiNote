//! File-name rules shared with Obsidian.

/// Characters Obsidian refuses in file and folder names: the ones forbidden by
/// common filesystems (`* " \ / < > : | ?`) plus the ones that break link
/// syntax (`# ^ [ ] |`).
pub const FORBIDDEN_NAME_CHARS: &[char] = &[
    '*', '"', '\\', '/', '<', '>', ':', '|', '?', '#', '^', '[', ']',
];

/// `true` if `name` is a valid file/folder name for Obsidian: non-empty, no
/// forbidden or control characters, not starting with `.` and not ending with
/// a space or `.` (Windows).
pub fn is_valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| FORBIDDEN_NAME_CHARS.contains(&c) || c.is_control())
}

/// Make `name` a valid file name: forbidden and control characters are
/// removed, leading dots and surrounding whitespace / trailing dots are
/// trimmed, runs of whitespace are collapsed. Returns `"Untitled"` when
/// nothing is left.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !FORBIDDEN_NAME_CHARS.contains(c))
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_file_name("a/b:c?"), "abc");
        assert_eq!(sanitize_file_name("  [[Note#1]] ^x  "), "Note1 x");
        assert_eq!(sanitize_file_name(""), "Untitled");
        assert_eq!(sanitize_file_name("..."), "Untitled");
        assert_eq!(sanitize_file_name(".hidden"), "hidden");
        assert_eq!(sanitize_file_name("Idées\tnouvelles."), "Idées nouvelles");
    }

    #[test]
    fn validity() {
        assert!(is_valid_file_name("My note"));
        assert!(is_valid_file_name("Idées"));
        assert!(!is_valid_file_name(""));
        assert!(!is_valid_file_name("a|b"));
        assert!(!is_valid_file_name("a#b"));
        assert!(!is_valid_file_name(".x"));
        assert!(!is_valid_file_name("x."));
    }
}
