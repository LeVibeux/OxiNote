//! Force-directed layout of the vault link graph (Obsidian's graph view).
//!
//! The simulation is a damped particle system with four forces, mirroring the
//! "Forces" section of Obsidian's graph settings:
//!
//! - **center**: every node is pulled towards the origin;
//! - **repel**: every pair of nodes repels (computed with a Barnes-Hut
//!   quadtree, see [`ForceParams::theta`]);
//! - **link**: edges behave like springs with a rest length;
//! - **damping**: velocities are multiplied by a factor below one every step.
//!
//! Everything is deterministic: initial positions follow a phyllotaxis spiral
//! and coincident nodes are separated with a jitter derived from their indices,
//! so the same graph always produces the same layout.
//!
//! The module also provides graph utilities used by the graph view:
//! [`local_graph`], [`filter_graph`], [`search_filter`], [`neighbors`],
//! [`connected_components`] and [`color_groups`].

#![warn(missing_docs)]

use std::collections::{HashMap, VecDeque};
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use ors_core::{GraphData, GraphNode, GraphNodeKind};

mod quadtree;

use quadtree::QuadTree;

/// A 2D point or vector in layout coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

impl Vec2 {
    /// The origin.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// Build a vector from its coordinates.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Squared euclidean length.
    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// Euclidean length.
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Distance to another point.
    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
    }

    /// True when both coordinates are finite (not NaN or infinite).
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Neg for Vec2 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
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
    ///
    /// A quadtree cell is approximated by its center of mass when
    /// `cell size / distance < theta`. Graphs of up to 128 nodes are always
    /// computed exactly.
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

/// Golden angle in radians, used by the phyllotaxis initial placement.
const GOLDEN_ANGLE: f32 = 2.399_963_3;
/// Spacing of the phyllotaxis spiral.
const INITIAL_RADIUS: f32 = 10.0;
/// Squared distance below which two nodes are considered coincident.
const COINCIDENT_EPSILON2: f32 = 1e-6;
/// Repulsion is clamped below this squared distance to avoid huge forces.
const MIN_REPEL_DISTANCE2: f32 = 1.0;
/// Graphs up to this size use exact repulsion regardless of `theta`.
const EXACT_REPULSION_MAX_NODES: usize = 128;
/// Below this temperature the simulation stops applying forces.
const ALPHA_MIN: f32 = 1e-3;
/// Per-step relative decay of the temperature: reaches [`ALPHA_MIN`] after
/// about 600 steps from 1.0.
const DEFAULT_ALPHA_DECAY: f32 = 0.0115;
/// Temperature restored when a node is moved or pinned.
const DRAG_ALPHA: f32 = 0.3;
/// Temperature restored when the graph changed.
const SYNC_ALPHA: f32 = 0.5;
/// Hard cap on node speed (layout units per unit of time), prevents blow-ups.
const MAX_SPEED: f32 = 1_000.0;

/// Incremental simulation; call [`ForceLayout::step`] every frame.
///
/// Node `i` of the layout corresponds to `graph.nodes[i]`. When the graph
/// changes, call [`ForceLayout::sync`] to carry positions over by node id.
#[derive(Debug, Clone)]
pub struct ForceLayout {
    /// Forces used by the next steps; may be changed at any time.
    pub params: ForceParams,
    positions: Vec<Vec2>,
    velocities: Vec<Vec2>,
    pinned: Vec<bool>,
    /// "Temperature" of the simulation, scales every force (see [`ForceLayout::alpha`]).
    alpha: f32,
    alpha_decay: f32,
    // Scratch buffers reused between steps.
    forces: Vec<Vec2>,
    springs: Vec<(u32, u32)>,
    degrees: Vec<u32>,
}

impl ForceLayout {
    /// Initial positions are deterministic (seeded), not all at the origin.
    pub fn new(graph: &GraphData, params: ForceParams) -> Self {
        let n = graph.nodes.len();
        Self {
            params,
            positions: (0..n).map(initial_position).collect(),
            velocities: vec![Vec2::ZERO; n],
            pinned: vec![false; n],
            alpha: 1.0,
            alpha_decay: DEFAULT_ALPHA_DECAY,
            forces: Vec::new(),
            springs: Vec::new(),
            degrees: Vec::new(),
        }
    }

