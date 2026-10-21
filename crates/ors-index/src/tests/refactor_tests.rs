use ors_core::{Error, VaultRead};

use super::{cache_of, p, vault_of};
use crate::{MetadataCache, RenamePlan, plan_rename, rename_folder_with_links, rename_with_links};

/// `(note, old link target, new target)` triples of a plan.
fn edits(plan: &RenamePlan) -> Vec<(String, String, String)> {
    plan.edits
        .iter()
        .flat_map(|(n, es)| {
            es.iter().map(move |e| {
                (
                    n.as_str().to_string(),
                    e.link.target.clone(),
                    e.new_target.clone(),
                )
            })
        })
        .collect()
}

fn t(n: &str, old: &str, new: &str) -> (String, String, String) {
    (n.into(), old.into(), new.into())
}

fn plan(
    files: &[(&str, &str)],
    from: &str,
    to: &str,
) -> (Vec<(String, String, String)>, MetadataCache) {
    let mut c = cache_of(files);
    let before: Vec<_> = c
        .notes()
        .iter()
        .map(|n| (n.clone(), c.outgoing(n)))
        .collect();
    let pl = plan_rename(&mut c, &p(from), &p(to)).unwrap();
    // The cache is restored.
    for (n, out) in before {
        assert_eq!(c.outgoing(&n), out, "cache not restored for {n}");
    }
    (edits(&pl), c)
}

#[test]
fn basename_links_get_new_basename() {
    let (e, _) = plan(
        &[("A.md", "[[Old]] [[Old#h|x]] ![[Old]]"), ("Old.md", "")],
        "Old.md",
        "New.md",
    );
    assert_eq!(
        e,
        vec![
            t("A.md", "Old", "New"),
            t("A.md", "Old", "New"),
            t("A.md", "Old", "New")
        ]
    );
}

#[test]
fn full_path_links_keep_full_path() {
    let (e, _) = plan(
        &[("A.md", "[[f/Old]] [[f/Old.md]]"), ("f/Old.md", "")],
        "f/Old.md",
        "g/New.md",
    );
    assert_eq!(
        e,
        vec![
            t("A.md", "f/Old", "g/New"),
            t("A.md", "f/Old.md", "g/New.md")
        ]
    );
}

#[test]
fn moving_folder_keeps_unambiguous_basename_links_untouched() {
    let (e, _) = plan(
        &[("A.md", "[[Old]]"), ("Old.md", "")],
        "Old.md",
        "sub/Old.md",
    );
    assert!(e.is_empty());
}

#[test]
fn basename_becoming_ambiguous_uses_full_path() {
    // Renaming to a name that already exists elsewhere: [[Other]] would
    // still resolve to the root `Other.md`, so the moved note needs its path.
    let (e, _) = plan(
        &[
            ("A.md", "[[Old]] [[Other]]"),
            ("Old.md", ""),
            ("Other.md", ""),
        ],
        "Old.md",
        "sub/Other.md",
    );
    assert_eq!(e, vec![t("A.md", "Old", "sub/Other")]);
}

#[test]
fn links_captured_by_new_file_are_pinned() {
    // `f/x.md` links [[N]] -> g/N.md. Moving Old.md to f/N.md would capture
    // it (same folder preference), so it is rewritten to the full path.
    let (e, _) = plan(
        &[("f/x.md", "[[N]]"), ("g/N.md", ""), ("Old.md", "")],
        "Old.md",
        "f/N.md",
    );
    assert_eq!(e, vec![t("f/x.md", "N", "g/N")]);
}

#[test]
fn markdown_links_relative_and_encoded() {
    let (e, _) = plan(
        &[
            ("dir/A.md", "[x](Old%20name.md) [y](../top.md)"),
            ("dir/Old name.md", ""),
            ("top.md", ""),
        ],
        "dir/Old name.md",
        "other/New name.md",
    );
    assert_eq!(
        e,
        vec![t("dir/A.md", "Old name.md", "../other/New%20name.md")]
    );
}

#[test]
fn markdown_absolute_links_stay_absolute() {
    let (e, _) = plan(
        &[
            ("dir/A.md", "[x](Projets/Rust%20notes.md)"),
            ("Projets/Rust notes.md", ""),
        ],
        "Projets/Rust notes.md",
        "Projets/Rust book.md",
    );
    assert_eq!(
        e,
        vec![t(
            "dir/A.md",
            "Projets/Rust notes.md",
            "Projets/Rust%20book.md"
        )]
    );
}

