use std::collections::BTreeMap;
use std::sync::RwLock;
use std::time::SystemTime;

use crate::{Error, FileMeta, NotePath, Result, VaultRead, VaultWrite};

/// In-memory vault for tests and examples.
#[derive(Debug, Default)]
pub struct MemoryVault {
    files: RwLock<BTreeMap<NotePath, (Vec<u8>, SystemTime)>>,
}

impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from `(path, contents)` pairs. Panics on invalid paths (test helper).
    pub fn from_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let v = Self::new();
        for (p, c) in files {
            #[allow(clippy::unwrap_used)]
            v.write(&NotePath::new(p).unwrap(), c).unwrap();
        }
        v
    }

    fn lock_err() -> Error {
        Error::Other("memory vault lock poisoned".into())
    }
}

impl VaultRead for MemoryVault {
    fn list_files(&self) -> Result<Vec<NotePath>> {
        let files = self.files.read().map_err(|_| Self::lock_err())?;
        Ok(files.keys().cloned().collect())
    }

    fn read_to_string(&self, path: &NotePath) -> Result<String> {
        let bytes = self.read_bytes(path)?;
        String::from_utf8(bytes).map_err(|e| Error::Parse(e.to_string()))
    }

    fn read_bytes(&self, path: &NotePath) -> Result<Vec<u8>> {
        let files = self.files.read().map_err(|_| Self::lock_err())?;
        files
            .get(path)
            .map(|(b, _)| b.clone())
            .ok_or_else(|| Error::NotFound(path.clone()))
    }

    fn metadata(&self, path: &NotePath) -> Result<FileMeta> {
        let files = self.files.read().map_err(|_| Self::lock_err())?;
        let (b, t) = files
            .get(path)
            .ok_or_else(|| Error::NotFound(path.clone()))?;
        Ok(FileMeta {
            size: b.len() as u64,
            modified: *t,
            created: *t,
        })
    }

    fn exists(&self, path: &NotePath) -> bool {
        self.files.read().is_ok_and(|f| f.contains_key(path))
    }
}

impl VaultWrite for MemoryVault {
    fn write(&self, path: &NotePath, contents: &str) -> Result<()> {
        let mut files = self.files.write().map_err(|_| Self::lock_err())?;
        files.insert(
            path.clone(),
            (contents.as_bytes().to_vec(), SystemTime::now()),
        );
        Ok(())
    }

    fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        let mut files = self.files.write().map_err(|_| Self::lock_err())?;
        if files.contains_key(to) {
            return Err(Error::AlreadyExists(to.clone()));
        }
        let entry = files
            .remove(from)
            .ok_or_else(|| Error::NotFound(from.clone()))?;
        files.insert(to.clone(), entry);
        Ok(())
    }

    fn delete(&self, path: &NotePath) -> Result<()> {
        let mut files = self.files.write().map_err(|_| Self::lock_err())?;
        files
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| Error::NotFound(path.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = MemoryVault::from_files([("a.md", "hello"), ("img/x.png", "")]);
        let a = NotePath::new("a.md").unwrap();
        assert_eq!(v.read_to_string(&a).unwrap(), "hello");
        assert_eq!(v.list_notes().unwrap(), vec![a.clone()]);
        let b = NotePath::new("b.md").unwrap();
        v.rename(&a, &b).unwrap();
        assert!(!v.exists(&a) && v.exists(&b));
        v.delete(&b).unwrap();
        assert_eq!(v.list_files().unwrap().len(), 1);
    }
}
