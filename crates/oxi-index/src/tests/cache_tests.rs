use oxi_core::{LinkKind, MemoryVault, NotePath, VaultWrite};

use super::{NoteBuilder, cache_of, md_link, mini_parse, p, paths, vault_of, wiki};
use crate::MetadataCache;

#[test]
fn rename_folder_metadata_only() {
    let mut c = cache_of(&[("f/a.md", "[[b]]"), ("f/b.md", ""), ("x.md", "[[f/b]]")]);
    c.rename_folder("f", "g/h");
    assert_eq!(paths(&c.files()), vec!["g/h/a.md", "g/h/b.md", "x.md"]);
    assert_eq!(backlink_sources(&c, "g/h/b.md"), vec!["g/h/a.md"]);
    assert_eq!(c.unresolved()[&p("x.md")], vec!["f/b"]);
}

fn backlink_sources(c: &MetadataCache, path: &str) -> Vec<String> {
    c.backlinks(&p(path))
        .into_iter()
        .map(|b| b.source.as_str().to_string())
        .collect()
}

#[test]
fn backlinks_include_embeds_and_frontmatter_links() {
    let mut c = MetadataCache::new();
    c.insert_parsed(p("A.md"), NoteBuilder::new().build());
    let mut fm = wiki("A");
    fm.kind = LinkKind::Frontmatter;
    let mut embed = wiki("A");
    embed.is_embed = true;
    c.insert_parsed(p("B.md"), NoteBuilder::new().link(fm).link(embed).build());
    c.insert_parsed(p("C.md"), NoteBuilder::new().links(&["a"]).build());
    assert_eq!(backlink_sources(&c, "A.md"), vec!["B.md", "B.md", "C.md"]);
    assert_eq!(c.backlink_sources(&p("A.md")).len(), 2);
}

#[test]
fn backlinks_grouped_by_source_in_path_order() {
    let c = cache_of(&[
        ("T.md", ""),
        ("z.md", "[[T]] [[T#h]]"),
        ("a.md", "[[T]]"),
        ("m/x.md", "![[T]]"),
    ]);
    assert_eq!(
        backlink_sources(&c, "T.md"),
        vec!["a.md", "m/x.md", "z.md", "z.md"]
    );
}

#[test]
fn self_links_not_in_backlinks() {
    let c = cache_of(&[("A.md", "[[#Heading]] [[A]]")]);
    assert!(c.backlinks(&p("A.md")).is_empty());
    let out = c.outgoing(&p("A.md"));
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| r.target.as_ref() == Some(&p("A.md"))));
    assert_eq!(paths(&c.orphans()), vec!["A.md"]);
}

#[test]
fn outgoing_reports_resolution() {
    let c = cache_of(&[("A.md", "[[B]] [[Nope]] [x](B.md)"), ("B.md", "")]);
    let out = c.outgoing(&p("A.md"));
    let targets: Vec<Option<&str>> = out
        .iter()
        .map(|r| r.target.as_ref().map(NotePath::as_str))
        .collect();
    assert_eq!(targets, vec![Some("B.md"), None, Some("B.md")]);
    assert_eq!(out[2].link.kind, LinkKind::Markdown);
}

#[test]
fn unresolved_map_deduplicates() {
    let c = cache_of(&[("A.md", "[[X]] [[X]] [[Y]]"), ("B.md", "[[A]]")]);
    let u = c.unresolved();
    assert_eq!(u.len(), 1);
    assert_eq!(u[&p("A.md")], vec!["X", "Y"]);
}

#[test]
fn new_file_resolves_previously_unresolved() {
    let mut c = cache_of(&[("A.md", "[[Later]]")]);
    assert_eq!(c.unresolved().len(), 1);
    c.insert_parsed(p("sub/Later.md"), NoteBuilder::new().build());
    assert!(c.unresolved().is_empty());
    assert_eq!(backlink_sources(&c, "sub/Later.md"), vec!["A.md"]);
}

#[test]
fn deleting_file_makes_links_unresolved() {
    let mut c = cache_of(&[("A.md", "[[B]]"), ("B.md", "")]);
    c.remove_file(&p("B.md"));
    assert_eq!(c.unresolved()[&p("A.md")], vec!["B"]);
    assert!(c.backlinks(&p("B.md")).is_empty());
    assert!(!c.contains(&p("B.md")));
}

#[test]
fn deleting_file_falls_back_to_other_candidate() {
    let mut c = cache_of(&[("f/A.md", "[[N]]"), ("f/N.md", ""), ("g/N.md", "")]);
    assert_eq!(c.outgoing(&p("f/A.md"))[0].target, Some(p("f/N.md")));
    c.remove_file(&p("f/N.md"));
    assert_eq!(c.outgoing(&p("f/A.md"))[0].target, Some(p("g/N.md")));
    assert_eq!(backlink_sources(&c, "g/N.md"), vec!["f/A.md"]);
}

