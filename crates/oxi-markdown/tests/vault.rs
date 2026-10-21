#![allow(clippy::unwrap_used)]
//! Parse every note of the sample vault.

use std::path::{Path, PathBuf};

use oxi_core::{LinkKind, NoResolver, NotePath, Subpath};
use oxi_markdown::{RenderOptions, parse, render_html, rewrite_links};

fn vault() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-vault")
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            md_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

fn read(name: &str) -> String {
    std::fs::read_to_string(vault().join(name)).unwrap()
}

#[test]
fn every_note_parses_renders_and_roundtrips() {
    let mut files = Vec::new();
    md_files(&vault(), &mut files);
    assert!(files.len() >= 6, "{files:?}");
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let p = parse(&src);
        for l in &p.links {
            assert!(l.span.end <= src.len());
        }
        let rel = f.strip_prefix(vault()).unwrap();
        let o = RenderOptions::new(NotePath::new(rel.to_string_lossy()).unwrap());
        let _ = render_html(&src, &o, &NoResolver);
        assert_eq!(rewrite_links(&src, |_| None), src);
    }
}

#[test]
fn idees() {
    let src = read("Idées.md");
    let p = parse(&src);
    let fm = p.frontmatter.as_ref().unwrap();
    assert_eq!(fm["status"], "brouillon");

    let tags: Vec<(&str, bool)> = p
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.from_frontmatter))
        .collect();
    assert_eq!(tags, [("idée", true), ("projet/oxinote", true)]);

    let heads: Vec<(u8, &str)> = p
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str()))
        .collect();
    assert_eq!(heads, [(1, "Idées"), (2, "Fonctionnalités"), (2, "Embeds")]);

    let links: Vec<(&str, LinkKind, bool)> = p
        .links
        .iter()
        .map(|l| (l.target.as_str(), l.kind, l.is_embed))
        .collect();
    assert_eq!(
        links,
        [
            ("Accueil", LinkKind::Frontmatter, false),
            ("Canvas", LinkKind::Wiki, false),
            ("Accueil", LinkKind::Wiki, false),
            ("diagramme.png", LinkKind::Wiki, true),
            ("Projets/OxiNote", LinkKind::Wiki, true),
            ("Idées", LinkKind::Wiki, true),
        ]
    );
    assert_eq!(p.links[2].display.as_deref(), Some("liens"));
    assert_eq!(
        p.links[2].subpath,
        Some(Subpath::Heading(vec!["Accueil".into()]))
    );
    assert_eq!(
        p.links[4].subpath,
        Some(Subpath::Heading(vec!["Architecture".into()]))
    );
    assert_eq!(p.links[5].subpath, Some(Subpath::Block("graphe".into())));
    assert!(!p.links.iter().any(|l| l.target.contains("caché")
        || l.target.contains("pas un lien")
        || l.target.contains("ni ceci")));

    assert_eq!(p.block_ids.len(), 1);
    assert_eq!(p.block_ids[0].id, "graphe");
    let tasks: Vec<char> = p.tasks.iter().map(|t| t.status).collect();
    assert_eq!(tasks, [' ', 'x', '/', ' ']);
    assert_eq!(p.tasks[2].text, "Recherche plein texte");
    assert!(p.word_count > 30);
}

#[test]
fn accueil() {
    let src = read("Accueil.md");
    let p = parse(&src);
    assert_eq!(p.aliases, ["Home", "Index"]);
    let tags: Vec<(&str, bool)> = p
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.from_frontmatter))
        .collect();
    assert_eq!(
        tags,
        [("hub", true), ("hub", false), ("meta/organisation", false)]
    );
    assert_eq!(p.headings.len(), 1);
    assert_eq!(p.headings[0].text, "Accueil");
    let links: Vec<(&str, LinkKind)> = p
        .links
        .iter()
        .map(|l| (l.target.as_str(), l.kind))
        .collect();
    assert_eq!(
        links,
        [
            ("Projets/OxiNote", LinkKind::Wiki),
            ("Idées", LinkKind::Wiki),
            ("2026-10-20", LinkKind::Wiki),
            ("Note fantôme", LinkKind::Wiki),
            ("Projets/Rust notes.md", LinkKind::Markdown),
        ]
    );
    assert_eq!(p.links[0].display.as_deref(), Some("le projet"));
    assert_eq!(
        p.links[4].subpath,
        Some(Subpath::Heading(vec!["Ownership".into()]))
    );
    assert_eq!(p.links[4].line, 10);
    assert_eq!(
        &src[p.links[3].span.start..p.links[3].span.end],
        "[[Note fantôme]]"
    );
}

#[test]
fn other_notes() {
    let p = parse(&read("Projets/Rust notes.md"));
    assert_eq!(p.block_ids[0].id, "borrow");
    assert_eq!(
        p.links.last().unwrap().subpath,
        Some(Subpath::Block("borrow".into()))
    );
    let p = parse(&read("Projets/OxiNote.md"));
    let tags: Vec<&str> = p.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tags, ["projet/oxinote", "rust", "projet/oxinote"]);
    assert_eq!(p.frontmatter.unwrap()["date"], "2026-10-20");
    let p = parse(&read("Journal/2026-10-20.md"));
    assert_eq!(p.links[1].display.as_deref(), Some("borrowing"));
    let p = parse(&read("Orpheline.md"));
    assert!(p.links.is_empty() && p.tags.is_empty());
    let p = parse(&read("Templates/Daily.md"));
    assert_eq!(p.tasks.len(), 1);
}
