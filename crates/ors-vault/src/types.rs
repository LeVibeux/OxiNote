//! Property types: `.obsidian/types.json` and value inference / coercion.
//!
//! Obsidian stores the type assigned to each property name in
//! `.obsidian/types.json`:
//!
//! ```json
//! { "types": { "aliases": "aliases", "tags": "tags", "due": "date" } }
//! ```
//!
//! [`Vault::property_types`] reads it (merged with the built-in types of
//! `tags`, `aliases` and `cssclasses`), [`Vault::set_property_type`] writes it
//! while preserving every other key and unknown type names.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use chrono::{NaiveDate, NaiveDateTime};
use ors_core::{Error, Result};
use serde_json::{Map, Value};

use crate::Vault;

pub(crate) const TYPES_JSON: &str = "types.json";

/// The type of a property, as shown in Obsidian's Properties view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PropertyType {
    /// `"text"`: a single string.
    Text,
    /// `"multitext"`: a list of strings ("List" in the UI).
    Multitext,
    /// `"number"`.
    Number,
    /// `"checkbox"`: a boolean.
    Checkbox,
    /// `"date"`: `YYYY-MM-DD`.
    Date,
    /// `"datetime"`: `YYYY-MM-DDTHH:mm[:ss]`.
    Datetime,
    /// `"aliases"`: the reserved `aliases` property (list of strings).
    Aliases,
    /// `"tags"`: the reserved `tags` property (list of tag names).
    Tags,
}

/// Built-in property types that Obsidian always applies.
pub const BUILTIN_PROPERTY_TYPES: [(&str, PropertyType); 3] = [
    ("aliases", PropertyType::Aliases),
    ("cssclasses", PropertyType::Multitext),
    ("tags", PropertyType::Tags),
];

impl PropertyType {
    /// Every type, in Obsidian's menu order.
    pub const ALL: [PropertyType; 8] = [
        PropertyType::Text,
        PropertyType::Multitext,
        PropertyType::Number,
        PropertyType::Checkbox,
        PropertyType::Date,
        PropertyType::Datetime,
        PropertyType::Aliases,
        PropertyType::Tags,
    ];

