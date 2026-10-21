//! Thin abstraction layer between the application state and the upstream
//! crates (`ors-vault`, `ors-index`, `ors-graph`).
//!
//! The app talks to a [`VaultBackend`] and an [`IndexBackend`] so that
//! [`crate::state::AppState`] can be unit-tested with an in-memory vault and
//! so that the `--demo` mode can run on hand-made data. The real
//! implementations ([`DiskVault`], [`RealIndex`], [`ForceEngine`]) are pure
//! delegation to the upstream public APIs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::mpsc::Receiver;

use ors_core::{
    Error, FileMeta, GraphData, LinkResolver, MemoryVault, NotePath, ParsedNote, Result,
    VaultEvent, VaultRead, VaultWrite,
};
use ors_graph::{ForceLayout, ForceParams, Vec2};
use ors_index::{Backlink, GraphOptions, MetadataCache, ResolvedLink, SearchIndex, SearchResult};
use ors_vault::{AppConfig, Date, Vault, VaultWatcher};

// ---------------------------------------------------------------------------
// Vault
// ---------------------------------------------------------------------------

/// Everything the app needs from a vault, on top of [`VaultWrite`].
pub trait VaultBackend: VaultWrite {
    /// Display name of the vault.
    fn name(&self) -> String;
    /// Folder on disk, if any.
    fn root(&self) -> Option<PathBuf>;
    /// All folders (vault-relative, sorted).
    fn list_folders(&self) -> Result<Vec<String>>;
    fn create_folder(&self, folder: &str) -> Result<()>;
    /// Create `"<name>.md"` in `folder`, deduplicating the name like Obsidian.
    fn create_note(&self, folder: &str, name: &str, contents: &str) -> Result<NotePath>;
    /// Delete a folder and everything inside it.
    fn delete_folder(&self, folder: &str) -> Result<()>;
    fn app_config(&self) -> Result<AppConfig>;
    fn open_daily_note(&self, date: Date) -> Result<NotePath>;
    /// Start the file watcher (no-op for in-memory vaults).
    fn start_watching(&mut self) -> Result<()>;
    /// Drain pending external change events.
    fn poll_events(&mut self) -> Vec<VaultEvent>;
    /// Write a binary file (pasted/dropped attachments), creating folders.
    fn write_bytes(&self, path: &NotePath, bytes: &[u8]) -> Result<()>;
    /// A free path for a new attachment named `file_name` pasted into
    /// `note`, honouring `attachmentFolderPath`. Nothing is created.
    fn attachment_path(&self, note: Option<&NotePath>, file_name: &str) -> Result<NotePath> {
        let folder = self.app_config()?.attachment_folder(note);
        let (stem, ext) = match file_name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (file_name.to_string(), String::new()),
        };
        let mut candidate = NotePath::join(&folder, file_name)?;
        let mut n = 1;
        while self.exists(&candidate) {
            candidate = NotePath::join(&folder, &format!("{stem} {n}{ext}"))?;
            n += 1;
        }
        Ok(candidate)
    }
}

/// The on-disk vault from `ors-vault`, plus its watcher.
pub struct DiskVault {
    vault: Vault,
    watcher: Option<(VaultWatcher, Receiver<VaultEvent>)>,
}

impl DiskVault {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            return Err(Error::InvalidPath(root.display().to_string()));
        }
        Ok(Self {
            vault: Vault::open(&root)?,
            watcher: None,
        })
    }
}

impl VaultRead for DiskVault {
    fn list_files(&self) -> Result<Vec<NotePath>> {
        self.vault.list_files()
    }
    fn read_to_string(&self, path: &NotePath) -> Result<String> {
        self.vault.read_to_string(path)
    }
    fn read_bytes(&self, path: &NotePath) -> Result<Vec<u8>> {
        self.vault.read_bytes(path)
    }
    fn metadata(&self, path: &NotePath) -> Result<FileMeta> {
        self.vault.metadata(path)
    }
    fn exists(&self, path: &NotePath) -> bool {
        self.vault.exists(path)
    }
}

impl VaultWrite for DiskVault {
    fn write(&self, path: &NotePath, contents: &str) -> Result<()> {
        self.vault.write(path, contents)
    }
    fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        self.vault.rename(from, to)
    }
    fn delete(&self, path: &NotePath) -> Result<()> {
        self.vault.delete(path)
    }
}

