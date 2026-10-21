#![allow(clippy::unwrap_used)]
use oxi_core::{LinkKind, ParsedNote, Subpath};
use oxi_markdown::parse;

fn targets(p: &ParsedNote) -> Vec<&str> {
    p.links.iter().map(|l| l.target.as_str()).collect()
}

fn tag_names(p: &ParsedNote) -> Vec<&str> {
    p.tags.iter().map(|t| t.name.as_str()).collect()
}

fn slice(src: &str, span: oxi_core::Span) -> &str {
    &src[span.start..span.end]
}

// ---------------------------------------------------------------- frontmatter

#[test]
fn empty_file() {
    let p = parse("");
    assert_eq!(p, ParsedNote::default());
}

#[test]
fn frontmatter_basic() {
    let src = "---\ntitle: Hello\ncount: 3\n---\nBody";
    let p = parse(src);
    let fm = p.frontmatter.unwrap();
    assert_eq!(fm["title"], "Hello");
    assert_eq!(fm["count"], 3);
    assert_eq!(p.frontmatter_span.unwrap().start, 0);
    assert_eq!(
        slice(src, p.frontmatter_span.unwrap()),
        "---\ntitle: Hello\ncount: 3\n---\n"
    );
    assert_eq!(p.word_count, 1);
}

#[test]
fn frontmatter_crlf() {
    let src = "---\r\ntags: [a, b]\r\naliases: X\r\n---\r\n# Title\r\nText #c\r\n";
    let p = parse(src);
    assert!(p.frontmatter.is_some());
    assert_eq!(tag_names(&p), ["a", "b", "c"]);
    assert_eq!(p.aliases, ["X"]);
    assert_eq!(p.headings[0].text, "Title");
    assert_eq!(p.headings[0].line, 4);
    assert_eq!(slice(src, p.headings[0].span), "# Title");
    assert_eq!(p.tags[2].line, 5);
}

#[test]
fn frontmatter_not_at_start() {
    let p = parse("\n---\ntags: x\n---\n");
    assert!(p.frontmatter.is_none());
    assert!(p.frontmatter_span.is_none());
}

#[test]
fn frontmatter_unclosed_is_body() {
    let p = parse("---\ntags: x\n#body");
    assert!(p.frontmatter.is_none());
    assert_eq!(tag_names(&p), ["body"]);
}

#[test]
fn frontmatter_invalid_yaml_skipped() {
    let src = "---\nkey: [unclosed\n: : :\n  - #nope\n---\n#real";
    let p = parse(src);
    assert!(p.frontmatter.is_none());
    assert!(p.frontmatter_span.is_some());
    assert_eq!(tag_names(&p), ["real"]);
}

#[test]
fn frontmatter_empty() {
    let p = parse("---\n---\nHi");
    assert_eq!(p.frontmatter, Some(Default::default()));
    assert_eq!(p.word_count, 1);
}

#[test]
fn frontmatter_non_mapping() {
    let p = parse("---\n- a\n- b\n---\nHi");
    assert!(p.frontmatter.is_none());
    assert!(p.frontmatter_span.is_some());
}

#[test]
fn frontmatter_types_converted() {
    let p = parse(
        "---\nb: true\nf: 1.5\nn: null\nd: 2026-10-20\nlist:\n  - 1\n  - two\nobj:\n  k: v\n1: numkey\n---\n",
    );
    let fm = p.frontmatter.unwrap();
    assert_eq!(fm["b"], true);
    assert_eq!(fm["f"], 1.5);
    assert!(fm["n"].is_null());
    assert_eq!(fm["d"], "2026-10-20");
    assert_eq!(fm["list"], serde_json::json!([1, "two"]));
    assert_eq!(fm["obj"]["k"], "v");
    assert_eq!(fm["1"], "numkey");
}

#[test]
fn aliases_forms() {
    assert_eq!(parse("---\naliases: [A, B]\n---\n").aliases, ["A", "B"]);
    assert_eq!(
        parse("---\naliases:\n  - A\n  - B c\n---\n").aliases,
        ["A", "B c"]
    );
    assert_eq!(parse("---\nalias: A, B\n---\n").aliases, ["A", "B"]);
    assert_eq!(parse("---\naliases: Solo\n---\n").aliases, ["Solo"]);
    assert!(parse("---\naliases:\n---\n").aliases.is_empty());
}

