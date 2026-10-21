//! Properties / tag views and edits. These use the real `oxi-markdown`
//! parser (through `MetadataCache::build`) since edits rewrite sources.

use oxi_core::{MemoryVault, VaultRead};
use serde_json::json;

use super::{p, paths, vault_of};
use crate::{
    MetadataCache, TagNode, add_tag_in, plan_tag_rename, remove_property_in, remove_tag_in,
    rename_property_everywhere, rename_tag, set_property_in,
};

fn build(files: &[(&str, &str)]) -> (MemoryVault, MetadataCache) {
    let v = vault_of(files);
    let c = MetadataCache::build(&v).unwrap();
    (v, c)
}

fn read(v: &MemoryVault, path: &str) -> String {
    v.read_to_string(&p(path)).unwrap()
}

const FILES: &[(&str, &str)] = &[
    (
        "a.md",
        "---\nstatus: draft\ntags: [Projet/Rust, idée]\ndue: 2026-10-20\ncount: 3\n---\nBody #rust #Projet/rust/async\n",
    ),
    (
        "b.md",
        "---\nStatus: done\ntags:\n  - projet\nrelated: \"[[a]]\"\ncount: 4\nflags: [x, y]\n---\n#idée here `#code`\n",
    ),
    ("c.md", "---\nstatus:\ncount: \"n/a\"\n---\nplain #other\n"),
    ("d.md", "no frontmatter #rust\n"),
    ("img.png", ""),
];

#[test]
fn all_properties() {
    let (_v, c) = build(FILES);
    let props = c.properties();
    let keys: Vec<&str> = props.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["count", "due", "flags", "related", "status", "tags"]
    );
    let status = &props["status"];
    assert_eq!(status.name, "status");
    assert_eq!(status.count, 3);
    assert_eq!(status.inferred_type, "text");
    assert_eq!(status.type_counts.get("text"), Some(&2));
    assert_eq!(props["count"].inferred_type, "number");
    assert_eq!(props["count"].count, 3);
    assert_eq!(props["due"].inferred_type, "date");
    assert_eq!(props["tags"].inferred_type, "multitext");
    assert_eq!(props["flags"].inferred_type, "multitext");
}

#[test]
fn values_and_files_with_property() {
    let (_v, c) = build(FILES);
    let vals = c.property_values("STATUS");
    assert_eq!(
        vals.into_iter().collect::<Vec<_>>(),
        vec![("done".to_string(), 1), ("draft".to_string(), 1)]
    );
    let vals = c.property_values("count");
    assert_eq!(vals["3"], 1);
    assert_eq!(vals["n/a"], 1);
    let vals = c.property_values("flags");
    assert_eq!(vals["x"], 1);
    assert_eq!(vals["y"], 1);
    assert!(c.property_values("nope").is_empty());

    assert_eq!(
        paths(&c.files_with_property("status", None)),
        vec!["a.md", "b.md", "c.md"]
    );
    assert_eq!(
        paths(&c.files_with_property("status", Some(&json!("DONE")))),
        vec!["b.md"]
    );
    assert_eq!(
        paths(&c.files_with_property("count", Some(&json!(3)))),
        vec!["a.md"]
    );
    assert_eq!(
        paths(&c.files_with_property("flags", Some(&json!("y")))),
        vec!["b.md"]
    );
    assert_eq!(
        paths(&c.files_with_property("flags", Some(&json!(["x", "y"])))),
        vec!["b.md"]
    );
    assert!(c.files_with_property("missing", None).is_empty());
}

#[test]
fn tag_suggestions_ranked() {
    let (_v, c) = build(FILES);
    let all = c.tag_suggestions("", 100);
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    // rust: a, d (2 notes); projet/rust: a; projet: a(children)+b...
    assert!(names.contains(&"rust"));
    assert!(names.contains(&"Projet/Rust"));
    assert!(!names.contains(&"code"));
    let s = c.tag_suggestions("#ru", 10);
    assert_eq!(s[0], ("rust".to_string(), 2));
    let rest: Vec<&str> = s[1..].iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(rest, vec!["Projet/Rust", "Projet/rust/async"]);
    let s = c.tag_suggestions("PROJ", 1);
    assert_eq!(s.len(), 1);
    // Display spelling is the first one seen (a.md writes `Projet/...`).
    assert_eq!(s[0].0, "Projet");
    assert_eq!(s[0].1, 2);
    let s = c.tag_suggestions("syn", 10);
    assert_eq!(s, vec![("Projet/rust/async".to_string(), 1)]);
    assert!(c.tag_suggestions("zzz", 10).is_empty());
}

fn flatten(nodes: &[TagNode], out: &mut Vec<(String, usize)>) {
    for n in nodes {
        out.push((n.full_name.clone(), n.count));
        flatten(&n.children, out);
    }
}

#[test]
fn tag_tree_counts_notes() {
    let (_v, c) = build(FILES);
    let tree = c.tag_tree();
    let mut flat = Vec::new();
    flatten(&tree, &mut flat);
    assert_eq!(
        flat,
        vec![
            ("idée".to_string(), 2),
            ("other".to_string(), 1),
            ("Projet".to_string(), 2),
            ("Projet/Rust".to_string(), 1),
            ("Projet/rust/async".to_string(), 1),
            ("rust".to_string(), 2),
        ]
    );
    let projet = tree.iter().find(|n| n.name == "Projet").unwrap();
    assert_eq!(projet.children[0].name, "Rust");
    assert_eq!(projet.children[0].children[0].name, "async");
}

