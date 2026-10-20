//! Graph view model, independent of egui: Obsidian-compatible settings
//! (`.obsidian/graph.json`), colour groups, local graph extraction and the
//! per-tab simulation state.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ors_core::{GraphData, GraphNode, GraphNodeKind, NotePath};
use ors_graph::{ForceParams, GroupQuery};
use ors_index::GraphOptions;
use serde::{Deserialize, Serialize};

use crate::backend::{IndexBackend, LayoutEngine};

/// File (relative to the vault's config folder) of the global graph settings.
pub const GRAPH_FILE: &str = "graph.json";
/// File of the local graph settings (Obsidian keeps those per leaf in
/// `workspace.json`; ObsiRust stores the defaults for new local graphs here).
pub const LOCAL_GRAPH_FILE: &str = "local-graph.json";

/// `{"a": 1, "rgb": 14701138}` like Obsidian.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphColor {
    pub a: f64,
    pub rgb: u32,
}

impl Default for GraphColor {
    fn default() -> Self {
        Self {
            a: 1.0,
            rgb: 0x00a8_82ff,
        }
    }
}

impl GraphColor {
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            a: 1.0,
            rgb: (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        }
    }
    pub fn rgb_bytes(&self) -> [u8; 3] {
        [
            ((self.rgb >> 16) & 0xff) as u8,
            ((self.rgb >> 8) & 0xff) as u8,
            (self.rgb & 0xff) as u8,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ColorGroup {
    pub query: String,
    pub color: GraphColor,
}

/// Contents of `.obsidian/graph.json` (same keys as Obsidian). Unknown keys
/// are kept in `extra` and written back unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphSettings {
    #[serde(rename = "collapse-filter")]
    pub collapse_filter: bool,
    pub search: String,
    pub show_tags: bool,
    pub show_attachments: bool,
    pub hide_unresolved: bool,
    pub show_orphans: bool,
    #[serde(rename = "collapse-color-groups")]
    pub collapse_color_groups: bool,
    pub color_groups: Vec<ColorGroup>,
    #[serde(rename = "collapse-display")]
    pub collapse_display: bool,
    pub show_arrow: bool,
    /// -3..=3: lower shows labels from further away.
    pub text_fade_multiplier: f64,
    /// 0.1..=5
    pub node_size_multiplier: f64,
    /// 0.1..=5
    pub line_size_multiplier: f64,
    #[serde(rename = "collapse-forces")]
    pub collapse_forces: bool,
    /// 0..=1
    pub center_strength: f64,
    /// 0..=20
    pub repel_strength: f64,
    /// 0..=1
    pub link_strength: f64,
    /// 30..=500
    pub link_distance: f64,
    pub scale: f64,
    /// Settings panel closed.
    pub close: bool,
    // Local graph only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_jumps: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_backlinks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_forelinks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_interlinks: Option<bool>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for GraphSettings {
    /// Obsidian's defaults.
    fn default() -> Self {
        Self {
            collapse_filter: false,
            search: String::new(),
            show_tags: false,
            show_attachments: false,
            hide_unresolved: false,
            show_orphans: true,
            collapse_color_groups: true,
            color_groups: Vec::new(),
            collapse_display: true,
            show_arrow: false,
            text_fade_multiplier: 0.0,
            node_size_multiplier: 1.0,
            line_size_multiplier: 1.0,
            collapse_forces: true,
            center_strength: 0.518_713_248_970_312,
            repel_strength: 10.0,
            link_strength: 1.0,
            link_distance: 250.0,
            scale: 1.0,
            close: true,
            local_jumps: None,
            local_backlinks: None,
            local_forelinks: None,
            local_interlinks: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl GraphSettings {
    /// Defaults of a local graph.
    pub fn local_default() -> Self {
        Self {
            local_jumps: Some(1),
            local_backlinks: Some(true),
            local_forelinks: Some(true),
            local_interlinks: Some(false),
            show_orphans: true,
            ..Self::default()
        }
    }

    pub fn is_local(&self) -> bool {
        self.local_jumps.is_some()
    }

    pub fn depth(&self) -> usize {
        self.local_jumps.unwrap_or(1).clamp(1, 5)
    }

    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Map Obsidian's slider values to the simulation parameters of
    /// `ors-graph` (whose defaults correspond to Obsidian's defaults).
    pub fn force_params(&self) -> ForceParams {
        let d = ForceParams::default();
        let base = GraphSettings::default();
        let ratio = |v: f64, b: f64| (v / b) as f32;
        ForceParams {
            center_strength: d.center_strength
                * ratio(self.center_strength, base.center_strength).max(0.0),
            repel_strength: d.repel_strength * ratio(self.repel_strength, base.repel_strength),
            link_strength: d.link_strength * ratio(self.link_strength, base.link_strength),
            link_distance: d.link_distance * ratio(self.link_distance, base.link_distance),
            ..d
        }
    }

    /// Key of everything that changes the set of displayed nodes.
    fn filter_key(&self) -> String {
        format!(
            "{}|{}{}{}{}|{:?}{:?}{:?}{:?}",
            self.search,
            self.show_tags,
            self.show_attachments,
            self.hide_unresolved,
            self.show_orphans,
            self.local_jumps,
            self.local_backlinks,
            self.local_forelinks,
            self.local_interlinks
        )
    }

    fn groups_key(&self) -> String {
        self.color_groups
            .iter()
            .map(|g| g.query.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}")
    }
}

/// Reads and writes a graph settings file in `<vault>/.obsidian/`, keeping
/// track of what is on disk to avoid rewriting it every frame.
#[derive(Debug, Clone)]
pub struct GraphStore {
    file: Option<PathBuf>,
    pub settings: GraphSettings,
    saved: GraphSettings,
    /// Time of the first unsaved change (debounce).
    dirty_since: Option<f64>,
}

impl GraphStore {
    /// `root` is the vault folder (`None` for in-memory vaults: no file).
    pub fn load(root: Option<&Path>, file_name: &str, default: GraphSettings) -> Self {
        let file = root.map(|r| r.join(".obsidian").join(file_name));
        let settings = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| {
                let mut s = GraphSettings::from_json(&t);
                if default.is_local() {
                    s.local_jumps = s.local_jumps.or(default.local_jumps);
                    s.local_backlinks = s.local_backlinks.or(default.local_backlinks);
                    s.local_forelinks = s.local_forelinks.or(default.local_forelinks);
                    s.local_interlinks = s.local_interlinks.or(default.local_interlinks);
                }
                s
            })
            .unwrap_or(default);
        Self {
            file,
            saved: settings.clone(),
            settings,
            dirty_since: None,
        }
    }

    pub fn in_memory(settings: GraphSettings) -> Self {
        Self {
            file: None,
            saved: settings.clone(),
            settings,
            dirty_since: None,
        }
    }

    /// Write the file if the settings changed more than `delay` seconds ago
    /// (or immediately with `delay = 0`). Returns an error message on failure.
    pub fn save_if_needed(&mut self, now: f64, delay: f64) -> Result<bool, String> {
        if self.settings == self.saved {
            self.dirty_since = None;
            return Ok(false);
        }
        let since = *self.dirty_since.get_or_insert(now);
        if now - since < delay {
            return Ok(false);
        }
        self.dirty_since = None;
        self.saved = self.settings.clone();
        let Some(file) = &self.file else {
            return Ok(false);
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(file, self.settings.to_json()).map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn has_pending(&self) -> bool {
        self.settings != self.saved
    }
}

// ---------------------------------------------------------------------------
// Graph building
// ---------------------------------------------------------------------------

/// Local graph around `center` (index in `graph`) following links up to
/// `depth` hops: `incoming` follows backlinks, `outgoing` forward links.
/// With `neighbors == false`, only edges touching the center are kept
/// (Obsidian's "Neighbor links" toggle). The center is node 0.
pub fn local_subgraph(
    graph: &GraphData,
    center: usize,
    depth: usize,
    incoming: bool,
    outgoing: bool,
    neighbors: bool,
) -> GraphData {
    let n = graph.nodes.len();
    if center >= n {
        return GraphData::default();
    }
    let mut out_adj = vec![Vec::new(); n];
    let mut in_adj = vec![Vec::new(); n];
    for e in &graph.edges {
        if e.source < n && e.target < n && e.source != e.target {
            out_adj[e.source].push(e.target);
            in_adj[e.target].push(e.source);
        }
    }
    let mut dist = vec![usize::MAX; n];
    dist[center] = 0;
    let mut order = vec![center];
    let mut queue = std::collections::VecDeque::from([center]);
    while let Some(u) = queue.pop_front() {
        if dist[u] >= depth {
            continue;
        }
        let mut next: Vec<usize> = Vec::new();
        if outgoing {
            next.extend(&out_adj[u]);
        }
        if incoming {
            next.extend(&in_adj[u]);
        }
        next.sort_unstable();
        next.dedup();
        for v in next {
            if dist[v] == usize::MAX {
                dist[v] = dist[u] + 1;
                order.push(v);
                queue.push_back(v);
            }
        }
    }
    let mut remap = vec![usize::MAX; n];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    let nodes = order.iter().map(|&i| graph.nodes[i].clone()).collect();
    let mut edges: Vec<ors_core::GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| e.source < n && e.target < n)
        .filter_map(|e| {
            let (s, t) = (remap[e.source], remap[e.target]);
            if s == usize::MAX || t == usize::MAX || s == t {
                return None;
            }
            let touches_center = s == 0 || t == 0;
            // Edge along the BFS tree (one hop further from the center).
            let tree = dist[e.source].abs_diff(dist[e.target]) == 1;
            (touches_center || neighbors || tree).then_some(ors_core::GraphEdge {
                source: s,
                target: t,
            })
        })
        .collect();
    edges.sort_by_key(|e| (e.source, e.target));
    edges.dedup();
    GraphData { nodes, edges }
}

/// Rule used for nodes without a path (tags, unresolved links) and as a
/// fallback when the search query is invalid: `path:x`, `file:x`, `tag:#x`
/// or plain text.
pub fn fallback_group_query(query: &str) -> GroupQuery {
    let q = query.trim();
    if let Some(p) = q.strip_prefix("path:") {
        GroupQuery::PathPrefix(p.trim().trim_matches('"').to_string())
    } else if let Some(f) = q.strip_prefix("file:") {
        GroupQuery::Text(f.trim().trim_matches('"').to_string())
    } else if let Some(t) = q.strip_prefix("tag:") {
        GroupQuery::Text(format!("#{}", t.trim().trim_start_matches('#')))
    } else {
        GroupQuery::Text(q.trim_matches('"').to_string())
    }
}

/// Does a tag node match a `tag:#x` group (`#x` or nested `#x/...`)?
fn tag_node_matches(node: &GraphNode, query: &str) -> Option<bool> {
    let t = query.trim().strip_prefix("tag:")?;
    let want = t.trim().trim_start_matches('#').to_lowercase();
    if node.kind != GraphNodeKind::Tag {
        return None;
    }
    let id = node.id.trim_start_matches('#').to_lowercase();
    Some(id == want || id.starts_with(&format!("{want}/")))
}

/// Group index (first matching group wins) of every node. Notes and
/// attachments are matched with the full search syntax through `search`
/// (which returns the matching files, or `None` if the query is invalid);
/// other nodes with [`fallback_group_query`].
pub fn assign_groups(
    graph: &GraphData,
    groups: &[ColorGroup],
    search: &dyn Fn(&str) -> Option<HashSet<NotePath>>,
) -> Vec<Option<usize>> {
    let matchers: Vec<(Option<HashSet<NotePath>>, GroupQuery, &str)> = groups
        .iter()
        .map(|g| {
            let q = g.query.trim();
            let files = if q.is_empty() { None } else { search(q) };
            (files, fallback_group_query(q), q)
        })
        .collect();
    graph
        .nodes
        .iter()
        .map(|node| {
            matchers.iter().position(|(files, rule, q)| {
                if q.is_empty() {
                    return false;
                }
                if let Some(m) = tag_node_matches(node, q) {
                    return m;
                }
                match (&node.path, files) {
                    (Some(p), Some(set)) if node.kind == GraphNodeKind::Note => set.contains(p),
                    _ => rule.matches(node),
                }
            })
        })
        .collect()
}

/// Keep the files matching the search box (full search syntax through
/// `search`; plain substring filter when the query is invalid), plus tags /
/// unresolved nodes linked to a kept file.
pub fn apply_search_filter(
    graph: &GraphData,
    query: &str,
    search: &dyn Fn(&str) -> Option<HashSet<NotePath>>,
) -> GraphData {
    let q = query.trim();
    if q.is_empty() {
        return graph.clone();
    }
    let Some(files) = search(q) else {
        return ors_graph::search_filter(graph, q);
    };
    let mut keep: Vec<bool> = graph
        .nodes
        .iter()
        .map(|n| n.path.as_ref().is_some_and(|p| files.contains(p)))
        .collect();
    for e in &graph.edges {
        let (s, t) = (e.source, e.target);
        if s >= keep.len() || t >= keep.len() {
            continue;
        }
        if keep[s] && graph.nodes[t].path.is_none() {
            keep[t] = true;
        }
        if keep[t] && graph.nodes[s].path.is_none() {
            keep[s] = true;
        }
    }
    let ids: HashSet<&str> = graph
        .nodes
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(n, _)| n.id.as_str())
        .collect();
    ors_graph::filter_graph(graph, |n| ids.contains(n.id.as_str()))
}

/// Graph view state (a tab, or the local graph pane).
pub struct GraphTab {
    /// Local graph around `center` instead of the whole vault.
    pub local: bool,
    pub center: Option<NotePath>,
    /// Settings shown in the panel; the UI syncs them with the vault's
    /// `graph.json` / `local-graph.json`.
    pub settings: GraphSettings,
    /// Set once `settings` were initialised from the stored ones.
    pub settings_loaded: bool,
    pub data: GraphData,
    pub layout: Option<Box<dyn LayoutEngine>>,
    /// Key of the last build (index generation + options), to rebuild lazily.
    built_key: Option<String>,
    groups_key: Option<(u64, String)>,
    /// Colour group of every node.
    pub groups: Vec<Option<usize>>,
    /// Undirected neighbours of every node (hover highlighting).
    pub adjacency: Vec<Vec<usize>>,
    pub degrees: Vec<usize>,
    pub pan: (f32, f32),
    pub zoom: f32,
    /// Zoom the view is animating towards, with the screen point kept fixed.
    pub zoom_target: Option<(f32, (f32, f32))>,
    pub dragging: Option<usize>,
    pub energy: f32,
    /// The view was fitted to the graph bounds once.
    pub fitted: bool,
    /// "Animate": (start time, nodes in reveal order).
    pub animation: Option<(f64, Vec<usize>)>,
    applied_params: Option<ForceParams>,
    /// Settings as last exchanged with the store (see [`GraphTab::sync_store`]).
    synced: Option<GraphSettings>,
}

impl GraphTab {
    pub fn new(center: Option<NotePath>) -> Self {
        let local = center.is_some();
        Self {
            local,
            center,
            settings: if local {
                GraphSettings::local_default()
            } else {
                GraphSettings::default()
            },
            settings_loaded: false,
            data: GraphData::default(),
            layout: None,
            built_key: None,
            groups_key: None,
            groups: Vec::new(),
            adjacency: Vec::new(),
            degrees: Vec::new(),
            pan: (0.0, 0.0),
            zoom: 1.0,
            zoom_target: None,
            dragging: None,
            energy: 1.0,
            fitted: false,
            animation: None,
            applied_params: None,
            synced: None,
        }
    }

    /// Keep the tab's settings and the persisted ones (`graph.json`) in
    /// step: local edits are pushed to the store, changes made elsewhere
    /// (another graph view) are pulled.
    pub fn sync_store(&mut self, store: &mut GraphStore) {
        match &self.synced {
            None => self.settings = store.settings.clone(),
            Some(s) if *s != self.settings => store.settings = self.settings.clone(),
            Some(s) if *s != store.settings => self.settings = store.settings.clone(),
            Some(_) => return,
        }
        self.settings_loaded = true;
        self.synced = Some(self.settings.clone());
    }

    fn key(&self, generation: u64) -> String {
        format!(
            "{generation}|{}|{:?}|{}",
            self.local,
            self.center,
            self.settings.filter_key()
        )
    }

    /// Rebuild the displayed graph if the index or the options changed.
    /// Returns true when rebuilt.
    pub fn refresh(&mut self, index: &dyn IndexBackend, generation: u64) -> bool {
        let rebuilt = self.rebuild(index, generation);
        let gkey = (generation, self.settings.groups_key());
        if rebuilt || self.groups_key.as_ref() != Some(&gkey) {
            let search = |q: &str| search_files(index, q);
            self.groups = assign_groups(&self.data, &self.settings.color_groups, &search);
            self.groups_key = Some(gkey);
        }
        let params = self.settings.force_params();
        if let Some(layout) = self.layout.as_mut()
            && self.applied_params != Some(params)
        {
            layout.set_params(params);
            if self.applied_params.is_some() {
                layout.reheat(0.5);
                self.energy = 1.0;
            }
            self.applied_params = Some(params);
        }
        rebuilt
    }

    fn rebuild(&mut self, index: &dyn IndexBackend, generation: u64) -> bool {
        let key = self.key(generation);
        if self.built_key.as_deref() == Some(key.as_str()) {
            return false;
        }
        let s = &self.settings;
        let opts = GraphOptions {
            include_tags: s.show_tags,
            include_attachments: s.show_attachments,
            include_unresolved: !s.hide_unresolved,
            include_orphans: s.show_orphans || self.local,
        };
        let mut g = index.graph(&opts);
        if self.local
            && let Some(c) = &self.center
        {
            g = match g.node_index(c.as_str()) {
                Some(i) => local_subgraph(
                    &g,
                    i,
                    s.depth(),
                    s.local_backlinks.unwrap_or(true),
                    s.local_forelinks.unwrap_or(true),
                    s.local_interlinks.unwrap_or(false),
                ),
                None => GraphData::default(),
            };
        }
        let search = |q: &str| search_files(index, q);
        g = apply_search_filter(&g, &s.search, &search);
        match &mut self.layout {
            Some(layout) => {
                layout.sync(&self.data, &g);
                layout.reheat(0.3);
            }
            None => {
                let mut layout = index.new_layout(&g);
                layout.set_params(self.settings.force_params());
                self.applied_params = Some(self.settings.force_params());
                self.layout = Some(layout);
            }
        }
        let n = g.nodes.len();
        let mut adjacency = vec![Vec::new(); n];
        for e in &g.edges {
            if e.source < n && e.target < n && e.source != e.target {
                adjacency[e.source].push(e.target);
                adjacency[e.target].push(e.source);
            }
        }
        for a in &mut adjacency {
            a.sort_unstable();
            a.dedup();
        }
        self.degrees = g.degrees();
        self.adjacency = adjacency;
        self.data = g;
        self.built_key = Some(key);
        self.energy = 1.0;
        true
    }

    /// Restart the layout from scratch (used by "Animate").
    pub fn reset_layout(&mut self, index: &dyn IndexBackend) {
        let mut layout = index.new_layout(&self.data);
        let params = self.settings.force_params();
        layout.set_params(params);
        self.applied_params = Some(params);
        self.layout = Some(layout);
        self.energy = 1.0;
        self.fitted = false;
    }
}

/// Files matching an Obsidian search query (`None` if the query is invalid).
pub fn search_files(index: &dyn IndexBackend, query: &str) -> Option<HashSet<NotePath>> {
    index
        .search(query)
        .ok()
        .map(|r| r.into_iter().map(|r| r.path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ors_core::GraphEdge;

    fn node(id: &str, kind: GraphNodeKind) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.rsplit('/').next().unwrap_or(id).trim_end_matches(".md").into(),
            kind,
            path: match kind {
                GraphNodeKind::Note | GraphNodeKind::Attachment => Some(NotePath::new(id).unwrap()),
                _ => None,
            },
        }
    }

    fn graph() -> GraphData {
        // 0:A -> 1:B -> 2:C, 3:D -> 0:A, 1 -> #tag, 2 -> img
        GraphData {
            nodes: vec![
                node("A.md", GraphNodeKind::Note),
                node("dir/B.md", GraphNodeKind::Note),
                node("dir/C.md", GraphNodeKind::Note),
                node("D.md", GraphNodeKind::Note),
                node("#tag/sub", GraphNodeKind::Tag),
                node("img.png", GraphNodeKind::Attachment),
                node("Ghost", GraphNodeKind::Unresolved),
            ],
            edges: vec![
                GraphEdge {
                    source: 0,
                    target: 1,
                },
                GraphEdge {
                    source: 1,
                    target: 2,
                },
                GraphEdge {
                    source: 3,
                    target: 0,
                },
                GraphEdge {
                    source: 1,
                    target: 4,
                },
                GraphEdge {
                    source: 2,
                    target: 5,
                },
                GraphEdge {
                    source: 3,
                    target: 1,
                },
            ],
        }
    }

    #[test]
    fn graph_json_roundtrip_with_obsidian_keys() {
        let src = r#"{
  "collapse-filter": true,
  "search": "path:Projets",
  "showTags": true,
  "showAttachments": false,
  "hideUnresolved": true,
  "showOrphans": false,
  "collapse-color-groups": false,
  "colorGroups": [
    { "query": "tag:#rust", "color": { "a": 1, "rgb": 14701138 } }
  ],
  "collapse-display": true,
  "showArrow": true,
  "textFadeMultiplier": -1.5,
  "nodeSizeMultiplier": 1.2,
  "lineSizeMultiplier": 2,
  "collapse-forces": false,
  "centerStrength": 0.3,
  "repelStrength": 12,
  "linkStrength": 0.8,
  "linkDistance": 200,
  "scale": 0.75,
  "close": false,
  "someFuturePluginKey": {"x": [1, 2]}
}"#;
        let s = GraphSettings::from_json(src);
        assert_eq!(s.search, "path:Projets");
        assert!(s.show_tags && s.hide_unresolved && !s.show_orphans && s.show_arrow);
        assert_eq!(s.color_groups.len(), 1);
        assert_eq!(s.color_groups[0].query, "tag:#rust");
        assert_eq!(s.color_groups[0].color.rgb, 14701138);
        assert_eq!(s.color_groups[0].color.rgb_bytes(), [0xe0, 0x52, 0x52]);
        assert_eq!(s.text_fade_multiplier, -1.5);
        assert_eq!(s.link_distance, 200.0);
        assert!(!s.close);
        assert!(!s.is_local());
        let out = s.to_json();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        for k in [
            "collapse-filter",
            "search",
            "showTags",
            "showAttachments",
            "hideUnresolved",
            "showOrphans",
            "collapse-color-groups",
            "colorGroups",
            "collapse-display",
            "showArrow",
            "textFadeMultiplier",
            "nodeSizeMultiplier",
            "lineSizeMultiplier",
            "collapse-forces",
            "centerStrength",
            "repelStrength",
            "linkStrength",
            "linkDistance",
            "scale",
            "close",
        ] {
            assert!(v.get(k).is_some(), "missing {k} in {out}");
        }
        // Unknown keys survive, local-only keys are not written.
        assert_eq!(v["someFuturePluginKey"]["x"][1], 2);
        assert!(v.get("localJumps").is_none());
        assert_eq!(v["colorGroups"][0]["color"]["rgb"], 14701138);
        assert_eq!(GraphSettings::from_json(&out), s);
        // Garbage gives defaults.
        assert_eq!(GraphSettings::from_json("nope"), GraphSettings::default());
    }

    #[test]
    fn store_saves_debounced_and_preserves_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("ors-graph-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        std::fs::write(
            dir.join(".obsidian/graph.json"),
            r#"{"showTags": true, "custom": 42}"#,
        )
        .unwrap();
        let mut st = GraphStore::load(Some(&dir), GRAPH_FILE, GraphSettings::default());
        assert!(st.settings.show_tags);
        assert_eq!(st.save_if_needed(0.0, 1.0), Ok(false));
        st.settings.show_arrow = true;
        assert_eq!(st.save_if_needed(10.0, 1.0), Ok(false));
        assert!(st.has_pending());
        assert_eq!(st.save_if_needed(11.5, 1.0), Ok(true));
        let text = std::fs::read_to_string(dir.join(".obsidian/graph.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["custom"], 42);
        assert_eq!(v["showArrow"], true);
        // Local store keeps its local keys.
        let mut local = GraphStore::load(
            Some(&dir),
            LOCAL_GRAPH_FILE,
            GraphSettings::local_default(),
        );
        assert_eq!(local.settings.depth(), 1);
        local.settings.local_jumps = Some(3);
        assert_eq!(local.save_if_needed(0.0, 0.0), Ok(true));
        let again = GraphStore::load(
            Some(&dir),
            LOCAL_GRAPH_FILE,
            GraphSettings::local_default(),
        );
        assert_eq!(again.settings.depth(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn force_params_follow_sliders() {
        let s = GraphSettings::default();
        let p = s.force_params();
        let d = ForceParams::default();
        assert!((p.repel_strength - d.repel_strength).abs() < 1e-3);
        assert!((p.link_distance - d.link_distance).abs() < 1e-3);
        let s2 = GraphSettings {
            repel_strength: 20.0,
            link_distance: 500.0,
            ..s
        };
        assert!(s2.force_params().repel_strength > p.repel_strength * 1.9);
        assert!(s2.force_params().link_distance > p.link_distance * 1.9);
    }

    #[test]
    fn local_subgraph_directions() {
        let g = graph();
        // From B: outgoing C and #tag, incoming A and D.
        let both = local_subgraph(&g, 1, 1, true, true, true);
        let ids: Vec<&str> = both.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids[0], "dir/B.md");
        assert_eq!(ids.len(), 5, "{ids:?}");
        // Neighbour link D -> A is kept only with "neighbor links".
        let has_da = |gd: &GraphData| {
            let d = gd.node_index("D.md").unwrap();
            let a = gd.node_index("A.md").unwrap();
            gd.edges.iter().any(|e| e.source == d && e.target == a)
        };
        assert!(has_da(&both));
        let no_neigh = local_subgraph(&g, 1, 1, true, true, false);
        assert!(!has_da(&no_neigh));
        let out = local_subgraph(&g, 1, 1, false, true, true);
        let ids: Vec<&str> = out.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["dir/B.md", "dir/C.md", "#tag/sub"]);
        let inc = local_subgraph(&g, 1, 1, true, false, true);
        let ids: Vec<&str> = inc.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["dir/B.md", "A.md", "D.md"]);
        // Depth 2 outgoing reaches the attachment.
        let deep = local_subgraph(&g, 1, 2, false, true, true);
        assert!(deep.node_index("img.png").is_some());
        assert!(local_subgraph(&g, 99, 1, true, true, true).nodes.is_empty());
    }

    #[test]
    fn color_groups_assignment() {
        let g = graph();
        let groups = vec![
            ColorGroup {
                query: "path:dir".into(),
                color: GraphColor::from_rgb(255, 0, 0),
            },
            ColorGroup {
                query: "tag:#tag".into(),
                color: GraphColor::from_rgb(0, 255, 0),
            },
            ColorGroup {
                query: "file:A".into(),
                color: GraphColor::from_rgb(0, 0, 255),
            },
            ColorGroup {
                query: String::new(),
                color: GraphColor::default(),
            },
        ];
        // A fake search engine: path:dir matches B and C, file:A matches A
        // and D (pretend), tag:#tag matches B (which has the tag).
        let search = |q: &str| -> Option<HashSet<NotePath>> {
            let names: &[&str] = match q {
                "path:dir" => &["dir/B.md", "dir/C.md"],
                "tag:#tag" => &["dir/B.md"],
                "file:A" => &["A.md"],
                _ => return None,
            };
            Some(names.iter().map(|n| NotePath::new(*n).unwrap()).collect())
        };
        let a = assign_groups(&g, &groups, &search);
        assert_eq!(a[0], Some(2)); // A: file:A
        assert_eq!(a[1], Some(0)); // B: first group wins over tag group
        assert_eq!(a[2], Some(0)); // C
        assert_eq!(a[3], None); // D
        assert_eq!(a[4], Some(1)); // #tag/sub node matches tag:#tag
        assert_eq!(a[5], None); // img.png: not in dir
        assert_eq!(a[6], None);
        // Invalid query: falls back to the simple rules.
        let none = |_: &str| None;
        let b = assign_groups(&g, &groups, &none);
        assert_eq!(b[1], Some(0));
        assert_eq!(b[0], Some(2));
        assert_eq!(
            fallback_group_query("path:Projets"),
            GroupQuery::PathPrefix("Projets".into())
        );
    }

    #[test]
    fn search_filter_keeps_linked_tags() {
        let g = graph();
        let search = |q: &str| -> Option<HashSet<NotePath>> {
            (q == "path:dir").then(|| {
                ["dir/B.md", "dir/C.md"]
                    .iter()
                    .map(|n| NotePath::new(*n).unwrap())
                    .collect()
            })
        };
        let f = apply_search_filter(&g, "path:dir", &search);
        let ids: Vec<&str> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"dir/B.md") && ids.contains(&"#tag/sub"));
        assert!(!ids.contains(&"A.md") && !ids.contains(&"img.png"));
        // Invalid query: substring filter.
        let f = apply_search_filter(&g, "ghost", &search);
        assert_eq!(f.nodes.len(), 1);
        assert_eq!(apply_search_filter(&g, " ", &search).nodes.len(), 7);
    }

    #[test]
    fn tabs_sync_settings_through_the_store() {
        let mut store = GraphStore::in_memory(GraphSettings::default());
        store.settings.show_tags = true;
        let mut a = GraphTab::new(None);
        let mut b = GraphTab::new(None);
        a.sync_store(&mut store);
        b.sync_store(&mut store);
        assert!(a.settings.show_tags && b.settings.show_tags);
        a.settings.repel_strength = 3.0;
        a.sync_store(&mut store);
        assert_eq!(store.settings.repel_strength, 3.0);
        b.sync_store(&mut store);
        assert_eq!(b.settings.repel_strength, 3.0);
    }
}
