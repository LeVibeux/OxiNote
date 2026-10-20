//! Pure HTML generation for the static site export: escaping, URLs, page
//! template, file tree, backlinks, transclusion of note embeds.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ors_core::{NotePath, ParsedNote};

/// Maximum nesting of note embeds (a page embedding a note embedding a note).
pub const MAX_EMBED_DEPTH: usize = 2;

// ---------------------------------------------------------------- escaping & urls

/// Escape text for HTML content and attribute values.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Reverse of [`escape`] for the common entities (and numeric ones).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Percent-encode a vault path for use in a URL (keeps `/` and unreserved chars).
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Decode `%XX` sequences; returns the input unchanged if the result is not UTF-8.
pub fn decode_percent(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Output path of a vault file: notes become `.html`, other files keep their path.
pub fn output_path(path: &NotePath) -> String {
    if path.is_markdown() {
        format!("{}.html", path.without_extension())
    } else {
        path.as_str().to_string()
    }
}

/// Relative prefix from a page at `path` to the site root (`""`, `"../"`, ...).
pub fn root_prefix(path: &str) -> String {
    "../".repeat(path.matches('/').count())
}

/// `href` from a page with root prefix `prefix` to a vault file.
pub fn href(prefix: &str, target: &NotePath) -> String {
    format!("{prefix}{}", encode_path(&output_path(target)))
}

/// Display name of a file in the tree / titles: stem for notes, file name otherwise.
pub fn display_name(path: &NotePath) -> &str {
    if path.is_markdown() {
        path.stem()
    } else {
        path.file_name()
    }
}

// ---------------------------------------------------------------- page

/// Everything needed to render one page.
pub struct Page<'a> {
    pub title: &'a str,
    pub vault_name: &'a str,
    /// Relative prefix to the site root.
    pub prefix: &'a str,
    /// File name of the index page (`index.html`).
    pub index_page: &'a str,
    /// Sidebar HTML (file tree).
    pub sidebar: &'a str,
    /// Main content (already rendered HTML).
    pub content: &'a str,
    /// Footer content (backlinks section), may be empty.
    pub footer: &'a str,
    /// Show the title as an inline `<h1>`.
    pub inline_title: bool,
}

pub fn page(p: &Page) -> String {
    let title = escape(p.title);
    let vault = escape(p.vault_name);
    let inline_title = if p.inline_title {
        format!("<h1 class=\"inline-title\">{title}</h1>\n")
    } else {
        String::new()
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="generator" content="ObsiRust">
<title>{title} - {vault}</title>
<link rel="stylesheet" href="{prefix}style.css">
</head>
<body>
<div class="app">
<nav class="sidebar">
<div class="vault-name"><a href="{prefix}{index}">{vault}</a></div>
{sidebar}
</nav>
<main class="view-content">
<article class="markdown-reading-view markdown-preview-view">
{inline_title}{content}
</article>
{footer}
</main>
</div>
</body>
</html>
"#,
        prefix = p.prefix,
        index = encode_path(p.index_page),
        sidebar = p.sidebar,
        content = p.content,
        footer = p.footer,
    )
}

/// The "Backlinks" section shown at the bottom of every page.
/// `entries` are `(source note, number of links)`.
pub fn backlinks_section(entries: &[(NotePath, usize)], prefix: &str) -> String {
    let mut out = String::from("<section class=\"backlinks\">\n<h2>Backlinks</h2>\n");
    if entries.is_empty() {
        out.push_str("<p class=\"backlinks-empty\">No backlinks found.</p>\n");
    } else {
        out.push_str("<ul>\n");
        for (source, count) in entries {
            let _ = writeln!(
                out,
                "<li><a class=\"internal-link\" href=\"{}\">{}</a> <span class=\"backlink-count\">{count}</span></li>",
                escape(&href(prefix, source)),
                escape(display_name(source)),
            );
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>");
    out
}

// ---------------------------------------------------------------- file tree

#[derive(Default)]
struct Folder<'a> {
    folders: BTreeMap<String, (&'a str, Folder<'a>)>,
    files: Vec<&'a NotePath>,
}

/// Nested `<ul>` of the vault files, folders first, sorted case-insensitively
/// like Obsidian's file explorer. Folders containing `current` (or all
/// folders when `current` is `None`) are expanded.
pub fn file_tree(files: &[NotePath], prefix: &str, current: Option<&NotePath>) -> String {
    let mut root = Folder::default();
    for f in files {
        let mut folder = &mut root;
        if let Some(parent) = f.parent() {
            for part in parent.split('/') {
                folder = &mut folder
                    .folders
                    .entry(part.to_lowercase())
                    .or_insert_with(|| (part, Folder::default()))
                    .1;
            }
        }
        folder.files.push(f);
    }
    let mut out = String::new();
    write_folder(&root, "", prefix, current, &mut out);
    out
}

fn write_folder(
    folder: &Folder,
    path: &str,
    prefix: &str,
    current: Option<&NotePath>,
    out: &mut String,
) {
    out.push_str("<ul class=\"nav-tree\">\n");
    for (name, sub) in folder.folders.values() {
        let sub_path = if path.is_empty() {
            (*name).to_string()
        } else {
            format!("{path}/{name}")
        };
        let open = current.is_none_or(|c| c.as_str().starts_with(&format!("{sub_path}/")));
        let _ = writeln!(
            out,
            "<li class=\"nav-folder\"><details{}><summary>{}</summary>",
            if open { " open" } else { "" },
            escape(name)
        );
        write_folder(sub, &sub_path, prefix, current, out);
        out.push_str("</details></li>\n");
    }
    let mut files = folder.files.clone();
    files.sort_by(|a, b| {
        a.file_name()
            .to_lowercase()
            .cmp(&b.file_name().to_lowercase())
            .then_with(|| a.cmp(b))
    });
    for f in files {
        let active = current == Some(f);
        let kind = if f.is_markdown() {
            ""
        } else {
            " is-attachment"
        };
        let _ = writeln!(
            out,
            "<li class=\"nav-file{kind}{}\"><a href=\"{}\">{}</a></li>",
            if active { " is-active" } else { "" },
            escape(&href(prefix, f)),
            escape(display_name(f))
        );
    }
    out.push_str("</ul>\n");
}

// ---------------------------------------------------------------- sections

fn normalize_heading(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                c,
                '#' | '|' | '^' | ':' | '%' | '[' | ']' | '*' | '_' | '`' | '='
            )
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_list_item(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return true;
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") "))
}

