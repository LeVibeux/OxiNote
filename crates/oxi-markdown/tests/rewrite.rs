#![allow(clippy::unwrap_used)]
use oxi_core::{Link, LinkKind};
use oxi_markdown::{parse, rewrite_links};

fn rename(src: &str, from: &str, to: &str) -> String {
    rewrite_links(src, |l| {
        let t = l.target.strip_suffix(".md").unwrap_or(&l.target);
        (t == from).then(|| to.to_string())
    })
}

#[test]
fn no_change_is_identity() {
    let src = "---\na: \"[[X]]\"\n---\n[[X]] [m](X.md) ![[X]] \u{1F600} é\r\n";
    assert_eq!(rewrite_links(src, |_| None), src);
}

#[test]
fn wikilinks_preserve_subpath_alias_embed() {
    let src = "[[Old]] [[Old|alias]] [[Old#H1#H2|a]] ![[Old#^blk]] [[Other]] [[ Old ]]";
    assert_eq!(
        rename(src, "Old", "New Name"),
        "[[New Name]] [[New Name|alias]] [[New Name#H1#H2|a]] ![[New Name#^blk]] [[Other]] [[ New Name ]]"
    );
}

#[test]
fn wikilink_in_table_keeps_escaped_pipe() {
    let src = "| [[Old\\|alias]] |\n|---|\n";
    assert_eq!(
        rename(src, "Old", "dir/New"),
        "| [[dir/New\\|alias]] |\n|---|\n"
    );
}

#[test]
fn markdown_links_encoded_and_md_kept() {
    let src = "[t](Old.md#Sec) [u](<Old.md>) ![i](Old.md) [v](Old)";
    assert_eq!(
        rename(src, "Old", "My New"),
        "[t](My%20New.md#Sec) [u](<My%20New.md>) ![i](My%20New.md) [v](My%20New)"
    );
}

#[test]
fn markdown_new_target_already_encoded_is_idempotent() {
    let src = "[t](Projets/Rust%20notes.md#Ownership)";
    let out = rewrite_links(src, |_| Some("Projets/Rust%20book.md".into()));
    assert_eq!(out, "[t](Projets/Rust%20book.md#Ownership)");
    let out = rewrite_links(src, |_| Some("Projets/Rust book.md".into()));
    assert_eq!(out, "[t](Projets/Rust%20book.md#Ownership)");
    let out = rewrite_links(src, |_| Some("A (1) <x>.md".into()));
    assert_eq!(out, "[t](A%20%281%29%20%3Cx%3E.md#Ownership)");
    assert_eq!(parse(&out).links[0].target, "A (1) <x>.md");
}

#[test]
fn md_extension_follows_original() {
    assert_eq!(rewrite_links("[[A]]", |_| Some("B.md".into())), "[[B]]");
    assert_eq!(
        rewrite_links("[x](A.md)", |_| Some("B".into())),
        "[x](B.md)"
    );
    assert_eq!(
        rewrite_links("[x](A.md)", |_| Some("B.md".into())),
        "[x](B.md)"
    );
    assert_eq!(
        rewrite_links("![[a.png]]", |_| Some("img/b.png".into())),
        "![[img/b.png]]"
    );
}

#[test]
fn frontmatter_links_rewritten() {
    let src = "---\nrelated: \"[[Old]]\"\nlist:\n  - \"[[Old|x]]\"\n---\nbody [[Old]]";
    assert_eq!(
        rename(src, "Old", "New"),
        "---\nrelated: \"[[New]]\"\nlist:\n  - \"[[New|x]]\"\n---\nbody [[New]]"
    );
}

#[test]
fn code_and_comments_untouched() {
    let src = "`[[Old]]` %%[[Old]]%% [[Old]]\n```\n[[Old]]\n```\n";
    assert_eq!(
        rename(src, "Old", "N"),
        "`[[Old]]` %%[[Old]]%% [[N]]\n```\n[[Old]]\n```\n"
    );
}

#[test]
fn same_note_links_receive_callback() {
    let mut seen = Vec::new();
    let _ = rewrite_links("[[#H]] [x](#Y)", |l| {
        seen.push((l.target.clone(), l.kind));
        None
    });
    assert_eq!(
        seen,
        [
            (String::new(), LinkKind::Wiki),
            (String::new(), LinkKind::Markdown)
        ]
    );
}

#[test]
fn callback_links_match_parse_exactly() {
    let src = "---\nrel: \"[[A]]\"\ntags: [t]\n---\n# H\n[[B#x|y]] ![[c.png|30]] [d](D%20e.md#^b) ![i](i.png)\n| [[E\\|e]] |\n|---|\n> [!note] [[F]]\n";
    let parsed = parse(src).links;
    let mut seen: Vec<Link> = Vec::new();
    let _ = rewrite_links(src, |l| {
        seen.push(l.clone());
        None
    });
    assert_eq!(seen, parsed);
    assert_eq!(seen.len(), 7);
}

#[test]
fn unicode_and_crlf_preserved() {
    let src = "Été\r\n[[Vieux|vieil été]] 😀\r\n[l](Vieux.md)\r\n";
    assert_eq!(
        rename(src, "Vieux", "Neuf é"),
        "Été\r\n[[Neuf é|vieil été]] 😀\r\n[l](Neuf%20é.md)\r\n"
    );
}
