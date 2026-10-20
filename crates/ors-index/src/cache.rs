//! [`MetadataCache`]: per-file metadata, link resolution, reverse link index
//! and tag index, all maintained incrementally.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ors_core::{
    GraphData, GraphEdge, GraphNode, GraphNodeKind, Link, LinkKind, LinkResolver, NotePath,
    ParsedNote, Result, VaultRead,
};

use crate::text::{self, Folded, key, name_key};
use crate::{Backlink, GraphOptions, ResolvedLink, SearchMatch};

/// Metadata of one markdown note plus the resolution of each of its links
/// (parallel to `parsed.links`).
#[derive(Debug, Clone)]
pub(crate) struct NoteData {
    pub parsed: ParsedNote,
    pub resolved: Vec<Option<NotePath>>,
}

/// Aggregate counts over the cache (for `obsirust stats` and status bars).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    /// Markdown notes.
    pub notes: usize,
    /// Non-markdown files.
    pub attachments: usize,
    /// Links and embeds (including frontmatter links), resolved or not.
    pub links: usize,
    /// Links whose target exists.
    pub resolved_links: usize,
    /// Links whose target does not exist.
    pub unresolved_links: usize,
    /// Distinct tags (lowercased, including parents of nested tags).
    pub tags: usize,
    /// Total body word count.
    pub words: usize,
}

/// In-memory index of the vault's metadata.
///
/// Holds every file of the vault (markdown notes with their [`ParsedNote`],
/// attachments as bare paths), resolves links with Obsidian's rules and keeps
/// reverse indexes (backlinks, link names, tags) consistent under incremental
/// updates.
#[derive(Debug, Default, Clone)]
pub struct MetadataCache {
    /// All files; `Some` for markdown notes, `None` for attachments.
    pub(crate) files: BTreeMap<NotePath, Option<NoteData>>,
    /// `key(full path)` -> files with that key (several only when paths differ by case).
    pub(crate) path_index: HashMap<String, Vec<NotePath>>,
    /// `name_key(file)` -> files.
    pub(crate) name_index: HashMap<String, BTreeSet<NotePath>>,
    /// `name_key(link target)` -> notes containing such a link.
    pub(crate) link_names: HashMap<String, BTreeSet<NotePath>>,
    /// Resolved target -> notes having at least one link resolving to it.
    pub(crate) backrefs: HashMap<NotePath, BTreeSet<NotePath>>,
    /// Notes having at least one unresolved link.
    pub(crate) unresolved_sources: BTreeSet<NotePath>,
    /// Tag key (and parents) -> occurrences.
    pub(crate) tag_counts: BTreeMap<String, usize>,
    /// Tag key (and parents) -> note -> occurrences.
    pub(crate) tag_files: HashMap<String, BTreeMap<NotePath, usize>>,
}

/// Every `/`-prefix of a tag key: `a/b/c` -> `a`, `a/b`, `a/b/c`.
fn tag_prefixes(tag: &str) -> impl Iterator<Item = &str> {
    tag.match_indices('/')
        .map(move |(i, _)| &tag[..i])
        .chain(std::iter::once(tag))
        .filter(|p| !p.is_empty())
}

/// Normalized key of a tag as written (`#Projet/Rust` -> `projet/rust`).
pub(crate) fn tag_key(tag: &str) -> String {
    key(tag.trim().trim_start_matches('#').trim_end_matches('/'))
}

fn parse_all(sources: Vec<(NotePath, String)>) -> Vec<(NotePath, ParsedNote)> {
    let threads = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .min(16);
    if threads <= 1 || sources.len() < 64 {
        return sources
            .into_iter()
            .map(|(p, s)| {
                let parsed = ors_markdown::parse(&s);
                (p, parsed)
            })
            .collect();
    }
    let chunk = sources.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|(p, s)| (p.clone(), ors_markdown::parse(s)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut out = Vec::with_capacity(sources.len());
        for (h, part) in handles.into_iter().zip(sources.chunks(chunk)) {
            match h.join() {
                Ok(v) => out.extend(v),
                // A panicking parser thread: parse that chunk again on this thread
                // so the panic surfaces normally instead of losing notes silently.
                Err(_) => out.extend(
                    part.iter()
                        .map(|(p, s)| (p.clone(), ors_markdown::parse(s))),
                ),
            }
        }
        out
    })
}