    /// Number of nodes in the layout.
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// True when the layout has no nodes.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Advance the simulation. Returns the total kinetic energy (≈0 when settled).
    ///
    /// Forces are scaled by the current [`ForceLayout::alpha`], which decays
    /// at every step: after roughly 600 steps the layout is cold and the
    /// energy quickly drops to zero. Use [`ForceLayout::reheat`] to restart it.
    ///
    /// `dt` is the time step (1.0 is a good default; values are clamped to
    /// `0..=2`). If the graph has a different number of nodes than the layout
    /// (the caller forgot [`ForceLayout::sync`]), positions are resized first.
    pub fn step(&mut self, graph: &GraphData, dt: f32) -> f32 {
        let n = graph.nodes.len();
        if n != self.positions.len() {
            self.resize(n);
        }
        if n == 0 {
            return 0.0;
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 2.0)
        } else {
            1.0
        };
        let p = sanitize_params(self.params);
        let alpha = self.alpha;
        self.alpha *= 1.0 - self.alpha_decay;
        if self.alpha < ALPHA_MIN {
            self.alpha = 0.0;
        }

        self.forces.clear();
        self.forces.resize(n, Vec2::ZERO);

        // Center gravity.
        if alpha > 0.0 && p.center_strength != 0.0 {
            for (f, pos) in self.forces.iter_mut().zip(&self.positions) {
                *f -= *pos * p.center_strength;
            }
        }

        // Many-body repulsion.
        if alpha > 0.0 && p.repel_strength != 0.0 && n > 1 {
            // Small graphs are computed exactly: it is cheap, and the
            // approximation noise would keep tiny graphs from settling.
            let theta = if n <= EXACT_REPULSION_MAX_NODES {
                0.0
            } else {
                p.theta
            };
            let tree = QuadTree::build(&self.positions);
            let mut net = Vec2::ZERO;
            for i in 0..n {
                let f = tree.repulsion(i, &self.positions, p.repel_strength, theta);
                self.forces[i] += f;
                net += f;
            }
            // Exact pairwise forces sum to zero; remove the residual net force
            // introduced by the approximation so the graph does not drift.
            if theta > 0.0 {
                let correction = net * (1.0 / n as f32);
                for f in &mut self.forces {
                    *f -= correction;
                }
            }
        }

        // Springs.
        self.prepare_springs(graph);
        if alpha > 0.0 && p.link_strength != 0.0 {
            for &(s, t) in &self.springs {
                let (s, t) = (s as usize, t as usize);
                let mut d = self.positions[t] - self.positions[s];
                let mut dist2 = d.length_squared();
                if dist2 < COINCIDENT_EPSILON2 {
                    d = jitter(s, t) * 1e-2;
                    dist2 = d.length_squared();
                }
                let dist = dist2.sqrt();
                let magnitude = p.link_strength * (dist - p.link_distance);
                let f = d * (magnitude / dist);
                self.forces[s] += f;
                self.forces[t] -= f;
            }
        }

        if alpha == 0.0 {
            // Cold: only let the remaining velocity die out.
            self.forces.iter_mut().for_each(|f| *f = Vec2::ZERO);
        }

