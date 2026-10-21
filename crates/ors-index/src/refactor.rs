//! Vault-wide refactoring: rename/move a file and update every link to it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ors_core::{Error, Link, LinkKind, NotePath, Result, VaultWrite};

use crate::MetadataCache;
use crate::text::{
    folder_of, has_md, join_normalize, key, name_key, percent_encode_path, relative_path,
};

/// One planned link rewrite inside a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEdit {
    /// The link as it currently appears in the note (its span locates it).
    pub link: Link,
    /// New target text handed to [`ors_markdown::rewrite_links`]. For
    /// markdown links it is already percent-encoded (`My%20Note.md`).
    pub new_target: String,
}

/// The link rewrites a rename requires, per note (keyed by the note's path
/// *after* the rename).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenamePlan {
    /// Note -> edits in source order.
    pub edits: BTreeMap<NotePath, Vec<LinkEdit>>,
}

impl RenamePlan {
    /// Notes that will be modified.
    pub fn notes(&self) -> Vec<NotePath> {
        self.edits.keys().cloned().collect()
    }

    /// Total number of links to rewrite.
    pub fn link_count(&self) -> usize {
        self.edits.values().map(Vec::len).sum()
    }
}

/// Resolutions of the links of a note before the rename.
struct Snapshot {
    source: NotePath,
    links: Vec<(Link, Option<NotePath>)>,
}

/// Notes whose links may need rewriting when `from` becomes `to`: notes
/// linking to `from`, notes with links named like `to` (they may be captured
/// by the new file), and `from` itself (its relative links move with it).
fn snapshot(cache: &MetadataCache, from: &NotePath, to: &NotePath) -> Vec<Snapshot> {
    let mut sources: BTreeSet<NotePath> = BTreeSet::new();
    if let Some(s) = cache.backrefs.get(from) {
        sources.extend(s.iter().cloned());
    }
    if let Some(s) = cache.link_names.get(&name_key(to.as_str())) {
        sources.extend(s.iter().cloned());
    }
    if cache.get(from).is_some() {
        sources.insert(from.clone());
    }
    sources
        .into_iter()
        .map(|source| {
            let links = cache
                .outgoing(&source)
                .into_iter()
                .map(|r| (r.link, r.target))
                .collect();
            Snapshot { source, links }
        })
        .collect()
}

fn compute_edits(
    cache: &MetadataCache,
    snaps: &[Snapshot],
    from: &NotePath,
    to: &NotePath,
) -> RenamePlan {
    let mut plan = RenamePlan::default();
    for snap in snaps {
        let src_post = if &snap.source == from {
            to
        } else {
            &snap.source
        };
        if cache.get(src_post).is_none() {
            continue;
        }
        for (link, old) in &snap.links {
            let Some(old) = old else { continue };
            if link.target.trim().is_empty() {
                continue;
            }
            let desired = if old == from { to } else { old };
            if cache.resolve_link(link, src_post).as_ref() == Some(desired) {
                continue;
            }
            let new_target = styled_target(cache, link, &snap.source, src_post, old, desired);
            plan.edits
                .entry(src_post.clone())
                .or_default()
                .push(LinkEdit {
                    link: link.clone(),
                    new_target,
                });
        }
    }
    plan
}

/// New link text pointing at `desired` from `src_post`, keeping the style of
/// the original link (basename / full path / relative path, `.md` or not).
fn styled_target(
    cache: &MetadataCache,
    link: &Link,
    src_pre: &NotePath,
    src_post: &NotePath,
    old: &NotePath,
    desired: &NotePath,
) -> String {
    let t = link.target.trim().replace('\\', "/");
    let had_md = has_md(&t);
    let strip = |p: &NotePath| -> String {
        if p.is_markdown() && !had_md {
            p.without_extension().to_string()
        } else {
            p.as_str().to_string()
        }
    };
    let full = strip(desired);
    let base = full.rsplit('/').next().unwrap_or(&full).to_string();
    let post_folder = folder_of(src_post.as_str());
    let dot_prefix = t.starts_with("./");
    let rel = {
        let r = relative_path(post_folder, &full);
        if dot_prefix && !r.starts_with("../") {
            format!("./{r}")
        } else {
            r
        }
    };
    let explicit_relative = t.starts_with("./") || t.starts_with("../");
    let candidates: Vec<String> = match link.kind {
        LinkKind::Markdown => {
            let pre_folder = folder_of(src_pre.as_str());
            let old_key = key(old.as_str());
            let was_relative = explicit_relative
                || (!pre_folder.is_empty()
                    && join_normalize(pre_folder, &t).is_some_and(|j| {
                        let jk = key(&j);
                        jk == old_key || format!("{jk}.md") == old_key
                    }));
            if was_relative {
                vec![rel, full]
            } else if t.contains('/') {
                vec![full, rel]
            } else {
                vec![base, rel, full]
            }
        }
        LinkKind::Wiki | LinkKind::Frontmatter => {
            if explicit_relative {
                vec![rel, full]
            } else if t.trim_start_matches('/').contains('/') {
                vec![full, base]
            } else {
                vec![base, full]
            }
        }
    };
    let chosen = candidates
        .iter()
        .find(|c| cache.resolve_target(c, link.kind, src_post).as_ref() == Some(desired))
        .or_else(|| candidates.first())
        .cloned()
        .unwrap_or_default();
    if link.kind == LinkKind::Markdown {
        percent_encode_path(&chosen)
    } else {
        chosen
    }
}