/// Markdown of the part of a note designated by `subpath`
/// (`"Heading"`, `"H1#H2"` or `"^block-id"`), or `None` if not found.
///
/// A heading section runs until the next heading of the same or a higher
/// level; a block is its list item or paragraph (the `^id` marker removed).
pub fn extract_section(source: &str, parsed: &ParsedNote, subpath: &str) -> Option<String> {
    let lines: Vec<&str> = source.split('\n').collect();
    if let Some(id) = subpath.strip_prefix('^') {
        let block = parsed
            .block_ids
            .iter()
            .find(|b| b.id.eq_ignore_ascii_case(id))?;
        let line = block.line.min(lines.len().checked_sub(1)?);
        let (start, end) = if lines[line].trim().starts_with('^') {
            // Id on its own line: it designates the block just above.
            let mut end = line;
            while end > 0 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            let mut start = end;
            while start > 0 && !lines[start - 1].trim().is_empty() {
                start -= 1;
            }
            (start, end)
        } else if is_list_item(lines[line]) {
            (line, line + 1)
        } else {
            let mut start = line;
            while start > 0
                && !lines[start - 1].trim().is_empty()
                && !lines[start - 1].trim_start().starts_with('#')
            {
                start -= 1;
            }
            (start, line + 1)
        };
        let text = lines[start..end].join("\n");
        let marker = format!("^{}", block.id);
        let trimmed = text.trim_end();
        let text = trimmed.strip_suffix(&marker).map_or(trimmed, str::trim_end);
        return Some(text.to_string());
    }

    let parts: Vec<String> = subpath
        .split('#')
        .filter(|p| !p.trim().is_empty())
        .map(normalize_heading)
        .collect();
    if parts.is_empty() {
        return None;
    }
    let headings = &parsed.headings;
    let mut lo = 0;
    let mut hi = headings.len();
    let mut found = None;
    for part in &parts {
        let idx = (lo..hi).find(|&i| normalize_heading(&headings[i].text) == *part)?;
        let level = headings[idx].level;
        let end = (idx + 1..headings.len())
            .find(|&j| headings[j].level <= level)
            .unwrap_or(headings.len());
        lo = idx + 1;
        hi = end;
        found = Some((idx, end));
    }
    let (idx, end) = found?;
    let start_line = headings[idx].line.min(lines.len());
    let end_line = headings
        .get(end)
        .map_or(lines.len(), |h| h.line.min(lines.len()))
        .max(start_line);
    Some(
        lines[start_line..end_line]
            .join("\n")
            .trim_end()
            .to_string(),
    )
}

/// Body of a note without its frontmatter.
pub fn without_frontmatter<'a>(source: &'a str, parsed: &ParsedNote) -> &'a str {
    match parsed.frontmatter_span {
        Some(span) if span.end <= source.len() && source.is_char_boundary(span.end) => {
            source[span.end..].trim_start_matches(['\r', '\n'])
        }
        _ => source,
    }
}

// ---------------------------------------------------------------- embeds

/// What the embed inliner needs from the vault.
pub trait EmbedSource {
    /// Resolve a link target written in `from`.
    fn resolve(&self, target: &str, from: &NotePath) -> Option<NotePath>;
    /// Markdown source of a note.
    fn source(&self, path: &NotePath) -> Option<String>;
    /// Parse a note (headings, block ids, frontmatter span).
    fn parse(&self, source: &str) -> ParsedNote;
    /// Render a markdown fragment of note `path` to HTML, with links relative
    /// to the page being generated.
    fn render(&self, markdown: &str, path: &NotePath) -> String;
    /// Root prefix of the page being generated.
    fn prefix(&self) -> &str;
}

