//! Tests of `oxi_markdown::properties`: source-preserving frontmatter edits.

#![allow(clippy::unwrap_used)]

use oxi_markdown::properties::*;
use serde_json::{Value, json};

fn props(src: &str) -> Value {
    Value::Object(get_properties(src).expect("valid frontmatter"))
}

fn tags(src: &str) -> Vec<String> {
    oxi_markdown::parse(src)
        .tags
        .into_iter()
        .map(|t| t.name)
        .collect()
}

// ------------------------------------------------------------ set_property

#[test]
fn creates_frontmatter_when_missing() {
    let src = "# Title\n\nBody\n";
    let out = set_property(src, "status", &json!("draft")).unwrap();
    assert_eq!(out, "---\nstatus: draft\n---\n# Title\n\nBody\n");
    assert_eq!(props(&out), json!({"status": "draft"}));
}

#[test]
fn creates_frontmatter_in_empty_and_crlf_and_bom_notes() {
    assert_eq!(
        set_property("", "a", &json!(1)).unwrap(),
        "---\na: 1\n---\n"
    );
    assert_eq!(
        set_property("Body\r\nMore\r\n", "a", &json!(true)).unwrap(),
        "---\r\na: true\r\n---\r\nBody\r\nMore\r\n"
    );
    let out = set_property("\u{FEFF}Body\n", "a", &json!("x")).unwrap();
    assert_eq!(out, "\u{FEFF}---\na: x\n---\nBody\n");
    assert_eq!(props(&out), json!({"a": "x"}));
}

#[test]
fn appends_new_key_and_keeps_other_lines() {
    let src = "---\n# a comment\ntitle: Hello   # trailing\nlist: [a, b]\n---\nBody #tag\n";
    let out = set_property(src, "status", &json!("done")).unwrap();
    assert_eq!(
        out,
        "---\n# a comment\ntitle: Hello   # trailing\nlist: [a, b]\nstatus: done\n---\nBody #tag\n"
    );
}

#[test]
fn replaces_whole_entry_including_continuations() {
    let src = "---\nfirst: 1\ntags:\n  - a\n  - b\n# keep me\n\nlast: z\n---\nbody\n";
    let out = set_property(src, "tags", &json!(["x"])).unwrap();
    assert_eq!(
        out,
        "---\nfirst: 1\ntags:\n  - x\n# keep me\n\nlast: z\n---\nbody\n"
    );
    // Column-0 list items are continuation lines too.
    let src = "---\ntags:\n- a\n- b\nnext: 1\n---\n";
    let out = set_property(src, "tags", &json!("solo")).unwrap();
    assert_eq!(out, "---\ntags: solo\nnext: 1\n---\n");
    // Block scalars with inner blank lines.
    let src = "---\ndesc: |\n  line 1\n\n  line 3\nafter: x\n---\n";
    let out = set_property(src, "desc", &json!("short")).unwrap();
    assert_eq!(out, "---\ndesc: short\nafter: x\n---\n");
}

#[test]
fn crlf_frontmatter_stays_crlf() {
    let src = "---\r\na: 1\r\nb:\r\n  - x\r\n---\r\nBody\r\n";
    let out = set_property(src, "b", &json!(["y", "z"])).unwrap();
    assert_eq!(
        out,
        "---\r\na: 1\r\nb:\r\n  - y\r\n  - z\r\n---\r\nBody\r\n"
    );
    let out = set_property(&out, "c", &json!(null)).unwrap();
    assert_eq!(
        out,
        "---\r\na: 1\r\nb:\r\n  - y\r\n  - z\r\nc:\r\n---\r\nBody\r\n"
    );
    assert_eq!(props(&out), json!({"a": 1, "b": ["y", "z"], "c": null}));
}

#[test]
fn empty_frontmatter_block() {
    let out = set_property("---\n---\nBody\n", "a", &json!("b")).unwrap();
    assert_eq!(out, "---\na: b\n---\nBody\n");
}

