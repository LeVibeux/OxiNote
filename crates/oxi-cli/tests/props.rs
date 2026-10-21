//! End-to-end tests of `props`, `prop set|rm` and `tag rename|add|rm` on a
//! temporary copy of `examples/sample-vault`.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_oxinote");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run oxinote")
}

fn run_ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "oxinote {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn run_err(args: &[&str]) -> String {
    let out = run(args);
    assert_eq!(out.status.code(), Some(1), "oxinote {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
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
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-props-{name}"));
    let _ = fs::remove_dir_all(&dir);
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-vault");
    copy_dir(&sample, &dir);
    dir
}

fn read(vault: &Path, note: &str) -> String {
    fs::read_to_string(vault.join(note)).unwrap()
}

#[test]
fn props_lists_properties_and_values() {
    let vault = vault_copy("list");
    let v = vault.to_str().unwrap();
    let out = run_ok(&["props", v]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 5, "{out}");
    assert!(
        lines[0].starts_with("aliases") && lines[0].ends_with(" 1"),
        "{out}"
    );
    assert!(lines[1].contains("date?"), "{out}");
    assert!(lines[4].starts_with("tags") && lines[4].contains("tags ") && lines[4].ends_with(" 3"));

    let json: serde_json::Value = serde_json::from_str(&run_ok(&["props", v, "--json"])).unwrap();
    let tags = json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "tags")
        .unwrap();
    assert_eq!(tags["type"], "tags");
    assert_eq!(tags["type_source"], "types.json");
    assert_eq!(tags["count"], 3);

    let out = run_ok(&["props", v, "--values", "tags"]);
    assert_eq!(out.lines().next().unwrap(), "projet/oxinote  2", "{out}");
    assert!(out.contains("hub "));
    let json: serde_json::Value =
        serde_json::from_str(&run_ok(&["props", v, "--values", "status", "--json"])).unwrap();
    assert_eq!(json, serde_json::json!([["brouillon", 1]]));
}

#[test]
fn prop_set_and_rm() {
    let vault = vault_copy("set");
    let v = vault.to_str().unwrap();
    // Inferred number, negative value.
    let out = run_ok(&["prop", "set", v, "Idées", "priority", "-3"]);
    assert!(
        out.contains("Set priority = -3 (number) in Idées.md"),
        "{out}"
    );
    // Existing text value keeps text.
    run_ok(&["prop", "set", v, "Idées", "status", "fini"]);
    // List type.
    run_ok(&[
        "prop",
        "set",
        v,
        "Idées",
        "authors",
        "Ann, Bob",
        "--type",
        "multitext",
    ]);
    // Explicit text type quotes date-like strings and is recorded.
    run_ok(&[
        "prop",
        "set",
        v,
        "Orpheline",
        "code",
        "2026-01-01",
        "--type",
        "text",
    ]);
    // tags are always a tags list.
    run_ok(&["prop", "set", v, "Orpheline", "tags", "#a, b"]);
    let idees = read(&vault, "Idées.md");
    assert!(
        idees.starts_with(
            "---\ntags:\n  - idée\n  - projet/oxinote\nstatus: fini\nrelated: \"[[Accueil]]\"\npriority: -3\nauthors:\n  - Ann\n  - Bob\n---\n# Idées\n"
        ),
        "{idees}"
    );
    assert_eq!(
        read(&vault, "Orpheline.md"),
        "---\ncode: \"2026-01-01\"\ntags:\n  - a\n  - b\n---\nUne note sans aucun lien, mais qui parle de Rust et de Oxinote.\n"
    );
    let types: serde_json::Value =
        serde_json::from_str(&read(&vault, ".obsidian/types.json")).unwrap();
    assert_eq!(types["types"]["code"], "text");
    assert_eq!(types["types"]["authors"], "multitext");

    // Same value: no change.
    let out = run_ok(&["prop", "set", v, "Idées", "priority", "-3"]);
    assert!(out.contains("already"), "{out}");

    // Errors.
    let err = run_err(&[
        "prop", "set", v, "Idées", "priority", "x", "--type", "number",
    ]);
    assert!(err.contains("not a valid number"), "{err}");
    let err = run_err(&["prop", "set", v, "Idées", "k", "v", "--type", "fancy"]);
    assert!(err.contains("unknown type"), "{err}");
    let err = run_err(&["prop", "set", v, "Nope", "k", "v"]);
    assert!(err.contains("note not found"), "{err}");

    let out = run_ok(&["prop", "rm", v, "Idées", "related"]);
    assert!(out.contains("Removed related from Idées.md"), "{out}");
    assert!(!read(&vault, "Idées.md").contains("related"));
    let out = run_ok(&["prop", "rm", v, "Idées", "related"]);
    assert!(out.contains("has no property"), "{out}");
}

