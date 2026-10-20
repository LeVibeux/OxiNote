//! Barnes-Hut quadtree used for the many-body repulsion.

use crate::{Vec2, repel};

/// Maximum subdivision depth; points closer than `extent / 2^MAX_DEPTH`
/// (e.g. coincident nodes) end up together in one leaf and are handled
/// pairwise.
const MAX_DEPTH: u32 = 32;
const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
struct Cell {
    /// Center of mass.
    com: Vec2,
    /// Number of bodies below this cell.
    mass: f32,
    /// Geometric center of the square cell.
    center: Vec2,
    /// Half of the side length.
    half: f32,
    /// Child cells (NONE when absent). All NONE for a leaf.
    children: [u32; 4],
    /// Range of `order` holding the bodies of a leaf.
    start: u32,
    end: u32,
}

impl Cell {
    fn is_leaf(&self) -> bool {
        self.children.iter().all(|&c| c == NONE)
    }

    fn contains(&self, p: Vec2) -> bool {
        (p.x - self.center.x).abs() <= self.half && (p.y - self.center.y).abs() <= self.half
    }
}

/// A quadtree over a set of positions, rebuilt every simulation step.
#[derive(Debug)]
pub(crate) struct QuadTree {
    cells: Vec<Cell>,
    order: Vec<u32>,
}

impl QuadTree {
    pub(crate) fn build(positions: &[Vec2]) -> Self {
        let mut tree = Self {
            cells: Vec::with_capacity(positions.len() * 2),
            order: (0..positions.len() as u32).collect(),
        };
        if positions.is_empty() {
            return tree;
        }
        let (mut min, mut max) = (positions[0], positions[0]);
        for p in positions {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        let center = (min + max) * 0.5;
        let half = ((max.x - min.x).max(max.y - min.y) * 0.5).max(1e-3) * 1.0001;
        tree.build_cell(positions, 0, positions.len(), center, half, 0);
        tree
    }

    fn build_cell(
        &mut self,
        positions: &[Vec2],
        start: usize,
        end: usize,
        center: Vec2,
        half: f32,
        depth: u32,
    ) -> u32 {
        let index = self.cells.len() as u32;
        let mut com = Vec2::ZERO;
        for &i in &self.order[start..end] {
            com += positions[i as usize];
        }
        let mass = (end - start) as f32;
        self.cells.push(Cell {
            com: com * (1.0 / mass),
            mass,
            center,
            half,
            children: [NONE; 4],
            start: start as u32,
            end: end as u32,
        });
        if end - start <= 1 || depth >= MAX_DEPTH {
            return index;
        }
        // Partition the range in place into the four quadrants:
        // [left-bottom, left-top, right-bottom, right-top].
        let mid = start + partition(&mut self.order[start..end], |i| positions[i].x < center.x);
        let q1 = start + partition(&mut self.order[start..mid], |i| positions[i].y < center.y);
        let q3 = mid + partition(&mut self.order[mid..end], |i| positions[i].y < center.y);
        let h = half * 0.5;
        let quadrants = [
            (start, q1, Vec2::new(center.x - h, center.y - h)),
            (q1, mid, Vec2::new(center.x - h, center.y + h)),
            (mid, q3, Vec2::new(center.x + h, center.y - h)),
            (q3, end, Vec2::new(center.x + h, center.y + h)),
        ];
        let mut children = [NONE; 4];
        for (k, (s, e, c)) in quadrants.into_iter().enumerate() {
            if e > s {
                children[k] = self.build_cell(positions, s, e, c, h, depth + 1);
            }
        }
        self.cells[index as usize].children = children;
        index
    }

    /// Total repulsion exerted on body `i` by all the others.
    pub(crate) fn repulsion(
        &self,
        i: usize,
        positions: &[Vec2],
        strength: f32,
        theta: f32,
    ) -> Vec2 {
        let mut force = Vec2::ZERO;
        if self.cells.is_empty() {
            return force;
        }
        let pi = positions[i];
        let theta2 = theta * theta;
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        stack.push(0);
        while let Some(c) = stack.pop() {
            let cell = &self.cells[c as usize];
            if cell.is_leaf() {
                for &j in &self.order[cell.start as usize..cell.end as usize] {
                    let j = j as usize;
                    if j != i {
                        force += repel(pi, positions[j], 1.0, strength, i, j);
                    }
                }
                continue;
            }
            let size = cell.half * 2.0;
            let dist2 = (cell.com - pi).length_squared();
            if theta > 0.0 && !cell.contains(pi) && size * size < theta2 * dist2 {
                force += repel(pi, cell.com, cell.mass, strength, i, c as usize);
            } else {
                stack.extend(cell.children.iter().copied().filter(|&ch| ch != NONE));
            }
        }
        force
    }
}

/// Stable-enough in-place partition: moves elements satisfying `pred` to the
/// front and returns how many there are.
fn partition(slice: &mut [u32], pred: impl Fn(usize) -> bool) -> usize {
    let mut k = 0;
    for idx in 0..slice.len() {
        if pred(slice[idx] as usize) {
            slice.swap(k, idx);
            k += 1;
        }
    }
    k
}
