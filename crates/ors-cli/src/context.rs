//! Opening a vault and resolving note arguments.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use ors_core::{LinkResolver, NotePath};
use ors_index::MetadataCache;
use ors_vault::Vault;

/// An opened vault with its metadata cache.
pub struct Context {
    pub vault: Vault,
    pub cache: MetadataCache,
}

/// Check that `path` is an existing folder and return it.
pub fn check_vault_dir(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        bail!("vault folder not found: {}", path.display());
    }
    if !path.is_dir() {
        bail!("not a folder: {}", path.display());
    }
    Ok(path.to_path_buf())
}

impl Context {
    /// Open the vault at `path` and index it.
    pub fn load(path: &Path) -> Result<Self> {
        let dir = check_vault_dir(path)?;
        let vault = Vault::open(&dir)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("cannot open vault {}", path.display()))?;
        let cache = MetadataCache::build(&vault)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("cannot index vault {}", path.display()))?;
        Ok(Self { vault, cache })
    }

    /// Resolve a note given on the command line (path or name).
    pub fn resolve_note(&self, arg: &str) -> Result<NotePath> {
        let files = self.cache.files();
        resolve_note_arg(arg, &files, &self.cache).map_err(|e| {
            let suggestions: Vec<String> = ors_index::fuzzy_find(&self.cache, arg, 3)
                .into_iter()
                .map(|(p, _, _)| p.to_string())
                .collect();
            if suggestions.is_empty() {
                e
            } else {
                anyhow!("{e} (did you mean: {}?)", suggestions.join(", "))
            }
        })
    }
}

/// Resolve a note argument against the vault files:
///
/// 1. exact vault path (`folder/Note.md`), or path without `.md`;
/// 2. wikilink resolution (`Note`, `folder/Note`) via `resolver`;
/// 3. case-insensitive path or name match, if unique.
pub fn resolve_note_arg(
    arg: &str,
    files: &[NotePath],
    resolver: &dyn LinkResolver,
) -> Result<NotePath> {
    let arg = arg.trim();
    let wanted = NotePath::new(arg).map_err(|_| anyhow!("invalid note name: {arg:?}"))?;
    if files.contains(&wanted) {
        return Ok(wanted);
    }
    if let Ok(with_md) = NotePath::new(format!("{wanted}.md"))
        && files.contains(&with_md)
    {
        return Ok(with_md);
    }
    // Resolve like a link written in a note at the vault root.
    let target = wanted
        .as_str()
        .strip_suffix(".md")
        .unwrap_or(wanted.as_str());
    let root_note = NotePath::new("_.md").map_err(|e| anyhow!("{e}"))?;
    if let Some(p) = resolver.resolve(target, &root_note) {
        return Ok(p);
    }
    let lower = wanted.as_str().to_lowercase();
    let lower_md = format!("{lower}.md");
    let candidates: Vec<&NotePath> = files
        .iter()
        .filter(|f| {
            let p = f.as_str().to_lowercase();
            p == lower || p == lower_md || (f.is_markdown() && f.stem().to_lowercase() == lower)
        })
        .collect();
    match candidates.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("note not found: {arg}"),
        many => bail!(
            "ambiguous note name {arg:?}: {}",
            many.iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ors_core::NoResolver;

    fn files() -> Vec<NotePath> {
        [
            "Accueil.md",
            "Projets/Rust notes.md",
            "a/X.md",
            "b/x.md",
            "img.png",
        ]
        .iter()
        .map(|p| NotePath::new(p).unwrap())
        .collect()
    }

    struct StemResolver(Vec<NotePath>);
    impl LinkResolver for StemResolver {
        fn resolve(&self, target: &str, _from: &NotePath) -> Option<NotePath> {
            self.0.iter().find(|p| p.stem() == target).cloned()
        }
    }

    #[test]
    fn resolves_paths_and_names() {
        let f = files();
        let r = |a: &str| resolve_note_arg(a, &f, &NoResolver).map(|p| p.to_string());
        assert_eq!(r("Accueil.md").unwrap(), "Accueil.md");
        assert_eq!(r("Projets/Rust notes").unwrap(), "Projets/Rust notes.md");
        assert_eq!(r("rust NOTES").unwrap(), "Projets/Rust notes.md");
        assert_eq!(r("img.png").unwrap(), "img.png");
        let err = r("x").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        let err = r("Nope").unwrap_err().to_string();
        assert_eq!(err, "note not found: Nope");
        assert!(r("../x").is_err());
    }

    #[test]
    fn uses_link_resolver() {
        let f = files();
        let resolver = StemResolver(f.clone());
        let p = resolve_note_arg("Rust notes", &f, &resolver).unwrap();
        assert_eq!(p.as_str(), "Projets/Rust notes.md");
    }

    #[test]
    fn missing_vault_dir() {
        let err = check_vault_dir(Path::new("/definitely/not/here")).unwrap_err();
        assert!(err.to_string().contains("vault folder not found"));
    }
}