impl MetadataCache {
    /// Empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse every note of the vault (in parallel) and register every attachment.
    pub fn build(vault: &dyn VaultRead) -> Result<Self> {
        let mut sources = Vec::new();
        let mut attachments = Vec::new();
        for path in vault.list_files()? {
            if path.is_markdown() {
                let src = vault.read_to_string(&path)?;
                sources.push((path, src));
            } else {
                attachments.push(path);
            }
        }
        let parsed = parse_all(sources);
        let mut cache = Self::default();
        cache.extend_entries(
            parsed
                .into_iter()
                .map(|(p, n)| (p, Some(n)))
                .chain(attachments.into_iter().map(|p| (p, None))),
        );
        Ok(cache)
    }

    /// (Re)index a single file after it was created or modified. Removes it
    /// from the cache if it no longer exists in the vault.
    pub fn update_file(&mut self, vault: &dyn VaultRead, path: &NotePath) -> Result<()> {
        if !vault.exists(path) {
            self.remove_file(path);
            return Ok(());
        }
        if path.is_markdown() {
            let src = vault.read_to_string(path)?;
            self.update_from_source(path, &src);
        } else {
            self.insert_file(path.clone());
        }
        Ok(())
    }

    /// Index a file from in-memory contents (unsaved editor buffer).
    /// Non-markdown paths are registered as attachments.
    pub fn update_from_source(&mut self, path: &NotePath, source: &str) {
        if path.is_markdown() {
            self.insert_parsed(path.clone(), ors_markdown::parse(source));
        } else {
            self.insert_file(path.clone());
        }
    }

    /// Insert or replace a markdown note from an already-parsed [`ParsedNote`].
    /// Links of other notes that may now resolve to it are re-resolved.
    pub fn insert_parsed(&mut self, path: NotePath, note: ParsedNote) {
        self.extend_entries(std::iter::once((path, Some(note))));
    }

    /// Register a file without metadata (attachment, canvas, ...). Markdown
    /// paths are registered as empty notes unless already known.
    pub fn insert_file(&mut self, path: NotePath) {
        if self.files.contains_key(&path) {
            return;
        }
        let entry = path.is_markdown().then(ParsedNote::default);
        self.extend_entries(std::iter::once((path, entry)));
    }

    /// Bulk version of [`insert_parsed`](Self::insert_parsed): links are
    /// re-resolved once at the end, which is much faster for large batches.
    pub fn extend_parsed(&mut self, notes: impl IntoIterator<Item = (NotePath, ParsedNote)>) {
        self.extend_entries(notes.into_iter().map(|(p, n)| (p, Some(n))));
    }

    fn extend_entries(
        &mut self,
        entries: impl IntoIterator<Item = (NotePath, Option<ParsedNote>)>,
    ) {
        let mut affected: BTreeSet<NotePath> = BTreeSet::new();
        let mut new_names: BTreeSet<String> = BTreeSet::new();
        for (path, parsed) in entries {
            if self.files.contains_key(&path) {
                self.detach(&path);
            } else {
                self.index_path(&path);
                new_names.insert(name_key(path.as_str()));
            }
            match parsed {
                Some(parsed) => {
                    self.attach(&path, &parsed);
                    self.files.insert(
                        path.clone(),
                        Some(NoteData {
                            parsed,
                            resolved: Vec::new(),
                        }),
                    );
                    affected.insert(path);
                }
                None => {
                    self.files.insert(path, None);
                }
            }
        }
        for name in &new_names {
            if let Some(srcs) = self.link_names.get(name) {
                affected.extend(srcs.iter().cloned());
            }
        }
        for src in &affected {
            self.reresolve(src);
        }
    }

    /// Remove a file. Links pointing to it become unresolved (or resolve to
    /// another file with the same name).
    pub fn remove_file(&mut self, path: &NotePath) {
        if !self.files.contains_key(path) {
            return;
        }
        self.detach(path);
        self.files.remove(path);
        self.unindex_path(path);
        let affected = self.backrefs.remove(path).unwrap_or_default();
        for src in &affected {
            if src != path {
                self.reresolve(src);
            }
        }
    }

    /// Move a file's metadata from `from` to `to` (the file contents are
    /// unchanged; links *inside* other notes are not rewritten — see
    /// [`crate::rename_with_links`]). If `to` already exists it is replaced.
    pub fn rename_file(&mut self, from: &NotePath, to: &NotePath) {
        if from == to || !self.files.contains_key(from) {
            return;
        }
        let data = self
            .files
            .get(from)
            .and_then(|d| d.as_ref().map(|d| d.parsed.clone()));
        self.remove_file(to);
        self.remove_file(from);
        let entry = if to.is_markdown() {
            Some(data.unwrap_or_default())
        } else {
            None
        };
        self.extend_entries(std::iter::once((to.clone(), entry)));
    }

