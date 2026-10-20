//! Obsidian **Properties**: reading and source-preserving editing of the YAML
//! frontmatter, plus tag and alias helpers.
//!
//! Every editing function takes the whole note source and returns the new
//! source. Edits are *surgical*: only the lines of the property being changed
//! are rewritten; every other byte of the note (comments, key order, blank
//! lines, quoting style of other properties, the body) is kept verbatim.
//!
//! # Frontmatter layout model
//!
//! A *property entry* is a top-level key line (a line starting at column 0
//! with `key:` or `"quoted key":`) plus its continuation lines: indented lines,
//! block-list items written at column 0 (`- a`) and blank lines inside the
//! value. Trailing blank lines and column-0 `#` comments after an entry are not
//! part of it, so they survive edits of the entry.
//!
//! # Serialization (like Obsidian)
//!
//! * strings are written plain when that round-trips as the same string,
//!   otherwise double-quoted (JSON escapes, which are valid YAML): strings
//!   containing `[[`, `: `, ` #`, starting with a YAML indicator, looking like
//!   numbers / booleans / null, with surrounding whitespace or line breaks;
//!   date-like strings are written plain unless
//!   [`WriteOptions::quote_date_strings`] is set;
//! * numbers and booleans as YAML scalars;
//! * `null` and empty lists as an empty value (`key:`), as Obsidian does
//!   (reading such a property back gives `null`);
//! * non-empty lists in block style with a two-space indent
//!   (`key:\n  - a\n  - b`); list items that are lists or objects are written
//!   in JSON flow style (`- {"a": 1}`), which is valid YAML;
//! * objects as nested block mappings (empty object: `key: {}`).
//!
//! New lines use the line ending of the frontmatter's opening fence (or of the
//! first line of the note when there is no frontmatter yet), so CRLF notes
//! stay CRLF.
//!
//! # Errors
//!
//! Editing functions return [`ors_core::Error::Parse`] when the existing
//! frontmatter is not valid YAML (or not a mapping): Obsidian refuses to edit
//! such properties too, and rewriting text we cannot parse could destroy data.
//! The tag helpers that also touch the body ([`remove_tag`], [`rename_tag`])
//! still edit inline tags in that case and leave the frontmatter alone.
//!
//! # Key matching
//!
//! Property keys are looked up exactly first, then ASCII-case-insensitively
//! (Obsidian treats `Status` and `status` as the same property). When an
//! existing entry is replaced, its spelling in the file is kept.

use ors_core::{Error, Frontmatter, Result};
use serde_json::Value;

use crate::frontmatter::{
    FrontmatterBlock, aliases_of, parse_yaml, split_frontmatter, tag_names_of,
};
use crate::scan::is_valid_tag_name;

/// Options for [`set_property_with`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOptions {
    /// Double-quote strings that look like dates or datetimes
    /// (`"2026-10-20"`), e.g. for a property whose type is *text*, so that
    /// Obsidian does not infer a date type from the plain value.
    pub quote_date_strings: bool,
}

/// State of a note's frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontmatterStatus {
    /// No `---` block at the top of the note.
    Missing,
    /// A block whose YAML parses to a mapping (possibly empty).
    Valid,
    /// A block whose YAML is invalid or not a mapping; edits are refused.
    Invalid,
}

/// Whether `source` has a frontmatter block and whether it is valid.
pub fn frontmatter_status(source: &str) -> FrontmatterStatus {
    match split_frontmatter(source) {
        None => FrontmatterStatus::Missing,
        Some(b) => match parse_yaml(&source[b.yaml_start..b.yaml_end]) {
            Some(_) => FrontmatterStatus::Valid,
            None => FrontmatterStatus::Invalid,
        },
    }
}

/// Properties of the note (`None` without frontmatter or with invalid YAML).
///
/// The returned map is ordered by key (serde_json's default map); use
/// [`ordered_properties`] or [`property_order`] for the order of the file.
pub fn get_properties(source: &str) -> Option<Frontmatter> {
    let b = split_frontmatter(source)?;
    parse_yaml(&source[b.yaml_start..b.yaml_end])
}

