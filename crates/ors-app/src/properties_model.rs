//! Model of the Properties panel, the "All properties" pane and the Tags
//! pane: value <-> widget conversions, and the edit operations of
//! [`AppState`] (every note edit goes through [`AppState::apply_edit`]).
//!
//! Nothing here draws anything; the egui code lives in
//! `ui/properties_view.rs` and `ui/tags_view.rs`.

use std::collections::BTreeMap;

use ors_core::{NotePath, Result};
use ors_markdown::properties::{self as props, FrontmatterStatus, WriteOptions};
use ors_vault::PropertyType;
use serde_json::Value;

use crate::settings::{PropertiesInDocument, Settings};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Types and widgets
// ---------------------------------------------------------------------------

/// The kind of editor shown for a property value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Widget {
    Text,
    /// Pills; `tags` adds tag normalisation, a `#` prefix and tag search.
    List {
        tags: bool,
    },
    Number,
    Checkbox,
    Date,
    DateTime,
}

pub fn widget_for(ty: PropertyType) -> Widget {
    match ty {
        PropertyType::Text => Widget::Text,
        PropertyType::Multitext | PropertyType::Aliases => Widget::List { tags: false },
        PropertyType::Tags => Widget::List { tags: true },
        PropertyType::Number => Widget::Number,
        PropertyType::Checkbox => Widget::Checkbox,
        PropertyType::Date => Widget::Date,
        PropertyType::Datetime => Widget::DateTime,
    }
}

/// Icon of a type (glyphs available in egui's bundled fonts).
pub fn type_icon(ty: PropertyType) -> &'static str {
    match ty {
        PropertyType::Text => "≡",
        PropertyType::Multitext => "☰",
        PropertyType::Number => "#",
        PropertyType::Checkbox => "☑",
        PropertyType::Date => "📅",
        PropertyType::Datetime => "🕓",
        PropertyType::Aliases => "↪",
        PropertyType::Tags => "🏷",
    }
}

/// Label of a type, as in Obsidian's menu.
pub fn type_label(ty: PropertyType) -> &'static str {
    match ty {
        PropertyType::Text => "Text",
        PropertyType::Multitext => "List",
        PropertyType::Number => "Number",
        PropertyType::Checkbox => "Checkbox",
        PropertyType::Date => "Date",
        PropertyType::Datetime => "Date & time",
        PropertyType::Aliases => "Aliases",
        PropertyType::Tags => "Tags",
    }
}

/// Types offered by the "Property type" menu.
pub const MENU_TYPES: [PropertyType; 6] = [
    PropertyType::Text,
    PropertyType::Multitext,
    PropertyType::Number,
    PropertyType::Checkbox,
    PropertyType::Date,
    PropertyType::Datetime,
];

/// Whether the type of `key` is fixed by Obsidian (`tags`, `aliases`,
/// `cssclasses`).
pub fn is_builtin_key(key: &str) -> bool {
    ors_vault::BUILTIN_PROPERTY_TYPES
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(key))
}

/// Type of `key` from `types.json` (case-insensitive).
pub fn type_of_key(key: &str, types: &BTreeMap<String, PropertyType>) -> Option<PropertyType> {
    types.get(key).copied().or_else(|| {
        types
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, t)| *t)
    })
}

/// Type shown for a property: `types.json`, else inferred from the value.
pub fn resolve_type(
    key: &str,
    value: &Value,
    types: &BTreeMap<String, PropertyType>,
) -> PropertyType {
    type_of_key(key, types).unwrap_or_else(|| PropertyType::infer(value))
}

/// Type of a vault property in the "All properties" pane.
pub fn info_type(
    name: &str,
    inferred: &str,
    types: &BTreeMap<String, PropertyType>,
) -> PropertyType {
    type_of_key(name, types)
        .or_else(|| PropertyType::from_name(inferred))
        .unwrap_or(PropertyType::Text)
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PropRow {
    pub key: String,
    pub value: Value,
    pub ty: PropertyType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelContent {
    /// No frontmatter block.
    Missing,
    /// Valid frontmatter (possibly empty) in file order.
    Rows(Vec<PropRow>),
    /// Invalid YAML: shown read-only.
    Invalid { yaml: String },
}

pub fn panel_content(source: &str, types: &BTreeMap<String, PropertyType>) -> PanelContent {
    match props::frontmatter_status(source) {
        FrontmatterStatus::Missing => PanelContent::Missing,
        FrontmatterStatus::Invalid => {
            let yaml = ors_markdown::split_frontmatter(source)
                .map(|b| source[b.yaml_start..b.yaml_end].trim_end().to_string())
                .unwrap_or_default();
            PanelContent::Invalid { yaml }
        }
        FrontmatterStatus::Valid => PanelContent::Rows(
            props::ordered_properties(source)
                .unwrap_or_default()
                .into_iter()
                .map(|(key, value)| {
                    let ty = resolve_type(&key, &value, types);
                    PropRow { key, value, ty }
                })
                .collect(),
        ),
    }
}

/// Whether the raw YAML block should be hidden in the editor (Live Preview)
/// because the Properties panel replaces it, or because properties are
/// hidden. Only the `Source` setting shows the raw YAML.
pub fn frontmatter_hidden(settings: &Settings) -> bool {
    settings.properties_in_document != PropertiesInDocument::Source
}

/// Whether the Properties panel is drawn in the document.
pub fn panel_visible(settings: &Settings) -> bool {
    settings.properties_in_document == PropertiesInDocument::Visible
}

// ---------------------------------------------------------------------------
// Value <-> widget conversions
// ---------------------------------------------------------------------------

/// Text shown in a single-line field for `value`.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => items.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Object(_) => value.to_string(),
    }
}