#[test]
fn serialization_rules() {
    let opts = WriteOptions::default();
    let f = |v: Value| format_entry("k", &v, "\n", &opts);
    assert_eq!(f(json!("plain text")), "k: plain text\n");
    assert_eq!(f(json!("a: b")), "k: \"a: b\"\n");
    assert_eq!(f(json!("[[Note]]")), "k: \"[[Note]]\"\n");
    assert_eq!(f(json!("#tag")), "k: \"#tag\"\n");
    assert_eq!(f(json!("- x")), "k: \"- x\"\n");
    assert_eq!(f(json!("42")), "k: \"42\"\n");
    assert_eq!(f(json!("true")), "k: \"true\"\n");
    assert_eq!(f(json!("null")), "k: \"null\"\n");
    assert_eq!(f(json!("~")), "k: \"~\"\n");
    assert_eq!(f(json!("")), "k: \"\"\n");
    assert_eq!(f(json!(" padded")), "k: \" padded\"\n");
    assert_eq!(f(json!("two\nlines")), "k: \"two\\nlines\"\n");
    assert_eq!(f(json!("say \"hi\"")), "k: say \"hi\"\n");
    assert_eq!(f(json!("a #b")), "k: \"a #b\"\n");
    assert_eq!(f(json!("https://x.org/a#b")), "k: https://x.org/a#b\n");
    assert_eq!(f(json!("2026-10-20")), "k: 2026-10-20\n");
    assert_eq!(f(json!(3)), "k: 3\n");
    assert_eq!(f(json!(-2.5)), "k: -2.5\n");
    assert_eq!(f(json!(false)), "k: false\n");
    assert_eq!(f(json!(null)), "k:\n");
    assert_eq!(f(json!([])), "k:\n");
    assert_eq!(
        f(json!(["a", 1, true, null])),
        "k:\n  - a\n  - 1\n  - true\n  -\n"
    );
    assert_eq!(
        f(json!(["[[A]]", "x: y"])),
        "k:\n  - \"[[A]]\"\n  - \"x: y\"\n"
    );
    assert_eq!(
        f(json!([[1, 2], {"a": "b"}])),
        "k:\n  - [1, 2]\n  - {\"a\": \"b\"}\n"
    );
    assert_eq!(f(json!({})), "k: {}\n");
    assert_eq!(
        f(json!({"inner": {"x": 1}, "list": ["a"], "e": []})),
        "k:\n  e: []\n  inner:\n    x: 1\n  list:\n    - a\n"
    );
    let quoted = WriteOptions {
        quote_date_strings: true,
    };
    assert_eq!(
        format_entry("d", &json!("2026-10-20"), "\n", &quoted),
        "d: \"2026-10-20\"\n"
    );
    assert_eq!(
        format_entry("d", &json!("2026-10-20T10:30"), "\n", &quoted),
        "d: \"2026-10-20T10:30\"\n"
    );
    // Keys.
    assert_eq!(format_key("simple key"), "simple key");
    assert_eq!(format_key("a: b"), "\"a: b\"");
    assert_eq!(format_key("#x"), "\"#x\"");
    // Scalar-looking keys are read back as the same string key.
    assert_eq!(format_key("true"), "true");
    assert_eq!(format_key("12"), "12");
}

#[test]
fn roundtrip_values() {
    let values = [
        json!("plain"),
        json!("needs: quotes"),
        json!("[[Link|alias]]"),
        json!("ünïcödé ✓"),
        json!("tab\there"),
        json!("back\\slash \"q\""),
        json!("'single'"),
        json!("@at"),
        json!("%pct"),
        json!("0x1F"),
        json!("1e3"),
        json!("yes"),
        json!("NO"),
        json!(".inf"),
        json!(12),
        json!(-7),
        json!(1.5),
        json!(true),
        json!(["a", "b c", "[[X]]", "12", 3]),
        json!({"nested": {"deep": [1, "two"]}, "flag": false}),
        json!([{"a": [1, {"b": null}]}]),
    ];
    for v in values {
        let out = set_property("---\nother: 1\n---\nBody\n", "key", &v).unwrap();
        let p = props(&out);
        assert_eq!(p["key"], v, "value {v} written as:\n{out}");
        assert_eq!(p["other"], json!(1));
        assert!(out.ends_with("---\nBody\n"));
        // Replace again (entry replacement path).
        let again = set_property(&out, "key", &v).unwrap();
        assert_eq!(again, out, "replacing with the same value is stable");
    }
}

#[test]
fn weird_keys_roundtrip() {
    for key in ["with space", "a: b", "#hash", "123", "true", "ключ", "x\"y"] {
        let out = set_property("", key, &json!("v")).unwrap();
        assert_eq!(props(&out)[key], json!("v"), "{key}: {out}");
        assert_eq!(property_order(&out), vec![key.to_string()]);
        let replaced = set_property(&out, key, &json!("w")).unwrap();
        assert_eq!(props(&replaced)[key], json!("w"), "{replaced}");
        let removed = remove_property(&replaced, key).unwrap();
        assert_eq!(removed, "");
    }
}

