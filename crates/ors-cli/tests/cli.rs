//! End-to-end tests of the `obsirust` binary on `examples/sample-vault`.
//!
//! ors-markdown / ors-vault / ors-index implementations; the others only
//! exercise the CLI layer.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_obsirust");

fn sample_vault() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-vault")
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run obsirust")
}

fn run_ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "obsirust {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout is not UTF-8")
}

fn run_err(args: &[&str]) -> String {
    let out = run(args);
    assert_eq!(out.status.code(), Some(1), "obsirust {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn vault_arg() -> String {
    sample_vault().to_string_lossy().into_owned()
}

/// Fresh temporary directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// A writable copy of the sample vault.
fn vault_copy(name: &str) -> PathBuf {
    let dir = temp_dir(name).join("vault");
    copy_dir(&sample_vault(), &dir);
    dir
}

// ------------------------------------------------------------ smoke tests

#[test]
fn help_lists_subcommands() {
    let out = run_ok(&["--help"]);
    for cmd in [
        "stats",
        "search",
        "backlinks",
        "links",
        "unresolved",
        "tags",
        "orphans",
        "find",
        "graph",
        "rename",
        "daily",
        "export",
    ] {
        assert!(out.contains(cmd), "--help does not mention {cmd}:\n{out}");
    }
    let out = run_ok(&["export", "--help"]);
    assert!(out.contains("--force"));
    assert!(run_ok(&["--version"]).starts_with("obsirust "));
}

#[test]
fn usage_errors_exit_with_1() {
    let err = run_err(&["frobnicate"]);
    assert!(err.contains("unrecognized subcommand"), "{err}");
    run_err(&["stats"]);
}

#[test]
fn missing_vault_is_an_error() {
    let err = run_err(&["stats", "/definitely/not/a/vault"]);
    assert!(err.contains("vault folder not found"), "{err}");
    let file = sample_vault().join("Accueil.md");
    let err = run_err(&["orphans", file.to_str().unwrap()]);
    assert!(err.contains("not a folder"), "{err}");
}

#[test]
fn export_refuses_non_empty_out_dir() {
    let out = temp_dir("export-non-empty");
    fs::write(out.join("keep.txt"), "precious").unwrap();
    let err = run_err(&["export", &vault_arg(), out.to_str().unwrap()]);
    assert!(err.contains("not empty"), "{err}");
    assert!(err.contains("--force"), "{err}");
    assert_eq!(
        fs::read_to_string(out.join("keep.txt")).unwrap(),
        "precious"
    );
}

#[test]
fn export_refuses_vault_itself() {
    let err = run_err(&["export", &vault_arg(), &vault_arg(), "--force"]);
    assert!(err.contains("vault folder"), "{err}");
}

#[test]
fn daily_rejects_bad_dates() {
    let err = run_err(&["daily", &vault_arg(), "--date", "2026-02-30"]);
    assert!(err.contains("invalid date"), "{err}");
}

#[test]
fn graph_outputs_are_well_formed() {
    let json = run_ok(&["graph", &vault_arg(), "--layout"]);
    let v: serde_json::Value = serde_json::from_str(&json).expect("graph JSON");
    assert!(v["nodes"].is_array());
    assert!(v["edges"].is_array());
    let dot = run_ok(&["graph", &vault_arg(), "--format", "dot"]);
    assert!(dot.starts_with("digraph vault {"));
    assert!(dot.trim_end().ends_with('}'));
}

#[test]
fn stats_json_is_well_formed() {
    let v: serde_json::Value =
        serde_json::from_str(&run_ok(&["stats", &vault_arg(), "--json"])).unwrap();
    for key in [
        "notes",
        "attachments",
        "links",
        "resolved_links",
        "unresolved_links",
        "tags",
        "orphans",
        "words",
    ] {
        assert!(v[key].is_number(), "missing {key}");
    }
}

// ------------------------------------------------------------ full behaviour

#[test]
fn stats_counts_sample_vault() {
    let v: serde_json::Value =
        serde_json::from_str(&run_ok(&["stats", &vault_arg(), "--json"])).unwrap();
    assert_eq!(v["notes"], 7);
    assert_eq!(v["attachments"], 1);
    // Accueil 5, Idées 6 (frontmatter link + 2 links + 3 embeds), Journal 2,
    // ObsiRust 3, Rust notes 2. Links in comments and code are ignored.
    assert_eq!(v["links"], 18);
    assert_eq!(v["embeds"], 3);
    assert_eq!(v["unresolved_links"], 2);
    assert_eq!(v["resolved_links"], 16);
    // hub, meta/organisation, idée, projet/obsirust, rust, journal
    assert_eq!(v["tags"], 6);
    assert_eq!(v["orphans"], 2);
    assert!(v["words"].as_u64().unwrap() > 50);
    let text = run_ok(&["stats", &vault_arg()]);
    assert!(text.contains("Notes:       7"), "{text}");
    assert!(text.contains("2 unresolved"), "{text}");
}

#[test]
fn search_with_tag_and_phrase() {
    let out = run_ok(&["search", &vault_arg(), "tag:#projet \"rust\""]);
    assert!(out.contains("Projets/ObsiRust.md"), "{out}");
    assert!(out.contains("Idées.md"), "{out}");
    assert!(!out.contains("Accueil.md"), "{out}");
    assert!(!out.contains("Orpheline.md"), "{out}");
    // Matching lines are printed as path:line: text.
    assert!(
        out.lines()
            .any(|l| l.starts_with("Projets/ObsiRust.md:7: ") && l.contains("Rust")),
        "{out}"
    );
    // No ANSI codes when stdout is not a terminal.
    assert!(!out.contains('\x1b'));
    let v: serde_json::Value = serde_json::from_str(&run_ok(&[
        "search",
        &vault_arg(),
        "ObsiRust",
        "--json",
        "--limit",
        "2",
    ]))
    .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
}

#[test]
fn backlinks_by_name_and_path() {
    let out = run_ok(&["backlinks", &vault_arg(), "Rust notes"]);
    assert!(
        out.contains("Accueil.md:11: [Rust](Projets/Rust%20notes.md#Ownership)"),
        "{out}"
    );
    assert!(
        out.contains("Journal/2026-10-20.md:4: [[Rust notes#Borrowing|borrowing]]"),
        "{out}"
    );
    assert!(
        out.contains("Projets/ObsiRust.md:7: [[Rust notes]]"),
        "{out}"
    );
    let same = run_ok(&["backlinks", &vault_arg(), "Projets/Rust notes.md"]);
    assert_eq!(out, same);
    let out = run_ok(&["backlinks", &vault_arg(), "Idées"]);
    assert!(out.contains("Accueil.md:"), "{out}");
    assert!(out.contains("Projets/ObsiRust.md:"), "{out}");
    // Links in comments do not count.
    let err = run_err(&["backlinks", &vault_arg(), "Lien caché"]);
    assert!(err.contains("note not found"), "{err}");
}

#[test]
fn outgoing_links_with_resolution() {
    let out = run_ok(&["links", &vault_arg(), "Accueil"]);
    assert!(
        out.contains("✓ [[Projets/ObsiRust|le projet]] → Projets/ObsiRust.md"),
        "{out}"
    );
    assert!(out.contains("✓ [[Idées]] → Idées.md"), "{out}");
    assert!(
        out.contains("✓ [[2026-10-20]] → Journal/2026-10-20.md"),
        "{out}"
    );
    assert!(out.contains("✗ [[Note fantôme]]"), "{out}");
    assert!(out.contains("→ Projets/Rust notes.md"), "{out}");
    assert!(
        !out.contains("docs.rs"),
        "external links are not internal links"
    );
    assert_eq!(out.lines().count(), 5, "{out}");
}

#[test]
fn unresolved_and_orphans() {
    let out = run_ok(&["unresolved", &vault_arg()]);
    assert_eq!(out, "Canvas ← Idées.md\nNote fantôme ← Accueil.md\n");
    let out = run_ok(&["orphans", &vault_arg()]);
    assert_eq!(out, "Orpheline.md\nTemplates/Daily.md\n");
}

#[test]
fn tags_flat_and_tree() {
    let out = run_ok(&["tags", &vault_arg()]);
    for tag in [
        "#hub",
        "#idée",
        "#journal",
        "#meta/organisation",
        "#projet/obsirust",
        "#rust",
    ] {
        assert!(out.contains(tag), "missing {tag}:\n{out}");
    }
    assert!(!out.contains("#pas-un-tag"));
    let tree = run_ok(&["tags", &vault_arg(), "--tree"]);
    assert!(tree.contains("#projet ("), "{tree}");
    assert!(tree.contains("\n  obsirust ("), "{tree}");
    assert!(tree.contains("\n  organisation (1)"), "{tree}");
}

#[test]
fn fuzzy_find_names_and_aliases() {
    let out = run_ok(&["find", &vault_arg(), "rust no"]);
    assert_eq!(out.lines().next(), Some("Projets/Rust notes.md"), "{out}");
    let out = run_ok(&["find", &vault_arg(), "Home"]);
    assert!(out.starts_with("Accueil.md  (alias: Home)"), "{out}");
}

#[test]
fn graph_json_and_local_graph() {
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["graph", &vault_arg()])).unwrap();
    let nodes = v["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 7, "7 notes (orphans included)");
    let index = |id: &str| nodes.iter().position(|n| n["id"] == id).unwrap();
    let (accueil, idees) = (index("Accueil.md"), index("Idées.md"));
    assert!(
        v["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == accueil && e["target"] == idees)
    );
    let v: serde_json::Value = serde_json::from_str(&run_ok(&[
        "graph",
        &vault_arg(),
        "--attachments",
        "--unresolved",
        "--tags",
        "--layout",
    ]))
    .unwrap();
    let nodes = v["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["kind"] == "attachment"));
    assert!(
        nodes
            .iter()
            .any(|n| n["kind"] == "unresolved" && n["label"] == "Note fantôme")
    );
    assert!(nodes.iter().any(|n| n["kind"] == "tag"));
    assert!(
        nodes
            .iter()
            .all(|n| n["x"].is_number() && n["y"].is_number())
    );

    let v: serde_json::Value =
        serde_json::from_str(&run_ok(&["graph", &vault_arg(), "--local", "Orpheline"])).unwrap();
    assert_eq!(v["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(v["nodes"][0]["id"], "Orpheline.md");
}

#[test]
fn rename_updates_links() {
    let vault = vault_copy("rename");
    let v = vault.to_str().unwrap();
    let dry = run_ok(&["rename", v, "Rust notes", "Projets/Rust", "--dry-run"]);
    assert!(
        dry.contains("Would rename Projets/Rust notes.md → Projets/Rust.md"),
        "{dry}"
    );
    assert!(dry.contains("Journal/2026-10-20.md"), "{dry}");
    assert!(
        vault.join("Projets/Rust notes.md").exists(),
        "dry run must not touch files"
    );

    let out = run_ok(&["rename", v, "Rust notes", "Projets/Rust"]);
    assert!(
        out.contains("Renamed Projets/Rust notes.md → Projets/Rust.md"),
        "{out}"
    );
    assert!(!vault.join("Projets/Rust notes.md").exists());
    assert!(vault.join("Projets/Rust.md").exists());
    let journal = fs::read_to_string(vault.join("Journal/2026-10-20.md")).unwrap();
    assert!(
        journal.contains("[[Rust#Borrowing|borrowing]]"),
        "{journal}"
    );
    let accueil = fs::read_to_string(vault.join("Accueil.md")).unwrap();
    assert!(accueil.contains("(Projets/Rust.md#Ownership)"), "{accueil}");
    let err = run_err(&["rename", v, "Accueil", "Idées"]);
    assert!(err.contains("already exists"), "{err}");
}

#[test]
fn daily_note_is_created_from_template() {
    let vault = vault_copy("daily");
    let v = vault.to_str().unwrap();
    let out = run_ok(&["daily", v, "--date", "2026-10-20"]);
    assert!(out.trim_end().ends_with("Journal/2026-10-20.md"), "{out}");
    let existing = fs::read_to_string(vault.join("Journal/2026-10-20.md")).unwrap();
    assert!(
        existing.contains("Travaillé sur"),
        "existing note must be kept"
    );

    let out = run_ok(&["daily", v, "--date", "2026-01-02"]);
    assert!(out.trim_end().ends_with("Journal/2026-01-02.md"), "{out}");
    let created = fs::read_to_string(vault.join("Journal/2026-01-02.md")).unwrap();
    assert!(created.starts_with("# 2026-01-02"), "{created}");
    assert!(
        !created.contains("{{"),
        "template must be expanded: {created}"
    );
}

#[test]
fn export_static_site() {
    let dir = temp_dir("export");
    let out = dir.join("site");
    let stdout = run_ok(&["export", &vault_arg(), out.to_str().unwrap()]);
    assert!(
        stdout.contains("Exported 7 notes and 1 attachments"),
        "{stdout}"
    );

    for f in [
        "index.html",
        "style.css",
        "Accueil.html",
        "Idées.html",
        "Projets/ObsiRust.html",
        "Projets/Rust notes.html",
        "Journal/2026-10-20.html",
        "attachments/diagramme.png",
    ] {
        assert!(out.join(f).exists(), "missing {f}");
    }
    assert_eq!(
        fs::read(out.join("attachments/diagramme.png")).unwrap(),
        fs::read(sample_vault().join("attachments/diagramme.png")).unwrap()
    );

    let accueil = fs::read_to_string(out.join("Accueil.html")).unwrap();
    assert!(accueil.contains("<title>Accueil - sample-vault</title>"));
    assert!(accueil.contains("href=\"style.css\""));
    assert!(
        accueil.contains("href=\"Projets/ObsiRust.html\""),
        "wikilink → relative .html"
    );
    assert!(accueil.contains("is-unresolved"), "unresolved link styled");
    assert!(accueil.contains("class=\"tag\""));
    assert!(accueil.contains("<h2>Backlinks</h2>"));
    assert!(accueil.contains("Id%C3%A9es.html"), "backlink from Idées");

    let obsirust = fs::read_to_string(out.join("Projets/ObsiRust.html")).unwrap();
    assert!(obsirust.contains("href=\"../style.css\""));
    assert!(
        obsirust.contains("href=\"../Projets/Rust%20notes.html\""),
        "{obsirust}"
    );
    assert!(
        obsirust.contains("href=\"../Accueil.html\""),
        "backlinks are relative"
    );

    let idees = fs::read_to_string(out.join("Idées.html")).unwrap();
    assert!(idees.contains("class=\"callout\""));
    assert!(idees.contains("data-callout=\"tip\""));
    // Embedded heading section of another note is inlined, not the rest of it.
    assert!(
        idees.contains("Workspace Cargo avec sept crates"),
        "{idees}"
    );
    assert!(
        !idees.contains("Mentionne ObsiRust sans lien"),
        "only the section"
    );
    // Embedded block of the same note.
    assert!(idees.contains("markdown-embed"));
    assert!(idees.contains("diagramme.png"));
    assert!(!idees.contains("Lien caché"), "comments are not exported");

    let index = fs::read_to_string(out.join("index.html")).unwrap();
    assert!(index.contains("<summary>Projets</summary>"));
    assert!(index.contains("href=\"Projets/Rust%20notes.html\""));

    // A second export needs --force.
    run_err(&["export", &vault_arg(), out.to_str().unwrap()]);
    run_ok(&["export", &vault_arg(), out.to_str().unwrap(), "--force"]);
}
