use serde_json::json;

use super::{NoteBuilder, cache_of, mini_parse, p};
use crate::{MetadataCache, SearchIndex, SearchResult};

const FILES: &[(&str, &str)] = &[
    (
        "Rust notes.md",
        "# Rust notes\n\n## Ownership\nEach value has an owner.\n\n## Borrowing\nReferences &T and &mut T.\n#rust",
    ),
    (
        "Projets/Oxinote.md",
        "# Oxinote\nPortage d'Obsidian en Rust.\n\n## Statut\n- [ ] écrire les tests\n- [x] parser rust\n- [/] recherche\n#projet/oxinote",
    ),
    (
        "Idées.md",
        "# Idées\nUne idée: vue graphe.\nAutre ligne avec graphe et rust.\n\nParagraphe séparé sur l'Été.\n#idée",
    ),
    ("Journal/2026.md", "Lu sur le borrowing.\nRien d'autre."),
];

fn setup() -> (SearchIndex, MetadataCache) {
    let mut idx = SearchIndex::new();
    let mut cache = cache_of(FILES);
    for (path, src) in FILES {
        idx.update(&p(path), src);
    }
    let mut ideas = mini_parse(FILES[2].1);
    ideas.frontmatter = NoteBuilder::new()
        .prop("status", json!("brouillon"))
        .prop("Related", json!(["Accueil", "Autre chose"]))
        .prop("count", json!(3))
        .build()
        .frontmatter;
    cache.insert_parsed(p("Idées.md"), ideas);
    (idx, cache)
}

fn search(q: &str) -> Vec<SearchResult> {
    let (idx, cache) = setup();
    idx.search(q, &cache).unwrap()
}

