//! Vault-wide views of properties and tags (for the "All properties" and
//! "Tags" panes and for autocomplete) and note edits that keep the
//! [`MetadataCache`] in sync.
//!
//! Property keys are compared ASCII-case-insensitively everywhere (Obsidian
//! treats `Status` and `status` as one property); tags are compared with the
//! cache's tag keys (Unicode NFC + lowercase).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ors_core::{Error, Frontmatter, NotePath, ParsedNote, Result, VaultRead, VaultWrite};
use ors_markdown::properties as props;
use serde_json::Value;

use crate::MetadataCache;
use crate::cache::tag_key;

/// One property key across the vault (an entry of the "All properties" pane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyInfo {
    /// Spelling of the key as first seen (notes in path order).
    pub name: String,
    /// Number of notes having the property.
    pub count: usize,
    /// Most common inferred type among the non-empty values
    /// ([`ors_markdown::properties::infer_type_name`]: `"text"`,
    /// `"multitext"`, `"number"`, `"checkbox"`, `"date"`, `"datetime"`);
    /// `"text"` if every value is empty. Ties go to the first type in that
    /// list. An explicit type from `.obsidian/types.json` (see
    /// `ors_vault::Vault::property_types`) should take precedence in a UI.
    pub inferred_type: &'static str,
    /// Inferred type name → number of notes (non-empty values only).
    pub type_counts: BTreeMap<&'static str, usize>,
}

/// A node of the tag tree (Tags pane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNode {
    /// Last segment, e.g. `"rust"` for `projet/rust` (case as first seen).
    pub name: String,
    /// Full nested name without `#`, e.g. `"projet/rust"` (case as first seen).
    pub full_name: String,
    /// Number of notes containing this tag or one of its children.
    pub count: usize,
    /// Child tags sorted by name (case-insensitive).
    pub children: Vec<TagNode>,
}

const TYPE_ORDER: [&str; 6] = [
    "text",
    "multitext",
    "number",
    "checkbox",
    "date",
    "datetime",
];

/// Value of `key` in a frontmatter (exact key first, then case-insensitive).
fn get_ci<'a>(fm: &'a Frontmatter, key: &str) -> Option<&'a Value> {
    fm.get(key).or_else(|| {
        fm.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    })
}

/// Display string of a scalar value (`None` for null / empty strings).
fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}

fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn value_matches(v: &Value, wanted: &Value) -> bool {
    if v == wanted {
        return true;
    }
    if let Value::Array(items) = v
        && !wanted.is_array()
    {
        return items.iter().any(|i| value_matches(i, wanted));
    }
    match (value_text(v), value_text(wanted)) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => false,
    }
}

impl MetadataCache {
    fn notes_iter(&self) -> impl Iterator<Item = (&NotePath, &ParsedNote)> {
        self.files
            .iter()
            .filter_map(|(p, d)| d.as_ref().map(|d| (p, &d.parsed)))
    }

    /// Every property key used in the vault, keyed by lowercased key (like
    /// Obsidian's "All properties" pane).
    pub fn properties(&self) -> BTreeMap<String, PropertyInfo> {
        let mut out: BTreeMap<String, PropertyInfo> = BTreeMap::new();
        for (_, note) in self.notes_iter() {
            let Some(fm) = &note.frontmatter else {
                continue;
            };
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for (k, v) in fm {
                let lk = k.to_lowercase();
                if !seen.insert(lk.clone()) {
                    continue;
                }
                let info = out.entry(lk).or_insert_with(|| PropertyInfo {
                    name: k.clone(),
                    count: 0,
                    inferred_type: "text",
                    type_counts: BTreeMap::new(),
                });
                info.count += 1;
                if !is_empty_value(v) {
                    *info
                        .type_counts
                        .entry(props::infer_type_name(v))
                        .or_default() += 1;
                }
            }
        }
        for info in out.values_mut() {
            let mut best = ("text", 0);
            for t in TYPE_ORDER {
                let n = info.type_counts.get(t).copied().unwrap_or(0);
                if n > best.1 {
                    best = (t, n);
                }
            }
            info.inferred_type = best.0;
        }
        out
    }

    /// Distinct values of property `key` (case-insensitive key) with the
    /// number of notes having each; list values are flattened. Scalars are
    /// shown as text (`3`, `true`), objects as JSON; empty values are
    /// skipped. Used for value autocomplete.
    pub fn property_values(&self, key: &str) -> BTreeMap<String, usize> {
        let mut out: BTreeMap<String, usize> = BTreeMap::new();
        for (_, note) in self.notes_iter() {
            let Some(v) = note.frontmatter.as_ref().and_then(|fm| get_ci(fm, key)) else {
                continue;
            };
            let mut vals: BTreeSet<String> = BTreeSet::new();
            match v {
                Value::Array(items) => vals.extend(items.iter().filter_map(value_text)),
                other => vals.extend(value_text(other)),
            }
            for s in vals {
                *out.entry(s).or_default() += 1;
            }
        }
        out
    }

