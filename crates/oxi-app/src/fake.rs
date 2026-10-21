//! Hand-made fake index used by `--demo` and the unit tests.
//!
//! While the upstream crates are being written, `oxi-index` / `oxi-markdown`
//! return empty data. To exercise the UI anyway, this module implements
//! [`IndexBackend`] with a deliberately naive scanner (wikilinks, `#tags`,
//! ATX headings, basename resolution, substring search). It is **not** used
//! for real vaults: those always go through [`crate::backend::RealIndex`].

use std::collections::BTreeMap;

use oxi_core::{
    GraphData, GraphEdge, GraphNode, GraphNodeKind, Heading, Link, LinkKind, LinkResolver,
    NotePath, ParsedNote, Result, Span, Subpath, Tag, VaultRead, VaultWrite,
};
use oxi_graph::Vec2;
use oxi_index::{
    Backlink, GraphOptions, PropertyInfo, ResolvedLink, SearchMatch, SearchResult, TagNode,
};

use crate::backend::{IndexBackend, LayoutEngine, MemVault};
use crate::util::fuzzy_score;

#[derive(Default)]
pub struct FakeIndex {
    files: Vec<NotePath>,
    notes: BTreeMap<NotePath, ParsedNote>,
    sources: BTreeMap<NotePath, String>,
}

impl FakeIndex {
    pub fn build(vault: &dyn VaultRead) -> Self {
        let mut idx = Self::default();
        for f in vault.list_files().unwrap_or_default() {
            idx.update_file(vault, &f);
        }
        idx
    }

    fn insert_file(&mut self, path: &NotePath) {
        if let Err(i) = self.files.binary_search(path) {
            self.files.insert(i, path.clone());
        }
    }
}

/// Very small scanner producing a [`ParsedNote`] (demo/test only).
pub fn naive_parse(src: &str) -> ParsedNote {
    let mut note = ParsedNote::default();
    let mut offset = 0;
    let mut in_code = false;
    let mut in_fm = false;
    for (line_no, line) in src.split_inclusive('\n').enumerate() {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if line_no == 0 && trimmed == "---" {
            in_fm = true;
        } else if in_fm {
            if trimmed == "---" {
                in_fm = false;
                note.frontmatter_span = Some(Span::new(0, offset + line.len()));
            } else if let Some(t) = trimmed.trim().strip_prefix("- ").or_else(|| {
                trimmed
                    .strip_prefix("tags:")
                    .map(str::trim)
                    .filter(|t| !t.is_empty() && !t.starts_with('['))
            }) && (trimmed.starts_with("tags:") || trimmed.starts_with("  - "))
            {
                note.tags.push(Tag {
                    name: t.trim().to_string(),
                    span: Span::new(offset, offset + line.len()),
                    line: line_no,
                    from_frontmatter: true,
                });
            }
        } else if trimmed.starts_with("```") {
            in_code = !in_code;
        } else if !in_code {
            scan_line(trimmed, offset, line_no, &mut note);
        }
        offset += line.len();
    }
    note.word_count = src
        .get(note.frontmatter_span.map_or(0, |s| s.end)..)
        .unwrap_or("")
        .split_whitespace()
        .count();
    note
}

fn scan_line(line: &str, offset: usize, line_no: usize, note: &mut ParsedNote) {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        note.headings.push(Heading {
            level: hashes as u8,
            text: line[hashes..].trim().to_string(),
            span: Span::new(offset, offset + line.len()),
            line: line_no,
        });
        return;
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            i = line[i + 1..].find('`').map_or(bytes.len(), |j| i + j + 2);
            continue;
        }
        if line[i..].starts_with("[[")
            && let Some(end) = line[i + 2..].find("]]")
        {
            let inner = &line[i + 2..i + 2 + end];
            let embed = i > 0 && bytes[i - 1] == b'!';
            let start = if embed { i - 1 } else { i };
            let (target_part, display) = match inner.split_once('|') {
                Some((t, d)) => (t, Some(d.to_string())),
                None => (inner, None),
            };
            let (target, subpath) = match target_part.split_once('#') {
                Some((t, s)) => (
                    t,
                    Some(match s.strip_prefix('^') {
                        Some(b) => Subpath::Block(b.to_string()),
                        None => Subpath::Heading(vec![s.to_string()]),
                    }),
                ),
                None => (target_part, None),
            };
            note.links.push(Link {
                target: target.to_string(),
                subpath,
                display,
                kind: LinkKind::Wiki,
                is_embed: embed,
                span: Span::new(offset + start, offset + i + 4 + end),
                line: line_no,
            });
            i += end + 4;
            continue;
        }
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let len = line[i + 1..]
                .find(|c: char| c.is_whitespace() || ",.;:!?()[]".contains(c))
                .unwrap_or(line.len() - i - 1);
            let name = &line[i + 1..i + 1 + len];
            if !name.is_empty() && !name.chars().all(|c| c.is_ascii_digit()) {
                note.tags.push(Tag {
                    name: name.to_string(),
                    span: Span::new(offset + i, offset + i + 1 + len),
                    line: line_no,
                    from_frontmatter: false,
                });
            }
            i += 1 + len;
            continue;
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
}

