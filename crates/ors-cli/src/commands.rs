//! Data gathering and text formatting of the simple subcommands.
//!
//! Formatting functions take plain data and return `String`s so they can be
//! unit-tested without a vault.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Result, anyhow, bail};
use ors_core::{Link, LinkKind, NotePath, Subpath};
use ors_index::{Backlink, MetadataCache, ResolvedLink, SearchResult};
use serde::Serialize;

use crate::context::Context;
use crate::output::highlight;

// ---------------------------------------------------------------- stats

/// Vault statistics (`obsirust stats`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Stats {
    /// Markdown notes.
    pub notes: usize,
    /// Non-markdown files.
    pub attachments: usize,
    /// Internal links and embeds in all notes.
    pub links: usize,
    /// Of `links`, how many are embeds.
    pub embeds: usize,
    pub resolved_links: usize,
    pub unresolved_links: usize,
    /// Distinct tags (case-insensitive, nested tags counted once by full name).
    pub tags: usize,
    /// Total number of tag occurrences (inline + frontmatter).
    pub tag_occurrences: usize,
    /// Notes without incoming or outgoing links.
    pub orphans: usize,
    pub words: usize,
    pub tasks: usize,
    pub tasks_done: usize,
}

impl Stats {
    pub fn compute(cache: &MetadataCache) -> Self {
        let mut stats = Stats::default();
        let mut tags = BTreeSet::new();
        for path in cache.files() {
            if !path.is_markdown() {
                stats.attachments += 1;
                continue;
            }
            stats.notes += 1;
            if let Some(note) = cache.get(&path) {
                stats.links += note.links.len();
                stats.embeds += note.embeds().count();
                stats.words += note.word_count;
                stats.tasks += note.tasks.len();
                stats.tasks_done += note.tasks.iter().filter(|t| t.is_done()).count();
                stats.tag_occurrences += note.tags.len();
                tags.extend(note.tags.iter().map(|t| t.name.to_lowercase()));
            }
            for link in cache.outgoing(&path) {
                if link.target.is_some() {
                    stats.resolved_links += 1;
                } else {
                    stats.unresolved_links += 1;
                }
            }
        }
        stats.tags = tags.len();
        stats.orphans = cache.orphans().len();
        stats
    }
}

pub fn format_stats(vault_name: &str, s: &Stats) -> String {
    let mut out = String::new();
    let rows = [
        ("Vault", vault_name.to_string()),
        ("Notes", s.notes.to_string()),
        ("Attachments", s.attachments.to_string()),
        (
            "Links",
            format!(
                "{} ({} resolved, {} unresolved; {} embeds)",
                s.links, s.resolved_links, s.unresolved_links, s.embeds
            ),
        ),
        (
            "Tags",
            format!("{} distinct, {} occurrences", s.tags, s.tag_occurrences),
        ),
        ("Orphans", s.orphans.to_string()),
        ("Words", s.words.to_string()),
        ("Tasks", format!("{} ({} done)", s.tasks, s.tasks_done)),
    ];
    for (k, v) in rows {
        let _ = writeln!(out, "{:<12} {v}", format!("{k}:"));
    }
    out
}

// ---------------------------------------------------------------- links

/// `#Heading#Sub` or `#^block`.
pub fn subpath_text(subpath: &Option<Subpath>) -> String {
    match subpath {
        None => String::new(),
        Some(Subpath::Heading(parts)) => format!("#{}", parts.join("#")),
        Some(Subpath::Block(id)) => format!("#^{id}"),
    }
}

/// Reconstruct the markup of a link, e.g. `![[Note#Heading|alias]]`.
pub fn link_markup(link: &Link) -> String {
    let bang = if link.is_embed { "!" } else { "" };
    let sub = subpath_text(&link.subpath);
    match link.kind {
        LinkKind::Wiki | LinkKind::Frontmatter => match &link.display {
            Some(d) => format!("{bang}[[{}{sub}|{d}]]", link.target),
            None => format!("{bang}[[{}{sub}]]", link.target),
        },
        LinkKind::Markdown => format!(
            "{bang}[{}]({}{sub})",
            link.display.as_deref().unwrap_or(""),
            link.target.replace(' ', "%20")
        ),
    }
}