#[test]
fn frontmatter_tags_forms() {
    let p = parse("---\ntags: \"one, #two three\"\n---\n");
    assert_eq!(tag_names(&p), ["one", "two", "three"]);
    assert!(p.tags.iter().all(|t| t.from_frontmatter));
    let p = parse("---\ntag:\n  - '#x/y'\n  - 123\n  - z\n---\n");
    assert_eq!(tag_names(&p), ["x/y", "z"]);
    let p = parse("---\nTags: [Up]\n---\n");
    assert_eq!(tag_names(&p), ["Up"]);
}

#[test]
fn frontmatter_tag_spans() {
    let src = "---\ntitle: hub\ntags:\n  - hub\n  - other\n---\n";
    let p = parse(src);
    assert_eq!(slice(src, p.tags[0].span), "hub");
    assert_eq!(p.tags[0].line, 3);
    assert_eq!(slice(src, p.tags[1].span), "other");
    assert_eq!(p.tags[1].line, 4);
}

#[test]
fn frontmatter_links() {
    let src = "---\nrelated: \"[[Accueil]]\"\nsee:\n  - \"[[A#H|alias]]\"\n  - plain\n  - \"[[B]]\"\nnested: [[C]]\ntext: \"not [[a link]] here\"\n---\nbody [[D]]";
    let p = parse(src);
    assert_eq!(targets(&p), ["Accueil", "A", "B", "C", "D"]);
    for l in &p.links[..4] {
        assert_eq!(l.kind, LinkKind::Frontmatter);
        assert!(slice(src, l.span).starts_with("[["));
    }
    assert_eq!(p.links[1].display.as_deref(), Some("alias"));
    assert_eq!(p.links[1].subpath, Some(Subpath::Heading(vec!["H".into()])));
    assert_eq!(p.links[0].line, 1);
    assert_eq!(p.links[4].kind, LinkKind::Wiki);
}

#[test]
fn bom_is_tolerated() {
    let p = parse("\u{FEFF}---\na: 1\n---\n#t");
    assert!(p.frontmatter.is_some());
    assert_eq!(tag_names(&p), ["t"]);
    let p = parse("\u{FEFF}#t");
    assert_eq!(tag_names(&p), ["t"]);
}

// ------------------------------------------------------------------ headings

#[test]
fn headings_atx_and_setext() {
    let src = "# One\n\nTwo\n===\n\n### Three **bold** `code` ###\n\nFour\n----\n";
    let p = parse(src);
    let h: Vec<(u8, &str)> = p
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str()))
        .collect();
    assert_eq!(
        h,
        [(1, "One"), (1, "Two"), (3, "Three bold code"), (2, "Four")]
    );
    assert_eq!(p.headings[1].line, 2);
    assert_eq!(slice(src, p.headings[1].span), "Two\n===");
}

#[test]
fn heading_with_links_and_highlight() {
    let p = parse("## See [[Note|the note]] and [[Other]] ==now== ^hid");
    assert_eq!(p.headings[0].text, "See the note and Other now");
    assert_eq!(targets(&p), ["Note", "Other"]);
}

#[test]
fn heading_vs_tag_at_line_start() {
    let p = parse("# Heading\n#tag at start\n##notheading");
    assert_eq!(p.headings.len(), 1);
    assert_eq!(tag_names(&p), ["tag"]);
}

#[test]
fn heading_with_comment() {
    let p = parse("# Title %%hidden%% end");
    assert_eq!(p.headings[0].text, "Title end");
}

#[test]
fn heading_in_code_is_ignored() {
    let p = parse("```\n# not\n```\n    # indented code\n");
    assert!(p.headings.is_empty());
}

// ----------------------------------------------------------------- wikilinks