        // Integrate. Nodes weigh their degree (at least 1) so that hubs
        // with many springs stay stable and move less than leaves; all forces
        // are pairwise antisymmetric, so momentum is conserved and damping
        // makes the system settle.
        let mut energy = 0.0f32;
        for i in 0..n {
            if self.pinned[i] {
                self.velocities[i] = Vec2::ZERO;
                continue;
            }
            let mass = self.degrees[i].max(1) as f32;
            let mut v = (self.velocities[i] + self.forces[i] * (alpha * dt / mass)) * p.damping;
            if !v.is_finite() {
                v = Vec2::ZERO;
            }
            let speed2 = v.length_squared();
            if speed2 > MAX_SPEED * MAX_SPEED {
                v = v * (MAX_SPEED / speed2.sqrt());
            }
            let new_pos = self.positions[i] + v * dt;
            if new_pos.is_finite() {
                self.positions[i] = new_pos;
            }
            self.velocities[i] = v;
            energy += 0.5 * mass * v.length_squared();
        }
        energy
    }

    /// Current temperature in `0..=1`. Every force is multiplied by it; it
    /// decays a little at each step (like d3-force's `alpha`), so the layout
    /// always comes to rest. `0` means the simulation is cold.
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Raise the temperature to at least `alpha` (clamped to `0..=1`) so the
    /// layout moves again, e.g. `1.0` after the graph changed or `0.3` while
    /// the user drags a node.
    pub fn reheat(&mut self, alpha: f32) {
        if alpha.is_finite() {
            self.alpha = self.alpha.max(alpha.clamp(0.0, 1.0));
        }
    }

    /// Set the temperature to exactly `alpha` (clamped to `0..=1`).
    pub fn set_alpha(&mut self, alpha: f32) {
        if alpha.is_finite() {
            self.alpha = alpha.clamp(0.0, 1.0);
        }
    }

    /// Change how fast the temperature decays per step (`0..1`, default
    /// `0.0115`, ≈600 steps to cool down). `0` keeps the simulation hot forever.
    pub fn set_alpha_decay(&mut self, decay: f32) {
        if decay.is_finite() {
            self.alpha_decay = decay.clamp(0.0, 0.999);
        }
    }

    /// Run until the energy drops below `epsilon` or `max_steps` is reached.
    pub fn settle(&mut self, graph: &GraphData, max_steps: usize, epsilon: f32) {
        self.settle_steps(graph, max_steps, epsilon);
    }

    /// Like [`ForceLayout::settle`], returning the number of steps performed.
    pub fn settle_steps(&mut self, graph: &GraphData, max_steps: usize, epsilon: f32) -> usize {
        for i in 0..max_steps {
            if self.step(graph, 1.0) < epsilon {
                return i + 1;
            }
        }
        max_steps
    }

    /// Current node positions, indexed like `graph.nodes`.
    pub fn positions(&self) -> &[Vec2] {
        &self.positions
    }

    /// Current node velocities, indexed like `graph.nodes`.
    pub fn velocities(&self) -> &[Vec2] {
        &self.velocities
    }

    /// Move a node without pinning it (its velocity is reset and the
    /// simulation is reheated a little).
    /// Out-of-range indices and non-finite positions are ignored.
    pub fn set_position(&mut self, node: usize, position: Vec2) {
        if node < self.positions.len() && position.is_finite() {
            self.positions[node] = position;
            self.velocities[node] = Vec2::ZERO;
            self.reheat(DRAG_ALPHA);
        }
    }

    /// Pin a node (dragged by the user) at a position; `None` releases it.
    ///
    /// Pinned nodes still exert forces on the others but never move. Pinning
    /// reheats the simulation a little so neighbors follow the dragged node.
    /// Out-of-range indices are ignored.
    pub fn pin(&mut self, node: usize, position: Option<Vec2>) {
        if node >= self.positions.len() {
            return;
        }
        match position {
            Some(p) => {
                if p.is_finite() {
                    self.positions[node] = p;
                }
                self.velocities[node] = Vec2::ZERO;
                self.pinned[node] = true;
                self.reheat(DRAG_ALPHA);
            }
            None => self.pinned[node] = false,
        }
    }

    /// Whether `node` is currently pinned.
    pub fn is_pinned(&self, node: usize) -> bool {
        self.pinned.get(node).copied().unwrap_or(false)
    }

    /// Keep positions of nodes that still exist (matched by id) after the graph changed.
    ///
    /// Velocities and pins are carried over too, and the simulation is
    /// reheated if the graph actually changed. New nodes are placed near the
    /// centroid of their already-placed neighbors (with a small deterministic
    /// offset), or on the initial spiral when they have no placed neighbor.
    pub fn sync(&mut self, old: &GraphData, new: &GraphData) {
        let old_index: HashMap<&str, usize> = old
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < self.positions.len())
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let n = new.nodes.len();
        let mut positions = vec![Vec2::ZERO; n];
        let mut velocities = vec![Vec2::ZERO; n];
        let mut pinned = vec![false; n];
        let mut placed = vec![false; n];
        for (i, node) in new.nodes.iter().enumerate() {
            if let Some(&j) = old_index.get(node.id.as_str()) {
                positions[i] = self.positions[j];
                velocities[i] = self.velocities[j];
                pinned[i] = self.pinned[j];
                placed[i] = true;
            }
        }
        let adjacency = adjacency(new);
        let mut fresh = 0usize;
        for i in 0..n {
            if placed[i] {
                continue;
            }
            let mut sum = Vec2::ZERO;
            let mut count = 0usize;
            for &j in &adjacency[i] {
                if placed[j] {
                    sum += positions[j];
                    count += 1;
                }
            }
            positions[i] = if count > 0 {
                let offset = jitter(i, hash_str(&new.nodes[i].id) as usize)
                    * (self.params.link_distance.abs().max(1.0) * 0.5);
                sum * (1.0 / count as f32) + offset
            } else {
                // Spiral positions after the ones already used, so new
                // isolated nodes do not land on top of existing ones.
                let p = initial_position(self.positions.len() + fresh);
                fresh += 1;
                p
            };
            placed[i] = true;
        }
        self.positions = positions;
        self.velocities = velocities;
        self.pinned = pinned;
        let changed = old.nodes.len() != new.nodes.len()
            || old.edges != new.edges
            || old.nodes.iter().zip(&new.nodes).any(|(a, b)| a.id != b.id);
        if changed {
            self.reheat(SYNC_ALPHA);
        }
    }

    /// Index of the node closest to `point` within `radius`.
    pub fn hit_test(&self, point: Vec2, radius: f32) -> Option<usize> {
        let r2 = radius * radius;
        let mut best: Option<(usize, f32)> = None;
        for (i, p) in self.positions.iter().enumerate() {
            let d2 = (*p - point).length_squared();
            if d2 <= r2 && best.is_none_or(|(_, b)| d2 < b) {
                best = Some((i, d2));
            }
        }
        best.map(|(i, _)| i)
    }

    /// (min, max) corners of the bounding box of all nodes.
    ///
    /// Both corners are the origin for an empty layout.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let mut iter = self.positions.iter();
        let Some(first) = iter.next() else {
            return (Vec2::ZERO, Vec2::ZERO);
        };
        let (mut min, mut max) = (*first, *first);
        for p in iter {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        (min, max)
    }

    fn resize(&mut self, n: usize) {
        let old = self.positions.len();
        self.positions.truncate(n);
        self.positions.extend((old..n).map(initial_position));
        self.velocities.resize(n, Vec2::ZERO);
        self.pinned.resize(n, false);
    }

    /// Deduplicated undirected springs (no self-loops, no out-of-range
    /// indices) and the degrees they induce.
    fn prepare_springs(&mut self, graph: &GraphData) {
        let n = self.positions.len();
        self.springs.clear();
        self.springs.extend(graph.edges.iter().filter_map(|e| {
            let (s, t) = (e.source.min(e.target), e.source.max(e.target));
            (s != t && t < n).then_some((s as u32, t as u32))
        }));
        self.springs.sort_unstable();
        self.springs.dedup();
        self.degrees.clear();
        self.degrees.resize(n, 0);
        for &(s, t) in &self.springs {
            self.degrees[s as usize] += 1;
            self.degrees[t as usize] += 1;
        }
    }
}

