//! Search operators return line-level matches on `examples/sample-vault`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use oxi_core::{MemoryVault, NotePath, VaultWrite};
use oxi_index::{MetadataCache, SearchIndex, SearchResult};

fn vault_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-vault")
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, Option<String>)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let text = (rel.ends_with(".md")).then(|| std::fs::read_to_string(&path).unwrap());
            out.push((rel, text));
        }
    }
}

/// The sample vault loaded into memory (attachments with empty contents).
fn sample() -> MemoryVault {
    let root = vault_root();
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let v = MemoryVault::new();
    for (path, text) in files {
        v.write(&p(&path), text.as_deref().unwrap_or("")).unwrap();
    }
    v
}

fn p(s: &str) -> NotePath {
    NotePath::new(s).unwrap()
}

fn run(q: &str) -> Vec<SearchResult> {
    let vault = sample();
    let cache = MetadataCache::build(&vault).unwrap();
    let idx = SearchIndex::build(&vault, &cache).unwrap();
    idx.search(q, &cache).unwrap()
}

fn lines_of(res: &[SearchResult], path: &str) -> Vec<(usize, String, Vec<String>)> {
    let r = res.iter().find(|r| r.path.as_str() == path).unwrap();
    r.matches
        .iter()
        .map(|m| {
            let texts = m
                .ranges
                .iter()
                .map(|&(s, e)| m.line_text[s..e].to_string())
                .collect();
            (m.line, m.line_text.clone(), texts)
        })
        .collect()
}

#[test]
fn tag_query_reports_inline_and_frontmatter_lines() {
    let res = run("tag:#projet");
    let obsi = lines_of(&res, "Projets/OxiNote.md");
    // Frontmatter `tags:` line (0-based line 1) and the inline tag on line 10.
    assert!(
        obsi.iter()
            .any(|(l, _, t)| *l == 1 && t == &["projet/oxinote"])
    );
    assert!(obsi.iter().any(|(l, text, t)| *l == 10
        && text.contains("#projet/oxinote")
        && t == &["#projet/oxinote"]));
    // Frontmatter list item.
    let ideas = lines_of(&res, "Idées.md");
    assert_eq!(ideas.len(), 1);
    assert_eq!(ideas[0].0, 3);
    assert_eq!(ideas[0].2, vec!["projet/oxinote"]);
}

#[test]
fn property_query_reports_property_line() {
    let res = run("[status]");
    let ideas = lines_of(&res, "Idées.md");
    assert_eq!(ideas.len(), 1);
    assert_eq!(ideas[0].0, 4);
    assert_eq!(ideas[0].1, "status: brouillon");
    let res = run("[status:brouillon]");
    assert_eq!(lines_of(&res, "Idées.md")[0].0, 4);
    assert!(run("[status:inconnu]").is_empty());
}

#[test]
fn scoped_tag_inside_line_operator() {
    let res = run("line:(tag:#rust)");
    let obsi = lines_of(&res, "Projets/OxiNote.md");
    assert!(obsi.iter().any(|(l, _, t)| *l == 10 && t == &["#rust"]));
}

#[test]
fn path_and_file_operators_match_without_lines() {
    let res = run("file:OxiNote");
    assert!(res.iter().any(|r| r.path.as_str() == "Projets/OxiNote.md"));
    let res = run("path:Projets");
    assert_eq!(res.len(), 2);
}

#[test]
fn line_block_section_task_operators_have_matches() {
    for q in [
        "line:(graphe)",
        "block:(graphe)",
        "section:(graphe)",
        "task:graphe",
    ] {
        let res = run(q);
        let ideas = lines_of(&res, "Idées.md");
        assert!(!ideas.is_empty(), "{q}");
        assert!(ideas.iter().all(|(_, _, t)| !t.is_empty()), "{q}");
    }
}