/// A `<div class="internal-embed" data-src=...>` element found in HTML.
struct Placeholder {
    start: usize,
    end: usize,
    data_src: String,
}

/// Parse the attributes of a tag starting at `html[0] == '<'`.
/// Returns (attributes, index just after `>`).
fn parse_tag(html: &str) -> Option<(Vec<(String, String)>, usize)> {
    let bytes = html.as_bytes();
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'>' => return Some((attrs, i + 1)),
            b'/' => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let name_start = i;
        while i < bytes.len()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
            && !bytes[i].is_ascii_whitespace()
        {
            i += 1;
        }
        let name = html[name_start..i].to_ascii_lowercase();
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                let vstart = i + 1;
                let vend = vstart + html[vstart..].bytes().position(|b| b == quote)?;
                value = unescape(&html[vstart..vend]);
                i = vend + 1;
            } else {
                let vstart = i;
                while i < bytes.len() && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = unescape(&html[vstart..i]);
            }
        }
        attrs.push((name, value));
    }
}

fn find_placeholder(html: &str) -> Option<Placeholder> {
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find("<div") {
        let start = search_from + rel;
        search_from = start + 4;
        let Some((attrs, tag_len)) = parse_tag(&html[start..]) else {
            continue;
        };
        let is_embed = attrs
            .iter()
            .any(|(n, v)| n == "class" && v.split_whitespace().any(|c| c == "internal-embed"));
        let data_src = attrs
            .iter()
            .find(|(n, _)| n == "data-src")
            .map(|(_, v)| v.clone());
        let (true, Some(data_src)) = (is_embed, data_src) else {
            continue;
        };
        // Find the matching </div>.
        let mut depth = 1usize;
        let mut pos = start + tag_len;
        let mut end = start + tag_len;
        while depth > 0 {
            let next_open = html[pos..].find("<div").map(|i| pos + i);
            let next_close = html[pos..].find("</div").map(|i| pos + i);
            match (next_open, next_close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    pos = o + 4;
                }
                (_, Some(c)) => {
                    depth -= 1;
                    pos = c + html[c..].find('>').map_or(html.len() - c, |i| i + 1);
                    if depth == 0 {
                        end = pos;
                    }
                }
                (_, None) => break,
            }
        }
        return Some(Placeholder {
            start,
            end,
            data_src,
        });
    }
    None
}

/// Split an embed reference (`data-src`) into (target, subpath), undoing the
/// percent-encoding and relative prefix a renderer may have applied.
fn split_embed_ref(data_src: &str) -> (String, Option<String>) {
    let mut s = decode_percent(data_src.trim());
    while let Some(rest) = s.strip_prefix("../").or_else(|| s.strip_prefix("./")) {
        s = rest.to_string();
    }
    match s.split_once('#') {
        Some((t, sub)) => (
            t.to_string(),
            Some(sub.to_string()).filter(|s| !s.is_empty()),
        ),
        None => (s, None),
    }
}

fn resolve_embed(src: &dyn EmbedSource, target: &str, from: &NotePath) -> Option<NotePath> {
    if target.is_empty() {
        return Some(from.clone());
    }
    src.resolve(target, from).or_else(|| {
        target
            .strip_suffix(".html")
            .and_then(|t| src.resolve(&format!("{t}.md"), from))
    })
}

fn media_html(prefix: &str, path: &NotePath, alt: &str) -> String {
    let url = escape(&href(prefix, path));
    let alt = escape(alt);
    match path.extension().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "avif" => {
            format!("<img src=\"{url}\" alt=\"{alt}\">")
        }
        "mp3" | "wav" | "m4a" | "ogg" | "3gp" | "flac" => {
            format!("<audio controls src=\"{url}\"></audio>")
        }
        "mp4" | "webm" | "ogv" | "mov" | "mkv" => {
            format!("<video controls src=\"{url}\"></video>")
        }
        "pdf" => format!("<iframe class=\"pdf-embed\" src=\"{url}\" title=\"{alt}\"></iframe>"),
        _ => format!("<a class=\"internal-link\" href=\"{url}\">{alt}</a>"),
    }
}

fn embed_key(path: &NotePath, subpath: Option<&str>) -> String {
    format!("{path}#{}", subpath.unwrap_or(""))
}

/// Stack key of a whole page, to seed [`inline_embeds`]' cycle detection.
pub fn page_key(path: &NotePath) -> String {
    embed_key(path, None)
}