fn hits(q: &str) -> Vec<String> {
    let mut v: Vec<String> = search(q)
        .into_iter()
        .map(|r| r.path.as_str().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn single_term_case_insensitive_substring() {
    assert_eq!(
        hits("RUST"),
        vec!["Idées.md", "Projets/Oxinote.md", "Rust notes.md"]
    );
    assert_eq!(hits("borrow"), vec!["Journal/2026.md", "Rust notes.md"]);
}

#[test]
fn implicit_and() {
    assert_eq!(hits("rust graphe"), vec!["Idées.md"]);
    assert!(hits("rust nothing-like-this").is_empty());
}

#[test]
fn or_and_groups() {
    assert_eq!(
        hits("graphe OR borrowing"),
        vec!["Idées.md", "Journal/2026.md", "Rust notes.md"]
    );
    assert_eq!(
        hits("(graphe OR owner) rust"),
        vec!["Idées.md", "Rust notes.md"]
    );
}

#[test]
fn negation() {
    assert_eq!(
        hits("rust -graphe"),
        vec!["Projets/Oxinote.md", "Rust notes.md"]
    );
    assert_eq!(hits("rust -(graphe OR owner)"), vec!["Projets/Oxinote.md"]);
    assert_eq!(hits("rust -\"en rust\""), vec!["Idées.md", "Rust notes.md"]);
}

#[test]
fn exact_phrase() {
    assert_eq!(hits("\"vue graphe\""), vec!["Idées.md"]);
    assert!(hits("\"graphe vue\"").is_empty());
}

#[test]
fn accent_sensitive() {
    assert_eq!(hits("été"), vec!["Idées.md"]);
    assert_eq!(hits("ÉTÉ"), vec!["Idées.md"]);
    assert!(hits("ete").is_empty());
}

#[test]
fn file_and_path_operators() {
    assert_eq!(hits("file:notes"), vec!["Rust notes.md"]);
    assert_eq!(
        hits("file:note"),
        vec!["Projets/Oxinote.md", "Rust notes.md"]
    );
    assert_eq!(hits("file:\"rust notes\""), vec!["Rust notes.md"]);
    assert_eq!(hits("path:projets"), vec!["Projets/Oxinote.md"]);
    assert_eq!(hits("path:Journal borrowing"), vec!["Journal/2026.md"]);
    assert_eq!(hits("path:/^Pro/"), vec!["Projets/Oxinote.md"]);
    assert_eq!(
        hits("file:(idées OR 2026)"),
        vec!["Idées.md", "Journal/2026.md"]
    );
}

#[test]
fn file_only_match_has_no_line_matches() {
    let r = search("file:2026");
    assert_eq!(r.len(), 1);
    assert!(r[0].matches.is_empty());
    assert!(r[0].score >= 1);
}

#[test]
fn terms_match_file_name_with_bonus() {
    // "notes" only appears in the title of "Rust notes.md" (and its H1).
    let r = search("oxinote");
    assert_eq!(r[0].path.as_str(), "Projets/Oxinote.md");
    let r = search("2026");
    // Matches only the file name: still a result.
    assert_eq!(r.len(), 1);
}

#[test]
fn tag_operator_nested_and_case() {
    assert_eq!(hits("tag:#projet"), vec!["Projets/Oxinote.md"]);
    assert_eq!(hits("tag:PROJET/oxinote"), vec!["Projets/Oxinote.md"]);
    assert!(hits("tag:oxinote").is_empty());
    assert_eq!(
        hits("tag:rust OR tag:idée"),
        vec!["Idées.md", "Rust notes.md"]
    );
    assert_eq!(hits("-tag:rust graphe"), vec!["Idées.md"]);
    // Tag matches produce no line matches.
    assert!(!search("tag:rust")[0].matches.is_empty());
}

#[test]
fn line_operator() {
    assert_eq!(hits("line:(graphe rust)"), vec!["Idées.md"]);
    assert!(hits("line:(idée rust)").is_empty());
    let r = search("line:(graphe rust)");
    assert_eq!(r[0].matches.len(), 1);
    assert_eq!(r[0].matches[0].line, 2);
}

#[test]
fn section_operator() {
    // "owner" and "Ownership" share a section, "References" is elsewhere.
    assert_eq!(hits("section:(ownership owner)"), vec!["Rust notes.md"]);
    assert!(hits("section:(owner references)").is_empty());
    assert_eq!(hits("section:(statut tests)"), vec!["Projets/Oxinote.md"]);
}

#[test]
fn block_operator() {
    assert_eq!(hits("block:(idée graphe)"), vec!["Idées.md"]);
    let (idx, cache) = setup();
    let r = idx.search("block:(vue été)", &cache).unwrap();
    assert!(r.is_empty(), "{r:#?}");
}

#[test]
fn task_operators() {
    assert_eq!(hits("task:rust"), vec!["Projets/Oxinote.md"]);
    assert_eq!(hits("task-todo:tests"), vec!["Projets/Oxinote.md"]);
    assert!(hits("task-todo:parser").is_empty());
    assert_eq!(hits("task-done:parser"), vec!["Projets/Oxinote.md"]);
    assert_eq!(hits("task-done:recherche"), vec!["Projets/Oxinote.md"]);
    assert_eq!(hits("task:\"\""), vec!["Projets/Oxinote.md"]);
    // The match range covers the term only.
    let r = search("task-todo:tests");
    let m = &r[0].matches[0];
    let (s, e) = m.ranges[0];
    assert_eq!(&m.line_text[s..e], "tests");
}

#[test]
fn property_operators() {
    assert_eq!(hits("[status]"), vec!["Idées.md"]);
    assert_eq!(hits("[STATUS:brouillon]"), vec!["Idées.md"]);
    assert!(hits("[status:final]").is_empty());
    assert_eq!(hits("[related:accueil]"), vec!["Idées.md"]);
    assert_eq!(hits("[related:\"autre chose\"]"), vec!["Idées.md"]);
    assert_eq!(hits("[count:3]"), vec!["Idées.md"]);
    assert_eq!(hits("[status:(final OR brouillon)]"), vec!["Idées.md"]);
    assert!(hits("[missing]").is_empty());
}

#[test]
fn regex_search() {
    assert_eq!(
        hits("/borrow(ing)?/"),
        vec!["Journal/2026.md", "Rust notes.md"]
    );
    assert_eq!(hits("/^## /"), vec!["Projets/Oxinote.md", "Rust notes.md"]);
    let (idx, cache) = setup();
    assert!(idx.search("/(unclosed/", &cache).is_err());
}

#[test]
fn match_case_operator() {
    assert_eq!(
        hits("match-case:Rust"),
        vec!["Projets/Oxinote.md", "Rust notes.md"]
    );
    assert_eq!(
        hits("match-case:rust"),
        vec!["Idées.md", "Projets/Oxinote.md", "Rust notes.md"]
    );
    assert_eq!(hits("match-case:RUST"), Vec::<String>::new());
    assert_eq!(hits("match-case:Portage"), vec!["Projets/Oxinote.md"]);
    assert!(hits("match-case:portage").is_empty());
}

#[test]
fn content_operator_ignores_file_name() {
    assert!(hits("content:2026").is_empty());
    assert_eq!(hits("content:rien"), vec!["Journal/2026.md"]);
}

#[test]
fn match_ranges_point_at_text() {
    let r = search("borrowing");
    for res in &r {
        for m in &res.matches {
            for &(s, e) in &m.ranges {
                assert_eq!(m.line_text[s..e].to_lowercase(), "borrowing");
            }
        }
    }
    let j = r
        .iter()
        .find(|r| r.path.as_str() == "Journal/2026.md")
        .unwrap();
    assert_eq!(j.matches[0].line, 0);
    assert_eq!(j.matches[0].line_text, "Lu sur le borrowing.");
}

#[test]
fn multiple_matches_on_same_line_grouped() {
    let mut idx = SearchIndex::new();
    idx.update(&p("a.md"), "foo bar foo\nfoo");
    let r = idx.search("foo", &MetadataCache::new()).unwrap();
    assert_eq!(r[0].matches.len(), 2);
    assert_eq!(r[0].matches[0].ranges, vec![(0, 3), (8, 11)]);
    assert_eq!(r[0].score, 3);
}

#[test]
fn score_orders_results() {
    let mut idx = SearchIndex::new();
    idx.update(&p("few.md"), "cat");
    idx.update(&p("many.md"), "cat cat cat");
    idx.update(&p("cat.md"), "dog");
    let r = idx.search("cat", &MetadataCache::new()).unwrap();
    let order: Vec<&str> = r.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(order, vec!["cat.md", "many.md", "few.md"]);
}

#[test]
fn unicode_case_folding_offsets() {
    let mut idx = SearchIndex::new();
    idx.update(&p("a.md"), "İstanbul ÉCOLE école");
    let r = idx.search("école", &MetadataCache::new()).unwrap();
    let m = &r[0].matches[0];
    let texts: Vec<&str> = m.ranges.iter().map(|&(s, e)| &m.line_text[s..e]).collect();
    assert_eq!(texts, vec!["ÉCOLE", "école"]);
}

#[test]
fn long_lines_are_trimmed() {
    let mut idx = SearchIndex::new();
    let long = format!("{}needle{}", "a ".repeat(400), " b".repeat(400));
    idx.update(&p("a.md"), &long);
    let r = idx.search("needle", &MetadataCache::new()).unwrap();
    let m = &r[0].matches[0];
    assert!(m.line_text.len() <= 320);
    let (s, e) = m.ranges[0];
    assert_eq!(&m.line_text[s..e], "needle");
}

#[test]
fn errors_and_empty_query() {
    let (idx, cache) = setup();
    assert!(idx.search("\"unbalanced", &cache).is_err());
    assert!(idx.search("(a OR b", &cache).is_err());
    assert!(idx.search("a)", &cache).is_err());
    assert!(idx.search("   ", &cache).unwrap().is_empty());
}

#[test]
fn update_remove_rename() {
    let (mut idx, cache) = setup();
    idx.update(&p("Idées.md"), "plus de graphe ici");
    assert!(hits_in(&idx, &cache, "été").is_empty());
    idx.remove(&p("Journal/2026.md"));
    assert_eq!(hits_in(&idx, &cache, "borrowing"), vec!["Rust notes.md"]);
    idx.rename(&p("Rust notes.md"), &p("Archive/Old.md"));
    assert_eq!(hits_in(&idx, &cache, "borrowing"), vec!["Archive/Old.md"]);
    assert_eq!(
        hits_in(&idx, &cache, "path:archive"),
        vec!["Archive/Old.md"]
    );
    assert_eq!(idx.len(), 3);
    assert!(idx.contains(&p("Archive/Old.md")));
}

fn hits_in(idx: &SearchIndex, cache: &MetadataCache, q: &str) -> Vec<String> {
    idx.search(q, cache)
        .unwrap()
        .into_iter()
        .map(|r| r.path.as_str().to_string())
        .collect()
}

#[test]
fn candidates_do_not_lose_results() {
    // Terms with punctuation and partial tokens must still match.
    let mut idx = SearchIndex::new();
    idx.update(&p("a.md"), "see foo-bar and C++ and x.y.z");
    let c = MetadataCache::new();
    for q in ["o-b", "c++", "x.y", "\"and C+\"", "oo", "++"] {
        assert_eq!(hits_in(&idx, &c, q), vec!["a.md"], "{q}");
    }
}

#[test]
fn build_from_vault() {
    let vault = oxi_core::MemoryVault::from_files(FILES.iter().copied());
    let cache = MetadataCache::build(&vault).unwrap();
    let idx = SearchIndex::build(&vault, &cache).unwrap();
    assert_eq!(idx.len(), FILES.len());
    assert_eq!(hits_in(&idx, &cache, "borrowing").len(), 2);
}