/// Compute the link rewrites renaming `from` to `to` would require, without
/// touching the vault. The cache is temporarily updated and restored.
/// Fails with [`Error::NotFound`] / [`Error::AlreadyExists`] like a rename.
pub fn plan_rename(
    cache: &mut MetadataCache,
    from: &NotePath,
    to: &NotePath,
) -> Result<RenamePlan> {
    if from == to {
        return Ok(RenamePlan::default());
    }
    if !cache.contains(from) {
        return Err(Error::NotFound(from.clone()));
    }
    if cache.contains(to) {
        return Err(Error::AlreadyExists(to.clone()));
    }
    let snaps = snapshot(cache, from, to);
    cache.rename_file(from, to);
    let plan = compute_edits(cache, &snaps, from, to);
    cache.rename_file(to, from);
    Ok(plan)
}

fn apply_plan(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    plan: &RenamePlan,
) -> Result<Vec<NotePath>> {
    let mut changed = Vec::new();
    for (path, edits) in &plan.edits {
        let source = vault.read_to_string(path)?;
        let by_span: HashMap<(usize, usize), (&str, &str)> = edits
            .iter()
            .map(|e| {
                (
                    (e.link.span.start, e.link.span.end),
                    (e.link.target.as_str(), e.new_target.as_str()),
                )
            })
            .collect();
        let new_source = ors_markdown::rewrite_links(&source, |l| {
            by_span
                .get(&(l.span.start, l.span.end))
                .filter(|(old, _)| *old == l.target)
                .map(|(_, new)| (*new).to_string())
        });
        if new_source != source {
            vault.write(path, &new_source)?;
            cache.update_from_source(path, &new_source);
            changed.push(path.clone());
        }
    }
    Ok(changed)
}

/// Rename/move `from` to `to` on disk and rewrite every link pointing to it in
/// the vault (wikilinks, markdown links, embeds, frontmatter links), then update
/// `cache`. Returns the notes whose contents were changed (paths after the
/// rename), sorted.
///
/// Link style is preserved: basename links stay basenames when still
/// unambiguous, full-path links get the new full path, relative markdown links
/// stay relative (and are percent-encoded). Links that the new file would
/// otherwise capture (another note with the same name) are rewritten to keep
/// pointing at their previous target, and relative links inside the moved
/// note are fixed.
pub fn rename_with_links(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    from: &NotePath,
    to: &NotePath,
) -> Result<Vec<NotePath>> {
    if from == to {
        return Ok(Vec::new());
    }
    if !vault.exists(from) {
        return Err(Error::NotFound(from.clone()));
    }
    if !cache.contains(from) {
        cache.update_file(vault, from)?;
    }
    let snaps = snapshot(cache, from, to);
    vault.rename(from, to)?;
    cache.rename_file(from, to);
    let plan = compute_edits(cache, &snaps, from, to);
    apply_plan(vault, cache, &plan)
}

/// Move every file under `from_folder` to `to_folder` (vault-relative, `/`
/// separated), updating links like [`rename_with_links`]. Returns the changed
/// notes (final paths), sorted. Empty source folders are left on disk.
pub fn rename_folder_with_links(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    from_folder: &str,
    to_folder: &str,
) -> Result<Vec<NotePath>> {
    let from_f = from_folder.trim_matches('/');
    let to_f = to_folder.trim_matches('/');
    if from_f.is_empty() {
        return Err(Error::InvalidPath(from_folder.to_string()));
    }
    if from_f == to_f {
        return Ok(Vec::new());
    }
    if to_f.starts_with(&format!("{from_f}/")) {
        return Err(Error::InvalidPath(format!(
            "cannot move {from_f} into its own subfolder {to_f}"
        )));
    }
    let prefix = format!("{from_f}/");
    let files: Vec<NotePath> = vault
        .list_files()?
        .into_iter()
        .filter(|p| p.as_str().starts_with(&prefix))
        .collect();
    // Check every destination before moving anything, so a clash never
    // leaves the folder half moved.
    let mut moves = Vec::with_capacity(files.len());
    for f in files {
        let to = NotePath::join(to_f, &f.as_str()[prefix.len()..])?;
        if vault.exists(&to) {
            return Err(Error::AlreadyExists(to));
        }
        moves.push((f, to));
    }
    let mut changed: BTreeSet<NotePath> = BTreeSet::new();
    for (f, to) in moves {
        let now = rename_with_links(vault, cache, &f, &to)?;
        if changed.remove(&f) {
            changed.insert(to.clone());
        }
        changed.extend(now);
    }
    Ok(changed.into_iter().collect())
}