impl VaultBackend for DiskVault {
    fn name(&self) -> String {
        self.vault.name()
    }
    fn root(&self) -> Option<PathBuf> {
        Some(self.vault.root().to_path_buf())
    }
    fn list_folders(&self) -> Result<Vec<String>> {
        self.vault.list_folders()
    }
    fn create_folder(&self, folder: &str) -> Result<()> {
        self.vault.create_folder(folder)
    }
    fn create_note(&self, folder: &str, name: &str, contents: &str) -> Result<NotePath> {
        self.vault.create_note(folder, name, contents)
    }
    fn delete_folder(&self, folder: &str) -> Result<()> {
        self.vault.delete_folder(folder)
    }
    fn app_config(&self) -> Result<AppConfig> {
        self.vault.app_config()
    }
    fn open_daily_note(&self, date: Date) -> Result<NotePath> {
        self.vault.open_daily_note(date)
    }
    fn start_watching(&mut self) -> Result<()> {
        self.watcher = Some(self.vault.watch()?);
        Ok(())
    }
    fn poll_events(&mut self) -> Vec<VaultEvent> {
        match &self.watcher {
            Some((_, rx)) => rx.try_iter().collect(),
            None => Vec::new(),
        }
    }
    fn write_bytes(&self, path: &NotePath, bytes: &[u8]) -> Result<()> {
        self.vault.write_bytes(path, bytes)
    }
    fn attachment_path(&self, note: Option<&NotePath>, file_name: &str) -> Result<NotePath> {
        self.vault.available_attachment_path(note, file_name)
    }
}

/// In-memory vault (tests and `--demo`). Text files live in a
/// [`MemoryVault`]; binary attachments in a side map. External changes can be
/// simulated with [`MemVault::external_write`].
#[derive(Default)]
pub struct MemVault {
    name: String,
    text: MemoryVault,
    binary: RwLock<BTreeMap<NotePath, Vec<u8>>>,
    folders: RwLock<BTreeSet<String>>,
    pending: RwLock<Vec<VaultEvent>>,
    /// Config returned by [`VaultBackend::app_config`].
    pub config: AppConfig,
}

impl MemVault {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    pub fn with_files<'a>(name: &str, files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let v = Self::new(name);
        for (p, c) in files {
            if let Ok(p) = NotePath::new(p) {
                let _ = v.text.write(&p, c);
            }
        }
        v
    }

    pub fn add_binary(&self, path: &NotePath, bytes: Vec<u8>) {
        if let Ok(mut b) = self.binary.write() {
            b.insert(path.clone(), bytes);
        }
    }

    /// Simulate a change made by another program: writes the file and queues
    /// a watcher event.
    #[cfg(test)]
    pub fn external_write(&self, path: &NotePath, contents: &str) -> Result<()> {
        let existed = self.exists(path);
        self.text.write(path, contents)?;
        if let Ok(mut p) = self.pending.write() {
            p.push(if existed {
                VaultEvent::Modified(path.clone())
            } else {
                VaultEvent::Created(path.clone())
            });
        }
        Ok(())
    }

    fn lock_err() -> Error {
        Error::Other("memory vault lock poisoned".into())
    }
}

impl VaultRead for MemVault {
    fn list_files(&self) -> Result<Vec<NotePath>> {
        let mut files = self.text.list_files()?;
        files.extend(
            self.binary
                .read()
                .map_err(|_| Self::lock_err())?
                .keys()
                .cloned(),
        );
        files.sort();
        files.dedup();
        Ok(files)
    }
    fn read_to_string(&self, path: &NotePath) -> Result<String> {
        self.text.read_to_string(path)
    }
    fn read_bytes(&self, path: &NotePath) -> Result<Vec<u8>> {
        if let Some(b) = self.binary.read().map_err(|_| Self::lock_err())?.get(path) {
            return Ok(b.clone());
        }
        self.text.read_bytes(path)
    }
    fn metadata(&self, path: &NotePath) -> Result<FileMeta> {
        self.text.metadata(path)
    }
    fn exists(&self, path: &NotePath) -> bool {
        self.text.exists(path) || self.binary.read().is_ok_and(|b| b.contains_key(path))
    }
}

impl VaultWrite for MemVault {
    fn write(&self, path: &NotePath, contents: &str) -> Result<()> {
        self.text.write(path, contents)
    }
    fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        if self.exists(to) {
            return Err(Error::AlreadyExists(to.clone()));
        }
        let moved = {
            let mut b = self.binary.write().map_err(|_| Self::lock_err())?;
            match b.remove(from) {
                Some(bytes) => {
                    b.insert(to.clone(), bytes);
                    true
                }
                None => false,
            }
        };
        if moved {
            Ok(())
        } else {
            self.text.rename(from, to)
        }
    }
    fn delete(&self, path: &NotePath) -> Result<()> {
        let removed = self
            .binary
            .write()
            .map_err(|_| Self::lock_err())?
            .remove(path)
            .is_some();
        if removed {
            Ok(())
        } else {
            self.text.delete(path)
        }
    }
}