impl LinkResolver for FakeIndex {
    fn resolve(&self, target: &str, _from: &NotePath) -> Option<NotePath> {
        if target.is_empty() {
            return None;
        }
        let t = target.to_lowercase();
        self.files
            .iter()
            .find(|f| f.as_str().to_lowercase() == t || f.without_extension().to_lowercase() == t)
            .or_else(|| self.files.iter().find(|f| f.stem().to_lowercase() == t))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|f| f.file_name().to_lowercase() == t)
            })
            .cloned()
    }
}

/// Tag tree from flat `name -> count` tags (demo/test only).
fn fake_tag_tree(tags: &BTreeMap<String, usize>) -> Vec<TagNode> {
    fn insert(level: &mut Vec<TagNode>, parts: &[&str], depth: usize, count: usize) {
        let name = parts[depth];
        let full = parts[..=depth].join("/");
        let i = match level.iter().position(|n| n.full_name == full) {
            Some(i) => i,
            None => {
                level.push(TagNode {
                    name: name.to_string(),
                    full_name: full,
                    count: 0,
                    children: Vec::new(),
                });
                level.len() - 1
            }
        };
        if depth + 1 == parts.len() {
            level[i].count = count;
        } else {
            insert(&mut level[i].children, parts, depth + 1, count);
        }
    }
    let mut roots = Vec::new();
    for (tag, count) in tags {
        let parts: Vec<&str> = tag.split('/').collect();
        insert(&mut roots, &parts, 0, *count);
    }
    roots
}

impl FakeIndex {
    fn frontmatters(&self) -> impl Iterator<Item = (&NotePath, oxi_core::Frontmatter)> {
        self.sources
            .iter()
            .filter_map(|(p, s)| oxi_markdown::properties::get_properties(s).map(|f| (p, f)))
    }
}

