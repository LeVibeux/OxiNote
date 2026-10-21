//! Obsidian link resolution rules.

use oxi_core::{LinkKind, NotePath};

use crate::MetadataCache;
use crate::text::{folder_of, has_md, join_normalize, key, last_component, name_key};

/// Tie-break rank of a basename candidate (lower is better).
type Rank = (u8, u8, usize, usize);

impl MetadataCache {
    /// Resolve a link target as written in note `from`.
    ///
    /// Rules (all comparisons NFC + case-insensitive):
    /// 1. an empty target (`[[#Heading]]`) is the note itself;
    /// 2. `./x` and `../x` are relative to the note's folder; a leading `/`
    ///    means a vault-absolute path;
    /// 3. otherwise the exact vault path is tried (with `.md` appended when the
    ///    target has no `.md`, then as written), then the path relative to the
    ///    note's folder — markdown links try the relative path first;
    /// 4. finally files whose name matches the last component and whose path
    ///    ends with the target are candidates; ties prefer a markdown note, then
    ///    the note's own folder, then the shortest path, then alphabetical order.
    pub fn resolve_target(
        &self,
        target: &str,
        kind: LinkKind,
        from: &NotePath,
    ) -> Option<NotePath> {
        let t = target.trim().replace('\\', "/");
        if t.is_empty() {
            return Some(from.clone());
        }
        let folder = from.parent().unwrap_or("");
        if t.starts_with("./") || t.starts_with("../") {
            if let Some(found) = join_normalize(folder, &t).and_then(|p| self.lookup_exact(&p)) {
                return Some(found);
            }
            let stripped = t.trim_start_matches(['.', '/']);
            return self.lookup_by_name(stripped, folder);
        }
        if let Some(abs) = t.strip_prefix('/') {
            let abs = abs.trim_start_matches('/');
            if abs.is_empty() {
                return None;
            }
            return self
                .lookup_exact(abs)
                .or_else(|| self.lookup_by_name(abs, folder));
        }
        let relative = || {
            if folder.is_empty() {
                None
            } else {
                join_normalize(folder, &t).and_then(|p| self.lookup_exact(&p))
            }
        };
        let found = if kind == LinkKind::Markdown {
            relative().or_else(|| self.lookup_exact(&t))
        } else {
            self.lookup_exact(&t).or_else(relative)
        };
        found.or_else(|| self.lookup_by_name(&t, folder))
    }

    /// Exact vault path lookup: `p.md` first (unless `p` already ends with
    /// `.md`), then `p` as written.
    pub(crate) fn lookup_exact(&self, p: &str) -> Option<NotePath> {
        let k = key(p);
        if !has_md(&k)
            && let Some(found) = self.lookup_key(&format!("{k}.md"), p)
        {
            return Some(found);
        }
        self.lookup_key(&k, p)
    }

    fn lookup_key(&self, k: &str, written: &str) -> Option<NotePath> {
        let candidates = self.path_index.get(k)?;
        // Several files only differ by case: prefer the exact spelling.
        candidates
            .iter()
            .find(|c| c.without_extension() == written || c.as_str() == written)
            .or_else(|| candidates.first())
            .cloned()
    }

    /// Basename / trailing-path lookup.
    fn lookup_by_name(&self, t: &str, folder: &str) -> Option<NotePath> {
        let candidates = self.name_index.get(&name_key(t))?;
        let tk = key(t);
        let tk_md = if has_md(&tk) {
            None
        } else {
            Some(format!("{tk}.md"))
        };
        let matches = |pk: &str, want: &str| {
            pk == want
                || (pk.len() > want.len()
                    && pk.ends_with(want)
                    && pk.as_bytes()[pk.len() - want.len() - 1] == b'/')
        };
        let mut best: Option<(Rank, &NotePath)> = None;
        for c in candidates {
            let pk = key(c.as_str());
            let md_match = tk_md.as_deref().is_some_and(|w| matches(&pk, w));
            let plain_match = matches(&pk, &tk);
            if !md_match && !plain_match {
                continue;
            }
            // A target ending in `.md` only matches markdown files, which
            // `plain_match` already covers.
            let is_md_rank = u8::from(!(md_match || (has_md(&tk) && plain_match)));
            let same_folder = u8::from(folder_of(c.as_str()) != folder);
            let depth = c.as_str().matches('/').count();
            let rank = (is_md_rank, same_folder, depth, c.as_str().len());
            let better = match &best {
                None => true,
                Some((r, p)) => rank < *r || (rank == *r && c < *p),
            };
            if better {
                best = Some((rank, c));
            }
        }
        best.map(|(_, p)| p.clone())
    }

    /// Whether more than one file shares the file name of `path`
    /// (case-insensitive), i.e. a basename link to it may be ambiguous.
    pub fn is_name_ambiguous(&self, path: &NotePath) -> bool {
        let fk = key(path.file_name());
        self.name_index
            .get(&name_key(path.as_str()))
            .is_some_and(|s| s.iter().filter(|p| key(p.file_name()) == fk).count() > 1)
    }

    /// Files whose name matches `name` (a stem like `"Note"` or a file name
    /// like `"image.png"`), case-insensitively.
    pub fn files_named(&self, name: &str) -> Vec<NotePath> {
        let k = key(last_component(name));
        let k_md = format!("{k}.md");
        self.name_index
            .get(&name_key(name))
            .map(|s| {
                s.iter()
                    .filter(|p| {
                        let fk = key(p.file_name());
                        fk == k || fk == k_md
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}