    /// Metadata-only folder move (after e.g. `ors_vault::Vault::rename_folder`):
    /// every file under `from_folder/` is moved under `to_folder/`. Links are
    /// not rewritten — see [`crate::rename_folder_with_links`].
    pub fn rename_folder(&mut self, from_folder: &str, to_folder: &str) {
        let from_f = from_folder.trim_matches('/');
        let to_f = to_folder.trim_matches('/');
        if from_f.is_empty() || from_f == to_f {
            return;
        }
        let prefix = format!("{from_f}/");
        let moved: Vec<NotePath> = self
            .files
            .keys()
            .filter(|p| p.as_str().starts_with(&prefix))
            .cloned()
            .collect();
        for from in moved {
            if let Ok(to) = NotePath::join(to_f, &from.as_str()[prefix.len()..]) {
                self.rename_file(&from, &to);
            }
        }
    }

    fn index_path(&mut self, path: &NotePath) {
        let v = self.path_index.entry(key(path.as_str())).or_default();
        v.push(path.clone());
        v.sort();
        self.name_index
            .entry(name_key(path.as_str()))
            .or_default()
            .insert(path.clone());
    }

    fn unindex_path(&mut self, path: &NotePath) {
        let k = key(path.as_str());
        if let Some(v) = self.path_index.get_mut(&k) {
            v.retain(|p| p != path);
            if v.is_empty() {
                self.path_index.remove(&k);
            }
        }
        let n = name_key(path.as_str());
        if let Some(s) = self.name_index.get_mut(&n) {
            s.remove(path);
            if s.is_empty() {
                self.name_index.remove(&n);
            }
        }
    }

    /// Register the tag and link-name entries of a note.
    fn attach(&mut self, path: &NotePath, parsed: &ParsedNote) {
        for tag in &parsed.tags {
            let k = tag_key(&tag.name);
            for prefix in tag_prefixes(&k) {
                *self.tag_counts.entry(prefix.to_string()).or_default() += 1;
                *self
                    .tag_files
                    .entry(prefix.to_string())
                    .or_default()
                    .entry(path.clone())
                    .or_default() += 1;
            }
        }
        for link in &parsed.links {
            if !link.target.trim().is_empty() {
                self.link_names
                    .entry(name_key(link.target.trim()))
                    .or_default()
                    .insert(path.clone());
            }
        }
    }

    /// Undo [`attach`](Self::attach) and drop the note's outgoing entries from
    /// the reverse index. The file itself stays registered.
    fn detach(&mut self, path: &NotePath) {
        let Some(Some(data)) = self.files.get_mut(path) else {
            return;
        };
        let parsed = std::mem::take(&mut data.parsed);
        let resolved = std::mem::take(&mut data.resolved);
        for tag in &parsed.tags {
            let k = tag_key(&tag.name);
            for prefix in tag_prefixes(&k) {
                if let Some(c) = self.tag_counts.get_mut(prefix) {
                    *c = c.saturating_sub(1);
                    if *c == 0 {
                        self.tag_counts.remove(prefix);
                    }
                }
                if let Some(files) = self.tag_files.get_mut(prefix) {
                    if let Some(c) = files.get_mut(path) {
                        *c = c.saturating_sub(1);
                        if *c == 0 {
                            files.remove(path);
                        }
                    }
                    if files.is_empty() {
                        self.tag_files.remove(prefix);
                    }
                }
            }
        }
        for link in &parsed.links {
            let n = name_key(link.target.trim());
            if let Some(s) = self.link_names.get_mut(&n) {
                s.remove(path);
                if s.is_empty() {
                    self.link_names.remove(&n);
                }
            }
        }
        for target in resolved.iter().flatten() {
            if let Some(s) = self.backrefs.get_mut(target) {
                s.remove(path);
                if s.is_empty() {
                    self.backrefs.remove(target);
                }
            }
        }
        self.unresolved_sources.remove(path);
    }