/// Properties in the order they appear in the file (`None` without
/// frontmatter or with invalid YAML).
pub fn ordered_properties(source: &str) -> Option<Vec<(String, Value)>> {
    let map = get_properties(source)?;
    let mut out: Vec<(String, Value)> = Vec::with_capacity(map.len());
    for k in property_order(source) {
        if let Some(v) = map.get(&k)
            && !out.iter().any(|(o, _)| o == &k)
        {
            out.push((k, v.clone()));
        }
    }
    // Keys the line scanner could not see (exotic YAML) go last.
    for (k, v) in &map {
        if !out.iter().any(|(o, _)| o == k) {
            out.push((k.clone(), v.clone()));
        }
    }
    Some(out)
}

/// Top-level property keys in file order (empty without frontmatter). Works
/// on the text layout, so it also lists keys of invalid YAML.
pub fn property_order(source: &str) -> Vec<String> {
    match split_frontmatter(source) {
        Some(b) => scan_entries(source, &b)
            .into_iter()
            .map(|e| e.key)
            .collect(),
        None => Vec::new(),
    }
}

/// Set `key` to `value`, creating the frontmatter block if needed, replacing
/// the existing entry (its whole text) or appending the key at the end of the
/// frontmatter. See the [module docs](self) for the serialization rules.
///
/// # Errors
///
/// [`Error::Parse`] for invalid existing YAML; [`Error::Other`] for an empty
/// key.
pub fn set_property(source: &str, key: &str, value: &Value) -> Result<String> {
    set_property_with(source, key, value, &WriteOptions::default())
}

/// [`set_property`] with explicit [`WriteOptions`].
pub fn set_property_with(
    source: &str,
    key: &str,
    value: &Value,
    opts: &WriteOptions,
) -> Result<String> {
    if key.trim().is_empty() {
        return Err(Error::Other("empty property name".into()));
    }
    let out = match split_frontmatter(source) {
        None => {
            let nl = newline_of_first_line(source);
            let bom = if source.starts_with('\u{FEFF}') { 3 } else { 0 };
            let entry = format_entry(key, value, nl, opts);
            let mut out = String::with_capacity(source.len() + entry.len() + 10);
            out.push_str(&source[..bom]);
            out.push_str("---");
            out.push_str(nl);
            out.push_str(&entry);
            out.push_str("---");
            out.push_str(nl);
            out.push_str(&source[bom..]);
            out
        }
        Some(block) => {
            let map = valid_map(source, &block)?;
            let nl = newline_of_block(source, &block);
            let entries = scan_entries(source, &block);
            match find_entry(&entries, key) {
                Some(e) => {
                    let entry = format_entry(&e.key, value, nl, opts);
                    splice(source, e.line_start, e.end, &entry)
                }
                None => {
                    if find_key_ci(&map, key).is_some() {
                        return Err(unlocatable(key));
                    }
                    let entry = format_entry(key, value, nl, opts);
                    splice(source, block.yaml_end, block.yaml_end, &entry)
                }
            }
        }
    };
    check_valid(out)
}

/// Remove property `key` (its whole entry). The frontmatter block is removed
/// when nothing but blank lines remains in it. Unchanged source if the key is
/// absent.
///
/// # Errors
///
/// [`Error::Parse`] for invalid existing YAML.
pub fn remove_property(source: &str, key: &str) -> Result<String> {
    let Some(block) = split_frontmatter(source) else {
        return Ok(source.to_string());
    };
    let map = valid_map(source, &block)?;
    let entries = scan_entries(source, &block);
    let Some(e) = find_entry(&entries, key) else {
        if find_key_ci(&map, key).is_some() {
            return Err(unlocatable(key));
        }
        return Ok(source.to_string());
    };
    let remaining_yaml = format!(
        "{}{}",
        &source[block.yaml_start..e.line_start],
        &source[e.end..block.yaml_end]
    );
    let out = if remaining_yaml.trim().is_empty() {
        splice(source, block.start, block.end, "")
    } else {
        splice(source, e.line_start, e.end, "")
    };
    check_valid(out)
}