impl VaultBackend for MemVault {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn root(&self) -> Option<PathBuf> {
        None
    }
    fn list_folders(&self) -> Result<Vec<String>> {
        let mut out: BTreeSet<String> = self.folders.read().map_err(|_| Self::lock_err())?.clone();
        for f in self.list_files()? {
            let mut parent = f.parent();
            while let Some(p) = parent {
                out.insert(p.to_string());
                parent = p.rfind('/').map(|i| &p[..i]);
            }
        }
        Ok(out.into_iter().collect())
    }
    fn create_folder(&self, folder: &str) -> Result<()> {
        let p = NotePath::new(folder)?;
        self.folders
            .write()
            .map_err(|_| Self::lock_err())?
            .insert(p.as_str().to_string());
        Ok(())
    }
    fn create_note(&self, folder: &str, name: &str, contents: &str) -> Result<NotePath> {
        let mut candidate = NotePath::join(folder, &format!("{name}.md"))?;
        let mut n = 1;
        while self.exists(&candidate) {
            candidate = NotePath::join(folder, &format!("{name} {n}.md"))?;
            n += 1;
        }
        self.text.write(&candidate, contents)?;
        Ok(candidate)
    }
    fn delete_folder(&self, folder: &str) -> Result<()> {
        let prefix = format!("{folder}/");
        for f in self.list_files()? {
            if f.as_str().starts_with(&prefix) {
                self.delete(&f)?;
            }
        }
        self.folders
            .write()
            .map_err(|_| Self::lock_err())?
            .retain(|f| f != folder && !f.starts_with(&prefix));
        Ok(())
    }
    fn app_config(&self) -> Result<AppConfig> {
        Ok(self.config.clone())
    }
    fn open_daily_note(&self, date: Date) -> Result<NotePath> {
        let path = NotePath::new(format!(
            "Journal/{:04}-{:02}-{:02}.md",
            date.year, date.month, date.day
        ))?;
        if !self.exists(&path) {
            self.text.write(&path, &format!("# {}\n\n", path.stem()))?;
        }
        Ok(path)
    }
    fn start_watching(&mut self) -> Result<()> {
        Ok(())
    }
    fn poll_events(&mut self) -> Vec<VaultEvent> {
        self.pending
            .write()
            .map(|mut p| std::mem::take(&mut *p))
            .unwrap_or_default()
    }
    fn write_bytes(&self, path: &NotePath, bytes: &[u8]) -> Result<()> {
        self.add_binary(path, bytes.to_vec());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Everything the app needs from the metadata cache and the search index.
pub trait IndexBackend: LinkResolver {
    fn files(&self) -> Vec<NotePath>;
    fn get(&self, path: &NotePath) -> Option<&ParsedNote>;
    fn backlinks(&self, path: &NotePath) -> Vec<Backlink>;
    fn unlinked_mentions(&self, vault: &dyn VaultRead, path: &NotePath) -> Vec<NotePath>;
    fn outgoing(&self, path: &NotePath) -> Vec<ResolvedLink>;
    fn tags(&self) -> BTreeMap<String, usize>;
    fn graph(&self, options: &GraphOptions) -> GraphData;
    fn link_text_for(&self, target: &NotePath, from: &NotePath) -> String;
    /// File created/modified on disk.
    fn update_file(&mut self, vault: &dyn VaultRead, path: &NotePath);
    /// Unsaved editor buffer changed.
    fn update_from_source(&mut self, path: &NotePath, source: &str);
    fn remove_file(&mut self, path: &NotePath);
    fn rename_file(&mut self, from: &NotePath, to: &NotePath);
    fn search(&self, query: &str) -> Result<Vec<SearchResult>>;
    fn fuzzy_find(&self, query: &str, limit: usize) -> Vec<(NotePath, Option<String>, i64)>;
    /// [`IndexBackend::fuzzy_find`], optionally including attachments
    /// (`![[` completion).
    fn fuzzy_find_files(
        &self,
        query: &str,
        limit: usize,
        include_attachments: bool,
    ) -> Vec<(NotePath, Option<String>, i64)> {
        let _ = include_attachments;
        self.fuzzy_find(query, limit)
    }
    /// Tags matching `prefix` for `#` completion, best first, with counts.
    fn tag_suggestions(&self, prefix: &str, limit: usize) -> Vec<(String, usize)> {
        let q = prefix.trim_start_matches('#').to_lowercase();
        let mut v: Vec<(String, usize)> = self
            .tags()
            .into_iter()
            .filter(|(t, _)| t.to_lowercase().contains(&q))
            .collect();
        v.sort_by(|a, b| {
            let pa = !a.0.to_lowercase().starts_with(&q);
            let pb = !b.0.to_lowercase().starts_with(&q);
            pa.cmp(&pb).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0))
        });
        v.truncate(limit);
        v
    }
    /// Rename on disk and rewrite links; returns notes whose contents changed.
    fn rename_with_links(
        &mut self,
        vault: &dyn VaultWrite,
        from: &NotePath,
        to: &NotePath,
    ) -> Result<Vec<NotePath>>;
    /// Move a folder and rewrite links; returns notes whose contents changed.
    fn rename_folder_with_links(
        &mut self,
        vault: &dyn VaultWrite,
        from: &str,
        to: &str,
    ) -> Result<Vec<NotePath>> {
        let prefix = format!("{from}/");
        let mut changed = Vec::new();
        for f in self.files() {
            if let Some(rest) = f.as_str().strip_prefix(&prefix) {
                let target = NotePath::join(to, rest)?;
                changed.extend(self.rename_with_links(vault, &f, &target)?);
            }
        }
        Ok(changed)
    }
    /// A layout engine for the graph view.
    fn new_layout(&self, graph: &GraphData) -> Box<dyn LayoutEngine>;
    /// Parse a markdown fragment (embeds, sections).
    fn parse(&self, source: &str) -> ParsedNote {
        ors_markdown::parse(source)
    }
}