fn sanitize_params(p: ForceParams) -> ForceParams {
    let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
    let d = ForceParams::default();
    ForceParams {
        center_strength: finite(p.center_strength, d.center_strength),
        repel_strength: finite(p.repel_strength, d.repel_strength),
        link_strength: finite(p.link_strength, d.link_strength),
        link_distance: finite(p.link_distance, d.link_distance).max(0.0),
        damping: finite(p.damping, d.damping).clamp(0.0, 1.0),
        theta: finite(p.theta, d.theta).max(0.0),
    }
}

/// Deterministic initial position of node `i` on a phyllotaxis spiral.
fn initial_position(i: usize) -> Vec2 {
    let r = INITIAL_RADIUS * (0.5 + i as f32).sqrt();
    let a = i as f32 * GOLDEN_ANGLE;
    Vec2::new(r * a.cos(), r * a.sin())
}

/// Deterministic unit vector derived from a pair of indices. Antisymmetric:
/// `jitter(a, b) == -jitter(b, a)`, so coincident nodes are pushed apart.
fn jitter(a: usize, b: usize) -> Vec2 {
    let (lo, hi, sign) = if a <= b { (a, b, 1.0) } else { (b, a, -1.0) };
    let mut h = (lo as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (hi as u64).rotate_left(29);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    let angle = (h % 1_000_000) as f32 / 1_000_000.0 * std::f32::consts::TAU;
    Vec2::new(angle.cos(), angle.sin()) * sign
}

fn hash_str(s: &str) -> u64 {
    // FNV-1a: stable across runs and platforms.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Pairwise repulsion exerted on a node at `pi` by a body of `mass` at `pj`.
fn repel(pi: Vec2, pj: Vec2, mass: f32, strength: f32, i: usize, j: usize) -> Vec2 {
    let mut d = pi - pj;
    let mut dist2 = d.length_squared();
    if dist2 < COINCIDENT_EPSILON2 {
        d = jitter(i, j) * 1e-3;
        dist2 = d.length_squared();
    }
    // Force magnitude ∝ strength / distance (d3-force many-body), clamped
    // near zero distance.
    d * (strength * mass / dist2.max(MIN_REPEL_DISTANCE2))
}

/// Undirected adjacency lists (deduplicated, no self-loops, invalid edges skipped).
fn adjacency(graph: &GraphData) -> Vec<Vec<usize>> {
    let n = graph.nodes.len();
    let mut adj = vec![Vec::new(); n];
    for e in &graph.edges {
        if e.source < n && e.target < n && e.source != e.target {
            adj[e.source].push(e.target);
            adj[e.target].push(e.source);
        }
    }
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }
    adj
}

/// Indices of the nodes directly linked to `node` (either direction),
/// sorted, without `node` itself. Used to highlight neighbors on hover.
pub fn neighbors(graph: &GraphData, node: usize) -> Vec<usize> {
    let mut out: Vec<usize> = graph
        .edges
        .iter()
        .filter_map(|e| {
            if e.source == node && e.target != node {
                Some(e.target)
            } else if e.target == node && e.source != node {
                Some(e.source)
            } else {
                None
            }
        })
        .filter(|&i| i < graph.nodes.len())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Subgraph of nodes within `depth` hops of `center` (edges in both directions),
/// like Obsidian's local graph. Node indices are remapped.
///
/// The center is node 0 of the result; the other nodes follow in BFS order
/// (by distance, then by index). All edges between kept nodes are kept.
/// An out-of-range `center` gives an empty graph.
pub fn local_graph(graph: &GraphData, center: usize, depth: usize) -> GraphData {
    let n = graph.nodes.len();
    if center >= n {
        return GraphData::default();
    }
    let adj = adjacency(graph);
    let mut dist = vec![usize::MAX; n];
    let mut order = vec![center];
    dist[center] = 0;
    let mut queue = VecDeque::from([center]);
    while let Some(u) = queue.pop_front() {
        if dist[u] >= depth {
            continue;
        }
        for &v in &adj[u] {
            if dist[v] == usize::MAX {
                dist[v] = dist[u] + 1;
                order.push(v);
                queue.push_back(v);
            }
        }
    }
    subgraph(graph, &order)
}

/// Build the subgraph made of `order` (old indices, in new order).
fn subgraph(graph: &GraphData, order: &[usize]) -> GraphData {
    let mut remap = vec![usize::MAX; graph.nodes.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    let nodes = order.iter().map(|&i| graph.nodes[i].clone()).collect();
    let edges = graph
        .edges
        .iter()
        .filter(|e| e.source < remap.len() && e.target < remap.len())
        .filter_map(|e| {
            let (s, t) = (remap[e.source], remap[e.target]);
            (s != usize::MAX && t != usize::MAX).then_some(ors_core::GraphEdge {
                source: s,
                target: t,
            })
        })
        .collect();
    GraphData { nodes, edges }
}

/// Keep only nodes for which `keep` returns true (and edges between them).
///
/// Node order is preserved and edge indices are remapped.
pub fn filter_graph(graph: &GraphData, keep: impl Fn(&ors_core::GraphNode) -> bool) -> GraphData {
    let order: Vec<usize> = (0..graph.nodes.len())
        .filter(|&i| keep(&graph.nodes[i]))
        .collect();
    subgraph(graph, &order)
}

/// Keep nodes whose label, id or path contains `query` (case-insensitive),
/// like the search box of Obsidian's graph filters. An empty (or blank)
/// query keeps everything.
pub fn search_filter(graph: &GraphData, query: &str) -> GraphData {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return graph.clone();
    }
    filter_graph(graph, |n| node_matches_text(n, &q))
}

fn node_matches_text(node: &GraphNode, lowercase_query: &str) -> bool {
    node.label.to_lowercase().contains(lowercase_query)
        || node.id.to_lowercase().contains(lowercase_query)
        || node
            .path
            .as_ref()
            .is_some_and(|p| p.as_str().to_lowercase().contains(lowercase_query))
}

/// Connected component index of every node (edges taken as undirected).
///
/// Components are numbered from 0 in order of their lowest node index.
pub fn connected_components(graph: &GraphData) -> Vec<usize> {
    let n = graph.nodes.len();
    let adj = adjacency(graph);
    let mut comp = vec![usize::MAX; n];
    let mut next = 0;
    for start in 0..n {
        if comp[start] != usize::MAX {
            continue;
        }
        comp[start] = next;
        let mut stack = vec![start];
        while let Some(u) = stack.pop() {
            for &v in &adj[u] {
                if comp[v] == usize::MAX {
                    comp[v] = next;
                    stack.push(v);
                }
            }
        }
        next += 1;
    }
    comp
}

/// A rule of a color group in the graph view ("Groups" settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupQuery {
    /// `path:` — node path (or id) is inside this folder, or equals it
    /// (case-insensitive). `"Projets"` matches `"Projets/A.md"` but not
    /// `"ProjetsX/A.md"`.
    PathPrefix(String),
    /// Label/id/path contains this text (case-insensitive).
    Text(String),
    /// Node is of this kind.
    Kind(GraphNodeKind),
}

impl GroupQuery {
    /// Whether `node` matches this rule.
    pub fn matches(&self, node: &GraphNode) -> bool {
        match self {
            GroupQuery::PathPrefix(prefix) => {
                let prefix = prefix.trim_matches('/').to_lowercase();
                if prefix.is_empty() {
                    return true;
                }
                let target = node
                    .path
                    .as_ref()
                    .map_or_else(|| node.id.to_lowercase(), |p| p.as_str().to_lowercase());
                target == prefix
                    || target
                        .strip_prefix(&prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
            }
            GroupQuery::Text(text) => {
                let q = text.trim().to_lowercase();
                q.is_empty() || node_matches_text(node, &q)
            }
            GroupQuery::Kind(kind) => node.kind == *kind,
        }
    }
}

/// Group index of every node: the first query (in order) that matches it,
/// or `None`. Like Obsidian, earlier groups take precedence.
pub fn color_groups(graph: &GraphData, groups: &[GroupQuery]) -> Vec<Option<usize>> {
    graph
        .nodes
        .iter()
        .map(|n| groups.iter().position(|g| g.matches(n)))
        .collect()
}

/// Radius used to draw a node, growing with its degree like Obsidian.
pub fn node_radius(degree: usize) -> f32 {
    4.0 + (degree as f32).sqrt() * 1.5
}

#[cfg(test)]
mod tests;