#[test]
fn wikilink_forms() {
    let src = "[[a]] [[b|Alias]] [[c#Head]] [[d#H1#H2]] [[e#^blk]] [[#Local]] [[f.md]]";
    let p = parse(src);
    assert_eq!(targets(&p), ["a", "b", "c", "d", "e", "", "f.md"]);
    assert_eq!(p.links[1].display.as_deref(), Some("Alias"));
    assert_eq!(
        p.links[2].subpath,
        Some(Subpath::Heading(vec!["Head".into()]))
    );
    assert_eq!(
        p.links[3].subpath,
        Some(Subpath::Heading(vec!["H1".into(), "H2".into()]))
    );
    assert_eq!(p.links[4].subpath, Some(Subpath::Block("blk".into())));
    assert_eq!(
        p.links[5].subpath,
        Some(Subpath::Heading(vec!["Local".into()]))
    );
    assert!(
        p.links
            .iter()
            .all(|l| l.kind == LinkKind::Wiki && !l.is_embed)
    );
    assert_eq!(slice(src, p.links[1].span), "[[b|Alias]]");
}

#[test]
fn embeds() {
    let src = "![[img.png|300]]\n![[Note#Sec]] ![[doc.pdf]]";
    let p = parse(src);
    assert_eq!(targets(&p), ["img.png", "Note", "doc.pdf"]);
    assert!(p.links.iter().all(|l| l.is_embed));
    assert_eq!(p.links[0].display.as_deref(), Some("300"));
    assert_eq!(slice(src, p.links[0].span), "![[img.png|300]]");
    assert_eq!(p.links[1].line, 1);
    assert_eq!(p.embeds().count(), 3);
    assert_eq!(p.plain_links().count(), 0);
}

#[test]
fn wikilink_escaped_pipe_in_table() {
    let src = "| a | b |\n|---|---|\n| [[Note\\|alias]] | [[X#H\\|y]] |\n";
    let p = parse(src);
    assert_eq!(targets(&p), ["Note", "X"]);
    assert_eq!(p.links[0].display.as_deref(), Some("alias"));
    assert_eq!(p.links[1].subpath, Some(Subpath::Heading(vec!["H".into()])));
    assert_eq!(p.links[0].line, 2);
}

#[test]
fn wikilink_unclosed_and_invalid() {
    let p = parse("[[unclosed and [[ok]] then [[ and ]] [[]] [[#]] [[ | x]] [[  ]]");
    assert_eq!(targets(&p), ["ok", "and"]);
}

#[test]
fn wikilink_not_across_lines() {
    let p = parse("[[a\nb]]");
    assert!(p.links.is_empty());
}

#[test]
fn wikilink_nested_brackets() {
    let src = "[[[a]]] and [[x]]] and [[b [[c]]";
    let p = parse(src);
    assert_eq!(targets(&p), ["a", "x", "c"]);
    assert_eq!(slice(src, p.links[0].span), "[[a]]");
}

#[test]
fn wikilink_escaped() {
    let p = parse("\\[[not]] \\![[link]] ok");
    // `\![[link]]` is still a link, but not an embed.
    assert_eq!(targets(&p), ["link"]);
    assert!(!p.links[0].is_embed);
}

#[test]
fn wikilink_unicode_and_spaces() {
    let src = "Voir [[ Idées fantômes ]] — [[日本語|ノート]]";
    let p = parse(src);
    assert_eq!(targets(&p), ["Idées fantômes", "日本語"]);
    assert_eq!(p.links[1].display.as_deref(), Some("ノート"));
    assert_eq!(slice(src, p.links[1].span), "[[日本語|ノート]]");
}

#[test]
fn links_in_callouts_and_lists() {
    let src = "> [!note] Title [[T]]\n> body [[A]] #tag\n\n- item [[B]]\n  - nested [[C]]\n";
    let p = parse(src);
    assert_eq!(targets(&p), ["T", "A", "B", "C"]);
    assert_eq!(tag_names(&p), ["tag"]);
}

// ------------------------------------------------------------ markdown links

#[test]
fn markdown_links() {
    let src = "[Rust](Projets/Rust%20notes.md#Ownership) [ext](https://x.y) [m](mailto:a@b.c) ![img](pic.png) [<a>](<My Note.md>) [loc](#Sec) [b](Note.md#^id)";
    let p = parse(src);
    assert_eq!(
        targets(&p),
        [
            "Projets/Rust notes.md",
            "pic.png",
            "My Note.md",
            "",
            "Note.md"
        ]
    );
    assert!(p.links.iter().all(|l| l.kind == LinkKind::Markdown));
    assert_eq!(p.links[0].display.as_deref(), Some("Rust"));
    assert_eq!(
        p.links[0].subpath,
        Some(Subpath::Heading(vec!["Ownership".into()]))
    );
    assert!(p.links[1].is_embed);
    assert_eq!(p.links[1].display.as_deref(), Some("img"));
    assert_eq!(p.links[4].subpath, Some(Subpath::Block("id".into())));
    assert_eq!(
        slice(src, p.links[0].span),
        "[Rust](Projets/Rust%20notes.md#Ownership)"
    );
}