/// [`MetadataCache`] + [`SearchIndex`] from `ors-index`.
pub struct RealIndex {
    cache: MetadataCache,
    search: SearchIndex,
}

impl RealIndex {
    pub fn build(vault: &dyn VaultRead) -> Result<Self> {
        let cache = MetadataCache::build(vault)?;
        let search = SearchIndex::build(vault, &cache)?;
        Ok(Self { cache, search })
    }
}

impl LinkResolver for RealIndex {
    fn resolve(&self, target: &str, from: &NotePath) -> Option<NotePath> {
        self.cache.resolve(target, from)
    }
}

impl IndexBackend for RealIndex {
    fn files(&self) -> Vec<NotePath> {
        self.cache.files()
    }
    fn get(&self, path: &NotePath) -> Option<&ParsedNote> {
        self.cache.get(path)
    }
    fn backlinks(&self, path: &NotePath) -> Vec<Backlink> {
        self.cache.backlinks(path)
    }
    fn unlinked_mentions(&self, vault: &dyn VaultRead, path: &NotePath) -> Vec<NotePath> {
        self.cache.unlinked_mentions(vault, path)
    }
    fn outgoing(&self, path: &NotePath) -> Vec<ResolvedLink> {
        self.cache.outgoing(path)
    }
    fn tags(&self) -> BTreeMap<String, usize> {
        self.cache.tags()
    }
    fn graph(&self, options: &GraphOptions) -> GraphData {
        self.cache.graph(options)
    }
    fn link_text_for(&self, target: &NotePath, from: &NotePath) -> String {
        self.cache.link_text_for(target, from)
    }
    fn update_file(&mut self, vault: &dyn VaultRead, path: &NotePath) {
        let _ = self.cache.update_file(vault, path);
        if path.is_markdown()
            && let Ok(src) = vault.read_to_string(path)
        {
            self.search.update(path, &src);
        }
    }
    fn update_from_source(&mut self, path: &NotePath, source: &str) {
        self.cache.update_from_source(path, source);
        self.search.update(path, source);
    }
    fn remove_file(&mut self, path: &NotePath) {
        self.cache.remove_file(path);
        self.search.remove(path);
    }
    fn rename_file(&mut self, from: &NotePath, to: &NotePath) {
        self.cache.rename_file(from, to);
        self.search.rename(from, to);
    }
    fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.search.search(query, &self.cache)
    }
    fn fuzzy_find(&self, query: &str, limit: usize) -> Vec<(NotePath, Option<String>, i64)> {
        ors_index::fuzzy_find(&self.cache, query, limit)
    }
    fn fuzzy_find_files(
        &self,
        query: &str,
        limit: usize,
        include_attachments: bool,
    ) -> Vec<(NotePath, Option<String>, i64)> {
        ors_index::fuzzy_find_with(
            &self.cache,
            query,
            limit,
            ors_index::FuzzyOptions {
                include_attachments,
            },
        )
    }
    fn tag_suggestions(&self, prefix: &str, limit: usize) -> Vec<(String, usize)> {
        self.cache.tag_suggestions(prefix, limit)
    }
    fn rename_with_links(
        &mut self,
        vault: &dyn VaultWrite,
        from: &NotePath,
        to: &NotePath,
    ) -> Result<Vec<NotePath>> {
        let changed = ors_index::rename_with_links(vault, &mut self.cache, from, to)?;
        self.search.rename(from, to);
        for p in &changed {
            if let Ok(src) = vault.read_to_string(p) {
                self.search.update(p, &src);
            }
        }
        Ok(changed)
    }
    fn rename_folder_with_links(
        &mut self,
        vault: &dyn VaultWrite,
        from: &str,
        to: &str,
    ) -> Result<Vec<NotePath>> {
        let prefix = format!("{from}/");
        let moved: Vec<NotePath> = self
            .cache
            .files()
            .into_iter()
            .filter(|f| f.as_str().starts_with(&prefix))
            .collect();
        let changed = ors_index::rename_folder_with_links(vault, &mut self.cache, from, to)?;
        for f in moved {
            if let Some(rest) = f.as_str().strip_prefix(&prefix)
                && let Ok(target) = NotePath::join(to, rest)
            {
                self.search.rename(&f, &target);
            }
        }
        for p in &changed {
            if let Ok(src) = vault.read_to_string(p) {
                self.search.update(p, &src);
            }
        }
        Ok(changed)
    }
    fn new_layout(&self, graph: &GraphData) -> Box<dyn LayoutEngine> {
        Box::new(ForceEngine(ForceLayout::new(graph, ForceParams::default())))
    }
}