#[test]
fn quoted_key_in_file_is_found() {
    let src = "---\n\"my key\": 1\n'other': 2\n---\n";
    let out = set_property(src, "my key", &json!(5)).unwrap();
    assert_eq!(out, "---\nmy key: 5\n'other': 2\n---\n");
    let out = set_property(&out, "other", &json!(6)).unwrap();
    assert_eq!(out, "---\nmy key: 5\nother: 6\n---\n");
}

#[test]
fn case_insensitive_key_match_keeps_spelling() {
    let src = "---\nStatus: draft\n---\n";
    let out = set_property(src, "status", &json!("done")).unwrap();
    assert_eq!(out, "---\nStatus: done\n---\n");
}

#[test]
fn invalid_yaml_is_refused() {
    let src = "---\nkey: [unclosed\nother: : :\n---\nBody\n";
    assert_eq!(frontmatter_status(src), FrontmatterStatus::Invalid);
    assert!(matches!(
        set_property(src, "a", &json!(1)),
        Err(oxi_core::Error::Parse(_))
    ));
    assert!(remove_property(src, "key").is_err());
    assert!(rename_property(src, "key", "k2").is_err());
    assert!(add_tag(src, "x").is_err());
    assert!(add_alias(src, "x").is_err());
    // Not a mapping.
    assert!(set_property("---\n- a\n- b\n---\n", "a", &json!(1)).is_err());
    assert!(get_properties(src).is_none());
    // Order is still available for display.
    assert_eq!(property_order(src), vec!["key", "other"]);
}

#[test]
fn empty_key_rejected() {
    assert!(set_property("", " ", &json!(1)).is_err());
    assert!(rename_property("---\na: 1\n---\n", "a", "").is_err());
}

#[test]
fn status_and_order() {
    assert_eq!(frontmatter_status("hello"), FrontmatterStatus::Missing);
    assert_eq!(frontmatter_status("---\n---\n"), FrontmatterStatus::Valid);
    let src = "---\nzeta: 1\nalpha:\n  - x\n\"mid\": true\n---\n";
    assert_eq!(property_order(src), vec!["zeta", "alpha", "mid"]);
    let ordered = ordered_properties(src).unwrap();
    assert_eq!(
        ordered,
        vec![
            ("zeta".to_string(), json!(1)),
            ("alpha".to_string(), json!(["x"])),
            ("mid".to_string(), json!(true)),
        ]
    );
    assert!(property_order("no fm").is_empty());
    assert!(ordered_properties("no fm").is_none());
}

#[test]
fn multiline_quoted_scalar_does_not_create_fake_keys() {
    let src = "---\ndesc: \"first\n  text: inside\"\nnext: 1\n---\n";
    assert_eq!(props(src)["desc"], json!("first text: inside"));
    assert_eq!(property_order(src), vec!["desc", "next"]);
    let out = set_property(src, "desc", &json!("x")).unwrap();
    assert_eq!(out, "---\ndesc: x\nnext: 1\n---\n");
}

// --------------------------------------------------------- remove / rename

#[test]
fn remove_property_keeps_rest() {
    let src = "---\na: 1\nb:\n  - x\n  - y\n# c\nc: 3\n---\nBody\n";
    let out = remove_property(src, "b").unwrap();
    assert_eq!(out, "---\na: 1\n# c\nc: 3\n---\nBody\n");
    assert_eq!(remove_property(&out, "nope").unwrap(), out);
    assert_eq!(remove_property("Body\n", "a").unwrap(), "Body\n");
}

#[test]
fn removing_last_property_removes_block() {
    assert_eq!(
        remove_property("---\na: 1\n---\nBody\n", "a").unwrap(),
        "Body\n"
    );
    assert_eq!(
        remove_property("---\r\na: 1\r\n\r\n---\r\nBody\r\n", "a").unwrap(),
        "Body\r\n"
    );
    // A remaining comment keeps the block.
    assert_eq!(
        remove_property("---\n# note\na: 1\n---\nBody\n", "a").unwrap(),
        "---\n# note\n---\nBody\n"
    );
}

#[test]
fn rename_property_keeps_value_verbatim() {
    let src = "---\nold:   [a,   b]   # c\nz: 1\n---\n";
    let out = rename_property(src, "old", "new name").unwrap();
    assert_eq!(out, "---\nnew name:   [a,   b]   # c\nz: 1\n---\n");
    assert_eq!(props(&out)["new name"], json!(["a", "b"]));
    // Case-only rename.
    let out = rename_property("---\nstatus: x\n---\n", "status", "Status").unwrap();
    assert_eq!(out, "---\nStatus: x\n---\n");
    // Needing quotes.
    let out = rename_property("---\na: 1\n---\n", "a", "b: c").unwrap();
    assert_eq!(out, "---\n\"b: c\": 1\n---\n");
    // Conflict.
    assert!(rename_property("---\na: 1\nb: 2\n---\n", "a", "b").is_err());
    // Missing old key.
    assert_eq!(
        rename_property("---\na: 1\n---\n", "x", "y").unwrap(),
        "---\na: 1\n---\n"
    );
}

