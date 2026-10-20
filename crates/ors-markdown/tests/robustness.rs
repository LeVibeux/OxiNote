#![allow(clippy::unwrap_used)]
//! No panics on arbitrary input; roughly linear performance.

use std::time::{Duration, Instant};

use ors_core::{NoResolver, NotePath};
use ors_markdown::{RenderOptions, parse, render_html, rewrite_links};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "[[",
    "]]",
    "![[",
    "|",
    "\\|",
    "#",
    "^",
    "%%",
    "==",
    "`",
    "```",
    "~~~",
    "$",
    "$$",
    "\n",
    "\r\n",
    " ",
    "  ",
    "    ",
    "-",
    "- [ ] ",
    "* [x] ",
    "1. ",
    "> ",
    "> [!note]",
    "[",
    "]",
    "(",
    ")",
    "<",
    ">",
    "<!--",
    "-->",
    "---\n",
    "a",
    "Note",
    "é",
    "日本",
    "🎉",
    "\\",
    "*",
    "_",
    "&",
    "#tag",
    "^id",
    "\u{E000}",
    "\u{E001}",
    "0",
    "tags: [a]\n",
    "http://x/#y",
    "[x](y.md)",
    "<div>",
    "</div>",
    "\t",
    "|---|",
    "\u{FEFF}",
    "%",
    "%2",
    "[^1]",
    "[^1]: n\n",
];

fn check(src: &str) {
    let p = parse(src);
    for l in &p.links {
        assert!(l.span.start <= l.span.end && l.span.end <= src.len());
    }
    for t in &p.tags {
        assert!(t.span.end <= src.len());
    }
    let o = RenderOptions::new(NotePath::new("a/b.md").unwrap());
    let _ = render_html(src, &o, &NoResolver);
    assert_eq!(rewrite_links(src, |_| None), src);
    let _ = rewrite_links(src, |l| Some(format!("{}x y", l.target)));
}

#[test]
fn fuzz_random_pieces() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3000 {
        let n = rng.below(40);
        let mut s = String::new();
        if rng.below(4) == 0 {
            s.push_str("---\n");
        }
        for _ in 0..n {
            s.push_str(PIECES[rng.below(PIECES.len())]);
        }
        check(&s);
    }
}

#[test]
fn fuzz_random_chars() {
    let mut rng = Rng(42);
    let alphabet: Vec<char> = "[]!|#^%=`$\\\n\r <>-*_()~:é😀a1 \t\"'&".chars().collect();
    for _ in 0..3000 {
        let n = rng.below(60);
        let s: String = (0..n)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        check(&s);
    }
}

#[test]
fn edge_inputs() {
    for s in [
        "",
        "\n",
        "---",
        "---\n",
        "---\n---",
        "---\r\n---\r\n",
        "[[",
        "]]",
        "![[",
        "[[|]]",
        "#",
        "^",
        "%%",
        "%%%",
        "==",
        "$",
        "$$",
        "```",
        "- [",
        "- [ ",
        "- [x",
        "1.",
        "[](",
        "[a](<",
        "\u{FEFF}",
        "\u{FEFF}---\n---",
        "[[a#]]",
        "[[#^]]",
        "> [!",
        "> [!]",
        "> [!x",
        "\\",
    ] {
        check(s);
    }
}

fn big_note(target: usize) -> String {
    let block = "## Section heading\n\nSome text with a [[Wiki link|alias]] and #tag/nested, a [md](Other%20note.md#H) link,\n\
                 `code [[not]]` and ==highlight== plus $x^2$ math %%comment [[hidden]]%%. ^blk1\n\n\
                 - [ ] task one [[T]]\n- [x] done #done\n\n> [!note] Callout\n> with [[link]]\n\n```\n#nope [[nope]]\n```\n\n\
                 | a | b |\n|---|---|\n| [[x\\|y]] | #t |\n\n";
    let mut s = String::from("---\ntags: [a, b]\nrel: \"[[R]]\"\n---\n");
    while s.len() < target {
        s.push_str(block);
    }
    s
}

#[test]
fn one_megabyte_note_is_fast() {
    let src = big_note(1 << 20);
    let start = Instant::now();
    let p = parse(&src);
    let parse_time = start.elapsed();
    let blocks = src.matches("## Section").count();
    assert_eq!(p.headings.len(), blocks);
    assert_eq!(p.tasks.len(), 2 * blocks);
    assert!(p.links.len() >= 5 * blocks);
    let start = Instant::now();
    let _ = render_html(
        &src,
        &RenderOptions::new(NotePath::new("x.md").unwrap()),
        &NoResolver,
    );
    let render_time = start.elapsed();
    eprintln!("1MB parse {parse_time:?} render {render_time:?}");
    // Generous bounds (debug builds on slow CI).
    assert!(
        parse_time < Duration::from_secs(10),
        "parse took {parse_time:?}"
    );
    assert!(
        render_time < Duration::from_secs(15),
        "render took {render_time:?}"
    );
}

#[test]
fn pathological_lines_are_linear() {
    for unit in ["[[", "]]", "[[a", "#a", "==a", "%%", "![[", "`", "[", "^"] {
        let src = unit.repeat(200_000 / unit.len());
        let start = Instant::now();
        let _ = parse(&src);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{unit}: {:?}",
            start.elapsed()
        );
    }
}