/// Rename property `old` to `new`, keeping its value text verbatim. A
/// case-only rename (`status` → `Status`) is allowed. Unchanged source if
/// `old` is absent.
///
/// # Errors
///
/// [`Error::Parse`] for invalid existing YAML, [`Error::Other`] if `new` is
/// empty or another entry already uses `new`.
pub fn rename_property(source: &str, old: &str, new: &str) -> Result<String> {
    if new.trim().is_empty() {
        return Err(Error::Other("empty property name".into()));
    }
    let Some(block) = split_frontmatter(source) else {
        return Ok(source.to_string());
    };
    let map = valid_map(source, &block)?;
    let entries = scan_entries(source, &block);
    let Some(e) = find_entry(&entries, old) else {
        if find_key_ci(&map, old).is_some() {
            return Err(unlocatable(old));
        }
        return Ok(source.to_string());
    };
    if e.key == new {
        return Ok(source.to_string());
    }
    if let Some(other) = find_entry(&entries, new)
        && other.line_start != e.line_start
    {
        return Err(Error::Other(format!("property {new:?} already exists")));
    }
    let out = splice(source, e.line_start, e.key_end, &format_key(new));
    check_valid(out)
}

// ------------------------------------------------------------------ tags

/// Normalize a tag as typed by a user: trims whitespace and leading `#`.
pub fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('#').trim().to_string()
}

/// Add `tag` (with or without `#`) to the frontmatter `tags` list, creating
/// the property (and the frontmatter) if needed. An existing `tags`/`tag`
/// string value (`tags: a, b`) is normalized into a list. Nothing changes if
/// the note's frontmatter already has the tag (case-insensitive).
///
/// # Errors
///
/// [`Error::Other`] for an invalid tag name, [`Error::Parse`] for invalid
/// existing YAML.
pub fn add_tag(source: &str, tag: &str) -> Result<String> {
    let tag = normalize_tag(tag);
    if !is_valid_tag_name(&tag) {
        return Err(Error::Other(format!("invalid tag name: {tag:?}")));
    }
    let map = editable_map(source)?;
    let (key, names) = fm_tags(&map);
    if names.iter().any(|n| eq_ci(n, &tag)) {
        return Ok(source.to_string());
    }
    let mut list = names;
    list.push(tag);
    set_property(
        source,
        key.as_deref().unwrap_or("tags"),
        &string_array(list),
    )
}

/// Remove `tag` (exact name, case-insensitive; nested children are other
/// tags and are kept) from the frontmatter list *and* from the body: each
/// inline `#tag` token is deleted together with one adjacent space. Code,
/// comments and other inert regions are never touched. The `tags` property is
/// removed when its list becomes empty.
///
/// With invalid frontmatter YAML only inline tags are removed.
pub fn remove_tag(source: &str, tag: &str) -> Result<String> {
    let tag = normalize_tag(tag);
    if tag.is_empty() {
        return Ok(source.to_string());
    }
    let parsed = crate::parse(source);
    let bytes = source.as_bytes();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for t in parsed.tags.iter().filter(|t| !t.from_frontmatter) {
        if !eq_ci(&t.name, &tag) {
            continue;
        }
        let (s, e) = (t.span.start, t.span.end);
        let r = if matches!(bytes.get(e), Some(b' ' | b'\t')) {
            (s, e + 1)
        } else if s > 0 && matches!(bytes.get(s - 1), Some(b' ' | b'\t')) {
            (s - 1, e)
        } else {
            (s, e)
        };
        ranges.push(r);
    }
    let mut out = apply_edits(
        source,
        ranges
            .into_iter()
            .map(|(s, e)| (s, e, String::new()))
            .collect(),
    );
    if frontmatter_status(&out) == FrontmatterStatus::Valid
        && let Some(map) = get_properties(&out)
    {
        let (key, names) = fm_tags(&map);
        if let Some(key) = key {
            let kept: Vec<String> = names.iter().filter(|n| !eq_ci(n, &tag)).cloned().collect();
            if kept.len() != names.len() {
                out = if kept.is_empty() {
                    remove_property(&out, &key)?
                } else {
                    set_property(&out, &key, &string_array(kept))?
                };
            }
        }
    }
    Ok(out)
}