    /// Recompute the resolution of every link of `src` and update the reverse
    /// index accordingly.
    pub(crate) fn reresolve(&mut self, src: &NotePath) {
        let Some(Some(data)) = self.files.get(src) else {
            return;
        };
        let new: Vec<Option<NotePath>> = data
            .parsed
            .links
            .iter()
            .map(|l| self.resolve_link(l, src))
            .collect();
        let old_targets: BTreeSet<NotePath> = data.resolved.iter().flatten().cloned().collect();
        let new_targets: BTreeSet<NotePath> = new.iter().flatten().cloned().collect();
        let has_unresolved = new.iter().any(Option::is_none);
        for t in old_targets.difference(&new_targets) {
            if let Some(s) = self.backrefs.get_mut(t) {
                s.remove(src);
                if s.is_empty() {
                    self.backrefs.remove(t);
                }
            }
        }
        for t in new_targets.difference(&old_targets) {
            self.backrefs
                .entry(t.clone())
                .or_default()
                .insert(src.clone());
        }
        if has_unresolved {
            self.unresolved_sources.insert(src.clone());
        } else {
            self.unresolved_sources.remove(src);
        }
        if let Some(Some(data)) = self.files.get_mut(src) {
            data.resolved = new;
        }
    }

    /// All known files (notes + attachments), sorted.
    pub fn files(&self) -> Vec<NotePath> {
        self.files.keys().cloned().collect()
    }