    /// Name used in `types.json` (`"text"`, `"multitext"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            PropertyType::Text => "text",
            PropertyType::Multitext => "multitext",
            PropertyType::Number => "number",
            PropertyType::Checkbox => "checkbox",
            PropertyType::Date => "date",
            PropertyType::Datetime => "datetime",
            PropertyType::Aliases => "aliases",
            PropertyType::Tags => "tags",
        }
    }

    /// Parse a `types.json` name (case-insensitive). `"list"` is accepted as
    /// a synonym of `multitext`.
    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.trim().to_ascii_lowercase();
        if n == "list" {
            return Some(PropertyType::Multitext);
        }
        Self::ALL.into_iter().find(|t| t.as_str() == n)
    }

    /// Whether values of this type are lists.
    pub fn is_list(self) -> bool {
        matches!(
            self,
            PropertyType::Multitext | PropertyType::Aliases | PropertyType::Tags
        )
    }

    /// Infer the type of a value the way Obsidian does for a property that
    /// has no type yet: list → multitext, bool → checkbox, number → number,
    /// `YYYY-MM-DD` → date, `YYYY-MM-DDTHH:mm[:ss]` → datetime, anything
    /// else (including `null`) → text.
    pub fn infer(value: &Value) -> Self {
        match value {
            Value::Array(_) => PropertyType::Multitext,
            Value::Bool(_) => PropertyType::Checkbox,
            Value::Number(_) => PropertyType::Number,
            Value::String(s) if parse_date(s).is_some() => PropertyType::Date,
            Value::String(s) if parse_datetime(s).is_some() => PropertyType::Datetime,
            _ => PropertyType::Text,
        }
    }

    /// Infer a type from text typed by a user (e.g. on the command line) for
    /// a property without a known type: `true`/`false` → checkbox, a number
    /// → number, a date / datetime → date / datetime, else text. Lists are
    /// never inferred from text (commas are common in plain text).
    pub fn infer_from_input(input: &str) -> Self {
        let t = input.trim();
        if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
            PropertyType::Checkbox
        } else if parse_number(t).is_some() {
            PropertyType::Number
        } else if parse_date(t).is_some() {
            PropertyType::Date
        } else if parse_datetime(t).is_some() {
            PropertyType::Datetime
        } else {
            PropertyType::Text
        }
    }

    /// Whether `value` already conforms to this type. `null` (an empty
    /// property) is valid for every type.
    pub fn validate(self, value: &Value) -> bool {
        match (self, value) {
            (_, Value::Null) => true,
            (PropertyType::Text, Value::String(_)) => true,
            (PropertyType::Number, Value::Number(_)) => true,
            (PropertyType::Checkbox, Value::Bool(_)) => true,
            (PropertyType::Date, Value::String(s)) => parse_date(s).is_some(),
            (PropertyType::Datetime, Value::String(s)) => parse_datetime(s).is_some(),
            (PropertyType::Multitext | PropertyType::Aliases, Value::Array(items)) => {
                items.iter().all(Value::is_string)
            }
            (PropertyType::Tags, Value::Array(items)) => items.iter().all(|i| {
                i.as_str()
                    .is_some_and(|s| !s.starts_with('#') && !s.is_empty())
            }),
            _ => false,
        }
    }

    /// Best-effort conversion of `value` to this type (what Obsidian does
    /// when the type of a property is changed). Values that cannot be
    /// converted are returned unchanged; `null` stays `null`.
    ///
    /// * text: numbers/bools stringified, lists joined with `", "`;
    /// * multitext / aliases: a string becomes `[string]` (empty string →
    ///   `[]`), scalars are stringified;
    /// * tags: like multitext, but strings are split on commas/whitespace and
    ///   leading `#` are stripped;
    /// * number: numeric strings are parsed (`"3"` → `3`), bools → 1 / 0;
    /// * checkbox: `"true"`/`"false"` (any case), numbers (0 = false);
    /// * date: a datetime is truncated to its date;
    /// * datetime: a date gets `T00:00`.
    pub fn coerce(self, value: &Value) -> Value {
        if value.is_null() {
            return Value::Null;
        }
        match self {
            PropertyType::Text => match value {
                Value::Array(items) => Value::String(
                    items
                        .iter()
                        .filter_map(scalar_to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                Value::Object(_) => value.clone(),
                other => scalar_to_string(other).map_or_else(|| other.clone(), Value::String),
            },
            PropertyType::Multitext | PropertyType::Aliases => match value {
                Value::Array(items) => Value::Array(
                    items
                        .iter()
                        .filter_map(scalar_to_string)
                        .map(Value::String)
                        .collect(),
                ),
                Value::String(s) if s.trim().is_empty() => Value::Array(Vec::new()),
                Value::Object(_) => value.clone(),
                other => scalar_to_string(other)
                    .map_or_else(|| other.clone(), |s| Value::Array(vec![Value::String(s)])),
            },
            PropertyType::Tags => {
                let raw: Vec<String> = match value {
                    Value::Array(items) => items.iter().filter_map(scalar_to_string).collect(),
                    Value::Object(_) => return value.clone(),
                    other => scalar_to_string(other).into_iter().collect(),
                };
                Value::Array(
                    raw.iter()
                        .flat_map(|s| split_tags(s))
                        .map(Value::String)
                        .collect(),
                )
            }
            PropertyType::Number => match value {
                Value::String(s) => parse_number(s.trim()).unwrap_or_else(|| value.clone()),
                Value::Bool(b) => Value::from(u8::from(*b)),
                _ => value.clone(),
            },
            PropertyType::Checkbox => match value {
                Value::String(s) if s.trim().eq_ignore_ascii_case("true") => Value::Bool(true),
                Value::String(s) if s.trim().eq_ignore_ascii_case("false") => Value::Bool(false),
                Value::Number(n) => Value::Bool(n.as_f64().is_some_and(|f| f != 0.0)),
                _ => value.clone(),
            },
            PropertyType::Date => match value {
                Value::String(s) => match parse_datetime(s.trim()) {
                    Some(dt) => Value::String(dt.date().format("%Y-%m-%d").to_string()),
                    None => Value::String(s.trim().to_string()),
                },
                _ => value.clone(),
            },
            PropertyType::Datetime => match value {
                Value::String(s) => match parse_date(s.trim()) {
                    Some(d) => Value::String(format!("{}T00:00", d.format("%Y-%m-%d"))),
                    None => Value::String(s.trim().to_string()),
                },
                _ => value.clone(),
            },
        }
    }

    /// Parse text typed by a user into a value of this type. Empty input
    /// gives `null`. Lists are comma-separated (`"a, b"` → `["a", "b"]`;
    /// tags also split on whitespace and lose their `#`). Numbers, checkboxes
    /// (`true`/`false`/`yes`/`no`/`1`/`0`), dates and datetimes are checked.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] when the text is not valid for a number, checkbox,
    /// date or datetime.
    pub fn parse_input(self, input: &str) -> Result<Value> {
        let t = input.trim();
        if t.is_empty() {
            return Ok(Value::Null);
        }
        let bad = |what: &str| Error::Parse(format!("{t:?} is not a valid {what}"));
        match self {
            PropertyType::Text => Ok(Value::String(t.to_string())),
            PropertyType::Multitext | PropertyType::Aliases => Ok(Value::Array(
                t.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            )),
            PropertyType::Tags => Ok(Value::Array(split_tags(t).map(Value::String).collect())),
            PropertyType::Number => parse_number(t).ok_or_else(|| bad("number")),
            PropertyType::Checkbox => match t.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" | "on" => Ok(Value::Bool(true)),
                "false" | "no" | "0" | "off" => Ok(Value::Bool(false)),
                _ => Err(bad("checkbox value (true/false)")),
            },
            PropertyType::Date => parse_date(t)
                .map(|_| Value::String(t.to_string()))
                .ok_or_else(|| bad("date (YYYY-MM-DD)")),
            PropertyType::Datetime => parse_datetime(t)
                .map(|_| Value::String(t.to_string()))
                .ok_or_else(|| bad("datetime (YYYY-MM-DDTHH:mm)")),
        }
    }
}