/// Rename tag `old` to `new` everywhere in the note: inline `#old` tokens and
/// frontmatter tags, including nested children (`#old/child` →
/// `#new/child`). Matching is case-insensitive; the case of the child part is
/// preserved. Code spans/blocks, comments, math, URLs and other inert regions
/// are never touched (only tags recognized by [`crate::parse`] are edited).
///
/// Frontmatter tags are edited in place when possible; if the rename creates
/// duplicates (merging into an existing tag) or an in-place edit would not
/// round-trip, the `tags` list is rewritten (deduplicated, block style).
/// With invalid frontmatter YAML only inline tags are renamed.
///
/// # Errors
///
/// [`Error::Other`] if `old` is empty or `new` is not a valid tag name.
pub fn rename_tag(source: &str, old: &str, new: &str) -> Result<String> {
    let old = normalize_tag(old);
    let new = normalize_tag(new);
    if old.is_empty() {
        return Err(Error::Other("empty tag name".into()));
    }
    if !is_valid_tag_name(&new) {
        return Err(Error::Other(format!("invalid tag name: {new:?}")));
    }
    let parsed = crate::parse(source);
    let mut inline_edits = Vec::new();
    let mut fm_edits = Vec::new();
    let mut fm_names = Vec::new();
    let mut fm_changed = false;
    let mut fm_in_place = true;
    for t in &parsed.tags {
        let renamed = renamed_tag(&t.name, &old, &new);
        if t.from_frontmatter {
            if let Some(r) = &renamed {
                fm_changed = true;
                if source.get(t.span.start..t.span.end) == Some(t.name.as_str()) {
                    fm_edits.push((t.span.start, t.span.end, r.clone()));
                } else {
                    fm_in_place = false;
                }
            }
            fm_names.push(renamed.unwrap_or_else(|| t.name.clone()));
        } else if let Some(r) = renamed {
            inline_edits.push((t.span.start, t.span.end, format!("#{r}")));
        }
    }
    if !fm_changed {
        return Ok(apply_edits(source, inline_edits));
    }
    let expected = dedup_ci(fm_names.clone());
    if fm_in_place && expected.len() == fm_names.len() {
        let mut all = inline_edits.clone();
        all.extend(fm_edits);
        let out = apply_edits(source, all);
        if fm_tag_names(&out).as_deref() == Some(expected.as_slice()) {
            return Ok(out);
        }
    }
    // Rewrite the whole tags list.
    let out = apply_edits(source, inline_edits);
    let Some(map) = get_properties(&out) else {
        return Ok(out);
    };
    match fm_tags(&map) {
        (Some(key), items) => {
            let list: Vec<String> = items
                .into_iter()
                .map(|t| renamed_tag(&t, &old, &new).unwrap_or(t))
                .collect();
            set_property(&out, &key, &string_array(dedup_ci(list)))
        }
        (None, _) => Ok(out),
    }
}

/// The new name of tag `name` when renaming `old` → `new` (both normalized),
/// or `None` if `name` is neither `old` nor one of its children.
fn renamed_tag(name: &str, old: &str, new: &str) -> Option<String> {
    let n = old.chars().count();
    let split = name.char_indices().nth(n).map_or(name.len(), |(i, _)| i);
    let (prefix, rest) = name.split_at(split);
    (eq_ci(prefix, old) && (rest.is_empty() || rest.starts_with('/')))
        .then(|| format!("{new}{rest}"))
}

/// Frontmatter tag names of a source (`None` without valid frontmatter).
fn fm_tag_names(source: &str) -> Option<Vec<String>> {
    get_properties(source).map(|m| tag_names_of(&m).1)
}

