use ors_core::{GraphData, GraphNodeKind};

use super::cache_of;
use crate::GraphOptions;

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("a.md", "[[b]] [[b#h]] ![[pic.png]] [[ghost]] [[Ghost]] #t"),
        ("b.md", "[[a]] [[b]]"),
        ("lonely.md", ""),
        ("pic.png", ""),
        ("unused.png", ""),
    ]
}

fn ids(g: &GraphData) -> Vec<&str> {
    g.nodes.iter().map(|n| n.id.as_str()).collect()
}

fn has_edge(g: &GraphData, s: &str, t: &str) -> bool {
    let (Some(s), Some(t)) = (g.node_index(s), g.node_index(t)) else {
        return false;
    };
    g.edges.iter().any(|e| e.source == s && e.target == t)
}

#[test]
fn default_graph_notes_only_no_orphans() {
    let c = cache_of(&files());
    let g = c.graph(&GraphOptions::default());
    assert_eq!(ids(&g), vec!["a.md", "b.md"]);
    // Deduplicated, no self loops.
    assert_eq!(g.edges.len(), 2);
    assert!(has_edge(&g, "a.md", "b.md"));
    assert!(has_edge(&g, "b.md", "a.md"));
}

#[test]
fn graph_with_everything() {
    let c = cache_of(&files());
    let g = c.graph(&GraphOptions {
        include_tags: true,
        include_attachments: true,
        include_unresolved: true,
        include_orphans: true,
    });
    let ids = ids(&g);
    assert!(ids.contains(&"lonely.md"));
    assert!(ids.contains(&"unused.png"));
    assert!(ids.contains(&"#t"));
    // Unresolved deduplicated case-insensitively, first spelling wins.
    assert!(ids.contains(&"ghost"));
    assert!(!ids.contains(&"Ghost"));
    assert!(has_edge(&g, "a.md", "pic.png"));
    assert!(has_edge(&g, "a.md", "ghost"));
    assert!(has_edge(&g, "a.md", "#t"));
    let kinds: Vec<GraphNodeKind> = g.nodes.iter().map(|n| n.kind).collect();
    assert!(kinds.contains(&GraphNodeKind::Unresolved));
    assert!(kinds.contains(&GraphNodeKind::Tag));
    let pic = &g.nodes[g.node_index("pic.png").unwrap()];
    assert_eq!(pic.kind, GraphNodeKind::Attachment);
    assert_eq!(pic.label, "pic.png");
    assert_eq!(g.nodes[g.node_index("a.md").unwrap()].label, "a");
}

#[test]
fn orphan_filter_applies_after_options() {
    let c = cache_of(&files());
    let g = c.graph(&GraphOptions {
        include_attachments: true,
        ..GraphOptions::default()
    });
    let ids = ids(&g);
    assert!(ids.contains(&"pic.png"));
    assert!(!ids.contains(&"unused.png"));
    assert!(!ids.contains(&"lonely.md"));
    // Edge indices are valid after remapping.
    assert!(
        g.edges
            .iter()
            .all(|e| e.source < g.nodes.len() && e.target < g.nodes.len())
    );
}

#[test]
fn embedded_attachment_edges_skipped_without_attachments() {
    let c = cache_of(&[("a.md", "![[pic.png]]"), ("pic.png", "")]);
    let g = c.graph(&GraphOptions {
        include_orphans: true,
        ..GraphOptions::default()
    });
    assert_eq!(ids(&g), vec!["a.md"]);
    assert!(g.edges.is_empty());
}