impl fmt::Display for PropertyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PropertyType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::from_name(s).ok_or_else(|| Error::Parse(format!("unknown property type: {s:?}")))
    }
}

fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn split_tags(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .map(|p| p.trim().trim_start_matches('#'))
        .filter(|p| !p.is_empty())
        .map(String::from)
}

fn parse_number(s: &str) -> Option<Value> {
    if let Ok(i) = s.parse::<i64>() {
        return Some(Value::from(i));
    }
    s.parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
}

/// Parse a strict `YYYY-MM-DD` date.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    if s.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// Parse `YYYY-MM-DDTHH:mm`, `YYYY-MM-DDTHH:mm:ss` or with fractional
/// seconds.
pub fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    if s.len() < 16 || s.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    [
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
}

impl Vault {
    /// Property types: the built-in ones (`tags` → tags, `aliases` →
    /// aliases, `cssclasses` → multitext, which always win, like in Obsidian)
    /// merged with `.obsidian/types.json`. Keys are as written in the file;
    /// unknown type names are skipped. A missing or invalid file yields only
    /// the built-in types.
    pub fn property_types(&self) -> Result<BTreeMap<String, PropertyType>> {
        let mut out = BTreeMap::new();
        if let Some(map) = self.read_config_map(TYPES_JSON)?
            && let Some(Value::Object(types)) = map.get("types")
        {
            for (k, v) in types {
                if let Some(t) = v.as_str().and_then(PropertyType::from_name) {
                    out.insert(k.clone(), t);
                }
            }
        }
        for (k, t) in BUILTIN_PROPERTY_TYPES {
            out.retain(|key: &String, _| !key.eq_ignore_ascii_case(k));
            out.insert(k.to_string(), t);
        }
        Ok(out)
    }

