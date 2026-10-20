//! Synthetic-vault scale tests (kept fast enough for debug builds).

use std::time::Instant;

use ors_core::{Link, LinkKind, NotePath, ParsedNote, Span};

use super::{p, tag, wiki};
use crate::{GraphOptions, MetadataCache, SearchIndex, fuzzy_find};

fn synthetic(i: usize, n: usize) -> (NotePath, ParsedNote) {
    let path = p(&format!("folder{}/note {i}.md", i % 37));
    let mut links: Vec<Link> = (1..=5)
        .map(|k| {
            let mut l = wiki(&format!("note {}", (i * 7 + k * 13) % n));
            l.span = Span::new(k * 20, k * 20 + 12);
            l.line = k;
            l
        })
        .collect();
    links.push(Link {
        kind: LinkKind::Markdown,
        ..wiki(&format!("missing {}", i % 50))
    });
    let note = ParsedNote {
        links,
        tags: vec![tag(&format!("topic/{}", i % 20)), tag("all")],
        aliases: vec![format!("alias{i}")],
        ..ParsedNote::default()
    };
    (path, note)
}

#[test]
fn two_thousand_notes_incremental() {
    let n = 2000;
    let start = Instant::now();
    let mut c = MetadataCache::new();
    for i in 0..n {
        let (path, note) = synthetic(i, n);
        c.insert_parsed(path, note);
    }
    let elapsed = start.elapsed();
    assert_eq!(c.notes().len(), n);
    let s = c.stats();
    assert_eq!(s.links, n * 6);
    assert_eq!(s.unresolved_links, n);
    assert_eq!(c.tags()["all"], n);
    assert_eq!(c.tags()["topic"], n);
    assert_eq!(c.files_with_tag("topic/3").len(), n / 20);
    // Every note is linked by the arithmetic above.
    let total_backlinks: usize = c.notes().iter().map(|p| c.backlinks(p).len()).sum();
    let self_links = (0..n)
        .flat_map(|i| (1..=5).map(move |k| (i, (i * 7 + k * 13) % n)))
        .filter(|(i, t)| i == t)
        .count();
    assert_eq!(total_backlinks + self_links, n * 5);
    let g = c.graph(&GraphOptions::default());
    assert_eq!(g.nodes.len(), n);
    // Generous bound: debug builds on slow CI.
    assert!(elapsed.as_secs() < 20, "took {elapsed:?}");
    let r = fuzzy_find(&c, "note 1999", 5);
    assert_eq!(r[0].0.stem(), "note 1999");
}

#[test]
fn ten_thousand_notes_bulk() {
    let n = 10_000;
    let start = Instant::now();
    let mut c = MetadataCache::new();
    c.extend_parsed((0..n).map(|i| synthetic(i, n)));
    let elapsed = start.elapsed();
    assert_eq!(c.notes().len(), n);
    assert_eq!(c.stats().resolved_links, n * 5);
    assert!(elapsed.as_secs() < 30, "took {elapsed:?}");
    // Incremental update stays consistent at scale.
    let before = c.backlinks(&p("folder0/note 0.md")).len();
    assert!(before > 0);
    c.remove_file(&p("folder0/note 0.md"));
    assert_eq!(c.stats().unresolved_links, (n - 1) + before);
    assert_eq!(
        count_links_to(&c, "note 0"),
        c.unresolved()
            .values()
            .filter(|v| v.contains(&"note 0".to_string()))
            .count()
    );
}

fn count_links_to(c: &MetadataCache, target: &str) -> usize {
    c.unresolved()
        .values()
        .flatten()
        .filter(|t| t.as_str() == target)
        .count()
}

#[test]
fn search_scales() {
    let mut idx = SearchIndex::new();
    for i in 0..2000 {
        let body = format!(
            "# Note {i}\nSome text about topic{} and rust {i}.\n- [ ] task {i}\n",
            i % 10
        );
        idx.update(&p(&format!("n{i}.md")), &body);
    }
    let c = MetadataCache::new();
    let start = Instant::now();
    let r = idx.search("topic3 rust", &c).unwrap();
    assert_eq!(r.len(), 200);
    let r = idx.search("\"rust 1999\"", &c).unwrap();
    assert_eq!(r.len(), 1);
    let r = idx.search("task-todo:1234", &c).unwrap();
    assert_eq!(r.len(), 1);
    assert!(start.elapsed().as_secs() < 10);
}