pub fn format_backlinks(backlinks: &[Backlink]) -> String {
    let mut out = String::new();
    for b in backlinks {
        let _ = writeln!(
            out,
            "{}:{}: {}",
            b.source,
            b.link.line + 1,
            link_markup(&b.link)
        );
    }
    out
}

#[derive(Debug, Serialize)]
pub struct LinkJson {
    pub line: usize,
    pub target: String,
    pub subpath: Option<String>,
    pub display: Option<String>,
    pub embed: bool,
    pub kind: &'static str,
    pub markup: String,
}

impl From<&Link> for LinkJson {
    fn from(l: &Link) -> Self {
        Self {
            line: l.line + 1,
            target: l.target.clone(),
            subpath: l.subpath.as_ref().map(|_| subpath_text(&l.subpath)),
            display: l.display.clone(),
            embed: l.is_embed,
            kind: match l.kind {
                LinkKind::Wiki => "wiki",
                LinkKind::Markdown => "markdown",
                LinkKind::Frontmatter => "frontmatter",
            },
            markup: link_markup(l),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BacklinkJson {
    pub source: NotePath,
    #[serde(flatten)]
    pub link: LinkJson,
}

pub fn backlinks_json(backlinks: &[Backlink]) -> Vec<BacklinkJson> {
    backlinks
        .iter()
        .map(|b| BacklinkJson {
            source: b.source.clone(),
            link: (&b.link).into(),
        })
        .collect()
}

pub fn format_links(links: &[ResolvedLink]) -> String {
    let mut out = String::new();
    for l in links {
        let markup = link_markup(&l.link);
        let line = l.link.line + 1;
        match &l.target {
            Some(t) => {
                let _ = writeln!(out, "{line:>4} ✓ {markup} → {t}");
            }
            None => {
                let _ = writeln!(out, "{line:>4} ✗ {markup}");
            }
        }
    }
    out
}

#[derive(Debug, Serialize)]
pub struct ResolvedLinkJson {
    #[serde(flatten)]
    pub link: LinkJson,
    pub resolved: Option<NotePath>,
}

pub fn links_json(links: &[ResolvedLink]) -> Vec<ResolvedLinkJson> {
    links
        .iter()
        .map(|l| ResolvedLinkJson {
            link: (&l.link).into(),
            resolved: l.target.clone(),
        })
        .collect()
}

/// An unresolved target with the notes linking to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnresolvedTarget {
    pub target: String,
    pub sources: Vec<NotePath>,
}

/// Invert `source -> targets` into `target -> sources`, sorted by target
/// (case-insensitive).
pub fn group_unresolved(map: &BTreeMap<NotePath, Vec<String>>) -> Vec<UnresolvedTarget> {
    let mut by_target: BTreeMap<String, BTreeSet<NotePath>> = BTreeMap::new();
    for (source, targets) in map {
        for t in targets {
            by_target
                .entry(t.clone())
                .or_default()
                .insert(source.clone());
        }
    }
    let mut groups: Vec<UnresolvedTarget> = by_target
        .into_iter()
        .map(|(target, sources)| UnresolvedTarget {
            target,
            sources: sources.into_iter().collect(),
        })
        .collect();
    groups.sort_by(|a, b| {
        a.target
            .to_lowercase()
            .cmp(&b.target.to_lowercase())
            .then_with(|| a.target.cmp(&b.target))
    });
    groups
}

pub fn format_unresolved(groups: &[UnresolvedTarget]) -> String {
    let mut out = String::new();
    for g in groups {
        let sources: Vec<&str> = g.sources.iter().map(NotePath::as_str).collect();
        let _ = writeln!(out, "{} ← {}", g.target, sources.join(", "));
    }
    out
}

// ---------------------------------------------------------------- tags

pub fn format_tags(tags: &BTreeMap<String, usize>, by_count: bool) -> String {
    let mut entries: Vec<(&String, &usize)> = tags.iter().collect();
    if by_count {
        entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    }
    let width = entries
        .iter()
        .map(|(t, _)| t.chars().count() + 1)
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (tag, count) in entries {
        let _ = writeln!(out, "{:<width$}  {count}", format!("#{tag}"));
    }
    out
}

#[derive(Default)]
struct TagNode<'a> {
    children: BTreeMap<&'a str, TagNode<'a>>,
}

pub fn format_tag_tree(tags: &BTreeMap<String, usize>) -> String {
    let mut root = TagNode::default();
    for tag in tags.keys() {
        let mut node = &mut root;
        for part in tag.split('/').filter(|p| !p.is_empty()) {
            node = node.children.entry(part).or_default();
        }
    }
    fn walk(
        node: &TagNode,
        prefix: &str,
        depth: usize,
        tags: &BTreeMap<String, usize>,
        out: &mut String,
    ) {
        for (name, child) in &node.children {
            let full = if prefix.is_empty() {
                (*name).to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let count = tags.get(&full).copied().unwrap_or(0);
            let indent = "  ".repeat(depth);
            let label = if depth == 0 {
                format!("#{name}")
            } else {
                (*name).to_string()
            };
            let _ = writeln!(out, "{indent}{label} ({count})");
            walk(child, &full, depth + 1, tags, out);
        }
    }
    let mut out = String::new();
    walk(&root, "", 0, tags, &mut out);
    out
}

// ---------------------------------------------------------------- misc lists

pub fn format_paths(paths: &[NotePath]) -> String {
    let mut out = String::new();
    for p in paths {
        let _ = writeln!(out, "{p}");
    }
    out
}

pub fn format_find(found: &[(NotePath, Option<String>, i64)]) -> String {
    let mut out = String::new();
    for (path, alias, _) in found {
        match alias {
            Some(a) => {
                let _ = writeln!(out, "{path}  (alias: {a})");
            }
            None => {
                let _ = writeln!(out, "{path}");
            }
        }
    }
    out
}

#[derive(Debug, Serialize)]
pub struct FindJson {
    pub path: NotePath,
    pub alias: Option<String>,
    pub score: i64,
}

pub fn find_json(found: &[(NotePath, Option<String>, i64)]) -> Vec<FindJson> {
    found
        .iter()
        .map(|(path, alias, score)| FindJson {
            path: path.clone(),
            alias: alias.clone(),
            score: *score,
        })
        .collect()
}

// ---------------------------------------------------------------- search

pub fn format_search(results: &[SearchResult], color: bool) -> String {
    let mut out = String::new();
    for r in results {
        if r.matches.is_empty() {
            let _ = writeln!(out, "{}", r.path);
        }
        for m in &r.matches {
            let _ = writeln!(
                out,
                "{}:{}: {}",
                r.path,
                m.line + 1,
                highlight(&m.line_text, &m.ranges, color)
            );
        }
    }
    out
}

#[derive(Debug, Serialize)]
pub struct SearchMatchJson {
    /// 1-based.
    pub line: usize,
    pub text: String,
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Serialize)]
pub struct SearchResultJson {
    pub path: NotePath,
    pub score: u32,
    pub matches: Vec<SearchMatchJson>,
}

pub fn search_json(results: &[SearchResult]) -> Vec<SearchResultJson> {
    results
        .iter()
        .map(|r| SearchResultJson {
            path: r.path.clone(),
            score: r.score,
            matches: r
                .matches
                .iter()
                .map(|m| SearchMatchJson {
                    line: m.line + 1,
                    text: m.line_text.clone(),
                    ranges: m.ranges.clone(),
                })
                .collect(),
        })
        .collect()
}

// ---------------------------------------------------------------- rename

/// Destination path for a rename: `to` as a vault path, with the extension
/// of `from` added when `to` has none (`"New"` → `"New.md"`).
pub fn rename_target(from: &NotePath, to: &str) -> Result<NotePath> {
    let to = to.trim();
    let mut target = NotePath::new(to).map_err(|_| anyhow!("invalid destination path: {to:?}"))?;
    let ext = from.extension();
    if !ext.is_empty() && target.extension() != ext {
        let with_ext = format!("{target}.{ext}");
        target = NotePath::new(&with_ext)
            .map_err(|_| anyhow!("invalid destination path: {with_ext:?}"))?;
    }
    Ok(target)
}

pub fn rename(ctx: &mut Context, from: &str, to: &str, dry_run: bool) -> Result<String> {
    use ors_core::VaultRead;
    let from = ctx.resolve_note(from)?;
    let to = rename_target(&from, to)?;
    if from == to {
        bail!("{from} already has this name");
    }
    if ctx.vault.exists(&to) || ctx.cache.files().contains(&to) {
        bail!("destination already exists: {to}");
    }
    let mut out = String::new();
    if dry_run {
        let sources: BTreeSet<NotePath> = ctx
            .cache
            .backlinks(&from)
            .into_iter()
            .map(|b| b.source)
            .collect();
        let _ = writeln!(out, "Would rename {from} → {to}");
        let _ = writeln!(out, "Would update links in {} note(s):", sources.len());
        for s in sources {
            let _ = writeln!(out, "  {s}");
        }
    } else {
        let changed = ors_index::rename_with_links(&ctx.vault, &mut ctx.cache, &from, &to)
            .map_err(|e| anyhow!("cannot rename {from} to {to}: {e}"))?;
        let _ = writeln!(out, "Renamed {from} → {to}");
        let _ = writeln!(out, "Updated links in {} note(s):", changed.len());
        for s in changed {
            let _ = writeln!(out, "  {s}");
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ors_core::Span;
    use ors_index::SearchMatch;

    fn p(s: &str) -> NotePath {
        NotePath::new(s).unwrap()
    }

    fn link(target: &str, kind: LinkKind, line: usize) -> Link {
        Link {
            target: target.into(),
            subpath: None,
            display: None,
            kind,
            is_embed: false,
            span: Span::new(0, 0),
            line,
        }
    }

    #[test]
    fn stats_format() {
        let s = Stats {
            notes: 7,
            attachments: 1,
            links: 18,
            embeds: 3,
            resolved_links: 16,
            unresolved_links: 2,
            tags: 6,
            tag_occurrences: 10,
            orphans: 2,
            words: 100,
            tasks: 4,
            tasks_done: 1,
        };
        let text = format_stats("vault", &s);
        assert!(text.contains("Notes:       7\n"), "{text}");
        assert!(text.contains("Links:       18 (16 resolved, 2 unresolved; 3 embeds)"));
        assert!(text.contains("Tags:        6 distinct, 10 occurrences"));
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["unresolved_links"], 2);
    }

    #[test]
    fn stats_of_empty_cache() {
        let vault = ors_core::MemoryVault::new();
        let cache = MetadataCache::build(&vault).unwrap();
        assert_eq!(Stats::compute(&cache), Stats::default());
    }

    #[test]
    fn markup() {
        let mut l = link("Note", LinkKind::Wiki, 0);
        assert_eq!(link_markup(&l), "[[Note]]");
        l.subpath = Some(Subpath::Heading(vec!["A".into(), "B".into()]));
        l.display = Some("x".into());
        l.is_embed = true;
        assert_eq!(link_markup(&l), "![[Note#A#B|x]]");
        let mut m = link("Projets/Rust notes.md", LinkKind::Markdown, 0);
        m.subpath = Some(Subpath::Block("id".into()));
        m.display = Some("Rust".into());
        assert_eq!(link_markup(&m), "[Rust](Projets/Rust%20notes.md#^id)");
    }

    #[test]
    fn backlinks_and_links_format() {
        let b = vec![Backlink {
            source: p("Accueil.md"),
            link: link("Idées", LinkKind::Wiki, 5),
        }];
        assert_eq!(format_backlinks(&b), "Accueil.md:6: [[Idées]]\n");
        let json = serde_json::to_value(backlinks_json(&b)).unwrap();
        assert_eq!(json[0]["source"], "Accueil.md");
        assert_eq!(json[0]["line"], 6);
        assert_eq!(json[0]["kind"], "wiki");

        let links = vec![
            ResolvedLink {
                link: link("Idées", LinkKind::Wiki, 5),
                target: Some(p("Idées.md")),
            },
            ResolvedLink {
                link: link("Note fantôme", LinkKind::Wiki, 7),
                target: None,
            },
        ];
        assert_eq!(
            format_links(&links),
            "   6 ✓ [[Idées]] → Idées.md\n   8 ✗ [[Note fantôme]]\n"
        );
        let json = serde_json::to_value(links_json(&links)).unwrap();
        assert_eq!(json[0]["resolved"], "Idées.md");
        assert!(json[1]["resolved"].is_null());
    }

    #[test]
    fn unresolved_grouping() {
        let mut map = BTreeMap::new();
        map.insert(p("b.md"), vec!["X".to_string(), "a".to_string()]);
        map.insert(p("a.md"), vec!["X".to_string(), "X".to_string()]);
        let groups = group_unresolved(&map);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].target, "a");
        assert_eq!(groups[1].sources, vec![p("a.md"), p("b.md")]);
        assert_eq!(format_unresolved(&groups), "a ← b.md\nX ← a.md, b.md\n");
    }

