//! Integration tests on `examples/sample-vault` with the real markdown parser.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use oxi_core::{GraphNodeKind, MemoryVault, NotePath, VaultRead, VaultWrite};
use oxi_index::{GraphOptions, MetadataCache, SearchIndex, fuzzy_find, rename_with_links};

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

fn strs(v: &[NotePath]) -> Vec<&str> {
    v.iter().map(NotePath::as_str).collect()
}

fn sources(cache: &MetadataCache, path: &str) -> Vec<String> {
    cache
        .backlink_sources(&p(path))
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

// ---------------------------------------------------------------- raw text only

#[test]
fn lists_all_files() {
    let vault = sample();
    let cache = MetadataCache::build(&vault).unwrap();
    assert_eq!(
        strs(&cache.files()),
        vec![
            "Accueil.md",
            "Idées.md",
            "Journal/2026-10-20.md",
            "Orpheline.md",
            "Projets/Oxinote.md",
            "Projets/Rust notes.md",
            "Templates/Daily.md",
            "attachments/diagramme.png",
        ]
    );
    assert_eq!(
        strs(&cache.attachments()),
        vec!["attachments/diagramme.png"]
    );
}

#[test]
fn raw_text_search_on_sample() {
    let vault = sample();
    let cache = MetadataCache::build(&vault).unwrap();
    let idx = SearchIndex::build(&vault, &cache).unwrap();
    let paths = |q: &str| -> Vec<String> {
        let mut v: Vec<String> = idx
            .search(q, &cache)
            .unwrap()
            .into_iter()
            .map(|r| r.path.as_str().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(paths("task-todo:graphe"), vec!["Idées.md"]);
    assert_eq!(paths("task-done:surlignages"), vec!["Idées.md"]);
    assert_eq!(
        paths("section:(ownership propriétaire)"),
        vec!["Projets/Rust notes.md"]
    );
    assert!(paths("section:(ownership références)").is_empty());
    assert_eq!(paths("path:Projets crates"), vec!["Projets/Oxinote.md"]);
    assert_eq!(
        paths("line:(mentionne oxinote)"),
        vec!["Projets/Oxinote.md"]
    );
    assert_eq!(
        paths("\"note fantôme\" OR orpheline"),
        vec!["Accueil.md", "Orpheline.md"]
    );
    assert_eq!(paths("file:2026-10-20"), vec!["Journal/2026-10-20.md"]);
    assert!(idx.search("\"unterminated", &cache).is_err());
}

#[test]
fn quick_switcher_on_sample() {
    let vault = sample();
    let cache = MetadataCache::build(&vault).unwrap();
    let r = fuzzy_find(&cache, "rustn", 3);
    assert_eq!(r[0].0.as_str(), "Projets/Rust notes.md");
}

// ------------------------------------------------------------ needs the parser

#[test]
fn backlinks_of_accueil() {
    let cache = MetadataCache::build(&sample()).unwrap();
    // Idées: frontmatter `related: "[[Accueil]]"` and the callout link.
    assert_eq!(
        sources(&cache, "Accueil.md"),
        vec!["Idées.md", "Projets/Oxinote.md"]
    );
    let from_ideas = cache
        .backlinks(&p("Accueil.md"))
        .into_iter()
        .filter(|b| b.source.as_str() == "Idées.md")
        .count();
    assert_eq!(from_ideas, 2);
}

#[test]
fn resolution_on_sample() {
    let cache = MetadataCache::build(&sample()).unwrap();
    let targets = |from: &str| -> Vec<Option<String>> {
        cache
            .outgoing(&p(from))
            .into_iter()
            .map(|r| r.target.map(|t| t.as_str().to_string()))
            .collect()
    };
    let accueil = targets("Accueil.md");
    assert_eq!(
        accueil,
        vec![
            Some("Projets/Oxinote.md".into()),
            Some("Idées.md".into()),
            Some("Journal/2026-10-20.md".into()),
            None,
            Some("Projets/Rust notes.md".into()),
        ]
    );
    // [[Oxinote]] from the same folder, self block link.
    assert_eq!(
        targets("Projets/Rust notes.md"),
        vec![
            Some("Projets/Oxinote.md".into()),
            Some("Projets/Rust notes.md".into())
        ]
    );
    assert_eq!(
        targets("Journal/2026-10-20.md"),
        vec![
            Some("Projets/Oxinote.md".into()),
            Some("Projets/Rust notes.md".into())
        ]
    );
    assert_eq!(
        sources(&cache, "attachments/diagramme.png"),
        vec!["Idées.md"]
    );
    assert_eq!(
        sources(&cache, "Projets/Oxinote.md"),
        vec![
            "Accueil.md",
            "Idées.md",
            "Journal/2026-10-20.md",
            "Projets/Rust notes.md"
        ]
    );
    assert_eq!(
        sources(&cache, "Idées.md"),
        vec!["Accueil.md", "Projets/Oxinote.md"]
    );
}

#[test]
fn unresolved_on_sample() {
    let cache = MetadataCache::build(&sample()).unwrap();
    let u = cache.unresolved();
    assert_eq!(u.len(), 2, "{u:?}");
    assert_eq!(u[&p("Accueil.md")], vec!["Note fantôme"]);
    // Links in comments and code are not links.
    assert_eq!(u[&p("Idées.md")], vec!["Canvas"]);
}

#[test]
fn tags_on_sample() {
    let cache = MetadataCache::build(&sample()).unwrap();
    let tags = cache.tags();
    let keys: Vec<&str> = tags.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "hub",
            "idée",
            "journal",
            "meta",
            "meta/organisation",
            "projet",
            "projet/oxinote",
            "rust"
        ]
    );
    // Frontmatter + inline.
    assert_eq!(tags["hub"], 2);
    assert_eq!(tags["projet/oxinote"], 3);
    assert_eq!(tags["projet"], 3);
    assert_eq!(tags["rust"], 2);
    assert_eq!(
        strs(&cache.files_with_tag("#projet")),
        vec!["Idées.md", "Projets/Oxinote.md"]
    );
    assert_eq!(strs(&cache.files_with_tag("META")), vec!["Accueil.md"]);
    assert_eq!(
        strs(&cache.files_with_tag("rust")),
        vec!["Projets/Oxinote.md", "Projets/Rust notes.md"]
    );
}

#[test]
fn orphans_mentions_and_graph_on_sample() {
    let vault = sample();
    let cache = MetadataCache::build(&vault).unwrap();
    assert_eq!(
        strs(&cache.orphans()),
        vec!["Orpheline.md", "Templates/Daily.md"]
    );
    assert_eq!(
        strs(&cache.unlinked_mentions(&vault, &p("Projets/Oxinote.md"))),
        vec!["Accueil.md", "Orpheline.md"]
    );
    // Alias "Home" / "Index" of Accueil are not mentioned anywhere.
    assert!(cache.unlinked_mentions(&vault, &p("Accueil.md")).is_empty());

    let g = cache.graph(&GraphOptions::default());
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "Accueil.md",
            "Idées.md",
            "Journal/2026-10-20.md",
            "Projets/Oxinote.md",
            "Projets/Rust notes.md"
        ]
    );
    let full = cache.graph(&GraphOptions {
        include_tags: true,
        include_attachments: true,
        include_unresolved: true,
        include_orphans: true,
    });
    for id in [
        "Note fantôme",
        "Canvas",
        "#hub",
        "#projet/oxinote",
        "attachments/diagramme.png",
        "Orpheline.md",
    ] {
        assert!(full.node_index(id).is_some(), "missing node {id}");
    }
    let unresolved = full
        .nodes
        .iter()
        .filter(|n| n.kind == GraphNodeKind::Unresolved)
        .count();
    assert_eq!(unresolved, 2);
}

