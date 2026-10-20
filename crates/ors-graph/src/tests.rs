use super::*;
use ors_core::{GraphEdge, NotePath};

fn node(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        label: NotePath::new(id).map_or_else(|_| id.to_string(), |p| p.stem().to_string()),
        kind: GraphNodeKind::Note,
        path: NotePath::new(id).ok(),
    }
}

fn graph(n: usize, edges: &[(usize, usize)]) -> GraphData {
    GraphData {
        nodes: (0..n).map(|i| node(&format!("n{i}.md"))).collect(),
        edges: edges
            .iter()
            .map(|&(source, target)| GraphEdge { source, target })
            .collect(),
    }
}

/// Deterministic pseudo-random graph (xorshift).
fn random_graph(n: usize, m: usize, seed: u64) -> GraphData {
    let mut s = seed.max(1);
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let edges: Vec<(usize, usize)> = (0..m)
        .map(|_| ((next() % n as u64) as usize, (next() % n as u64) as usize))
        .collect();
    graph(n, &edges)
}

fn all_finite(layout: &ForceLayout) -> bool {
    layout.positions().iter().all(|p| p.is_finite())
}

#[test]
fn initial_positions_are_distinct_and_deterministic() {
    let g = graph(50, &[]);
    let a = ForceLayout::new(&g, ForceParams::default());
    let b = ForceLayout::new(&g, ForceParams::default());
    assert_eq!(a.positions(), b.positions());
    for i in 0..50 {
        for j in i + 1..50 {
            assert!(a.positions()[i].distance(a.positions()[j]) > 1.0);
        }
    }
}