    /// Type of one property (case-insensitive key lookup), if assigned.
    pub fn property_type(&self, key: &str) -> Result<Option<PropertyType>> {
        let types = self.property_types()?;
        Ok(types.get(key).copied().or_else(|| {
            types
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, t)| *t)
        }))
    }

    /// Assign a type to a property in `.obsidian/types.json`, preserving the
    /// other keys of the file and the other entries of `types`. An existing
    /// entry differing only by case is replaced.
    pub fn set_property_type(&self, key: &str, ty: PropertyType) -> Result<()> {
        if key.trim().is_empty() {
            return Err(Error::Other("empty property name".into()));
        }
        self.edit_types(|types| {
            types.retain(|k, _| k == key || !k.eq_ignore_ascii_case(key));
            types.insert(key.to_string(), Value::String(ty.as_str().into()));
        })
    }

    /// Remove a property's entry (case-insensitive) from `types.json`.
    pub fn remove_property_type(&self, key: &str) -> Result<()> {
        self.edit_types(|types| types.retain(|k, _| !k.eq_ignore_ascii_case(key)))
    }

    fn edit_types(&self, f: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
        let mut map = self.read_config_map(TYPES_JSON)?.unwrap_or_default();
        let mut types = match map.remove("types") {
            Some(Value::Object(t)) => t,
            _ => Map::new(),
        };
        f(&mut types);
        map.insert("types".into(), Value::Object(types));
        self.write_config_map(TYPES_JSON, map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_roundtrip() {
        for t in PropertyType::ALL {
            assert_eq!(PropertyType::from_name(t.as_str()), Some(t));
            assert_eq!(t.to_string().parse::<PropertyType>().unwrap(), t);
        }
        assert_eq!(
            PropertyType::from_name("LIST"),
            Some(PropertyType::Multitext)
        );
        assert!(PropertyType::from_name("weird").is_none());
        assert!("weird".parse::<PropertyType>().is_err());
    }

    #[test]
    fn infer() {
        use PropertyType as T;
        assert_eq!(T::infer(&json!(["a"])), T::Multitext);
        assert_eq!(T::infer(&json!(false)), T::Checkbox);
        assert_eq!(T::infer(&json!(1.5)), T::Number);
        assert_eq!(T::infer(&json!("2026-10-20")), T::Date);
        assert_eq!(T::infer(&json!("2026-10-20T09:30")), T::Datetime);
        assert_eq!(T::infer(&json!("2026-10-20T09:30:15")), T::Datetime);
        assert_eq!(T::infer(&json!("2026-02-30")), T::Text);
        assert_eq!(T::infer(&json!("hello")), T::Text);
        assert_eq!(T::infer(&json!(null)), T::Text);
        assert_eq!(T::infer_from_input("TRUE"), T::Checkbox);
        assert_eq!(T::infer_from_input("-3.5"), T::Number);
        assert_eq!(T::infer_from_input("2026-10-20"), T::Date);
        assert_eq!(T::infer_from_input("a, b"), T::Text);
    }

    #[test]
    fn coerce_and_validate() {
        use PropertyType as T;
        assert_eq!(T::Number.coerce(&json!("3")), json!(3));
        assert_eq!(T::Number.coerce(&json!(" 2.5 ")), json!(2.5));
        assert_eq!(T::Number.coerce(&json!("x")), json!("x"));
        assert_eq!(T::Multitext.coerce(&json!("a")), json!(["a"]));
        assert_eq!(T::Multitext.coerce(&json!("")), json!([]));
        assert_eq!(
            T::Multitext.coerce(&json!([1, "b", null])),
            json!(["1", "b"])
        );
        assert_eq!(T::Tags.coerce(&json!("#a, b c")), json!(["a", "b", "c"]));
        assert_eq!(T::Text.coerce(&json!(["a", "b"])), json!("a, b"));
        assert_eq!(T::Text.coerce(&json!(12)), json!("12"));
        assert_eq!(T::Checkbox.coerce(&json!("True")), json!(true));
        assert_eq!(T::Checkbox.coerce(&json!(0)), json!(false));
        assert_eq!(
            T::Date.coerce(&json!("2026-10-20T10:00")),
            json!("2026-10-20")
        );
        assert_eq!(
            T::Datetime.coerce(&json!("2026-10-20")),
            json!("2026-10-20T00:00")
        );
        assert_eq!(T::Text.coerce(&json!(null)), json!(null));
        for t in PropertyType::ALL {
            assert!(t.validate(&json!(null)));
        }
        assert!(T::Number.validate(&json!(1)));
        assert!(!T::Number.validate(&json!("1")));
        assert!(T::Tags.validate(&json!(["a"])));
        assert!(!T::Tags.validate(&json!(["#a"])));
        assert!(!T::Multitext.validate(&json!("a")));
        assert!(T::Date.validate(&json!("2026-10-20")));
        assert!(!T::Date.validate(&json!("20/10/2026")));
        assert!(T::Checkbox.validate(&json!(true)));
    }

    #[test]
    fn parse_input() {
        use PropertyType as T;
        assert_eq!(
            T::Text.parse_input(" hi, there ").unwrap(),
            json!("hi, there")
        );
        assert_eq!(
            T::Multitext.parse_input("a, b ,,c").unwrap(),
            json!(["a", "b", "c"])
        );
        assert_eq!(
            T::Tags.parse_input("#x y,z").unwrap(),
            json!(["x", "y", "z"])
        );
        assert_eq!(T::Number.parse_input("42").unwrap(), json!(42));
        assert!(T::Number.parse_input("forty").is_err());
        assert_eq!(T::Checkbox.parse_input("yes").unwrap(), json!(true));
        assert!(T::Checkbox.parse_input("maybe").is_err());
        assert_eq!(
            T::Date.parse_input("2026-01-31").unwrap(),
            json!("2026-01-31")
        );
        assert!(T::Date.parse_input("2026-01-32").is_err());
        assert!(T::Datetime.parse_input("2026-01-31T12:00").is_ok());
        assert!(T::Datetime.parse_input("2026-01-31 12:00").is_err());
        assert_eq!(T::Number.parse_input("  ").unwrap(), json!(null));
    }
}