// ---------------------------------------------------------------------------
// Graph layout
// ---------------------------------------------------------------------------

/// Layout simulation used by the graph view.
pub trait LayoutEngine {
    /// Advance; returns kinetic energy.
    fn step(&mut self, graph: &GraphData, dt: f32) -> f32;
    fn positions(&self) -> &[Vec2];
    fn pin(&mut self, node: usize, position: Option<Vec2>);
    /// Keep positions of surviving nodes after the graph changed.
    fn sync(&mut self, old: &GraphData, new: &GraphData);
    fn hit_test(&self, point: Vec2, radius: f32) -> Option<usize>;
    fn bounds(&self) -> (Vec2, Vec2);
    /// Wake the simulation up (after a drag or a graph change).
    fn reheat(&mut self, _alpha: f32) {}
    /// Change the forces (graph settings sliders).
    fn set_params(&mut self, _params: ForceParams) {}
}

/// [`ForceLayout`] from `ors-graph`.
pub struct ForceEngine(pub ForceLayout);

impl LayoutEngine for ForceEngine {
    fn step(&mut self, graph: &GraphData, dt: f32) -> f32 {
        self.0.step(graph, dt)
    }
    fn positions(&self) -> &[Vec2] {
        self.0.positions()
    }
    fn pin(&mut self, node: usize, position: Option<Vec2>) {
        self.0.pin(node, position);
    }
    fn sync(&mut self, old: &GraphData, new: &GraphData) {
        self.0.sync(old, new);
    }
    fn hit_test(&self, point: Vec2, radius: f32) -> Option<usize> {
        self.0.hit_test(point, radius)
    }
    fn bounds(&self) -> (Vec2, Vec2) {
        self.0.bounds()
    }
    fn reheat(&mut self, alpha: f32) {
        self.0.reheat(alpha);
    }
    fn set_params(&mut self, params: ForceParams) {
        self.0.params = params;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_vault_folders_and_dedup() {
        let v = MemVault::with_files("t", [("a/b/c.md", "x"), ("d.md", "")]);
        assert_eq!(v.list_folders().unwrap(), vec!["a", "a/b"]);
        let p = v.create_note("", "d", "").unwrap();
        assert_eq!(p.as_str(), "d 1.md");
        v.delete_folder("a").unwrap();
        assert!(!v.exists(&NotePath::new("a/b/c.md").unwrap()));
    }

    #[test]
    fn mem_vault_binary_and_events() {
        let mut v = MemVault::new("t");
        let img = NotePath::new("img.png").unwrap();
        v.add_binary(&img, vec![1, 2, 3]);
        assert_eq!(v.read_bytes(&img).unwrap(), vec![1, 2, 3]);
        assert_eq!(v.list_files().unwrap(), vec![img.clone()]);
        let n = NotePath::new("n.md").unwrap();
        v.external_write(&n, "hi").unwrap();
        assert_eq!(v.poll_events(), vec![VaultEvent::Created(n)]);
        assert!(v.poll_events().is_empty());
    }
}