#[test]
fn markdown_link_with_title_and_parens() {
    let p = parse("[x](a(b).md \"title\") [y](c%C3%A9.md)");
    assert_eq!(targets(&p), ["a(b).md", "cé.md"]);
}

#[test]
fn obsidian_url_not_reported() {
    let p = parse("[o](obsidian://open?vault=x) [f](file:///tmp/a.md) <https://auto.link>");
    assert!(p.links.is_empty());
}

#[test]
fn links_sorted_in_source_order() {
    let p = parse("[[w1]] [m](m.md) [[w2]]");
    assert_eq!(targets(&p), ["w1", "m.md", "w2"]);
}

// ---------------------------------------------------------------------- tags

#[test]
fn tag_forms() {
    let src = "#tag #nested/tag #tag-with_dashes #élève #日本 #emoji🎉 end";
    let p = parse(src);
    assert_eq!(
        tag_names(&p),
        [
            "tag",
            "nested/tag",
            "tag-with_dashes",
            "élève",
            "日本",
            "emoji🎉"
        ]
    );
    assert_eq!(slice(src, p.tags[1].span), "#nested/tag");
    assert!(p.tags.iter().all(|t| !t.from_frontmatter));
}

#[test]
fn tag_rejections() {
    let p = parse("#123 a#b https://x.com/#frag &#123; \\#esc # space #2026/10 (#par) ##double");
    assert!(p.tags.is_empty(), "{:?}", p.tags);
}

#[test]
fn tag_numeric_with_letter_ok() {
    assert_eq!(tag_names(&parse("#1a #y2026")), ["1a", "y2026"]);
}

#[test]
fn tag_punctuation_terminates() {
    assert_eq!(
        tag_names(&parse("Done #todo, then #next. Also #q?")),
        ["todo", "next", "q"]
    );
}

#[test]
fn tags_not_in_code_math_comments_html() {
    let src = "`#code` $#math$ $$\n#display\n$$ %%#comment%% <!-- #html --> <span title=\"#attr\">x #ok</span>\n\n```\n#fenced\n```\n~~~\n#tilde\n~~~\n\n    #indented\n";
    let p = parse(src);
    assert_eq!(tag_names(&p), ["ok"] as [&str; 1], "{:?}", p.tags);
}

#[test]
fn tags_in_heading() {
    let p = parse("# Title #inhead");
    assert_eq!(tag_names(&p), ["inhead"]);
}

#[test]
fn tag_inside_wikilink_is_not_tag() {
    let p = parse("[[Note #nottag]] [text #nottag2](x.md)");
    assert!(p.tags.is_empty());
}

// ----------------------------------------------------------------- block ids

#[test]
fn block_ids() {
    let src = "Para one ^abc-1\n\n- item ^item\n\n> quote\n\n^standalone\n\nnot^this\nnot ^bad_id\n`code ^nope`";
    let p = parse(src);
    let ids: Vec<&str> = p.block_ids.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["abc-1", "item", "standalone"]);
    assert_eq!(slice(src, p.block_ids[0].span), "^abc-1");
    assert_eq!(p.block_ids[2].line, 6);
}

#[test]
fn block_id_crlf_and_trailing_space() {
    let p = parse("text ^id1  \r\nmore\r\n");
    assert_eq!(p.block_ids[0].id, "id1");
}

// --------------------------------------------------------------------- tasks

#[test]
fn tasks() {
    let src = "- [ ] todo\n* [x] done\n1. [/] partial\n- [-] cancelled\n  - [X] nested\n+ [>] fwd\n- not a task\n- [ab] no\n- [ ]\n";
    let p = parse(src);
    let t: Vec<(char, &str)> = p
        .tasks
        .iter()
        .map(|t| (t.status, t.text.as_str()))
        .collect();
    assert_eq!(
        t,
        [
            (' ', "todo"),
            ('x', "done"),
            ('/', "partial"),
            ('-', "cancelled"),
            ('X', "nested"),
            ('>', "fwd"),
            (' ', ""),
        ]
    );
    assert!(p.tasks[1].is_done() && p.tasks[4].is_done() && !p.tasks[2].is_done());
    assert_eq!(slice(src, p.tasks[0].span), "- [ ] todo");
    assert_eq!(p.tasks[4].line, 4);
    assert_eq!(slice(src, p.tasks[4].span), "- [X] nested");
}