impl IndexBackend for FakeIndex {
    fn files(&self) -> Vec<NotePath> {
        self.files.clone()
    }
    fn get(&self, path: &NotePath) -> Option<&ParsedNote> {
        self.notes.get(path)
    }
    fn backlinks(&self, path: &NotePath) -> Vec<Backlink> {
        let mut out = Vec::new();
        for (src, note) in &self.notes {
            for l in &note.links {
                if self.resolve(&l.target, src).as_ref() == Some(path) {
                    out.push(Backlink {
                        source: src.clone(),
                        link: l.clone(),
                    });
                }
            }
        }
        out
    }
    fn unlinked_mentions(&self, _vault: &dyn VaultRead, path: &NotePath) -> Vec<NotePath> {
        let needle = path.stem().to_lowercase();
        let linked: Vec<NotePath> = self.backlinks(path).into_iter().map(|b| b.source).collect();
        self.sources
            .iter()
            .filter(|(p, s)| {
                *p != path && !linked.contains(p) && s.to_lowercase().contains(&needle)
            })
            .map(|(p, _)| p.clone())
            .collect()
    }
    fn outgoing(&self, path: &NotePath) -> Vec<ResolvedLink> {
        self.notes
            .get(path)
            .map(|n| {
                n.links
                    .iter()
                    .map(|l| ResolvedLink {
                        link: l.clone(),
                        target: self.resolve(&l.target, path),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn properties(&self) -> BTreeMap<String, PropertyInfo> {
        let mut out: BTreeMap<String, PropertyInfo> = BTreeMap::new();
        for (_, fm) in self.frontmatters() {
            for (k, v) in &fm {
                let ty = oxi_markdown::properties::infer_type_name(v);
                let info = out.entry(k.to_lowercase()).or_insert_with(|| PropertyInfo {
                    name: k.clone(),
                    count: 0,
                    inferred_type: ty,
                    type_counts: BTreeMap::new(),
                });
                info.count += 1;
                *info.type_counts.entry(ty).or_default() += 1;
            }
        }
        out
    }
    fn property_values(&self, key: &str) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for (_, fm) in self.frontmatters() {
            let Some((_, v)) = fm.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) else {
                continue;
            };
            let items = match v {
                serde_json::Value::Array(a) => a.clone(),
                other => vec![other.clone()],
            };
            for i in items {
                let t = match i {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Null => continue,
                    other => other.to_string(),
                };
                if !t.trim().is_empty() {
                    *out.entry(t).or_insert(0) += 1;
                }
            }
        }
        out
    }
    fn files_with_property(&self, key: &str) -> Vec<NotePath> {
        self.frontmatters()
            .filter(|(_, fm)| fm.keys().any(|k| k.eq_ignore_ascii_case(key)))
            .map(|(p, _)| p.clone())
            .collect()
    }
    fn tag_tree(&self) -> Vec<TagNode> {
        fake_tag_tree(&self.tags())
    }
    fn tag_suggestions(&self, prefix: &str, limit: usize) -> Vec<(String, usize)> {
        let q = prefix.trim_start_matches('#').to_lowercase();
        let mut v: Vec<(String, usize)> = self
            .tags()
            .into_iter()
            .filter(|(t, _)| t.contains(&q))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(limit);
        v
    }
    fn files_with_tag(&self, tag: &str) -> Vec<NotePath> {
        let t = tag.trim_start_matches('#').to_lowercase();
        self.notes
            .iter()
            .filter(|(_, n)| {
                n.tags.iter().any(|x| {
                    let x = x.name.to_lowercase();
                    x == t || x.starts_with(&format!("{t}/"))
                })
            })
            .map(|(p, _)| p.clone())
            .collect()
    }
    fn tags(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for n in self.notes.values() {
            for t in &n.tags {
                let name = t.name.to_lowercase();
                let mut acc = String::new();
                for part in name.split('/') {
                    if !acc.is_empty() {
                        acc.push('/');
                    }
                    acc.push_str(part);
                    *out.entry(acc.clone()).or_insert(0) += 1;
                }
            }
        }
        out
    }
    fn graph(&self, options: &GraphOptions) -> GraphData {
        let mut g = GraphData::default();
        for f in &self.files {
            if f.is_markdown() || options.include_attachments {
                g.nodes.push(GraphNode {
                    id: f.as_str().to_string(),
                    label: f.stem().to_string(),
                    kind: if f.is_markdown() {
                        GraphNodeKind::Note
                    } else {
                        GraphNodeKind::Attachment
                    },
                    path: Some(f.clone()),
                });
            }
        }
        let mut edges = Vec::new();
        for (src, note) in &self.notes {
            let Some(s) = g.node_index(src.as_str()) else {
                continue;
            };
            for l in &note.links {
                let t = match self.resolve(&l.target, src) {
                    Some(p) => g.node_index(p.as_str()),
                    None if options.include_unresolved && !l.target.is_empty() => {
                        Some(g.node_index(&l.target).unwrap_or_else(|| {
                            g.nodes.push(GraphNode {
                                id: l.target.clone(),
                                label: l.target.clone(),
                                kind: GraphNodeKind::Unresolved,
                                path: None,
                            });
                            g.nodes.len() - 1
                        }))
                    }
                    None => None,
                };
                if let Some(t) = t
                    && t != s
                {
                    edges.push(GraphEdge {
                        source: s,
                        target: t,
                    });
                }
            }
            if options.include_tags {
                for tag in &note.tags {
                    let id = format!("#{}", tag.name.to_lowercase());
                    let t = g.node_index(&id).unwrap_or_else(|| {
                        g.nodes.push(GraphNode {
                            id: id.clone(),
                            label: id.clone(),
                            kind: GraphNodeKind::Tag,
                            path: None,
                        });
                        g.nodes.len() - 1
                    });
                    edges.push(GraphEdge {
                        source: s,
                        target: t,
                    });
                }
            }
        }
        edges.sort_by_key(|e| (e.source, e.target));
        edges.dedup();
        g.edges = edges;
        if !options.include_orphans {
            let deg = g.degrees();
            g = ors_core_filter(&g, |i| deg[i] > 0);
        }
        g
    }
    fn link_text_for(&self, target: &NotePath, _from: &NotePath) -> String {
        target.stem().to_string()
    }
    fn update_file(&mut self, vault: &dyn VaultRead, path: &NotePath) {
        self.insert_file(path);
        if path.is_markdown()
            && let Ok(src) = vault.read_to_string(path)
        {
            self.update_from_source(path, &src);
        }
    }
    fn update_from_source(&mut self, path: &NotePath, source: &str) {
        self.insert_file(path);
        self.notes.insert(path.clone(), naive_parse(source));
        self.sources.insert(path.clone(), source.to_string());
    }
    fn remove_file(&mut self, path: &NotePath) {
        self.files.retain(|f| f != path);
        self.notes.remove(path);
        self.sources.remove(path);
    }
    fn rename_file(&mut self, from: &NotePath, to: &NotePath) {
        self.files.retain(|f| f != from);
        self.insert_file(to);
        if let Some(n) = self.notes.remove(from) {
            self.notes.insert(to.clone(), n);
        }
        if let Some(s) = self.sources.remove(from) {
            self.sources.insert(to.clone(), s);
        }
    }
    fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(tag) = q.strip_prefix("tag:") {
            let tag = tag.trim_start_matches('#').to_lowercase();
            return Ok(self
                .notes
                .iter()
                .filter(|(_, n)| {
                    n.tags.iter().any(|t| {
                        let name = t.name.to_lowercase();
                        name == tag || name.starts_with(&format!("{tag}/"))
                    })
                })
                .map(|(p, _)| SearchResult {
                    path: p.clone(),
                    score: 1,
                    matches: Vec::new(),
                })
                .collect());
        }
        let needle = q.trim_matches('"').to_lowercase();
        let mut out = Vec::new();
        for (path, src) in &self.sources {
            let mut matches = Vec::new();
            for (line, text) in src.lines().enumerate() {
                let lower = text.to_lowercase();
                // Only keep byte ranges when lowercasing preserved lengths.
                if lower.len() != text.len() {
                    if lower.contains(&needle) {
                        matches.push(SearchMatch {
                            line,
                            line_text: text.to_string(),
                            ranges: Vec::new(),
                        });
                    }
                    continue;
                }
                let ranges: Vec<(usize, usize)> = lower
                    .match_indices(&needle)
                    .map(|(i, m)| (i, i + m.len()))
                    .collect();
                if !ranges.is_empty() {
                    matches.push(SearchMatch {
                        line,
                        line_text: text.to_string(),
                        ranges,
                    });
                }
            }
            if !matches.is_empty() || path.stem().to_lowercase().contains(&needle) {
                out.push(SearchResult {
                    path: path.clone(),
                    score: matches.len() as u32,
                    matches,
                });
            }
        }
        out.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
        Ok(out)
    }
    fn fuzzy_find(&self, query: &str, limit: usize) -> Vec<(NotePath, Option<String>, i64)> {
        let mut out: Vec<(NotePath, Option<String>, i64)> = self
            .files
            .iter()
            .filter_map(|f| fuzzy_score(query, f.as_str()).map(|s| (f.clone(), None, s)))
            .collect();
        out.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        out.truncate(limit);
        out
    }
    fn fuzzy_find_files(
        &self,
        query: &str,
        limit: usize,
        include_attachments: bool,
    ) -> Vec<(NotePath, Option<String>, i64)> {
        let mut v = self.fuzzy_find(query, usize::MAX);
        v.retain(|(p, _, _)| include_attachments || p.is_markdown());
        v.truncate(limit);
        v
    }
    fn rename_with_links(
        &mut self,
        vault: &dyn VaultWrite,
        from: &NotePath,
        to: &NotePath,
    ) -> Result<Vec<NotePath>> {
        vault.rename(from, to)?;
        self.rename_file(from, to);
        Ok(Vec::new())
    }
    fn new_layout(&self, graph: &GraphData) -> Box<dyn LayoutEngine> {
        Box::new(CircleLayout::new(graph))
    }
    fn parse(&self, source: &str) -> ParsedNote {
        naive_parse(source)
    }
}

fn ors_core_filter(g: &GraphData, keep: impl Fn(usize) -> bool) -> GraphData {
    let mut map = vec![None; g.nodes.len()];
    let mut out = GraphData::default();
    for (i, n) in g.nodes.iter().enumerate() {
        if keep(i) {
            map[i] = Some(out.nodes.len());
            out.nodes.push(n.clone());
        }
    }
    for e in &g.edges {
        if let (Some(s), Some(t)) = (map[e.source], map[e.target]) {
            out.edges.push(GraphEdge {
                source: s,
                target: t,
            });
        }
    }
    out
}

/// Static layout on concentric rings (demo only, no physics).
pub struct CircleLayout {
    positions: Vec<Vec2>,
}

impl CircleLayout {
    pub fn new(graph: &GraphData) -> Self {
        let n = graph.nodes.len();
        let positions = (0..n)
            .map(|i| {
                let ring = if i == 0 {
                    0.0
                } else {
                    1.0 + (i % 3) as f32 * 0.45
                };
                let a = i as f32 * 2.399_963; // golden angle
                Vec2::new(a.cos() * 140.0 * ring, a.sin() * 140.0 * ring)
            })
            .collect();
        Self { positions }
    }
}

impl LayoutEngine for CircleLayout {
    fn step(&mut self, _graph: &GraphData, _dt: f32) -> f32 {
        0.0
    }
    fn positions(&self) -> &[Vec2] {
        &self.positions
    }
    fn pin(&mut self, node: usize, position: Option<Vec2>) {
        if let (Some(p), Some(slot)) = (position, self.positions.get_mut(node)) {
            *slot = p;
        }
    }
    fn sync(&mut self, _old: &GraphData, new: &GraphData) {
        *self = Self::new(new);
    }
    fn hit_test(&self, point: Vec2, radius: f32) -> Option<usize> {
        self.positions
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (p.x - point.x).powi(2) + (p.y - point.y).powi(2)))
            .filter(|(_, d)| *d <= radius * radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    fn bounds(&self) -> (Vec2, Vec2) {
        let mut min = Vec2::new(f32::MAX, f32::MAX);
        let mut max = Vec2::new(f32::MIN, f32::MIN);
        for p in &self.positions {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        if self.positions.is_empty() {
            (Vec2::default(), Vec2::default())
        } else {
            (min, max)
        }
    }
}

/// The sample vault (`examples/sample-vault`), embedded for `--demo`.
pub fn demo_vault() -> MemVault {
    let v = MemVault::with_files(
        "sample-vault (demo)",
        [
            (
                "Accueil.md",
                include_str!("../../../examples/sample-vault/Accueil.md"),
            ),
            (
                "Idées.md",
                include_str!("../../../examples/sample-vault/Idées.md"),
            ),
            (
                "Orpheline.md",
                include_str!("../../../examples/sample-vault/Orpheline.md"),
            ),
            (
                "Journal/2026-10-20.md",
                include_str!("../../../examples/sample-vault/Journal/2026-10-20.md"),
            ),
            (
                "Projets/Oxinote.md",
                include_str!("../../../examples/sample-vault/Projets/Oxinote.md"),
            ),
            (
                "Projets/Rust notes.md",
                include_str!("../../../examples/sample-vault/Projets/Rust notes.md"),
            ),
            (
                "Templates/Daily.md",
                include_str!("../../../examples/sample-vault/Templates/Daily.md"),
            ),
        ],
    );
    if let Ok(p) = NotePath::new("attachments/diagramme.png") {
        v.add_binary(&p, demo_png());
    }
    v
}

/// A small generated gradient image (the sample PNG is a placeholder).
fn demo_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(240, 120, |x, y| {
        let t = x as f32 / 240.0;
        let u = y as f32 / 120.0;
        let ring = (((x as f32 - 120.0).powi(2) + (y as f32 - 60.0).powi(2)).sqrt() / 8.0).sin();
        image::Rgba([
            (124.0 + 80.0 * t) as u8,
            (58.0 + 60.0 * u + 20.0 * ring) as u8,
            (237.0 - 60.0 * t) as u8,
            255,
        ])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    match img.write_to(&mut out, image::ImageFormat::Png) {
        Ok(()) => out.into_inner(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naive_parse_finds_links_tags_headings() {
        let n = naive_parse(
            "---\ntags: [a]\n---\n# Title\nSee [[B#H|bee]] and ![[img.png]] #tag `#no`\n",
        );
        assert_eq!(n.headings.len(), 1);
        assert_eq!(n.links.len(), 2);
        assert_eq!(n.links[0].target, "B");
        assert_eq!(n.links[0].display.as_deref(), Some("bee"));
        assert!(n.links[1].is_embed);
        assert_eq!(n.tags.len(), 1);
        assert!(n.frontmatter_span.is_some());
    }

    #[test]
    fn demo_vault_index_has_backlinks() {
        let v = demo_vault();
        let idx = FakeIndex::build(&v);
        let accueil = NotePath::new("Accueil.md").unwrap();
        assert!(!idx.backlinks(&accueil).is_empty());
        assert!(!idx.graph(&GraphOptions::default()).nodes.is_empty());
        assert!(!idx.search("rust").unwrap().is_empty());
    }
}