#[test]
fn tag_search_on_sample() {
    let vault = sample();
    let cache = MetadataCache::build(&vault).unwrap();
    let idx = SearchIndex::build(&vault, &cache).unwrap();
    let mut r: Vec<String> = idx
        .search("tag:#projet \"rust\"", &cache)
        .unwrap()
        .into_iter()
        .map(|r| r.path.as_str().to_string())
        .collect();
    r.sort();
    // Idées matches through its ```rust code fence.
    assert_eq!(r, vec!["Idées.md", "Projets/Oxinote.md"]);
    let r = idx.search("[status:brouillon]", &cache).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].path.as_str(), "Idées.md");
    let r = idx.search("[aliases:home]", &cache).unwrap();
    assert_eq!(r[0].path.as_str(), "Accueil.md");
}

fn read(vault: &MemoryVault, path: &str) -> String {
    vault.read_to_string(&p(path)).unwrap()
}

#[test]
fn rename_note_updates_links() {
    let vault = sample();
    let mut cache = MetadataCache::build(&vault).unwrap();
    let changed = rename_with_links(
        &vault,
        &mut cache,
        &p("Idées.md"),
        &p("Archive/Brainstorm.md"),
    )
    .unwrap();
    assert_eq!(
        strs(&changed),
        vec!["Accueil.md", "Archive/Brainstorm.md", "Projets/Oxinote.md"]
    );
    assert!(read(&vault, "Accueil.md").contains("et [[Brainstorm]]."));
    assert!(read(&vault, "Projets/Oxinote.md").contains("[[Brainstorm#Fonctionnalités]]"));
    let moved = read(&vault, "Archive/Brainstorm.md");
    assert!(moved.contains("![[Brainstorm#^graphe]]"));
    // Untouched links in the moved note.
    assert!(moved.contains("![[Projets/Oxinote#Architecture]]"));
    assert!(moved.contains("[[Accueil#Accueil|liens]]"));
    assert_eq!(
        sources(&cache, "Archive/Brainstorm.md"),
        vec!["Accueil.md", "Projets/Oxinote.md"]
    );
    let u = cache.unresolved();
    assert_eq!(u.len(), 2);
    assert_eq!(u[&p("Archive/Brainstorm.md")], vec!["Canvas"]);
}

