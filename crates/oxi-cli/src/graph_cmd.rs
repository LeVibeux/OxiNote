//! `oxinote graph`: JSON / DOT serialization of the link graph.

use std::fmt::Write as _;

use anyhow::Result;
use oxi_core::{GraphData, GraphNodeKind, NotePath};
use oxi_graph::{ForceLayout, ForceParams, Vec2};
use serde::Serialize;

/// Run the force-directed layout until it settles.
pub fn layout(graph: &GraphData) -> Vec<Vec2> {
    let mut layout = ForceLayout::new(graph, ForceParams::default());
    layout.settle(graph, 3_000, 1e-3);
    layout.positions().to_vec()
}

fn kind_name(kind: GraphNodeKind) -> &'static str {
    match kind {
        GraphNodeKind::Note => "note",
        GraphNodeKind::Attachment => "attachment",
        GraphNodeKind::Unresolved => "unresolved",
        GraphNodeKind::Tag => "tag",
    }
}

#[derive(Serialize)]
struct NodeJson<'a> {
    id: &'a str,
    label: &'a str,
    kind: &'static str,
    path: Option<&'a NotePath>,
    degree: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
}

#[derive(Serialize)]
struct EdgeJson {
    source: usize,
    target: usize,
}

#[derive(Serialize)]
struct GraphJson<'a> {
    nodes: Vec<NodeJson<'a>>,
    edges: Vec<EdgeJson>,
}

fn round2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

/// `{"nodes": [{id, label, kind, path, degree, x?, y?}], "edges": [{source, target}]}`.
pub fn to_json(graph: &GraphData, positions: Option<&[Vec2]>) -> Result<String> {
    let degrees = graph.degrees();
    let nodes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let pos = positions.and_then(|p| p.get(i));
            NodeJson {
                id: &n.id,
                label: &n.label,
                kind: kind_name(n.kind),
                path: n.path.as_ref(),
                degree: degrees[i],
                x: pos.map(|p| round2(p.x)),
                y: pos.map(|p| round2(p.y)),
            }
        })
        .collect();
    let edges = graph
        .edges
        .iter()
        .map(|e| EdgeJson {
            source: e.source,
            target: e.target,
        })
        .collect();
    Ok(serde_json::to_string_pretty(&GraphJson { nodes, edges })?)
}

fn dot_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Graphviz DOT. With positions, nodes get a fixed `pos` (use `neato -n`).
pub fn to_dot(graph: &GraphData, positions: Option<&[Vec2]>) -> String {
    let mut out =
        String::from("digraph vault {\n  node [shape=ellipse, fontname=\"sans-serif\"];\n");
    for (i, n) in graph.nodes.iter().enumerate() {
        let style = match n.kind {
            GraphNodeKind::Note => "",
            GraphNodeKind::Attachment => ", shape=note",
            GraphNodeKind::Unresolved => ", style=dashed",
            GraphNodeKind::Tag => ", shape=plaintext",
        };
        let pos = positions
            .and_then(|p| p.get(i))
            .map(|p| format!(", pos=\"{},{}!\"", round2(p.x), round2(-p.y)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  n{i} [label=\"{}\", tooltip=\"{}\"{style}{pos}];",
            dot_escape(&n.label),
            dot_escape(&n.id)
        );
    }
    for e in &graph.edges {
        let _ = writeln!(out, "  n{} -> n{};", e.source, e.target);
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxi_core::{GraphEdge, GraphNode};

    fn sample() -> GraphData {
        GraphData {
            nodes: vec![
                GraphNode {
                    id: "Accueil.md".into(),
                    label: "Accueil".into(),
                    kind: GraphNodeKind::Note,
                    path: NotePath::new("Accueil.md").ok(),
                },
                GraphNode {
                    id: "Note \"fantôme\"".into(),
                    label: "Note \"fantôme\"".into(),
                    kind: GraphNodeKind::Unresolved,
                    path: None,
                },
            ],
            edges: vec![GraphEdge {
                source: 0,
                target: 1,
            }],
        }
    }

    #[test]
    fn json_output() {
        let g = sample();
        let v: serde_json::Value = serde_json::from_str(&to_json(&g, None).unwrap()).unwrap();
        assert_eq!(v["nodes"][0]["id"], "Accueil.md");
        assert_eq!(v["nodes"][0]["kind"], "note");
        assert_eq!(v["nodes"][0]["degree"], 1);
        assert!(v["nodes"][0].get("x").is_none());
        assert!(v["nodes"][1]["path"].is_null());
        assert_eq!(v["edges"][0]["target"], 1);

        let pos = layout(&g);
        let v: serde_json::Value = serde_json::from_str(&to_json(&g, Some(&pos)).unwrap()).unwrap();
        assert!(v["nodes"][1]["x"].is_number());
        let dx = v["nodes"][0]["x"].as_f64().unwrap() - v["nodes"][1]["x"].as_f64().unwrap();
        let dy = v["nodes"][0]["y"].as_f64().unwrap() - v["nodes"][1]["y"].as_f64().unwrap();
        assert!((dx * dx + dy * dy).sqrt() > 1.0);
    }

    #[test]
    fn dot_output() {
        let dot = to_dot(&sample(), None);
        assert!(dot.starts_with("digraph vault {"));
        assert!(dot.contains("n0 [label=\"Accueil\", tooltip=\"Accueil.md\"];"));
        assert!(dot.contains("label=\"Note \\\"fantôme\\\"\""));
        assert!(dot.contains("style=dashed"));
        assert!(dot.contains("  n0 -> n1;"));
        assert!(dot.ends_with('}'));
        let dot = to_dot(
            &sample(),
            Some(&[Vec2::new(1.0, 2.0), Vec2::new(3.5, -4.0)]),
        );
        assert!(dot.contains("pos=\"1,-2!\""), "{dot}");
        assert!(dot.contains("pos=\"3.5,4!\""));
    }

    #[test]
    fn empty_graph() {
        let g = GraphData::default();
        assert!(layout(&g).is_empty());
        let v: serde_json::Value = serde_json::from_str(&to_json(&g, None).unwrap()).unwrap();
        assert_eq!(v["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(to_dot(&g, None).lines().count(), 3);
    }
}