#[test]
fn new_closer_file_steals_resolution() {
    let mut c = cache_of(&[("f/A.md", "[[N]]"), ("g/N.md", "")]);
    assert_eq!(c.outgoing(&p("f/A.md"))[0].target, Some(p("g/N.md")));
    c.insert_parsed(p("f/N.md"), NoteBuilder::new().build());
    assert_eq!(c.outgoing(&p("f/A.md"))[0].target, Some(p("f/N.md")));
    assert!(c.backlinks(&p("g/N.md")).is_empty());
}

#[test]
fn update_note_replaces_links_and_tags() {
    let mut c = cache_of(&[("A.md", "[[B]] #old"), ("B.md", ""), ("C.md", "")]);
    c.insert_parsed(p("A.md"), mini_parse("[[C]] #new"));
    assert!(c.backlinks(&p("B.md")).is_empty());
    assert_eq!(backlink_sources(&c, "C.md"), vec!["A.md"]);
    assert!(c.files_with_tag("old").is_empty());
    assert_eq!(paths(&c.files_with_tag("new")), vec!["A.md"]);
    assert!(!c.tags().contains_key("old"));
}

#[test]
fn rename_file_updates_resolution() {
    let mut c = cache_of(&[("A.md", "[[B]]"), ("B.md", "[[A]]")]);
    c.rename_file(&p("B.md"), &p("x/C.md"));
    // Text says [[B]] -> now unresolved.
    assert_eq!(c.unresolved()[&p("A.md")], vec!["B"]);
    // Links from the moved note still work.
    assert_eq!(backlink_sources(&c, "A.md"), vec!["x/C.md"]);
    assert!(c.get(&p("x/C.md")).is_some());
    assert!(c.get(&p("B.md")).is_none());
}

#[test]
fn rename_keeps_basename_links_when_moving_folders() {
    let mut c = cache_of(&[("A.md", "[[B]]"), ("B.md", "")]);
    c.rename_file(&p("B.md"), &p("deep/B.md"));
    assert_eq!(backlink_sources(&c, "deep/B.md"), vec!["A.md"]);
    assert!(c.unresolved().is_empty());
}

#[test]
fn relative_links_follow_the_moved_note() {
    let mut c = cache_of(&[("f/A.md", "[x](N.md)"), ("f/N.md", ""), ("g/N.md", "")]);
    assert_eq!(c.outgoing(&p("f/A.md"))[0].target, Some(p("f/N.md")));
    c.rename_file(&p("f/A.md"), &p("g/A.md"));
    assert_eq!(c.outgoing(&p("g/A.md"))[0].target, Some(p("g/N.md")));
}

#[test]
fn tags_count_occurrences_and_parents() {
    let c = cache_of(&[
        ("a.md", "#Projet/Rust #projet #rust"),
        ("b.md", "#projet/rust/core"),
    ]);
    let t = c.tags();
    assert_eq!(t["projet"], 3);
    assert_eq!(t["projet/rust"], 2);
    assert_eq!(t["projet/rust/core"], 1);
    assert_eq!(t["rust"], 1);
    assert_eq!(c.tags_of(&p("a.md")), vec!["projet", "projet/rust", "rust"]);
}

#[test]
fn files_with_tag_nested_and_hash() {
    let c = cache_of(&[
        ("a.md", "#Projet/Rust"),
        ("b.md", "#projet"),
        ("c.md", "#projets"),
    ]);
    assert_eq!(paths(&c.files_with_tag("projet")), vec!["a.md", "b.md"]);
    assert_eq!(paths(&c.files_with_tag("#PROJET")), vec!["a.md", "b.md"]);
    assert_eq!(paths(&c.files_with_tag("projet/rust")), vec!["a.md"]);
    assert!(c.files_with_tag("rust").is_empty());
    assert!(c.has_tag(&p("a.md"), "#projet"));
}

#[test]
fn frontmatter_tags_counted() {
    let mut note = NoteBuilder::new().tags(&["hub"]).build();
    note.tags[0].from_frontmatter = true;
    let mut c = MetadataCache::new();
    c.insert_parsed(p("A.md"), note);
    assert_eq!(c.tags()["hub"], 1);
}

#[test]
fn orphans_ignore_unresolved_links() {
    let c = cache_of(&[
        ("a.md", "[[b]]"),
        ("b.md", ""),
        ("lonely.md", "[[ghost]]"),
        ("img.png", ""),
    ]);
    assert_eq!(paths(&c.orphans()), vec!["lonely.md"]);
}

#[test]
fn attachments_listed_and_linkable() {
    let c = cache_of(&[("n.md", "![[pic.png]]"), ("a/pic.png", "")]);
    assert_eq!(paths(&c.attachments()), vec!["a/pic.png"]);
    assert_eq!(paths(&c.notes()), vec!["n.md"]);
    assert_eq!(paths(&c.files()), vec!["a/pic.png", "n.md"]);
    assert_eq!(backlink_sources(&c, "a/pic.png"), vec!["n.md"]);
    assert!(c.get(&p("a/pic.png")).is_none());
}

