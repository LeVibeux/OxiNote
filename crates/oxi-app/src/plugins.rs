//! Core plugin toggles: `.obsidian/core-plugins.json`.
//!
//! Obsidian ≥ 1.4 writes an object `{"daily-notes": true, "graph": false,
//! ...}`; older versions an array of enabled plugin ids. Both are read, the
//! object form is written, unknown plugin ids are preserved.

use serde_json::{Map, Value};

pub const CORE_PLUGINS_JSON: &str = "core-plugins.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CorePlugin {
    Backlinks,
    Bookmarks,
    DailyNotes,
    Graph,
    Outline,
    Properties,
    TagPane,
    Templates,
    WordCount,
}

impl CorePlugin {
    pub const ALL: [CorePlugin; 9] = [
        CorePlugin::Backlinks,
        CorePlugin::Bookmarks,
        CorePlugin::DailyNotes,
        CorePlugin::Graph,
        CorePlugin::Outline,
        CorePlugin::Properties,
        CorePlugin::TagPane,
        CorePlugin::Templates,
        CorePlugin::WordCount,
    ];

    pub fn id(self) -> &'static str {
        match self {
            CorePlugin::Backlinks => "backlink",
            CorePlugin::Bookmarks => "bookmarks",
            CorePlugin::DailyNotes => "daily-notes",
            CorePlugin::Graph => "graph",
            CorePlugin::Outline => "outline",
            CorePlugin::Properties => "properties",
            CorePlugin::TagPane => "tag-pane",
            CorePlugin::Templates => "templates",
            CorePlugin::WordCount => "word-count",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CorePlugin::Backlinks => "Backlinks",
            CorePlugin::Bookmarks => "Bookmarks",
            CorePlugin::DailyNotes => "Daily notes",
            CorePlugin::Graph => "Graph view",
            CorePlugin::Outline => "Outline",
            CorePlugin::Properties => "Properties view",
            CorePlugin::TagPane => "Tags view",
            CorePlugin::Templates => "Templates",
            CorePlugin::WordCount => "Word count",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            CorePlugin::Backlinks => "Show links to the current note in the sidebar.",
            CorePlugin::Bookmarks => "Bookmark files, headings, folders and searches.",
            CorePlugin::DailyNotes => "Create and open a note for today.",
            CorePlugin::Graph => "Show notes and their links as a graph.",
            CorePlugin::Outline => "Show the headings of the current note.",
            CorePlugin::Properties => "Show and edit note properties.",
            CorePlugin::TagPane => "Show all tags in the vault.",
            CorePlugin::Templates => "Insert templates into the current note.",
            CorePlugin::WordCount => "Show word and character counts in the status bar.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CorePlugins {
    /// All known entries of the file (also plugins Oxinote ignores).
    map: Map<String, Value>,
}

impl CorePlugins {
    pub fn from_json(s: &str) -> Self {
        let map = match serde_json::from_str::<Value>(s) {
            Ok(Value::Object(m)) => m,
            Ok(Value::Array(a)) => {
                // Old format: listed = enabled, the others disabled.
                let mut m: Map<String, Value> = a
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|id| (id.to_string(), Value::Bool(true)))
                    .collect();
                for p in CorePlugin::ALL {
                    m.entry(p.id()).or_insert(Value::Bool(false));
                }
                m
            }
            _ => Map::new(),
        };
        Self { map }
    }

    pub fn to_json(&self) -> String {
        let mut map = self.map.clone();
        for p in CorePlugin::ALL {
            map.entry(p.id()).or_insert(Value::Bool(true));
        }
        serde_json::to_string_pretty(&map).unwrap_or_else(|_| "{}".into())
    }

    /// Plugins missing from the file are enabled (fresh vault).
    pub fn is_enabled(&self, p: CorePlugin) -> bool {
        self.map
            .get(p.id())
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set(&mut self, p: CorePlugin, enabled: bool) {
        self.map.insert(p.id().to_string(), Value::Bool(enabled));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_format_round_trip() {
        let src = r#"{"file-explorer":true,"graph":false,"daily-notes":true,"sync":false}"#;
        let mut p = CorePlugins::from_json(src);
        assert!(!p.is_enabled(CorePlugin::Graph));
        assert!(p.is_enabled(CorePlugin::DailyNotes));
        assert!(p.is_enabled(CorePlugin::Bookmarks)); // missing = on
        p.set(CorePlugin::Templates, false);
        let v: Value = serde_json::from_str(&p.to_json()).unwrap();
        assert_eq!(v["sync"], Value::Bool(false));
        assert_eq!(v["file-explorer"], Value::Bool(true));
        assert_eq!(v["templates"], Value::Bool(false));
        assert_eq!(v["graph"], Value::Bool(false));
        assert_eq!(CorePlugins::from_json(&p.to_json()), {
            let mut q = p.clone();
            q.map = v.as_object().unwrap().clone();
            q
        });
    }

    #[test]
    fn array_format() {
        let p = CorePlugins::from_json(r#"["graph","bookmarks"]"#);
        assert!(p.is_enabled(CorePlugin::Graph));
        assert!(!p.is_enabled(CorePlugin::DailyNotes));
        assert!(CorePlugins::from_json("garbage").is_enabled(CorePlugin::Outline));
    }
}