#[test]
fn empty_and_single_node() {
    let g = GraphData::default();
    let mut l = ForceLayout::new(&g, ForceParams::default());
    assert_eq!(l.step(&g, 1.0), 0.0);
    l.settle(&g, 10, 0.01);
    assert_eq!(l.bounds(), (Vec2::ZERO, Vec2::ZERO));
    assert_eq!(l.hit_test(Vec2::ZERO, 100.0), None);

    let g = graph(1, &[(0, 0)]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    l.settle(&g, 500, 1e-6);
    assert!(all_finite(&l));
    assert!(
        l.positions()[0].length() < 1.0,
        "single node goes to center"
    );
}

#[test]
fn energy_decreases_and_converges() {
    let g = graph(6, &[(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 5)]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    let early: f32 = (0..10).map(|_| l.step(&g, 1.0)).sum::<f32>() / 10.0;
    for _ in 0..300 {
        l.step(&g, 1.0);
    }
    let late = l.step(&g, 1.0);
    assert!(late < early * 0.01, "early {early}, late {late}");
    let steps = l.settle_steps(&g, 2000, 1e-4);
    assert!(steps < 2000, "did not settle");
    assert!(all_finite(&l));
}

#[test]
fn connected_nodes_end_closer_than_unconnected() {
    // Two triangles joined by nothing, plus a chain.
    let g = graph(6, &[(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    l.settle(&g, 2000, 1e-4);
    let p = l.positions();
    let mut linked = 0.0;
    for &(a, b) in &[(0, 1), (1, 2), (3, 4)] {
        linked += p[a].distance(p[b]);
    }
    let mut unlinked = 0.0;
    for &(a, b) in &[(0, 3), (1, 4), (2, 5)] {
        unlinked += p[a].distance(p[b]);
    }
    assert!(linked < unlinked, "linked {linked} unlinked {unlinked}");
    // Spring rest length is roughly respected.
    let d = p[0].distance(p[1]);
    assert!(d > 20.0 && d < 200.0, "edge length {d}");
}

#[test]
fn determinism() {
    let g = random_graph(200, 400, 7);
    let mut a = ForceLayout::new(&g, ForceParams::default());
    let mut b = ForceLayout::new(&g, ForceParams::default());
    for _ in 0..50 {
        assert_eq!(a.step(&g, 1.0), b.step(&g, 1.0));
    }
    assert_eq!(a.positions(), b.positions());
}

#[test]
fn barnes_hut_close_to_exact() {
    let g = random_graph(300, 300, 3);
    let exact = ForceParams {
        theta: 0.0,
        ..ForceParams::default()
    };
    let approx = ForceParams {
        theta: 0.5,
        ..ForceParams::default()
    };
    let mut a = ForceLayout::new(&g, exact);
    let mut b = ForceLayout::new(&g, approx);
    a.step(&g, 1.0);
    b.step(&g, 1.0);
    let mut err = 0.0;
    let mut norm = 0.0;
    for (va, vb) in a.velocities().iter().zip(b.velocities()) {
        err += (*va - *vb).length();
        norm += va.length();
    }
    assert!(err / norm < 0.05, "relative error {}", err / norm);
}

#[test]
fn theta_zero_matches_brute_force() {
    let g = random_graph(40, 0, 11);
    let params = ForceParams {
        theta: 0.0,
        center_strength: 0.0,
        link_strength: 0.0,
        damping: 1.0,
        ..ForceParams::default()
    };
    let mut l = ForceLayout::new(&g, params);
    let before = l.positions().to_vec();
    l.step(&g, 1.0);
    for i in 0..40 {
        let mut f = Vec2::ZERO;
        for j in 0..40 {
            if i != j {
                f += repel(before[i], before[j], 1.0, params.repel_strength, i, j);
            }
        }
        let v = l.velocities()[i];
        assert!(
            (v - f).length() < 1e-3 * f.length().max(1.0),
            "{v:?} vs {f:?}"
        );
    }
}

#[test]
fn coincident_nodes_self_loops_duplicates_are_robust() {
    let g = graph(5, &[(0, 1), (0, 1), (1, 0), (2, 2), (3, 4), (4, 99)]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    for i in 0..5 {
        l.set_position(i, Vec2::new(3.0, 3.0));
    }
    for _ in 0..200 {
        let e = l.step(&g, 1.0);
        assert!(e.is_finite());
    }
    assert!(all_finite(&l));
    let p = l.positions();
    for i in 0..5 {
        for j in i + 1..5 {
            assert!(p[i].distance(p[j]) > 1.0, "{i} and {j} still overlap");
        }
    }
}

#[test]
fn extreme_params_do_not_produce_nan() {
    let g = random_graph(50, 80, 5);
    let params = ForceParams {
        center_strength: f32::NAN,
        repel_strength: 1e9,
        link_strength: 5.0,
        link_distance: -3.0,
        damping: 7.0,
        theta: f32::INFINITY,
    };
    let mut l = ForceLayout::new(&g, params);
    for _ in 0..100 {
        assert!(l.step(&g, f32::NAN).is_finite());
    }
    assert!(all_finite(&l));
}

#[test]
fn pinned_nodes_do_not_move() {
    let g = graph(3, &[(0, 1), (1, 2)]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    let target = Vec2::new(500.0, -200.0);
    l.pin(1, Some(target));
    assert!(l.is_pinned(1));
    for _ in 0..100 {
        l.step(&g, 1.0);
    }
    assert_eq!(l.positions()[1], target);
    // Neighbors are attracted towards the pinned node.
    assert!(l.positions()[0].distance(target) < 300.0);
    l.pin(1, None);
    assert!(!l.is_pinned(1));
    l.step(&g, 1.0);
    assert_ne!(l.positions()[1], target);
    l.pin(42, Some(Vec2::ZERO)); // ignored
}

#[test]
fn sync_preserves_positions_by_id() {
    let old = graph(3, &[(0, 1), (1, 2)]);
    let mut l = ForceLayout::new(&old, ForceParams::default());
    l.settle(&old, 300, 1e-3);
    l.pin(2, Some(Vec2::new(10.0, 10.0)));
    let before = l.positions().to_vec();
    // New graph: n2 and n0 swapped, n1 removed, new node "x" linked to n0,
    // new isolated node "y".
    let new = GraphData {
        nodes: vec![node("n2.md"), node("x.md"), node("n0.md"), node("y.md")],
        edges: vec![GraphEdge {
            source: 1,
            target: 2,
        }],
    };
    l.sync(&old, &new);
    assert_eq!(l.len(), 4);
    assert_eq!(l.positions()[0], before[2]);
    assert_eq!(l.positions()[2], before[0]);
    assert!(l.is_pinned(0));
    assert!(!l.is_pinned(2));
    // "x" is placed near its neighbor n0.
    let d = l.positions()[1].distance(before[0]);
    assert!(
        d <= ForceParams::default().link_distance,
        "x placed {d} away"
    );
    assert!(all_finite(&l));
    // Stepping with the new graph works.
    l.step(&new, 1.0);
}

#[test]
fn step_resizes_when_graph_changed_without_sync() {
    let g = graph(2, &[]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    let g2 = graph(5, &[(3, 4)]);
    l.step(&g2, 1.0);
    assert_eq!(l.len(), 5);
    assert!(all_finite(&l));
}

#[test]
fn hit_test_and_bounds() {
    let g = graph(3, &[]);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    l.set_position(0, Vec2::new(0.0, 0.0));
    l.set_position(1, Vec2::new(10.0, 0.0));
    l.set_position(2, Vec2::new(-5.0, 20.0));
    assert_eq!(l.hit_test(Vec2::new(9.0, 1.0), 3.0), Some(1));
    assert_eq!(l.hit_test(Vec2::new(4.0, 0.0), 5.0), Some(0));
    assert_eq!(l.hit_test(Vec2::new(100.0, 100.0), 5.0), None);
    assert_eq!(l.bounds(), (Vec2::new(-5.0, 0.0), Vec2::new(10.0, 20.0)));
}

#[test]
fn local_graph_depths() {
    // 0 - 1 - 2 - 3, 4 -> 1 (incoming), 5 isolated
    let g = graph(6, &[(0, 1), (1, 2), (2, 3), (4, 1)]);
    let l0 = local_graph(&g, 1, 0);
    assert_eq!(l0.nodes.len(), 1);
    assert_eq!(l0.nodes[0].id, "n1.md");
    assert!(l0.edges.is_empty());

    let l1 = local_graph(&g, 1, 1);
    let ids: Vec<&str> = l1.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["n1.md", "n0.md", "n2.md", "n4.md"]);
    // Edges remapped: 0->1 becomes 1->0, 1->2 becomes 0->2, 4->1 becomes 3->0.
    let mut edges: Vec<(usize, usize)> = l1.edges.iter().map(|e| (e.source, e.target)).collect();
    edges.sort();
    assert_eq!(edges, [(0, 2), (1, 0), (3, 0)]);

    let l2 = local_graph(&g, 1, 2);
    assert_eq!(l2.nodes.len(), 5);
    assert_eq!(l2.nodes[4].id, "n3.md");
    assert_eq!(l2.edges.len(), 4);

    assert!(local_graph(&g, 99, 3).nodes.is_empty());
    assert_eq!(local_graph(&g, 5, 3).nodes.len(), 1);
}

#[test]
fn filter_graph_remaps() {
    let g = graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]);
    let f = filter_graph(&g, |n| n.id != "n1.md");
    let ids: Vec<&str> = f.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["n0.md", "n2.md", "n3.md"]);
    let edges: Vec<(usize, usize)> = f.edges.iter().map(|e| (e.source, e.target)).collect();
    assert_eq!(edges, [(1, 2), (2, 0), (0, 1)]);
    for e in &f.edges {
        assert!(e.source < f.nodes.len() && e.target < f.nodes.len());
    }
    assert!(filter_graph(&g, |_| false).nodes.is_empty());
}

#[test]
fn search_filter_by_label() {
    let mut g = graph(3, &[(0, 1), (1, 2)]);
    g.nodes[1].label = "Rust Notes".into();
    let f = search_filter(&g, "rust");
    assert_eq!(f.nodes.len(), 1);
    assert_eq!(f.nodes[0].label, "Rust Notes");
    assert_eq!(search_filter(&g, "  ").nodes.len(), 3);
}

#[test]
fn components_and_neighbors() {
    let g = graph(6, &[(0, 1), (2, 1), (3, 4), (5, 5)]);
    assert_eq!(connected_components(&g), [0, 0, 0, 1, 1, 2]);
    assert_eq!(neighbors(&g, 1), [0, 2]);
    assert!(neighbors(&g, 5).is_empty());
}

#[test]
fn color_group_precedence() {
    let mut g = graph(0, &[]);
    g.nodes = vec![
        node("Projets/A.md"),
        node("ProjetsX/B.md"),
        node("Journal/C.md"),
        GraphNode {
            id: "#rust".into(),
            label: "#rust".into(),
            kind: GraphNodeKind::Tag,
            path: None,
        },
    ];
    let groups = [
        GroupQuery::PathPrefix("projets".into()),
        GroupQuery::Kind(GraphNodeKind::Tag),
        GroupQuery::Text("c".into()),
        GroupQuery::Kind(GraphNodeKind::Note),
    ];
    assert_eq!(
        color_groups(&g, &groups),
        [Some(0), Some(3), Some(2), Some(1)]
    );
    assert_eq!(color_groups(&g, &[]), [None, None, None, None]);
}

#[test]
fn node_radius_grows() {
    assert!(node_radius(0) < node_radius(1));
    assert!(node_radius(10) < node_radius(100));
}

/// Debug-friendly performance check: 1k nodes / 2k edges.
#[test]
fn step_is_fast_enough_in_debug() {
    let g = random_graph(1_000, 2_000, 42);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    let start = std::time::Instant::now();
    for _ in 0..5 {
        l.step(&g, 1.0);
    }
    let per_step = start.elapsed() / 5;
    // Generous bound for unoptimized builds on slow CI machines.
    assert!(per_step.as_millis() < 500, "{per_step:?} per step");
    assert!(all_finite(&l));
}

/// Run with `cargo test -p ors-graph --release -- --ignored perf`.
#[test]
#[ignore = "performance test, run in release mode"]
fn perf_5k_nodes_10k_edges() {
    let g = random_graph(5_000, 10_000, 1);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    l.step(&g, 1.0);
    let start = std::time::Instant::now();
    let steps = 20;
    for _ in 0..steps {
        l.step(&g, 1.0);
    }
    let per_step = start.elapsed() / steps;
    eprintln!("5k/10k: {per_step:?} per step");
    assert!(per_step.as_millis() < 50, "{per_step:?} per step");
}

#[test]
fn cools_down_and_reheats() {
    let g = random_graph(300, 600, 9);
    let mut l = ForceLayout::new(&g, ForceParams::default());
    assert_eq!(l.alpha(), 1.0);
    let steps = l.settle_steps(&g, 5_000, 1e-6);
    assert!(
        steps < 5_000,
        "a medium graph with Barnes-Hut must come to rest"
    );
    for _ in 0..1_000 {
        l.step(&g, 1.0);
    }
    assert_eq!(l.alpha(), 0.0);
    let frozen = l.positions().to_vec();
    assert_eq!(l.step(&g, 1.0), 0.0);
    assert_eq!(l.positions(), &frozen[..]);
    l.reheat(1.0);
    assert_eq!(l.alpha(), 1.0);
    l.pin(0, Some(Vec2::new(1e4, 0.0)));
    l.step(&g, 1.0);
    assert_ne!(l.positions(), &frozen[..]);
    l.set_alpha(0.0);
    l.set_alpha_decay(0.0);
    l.reheat(0.5);
    l.step(&g, 1.0);
    assert_eq!(l.alpha(), 0.5, "decay 0 keeps the temperature");
}