/// Replace note-embed placeholders in `html` (rendered from note `from`) by
/// the rendered target note or section, recursively up to
/// [`MAX_EMBED_DEPTH`]. `stack` holds the keys of the embeds being expanded
/// (seed it with [`page_key`] of the page) and guards against cycles.
pub fn inline_embeds(
    html: &str,
    from: &NotePath,
    src: &dyn EmbedSource,
    stack: &mut Vec<String>,
    depth: usize,
) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(ph) = find_placeholder(rest) {
        out.push_str(&rest[..ph.start]);
        out.push_str(&render_embed(&ph.data_src, from, src, stack, depth));
        rest = &rest[ph.end..];
    }
    out.push_str(rest);
    out
}

fn render_embed(
    data_src: &str,
    from: &NotePath,
    src: &dyn EmbedSource,
    stack: &mut Vec<String>,
    depth: usize,
) -> String {
    let (target, subpath) = split_embed_ref(data_src);
    let attr = escape(data_src);
    let prefix = src.prefix().to_string();
    let Some(path) = resolve_embed(src, &target, from).or_else(|| src.resolve(data_src, from))
    else {
        return format!(
            "<div class=\"internal-embed is-unresolved\" data-src=\"{attr}\"><span class=\"embed-unresolved\">“{}” could not be found.</span></div>",
            escape(&target)
        );
    };
    if !path.is_markdown() {
        return format!(
            "<div class=\"internal-embed media-embed\" data-src=\"{attr}\">{}</div>",
            media_html(&prefix, &path, path.file_name())
        );
    }
    let title = match &subpath {
        Some(s) => format!("{} › {}", display_name(&path), s.trim_start_matches('^')),
        None => display_name(&path).to_string(),
    };
    let link = format!(
        "<a class=\"internal-link markdown-embed-link\" href=\"{}\">{}</a>",
        escape(&href(&prefix, &path)),
        escape(&title)
    );
    let key = embed_key(&path, subpath.as_deref());
    if stack.contains(&key) {
        return format!(
            "<div class=\"internal-embed markdown-embed is-cycle\" data-src=\"{attr}\">{link} <span class=\"embed-note\">(recursive embed)</span></div>"
        );
    }
    if depth >= MAX_EMBED_DEPTH {
        return format!(
            "<div class=\"internal-embed markdown-embed is-depth-limit\" data-src=\"{attr}\">{link}</div>"
        );
    }
    let Some(source) = src.source(&path) else {
        return format!(
            "<div class=\"internal-embed is-unresolved\" data-src=\"{attr}\">{link}</div>"
        );
    };
    let parsed = src.parse(&source);
    let markdown = match &subpath {
        Some(sub) => match extract_section(&source, &parsed, sub) {
            Some(m) => m,
            None => {
                return format!(
                    "<div class=\"internal-embed is-unresolved\" data-src=\"{attr}\"><span class=\"embed-unresolved\">Unable to find section #{} in {link}</span></div>",
                    escape(sub)
                );
            }
        },
        None => without_frontmatter(&source, &parsed).to_string(),
    };
    let rendered = src.render(&markdown, &path);
    stack.push(key);
    let inner = inline_embeds(&rendered, &path, src, stack, depth + 1);
    stack.pop();
    format!(
        "<div class=\"internal-embed markdown-embed\" data-src=\"{attr}\">\n<div class=\"markdown-embed-title\">{link}</div>\n<div class=\"markdown-embed-content\">\n{inner}\n</div>\n</div>"
    )
}

// ---------------------------------------------------------------- css