/// The `tags`/`tag` key and its raw items: list items stringified as
/// written (a leading `#` stripped), or a string split on commas and
/// whitespace.
fn fm_tags(map: &Frontmatter) -> (Option<String>, Vec<String>) {
    let Some(key) = find_key_ci(map, "tags").or_else(|| find_key_ci(map, "tag")) else {
        return (None, Vec::new());
    };
    let mut out = Vec::new();
    match map.get(key) {
        Some(Value::Array(items)) => {
            for it in items {
                let s = match it {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                let s = normalize_tag(&s);
                if !s.is_empty() {
                    out.push(s);
                }
            }
        }
        Some(Value::String(s)) => out.extend(
            s.split(|c: char| c == ',' || c.is_whitespace())
                .map(normalize_tag)
                .filter(|s| !s.is_empty()),
        ),
        Some(Value::Number(n)) => out.push(n.to_string()),
        _ => {}
    }
    (Some(key.to_string()), out)
}

// --------------------------------------------------------------- aliases

/// Add `alias` to the frontmatter `aliases` list (creating it, normalizing a
/// comma-separated string into a list). Nothing changes if the alias already
/// exists (case-insensitive).
///
/// # Errors
///
/// [`Error::Other`] for an empty alias, [`Error::Parse`] for invalid YAML.
pub fn add_alias(source: &str, alias: &str) -> Result<String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(Error::Other("empty alias".into()));
    }
    let map = editable_map(source)?;
    let key = find_key_ci(&map, "aliases")
        .or_else(|| find_key_ci(&map, "alias"))
        .map(str::to_string);
    let mut list = aliases_of(&map);
    if list.iter().any(|a| eq_ci(a, alias)) {
        return Ok(source.to_string());
    }
    list.push(alias.to_string());
    set_property(
        source,
        key.as_deref().unwrap_or("aliases"),
        &string_array(list),
    )
}

/// Remove `alias` (case-insensitive) from the frontmatter aliases; the
/// property is removed when the list becomes empty.
///
/// # Errors
///
/// [`Error::Parse`] for invalid YAML.
pub fn remove_alias(source: &str, alias: &str) -> Result<String> {
    let alias = alias.trim();
    let map = editable_map(source)?;
    let Some(key) = find_key_ci(&map, "aliases")
        .or_else(|| find_key_ci(&map, "alias"))
        .map(str::to_string)
    else {
        return Ok(source.to_string());
    };
    let list = aliases_of(&map);
    let kept: Vec<String> = list.iter().filter(|a| !eq_ci(a, alias)).cloned().collect();
    if kept.len() == list.len() {
        return Ok(source.to_string());
    }
    if kept.is_empty() {
        remove_property(source, &key)
    } else {
        set_property(source, &key, &string_array(kept))
    }
}

// ------------------------------------------------------------ type names

/// Obsidian type name inferred from a value: `"multitext"` (list),
/// `"checkbox"` (bool), `"number"`, `"date"` (`YYYY-MM-DD`), `"datetime"`
/// (`YYYY-MM-DDTHH:mm[:ss[.sss]]`), otherwise `"text"` (including `null` and
/// objects).
pub fn infer_type_name(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "multitext",
        Value::Bool(_) => "checkbox",
        Value::Number(_) => "number",
        Value::String(s) if is_date(s) => "date",
        Value::String(s) if is_datetime(s) => "datetime",
        _ => "text",
    }
}

fn digits(s: &[u8]) -> Option<u32> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    s.iter().try_fold(0u32, |acc, d| {
        acc.checked_mul(10)?.checked_add(u32::from(d - b'0'))
    })
}

/// `YYYY-MM-DD` with a plausible month and day.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && digits(&b[..4]).is_some()
        && digits(&b[5..7]).is_some_and(|m| (1..=12).contains(&m))
        && digits(&b[8..10]).is_some_and(|d| (1..=31).contains(&d))
}