#[test]
fn rename_with_spaces_updates_markdown_and_wiki_links() {
    let vault = sample();
    let mut cache = MetadataCache::build(&vault).unwrap();
    let changed = rename_with_links(
        &vault,
        &mut cache,
        &p("Projets/Rust notes.md"),
        &p("Projets/Rust book.md"),
    )
    .unwrap();
    assert_eq!(
        strs(&changed),
        vec![
            "Accueil.md",
            "Journal/2026-10-20.md",
            "Projets/Oxinote.md",
            "Projets/Rust book.md"
        ]
    );
    assert!(read(&vault, "Accueil.md").contains("[Rust](Projets/Rust%20book.md#Ownership)"));
    assert!(read(&vault, "Journal/2026-10-20.md").contains("[[Rust book#Borrowing|borrowing]]"));
    assert!(read(&vault, "Projets/Oxinote.md").contains("Voir [[Rust book]] et"));
    assert!(read(&vault, "Projets/Rust book.md").contains("[[Rust book#^borrow]]"));
    assert_eq!(cache.unresolved().len(), 2);
}

#[test]
fn move_note_keeps_style() {
    let vault = sample();
    let mut cache = MetadataCache::build(&vault).unwrap();
    let changed = rename_with_links(
        &vault,
        &mut cache,
        &p("Projets/Oxinote.md"),
        &p("Archive/Oxinote.md"),
    )
    .unwrap();
    // [[Oxinote]] in "Rust notes" is a unique basename: unchanged.
    assert_eq!(
        strs(&changed),
        vec!["Accueil.md", "Idées.md", "Journal/2026-10-20.md"]
    );
    assert!(read(&vault, "Accueil.md").contains("[[Archive/Oxinote|le projet]]"));
    assert!(read(&vault, "Idées.md").contains("![[Archive/Oxinote#Architecture]]"));
    assert!(read(&vault, "Journal/2026-10-20.md").contains("[[Archive/Oxinote]]"));
    assert!(read(&vault, "Projets/Rust notes.md").contains("[[Oxinote]]"));
    assert_eq!(
        sources(&cache, "Archive/Oxinote.md"),
        vec![
            "Accueil.md",
            "Idées.md",
            "Journal/2026-10-20.md",
            "Projets/Rust notes.md"
        ]
    );
    assert_eq!(cache.unresolved().len(), 2);
}