#[test]
fn prop_set_refuses_invalid_yaml() {
    let vault = vault_copy("invalid");
    let v = vault.to_str().unwrap();
    fs::write(vault.join("Bad.md"), "---\nkey: [unclosed\n---\nBody\n").unwrap();
    let err = run_err(&["prop", "set", v, "Bad", "a", "1"]);
    assert!(err.contains("not valid YAML"), "{err}");
    assert_eq!(read(&vault, "Bad.md"), "---\nkey: [unclosed\n---\nBody\n");
}

#[test]
fn tag_rename_add_rm() {
    let vault = vault_copy("tags");
    let v = vault.to_str().unwrap();
    let before = read(&vault, "Idées.md");
    let dry = run_ok(&["tag", "rename", v, "#projet", "work", "--dry-run"]);
    assert!(
        dry.contains("Would rename #projet → #work in 2 note(s):"),
        "{dry}"
    );
    assert!(dry.contains("Projets/Oxinote.md"));
    assert_eq!(read(&vault, "Idées.md"), before, "dry run writes nothing");

    let out = run_ok(&["tag", "rename", v, "projet", "work"]);
    assert!(
        out.contains("Renamed #projet → #work in 2 note(s):"),
        "{out}"
    );
    assert!(read(&vault, "Idées.md").contains("  - work/oxinote\n"));
    let obsi = read(&vault, "Projets/Oxinote.md");
    assert!(obsi.starts_with("---\ntags: work/oxinote\n"), "{obsi}");
    assert!(obsi.contains("#rust #work/oxinote"), "{obsi}");
    // Code spans untouched.
    assert!(obsi.contains("`oxi-core`"));
    let tags = run_ok(&["tags", v]);
    assert!(tags.contains("#work/oxinote"), "{tags}");
    assert!(!tags.contains("#projet"), "{tags}");

    // Merge into an existing tag dedupes the frontmatter list.
    run_ok(&["tag", "rename", v, "work/oxinote", "idée"]);
    let idees = read(&vault, "Idées.md");
    assert!(
        idees.starts_with("---\ntags:\n  - idée\nstatus:"),
        "{idees}"
    );

    let out = run_ok(&["tag", "add", v, "Orpheline", "#lonely"]);
    assert!(out.contains("Added #lonely to Orpheline.md"), "{out}");
    let out = run_ok(&["tag", "add", v, "Orpheline", "LONELY"]);
    assert!(out.contains("already has"), "{out}");
    let out = run_ok(&["tag", "rm", v, "Accueil", "hub"]);
    assert!(out.contains("Removed #hub from Accueil.md"), "{out}");
    let accueil = read(&vault, "Accueil.md");
    assert!(
        accueil.starts_with("---\naliases: [Home, Index]\n---\n"),
        "{accueil}"
    );
    assert!(accueil.ends_with("\n#meta/organisation\n"), "{accueil}");
    let out = run_ok(&["tag", "rm", v, "Accueil", "hub"]);
    assert!(out.contains("does not have"), "{out}");

    let err = run_err(&["tag", "rename", v, "rust", "not valid"]);
    assert!(err.contains("invalid tag name"), "{err}");
    let err = run_err(&["tag", "add", v, "Orpheline", "12"]);
    assert!(err.contains("invalid tag name"), "{err}");
}