/// `YYYY-MM-DDTHH:mm`, optionally followed by `:ss` and `.fraction`.
pub fn is_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 16 || !s.is_char_boundary(10) || !is_date(&s[..10]) || b[10] != b'T' {
        return false;
    }
    let time = &b[11..];
    let hm = time.len() >= 5
        && time[2] == b':'
        && digits(&time[..2]).is_some_and(|h| h < 24)
        && digits(&time[3..5]).is_some_and(|m| m < 60);
    if !hm {
        return false;
    }
    let rest = &time[5..];
    if rest.is_empty() {
        return true;
    }
    if rest.len() < 3 || rest[0] != b':' || !digits(&rest[1..3]).is_some_and(|s| s < 61) {
        return false;
    }
    let frac = &rest[3..];
    frac.is_empty() || (frac.len() > 1 && frac[0] == b'.' && digits(&frac[1..]).is_some())
}

// ------------------------------------------------------- serialization

/// The YAML text of one property entry (`key: value` plus continuation
/// lines), ending with `nl`. Exposed for previews and tests; see the
/// [module docs](self) for the rules.
pub fn format_entry(key: &str, value: &Value, nl: &str, opts: &WriteOptions) -> String {
    let mut out = format_key(key);
    out.push(':');
    emit_value(&mut out, value, 0, nl, opts);
    out.push_str(nl);
    out
}

/// Write a value after `key:` (the colon is already written).
fn emit_value(out: &mut String, value: &Value, indent: usize, nl: &str, opts: &WriteOptions) {
    match value {
        Value::Null => {}
        Value::Array(items) if items.is_empty() => {
            if indent > 0 {
                out.push_str(" []");
            }
        }
        Value::Array(items) => {
            for it in items {
                out.push_str(nl);
                push_indent(out, indent + 2);
                out.push('-');
                match it {
                    Value::Null => {}
                    Value::Array(_) | Value::Object(_) => {
                        out.push(' ');
                        out.push_str(&flow_json(it));
                    }
                    scalar => {
                        out.push(' ');
                        out.push_str(&format_scalar(scalar, opts));
                    }
                }
            }
        }
        Value::Object(m) if m.is_empty() => out.push_str(" {}"),
        Value::Object(m) => {
            for (k, v) in m {
                out.push_str(nl);
                push_indent(out, indent + 2);
                out.push_str(&format_key(k));
                out.push(':');
                emit_value(out, v, indent + 2, nl, opts);
            }
        }
        scalar => {
            out.push(' ');
            out.push_str(&format_scalar(scalar, opts));
        }
    }
}

fn push_indent(out: &mut String, n: usize) {
    out.extend(std::iter::repeat_n(' ', n));
}

/// Compact JSON with a space after `:` and `,` (still valid YAML flow).
fn flow_json(v: &Value) -> String {
    match v {
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(flow_json).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(m) => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", json_quote(k), flow_json(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        other => other.to_string(),
    }
}

