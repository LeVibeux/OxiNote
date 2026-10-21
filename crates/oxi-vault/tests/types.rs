//! `.obsidian/types.json` handling.

mod common;

use std::fs;

use common::empty;
use oxi_vault::PropertyType;
use serde_json::{Value, json};

#[test]
fn defaults_without_file() {
    let (_t, v) = empty();
    let types = v.property_types().unwrap();
    assert_eq!(types.len(), 3);
    assert_eq!(types["tags"], PropertyType::Tags);
    assert_eq!(types["aliases"], PropertyType::Aliases);
    assert_eq!(types["cssclasses"], PropertyType::Multitext);
    assert_eq!(v.property_type("TAGS").unwrap(), Some(PropertyType::Tags));
    assert_eq!(v.property_type("due").unwrap(), None);
}

#[test]
fn reads_merges_and_preserves_unknown() {
    let (_t, v) = empty();
    let path = v.config_dir().join("types.json");
    fs::write(
        &path,
        r#"{"types": {"due": "date", "Tags": "text", "odd": "fancy", "done": "checkbox"}, "other": 1}"#,
    )
    .unwrap();
    let types = v.property_types().unwrap();
    assert_eq!(types["due"], PropertyType::Date);
    assert_eq!(types["done"], PropertyType::Checkbox);
    assert_eq!(types["tags"], PropertyType::Tags, "built-ins win");
    assert!(!types.contains_key("Tags"));
    assert!(!types.contains_key("odd"));

    v.set_property_type("Due", PropertyType::Datetime).unwrap();
    v.set_property_type("count", PropertyType::Number).unwrap();
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["other"], json!(1));
    assert_eq!(raw["types"]["odd"], json!("fancy"));
    assert_eq!(raw["types"]["Due"], json!("datetime"));
    assert!(raw["types"].get("due").is_none());
    assert_eq!(raw["types"]["count"], json!("number"));
    assert_eq!(
        v.property_type("due").unwrap(),
        Some(PropertyType::Datetime)
    );

    v.remove_property_type("COUNT").unwrap();
    assert_eq!(v.property_type("count").unwrap(), None);
    assert!(v.set_property_type(" ", PropertyType::Text).is_err());
}

#[test]
fn invalid_file_yields_defaults_and_is_replaced_on_write() {
    let (_t, v) = empty();
    let path = v.config_dir().join("types.json");
    fs::write(&path, "not json").unwrap();
    assert_eq!(v.property_types().unwrap().len(), 3);
    v.set_property_type("x", PropertyType::Text).unwrap();
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw, json!({"types": {"x": "text"}}));
}