/// Value written for text typed in a field of type `ty` (`null` for empty).
///
/// # Errors
///
/// [`Error::Parse`] when the text is invalid for the type (number, date...).
pub fn value_from_text(ty: PropertyType, text: &str) -> Result<Value> {
    ty.parse_input(text)
}

pub fn checkbox_of(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::String(s) => s.trim().eq_ignore_ascii_case("true"),
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        _ => false,
    }
}

/// Items of a list value (scalars become a one-item list; tags are split
/// on commas / spaces by [`PropertyType::coerce`]).
pub fn list_items(ty: PropertyType, value: &Value) -> Vec<String> {
    let list_ty = if ty == PropertyType::Tags {
        PropertyType::Tags
    } else {
        PropertyType::Multitext
    };
    match list_ty.coerce(value) {
        Value::Array(items) => items
            .iter()
            .map(value_text)
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse the text of an "add item" field: items split on commas, trimmed,
/// `#` stripped for tags.
pub fn parse_new_items(ty: PropertyType, input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| {
            if ty == PropertyType::Tags {
                props::normalize_tag(s)
            } else {
                s.trim().to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// New list value after adding the items typed in `input` (`None` when
/// nothing is added: empty, duplicates, invalid tag names).
pub fn list_with_added(ty: PropertyType, value: &Value, input: &str) -> Option<Value> {
    let mut items = list_items(ty, value);
    let before = items.len();
    for new in parse_new_items(ty, input) {
        if ty == PropertyType::Tags && !ors_markdown::is_valid_tag_name(&new) {
            continue;
        }
        if !items.iter().any(|i| i.eq_ignore_ascii_case(&new)) {
            items.push(new);
        }
    }
    (items.len() > before).then(|| Value::Array(items.into_iter().map(Value::String).collect()))
}

/// New list value after removing item `index`.
pub fn list_without(ty: PropertyType, value: &Value, index: usize) -> Value {
    let mut items = list_items(ty, value);
    if index < items.len() {
        items.remove(index);
    }
    Value::Array(items.into_iter().map(Value::String).collect())
}

/// Split `YYYY-MM-DDTHH:mm[:ss]` into the date and `HH:mm` parts.
pub fn split_datetime(s: &str) -> (String, String) {
    match s.split_once('T') {
        Some((d, t)) => (d.to_string(), t.chars().take(5).collect()),
        None => (s.to_string(), String::new()),
    }
}

/// Inverse of [`split_datetime`]; `None` while the parts are invalid.
pub fn join_datetime(date: &str, time: &str) -> Option<Value> {
    if date.trim().is_empty() && time.trim().is_empty() {
        return Some(Value::Null);
    }
    let s = format!("{}T{}", date.trim(), time.trim());
    props::is_datetime(&s).then_some(Value::String(s))
}

/// Whether `text` is an acceptable date (or empty).
pub fn date_ok(text: &str) -> bool {
    text.trim().is_empty() || props::is_date(text.trim())
}

/// Whether `text` is an acceptable `HH:mm` time (or empty).
pub fn time_ok(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || props::is_datetime(&format!("2000-01-01T{t}"))
}

/// Value to display in the widget of `ty` (the stored value coerced).
pub fn display_value(ty: PropertyType, value: &Value) -> Value {
    if ty.validate(value) {
        value.clone()
    } else {
        ty.coerce(value)
    }
}

/// Property names matching `query` (case-insensitive substring), from the
/// vault's properties, best first. `existing` keys are skipped.
pub fn key_suggestions(
    all: &BTreeMap<String, ors_index::PropertyInfo>,
    types: &BTreeMap<String, PropertyType>,
    query: &str,
    existing: &[String],
    limit: usize,
) -> Vec<(String, PropertyType)> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<(u8, String, PropertyType, usize)> = all
        .values()
        .filter(|i| !existing.iter().any(|e| e.eq_ignore_ascii_case(&i.name)))
        .filter_map(|i| {
            let l = i.name.to_lowercase();
            let rank = if q.is_empty() || l.starts_with(&q) {
                0
            } else if l.contains(&q) {
                1
            } else {
                return None;
            };
            Some((
                rank,
                i.name.clone(),
                info_type(&i.name, i.inferred_type, types),
                i.count,
            ))
        })
        .collect();
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.3.cmp(&a.3))
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    out.into_iter()
        .take(limit)
        .map(|(_, n, t, _)| (n, t))
        .collect()
}

/// Initial value of a freshly added property of type `ty`.
pub fn default_value(ty: PropertyType) -> Value {
    match ty {
        PropertyType::Checkbox => Value::Bool(false),
        _ => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// Sorting of the Tags / All-properties panes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    NameAsc,
    NameDesc,
    CountDesc,
    CountAsc,
}

impl SortMode {
    pub const ALL: [SortMode; 4] = [
        SortMode::NameAsc,
        SortMode::NameDesc,
        SortMode::CountDesc,
        SortMode::CountAsc,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortMode::NameAsc => "Name (A to Z)",
            SortMode::NameDesc => "Name (Z to A)",
            SortMode::CountDesc => "Frequency (most first)",
            SortMode::CountAsc => "Frequency (least first)",
        }
    }

    /// Compare `(name, count)` pairs.
    pub fn compare(self, a: (&str, usize), b: (&str, usize)) -> std::cmp::Ordering {
        let name = |x: &str, y: &str| x.to_lowercase().cmp(&y.to_lowercase());
        match self {
            SortMode::NameAsc => name(a.0, b.0),
            SortMode::NameDesc => name(b.0, a.0),
            SortMode::CountDesc => b.1.cmp(&a.1).then_with(|| name(a.0, b.0)),
            SortMode::CountAsc => a.1.cmp(&b.1).then_with(|| name(a.0, b.0)),
        }
    }
}

/// One displayed tag row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub name: String,
    pub full_name: String,
    pub count: usize,
    pub children: Vec<TagRow>,
}

/// Sorted tag rows: a tree, or (flat) every tag by its full name.
pub fn tag_rows(tree: &[ors_index::TagNode], nested: bool, sort: SortMode) -> Vec<TagRow> {
    fn conv(n: &ors_index::TagNode, sort: SortMode) -> TagRow {
        let mut children: Vec<TagRow> = n.children.iter().map(|c| conv(c, sort)).collect();
        children.sort_by(|a, b| sort.compare((&a.name, a.count), (&b.name, b.count)));
        TagRow {
            name: n.name.clone(),
            full_name: n.full_name.clone(),
            count: n.count,
            children,
        }
    }
    fn flatten(n: &ors_index::TagNode, out: &mut Vec<TagRow>) {
        out.push(TagRow {
            name: n.full_name.clone(),
            full_name: n.full_name.clone(),
            count: n.count,
            children: Vec::new(),
        });
        for c in &n.children {
            flatten(c, out);
        }
    }
    let mut rows: Vec<TagRow> = if nested {
        tree.iter().map(|n| conv(n, sort)).collect()
    } else {
        let mut v = Vec::new();
        for n in tree {
            flatten(n, &mut v);
        }
        v
    };
    rows.sort_by(|a, b| sort.compare((&a.name, a.count), (&b.name, b.count)));
    rows
}

// ---------------------------------------------------------------------------
// Actions and dialogs
// ---------------------------------------------------------------------------

/// A change requested by the properties / tags UI.
#[derive(Debug, Clone, PartialEq)]
pub enum PropAction {
    SetValue {
        path: NotePath,
        key: String,
        value: Value,
    },
    Add {
        path: NotePath,
        key: String,
        ty: PropertyType,
    },
    Remove {
        path: NotePath,
        key: String,
    },
    /// `path: None` only assigns the type (All properties pane).
    ChangeType {
        path: Option<NotePath>,
        key: String,
        ty: PropertyType,
    },
    /// Search `[key]`.
    SearchProperty(String),
    /// The panel handled `AppState::add_property_requested`.
    ClearAddRequest,
}

/// Dialogs of the properties / tags features (`Modal::Props`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropsModal {
    RenameTag {
        old: String,
        new: String,
    },
    RenameProperty {
        /// The note when renamed from a note's panel.
        path: Option<NotePath>,
        key: String,
        new: String,
    },
}

// ---------------------------------------------------------------------------
// AppState operations
// ---------------------------------------------------------------------------

impl AppState {
    /// Apply `edit` to the text of note `path`.
    ///
    /// * With an open buffer the edit applies to the buffer text (it becomes
    ///   dirty and autosave writes it: no keystroke is ever lost);
    /// * otherwise the file is read, edited and written.
    ///
    /// Either way the index (and with it the search) is updated. Returns
    /// whether the text changed; on error nothing changes.
    ///
    /// # Errors
    ///
    /// Read / write errors and errors of `edit`.
    pub fn apply_edit(
        &mut self,
        path: &NotePath,
        edit: impl FnOnce(&str) -> Result<String>,
    ) -> Result<bool> {
        let new = if let Some(b) = self.buffers.get(path) {
            let new = edit(&b.text)?;
            if new == b.text {
                return Ok(false);
            }
            let now = self.now;
            if let Some(b) = self.buffers.get_mut(path) {
                b.text = new.clone();
                b.last_edit = Some(now);
            }
            new
        } else {
            let source = self.vault.read_to_string(path)?;
            let new = edit(&source)?;
            if new == source {
                return Ok(false);
            }
            self.vault.write(path, &new)?;
            new
        };
        self.index.update_from_source(path, &new);
        self.bump();
        Ok(true)
    }

    /// [`Self::apply_edit`] that reports errors in a toast.
    fn edit_or_toast(
        &mut self,
        path: &NotePath,
        edit: impl FnOnce(&str) -> Result<String>,
    ) -> bool {
        match self.apply_edit(path, edit) {
            Ok(changed) => changed,
            Err(e) => {
                self.error(format!("Cannot edit properties of {}: {e}", path.stem()));
                false
            }
        }
    }

    /// Property types known to the vault.
    pub fn property_types(&self) -> BTreeMap<String, PropertyType> {
        self.vault.property_types()
    }

    /// Set a property value (text-typed properties keep date-like strings
    /// quoted so that Obsidian does not infer a date).
    pub fn set_property_value(&mut self, path: &NotePath, key: &str, value: &Value) {
        let ty = type_of_key(key, &self.vault.property_types());
        let opts = WriteOptions {
            quote_date_strings: ty == Some(PropertyType::Text),
        };
        self.edit_or_toast(path, |s| props::set_property_with(s, key, value, &opts));
    }

    pub fn remove_property_from(&mut self, path: &NotePath, key: &str) {
        self.edit_or_toast(path, |s| props::remove_property(s, key));
    }

    /// Rename a property in one note.
    pub fn rename_property_in(&mut self, path: &NotePath, old: &str, new: &str) {
        let new = new.trim();
        if new.is_empty() || new == old {
            return;
        }
        self.edit_or_toast(path, |s| props::rename_property(s, old, new));
    }

    /// Add a property with the type `ty` (recorded in `types.json` when the
    /// key has no type yet).
    pub fn add_property(&mut self, path: &NotePath, key: &str, ty: PropertyType) {
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        let types = self.vault.property_types();
        let ty = type_of_key(key, &types).unwrap_or_else(|| {
            if !is_builtin_key(key)
                && let Err(e) = self.vault.set_property_type(key, ty)
            {
                self.error(format!("Cannot save the type of {key}: {e}"));
            }
            ty
        });
        let value = default_value(ty);
        self.set_property_value(path, key, &value);
    }

    /// Change the type of a property in `types.json`, converting its value
    /// in `path` when given.
    pub fn change_property_type(&mut self, path: Option<&NotePath>, key: &str, ty: PropertyType) {
        if is_builtin_key(key) {
            self.info(format!("The type of {key} cannot be changed"));
            return;
        }
        if let Err(e) = self.vault.set_property_type(key, ty) {
            self.error(format!("Cannot save the type of {key}: {e}"));
            return;
        }
        if let Some(path) = path {
            let opts = WriteOptions {
                quote_date_strings: ty == PropertyType::Text,
            };
            self.edit_or_toast(path, |s| {
                let current = props::get_properties(s)
                    .and_then(|m| {
                        m.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(key))
                            .map(|(_, v)| v.clone())
                    })
                    .unwrap_or(Value::Null);
                let converted = ty.coerce(&current);
                if converted == current && !opts.quote_date_strings {
                    return Ok(s.to_string());
                }
                props::set_property_with(s, key, &converted, &opts)
            });
        }
        self.bump();
    }

    /// Rename a property in every note having it. Open buffers are edited
    /// in place; the type in `types.json` follows. Returns the number of
    /// notes changed.
    pub fn rename_property_everywhere(&mut self, old: &str, new: &str) -> usize {
        let new = new.trim();
        if new.is_empty() || new == old {
            return 0;
        }
        let paths = self.index.files_with_property(old);
        let mut n = 0;
        for p in &paths {
            let was_clean = self.buffers.get(p).is_none_or(|b| !b.is_dirty());
            if matches!(
                self.apply_edit(p, |s| props::rename_property(s, old, new)),
                Ok(true)
            ) {
                n += 1;
                if was_clean && self.buffers.contains_key(p) {
                    self.save(p);
                }
            }
        }
        if let Some(ty) = type_of_key(old, &self.vault.property_types())
            && !is_builtin_key(old)
            && !is_builtin_key(new)
        {
            let _ = self.vault.set_property_type(new, ty);
        }
        self.bump();
        self.info(format!("Renamed {old} to {new} in {n} notes"));
        n
    }

    /// Notes whose text would change when `old` is renamed to `new`.
    ///
    /// # Errors
    ///
    /// [`Error::Other`] for an invalid tag name.
    pub fn plan_tag_rename(&self, old: &str, new: &str) -> Result<Vec<NotePath>> {
        props::rename_tag("", old, new)?;
        let mut out = Vec::new();
        for p in self.index.files_with_tag(old) {
            let Some(text) = self.note_text(&p) else {
                continue;
            };
            if props::rename_tag(&text, old, new)? != text {
                out.push(p);
            }
        }
        Ok(out)
    }

    /// Rename a tag (and its children) in every note. Open buffers receive
    /// the edit (clean ones are saved right away). Returns the number of
    /// notes changed.
    ///
    /// # Errors
    ///
    /// [`Error::Other`] for an invalid tag name (nothing is written).
    pub fn rename_tag_everywhere(&mut self, old: &str, new: &str) -> Result<usize> {
        let plan = self.plan_tag_rename(old, new)?;
        let mut n = 0;
        for p in &plan {
            let was_clean = self.buffers.get(p).is_none_or(|b| !b.is_dirty());
            match self.apply_edit(p, |s| props::rename_tag(s, old, new)) {
                Ok(true) => {
                    n += 1;
                    if was_clean && self.buffers.contains_key(p) {
                        self.save(p);
                    }
                }
                Ok(false) => {}
                Err(e) => self.error(format!("Cannot rename the tag in {}: {e}", p.stem())),
            }
        }
        self.bump();
        Ok(n)
    }

    /// Run a [`PropAction`].
    pub fn apply_prop_action(&mut self, action: PropAction) {
        match action {
            PropAction::SetValue { path, key, value } => {
                self.set_property_value(&path, &key, &value);
            }
            PropAction::Add { path, key, ty } => self.add_property(&path, &key, ty),
            PropAction::Remove { path, key } => self.remove_property_from(&path, &key),
            PropAction::ChangeType { path, key, ty } => {
                self.change_property_type(path.as_ref(), &key, ty);
            }
            PropAction::ClearAddRequest => self.add_property_requested = false,
            PropAction::SearchProperty(key) => {
                self.search_query = format!("[{key}]");
                self.settings.left_pane = crate::settings::LeftPane::Search;
                self.settings.left_open = true;
            }
        }
    }

    /// Confirm a [`PropsModal`].
    pub fn confirm_props_modal(&mut self, modal: PropsModal) {
        match modal {
            PropsModal::RenameTag { old, new } => {
                let new = props::normalize_tag(&new);
                match self.rename_tag_everywhere(&old, &new) {
                    Ok(n) => self.info(format!("Renamed #{old} to #{new} in {n} notes")),
                    Err(e) => self.error(format!("Cannot rename tag: {e}")),
                }
            }
            PropsModal::RenameProperty {
                path: Some(p),
                key,
                new,
            } => self.rename_property_in(&p, &key, &new),
            PropsModal::RenameProperty {
                path: None,
                key,
                new,
            } => {
                self.rename_property_everywhere(&key, &new);
            }
        }
    }

    /// "Add property" command: show the panel and focus the name field.
    pub fn request_add_property(&mut self) {
        if self.active_note().is_none() {
            return;
        }
        if !panel_visible(&self.settings) {
            self.settings.properties_in_document = PropertiesInDocument::Visible;
        }
        self.add_property_requested = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemVault;
    use crate::fake::FakeIndex;
    use crate::settings::Settings;
    use crate::state::OpenIn;
    use serde_json::json;

    fn fake_index(v: &dyn ors_core::VaultRead) -> Box<dyn crate::backend::IndexBackend> {
        Box::new(FakeIndex::build(v))
    }

    fn state(files: &[(&str, &str)]) -> AppState {
        let v = MemVault::with_files("t", files.iter().copied());
        AppState::new(Box::new(v), fake_index, Settings::default())
    }

    fn np(s: &str) -> NotePath {
        NotePath::new(s).expect("path")
    }

    const NOTE: &str = "---\ntags:\n  - a\n  - b/c\nstatus: draft\ndone: false\ncount: 3\ndue: 2026-10-20\n---\nBody #inline\n";

    #[test]
    fn rows_follow_file_order_with_types() {
        let mut types = BTreeMap::new();
        types.insert("status".to_string(), PropertyType::Multitext);
        types.insert("tags".to_string(), PropertyType::Tags);
        let PanelContent::Rows(rows) = panel_content(NOTE, &types) else {
            panic!("rows expected");
        };
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["tags", "status", "done", "count", "due"]);
        let tys: Vec<PropertyType> = rows.iter().map(|r| r.ty).collect();
        assert_eq!(
            tys,
            [
                PropertyType::Tags,
                PropertyType::Multitext,
                PropertyType::Checkbox,
                PropertyType::Number,
                PropertyType::Date
            ]
        );
        assert_eq!(panel_content("no fm", &types), PanelContent::Missing);
        match panel_content("---\na: [unclosed\n---\nx", &types) {
            PanelContent::Invalid { yaml } => assert!(yaml.contains("unclosed")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn widgets_per_type() {
        assert_eq!(widget_for(PropertyType::Tags), Widget::List { tags: true });
        assert_eq!(
            widget_for(PropertyType::Aliases),
            Widget::List { tags: false }
        );
        assert_eq!(widget_for(PropertyType::Datetime), Widget::DateTime);
        assert_eq!(widget_for(PropertyType::Checkbox), Widget::Checkbox);
        assert_eq!(type_label(PropertyType::Multitext), "List");
    }

    #[test]
    fn value_widget_conversions() {
        assert_eq!(value_text(&json!(["a", "b"])), "a, b");
        assert_eq!(value_text(&Value::Null), "");
        assert_eq!(
            value_from_text(PropertyType::Number, "3.5").expect("n"),
            json!(3.5)
        );
        assert!(value_from_text(PropertyType::Number, "abc").is_err());
        assert_eq!(
            value_from_text(PropertyType::Date, "2026-10-20").expect("d"),
            json!("2026-10-20")
        );
        assert!(value_from_text(PropertyType::Date, "20/10/2026").is_err());
        assert_eq!(
            value_from_text(PropertyType::Text, "").expect("empty"),
            Value::Null
        );
        assert!(checkbox_of(&json!(true)));
        assert!(!checkbox_of(&Value::Null));
    }

    #[test]
    fn list_editing() {
        let v = json!(["a", "b"]);
        let t = PropertyType::Multitext;
        assert_eq!(list_items(t, &v), ["a", "b"]);
        assert_eq!(list_items(t, &json!("solo")), ["solo"]);
        assert_eq!(list_items(t, &Value::Null), Vec::<String>::new());
        assert_eq!(
            list_with_added(t, &v, "c, d"),
            Some(json!(["a", "b", "c", "d"]))
        );
        assert_eq!(list_with_added(t, &v, "A"), None, "duplicate ignored");
        assert_eq!(list_with_added(t, &v, "  ,"), None);
        assert_eq!(list_without(t, &v, 0), json!(["b"]));
        assert_eq!(list_without(t, &v, 9), v);
        let tags = PropertyType::Tags;
        assert_eq!(
            list_with_added(tags, &json!(["x"]), "#proj/a, #y"),
            Some(json!(["x", "proj/a", "y"]))
        );
        assert_eq!(list_with_added(tags, &json!(["x"]), "bad tag!"), None);
        assert_eq!(list_items(tags, &json!("a, #b")), ["a", "b"]);
    }

    #[test]
    fn datetime_helpers() {
        assert_eq!(
            split_datetime("2026-10-20T14:30:00"),
            ("2026-10-20".to_string(), "14:30".to_string())
        );
        assert_eq!(
            join_datetime("2026-10-20", "14:30"),
            Some(json!("2026-10-20T14:30"))
        );
        assert_eq!(join_datetime("2026-10-20", "99:99"), None);
        assert_eq!(join_datetime("", ""), Some(Value::Null));
        assert!(date_ok("2026-02-03") && date_ok("") && !date_ok("2026-13-01"));
        assert!(time_ok("09:05") && !time_ok("25:00"));
    }

    #[test]
    fn type_resolution_and_coercion() {
        let mut types = BTreeMap::new();
        types.insert("Due".to_string(), PropertyType::Date);
        assert_eq!(resolve_type("due", &json!("x"), &types), PropertyType::Date);
        assert_eq!(resolve_type("n", &json!(3), &types), PropertyType::Number);
        assert_eq!(
            display_value(PropertyType::Multitext, &json!("a")),
            json!(["a"])
        );
        assert_eq!(display_value(PropertyType::Number, &json!(3)), json!(3));
    }

    #[test]
    fn settings_flags() {
        let mut s = Settings::default();
        assert!(panel_visible(&s) && frontmatter_hidden(&s));
        s.properties_in_document = PropertiesInDocument::Hidden;
        assert!(!panel_visible(&s) && frontmatter_hidden(&s));
        s.properties_in_document = PropertiesInDocument::Source;
        assert!(!panel_visible(&s) && !frontmatter_hidden(&s));
    }

    #[test]
    fn sorting_and_flat_tag_rows() {
        let tree = vec![
            ors_index::TagNode {
                name: "b".into(),
                full_name: "b".into(),
                count: 5,
                children: vec![ors_index::TagNode {
                    name: "z".into(),
                    full_name: "b/z".into(),
                    count: 1,
                    children: vec![],
                }],
            },
            ors_index::TagNode {
                name: "a".into(),
                full_name: "a".into(),
                count: 2,
                children: vec![],
            },
        ];
        let names = |r: &[TagRow]| r.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&tag_rows(&tree, true, SortMode::NameAsc)), ["a", "b"]);
        assert_eq!(
            names(&tag_rows(&tree, true, SortMode::NameDesc)),
            ["b", "a"]
        );
        assert_eq!(
            names(&tag_rows(&tree, true, SortMode::CountDesc)),
            ["b", "a"]
        );
        assert_eq!(
            names(&tag_rows(&tree, true, SortMode::CountAsc)),
            ["a", "b"]
        );
        assert_eq!(
            names(&tag_rows(&tree, false, SortMode::NameAsc)),
            ["a", "b", "b/z"]
        );
    }

    #[test]
    fn apply_edit_routes_to_buffer_or_disk() {
        let mut s = state(&[("n.md", NOTE), ("other.md", "plain\n")]);
        // Not open: goes to disk.
        let other = np("other.md");
        let changed = s
            .apply_edit(&other, |t| props::set_property(t, "k", &json!("v")))
            .expect("edit");
        assert!(changed);
        assert!(!s.buffers.contains_key(&other));
        assert!(
            s.vault
                .read_to_string(&other)
                .expect("read")
                .contains("k: v")
        );
        // Identical result: no change.
        assert!(!s.apply_edit(&other, |t| Ok(t.to_string())).expect("noop"));
        // Open: goes to the buffer, disk untouched until autosave.
        let n = np("n.md");
        s.open(&n, OpenIn::CurrentTab);
        let disk_before = s.vault.read_to_string(&n).expect("read");
        assert!(
            s.apply_edit(&n, |t| props::set_property(t, "status", &json!("done")))
                .expect("edit")
        );
        assert!(s.buffers[&n].text.contains("status: done"));
        assert!(s.buffers[&n].is_dirty());
        assert_eq!(s.vault.read_to_string(&n).expect("read"), disk_before);
        // An unsaved keystroke survives a properties edit.
        s.buffers.get_mut(&n).expect("buf").text.push_str("typed");
        s.apply_edit(&n, |t| props::set_property(t, "count", &json!(9)))
            .expect("edit");
        assert!(s.buffers[&n].text.ends_with("typed"));
        assert!(s.buffers[&n].text.contains("count: 9"));
        // Errors leave the text alone.
        let before = s.buffers[&n].text.clone();
        assert!(
            s.apply_edit(&n, |_| Err(ors_core::Error::Other("boom".into())))
                .is_err()
        );
        assert_eq!(s.buffers[&n].text, before);
        // The index follows the buffer.
        assert!(
            s.index
                .properties()
                .values()
                .any(|i| i.name == "count" && i.count == 1)
        );
    }

    #[test]
    fn invalid_frontmatter_is_refused_with_a_toast() {
        let mut s = state(&[("bad.md", "---\na: [unclosed\n---\nx\n")]);
        let p = np("bad.md");
        s.set_property_value(&p, "k", &json!("v"));
        assert!(!s.toasts.is_empty());
        assert!(
            s.vault
                .read_to_string(&p)
                .expect("read")
                .contains("unclosed")
        );
    }

    #[test]
    fn add_remove_rename_and_type_change() {
        let mut s = state(&[("n.md", NOTE), ("m.md", "---\nstatus: x\n---\n")]);
        let n = np("n.md");
        s.open(&n, OpenIn::CurrentTab);
        s.add_property(&n, "rating", PropertyType::Number);
        assert_eq!(
            s.vault.property_types().get("rating"),
            Some(&PropertyType::Number)
        );
        assert!(s.buffers[&n].text.contains("rating:\n"));
        s.set_property_value(&n, "rating", &json!(4));
        assert!(s.buffers[&n].text.contains("rating: 4"));
        s.remove_property_from(&n, "rating");
        assert!(!s.buffers[&n].text.contains("rating"));
        s.rename_property_in(&n, "status", "state");
        assert!(s.buffers[&n].text.contains("state: draft"));
        // Type change converts the value.
        s.change_property_type(Some(&n), "count", PropertyType::Text);
        assert!(
            s.buffers[&n].text.contains("count: \"3\"") || s.buffers[&n].text.contains("count: 3")
        );
        s.change_property_type(Some(&n), "state", PropertyType::Multitext);
        assert!(s.buffers[&n].text.contains("state:\n  - draft"));
        assert_eq!(
            s.vault.property_types().get("state"),
            Some(&PropertyType::Multitext)
        );
        // Built-in types are fixed.
        s.change_property_type(Some(&n), "tags", PropertyType::Text);
        assert_eq!(
            s.vault.property_types().get("tags"),
            Some(&PropertyType::Tags)
        );
    }

    #[test]
    fn rename_property_everywhere_handles_buffers_and_disk() {
        let mut s = state(&[
            ("a.md", "---\nstatus: x\n---\n"),
            ("b.md", "---\nStatus: y\n---\n"),
            ("c.md", "no props\n"),
        ]);
        let a = np("a.md");
        s.open(&a, OpenIn::CurrentTab);
        s.vault
            .set_property_type("status", PropertyType::Text)
            .expect("type");
        let n = s.rename_property_everywhere("status", "etat");
        assert_eq!(n, 2);
        assert!(s.vault.read_to_string(&a).expect("a").contains("etat: x"));
        assert!(
            s.vault
                .read_to_string(&np("b.md"))
                .expect("b")
                .contains("etat: y")
        );
        assert_eq!(
            s.vault.property_types().get("etat"),
            Some(&PropertyType::Text)
        );
    }

    #[test]
    fn tag_rename_plan_and_apply_with_buffers() {
        let mut s = state(&[
            ("a.md", "---\ntags:\n  - proj/x\n---\nhello #proj\n"),
            ("b.md", "text #proj/y here\n"),
            ("c.md", "nothing #other\n"),
        ]);
        let a = np("a.md");
        let b = np("b.md");
        s.open(&a, OpenIn::CurrentTab);
        s.open(&b, OpenIn::NewTab);
        // b has unsaved typing.
        s.buffers.get_mut(&b).expect("buf").text.push_str("more\n");
        s.buffer_edited(&b);
        let plan = s.plan_tag_rename("proj", "work").expect("plan");
        assert_eq!(plan.len(), 2);
        assert!(s.plan_tag_rename("proj", "bad name").is_err());
        let n = s.rename_tag_everywhere("proj", "work").expect("rename");
        assert_eq!(n, 2);
        // Clean buffer a was written to disk and not left dirty.
        assert!(s.vault.read_to_string(&a).expect("a").contains("work/x"));
        assert!(!s.buffers[&a].is_dirty());
        // Dirty buffer b got the edit on its text, typing kept, disk untouched.
        assert!(s.buffers[&b].text.contains("#work/y"));
        assert!(s.buffers[&b].text.ends_with("more\n"));
        assert!(s.buffers[&b].is_dirty());
        assert!(s.vault.read_to_string(&b).expect("b").contains("#proj/y"));
        // Untouched note.
        assert!(
            s.vault
                .read_to_string(&np("c.md"))
                .expect("c")
                .contains("#other")
        );
    }

    #[test]
    fn prop_actions_and_search() {
        let mut s = state(&[("n.md", NOTE)]);
        let n = np("n.md");
        s.apply_prop_action(PropAction::SetValue {
            path: n.clone(),
            key: "count".into(),
            value: json!(10),
        });
        assert!(s.vault.read_to_string(&n).expect("n").contains("count: 10"));
        s.apply_prop_action(PropAction::SearchProperty("status".into()));
        assert_eq!(s.search_query, "[status]");
    }

    #[test]
    fn key_suggestions_rank_and_skip_existing() {
        let s = state(&[
            ("a.md", "---\nstatus: x\nstart: 1\nauthor: me\n---\n"),
            ("b.md", "---\nstatus: y\n---\n"),
        ]);
        let all = s.index.properties();
        let types = s.vault.property_types();
        let r = key_suggestions(&all, &types, "st", &[], 10);
        assert_eq!(r[0].0, "status", "most used first: {r:?}");
        assert!(r.iter().any(|(k, _)| k == "start"));
        assert!(!r.iter().any(|(k, _)| k == "author"));
        let r = key_suggestions(&all, &types, "", &["status".to_string()], 10);
        assert!(!r.iter().any(|(k, _)| k == "status"));
    }
}