    /// Notes having property `key` (case-insensitive), sorted. With
    /// `value`, only notes whose value equals it, contains it (lists), or has
    /// the same text ignoring case.
    pub fn files_with_property(&self, key: &str, value: Option<&Value>) -> Vec<NotePath> {
        self.notes_iter()
            .filter(|(_, n)| {
                n.frontmatter
                    .as_ref()
                    .and_then(|fm| get_ci(fm, key))
                    .is_some_and(|v| value.is_none_or(|w| value_matches(v, w)))
            })
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Display spelling (first seen) of every tag key, including parents.
    fn tag_display_names(&self) -> HashMap<String, String> {
        let mut names: HashMap<String, String> = HashMap::new();
        for (_, note) in self.notes_iter() {
            for t in &note.tags {
                let name = t.name.trim_end_matches('/');
                for (i, _) in name
                    .match_indices('/')
                    .chain(std::iter::once((name.len(), "")))
                {
                    let prefix = &name[..i];
                    if !prefix.is_empty() {
                        names
                            .entry(tag_key(prefix))
                            .or_insert_with(|| prefix.to_string());
                    }
                }
            }
        }
        names
    }

    /// Tags actually used in the vault (not synthesized parents) → number
    /// of notes using them, keyed by display spelling.
    fn used_tags(&self) -> Vec<(String, usize)> {
        let names = self.tag_display_names();
        let mut used: BTreeSet<String> = BTreeSet::new();
        for (_, note) in self.notes_iter() {
            for t in &note.tags {
                used.insert(tag_key(&t.name));
            }
        }
        used.into_iter()
            .map(|k| {
                let n = self.tag_files.get(&k).map_or(0, BTreeMap::len);
                let display = names.get(&k).cloned().unwrap_or_else(|| k.clone());
                (display, n)
            })
            .collect()
    }

    /// Tag autocomplete for `#prefix`: tags used in the vault whose full
    /// nested name matches `prefix` case-insensitively (a leading `#` is
    /// ignored). Matches at the start of the name come first, then matches
    /// at the start of a nested segment (`rust` → `projet/rust`), then other
    /// substring matches; within a group by number of notes (descending),
    /// then name. An empty prefix lists every tag. Counts are numbers of
    /// notes having the tag or a child.
    pub fn tag_suggestions(&self, prefix: &str, limit: usize) -> Vec<(String, usize)> {
        let q = tag_key(prefix);
        let mut scored: Vec<(u8, String, usize)> = self
            .used_tags()
            .into_iter()
            .filter_map(|(name, n)| {
                let k = tag_key(&name);
                let rank = if k.starts_with(&q) {
                    0
                } else if k.split('/').skip(1).any(|seg| seg.starts_with(&q))
                    || k.contains(&format!("/{q}"))
                {
                    1
                } else if k.contains(&q) {
                    2
                } else {
                    return None;
                };
                Some((rank, name, n))
            })
            .collect();
        scored.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(b.2.cmp(&a.2))
                .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, name, n)| (name, n))
            .collect()
    }

    /// The tag hierarchy for the Tags pane: root tags sorted by name, each
    /// with its nested children. Counts are numbers of notes containing the
    /// tag or one of its children.
    pub fn tag_tree(&self) -> Vec<TagNode> {
        let names = self.tag_display_names();
        let mut keys: Vec<&String> = names.keys().collect();
        keys.sort();
        let mut roots: Vec<TagNode> = Vec::new();
        for k in keys {
            let segments: Vec<&str> = k.split('/').collect();
            let mut level = &mut roots;
            for depth in 0..segments.len() {
                let full_key = segments[..=depth].join("/");
                let pos = level.iter().position(|n| tag_key(&n.full_name) == full_key);
                let idx = match pos {
                    Some(i) => i,
                    None => {
                        let full_name = names
                            .get(&full_key)
                            .cloned()
                            .unwrap_or_else(|| full_key.clone());
                        let name = full_name
                            .rsplit('/')
                            .next()
                            .unwrap_or(&full_name)
                            .to_string();
                        level.push(TagNode {
                            name,
                            full_name,
                            count: self.tag_files.get(&full_key).map_or(0, BTreeMap::len),
                            children: Vec::new(),
                        });
                        level.len() - 1
                    }
                };
                level = &mut level[idx].children;
            }
        }
        sort_tree(&mut roots);
        roots
    }
}

fn sort_tree(nodes: &mut [TagNode]) {
    nodes.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    for n in nodes {
        sort_tree(&mut n.children);
    }
}

// ----------------------------------------------------------------- edits

/// Apply `edit` to the source of note `path`; if the text changes, write it
/// and re-index the note. Returns whether the note changed.
///
/// # Errors
///
/// Read/write errors, and any error returned by `edit` (nothing is written
/// then).
pub fn edit_note(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    path: &NotePath,
    edit: impl FnOnce(&str) -> Result<String>,
) -> Result<bool> {
    if !path.is_markdown() {
        return Err(Error::InvalidPath(format!("{path} is not a markdown note")));
    }
    let source = vault.read_to_string(path)?;
    let new = edit(&source)?;
    if new == source {
        return Ok(false);
    }
    vault.write(path, &new)?;
    cache.update_from_source(path, &new);
    Ok(true)
}