    /// Markdown notes, sorted.
    pub fn notes(&self) -> Vec<NotePath> {
        self.files
            .iter()
            .filter(|(_, d)| d.is_some())
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Non-markdown files, sorted.
    pub fn attachments(&self) -> Vec<NotePath> {
        self.files
            .iter()
            .filter(|(_, d)| d.is_none())
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Number of files (notes + attachments).
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the cache holds no file.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Whether `path` is a known file.
    pub fn contains(&self, path: &NotePath) -> bool {
        self.files.contains_key(path)
    }

    /// Parsed metadata of a markdown note.
    pub fn get(&self, path: &NotePath) -> Option<&ParsedNote> {
        self.files
            .get(path)
            .and_then(|d| d.as_ref().map(|d| &d.parsed))
    }

    /// Links pointing to `path` (including embeds and frontmatter links),
    /// grouped by source in path order. Links of the note to itself are not
    /// reported.
    pub fn backlinks(&self, path: &NotePath) -> Vec<Backlink> {
        let mut out = Vec::new();
        let Some(sources) = self.backrefs.get(path) else {
            return out;
        };
        for src in sources {
            if src == path {
                continue;
            }
            if let Some(Some(data)) = self.files.get(src) {
                for (link, res) in data.parsed.links.iter().zip(&data.resolved) {
                    if res.as_ref() == Some(path) {
                        out.push(Backlink {
                            source: src.clone(),
                            link: link.clone(),
                        });
                    }
                }
            }
        }
        out
    }

    /// Distinct notes linking to `path` (excluding itself), sorted.
    pub fn backlink_sources(&self, path: &NotePath) -> Vec<NotePath> {
        self.backrefs
            .get(path)
            .map(|s| s.iter().filter(|p| *p != path).cloned().collect())
            .unwrap_or_default()
    }

    /// Outgoing links and embeds of `path` with their resolution, in source order.
    pub fn outgoing(&self, path: &NotePath) -> Vec<ResolvedLink> {
        match self.files.get(path) {
            Some(Some(data)) => data
                .parsed
                .links
                .iter()
                .zip(&data.resolved)
                .map(|(link, target)| ResolvedLink {
                    link: link.clone(),
                    target: target.clone(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Source note -> unresolved targets (as written, deduplicated, in source order).
    pub fn unresolved(&self) -> BTreeMap<NotePath, Vec<String>> {
        let mut out = BTreeMap::new();
        for src in &self.unresolved_sources {
            if let Some(Some(data)) = self.files.get(src) {
                let mut targets: Vec<String> = Vec::new();
                for (link, res) in data.parsed.links.iter().zip(&data.resolved) {
                    if res.is_none() && !targets.contains(&link.target) {
                        targets.push(link.target.clone());
                    }
                }
                if !targets.is_empty() {
                    out.insert(src.clone(), targets);
                }
            }
        }
        out
    }

    /// Notes mentioning the title/aliases of `path` in plain text without linking it.
    ///
    /// An occurrence is a case-insensitive whole-word match of the note stem
    /// or one of its aliases, outside the frontmatter and outside any link.
    /// The note itself is excluded.
    pub fn unlinked_mentions(&self, vault: &dyn VaultRead, path: &NotePath) -> Vec<NotePath> {
        self.unlinked_mentions_detailed(vault, path)
            .into_iter()
            .map(|(p, _)| p)
            .collect()
    }

    /// Like [`unlinked_mentions`](Self::unlinked_mentions) but also returns
    /// the matching lines with byte ranges (for highlighting / "link" actions).
    pub fn unlinked_mentions_detailed(
        &self,
        vault: &dyn VaultRead,
        path: &NotePath,
    ) -> Vec<(NotePath, Vec<SearchMatch>)> {
        let mut needles: Vec<String> = Vec::new();
        let mut add = |s: &str| {
            let f = Folded::new(s.trim()).text;
            if !f.is_empty() && !needles.contains(&f) {
                needles.push(f);
            }
        };
        add(path.stem());
        if let Some(note) = self.get(path) {
            for a in &note.aliases {
                add(a);
            }
        }
        let mut out = Vec::new();
        for (src, data) in &self.files {
            let Some(data) = data else { continue };
            if src == path {
                continue;
            }
            let Ok(source) = vault.read_to_string(src) else {
                continue;
            };
            let matches = mention_matches(&source, &data.parsed, &needles);
            if !matches.is_empty() {
                out.push((src.clone(), matches));
            }
        }
        out
    }

    /// Tag (without `#`, lowercased) -> number of occurrences. Parent tags of
    /// nested tags (`a` for `a/b`) are counted too, like Obsidian's tag pane.
    pub fn tags(&self) -> BTreeMap<String, usize> {
        self.tag_counts.clone()
    }

    /// Distinct tags of one note (lowercased, without `#`, no parents), sorted.
    pub fn tags_of(&self, path: &NotePath) -> Vec<String> {
        let mut v: Vec<String> = self
            .get(path)
            .map(|n| n.tags.iter().map(|t| tag_key(&t.name)).collect())
            .unwrap_or_default();
        v.sort();
        v.dedup();
        v
    }

    /// Notes having `tag` or one of its nested children (case-insensitive,
    /// with or without the leading `#`), sorted.
    pub fn files_with_tag(&self, tag: &str) -> Vec<NotePath> {
        self.tag_files
            .get(&tag_key(tag))
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Whether `path` has `tag` or one of its nested children.
    pub fn has_tag(&self, path: &NotePath, tag: &str) -> bool {
        self.tag_files
            .get(&tag_key(tag))
            .is_some_and(|m| m.contains_key(path))
    }

    /// Notes without any incoming or outgoing resolved link (links of a note
    /// to itself do not count).
    pub fn orphans(&self) -> Vec<NotePath> {
        self.files
            .iter()
            .filter_map(|(p, d)| {
                let data = d.as_ref()?;
                let has_out = data.resolved.iter().flatten().any(|t| t != p);
                let has_in = self
                    .backrefs
                    .get(p)
                    .is_some_and(|s| s.iter().any(|x| x != p));
                (!has_out && !has_in).then(|| p.clone())
            })
            .collect()
    }

    /// Aggregate counts.
    pub fn stats(&self) -> CacheStats {
        let mut s = CacheStats {
            tags: self.tag_counts.len(),
            ..CacheStats::default()
        };
        for data in self.files.values() {
            match data {
                Some(d) => {
                    s.notes += 1;
                    s.links += d.resolved.len();
                    s.resolved_links += d.resolved.iter().filter(|r| r.is_some()).count();
                    s.words += d.parsed.word_count;
                }
                None => s.attachments += 1,
            }
        }
        s.unresolved_links = s.links - s.resolved_links;
        s
    }

    /// The link graph. Node ids: vault path for notes/attachments, raw target
    /// for unresolved links (case-insensitively deduplicated, first spelling
    /// wins), `"#tag"` for tags. Edges are directed and deduplicated; self
    /// links are dropped. Nodes without edges are dropped unless
    /// `include_orphans`.
    pub fn graph(&self, options: &GraphOptions) -> GraphData {
        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for (p, d) in &self.files {
            let is_note = d.is_some();
            if !is_note && !options.include_attachments {
                continue;
            }
            index.insert(p.as_str().to_string(), nodes.len());
            nodes.push(GraphNode {
                id: p.as_str().to_string(),
                label: if is_note {
                    p.stem().to_string()
                } else {
                    p.file_name().to_string()
                },
                kind: if is_note {
                    GraphNodeKind::Note
                } else {
                    GraphNodeKind::Attachment
                },
                path: Some(p.clone()),
            });
        }
        let mut unresolved_index: HashMap<String, usize> = HashMap::new();
        let mut tag_index: HashMap<String, usize> = HashMap::new();
        let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
        for (p, d) in &self.files {
            let Some(data) = d else { continue };
            let Some(&src) = index.get(p.as_str()) else {
                continue;
            };
            for (link, res) in data.parsed.links.iter().zip(&data.resolved) {
                match res {
                    Some(t) => {
                        if t == p {
                            continue;
                        }
                        if let Some(&dst) = index.get(t.as_str()) {
                            edges.insert((src, dst));
                        }
                    }
                    None if options.include_unresolved => {
                        let k = key(link.target.trim());
                        let dst = *unresolved_index.entry(k).or_insert_with(|| {
                            nodes.push(GraphNode {
                                id: link.target.clone(),
                                label: link.target.clone(),
                                kind: GraphNodeKind::Unresolved,
                                path: None,
                            });
                            nodes.len() - 1
                        });
                        edges.insert((src, dst));
                    }
                    None => {}
                }
            }
            if options.include_tags {
                for tag in &data.parsed.tags {
                    let k = tag_key(&tag.name);
                    if k.is_empty() {
                        continue;
                    }
                    let dst = *tag_index.entry(k.clone()).or_insert_with(|| {
                        let id = format!("#{k}");
                        nodes.push(GraphNode {
                            id: id.clone(),
                            label: id,
                            kind: GraphNodeKind::Tag,
                            path: None,
                        });
                        nodes.len() - 1
                    });
                    edges.insert((src, dst));
                }
            }
        }
        let edges: Vec<GraphEdge> = edges
            .into_iter()
            .map(|(source, target)| GraphEdge { source, target })
            .collect();
        let mut graph = GraphData { nodes, edges };
        if !options.include_orphans {
            let degrees = graph.degrees();
            let mut remap = vec![usize::MAX; graph.nodes.len()];
            let mut kept = Vec::new();
            for (i, node) in graph.nodes.into_iter().enumerate() {
                if degrees[i] > 0 {
                    remap[i] = kept.len();
                    kept.push(node);
                }
            }
            graph.nodes = kept;
            for e in &mut graph.edges {
                e.source = remap[e.source];
                e.target = remap[e.target];
            }
        }
        graph
    }

    /// Shortest link text from `from` to `target` according to Obsidian's
    /// "shortest path when possible" rule: the basename (stem for notes, file
    /// name for attachments) if it is unique in the vault and resolves to
    /// `target`, else the full vault path (without `.md` for notes).
    pub fn link_text_for(&self, target: &NotePath, from: &NotePath) -> String {
        let name = if target.is_markdown() {
            target.stem()
        } else {
            target.file_name()
        };
        let file_key = key(target.file_name());
        let same_name = self
            .name_index
            .get(&name_key(target.as_str()))
            .map_or(0, |s| {
                s.iter().filter(|p| key(p.file_name()) == file_key).count()
            });
        if same_name <= 1
            && self.resolve_target(name, LinkKind::Wiki, from).as_ref() == Some(target)
        {
            return name.to_string();
        }
        self.full_link_text(target)
    }

    /// Full vault path link text (without `.md` for notes).
    pub(crate) fn full_link_text(&self, target: &NotePath) -> String {
        if target.is_markdown() {
            target.without_extension().to_string()
        } else {
            target.as_str().to_string()
        }
    }

    /// Resolution of a link found in note `from`, honouring the link kind
    /// (markdown links are resolved relative to the note's folder first).
    pub fn resolve_link(&self, link: &Link, from: &NotePath) -> Option<NotePath> {
        self.resolve_target(&link.target, link.kind, from)
    }
}

impl LinkResolver for MetadataCache {
    /// Resolves with wikilink semantics (see [`MetadataCache::resolve_target`]).
    fn resolve(&self, target: &str, from: &NotePath) -> Option<NotePath> {
        self.resolve_target(target, LinkKind::Wiki, from)
    }
}

/// Whole-word occurrences of any of `needles` (folded) in `source`, outside
/// the frontmatter and outside link spans, grouped by line.
fn mention_matches(source: &str, parsed: &ParsedNote, needles: &[String]) -> Vec<SearchMatch> {
    let folded = Folded::new(source);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for n in needles {
        for (s, e) in text::find_whole_words(&folded.text, n) {
            let (os, oe) = (folded.to_original(s), folded.to_original(e));
            let in_fm = parsed
                .frontmatter_span
                .is_some_and(|sp| os < sp.end && oe > sp.start);
            let in_link = parsed
                .links
                .iter()
                .any(|l| os < l.span.end && oe > l.span.start);
            if !in_fm && !in_link {
                ranges.push((os, oe));
            }
        }
    }
    crate::search::group_ranges_by_line(source, ranges)
}
