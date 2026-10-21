//! `props`, `prop set|rm` and `tag rename|add|rm`: Obsidian properties and
//! tag management.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Result, anyhow, bail};
use ors_index::PropertyInfo;
use ors_markdown::properties::WriteOptions;
use ors_vault::PropertyType;
use serde::Serialize;
use serde_json::Value;

use crate::context::Context;

/// One row of `props`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropRow {
    /// Key as first seen in the vault.
    pub name: String,
    /// Type name (`text`, `multitext`, `number`, ...).
    #[serde(rename = "type")]
    pub ty: String,
    /// `"types.json"` when assigned in `.obsidian/types.json`, else `"inferred"`.
    pub type_source: &'static str,
    /// Number of notes having the property.
    pub count: usize,
}

/// Combine the cache's property list with the vault's assigned types.
pub fn prop_rows(
    props: &BTreeMap<String, PropertyInfo>,
    types: &BTreeMap<String, PropertyType>,
) -> Vec<PropRow> {
    props
        .values()
        .map(|info| {
            let assigned = types.get(&info.name).copied().or_else(|| {
                types
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&info.name))
                    .map(|(_, t)| *t)
            });
            let (ty, type_source) = match assigned {
                Some(t) => (t.as_str().to_string(), "types.json"),
                None => (info.inferred_type.to_string(), "inferred"),
            };
            PropRow {
                name: info.name.clone(),
                ty,
                type_source,
                count: info.count,
            }
        })
        .collect()
}

/// `name  type  count`, one property per line; inferred types end with `?`.
pub fn format_props(rows: &[PropRow]) -> String {
    let w = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0);
    let tw = rows.iter().map(|r| r.ty.len() + 1).max().unwrap_or(0);
    let mut out = String::new();
    for r in rows {
        let ty = if r.type_source == "inferred" {
            format!("{}?", r.ty)
        } else {
            r.ty.clone()
        };
        let _ = writeln!(out, "{:<w$}  {ty:<tw$}  {}", r.name, r.count);
    }
    out
}

