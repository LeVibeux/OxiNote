use serde::{Deserialize, Serialize};

use crate::NotePath;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphNodeKind {
    Note,
    Attachment,
    /// A link target that does not exist in the vault.
    Unresolved,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Stable identifier: the vault path for notes/attachments, the raw target
    /// for unresolved links, `"#tag"` for tags.
    pub id: String,
    /// Display label (note stem, tag name, ...).
    pub label: String,
    pub kind: GraphNodeKind,
    /// Set for notes and attachments.
    pub path: Option<NotePath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Index into [`GraphData::nodes`].
    pub source: usize,
    pub target: usize,
}

/// The link graph of a vault, produced by `ors-index`, laid out by `ors-graph`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    /// Directed (source links to target), deduplicated.
    pub edges: Vec<GraphEdge>,
}

impl GraphData {
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Number of edges touching each node (in + out).
    pub fn degrees(&self) -> Vec<usize> {
        let mut d = vec![0; self.nodes.len()];
        for e in &self.edges {
            d[e.source] += 1;
            d[e.target] += 1;
        }
        d
    }
}