fn changed(path: &NotePath, did: bool) -> Vec<NotePath> {
    if did { vec![path.clone()] } else { Vec::new() }
}

/// Set property `key` of note `path` (see
/// [`ors_markdown::properties::set_property`]). Returns `[path]` if the note
/// changed, `[]` otherwise.
pub fn set_property_in(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    path: &NotePath,
    key: &str,
    value: &Value,
) -> Result<Vec<NotePath>> {
    let did = edit_note(vault, cache, path, |s| props::set_property(s, key, value))?;
    Ok(changed(path, did))
}

/// [`set_property_in`] with [`props::WriteOptions`] (e.g. to quote
/// date-like strings of a text property).
pub fn set_property_in_with(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    path: &NotePath,
    key: &str,
    value: &Value,
    opts: &props::WriteOptions,
) -> Result<Vec<NotePath>> {
    let did = edit_note(vault, cache, path, |s| {
        props::set_property_with(s, key, value, opts)
    })?;
    Ok(changed(path, did))
}

/// Remove property `key` from note `path`.
pub fn remove_property_in(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    path: &NotePath,
    key: &str,
) -> Result<Vec<NotePath>> {
    let did = edit_note(vault, cache, path, |s| props::remove_property(s, key))?;
    Ok(changed(path, did))
}

/// Add tag `tag` to the frontmatter of note `path`.
pub fn add_tag_in(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    path: &NotePath,
    tag: &str,
) -> Result<Vec<NotePath>> {
    let did = edit_note(vault, cache, path, |s| props::add_tag(s, tag))?;
    Ok(changed(path, did))
}

/// Remove tag `tag` from note `path` (frontmatter and inline occurrences).
pub fn remove_tag_in(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    path: &NotePath,
    tag: &str,
) -> Result<Vec<NotePath>> {
    let did = edit_note(vault, cache, path, |s| props::remove_tag(s, tag))?;
    Ok(changed(path, did))
}

/// Rename property `old` to `new` in every note having it
/// (case-insensitive). Notes where the edit is impossible (another entry
/// already named `new`, unsupported YAML layout) are left unchanged; they
/// still show up in `cache.files_with_property(old, None)` afterwards.
/// Returns the changed notes.
///
/// # Errors
///
/// [`Error::Other`] for an empty `new`; read/write errors.
pub fn rename_property_everywhere(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    old: &str,
    new: &str,
) -> Result<Vec<NotePath>> {
    if new.trim().is_empty() {
        return Err(Error::Other("empty property name".into()));
    }
    let mut out = Vec::new();
    for path in cache.files_with_property(old, None) {
        let source = vault.read_to_string(&path)?;
        let Ok(updated) = props::rename_property(&source, old, new) else {
            continue;
        };
        if updated != source {
            vault.write(&path, &updated)?;
            cache.update_from_source(&path, &updated);
            out.push(path);
        }
    }
    Ok(out)
}

fn check_tag_rename(old: &str, new: &str) -> Result<()> {
    // Validates both names without touching any note.
    props::rename_tag("", old, new).map(|_| ())
}

/// Notes that [`rename_tag`] would change, with their new sources (nothing
/// is written).
///
/// # Errors
///
/// [`Error::Other`] for an empty `old` or invalid `new`; read errors.
pub fn plan_tag_rename(
    vault: &dyn VaultRead,
    cache: &MetadataCache,
    old: &str,
    new: &str,
) -> Result<Vec<(NotePath, String)>> {
    check_tag_rename(old, new)?;
    let mut out = Vec::new();
    for path in cache.files_with_tag(old) {
        let source = vault.read_to_string(&path)?;
        let updated = props::rename_tag(&source, old, new)?;
        if updated != source {
            out.push((path, updated));
        }
    }
    Ok(out)
}

/// Rename tag `old` (and its nested children) to `new` in every note,
/// inline and in frontmatter (see
/// [`ors_markdown::properties::rename_tag`]). Renaming into an existing tag
/// merges them (frontmatter lists are deduplicated). Returns the changed
/// notes; the cache is updated.
///
/// # Errors
///
/// [`Error::Other`] for an empty `old` or invalid `new` (checked before any
/// write); read/write errors.
pub fn rename_tag(
    vault: &dyn VaultWrite,
    cache: &mut MetadataCache,
    old: &str,
    new: &str,
) -> Result<Vec<NotePath>> {
    let plan = plan_tag_rename(vault, cache, old, new)?;
    let mut out = Vec::with_capacity(plan.len());
    for (path, updated) in plan {
        vault.write(&path, &updated)?;
        cache.update_from_source(&path, &updated);
        out.push(path);
    }
    Ok(out)
}