// ------------------------------------------------------------------- tags

#[test]
fn add_tag_creates_and_normalizes() {
    let out = add_tag("Body\n", "#project").unwrap();
    assert_eq!(out, "---\ntags:\n  - project\n---\nBody\n");
    let out = add_tag(&out, "Project").unwrap();
    assert_eq!(out, "---\ntags:\n  - project\n---\nBody\n", "no duplicates");
    let out = add_tag("---\ntags: a, b\n---\n", "c").unwrap();
    assert_eq!(out, "---\ntags:\n  - a\n  - b\n  - c\n---\n");
    let out = add_tag("---\ntags: [hub]\ntitle: x\n---\n", "new/child").unwrap();
    assert_eq!(out, "---\ntags:\n  - hub\n  - new/child\ntitle: x\n---\n");
    let out = add_tag("---\ntag: single\n---\n", "two").unwrap();
    assert_eq!(out, "---\ntag:\n  - single\n  - two\n---\n");
    assert!(add_tag("", "not valid").is_err());
    assert!(add_tag("", "123").is_err());
    assert!(add_tag("", "#").is_err());
}

#[test]
fn remove_tag_frontmatter_and_inline() {
    let src = "---\ntags:\n  - a\n  - b\n---\nText #a and #b, end #a\n#a #c\n`#a` code\n";
    let out = remove_tag(src, "#A").unwrap();
    assert_eq!(
        out,
        "---\ntags:\n  - b\n---\nText and #b, end\n#c\n`#a` code\n"
    );
    // Removing the last frontmatter tag removes the property (and block).
    let out = remove_tag("---\ntags: [x]\n---\nBody #x\n", "x").unwrap();
    assert_eq!(out, "Body\n");
    // Children are separate tags.
    let out = remove_tag("Body #a/b #a\n", "a").unwrap();
    assert_eq!(out, "Body #a/b\n");
    // Adjacent duplicates.
    assert_eq!(remove_tag("#a #a x\n", "a").unwrap(), "x\n");
    // Invalid YAML: inline only.
    let bad = "---\ntags: [a\n---\nx #a y\n";
    assert_eq!(remove_tag(bad, "a").unwrap(), "---\ntags: [a\n---\nx y\n");
    // Code blocks and comments are untouched.
    let src = "```\n#a\n```\n%% #a %%\n<!-- #a -->\n";
    assert_eq!(remove_tag(src, "a").unwrap(), src);
}

#[test]
fn rename_tag_inline_and_frontmatter_with_children() {
    let src = "---\ntags:\n  - Old\n  - old/child\n  - other\n---\n# T\n#old text #OLD/Sub/deep #older #x/old\n`#old` and [l](https://x.org/#old)\n";
    let out = rename_tag(src, "old", "new").unwrap();
    assert_eq!(
        out,
        "---\ntags:\n  - new\n  - new/child\n  - other\n---\n# T\n#new text #new/Sub/deep #older #x/old\n`#old` and [l](https://x.org/#old)\n"
    );
    assert_eq!(
        tags(&out),
        vec![
            "new",
            "new/child",
            "other",
            "new",
            "new/Sub/deep",
            "older",
            "x/old"
        ]
    );
}

#[test]
fn rename_tag_preserves_flow_and_string_forms() {
    let out = rename_tag("---\ntags: [a, b]  # c\n---\n", "a", "z").unwrap();
    assert_eq!(out, "---\ntags: [z, b]  # c\n---\n");
    let out = rename_tag("---\ntags: a, b\n---\n", "b", "y/w").unwrap();
    assert_eq!(out, "---\ntags: a, y/w\n---\n");
    let out = rename_tag("---\ntags:\n  - \"#a\"\n---\n", "a", "q").unwrap();
    assert_eq!(out, "---\ntags:\n  - \"#q\"\n---\n");
}

#[test]
fn rename_tag_merge_dedupes() {
    let src = "---\ntags:\n  - a\n  - b\n  - c\n---\n#a #b\n";
    let out = rename_tag(src, "a", "B").unwrap();
    assert_eq!(out, "---\ntags:\n  - B\n  - c\n---\n#B #b\n");
    let p = props(&out);
    assert_eq!(p["tags"], json!(["B", "c"]));
}

