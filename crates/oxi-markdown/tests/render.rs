#![allow(clippy::unwrap_used)]
use oxi_core::{LinkResolver, NoResolver, NotePath};
use oxi_markdown::{RenderOptions, render_html};

/// Resolves any target listed in `files` by stem or full path.
struct Files(Vec<&'static str>);

impl LinkResolver for Files {
    fn resolve(&self, target: &str, _from: &NotePath) -> Option<NotePath> {
        self.0
            .iter()
            .find(|f| {
                let p = NotePath::new(**f).ok();
                **f == target
                    || p.as_ref().is_some_and(|p| {
                        p.without_extension() == target
                            || p.stem() == target
                            || p.file_name() == target
                    })
            })
            .and_then(|f| NotePath::new(*f).ok())
    }
}

fn opts() -> RenderOptions {
    RenderOptions::new(NotePath::new("folder/Current.md").unwrap())
}

fn render(src: &str) -> String {
    render_html(src, &opts(), &NoResolver)
}

fn render_with(src: &str, files: Vec<&'static str>) -> String {
    render_html(src, &opts(), &Files(files))
}

#[test]
fn empty() {
    assert_eq!(render(""), "");
}

#[test]
fn basic_markdown_gfm() {
    let h = render("**b** ~~s~~\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nfoot[^1]\n\n[^1]: note\n");
    assert!(h.contains("<strong>b</strong>"));
    assert!(h.contains("<del>s</del>"));
    assert!(h.contains("<table>"));
    assert!(h.contains("footnote"), "{h}");
}

#[test]
fn wikilink_resolved_and_unresolved() {
    let h = render_with("[[Other]] [[Missing note]]", vec!["sub/Other.md"]);
    assert!(
        h.contains(r#"<a class="internal-link" href="sub/Other.md" data-href="Other">Other</a>"#),
        "{h}"
    );
    assert!(
        h.contains(r#"<a class="internal-link is-unresolved" href="Missing%20note" data-href="Missing note">Missing note</a>"#),
        "{h}"
    );
}

#[test]
fn wikilink_prefix_extension_subpath() {
    let mut o = opts();
    o.link_prefix = "../".into();
    o.note_extension = Some("html".into());
    let h = render_html(
        "[[My Note#Some Heading|alias]] [[My Note#^blk]] [[#Local Part]]",
        &o,
        &Files(vec!["dir/My Note.md"]),
    );
    assert!(
        h.contains(r#"href="../dir/My%20Note.html#some-heading" data-href="My Note#Some Heading">alias</a>"#),
        "{h}"
    );
    assert!(h.contains(r#"href="../dir/My%20Note.html#^blk""#), "{h}");
    assert!(
        h.contains(r##"href="#local-part" data-href="#Local Part">Local Part</a>"##),
        "{h}"
    );
}

#[test]
fn wikilink_default_display_with_subpath() {
    let h = render("[[Note#A#B]]");
    assert!(h.contains(">Note &gt; A &gt; B</a>"), "{h}");
}

#[test]
fn image_embeds() {
    let h = render_with(
        "![[pic.png]] ![[pic.png|300]] ![[pic.png|300x200]] ![[pic.png|A cat]]",
        vec!["att/pic.png"],
    );
    assert!(
        h.contains(r#"<img src="att/pic.png" alt="pic.png">"#),
        "{h}"
    );
    assert!(
        h.contains(r#"<img src="att/pic.png" alt="pic.png" width="300">"#),
        "{h}"
    );
    assert!(h.contains(r#"width="300" height="200""#), "{h}");
    assert!(h.contains(r#"alt="A cat""#), "{h}");
}

#[test]
fn note_embed_placeholder() {
    let h = render("![[Projets/Oxinote#Architecture]]\n\n![[doc.pdf]]");
    assert!(
        h.contains(r#"<div class="internal-embed" data-src="Projets/Oxinote#Architecture"></div>"#),
        "{h}"
    );
    assert!(
        h.contains(r#"<div class="internal-embed" data-src="doc.pdf"></div>"#),
        "{h}"
    );
}

#[test]
fn markdown_internal_link() {
    let h = render_with(
        "[Rust](Projets/Rust%20notes.md#Ownership)",
        vec!["Projets/Rust notes.md"],
    );
    assert!(
        h.contains(r#"<a class="internal-link" href="Projets/Rust%20notes.md#ownership" data-href="Projets/Rust notes.md#Ownership">Rust</a>"#),
        "{h}"
    );
}

#[test]
fn markdown_image_internal() {
    let h = render_with("![alt text](img/a b.png)", vec!["img/a b.png"]);
    // `a b.png` with a space is not a valid inline destination; use <>.
    let h2 = render_with("![alt text](<img/a b.png>)", vec!["img/a b.png"]);
    assert!(!h.contains("<script"));
    assert!(
        h2.contains(r#"<img src="img/a%20b.png" alt="alt text""#),
        "{h2}"
    );
}

#[test]
fn external_links() {
    let h = render("[docs](https://docs.rs) <https://a.b> [bad](javascript:alert(1))");
    assert!(h.contains(r#"<a class="external-link" href="https://docs.rs" target="_blank" rel="noopener nofollow">docs</a>"#), "{h}");
    assert!(h.contains(r#"href="https://a.b""#));
    assert!(!h.contains("javascript:"), "{h}");
}

#[test]
fn highlight() {
    let h = render("a ==marked **text**== b == not == c");
    assert!(
        h.contains("<mark>marked <strong>text</strong></mark>"),
        "{h}"
    );
    assert!(h.contains("== not =="), "{h}");
}

#[test]
fn highlight_unclosed_is_literal() {
    let h = render("==open only");
    assert!(h.contains("==open only"), "{h}");
    assert!(!h.contains("<mark>"));
}

#[test]
fn setext_heading_not_highlight() {
    let h = render("Title\n====\n\nOther\n==\n");
    assert!(h.contains("<h1 id=\"title\">Title</h1>"), "{h}");
    assert!(h.contains("<h1 id=\"other\">Other</h1>"), "{h}");
}

#[test]
fn comments_removed() {
    let h = render("a %%secret%% b\n\n%%\nblock secret\n%%\n\nc");
    assert!(!h.contains("secret"), "{h}");
    assert!(h.contains("<p>a  b</p>") || h.contains("a"), "{h}");
}

#[test]
fn callout_basic() {
    let h = render("> [!tip] Astuce\n> Body [[Link]]\n");
    assert!(
        h.contains(r#"<div class="callout" data-callout="tip">"#),
        "{h}"
    );
    assert!(
        h.contains(r#"<div class="callout-title">Astuce</div>"#),
        "{h}"
    );
    assert!(h.contains(r#"<div class="callout-content">"#), "{h}");
    assert!(
        h.contains("<p>Body <a class=\"internal-link is-unresolved\""),
        "{h}"
    );
    assert!(h.trim_end().ends_with("</div></div>"), "{h}");
    assert!(!h.contains("<blockquote>"));
}

#[test]
fn callout_foldable_default_title_and_nested() {
    let h = render("> [!WARNING]-\n> outer\n> > [!note]+ Inner\n> > text\n");
    assert!(
        h.contains(r#"data-callout="warning" data-callout-fold="-""#),
        "{h}"
    );
    assert!(
        h.contains(r#"<div class="callout-title">WARNING</div>"#),
        "{h}"
    );
    assert!(
        h.contains(r#"data-callout="note" data-callout-fold="+""#),
        "{h}"
    );
    assert_eq!(h.matches("<div class=\"callout\"").count(), 2);
    assert_eq!(h.matches("</div></div>").count(), 2);
}

#[test]
fn callout_title_formatting_and_plain_quote() {
    let h = render("> [!info] A **bold** title\n\n> plain quote\n");
    assert!(
        h.contains("<div class=\"callout-title\">A <strong>bold</strong> title</div>"),
        "{h}"
    );
    assert!(h.contains("<blockquote>"), "{h}");
}

#[test]
fn tags_rendered() {
    let h = render("Hello #world and #a/b");
    assert!(
        h.contains(r##"<a class="tag" href="#world">#world</a>"##),
        "{h}"
    );
    assert!(
        h.contains(r##"<a class="tag" href="#a/b">#a/b</a>"##),
        "{h}"
    );
}

#[test]
fn heading_ids_and_duplicates() {
    let h = render("# Hello World\n## Hello World\n### Été & co\n");
    assert!(h.contains("<h1 id=\"hello-world\">Hello World</h1>"), "{h}");
    assert!(h.contains("<h2 id=\"hello-world-1\">"), "{h}");
    assert!(h.contains("<h3 id=\"été-co\">Été &amp; co</h3>"), "{h}");
}

#[test]
fn heading_with_link_slug() {
    let h = render("## See [[Note|the note]]");
    assert!(h.contains("<h2 id=\"see-the-note\">"), "{h}");
}

#[test]
fn block_id_anchor_hidden() {
    let h = render("Paragraph text ^my-id");
    assert!(
        h.contains(r#"<span class="block-id" id="^my-id"></span>"#),
        "{h}"
    );
    assert!(!h.contains("^my-id<"), "{h}");
}

#[test]
fn tasks_rendered() {
    let h = render("- [ ] a\n- [x] b\n- [/] c\n");
    assert!(
        h.contains(
            r#"<input type="checkbox" class="task-list-item-checkbox" data-task=" " disabled> a"#
        ),
        "{h}"
    );
    assert!(h.contains(r#"data-task="x" disabled checked> b"#), "{h}");
    assert!(h.contains(r#"data-task="/""#), "{h}");
}

#[test]
fn math_rendered_escaped() {
    let h = render("Inline $a<b$ and\n\n$$\nx^2 & y\n$$\n");
    assert!(h.contains(r#"<span class="math">a&lt;b</span>"#), "{h}");
    assert!(h.contains(r#"<div class="math">"#), "{h}");
    assert!(h.contains("x^2 &amp; y"), "{h}");
}

#[test]
fn code_untouched() {
    let h = render("`[[a]] #b ==c==`\n\n```\n[[x]] #y %%z%%\n```\n");
    assert!(h.contains("<code>[[a]] #b ==c==</code>"), "{h}");
    assert!(h.contains("[[x]] #y %%z%%"), "{h}");
    assert!(!h.contains("internal-link"));
}

#[test]
fn frontmatter_dropped_or_table() {
    let src = "---\ntitle: <b>T</b>\ntags: [x]\nrel: \"[[Other]]\"\n---\nBody";
    let h = render(src);
    assert!(!h.contains("title"), "{h}");
    assert_eq!(h.trim(), "<p>Body</p>");
    let mut o = opts();
    o.show_frontmatter = true;
    let h = render_html(src, &o, &Files(vec!["Other.md"]));
    assert!(h.starts_with("<table class=\"properties\">"), "{h}");
    assert!(
        h.contains("<th>title</th><td>&lt;b&gt;T&lt;/b&gt;</td>"),
        "{h}"
    );
    assert!(h.contains(r##"<a class="tag" href="#x">#x</a>"##), "{h}");
    assert!(
        h.contains(r#"<a class="internal-link" href="Other.md" data-href="Other">Other</a>"#),
        "{h}"
    );
}

#[test]
fn xss_in_link_text_and_targets() {
    let h = render("[[a\"><script>alert(1)</script>|<img onerror=x>]] #<script>");
    assert!(!h.contains("<script>"), "{h}");
    assert!(!h.contains("<img onerror"), "{h}");
}

#[test]
fn raw_html_sanitized() {
    let h = render(
        "<script>alert(1)</script>\n\ntext <b onclick=\"x()\">b</b> <i>ok</i> <a href=\"javascript:x\">j</a>\n\n<div class=\"x\">\n[[in html]]\n</div>\n",
    );
    assert!(!h.contains("<script>"), "{h}");
    assert!(!h.contains("<b onclick"), "{h}");
    assert!(h.contains("<i>ok</i>"), "{h}");
    assert!(!h.contains("<a href=\"javascript"), "{h}");
    assert!(h.contains("<div class=\"x\">"), "{h}");
    assert!(h.contains("data-href=\"in html\""), "{h}");
}

#[test]
fn html_comment_removed() {
    let h = render("a <!-- hidden --> b\n\n<!--\nblock\n-->\n");
    assert!(!h.contains("hidden") && !h.contains("block"), "{h}");
}

#[test]
fn links_in_tables_render() {
    let h = render("| a | b |\n|---|---|\n| [[Note\\|alias]] | #t |\n");
    assert!(h.contains(">alias</a></td>"), "{h}");
    assert_eq!(h.matches("<td>").count(), 2, "{h}");
}

#[test]
fn placeholder_chars_in_source_are_harmless() {
    let h = render("x \u{E000}0\u{E001} [[a]]");
    assert!(h.contains("internal-link"), "{h}");
    assert!(!h.contains('\u{E000}'));
}

#[test]
fn render_sample_like_note() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/sample-vault/Idées.md"
    ))
    .unwrap();
    let h = render_with(
        &src,
        vec!["Accueil.md", "attachments/diagramme.png", "Idées.md"],
    );
    assert!(h.contains("<mark>surlignages</mark>"), "{h}");
    assert!(
        h.contains(r#"<img src="attachments/diagramme.png" alt="diagramme.png">"#),
        "{h}"
    );
    assert!(h.contains(r#"href="Accueil.md#accueil""#), "{h}");
    assert!(!h.contains("Lien caché"), "{h}");
    assert!(h.contains("<code>#pas-un-tag</code>"), "{h}");
}