#[test]
fn stats_are_consistent() {
    let c = cache_of(&[("a.md", "[[b]] [[x]] #t/u"), ("b.md", "w"), ("i.png", "")]);
    let s = c.stats();
    assert_eq!(s.notes, 2);
    assert_eq!(s.attachments, 1);
    assert_eq!(s.links, 2);
    assert_eq!(s.resolved_links, 1);
    assert_eq!(s.unresolved_links, 1);
    assert_eq!(s.tags, 2);
}

#[test]
fn unlinked_mentions_whole_word_outside_links() {
    let files = [
        ("Rust.md", "about rust"),
        ("a.md", "I like Rust a lot"),
        ("b.md", "I like [[Rust]] only"),
        ("c.md", "trusty rusty"),
        ("d.md", "[[Rust]] and RUST again"),
        ("e.md", "the oxide crab"),
    ];
    let vault = vault_of(&files);
    let mut c = cache_of(&files);
    c.insert_parsed(p("Rust.md"), NoteBuilder::new().alias("Oxide").build());
    let m = c.unlinked_mentions(&vault, &p("Rust.md"));
    assert_eq!(paths(&m), vec!["a.md", "d.md", "e.md"]);
    let detailed = c.unlinked_mentions_detailed(&vault, &p("Rust.md"));
    let d = &detailed
        .iter()
        .find(|(p, _)| p.as_str() == "d.md")
        .unwrap()
        .1;
    assert_eq!(d[0].ranges, vec![(13, 17)]);
}

#[test]
fn update_file_reads_vault_and_handles_deletion() {
    let vault = MemoryVault::from_files([("a.md", "x"), ("b.png", "")]);
    let mut c = MetadataCache::build(&vault).unwrap();
    assert_eq!(c.len(), 2);
    vault.write(&p("new.md"), "hello").unwrap();
    c.update_file(&vault, &p("new.md")).unwrap();
    assert!(c.get(&p("new.md")).is_some());
    vault.delete(&p("b.png")).unwrap();
    c.update_file(&vault, &p("b.png")).unwrap();
    assert!(!c.contains(&p("b.png")));
}

#[test]
fn update_from_source_non_markdown_is_attachment() {
    let mut c = MetadataCache::new();
    c.update_from_source(&p("x.canvas"), "{}");
    assert!(c.contains(&p("x.canvas")));
    assert!(c.get(&p("x.canvas")).is_none());
}

#[test]
fn insert_file_markdown_registers_empty_note() {
    let mut c = MetadataCache::new();
    c.insert_file(p("n.md"));
    assert!(c.get(&p("n.md")).is_some());
    // insert_file does not clobber an existing note.
    c.insert_parsed(p("m.md"), NoteBuilder::new().links(&["n"]).build());
    c.insert_file(p("m.md"));
    assert_eq!(c.outgoing(&p("m.md")).len(), 1);
}

#[test]
fn extend_parsed_bulk_resolves_forward_links() {
    let mut c = MetadataCache::new();
    c.extend_parsed(vec![
        (p("a.md"), NoteBuilder::new().links(&["b", "c"]).build()),
        (p("b.md"), NoteBuilder::new().links(&["a"]).build()),
    ]);
    assert_eq!(c.unresolved()[&p("a.md")], vec!["c"]);
    assert_eq!(backlink_sources(&c, "b.md"), vec!["a.md"]);
    assert_eq!(backlink_sources(&c, "a.md"), vec!["b.md"]);
}

#[test]
fn markdown_link_kinds_resolve() {
    let mut c = MetadataCache::new();
    c.insert_parsed(p("dir/T.md"), NoteBuilder::new().build());
    c.insert_parsed(
        p("dir/S.md"),
        NoteBuilder::new().link(md_link("T.md")).build(),
    );
    assert_eq!(backlink_sources(&c, "dir/T.md"), vec!["dir/S.md"]);
}

#[test]
fn consistency_after_many_operations() {
    // Random-ish sequence; compare with a cache rebuilt from scratch.
    let mut c = MetadataCache::new();
    let names = ["a", "b", "c", "d/a", "d/b", "e/f/c"];
    for (i, n) in names.iter().enumerate() {
        let links: Vec<&str> = names
            .iter()
            .cycle()
            .skip(i + 1)
            .take(2)
            .map(|x| x.rsplit('/').next().unwrap())
            .collect();
        c.insert_parsed(
            p(&format!("{n}.md")),
            NoteBuilder::new().links(&links).build(),
        );
    }
    c.remove_file(&p("b.md"));
    c.rename_file(&p("d/a.md"), &p("z/zz.md"));
    c.insert_parsed(p("b2/b.md"), NoteBuilder::new().links(&["zz"]).build());
    let mut fresh = MetadataCache::new();
    for path in c.notes() {
        fresh.insert_parsed(path.clone(), c.get(&path).unwrap().clone());
    }
    for path in c.notes() {
        assert_eq!(c.outgoing(&path), fresh.outgoing(&path), "{path}");
        assert_eq!(c.backlinks(&path), fresh.backlinks(&path), "{path}");
    }
    assert_eq!(c.unresolved(), fresh.unresolved());
    assert_eq!(c.orphans(), fresh.orphans());
}