#[test]
fn rename_tag_everywhere_with_merge() {
    let (v, mut c) = build(FILES);
    let plan = plan_tag_rename(&v, &c, "projet", "work").unwrap();
    assert_eq!(
        plan.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        vec!["a.md", "b.md"]
    );
    assert_eq!(read(&v, "a.md"), FILES[0].1, "plan writes nothing");

    let changed = rename_tag(&v, &mut c, "#projet", "work").unwrap();
    assert_eq!(paths(&changed), vec!["a.md", "b.md"]);
    assert_eq!(
        read(&v, "a.md"),
        "---\nstatus: draft\ntags: [work/Rust, idée]\ndue: 2026-10-20\ncount: 3\n---\nBody #rust #work/rust/async\n"
    );
    assert!(read(&v, "b.md").contains("tags:\n  - work\n"));
    assert!(c.files_with_tag("projet").is_empty());
    assert_eq!(paths(&c.files_with_tag("work")), vec!["a.md", "b.md"]);

    // Merge `rust` into `work/rust`... and `idée` into `other` (dedupe).
    let changed = rename_tag(&v, &mut c, "idée", "rust").unwrap();
    assert_eq!(paths(&changed), vec!["a.md", "b.md"]);
    assert!(read(&v, "a.md").contains("tags: [work/Rust, rust]"));
    let changed = rename_tag(&v, &mut c, "work/rust", "rust").unwrap();
    assert_eq!(paths(&changed), vec!["a.md"]);
    let a = read(&v, "a.md");
    assert!(a.contains("tags:\n  - rust\n"), "{a}");
    assert!(a.ends_with("Body #rust #rust/async\n"), "{a}");
    assert_eq!(c.tags_of(&p("a.md")), vec!["rust", "rust/async"]);

    assert!(rename_tag(&v, &mut c, "rust", "bad tag").is_err());
    assert!(rename_tag(&v, &mut c, "", "x").is_err());
    assert!(rename_tag(&v, &mut c, "absent", "x").unwrap().is_empty());
}

#[test]
fn property_edits_update_cache() {
    let (v, mut c) = build(FILES);
    let d = p("d.md");
    let changed = set_property_in(&v, &mut c, &d, "status", &json!("new")).unwrap();
    assert_eq!(paths(&changed), vec!["d.md"]);
    assert_eq!(
        read(&v, "d.md"),
        "---\nstatus: new\n---\nno frontmatter #rust\n"
    );
    assert_eq!(c.properties()["status"].count, 4);
    // Same value again: no change.
    assert!(
        set_property_in(&v, &mut c, &d, "status", &json!("new"))
            .unwrap()
            .is_empty()
    );

    let changed = remove_property_in(&v, &mut c, &d, "status").unwrap();
    assert_eq!(paths(&changed), vec!["d.md"]);
    assert_eq!(read(&v, "d.md"), "no frontmatter #rust\n");
    assert!(
        remove_property_in(&v, &mut c, &d, "status")
            .unwrap()
            .is_empty()
    );

    add_tag_in(&v, &mut c, &d, "fresh").unwrap();
    assert!(c.has_tag(&d, "fresh"));
    remove_tag_in(&v, &mut c, &d, "rust").unwrap();
    assert_eq!(c.tags_of(&d), vec!["fresh"]);
    assert_eq!(
        read(&v, "d.md"),
        "---\ntags:\n  - fresh\n---\nno frontmatter\n"
    );

    assert!(set_property_in(&v, &mut c, &p("img.png"), "x", &json!(1)).is_err());
    assert!(set_property_in(&v, &mut c, &p("missing.md"), "x", &json!(1)).is_err());
}

#[test]
fn invalid_yaml_note_is_refused_and_untouched() {
    let (v, mut c) = build(&[("bad.md", "---\na: [1\n---\nx\n")]);
    assert!(set_property_in(&v, &mut c, &p("bad.md"), "b", &json!(1)).is_err());
    assert_eq!(read(&v, "bad.md"), "---\na: [1\n---\nx\n");
}

#[test]
fn rename_property_vault_wide() {
    let (v, mut c) = build(&[
        ("a.md", "---\nstatus: x\n---\n"),
        ("b.md", "---\nStatus: y\nother: 1\n---\n"),
        ("c.md", "---\nstatus: z\nstate: taken\n---\n"),
        ("d.md", "body\n"),
    ]);
    let changed = rename_property_everywhere(&v, &mut c, "status", "state").unwrap();
    assert_eq!(paths(&changed), vec!["a.md", "b.md"]);
    assert_eq!(read(&v, "a.md"), "---\nstate: x\n---\n");
    assert_eq!(read(&v, "b.md"), "---\nstate: y\nother: 1\n---\n");
    assert_eq!(
        read(&v, "c.md"),
        "---\nstatus: z\nstate: taken\n---\n",
        "conflict skipped"
    );
    assert_eq!(paths(&c.files_with_property("status", None)), vec!["c.md"]);
    assert_eq!(c.properties()["state"].count, 3);
    assert!(rename_property_everywhere(&v, &mut c, "state", " ").is_err());
}
