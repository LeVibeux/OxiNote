use oxi_core::{LinkKind, LinkResolver};

use super::{cache_of, p};

fn res(c: &crate::MetadataCache, target: &str, from: &str) -> Option<String> {
    c.resolve(target, &p(from)).map(|x| x.as_str().to_string())
}

fn res_md(c: &crate::MetadataCache, target: &str, from: &str) -> Option<String> {
    c.resolve_target(target, LinkKind::Markdown, &p(from))
        .map(|x| x.as_str().to_string())
}

#[test]
fn basename_resolution_case_insensitive() {
    let c = cache_of(&[("a/Note.md", ""), ("src.md", "")]);
    assert_eq!(res(&c, "Note", "src.md").as_deref(), Some("a/Note.md"));
    assert_eq!(res(&c, "note", "src.md").as_deref(), Some("a/Note.md"));
    assert_eq!(res(&c, "NOTE.md", "src.md").as_deref(), Some("a/Note.md"));
    assert_eq!(res(&c, "Missing", "src.md"), None);
}

#[test]
fn exact_path_with_or_without_md() {
    let c = cache_of(&[("a/Note.md", ""), ("b/Note.md", ""), ("x.md", "")]);
    assert_eq!(res(&c, "b/Note", "x.md").as_deref(), Some("b/Note.md"));
    assert_eq!(res(&c, "b/Note.md", "x.md").as_deref(), Some("b/Note.md"));
    assert_eq!(res(&c, "B/NOTE", "x.md").as_deref(), Some("b/Note.md"));
    assert_eq!(res(&c, "/a/Note", "x.md").as_deref(), Some("a/Note.md"));
}

#[test]
fn attachments_resolve_with_extension() {
    let c = cache_of(&[("attachments/diagramme.png", ""), ("n.md", "")]);
    assert_eq!(
        res(&c, "diagramme.png", "n.md").as_deref(),
        Some("attachments/diagramme.png")
    );
    assert_eq!(
        res(&c, "attachments/diagramme.png", "n.md").as_deref(),
        Some("attachments/diagramme.png")
    );
    // No extension: does not match the png.
    assert_eq!(res(&c, "diagramme", "n.md"), None);
}

#[test]
fn ambiguity_prefers_same_folder_then_shortest_then_alpha() {
    let c = cache_of(&[
        ("x/y/Note.md", ""),
        ("b/Note.md", ""),
        ("a/Note.md", ""),
        ("x/y/src.md", ""),
        ("root.md", ""),
    ]);
    assert_eq!(
        res(&c, "Note", "x/y/src.md").as_deref(),
        Some("x/y/Note.md")
    );
    // From the root: shortest (depth 1) then alphabetical.
    assert_eq!(res(&c, "Note", "root.md").as_deref(), Some("a/Note.md"));
}

#[test]
fn exact_root_path_wins_over_basename() {
    let c = cache_of(&[("Note.md", ""), ("f/Note.md", ""), ("f/src.md", "")]);
    // Exact vault path first (Obsidian behaviour for a root-level match).
    assert_eq!(res(&c, "Note", "f/src.md").as_deref(), Some("Note.md"));
}

#[test]
fn relative_to_source_folder() {
    let c = cache_of(&[("p/sub/Deep.md", ""), ("p/src.md", "")]);
    assert_eq!(
        res(&c, "sub/Deep", "p/src.md").as_deref(),
        Some("p/sub/Deep.md")
    );
    assert_eq!(
        res(&c, "./sub/Deep", "p/src.md").as_deref(),
        Some("p/sub/Deep.md")
    );
    assert_eq!(
        res(&c, "../p/sub/Deep", "p/src.md").as_deref(),
        Some("p/sub/Deep.md")
    );
}

#[test]
fn trailing_path_suffix_match() {
    let c = cache_of(&[
        ("deep/folder/Note.md", ""),
        ("other/Note.md", ""),
        ("s.md", ""),
    ]);
    assert_eq!(
        res(&c, "folder/Note", "s.md").as_deref(),
        Some("deep/folder/Note.md")
    );
    // A suffix must match whole components.
    assert_eq!(res(&c, "older/Note", "s.md"), None);
}

#[test]
fn markdown_links_prefer_relative() {
    let c = cache_of(&[("Note.md", ""), ("f/Note.md", ""), ("f/src.md", "")]);
    assert_eq!(
        res_md(&c, "Note.md", "f/src.md").as_deref(),
        Some("f/Note.md")
    );
    assert_eq!(res(&c, "Note.md", "f/src.md").as_deref(), Some("Note.md"));
    assert_eq!(
        res_md(&c, "../Note.md", "f/src.md").as_deref(),
        Some("Note.md")
    );
}

#[test]
fn empty_target_is_self() {
    let c = cache_of(&[("a.md", "")]);
    assert_eq!(res(&c, "", "a.md").as_deref(), Some("a.md"));
}

#[test]
fn unicode_nfc_comparison() {
    // File stored decomposed, link written precomposed.
    let c = cache_of(&[("Ide\u{301}es.md", ""), ("s.md", "")]);
    assert_eq!(res(&c, "Idées", "s.md").as_deref(), Some("Ide\u{301}es.md"));
    assert_eq!(res(&c, "IDÉES", "s.md").as_deref(), Some("Ide\u{301}es.md"));
}

#[test]
fn md_note_preferred_over_extensionless_file() {
    let c = cache_of(&[("a/Note", ""), ("b/Note.md", ""), ("s.md", "")]);
    assert_eq!(res(&c, "Note", "s.md").as_deref(), Some("b/Note.md"));
}

#[test]
fn case_variants_prefer_exact_spelling() {
    let c = cache_of(&[("note.md", ""), ("Note.md", ""), ("s.md", "")]);
    assert_eq!(res(&c, "Note", "s.md").as_deref(), Some("Note.md"));
    assert_eq!(res(&c, "note", "s.md").as_deref(), Some("note.md"));
}

#[test]
fn files_named_and_ambiguity() {
    let c = cache_of(&[("a/N.md", ""), ("b/n.md", ""), ("c/N.png", "")]);
    assert_eq!(c.files_named("N").len(), 2);
    assert_eq!(c.files_named("N.png").len(), 1);
    assert!(c.is_name_ambiguous(&p("a/N.md")));
    assert!(!c.is_name_ambiguous(&p("c/N.png")));
}

#[test]
fn link_text_for_shortest() {
    let c = cache_of(&[
        ("a/Unique.md", ""),
        ("a/Dup.md", ""),
        ("b/Dup.md", ""),
        ("img/pic.png", ""),
        ("s.md", ""),
    ]);
    assert_eq!(c.link_text_for(&p("a/Unique.md"), &p("s.md")), "Unique");
    assert_eq!(c.link_text_for(&p("b/Dup.md"), &p("s.md")), "b/Dup");
    assert_eq!(c.link_text_for(&p("img/pic.png"), &p("s.md")), "pic.png");
}