fn json_quote(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

fn format_scalar(v: &Value, opts: &WriteOptions) -> String {
    match v {
        Value::String(s) => format_string(s, opts),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

const INDICATORS: &[char] = &[
    '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
];

/// A string scalar: plain when safe, double-quoted otherwise.
fn format_string(s: &str, opts: &WriteOptions) -> String {
    let risky = s.is_empty()
        || s.trim() != s
        || s.contains(['\n', '\r', '\t'])
        || s.starts_with(INDICATORS)
        || s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
        || s.contains("[[")
        || (opts.quote_date_strings && (is_date(s) || is_datetime(s)));
    if !risky && plain_roundtrips(s) {
        s.to_string()
    } else {
        json_quote(s)
    }
}

fn plain_roundtrips(s: &str) -> bool {
    parse_yaml(&format!("k: {s}\n"))
        .and_then(|m| m.get("k").cloned())
        .is_some_and(|v| v == Value::String(s.to_string()))
}

/// A mapping key: plain when safe, double-quoted otherwise.
pub fn format_key(k: &str) -> String {
    let risky = k.is_empty()
        || k.trim() != k
        || k.contains(['\n', '\r', '\t'])
        || k.starts_with(INDICATORS)
        || k.contains(": ")
        || k.contains(" #")
        || k.ends_with(':');
    let ok = !risky
        && parse_yaml(&format!("{k}: 0\n")).is_some_and(|m| m.len() == 1 && m.contains_key(k));
    if ok { k.to_string() } else { json_quote(k) }
}

fn string_array(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

// ----------------------------------------------------- layout scanning

/// One top-level property entry in the source.
#[derive(Debug, Clone)]
struct Entry {
    /// Decoded key.
    key: String,
    /// Absolute start of the key line.
    line_start: usize,
    /// Absolute end of the key text (including quotes).
    key_end: usize,
    /// Absolute end of the entry (after the newline of its last line).
    end: usize,
}

/// Lines of `source[from..to]` as `(start, content_end, next_start)`;
/// `content_end` excludes `\n` and a preceding `\r`.
fn lines(source: &str, from: usize, to: usize) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut p = from;
    while p < to {
        let nl = source[p..to].find('\n').map(|i| p + i);
        let (end, next) = match nl {
            Some(i) => (i, i + 1),
            None => (to, to),
        };
        let content_end = if end > p && source.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        };
        out.push((p, content_end, next));
        p = next;
    }
    out
}

/// Top-level entries of a frontmatter block. When the YAML is valid, key
/// lines whose key is not a top-level key of the parsed mapping (e.g. a
/// `word: text` line inside a multi-line quoted scalar) are treated as
/// continuation lines.
fn scan_entries(source: &str, block: &FrontmatterBlock) -> Vec<Entry> {
    let ls = lines(source, block.yaml_start, block.yaml_end);
    let map = parse_yaml(&source[block.yaml_start..block.yaml_end]);
    let mut keys: Vec<(usize, String, usize)> = Vec::new(); // (line index, key, key_end)
    for (i, &(s, e, _)) in ls.iter().enumerate() {
        if let Some((klen, key)) = parse_key_line(&source[s..e])
            && map.as_ref().is_none_or(|m| m.contains_key(&key))
        {
            keys.push((i, key, s + klen));
        }
    }
    let mut out = Vec::with_capacity(keys.len());
    for (n, (i, key, key_end)) in keys.iter().enumerate() {
        let stop = keys.get(n + 1).map_or(ls.len(), |k| k.0);
        let last = (*i + 1..stop)
            .rev()
            .find(|&j| {
                let line = &source[ls[j].0..ls[j].1];
                !(line.trim().is_empty() || line.starts_with('#'))
            })
            .unwrap_or(*i);
        out.push(Entry {
            key: key.clone(),
            line_start: ls[*i].0,
            key_end: *key_end,
            end: ls[last].2,
        });
    }
    out
}

/// Parse a column-0 `key:` line: length of the key text and decoded key.
fn parse_key_line(line: &str) -> Option<(usize, String)> {
    let first = line.chars().next()?;
    if first.is_whitespace() || first == '#' || first == '?' {
        return None;
    }
    if line.starts_with('-') && (line.len() == 1 || line[1..].starts_with([' ', '\t'])) {
        return None;
    }
    if line.starts_with("---") || line.starts_with("...") {
        return None;
    }
    let klen = if first == '"' || first == '\'' {
        let close = closing_quote(line, first)?;
        let rest = line[close + 1..].trim_start_matches([' ', '\t']);
        let after = rest.strip_prefix(':')?;
        if !(after.is_empty() || after.starts_with([' ', '\t'])) {
            return None;
        }
        close + 1
    } else {
        let bytes = line.as_bytes();
        let mut found = None;
        for (i, &c) in bytes.iter().enumerate() {
            if c == b'#' && i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
                return None;
            }
            if c == b':' && matches!(bytes.get(i + 1), None | Some(b' ' | b'\t')) {
                found = Some(i);
                break;
            }
        }
        let colon = found?;
        let k = line[..colon].trim_end();
        if k.is_empty() || k.starts_with(['[', '{']) {
            return None;
        }
        k.len()
    };
    let raw = &line[..klen];
    let key = parse_yaml(&format!("{raw}: 0\n"))
        .and_then(|m| (m.len() == 1).then(|| m.keys().next().cloned()).flatten())
        .unwrap_or_else(|| raw.trim_matches(['"', '\'']).to_string());
    Some((klen, key))
}

/// Byte index of the quote closing the one at index 0.
fn closing_quote(line: &str, q: char) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = 1;
    while i < b.len() {
        match (q, b[i]) {
            ('"', b'\\') => i += 2,
            ('"', b'"') => return Some(i),
            ('\'', b'\'') => {
                if b.get(i + 1) == Some(&b'\'') {
                    i += 2;
                } else {
                    return Some(i);
                }
            }
            _ => i += 1,
        }
    }
    None
}