#[test]
fn rename_tag_falls_back_when_inplace_breaks_yaml_types() {
    // `1e5` would be read as a number if written plain.
    let out = rename_tag("---\ntags: [abc]\n---\n", "abc", "1e5").unwrap();
    assert_eq!(props(&out)["tags"], json!(["1e5"]));
}

#[test]
fn rename_tag_errors_and_noop() {
    assert!(rename_tag("x", "", "a").is_err());
    assert!(rename_tag("x", "a", "bad name").is_err());
    let src = "no tags here #other\n";
    assert_eq!(rename_tag(src, "a", "b").unwrap(), src);
    // CRLF and invalid YAML: inline only.
    let bad = "---\r\ntags: [a\r\n---\r\n#a\r\n";
    assert_eq!(
        rename_tag(bad, "a", "b").unwrap(),
        "---\r\ntags: [a\r\n---\r\n#b\r\n"
    );
}

#[test]
fn rename_tag_unicode_case() {
    let out = rename_tag("#Équipe/Été #équipe\n", "ÉQUIPE", "team").unwrap();
    assert_eq!(out, "#team/Été #team\n");
}

// ---------------------------------------------------------------- aliases

#[test]
fn aliases_helpers() {
    let out = add_alias("Body\n", "Home").unwrap();
    assert_eq!(out, "---\naliases:\n  - Home\n---\nBody\n");
    let out = add_alias(&out, "home").unwrap();
    assert_eq!(out, "---\naliases:\n  - Home\n---\nBody\n");
    let out = add_alias("---\naliases: A, B\n---\n", "C: d").unwrap();
    assert_eq!(out, "---\naliases:\n  - A\n  - B\n  - \"C: d\"\n---\n");
    let out = remove_alias(&out, "b").unwrap();
    assert_eq!(out, "---\naliases:\n  - A\n  - \"C: d\"\n---\n");
    let out = remove_alias("---\naliases: [X]\nz: 1\n---\n", "x").unwrap();
    assert_eq!(out, "---\nz: 1\n---\n");
    assert_eq!(remove_alias("Body", "x").unwrap(), "Body");
    assert!(add_alias("", "  ").is_err());
    assert_eq!(
        oxi_markdown::parse(&add_alias("", "Foo").unwrap()).aliases,
        vec!["Foo"]
    );
}

// ------------------------------------------------------------- type names

#[test]
fn inference() {
    assert_eq!(infer_type_name(&json!(["a"])), "multitext");
    assert_eq!(infer_type_name(&json!(true)), "checkbox");
    assert_eq!(infer_type_name(&json!(2.5)), "number");
    assert_eq!(infer_type_name(&json!("2026-10-20")), "date");
    assert_eq!(infer_type_name(&json!("2026-10-20T08:15")), "datetime");
    assert_eq!(infer_type_name(&json!("2026-10-20T08:15:30")), "datetime");
    assert_eq!(
        infer_type_name(&json!("2026-10-20T08:15:30.250")),
        "datetime"
    );
    assert_eq!(infer_type_name(&json!("2026-13-20")), "text");
    assert_eq!(infer_type_name(&json!("2026-10-20T25:00")), "text");
    assert_eq!(infer_type_name(&json!("hello")), "text");
    assert_eq!(infer_type_name(&json!(null)), "text");
    assert_eq!(infer_type_name(&json!({"a": 1})), "text");
}

#[test]
fn untouched_bytes_are_identical_on_sample_like_note() {
    let src = "---\naliases: [Home, Index]\ntags: [hub]\n---\n# Accueil\n\nBienvenue #hub #meta/organisation\n";
    let out = set_property(src, "status", &json!("ok")).unwrap();
    let insert_at = src.find("---\n# Accueil").unwrap();
    assert_eq!(&out[..insert_at], &src[..insert_at]);
    assert_eq!(&out[insert_at..insert_at + 11], "status: ok\n");
    assert_eq!(&out[insert_at + 11..], &src[insert_at..]);
    let p = oxi_markdown::parse(&out);
    assert_eq!(p.aliases, vec!["Home", "Index"]);
}

#[test]
fn frontmatter_tag_spans_are_whole_tokens() {
    let src = "---\ntags: [cat, a, \"#t\"]\n---\n";
    let p = oxi_markdown::parse(src);
    let found: Vec<&str> = p
        .tags
        .iter()
        .map(|t| &src[t.span.start..t.span.end])
        .collect();
    assert_eq!(found, vec!["cat", "a", "t"]);
    assert_eq!(p.tags[1].span.start, src.find("a,").unwrap());
}