/// Stylesheet of the exported site, close to Obsidian's reading view.
pub const STYLE_CSS: &str = r#"/* ObsiRust static export — reading view styles */
:root {
  --background-primary: #ffffff;
  --background-secondary: #f6f6f6;
  --background-modifier-border: #e0e0e0;
  --text-normal: #222222;
  --text-muted: #6b6b6b;
  --text-faint: #999999;
  --text-accent: #705dcf;
  --text-accent-hover: #8a79e0;
  --link-unresolved: #a593e0;
  --tag-background: rgba(112, 93, 207, 0.1);
  --tag-border: rgba(112, 93, 207, 0.15);
  --code-background: #f4f4f4;
  --highlight: rgba(255, 208, 0, 0.4);
  --blockquote-border: #705dcf;
  --font-text: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, "Helvetica Neue", sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
  --line-width: 46rem;
}
@media (prefers-color-scheme: dark) {
  :root {
    --background-primary: #1e1e1e;
    --background-secondary: #262626;
    --background-modifier-border: #363636;
    --text-normal: #dadada;
    --text-muted: #a3a3a3;
    --text-faint: #6f6f6f;
    --text-accent: #a68af9;
    --text-accent-hover: #bba6ff;
    --link-unresolved: #7d6bb5;
    --tag-background: rgba(166, 138, 249, 0.12);
    --tag-border: rgba(166, 138, 249, 0.2);
    --code-background: #2a2a2a;
    --highlight: rgba(255, 208, 0, 0.3);
    --blockquote-border: #a68af9;
  }
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  background: var(--background-primary);
  color: var(--text-normal);
  font-family: var(--font-text);
  font-size: 16px;
  line-height: 1.6;
}
.app { display: flex; min-height: 100vh; }
.sidebar {
  flex: 0 0 260px;
  background: var(--background-secondary);
  border-right: 1px solid var(--background-modifier-border);
  padding: 16px 12px;
  font-size: 14px;
  overflow-y: auto;
  max-height: 100vh;
  position: sticky;
  top: 0;
}
.vault-name { font-weight: 600; margin-bottom: 12px; }
.vault-name a { color: var(--text-normal); text-decoration: none; }
.nav-tree { list-style: none; margin: 0; padding-left: 12px; }
.sidebar > .nav-tree { padding-left: 0; }
.nav-tree li { margin: 1px 0; }
.nav-tree summary { cursor: pointer; color: var(--text-muted); }
.nav-file a { color: var(--text-muted); text-decoration: none; display: block; padding: 1px 6px; border-radius: 4px; }
.nav-file a:hover { background: var(--background-modifier-border); color: var(--text-normal); }
.nav-file.is-active a { background: var(--background-modifier-border); color: var(--text-normal); font-weight: 500; }
.nav-file.is-attachment a { color: var(--text-faint); }
.view-content { flex: 1; min-width: 0; padding: 32px 48px 64px; }
.markdown-preview-view, .backlinks { max-width: var(--line-width); margin: 0 auto; }
.inline-title { font-size: 2em; font-weight: 700; margin: 0 0 0.6em; }
h1, h2, h3, h4, h5, h6 { line-height: 1.3; margin: 1.4em 0 0.5em; }
h1 { font-size: 1.8em; } h2 { font-size: 1.5em; } h3 { font-size: 1.3em; }
h4 { font-size: 1.15em; } h5 { font-size: 1.05em; } h6 { font-size: 1em; color: var(--text-muted); }
a { color: var(--text-accent); }
a:hover { color: var(--text-accent-hover); }
a.internal-link { text-decoration: none; }
a.internal-link:hover { text-decoration: underline; }
a.internal-link.is-unresolved { color: var(--link-unresolved); opacity: 0.8; }
a.external-link { text-decoration: underline; }
a.tag {
  background: var(--tag-background);
  border: 1px solid var(--tag-border);
  border-radius: 1em;
  padding: 0.05em 0.6em;
  font-size: 0.875em;
  text-decoration: none;
  white-space: nowrap;
}
mark { background: var(--highlight); color: inherit; padding: 0 2px; border-radius: 2px; }
code { font-family: var(--font-mono); font-size: 0.875em; background: var(--code-background); padding: 0.1em 0.3em; border-radius: 4px; }
pre { background: var(--code-background); padding: 12px 16px; border-radius: 6px; overflow-x: auto; }
pre code { background: none; padding: 0; }
blockquote { margin: 1em 0; padding: 0 1em; border-left: 3px solid var(--blockquote-border); color: var(--text-muted); }
table { border-collapse: collapse; margin: 1em 0; }
th, td { border: 1px solid var(--background-modifier-border); padding: 4px 10px; }
th { background: var(--background-secondary); }
img { max-width: 100%; }
hr { border: none; border-top: 1px solid var(--background-modifier-border); margin: 2em 0; }
ul.contains-task-list, ul:has(> li.task-list-item) { padding-left: 1.2em; }
li.task-list-item { list-style: none; }
li.task-list-item input { margin: 0 0.5em 0 -1.3em; }
li.is-checked, li[data-task="x"] { color: var(--text-faint); text-decoration: line-through; }
.frontmatter, .metadata-container, table.properties {
  font-size: 0.875em;
  color: var(--text-muted);
  margin-bottom: 1.5em;
}
.footnotes { font-size: 0.875em; color: var(--text-muted); }
/* Callouts */
.callout {
  --callout-color: 8, 109, 221;
  background: rgba(var(--callout-color), 0.1);
  border-left: 4px solid rgb(var(--callout-color));
  border-radius: 4px;
  padding: 12px 16px;
  margin: 1em 0;
}
.callout-title { display: flex; gap: 6px; align-items: center; font-weight: 600; color: rgb(var(--callout-color)); }
.callout-content > :first-child { margin-top: 0.4em; }
.callout-content > :last-child { margin-bottom: 0; }
.callout[data-callout="abstract"], .callout[data-callout="summary"], .callout[data-callout="tldr"] { --callout-color: 0, 191, 188; }
.callout[data-callout="info"], .callout[data-callout="todo"] { --callout-color: 8, 109, 221; }
.callout[data-callout="tip"], .callout[data-callout="hint"], .callout[data-callout="important"] { --callout-color: 0, 191, 188; }
.callout[data-callout="success"], .callout[data-callout="check"], .callout[data-callout="done"] { --callout-color: 8, 185, 78; }
.callout[data-callout="question"], .callout[data-callout="help"], .callout[data-callout="faq"],
.callout[data-callout="warning"], .callout[data-callout="caution"], .callout[data-callout="attention"] { --callout-color: 236, 117, 0; }
.callout[data-callout="failure"], .callout[data-callout="fail"], .callout[data-callout="missing"],
.callout[data-callout="danger"], .callout[data-callout="error"], .callout[data-callout="bug"] { --callout-color: 233, 49, 71; }
.callout[data-callout="example"] { --callout-color: 120, 82, 238; }
.callout[data-callout="quote"], .callout[data-callout="cite"] { --callout-color: 158, 158, 158; }
details.callout > summary { cursor: pointer; list-style: none; }
/* Embeds */
.internal-embed.markdown-embed {
  border-left: 2px solid var(--text-accent);
  padding: 4px 0 4px 16px;
  margin: 1em 0;
}
.markdown-embed-title { font-size: 0.8em; color: var(--text-muted); margin-bottom: 4px; }
.markdown-embed-title a { color: var(--text-muted); }
.internal-embed.is-unresolved, .embed-note { color: var(--text-faint); font-style: italic; }
.internal-embed.is-depth-limit, .internal-embed.is-cycle { border-left-style: dashed; }
.pdf-embed { width: 100%; height: 600px; border: 1px solid var(--background-modifier-border); }
audio, video { max-width: 100%; }
/* Backlinks */
.backlinks { margin-top: 3em; padding-top: 1em; border-top: 1px solid var(--background-modifier-border); font-size: 0.9em; }
.backlinks h2 { font-size: 1em; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; margin-top: 0; }
.backlinks ul { padding-left: 1.2em; }
.backlink-count { color: var(--text-faint); font-size: 0.85em; }
.backlink-count::before { content: "("; } .backlink-count::after { content: ")"; }
.backlinks-empty { color: var(--text-faint); }
@media (max-width: 800px) {
  .app { flex-direction: column; }
  .sidebar { position: static; max-height: none; flex: none; border-right: none; border-bottom: 1px solid var(--background-modifier-border); }
  .view-content { padding: 20px 16px 48px; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use ors_core::{BlockId, Heading, Span};
    use std::collections::HashMap;

    fn p(s: &str) -> NotePath {
        NotePath::new(s).unwrap()
    }

    #[test]
    fn escaping_roundtrip() {
        let s = r#"<a href="x">Tom & 'Jerry'</a>"#;
        let e = escape(s);
        assert_eq!(
            e,
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/a&gt;"
        );
        assert_eq!(unescape(&e), s);
        assert_eq!(unescape("&#x41;&#66;&unknown; & a"), "AB&unknown; & a");
    }

    #[test]
    fn urls() {
        assert_eq!(
            encode_path("Projets/Rust notes.html"),
            "Projets/Rust%20notes.html"
        );
        assert_eq!(encode_path("Idées.html"), "Id%C3%A9es.html");
        assert_eq!(decode_percent("Id%C3%A9es%20x"), "Idées x");
        assert_eq!(decode_percent("100%"), "100%");
        assert_eq!(decode_percent("%FF"), "%FF");
        assert_eq!(output_path(&p("a/B.md")), "a/B.html");
        assert_eq!(output_path(&p("img/x.png")), "img/x.png");
        assert_eq!(root_prefix("a/b/c.html"), "../../");
        assert_eq!(root_prefix("c.html"), "");
        assert_eq!(
            href("../", &p("Projets/Rust notes.md")),
            "../Projets/Rust%20notes.html"
        );
    }

    #[test]
    fn page_template() {
        let html = page(&Page {
            title: "A <b>",
            vault_name: "V",
            prefix: "../",
            index_page: "index.html",
            sidebar: "<ul></ul>",
            content: "<p>hi</p>",
            footer: "<section class=\"backlinks\"></section>",
            inline_title: true,
        });
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>A &lt;b&gt; - V</title>"));
        assert!(html.contains("href=\"../style.css\""));
        assert!(html.contains("<a href=\"../index.html\">V</a>"));
        assert!(html.contains("<h1 class=\"inline-title\">A &lt;b&gt;</h1>\n<p>hi</p>"));
        assert!(html.contains("<section class=\"backlinks\">"));
    }

    #[test]
    fn backlinks_html() {
        let html = backlinks_section(&[(p("Journal/2026-10-20.md"), 2)], "../");
        assert!(html.contains("<h2>Backlinks</h2>"));
        assert!(html.contains("href=\"../Journal/2026-10-20.html\">2026-10-20</a>"));
        assert!(html.contains("<span class=\"backlink-count\">2</span>"));
        assert!(backlinks_section(&[], "").contains("No backlinks found."));
    }

    #[test]
    fn tree() {
        let files = vec![
            p("Accueil.md"),
            p("attachments/diagramme.png"),
            p("Projets/ObsiRust.md"),
            p("Projets/Rust notes.md"),
            p("b.md"),
        ];
        let html = file_tree(&files, "../", Some(&p("Projets/ObsiRust.md")));
        // Folders first (sorted case-insensitively), then files.
        let attach = html.find("<summary>attachments</summary>").unwrap();
        let projets = html.find("<summary>Projets</summary>").unwrap();
        let accueil = html.find(">Accueil</a>").unwrap();
        assert!(attach < projets && projets < accueil);
        assert!(html.contains("<details open><summary>Projets</summary>"));
        assert!(html.contains("<details><summary>attachments</summary>"));
        assert!(html.contains(
            "<li class=\"nav-file is-active\"><a href=\"../Projets/ObsiRust.html\">ObsiRust</a></li>"
        ));
        assert!(html.contains(
            "<li class=\"nav-file is-attachment\"><a href=\"../attachments/diagramme.png\">diagramme.png</a></li>"
        ));
        let all = file_tree(&files, "", None);
        assert!(all.contains("<details open><summary>attachments</summary>"));
    }

    fn heading(level: u8, text: &str, line: usize) -> Heading {
        Heading {
            level,
            text: text.into(),
            span: Span::default(),
            line,
        }
    }

    const NOTE: &str = "# ObsiRust\n\nIntro.\n\n## Architecture\n\nWorkspace Cargo.\n\n### Crates\n\nSept.\n\n## Statut\n\nOK.\n- item one ^item\n\nPara line 1\npara line 2 ^para\n\n| a |\n| - |\n\n^table";

    fn parsed_note() -> ParsedNote {
        ParsedNote {
            headings: vec![
                heading(1, "ObsiRust", 0),
                heading(2, "Architecture", 4),
                heading(3, "Crates", 8),
                heading(2, "Statut", 12),
            ],
            block_ids: vec![
                BlockId {
                    id: "item".into(),
                    span: Span::default(),
                    line: 15,
                },
                BlockId {
                    id: "para".into(),
                    span: Span::default(),
                    line: 18,
                },
                BlockId {
                    id: "table".into(),
                    span: Span::default(),
                    line: 23,
                },
            ],
            ..ParsedNote::default()
        }
    }

    #[test]
    fn sections() {
        let parsed = parsed_note();
        assert_eq!(
            extract_section(NOTE, &parsed, "Architecture").unwrap(),
            "## Architecture\n\nWorkspace Cargo.\n\n### Crates\n\nSept."
        );
        assert_eq!(
            extract_section(NOTE, &parsed, "architecture#crates").unwrap(),
            "### Crates\n\nSept."
        );
        assert_eq!(
            extract_section(NOTE, &parsed, "ObsiRust#Statut").unwrap(),
            "## Statut\n\nOK.\n- item one ^item\n\nPara line 1\npara line 2 ^para\n\n| a |\n| - |\n\n^table"
        );
        assert!(extract_section(NOTE, &parsed, "Crates#Architecture").is_none());
        assert!(extract_section(NOTE, &parsed, "Nope").is_none());
        assert_eq!(
            extract_section(NOTE, &parsed, "^item").unwrap(),
            "- item one"
        );
        assert_eq!(
            extract_section(NOTE, &parsed, "^PARA").unwrap(),
            "Para line 1\npara line 2"
        );
        assert_eq!(
            extract_section(NOTE, &parsed, "^table").unwrap(),
            "| a |\n| - |"
        );
        assert!(extract_section(NOTE, &parsed, "^missing").is_none());
    }

    #[test]
    fn frontmatter_is_stripped() {
        let src = "---\na: 1\n---\n# T\n";
        let parsed = ParsedNote {
            frontmatter_span: Some(Span::new(0, 12)),
            ..ParsedNote::default()
        };
        assert_eq!(without_frontmatter(src, &parsed), "# T\n");
        assert_eq!(without_frontmatter(src, &ParsedNote::default()), src);
    }

    /// Fake vault: renders `![[X]]` lines as placeholders and other lines as paragraphs.
    struct Fake {
        notes: HashMap<String, String>,
        prefix: String,
    }

    impl EmbedSource for Fake {
        fn resolve(&self, target: &str, _from: &NotePath) -> Option<NotePath> {
            let t = target.strip_suffix(".md").unwrap_or(target);
            if self.notes.contains_key(t) {
                return Some(p(&format!("{t}.md")));
            }
            (t == "img.png").then(|| p("img.png"))
        }
        fn source(&self, path: &NotePath) -> Option<String> {
            self.notes.get(path.without_extension()).cloned()
        }
        fn parse(&self, source: &str) -> ParsedNote {
            let headings = source
                .lines()
                .enumerate()
                .filter(|(_, l)| l.starts_with('#'))
                .map(|(i, l)| {
                    let level = l.chars().take_while(|&c| c == '#').count() as u8;
                    heading(level, l[level as usize..].trim(), i)
                })
                .collect();
            ParsedNote {
                headings,
                ..ParsedNote::default()
            }
        }
        fn render(&self, markdown: &str, _path: &NotePath) -> String {
            markdown
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(
                    |l| match l.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
                        Some(t) => format!(
                            "<div class=\"internal-embed\" data-src=\"{}\"></div>",
                            escape(t)
                        ),
                        None => format!("<p>{}</p>", escape(l)),
                    },
                )
                .collect::<Vec<_>>()
                .join("\n")
        }
        fn prefix(&self) -> &str {
            &self.prefix
        }
    }

    fn fake() -> Fake {
        let mut notes = HashMap::new();
        notes.insert("A".to_string(), "# A\n\nText A\n![[B#Part]]".to_string());
        notes.insert(
            "B".to_string(),
            "# B\n## Part\nPart text\n![[C]]\n## Other\nNo".to_string(),
        );
        notes.insert("C".to_string(), "C text\n![[D]]".to_string());
        notes.insert("D".to_string(), "D text".to_string());
        notes.insert("Self".to_string(), "Me\n![[Self]]".to_string());
        notes.insert("X".to_string(), "x\n![[Y]]".to_string());
        notes.insert("Y".to_string(), "y\n![[X]]".to_string());
        Fake {
            notes,
            prefix: "../".into(),
        }
    }

    fn render_page(src: &Fake, name: &str) -> String {
        let path = p(&format!("{name}.md"));
        let html = src.render(&src.notes[name], &path);
        inline_embeds(&html, &path, src, &mut vec![page_key(&path)], 0)
    }

    #[test]
    fn embeds_are_inlined_up_to_depth_2() {
        let src = fake();
        let html = render_page(&src, "A");
        assert!(html.contains("<p>Text A</p>"));
        // Depth 1: section of B (not "Other").
        assert!(html.contains("<p>Part text</p>"), "{html}");
        assert!(!html.contains("<p>No</p>"));
        assert!(html.contains("href=\"../B.html\">B › Part</a>"));
        // Depth 2: C is inlined, D (depth 3) is only linked.
        assert!(html.contains("<p>C text</p>"));
        assert!(!html.contains("<p>D text</p>"));
        assert!(html.contains("is-depth-limit"));
        assert!(html.contains("href=\"../D.html\">D</a>"));
        // No placeholder left.
        assert!(!html.contains("data-src=\"B#Part\"></div>"));
    }

    #[test]
    fn embed_cycles_are_detected() {
        let src = fake();
        let html = render_page(&src, "Self");
        assert!(html.contains("is-cycle"));
        assert_eq!(html.matches("<p>Me</p>").count(), 1);
        let html = render_page(&src, "X");
        // X embeds Y, which embeds X again: cycle.
        assert!(html.contains("<p>y</p>"));
        assert!(html.contains("is-cycle"));
    }

    #[test]
    fn unresolved_media_and_missing_section_embeds() {
        let src = fake();
        let from = p("A.md");
        let html = "<p>a</p><div class=\"internal-embed foo\" data-src=\"Nope\"><span>x</span></div><p>b</p>\
                    <div data-src=\"img.png\" class=\"internal-embed\"></div>\
                    <div class=\"internal-embed\" data-src=\"B#Missing\"></div>\
                    <div class=\"other\">kept</div>";
        let out = inline_embeds(html, &from, &src, &mut vec![page_key(&from)], 0);
        assert!(
            out.starts_with(
                "<p>a</p><div class=\"internal-embed is-unresolved\" data-src=\"Nope\">"
            )
        );
        assert!(out.contains("“Nope” could not be found."));
        assert!(!out.contains("<span>x</span>"));
        assert!(out.contains("</div><p>b</p>"));
        assert!(out.contains("<img src=\"../img.png\" alt=\"img.png\">"));
        assert!(out.contains("Unable to find section #Missing"));
        assert!(out.ends_with("<div class=\"other\">kept</div>"));
    }

    #[test]
    fn embed_refs_are_decoded() {
        assert_eq!(
            split_embed_ref("../Projets/Rust%20notes.html#Ownership"),
            ("Projets/Rust notes.html".into(), Some("Ownership".into()))
        );
        assert_eq!(
            split_embed_ref("Idées#^graphe"),
            ("Idées".into(), Some("^graphe".into()))
        );
        assert_eq!(
            split_embed_ref("#Heading"),
            (String::new(), Some("Heading".into()))
        );
        assert_eq!(split_embed_ref("Note#"), ("Note".into(), None));
        let src = fake();
        assert_eq!(resolve_embed(&src, "B.html", &p("A.md")), Some(p("B.md")));
        assert_eq!(resolve_embed(&src, "", &p("A.md")), Some(p("A.md")));
    }

    #[test]
    fn nested_divs_inside_placeholder() {
        let src = fake();
        let from = p("A.md");
        let html = "<div class=\"internal-embed\" data-src=\"D\"><div><div>x</div></div></div>tail";
        let out = inline_embeds(html, &from, &src, &mut vec![], 0);
        assert!(out.contains("<p>D text</p>"));
        assert!(out.ends_with("</div>tail"));
        assert!(!out.contains(">x<"));
    }
}
