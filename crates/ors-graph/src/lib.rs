//! Force-directed layout of the vault link graph (Obsidian's graph view).
//!
//! STUB — public signatures are the contract; the bodies are placeholders.

use ors_core::GraphData;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Mirrors the "Forces" section of Obsidian's graph settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForceParams {
    /// Pull towards the origin.
    pub center_strength: f32,
    /// Node-node repulsion.
    pub repel_strength: f32,
    /// Spring force along edges.
    pub link_strength: f32,
    /// Rest length of edges.
    pub link_distance: f32,
    /// Velocity damping per step, 0..1.
    pub damping: f32,
    /// Barnes-Hut accuracy (0 = exact O(n²)).
    pub theta: f32,
}

impl Default for ForceParams {
    fn default() -> Self {
        Self {
            center_strength: 0.05,
            repel_strength: 400.0,
            link_strength: 0.1,
            link_distance: 60.0,
            damping: 0.85,
            theta: 0.8,
        }
    }
}

/// Incremental simulation; call [`ForceLayout::step`] every frame.
#[derive(Debug, Clone)]
pub struct ForceLayout {
    pub params: ForceParams,
    positions: Vec<Vec2>,
}

impl ForceLayout {
    /// Initial positions are deterministic (seeded), not all at the origin.
    pub fn new(graph: &GraphData, params: ForceParams) -> Self {
        Self {
            params,
            positions: vec![Vec2::default(); graph.nodes.len()],
        }
    }

    /// Advance the simulation. Returns the total kinetic energy (≈0 when settled).
    pub fn step(&mut self, graph: &GraphData, dt: f32) -> f32 {
        let _ = (graph, dt);
        0.0
    }

    /// Run until the energy drops below `epsilon` or `max_steps` is reached.
    pub fn settle(&mut self, graph: &GraphData, max_steps: usize, epsilon: f32) {
        let _ = (graph, max_steps, epsilon);
    }

    pub fn positions(&self) -> &[Vec2] {
        &self.positions
    }

    /// Pin a node (dragged by the user) at a position; `None` releases it.
    pub fn pin(&mut self, node: usize, position: Option<Vec2>) {
        let _ = (node, position);
    }

    /// Keep positions of nodes that still exist (matched by id) after the graph changed.
    pub fn sync(&mut self, old: &GraphData, new: &GraphData) {
        let _ = old;
        self.positions.resize(new.nodes.len(), Vec2::default());
    }

    /// Index of the node closest to `point` within `radius`.
    pub fn hit_test(&self, point: Vec2, radius: f32) -> Option<usize> {
        let _ = (point, radius);
        None
    }

    /// (min, max) corners of the bounding box of all nodes.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        (Vec2::default(), Vec2::default())
    }
}

/// Subgraph of nodes within `depth` hops of `center` (edges in both directions),
/// like Obsidian's local graph. Node indices are remapped.
pub fn local_graph(graph: &GraphData, center: usize, depth: usize) -> GraphData {
    let _ = (graph, center, depth);
    GraphData::default()
}

/// Keep only nodes for which `keep` returns true (and edges between them).
pub fn filter_graph(graph: &GraphData, keep: impl Fn(&ors_core::GraphNode) -> bool) -> GraphData {
    let _ = keep;
    GraphData {
        nodes: graph.nodes.clone(),
        edges: graph.edges.clone(),
    }
}

/// Radius used to draw a node, growing with its degree like Obsidian.
pub fn node_radius(degree: usize) -> f32 {
    4.0 + (degree as f32).sqrt() * 1.5
}
