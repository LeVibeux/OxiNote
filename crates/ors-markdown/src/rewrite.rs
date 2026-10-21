//! Link rewriting (used when notes are renamed).

use ors_core::{Link, LinkKind};

use crate::scan::analyze;
use crate::util::{encode_markdown_target, ends_with_md};

pub(crate) fn rewrite(source: &str, mut f: impl FnMut(&Link) -> Option<String>) -> String {
    let a = analyze(source, false);
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for rec in &a.links {
        let Some((ts, te)) = rec.target_range else {
            continue;
        };
        let Some(mut new) = f(&rec.link) else {
            continue;
        };
        let orig_md = ends_with_md(&rec.link.target);
        let new_md = ends_with_md(&new);
        if orig_md && !new_md && !new.is_empty() {
            new.push_str(".md");
        } else if !orig_md && new_md {
            new.truncate(new.len() - 3);
        }
        let replacement = match rec.link.kind {
            LinkKind::Markdown => encode_markdown_target(&new),
            LinkKind::Wiki | LinkKind::Frontmatter => new,
        };
        if source.get(ts..te) != Some(replacement.as_str()) {
            edits.push((ts, te, replacement));
        }
    }
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(source.len());
    let mut pos = 0;
    for (s, e, r) in edits {
        if s < pos || e > source.len() {
            continue;
        }
        out.push_str(&source[pos..s]);
        out.push_str(&r);
        pos = e;
    }
    out.push_str(&source[pos..]);
    out
}
