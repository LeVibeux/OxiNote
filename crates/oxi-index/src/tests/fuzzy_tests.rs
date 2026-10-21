use super::{NoteBuilder, cache_of, p};
use crate::{FuzzyOptions, fuzzy_find, fuzzy_find_with};

#[test]
fn empty_query_alphabetical() {
    let c = cache_of(&[("b.md", ""), ("a.md", ""), ("c.png", ""), ("d/e.md", "")]);
    let r = fuzzy_find(&c, "", 2);
    let names: Vec<&str> = r.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names, vec!["a.md", "b.md"]);
    assert!(r.iter().all(|x| x.2 == 0 && x.1.is_none()));
}

#[test]
fn stem_beats_path_match() {
    let c = cache_of(&[("rust/zzz.md", ""), ("other/Rust.md", ""), ("x.md", "")]);
    let r = fuzzy_find(&c, "rust", 10);
    assert_eq!(r[0].0.as_str(), "other/Rust.md");
    assert_eq!(r[1].0.as_str(), "rust/zzz.md");
    assert_eq!(r.len(), 2);
}

#[test]
fn alias_match_reported() {
    let mut c = cache_of(&[("Accueil.md", ""), ("Other.md", "")]);
    c.insert_parsed(p("Accueil.md"), NoteBuilder::new().alias("Home").build());
    let r = fuzzy_find(&c, "home", 10);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0.as_str(), "Accueil.md");
    assert_eq!(r[0].1.as_deref(), Some("Home"));
}

#[test]
fn attachments_only_when_requested() {
    let c = cache_of(&[("pic.png", ""), ("picnic.md", "")]);
    assert_eq!(fuzzy_find(&c, "pic", 10).len(), 1);
    let r = fuzzy_find_with(
        &c,
        "pic.png",
        10,
        FuzzyOptions {
            include_attachments: true,
        },
    );
    assert_eq!(r[0].0.as_str(), "pic.png");
}

#[test]
fn limit_and_no_match() {
    let c = cache_of(&[("aa.md", ""), ("ab.md", ""), ("ac.md", "")]);
    assert_eq!(fuzzy_find(&c, "a", 2).len(), 2);
    assert!(fuzzy_find(&c, "zz", 10).is_empty());
}
