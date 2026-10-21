use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A vault-relative path, always using `/` separators, never starting or ending
/// with `/`, without `.` or `..` components. Example: `"folder/My Note.md"`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NotePath(String);

impl NotePath {
    /// Normalizes `\` to `/`, strips leading/trailing slashes and `./` components,
    /// and rejects empty paths and `..` components.
    pub fn new(path: impl AsRef<str>) -> Result<Self> {
        let raw = path.as_ref().replace('\\', "/");
        let mut parts = Vec::new();
        for part in raw.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(Error::InvalidPath(raw)),
                p => parts.push(p),
            }
        }
        if parts.is_empty() {
            return Err(Error::InvalidPath(raw));
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Last component, e.g. `"My Note.md"`.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// File name without extension, e.g. `"My Note"`. This is what Obsidian
    /// displays as the note title and what `[[My Note]]` matches.
    pub fn stem(&self) -> &str {
        let name = self.file_name();
        match name.rfind('.') {
            Some(i) if i > 0 => &name[..i],
            _ => name,
        }
    }

    /// Lowercased extension without the dot, e.g. `"md"`; empty if none.
    pub fn extension(&self) -> String {
        let name = self.file_name();
        match name.rfind('.') {
            Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
            _ => String::new(),
        }
    }

    /// Parent folder (vault-relative), `None` for files at the vault root.
    pub fn parent(&self) -> Option<&str> {
        self.0.rfind('/').map(|i| &self.0[..i])
    }

    /// Path without the extension, e.g. `"folder/My Note"`.
    pub fn without_extension(&self) -> &str {
        let ext_len = self.extension().len();
        if ext_len == 0 {
            &self.0
        } else {
            &self.0[..self.0.len() - ext_len - 1]
        }
    }

    pub fn is_markdown(&self) -> bool {
        self.extension() == "md"
    }

    /// Join a child name onto a folder path (`folder` may be empty for the root).
    pub fn join(folder: &str, name: &str) -> Result<Self> {
        if folder.is_empty() {
            Self::new(name)
        } else {
            Self::new(format!("{folder}/{name}"))
        }
    }

    /// Same folder, different file name.
    pub fn with_file_name(&self, name: &str) -> Result<Self> {
        Self::join(self.parent().unwrap_or(""), name)
    }
}

impl fmt::Display for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for NotePath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NotePath {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<&str> for NotePath {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl From<NotePath> for String {
    fn from(value: NotePath) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        let p = NotePath::new("/a\\b/./My Note.md/").unwrap();
        assert_eq!(p.as_str(), "a/b/My Note.md");
        assert_eq!(p.file_name(), "My Note.md");
        assert_eq!(p.stem(), "My Note");
        assert_eq!(p.extension(), "md");
        assert_eq!(p.parent(), Some("a/b"));
        assert_eq!(p.without_extension(), "a/b/My Note");
        assert!(p.is_markdown());
    }

    #[test]
    fn rejects_invalid() {
        assert!(NotePath::new("").is_err());
        assert!(NotePath::new("/").is_err());
        assert!(NotePath::new("a/../b").is_err());
    }

    #[test]
    fn dotfiles_and_no_ext() {
        let p = NotePath::new(".hidden").unwrap();
        assert_eq!(p.stem(), ".hidden");
        assert_eq!(p.extension(), "");
        let p = NotePath::new("README").unwrap();
        assert_eq!(p.parent(), None);
        assert_eq!(p.without_extension(), "README");
    }

    #[test]
    fn join_and_rename() {
        let p = NotePath::join("", "a.md").unwrap();
        assert_eq!(p.as_str(), "a.md");
        let p = NotePath::join("x/y", "a.md").unwrap();
        assert_eq!(p.with_file_name("b.md").unwrap().as_str(), "x/y/b.md");
    }
}