fn find_entry<'a>(entries: &'a [Entry], key: &str) -> Option<&'a Entry> {
    entries
        .iter()
        .find(|e| e.key == key)
        .or_else(|| entries.iter().find(|e| e.key.eq_ignore_ascii_case(key)))
}

fn find_key_ci<'a>(map: &'a Frontmatter, key: &str) -> Option<&'a str> {
    if let Some((k, _)) = map.get_key_value(key) {
        return Some(k.as_str());
    }
    map.keys()
        .find(|k| k.eq_ignore_ascii_case(key))
        .map(String::as_str)
}

// ------------------------------------------------------------- helpers

fn invalid_yaml() -> Error {
    Error::Parse("the frontmatter is not valid YAML; refusing to edit it".into())
}

fn unlocatable(key: &str) -> Error {
    Error::Parse(format!(
        "cannot locate property {key:?} in the frontmatter text (unsupported YAML layout)"
    ))
}

fn valid_map(source: &str, block: &FrontmatterBlock) -> Result<Frontmatter> {
    parse_yaml(&source[block.yaml_start..block.yaml_end]).ok_or_else(invalid_yaml)
}

/// Properties of a note about to be edited: empty without frontmatter,
/// error for invalid YAML.
fn editable_map(source: &str) -> Result<Frontmatter> {
    match split_frontmatter(source) {
        None => Ok(Frontmatter::new()),
        Some(b) => valid_map(source, &b),
    }
}

/// Final safety net: an edit must leave valid YAML behind.
fn check_valid(out: String) -> Result<String> {
    match frontmatter_status(&out) {
        FrontmatterStatus::Invalid => Err(Error::Parse(
            "the edit would produce invalid YAML; nothing was changed".into(),
        )),
        _ => Ok(out),
    }
}

fn newline_of_block(source: &str, block: &FrontmatterBlock) -> &'static str {
    if source[block.start..block.yaml_start].ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn newline_of_first_line(source: &str) -> &'static str {
    match source.find('\n') {
        Some(i) if i > 0 && source.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

fn splice(source: &str, start: usize, end: usize, with: &str) -> String {
    let mut out = String::with_capacity(source.len() + with.len());
    out.push_str(&source[..start]);
    out.push_str(with);
    out.push_str(&source[end..]);
    out
}

/// Apply non-overlapping-ish `(start, end, replacement)` edits (overlaps are
/// clamped, out-of-range edits skipped).
fn apply_edits(source: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| (e.0, e.1));
    let mut out = String::with_capacity(source.len());
    let mut pos = 0;
    for (s, e, r) in edits {
        let s = s.max(pos);
        if e < s || e > source.len() || !source.is_char_boundary(s) || !source.is_char_boundary(e) {
            continue;
        }
        out.push_str(&source[pos..s]);
        out.push_str(&r);
        pos = e;
    }
    out.push_str(&source[pos..]);
    out
}

fn eq_ci(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn dedup_ci(items: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for it in items {
        let l = it.to_lowercase();
        if !seen.contains(&l) {
            seen.push(l);
            out.push(it);
        }
    }
    out
}