/// Values sorted by count (descending) then text.
pub fn sorted_values(values: &BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = values.iter().map(|(k, n)| (k.clone(), *n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// `value  count`, one value per line.
pub fn format_values(values: &[(String, usize)]) -> String {
    let w = values
        .iter()
        .map(|(v, _)| v.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (v, n) in values {
        let _ = writeln!(out, "{v:<w$}  {n}");
    }
    out
}

/// Choose the type used to parse a `prop set` value: `--type`, else the type
/// assigned in types.json, else the type inferred from the note's current
/// value, else the type inferred from the input text. The flag says whether
/// the type was given explicitly or assigned (as opposed to inferred).
pub fn choose_type(
    explicit: Option<PropertyType>,
    assigned: Option<PropertyType>,
    current: Option<&Value>,
    input: &str,
) -> (PropertyType, bool) {
    if let Some(t) = explicit.or(assigned) {
        return (t, true);
    }
    match current {
        Some(v) if !v.is_null() => (PropertyType::infer(v), false),
        _ => (PropertyType::infer_from_input(input), false),
    }
}

fn current_value(ctx: &Context, path: &ors_core::NotePath, key: &str) -> Option<Value> {
    let fm = ctx.cache.get(path)?.frontmatter.as_ref()?;
    fm.get(key)
        .or_else(|| {
            fm.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v)
        })
        .cloned()
}

/// `prop set`.
pub fn prop_set(
    ctx: &mut Context,
    note: &str,
    key: &str,
    input: &str,
    explicit: Option<&str>,
) -> Result<String> {
    let path = ctx.resolve_note(note)?;
    let explicit = explicit
        .map(|t| {
            PropertyType::from_name(t).ok_or_else(|| {
                let names: Vec<&str> = PropertyType::ALL.iter().map(|t| t.as_str()).collect();
                anyhow!("unknown type {t:?} (expected one of: {})", names.join(", "))
            })
        })
        .transpose()?;
    let assigned = ctx
        .vault
        .property_type(key)
        .map_err(|e| anyhow!("cannot read types.json: {e}"))?;
    let current = current_value(ctx, &path, key);
    let (ty, typed) = choose_type(explicit, assigned, current.as_ref(), input);
    let value = ty.parse_input(input).map_err(|e| anyhow!("{e}"))?;
    let opts = WriteOptions {
        quote_date_strings: typed && ty == PropertyType::Text,
    };
    let changed =
        ors_index::set_property_in_with(&ctx.vault, &mut ctx.cache, &path, key, &value, &opts)
            .map_err(|e| anyhow!("cannot set {key:?} in {path}: {e}"))?;
    if let Some(t) = explicit
        && assigned != Some(t)
    {
        ctx.vault
            .set_property_type(key, t)
            .map_err(|e| anyhow!("cannot write types.json: {e}"))?;
    }
    let shown = serde_json::to_string(&value)?;
    Ok(if changed.is_empty() {
        format!("{path}: {key} is already {shown}\n")
    } else {
        format!("Set {key} = {shown} ({ty}) in {path}\n")
    })
}

/// `prop rm`.
pub fn prop_rm(ctx: &mut Context, note: &str, key: &str) -> Result<String> {
    let path = ctx.resolve_note(note)?;
    let changed = ors_index::remove_property_in(&ctx.vault, &mut ctx.cache, &path, key)
        .map_err(|e| anyhow!("cannot remove {key:?} from {path}: {e}"))?;
    Ok(if changed.is_empty() {
        format!("{path} has no property {key:?}\n")
    } else {
        format!("Removed {key} from {path}\n")
    })
}

fn hash(tag: &str) -> String {
    format!("#{}", ors_markdown::properties::normalize_tag(tag))
}

/// `tag rename`.
pub fn tag_rename(ctx: &mut Context, old: &str, new: &str, dry_run: bool) -> Result<String> {
    let (o, n) = (hash(old), hash(new));
    let mut out = String::new();
    if dry_run {
        let plan = ors_index::plan_tag_rename(&ctx.vault, &ctx.cache, old, new)
            .map_err(|e| anyhow!("cannot rename {o}: {e}"))?;
        let _ = writeln!(out, "Would rename {o} → {n} in {} note(s):", plan.len());
        for (p, _) in plan {
            let _ = writeln!(out, "  {p}");
        }
    } else {
        let changed = ors_index::rename_tag(&ctx.vault, &mut ctx.cache, old, new)
            .map_err(|e| anyhow!("cannot rename {o}: {e}"))?;
        let _ = writeln!(out, "Renamed {o} → {n} in {} note(s):", changed.len());
        for p in changed {
            let _ = writeln!(out, "  {p}");
        }
    }
    Ok(out)
}

/// `tag add`.
pub fn tag_add(ctx: &mut Context, note: &str, tag: &str) -> Result<String> {
    let path = ctx.resolve_note(note)?;
    let t = hash(tag);
    let changed = ors_index::add_tag_in(&ctx.vault, &mut ctx.cache, &path, tag)
        .map_err(|e| anyhow!("cannot add {t} to {path}: {e}"))?;
    Ok(if changed.is_empty() {
        format!("{path} already has {t}\n")
    } else {
        format!("Added {t} to {path}\n")
    })
}

/// `tag rm`.
pub fn tag_rm(ctx: &mut Context, note: &str, tag: &str) -> Result<String> {
    let path = ctx.resolve_note(note)?;
    let t = hash(tag);
    if ors_markdown::properties::normalize_tag(tag).is_empty() {
        bail!("empty tag name");
    }
    let changed = ors_index::remove_tag_in(&ctx.vault, &mut ctx.cache, &path, tag)
        .map_err(|e| anyhow!("cannot remove {t} from {path}: {e}"))?;
    Ok(if changed.is_empty() {
        format!("{path} does not have {t}\n")
    } else {
        format!("Removed {t} from {path}\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info(name: &str, ty: &'static str, count: usize) -> PropertyInfo {
        PropertyInfo {
            name: name.into(),
            count,
            inferred_type: ty,
            type_counts: BTreeMap::new(),
        }
    }

    #[test]
    fn rows_prefer_assigned_types() {
        let mut props = BTreeMap::new();
        props.insert("due".to_string(), info("due", "text", 2));
        props.insert("tags".to_string(), info("tags", "multitext", 5));
        let mut types = BTreeMap::new();
        types.insert("tags".to_string(), PropertyType::Tags);
        types.insert("DUE".to_string(), PropertyType::Date);
        let rows = prop_rows(&props, &types);
        assert_eq!(rows[0].ty, "date");
        assert_eq!(rows[0].type_source, "types.json");
        assert_eq!(rows[1].ty, "tags");
        let mut types = BTreeMap::new();
        types.insert("x".to_string(), PropertyType::Tags);
        let rows = prop_rows(&props, &types);
        assert_eq!(rows[0].type_source, "inferred");
        assert_eq!(
            format_props(&rows),
            "due   text?       2\ntags  multitext?  5\n"
        );
    }

    #[test]
    fn values_sorted_by_count() {
        let mut v = BTreeMap::new();
        v.insert("a".to_string(), 1);
        v.insert("b".to_string(), 3);
        v.insert("c".to_string(), 1);
        let s = sorted_values(&v);
        assert_eq!(format_values(&s), "b  3\na  1\nc  1\n");
    }

    #[test]
    fn type_choice() {
        use PropertyType as T;
        assert_eq!(
            choose_type(Some(T::Text), Some(T::Number), None, "1"),
            (T::Text, true)
        );
        assert_eq!(
            choose_type(None, Some(T::Number), None, "x"),
            (T::Number, true)
        );
        assert_eq!(
            choose_type(None, None, Some(&json!(["a"])), "b, c"),
            (T::Multitext, false)
        );
        assert_eq!(
            choose_type(None, None, Some(&json!(null)), "3"),
            (T::Number, false)
        );
        assert_eq!(choose_type(None, None, None, "true"), (T::Checkbox, false));
        assert_eq!(
            choose_type(None, None, None, "hello, world"),
            (T::Text, false)
        );
    }
}