    #[test]
    fn tags_format() {
        let mut tags = BTreeMap::new();
        tags.insert("hub".to_string(), 2);
        tags.insert("projet".to_string(), 3);
        tags.insert("projet/obsirust".to_string(), 3);
        tags.insert("a/b/c".to_string(), 1);
        assert_eq!(
            format_tags(&tags, false),
            "#a/b/c            1\n#hub              2\n#projet           3\n#projet/obsirust  3\n"
        );
        assert!(format_tags(&tags, true).starts_with("#projet "));
        assert_eq!(
            format_tag_tree(&tags),
            "#a (0)\n  b (0)\n    c (1)\n#hub (2)\n#projet (3)\n  obsirust (3)\n"
        );
    }

    #[test]
    fn find_and_paths() {
        let found = vec![
            (p("Accueil.md"), Some("Home".to_string()), 10),
            (p("b.md"), None, 3),
        ];
        assert_eq!(format_find(&found), "Accueil.md  (alias: Home)\nb.md\n");
        assert_eq!(
            serde_json::to_value(find_json(&found)).unwrap()[0]["alias"],
            "Home"
        );
        assert_eq!(format_paths(&[p("a.md"), p("b/c.md")]), "a.md\nb/c.md\n");
    }

    #[test]
    fn search_format() {
        let results = vec![
            SearchResult {
                path: p("Projets/ObsiRust.md"),
                score: 3,
                matches: vec![SearchMatch {
                    line: 6,
                    line_text: "Portage d'Obsidian en Rust.".into(),
                    ranges: vec![(22, 26)],
                }],
            },
            SearchResult {
                path: p("Idées.md"),
                score: 1,
                matches: vec![],
            },
        ];
        assert_eq!(
            format_search(&results, false),
            "Projets/ObsiRust.md:7: Portage d'Obsidian en Rust.\nIdées.md\n"
        );
        assert!(format_search(&results, true).contains("en \x1b[1mRust\x1b[0m."));
        let json = serde_json::to_value(search_json(&results)).unwrap();
        assert_eq!(json[0]["matches"][0]["line"], 7);
        assert_eq!(json[0]["matches"][0]["ranges"][0][0], 22);
    }

    #[test]
    fn rename_targets() {
        let from = p("Projets/Rust notes.md");
        assert_eq!(
            rename_target(&from, "Projets/Rust").unwrap(),
            p("Projets/Rust.md")
        );
        assert_eq!(rename_target(&from, "Rust.md").unwrap(), p("Rust.md"));
        assert_eq!(rename_target(&from, "v1.2").unwrap(), p("v1.2.md"));
        assert!(rename_target(&from, "../x").is_err());
        let img = p("a/img.png");
        assert_eq!(rename_target(&img, "b/pic").unwrap(), p("b/pic.png"));
    }
}