#[test]
fn tasks_in_quote_and_code() {
    let p = parse("> - [ ] quoted\n\n```\n- [ ] code\n```\n%%\n- [ ] hidden\n%%\n");
    assert_eq!(p.tasks.len(), 1);
    assert_eq!(p.tasks[0].text, "quoted");
}

#[test]
fn task_with_link_and_tag() {
    let p = parse("- [ ] call [[Bob]] #urgent");
    assert_eq!(p.tasks[0].text, "call [[Bob]] #urgent");
    assert_eq!(targets(&p), ["Bob"]);
    assert_eq!(tag_names(&p), ["urgent"]);
}

// ------------------------------------------------------------------ comments

#[test]
fn comments_hide_everything() {
    let src = "a %%[[x]] #y%% b\n%%\n# Hidden\n[[z]] - [ ] t ^blk\n%%\nc [[ok]]";
    let p = parse(src);
    assert_eq!(targets(&p), ["ok"]);
    assert!(p.tags.is_empty());
    assert!(p.headings.is_empty());
    assert!(p.tasks.is_empty());
    assert!(p.block_ids.is_empty());
    assert_eq!(p.word_count, 4);
}

#[test]
fn unclosed_comment_runs_to_end() {
    let p = parse("[[a]] %% [[b]]\n\n#c");
    assert_eq!(targets(&p), ["a"]);
    assert!(p.tags.is_empty());
}

#[test]
fn comment_marker_in_code_is_literal() {
    let p = parse("`%%` [[a]] `%%`");
    assert_eq!(targets(&p), ["a"]);
}

#[test]
fn comment_at_line_start_keeps_paragraph() {
    let p = parse("%%note%% Some text [[L]] #t");
    assert_eq!(targets(&p), ["L"]);
    assert_eq!(tag_names(&p), ["t"]);
}

// ---------------------------------------------------------------------- code

#[test]
fn nothing_in_code() {
    let src = "`[[a]]` ``x [[b]] `` \n\n```md\n[[c]] #d [e](e.md)\n```\n\n~~~\n[[f]]\n~~~\n\n    [[g]]\n\n````\n```\n[[h]]\n````\n";
    let p = parse(src);
    assert!(p.links.is_empty(), "{:?}", p.links);
    assert!(p.tags.is_empty());
}

#[test]
fn nothing_in_math() {
    let p = parse("$[[a]]$ and $$[[b]] #c$$ but $5 and [[d]] cost $6");
    assert_eq!(targets(&p), ["d"]);
}

#[test]
fn html_comment_hides_links() {
    let p = parse("<!-- [[a]] -->\n\ntext <!-- [[b]] --> [[c]]");
    assert_eq!(targets(&p), ["c"]);
}

// ---------------------------------------------------------------- word count

#[test]
fn word_count_rules() {
    assert_eq!(parse("one two  three\nfour").word_count, 4);
    assert_eq!(parse("---\na: b c d\n---\none").word_count, 1);
    assert_eq!(parse("# Title\n\n- [ ] l'été est **là**").word_count, 4);
    assert_eq!(parse("a %%b c%% d").word_count, 2);
    assert_eq!(parse("").word_count, 0);
}

// ---------------------------------------------------------------- lines/spans

#[test]
fn spans_and_lines_are_exact() {
    let src = "line0\nl1 [[A]] #t\n\nl3 ![[B|x]]\n";
    let p = parse(src);
    assert_eq!(p.links[0].line, 1);
    assert_eq!(p.links[1].line, 3);
    assert_eq!(slice(src, p.links[0].span), "[[A]]");
    assert_eq!(slice(src, p.links[1].span), "![[B|x]]");
    assert_eq!(p.tags[0].line, 1);
    assert_eq!(slice(src, p.tags[0].span), "#t");
}