#[test]
fn moved_note_relative_links_are_fixed() {
    let (e, _) = plan(
        &[
            ("a/Me.md", "[x](Sib.md) [[Sib]]"),
            ("a/Sib.md", ""),
            ("Sib.md", ""),
        ],
        "a/Me.md",
        "b/Me.md",
    );
    // The markdown link was relative to `a/` (from `b/` it would now hit the
    // root `Sib.md`); the wikilink pointed at the root note and still does.
    assert_eq!(e, vec![t("b/Me.md", "Sib.md", "../a/Sib.md")]);
}

#[test]
fn self_links_untouched() {
    let (e, _) = plan(&[("Old.md", "[[#h]] [[Old#h]]")], "Old.md", "New.md");
    assert_eq!(e, vec![t("New.md", "Old", "New")]);
}

#[test]
fn attachments_renamed() {
    let (e, _) = plan(
        &[
            ("n.md", "![[pic.png]] ![](img/pic.png)"),
            ("img/pic.png", ""),
        ],
        "img/pic.png",
        "img/photo.png",
    );
    assert_eq!(
        e,
        vec![
            t("n.md", "pic.png", "photo.png"),
            t("n.md", "img/pic.png", "img/photo.png")
        ]
    );
}

#[test]
fn plan_rename_errors() {
    let mut c = cache_of(&[("a.md", ""), ("b.md", "")]);
    assert!(matches!(
        plan_rename(&mut c, &p("x.md"), &p("y.md")),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        plan_rename(&mut c, &p("a.md"), &p("b.md")),
        Err(Error::AlreadyExists(_))
    ));
    assert!(
        plan_rename(&mut c, &p("a.md"), &p("a.md"))
            .unwrap()
            .edits
            .is_empty()
    );
}

#[test]
fn rename_with_links_moves_file_and_cache() {
    let files = [("A.md", "[[Old]]"), ("Old.md", "x")];
    let vault = vault_of(&files);
    let mut c = cache_of(&files);
    rename_with_links(&vault, &mut c, &p("Old.md"), &p("New.md")).unwrap();
    assert!(vault.exists(&p("New.md")));
    assert!(!vault.exists(&p("Old.md")));
    assert!(c.contains(&p("New.md")));
    assert!(!c.contains(&p("Old.md")));
}

#[test]
fn rename_with_links_errors() {
    let files = [("a.md", ""), ("b.md", "")];
    let vault = vault_of(&files);
    let mut c = cache_of(&files);
    assert!(rename_with_links(&vault, &mut c, &p("zz.md"), &p("y.md")).is_err());
    assert!(rename_with_links(&vault, &mut c, &p("a.md"), &p("b.md")).is_err());
    // Nothing changed after a failed rename.
    assert!(c.contains(&p("a.md")) && vault.exists(&p("a.md")));
    assert!(
        rename_with_links(&vault, &mut c, &p("a.md"), &p("a.md"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn rename_folder_moves_everything() {
    let files = [
        ("old/a.md", "[[b]]"),
        ("old/sub/b.md", ""),
        ("old/pic.png", ""),
        ("keep.md", "[[a]]"),
        ("older/x.md", ""),
    ];
    let vault = vault_of(&files);
    let mut c = cache_of(&files);
    rename_folder_with_links(&vault, &mut c, "old", "new/place").unwrap();
    let all = vault.list_files().unwrap();
    let names: Vec<&str> = all.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "keep.md",
            "new/place/a.md",
            "new/place/pic.png",
            "new/place/sub/b.md",
            "older/x.md"
        ]
    );
    assert_eq!(c.files(), all);
    assert!(rename_folder_with_links(&vault, &mut c, "x", "x/y").is_err());
    assert!(rename_folder_with_links(&vault, &mut c, "", "y").is_err());
}

#[test]
fn rename_folder_clash_moves_nothing() {
    let files = [
        ("old/a.md", "[[b]]"),
        ("old/b.md", ""),
        ("new/b.md", "taken"),
        ("keep.md", "[[a]]"),
    ];
    let vault = vault_of(&files);
    let mut c = cache_of(&files);
    let before = vault.list_files().unwrap();
    assert!(rename_folder_with_links(&vault, &mut c, "old", "new").is_err());
    assert_eq!(vault.list_files().unwrap(), before);
    assert_eq!(c.files(), before);
    let keep = ors_core::NotePath::new("keep.md").unwrap();
    assert_eq!(vault.read_to_string(&keep).unwrap(), "[[a]]");
}
