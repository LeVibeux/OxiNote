//! JSON mapping of the `.obsidian/*.json` configuration files.
//!
//! The public config structs derive serde with Rust field names (contract), so
//! the camelCase Obsidian mapping is done by hand here. Unknown keys and keys
//! with unexpected types are preserved.

use std::io;

use ors_core::{NotePath, Result};
use serde_json::{Map, Value};

use crate::fsutil::{atomic_write, normalize_folder};
use crate::{
    AppConfig, CONFIG_DIR, DEFAULT_DATE_FORMAT, DailyNotesConfig, TemplatesConfig, TrashMode, Vault,
};

pub(crate) const APP_JSON: &str = "app.json";
pub(crate) const DAILY_NOTES_JSON: &str = "daily-notes.json";
pub(crate) const TEMPLATES_JSON: &str = "templates.json";

const K_NEW_FILE_LOCATION: &str = "newFileLocation";
const K_NEW_FILE_FOLDER: &str = "newFileFolderPath";
const K_ATTACHMENT_FOLDER: &str = "attachmentFolderPath";
const K_USE_MD_LINKS: &str = "useMarkdownLinks";
const K_NEW_LINK_FORMAT: &str = "newLinkFormat";
const K_ALWAYS_UPDATE: &str = "alwaysUpdateLinks";
const K_IGNORE_FILTERS: &str = "userIgnoreFilters";
const K_TRASH: &str = "trashOption";

/// Parse a JSON document into an object map; anything else yields `None`.
fn parse_object(s: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// Take a string value; non-strings are left in `extra`.
fn take_string(
    map: &Map<String, Value>,
    key: &str,
    extra: &mut Map<String, Value>,
) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => {
            extra.insert(key.to_string(), other.clone());
            None
        }
    }
}

fn take_bool(map: &Map<String, Value>, key: &str, extra: &mut Map<String, Value>) -> bool {
    match map.get(key) {
        Some(Value::Bool(b)) => *b,
        None | Some(Value::Null) => false,
        Some(other) => {
            extra.insert(key.to_string(), other.clone());
            false
        }
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty())
}

impl TrashMode {
    /// Value of `trashOption` in `app.json`.
    pub fn as_obsidian_str(self) -> &'static str {
        match self {
            TrashMode::System => "system",
            TrashMode::Local => "local",
            TrashMode::Permanent => "none",
        }
    }

    /// Parse a `trashOption` value (`"system"`, `"local"`, `"none"`).
    pub fn from_obsidian_str(s: &str) -> Option<Self> {
        match s {
            "system" => Some(TrashMode::System),
            "local" => Some(TrashMode::Local),
            "none" => Some(TrashMode::Permanent),
            _ => None,
        }
    }
}

impl AppConfig {
    /// Parse the contents of `app.json`. Invalid JSON yields defaults.
    pub fn from_json_str(s: &str) -> Self {
        parse_object(s)
            .map(|m| Self::from_json_map(&m))
            .unwrap_or_default()
    }

    /// Build from the parsed `app.json` object. Known keys with an unexpected
    /// type are kept in [`AppConfig::extra`] so they survive a save.
    pub fn from_json_map(map: &Map<String, Value>) -> Self {
        const KNOWN: [&str; 8] = [
            K_NEW_FILE_LOCATION,
            K_NEW_FILE_FOLDER,
            K_ATTACHMENT_FOLDER,
            K_USE_MD_LINKS,
            K_NEW_LINK_FORMAT,
            K_ALWAYS_UPDATE,
            K_IGNORE_FILTERS,
            K_TRASH,
        ];
        let mut extra: Map<String, Value> = map
            .iter()
            .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let user_ignore_filters = match map.get(K_IGNORE_FILTERS) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            None | Some(Value::Null) => Vec::new(),
            Some(other) => {
                extra.insert(K_IGNORE_FILTERS.into(), other.clone());
                Vec::new()
            }
        };
        let trash_mode = match map.get(K_TRASH) {
            None | Some(Value::Null) => TrashMode::default(),
            Some(v) => match v.as_str().and_then(TrashMode::from_obsidian_str) {
                Some(m) => m,
                None => {
                    extra.insert(K_TRASH.into(), v.clone());
                    TrashMode::default()
                }
            },
        };
        Self {
            new_file_location: take_string(map, K_NEW_FILE_LOCATION, &mut extra),
            new_file_folder_path: take_string(map, K_NEW_FILE_FOLDER, &mut extra),
            attachment_folder_path: take_string(map, K_ATTACHMENT_FOLDER, &mut extra),
            use_markdown_links: take_bool(map, K_USE_MD_LINKS, &mut extra),
            new_link_format: take_string(map, K_NEW_LINK_FORMAT, &mut extra),
            always_update_links: take_bool(map, K_ALWAYS_UPDATE, &mut extra),
            user_ignore_filters,
            trash_mode,
            extra,
        }
    }

    /// Serialize to the `app.json` object: [`AppConfig::extra`] plus the known
    /// keys. `Option` fields are written when `Some`; booleans, filters and
    /// the trash mode only when they differ from their default.
    pub fn to_json_map(&self) -> Map<String, Value> {
        self.to_json_map_preserving(&Map::new())
    }

    /// Like [`AppConfig::to_json_map`], but default-valued keys that are
    /// present in `existing` (the file on disk) are written too, so a load/save
    /// round trip does not drop them.
    pub fn to_json_map_preserving(&self, existing: &Map<String, Value>) -> Map<String, Value> {
        let mut out = self.extra.clone();
        // `None` leaves the key out (or keeps a wrongly-typed original carried
        // in `extra`).
        let set_opt = |out: &mut Map<String, Value>, key: &str, v: &Option<String>| {
            if let Some(s) = v {
                out.insert(key.into(), Value::String(s.clone()));
            }
        };
        set_opt(&mut out, K_NEW_FILE_LOCATION, &self.new_file_location);
        set_opt(&mut out, K_NEW_FILE_FOLDER, &self.new_file_folder_path);
        set_opt(&mut out, K_ATTACHMENT_FOLDER, &self.attachment_folder_path);
        set_opt(&mut out, K_NEW_LINK_FORMAT, &self.new_link_format);
        let keep = |key: &str, is_default: bool| !is_default || existing.contains_key(key);
        if keep(K_USE_MD_LINKS, !self.use_markdown_links) {
            out.insert(K_USE_MD_LINKS.into(), Value::Bool(self.use_markdown_links));
        }
        if keep(K_ALWAYS_UPDATE, !self.always_update_links) {
            out.insert(
                K_ALWAYS_UPDATE.into(),
                Value::Bool(self.always_update_links),
            );
        }
        if keep(K_IGNORE_FILTERS, self.user_ignore_filters.is_empty()) {
            out.insert(
                K_IGNORE_FILTERS.into(),
                Value::Array(
                    self.user_ignore_filters
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if keep(K_TRASH, self.trash_mode == TrashMode::default()) {
            out.insert(
                K_TRASH.into(),
                Value::String(self.trash_mode.as_obsidian_str().into()),
            );
        }
        out
    }

    /// `true` if `path` (vault-relative) is excluded by `userIgnoreFilters`.
    ///
    /// Filters are matched as path prefixes, like Obsidian (`"Archive/"`
    /// excludes everything in `Archive`). Regex filters (`/.../`) are not
    /// supported and never match.
    pub fn is_user_ignored(&self, path: &str) -> bool {
        self.user_ignore_filters.iter().any(|f| {
            let f = f.trim();
            if f.is_empty() || (f.len() > 1 && f.starts_with('/') && f.ends_with('/')) {
                return false;
            }
            path.starts_with(f.trim_start_matches('/'))
        })
    }

    /// Folder (vault-relative, `""` = root) where new notes go:
    /// `"current"` → `current_folder` (root if `None`), `"folder"` →
    /// `newFileFolderPath`, anything else (`"root"`, missing) → root.
    pub fn new_note_folder(&self, current_folder: Option<&str>) -> String {
        let folder = match self.new_file_location.as_deref() {
            Some("current") => current_folder.unwrap_or(""),
            Some("folder") => self.new_file_folder_path.as_deref().unwrap_or(""),
            _ => "",
        };
        normalize_folder(folder).unwrap_or_default()
    }

    /// Folder (vault-relative, `""` = root) for a new attachment of `note`,
    /// following `attachmentFolderPath`: missing / `"/"` → root, `"./"` →
    /// same folder as the note, `"./sub"` → subfolder of the note's folder,
    /// anything else → that folder.
    pub fn attachment_folder(&self, note: Option<&NotePath>) -> String {
        let setting = self
            .attachment_folder_path
            .as_deref()
            .unwrap_or("/")
            .trim()
            .replace('\\', "/");
        let note_folder = note.and_then(NotePath::parent).unwrap_or("");
        let folder = if setting == "." || setting == "./" {
            note_folder.to_string()
        } else if let Some(sub) = setting.strip_prefix("./") {
            if note_folder.is_empty() {
                sub.to_string()
            } else {
                format!("{note_folder}/{sub}")
            }
        } else {
            setting
        };
        normalize_folder(&folder).unwrap_or_default()
    }
}

impl DailyNotesConfig {
    /// Parse the contents of `daily-notes.json`. Invalid JSON yields defaults.
    pub fn from_json_str(s: &str) -> Self {
        parse_object(s)
            .map(|m| Self::from_json_map(&m))
            .unwrap_or_default()
    }

    /// Build from the parsed object (`folder`, `format`, `template`). Empty
    /// values fall back to defaults; a `.md` suffix on the template is removed.
    pub fn from_json_map(map: &Map<String, Value>) -> Self {
        let get = |k: &str| map.get(k).and_then(Value::as_str).map(str::to_string);
        let format = non_empty(get("format")).unwrap_or_else(|| DEFAULT_DATE_FORMAT.into());
        let folder = get("folder")
            .and_then(|f| normalize_folder(&f).ok())
            .unwrap_or_default();
        let template = non_empty(get("template")).map(|t| {
            let t = t.trim().trim_matches('/');
            t.strip_suffix(".md").unwrap_or(t).to_string()
        });
        Self {
            format,
            folder,
            template,
        }
    }

    /// Serialize to a JSON object, on top of `existing` (unknown keys kept).
    pub fn to_json_map(&self, existing: &Map<String, Value>) -> Map<String, Value> {
        let mut out = existing.clone();
        out.insert("folder".into(), Value::String(self.folder.clone()));
        out.insert("format".into(), Value::String(self.format.clone()));
        match &self.template {
            Some(t) => {
                out.insert("template".into(), Value::String(t.clone()));
            }
            None => {
                out.remove("template");
            }
        }
        out
    }

    /// Vault path of the template note (`template` + `.md`), if configured.
    pub fn template_path(&self) -> Option<NotePath> {
        let t = self.template.as_deref()?;
        NotePath::new(format!("{t}.md")).ok()
    }
}

impl TemplatesConfig {
    /// Parse the contents of `templates.json`. Invalid JSON yields defaults.
    pub fn from_json_str(s: &str) -> Self {
        parse_object(s)
            .map(|m| Self::from_json_map(&m))
            .unwrap_or_default()
    }

    /// Build from the parsed object (`folder`, `dateFormat`, `timeFormat`).
    /// Empty strings are treated as unset.
    pub fn from_json_map(map: &Map<String, Value>) -> Self {
        let get = |k: &str| non_empty(map.get(k).and_then(Value::as_str).map(str::to_string));
        Self {
            folder: get("folder").and_then(|f| normalize_folder(&f).ok()),
            date_format: get("dateFormat"),
            time_format: get("timeFormat"),
        }
    }

    /// Serialize to a JSON object, on top of `existing` (unknown keys kept).
    pub fn to_json_map(&self, existing: &Map<String, Value>) -> Map<String, Value> {
        let mut out = existing.clone();
        for (k, v) in [
            ("folder", &self.folder),
            ("dateFormat", &self.date_format),
            ("timeFormat", &self.time_format),
        ] {
            match v {
                Some(s) => {
                    out.insert(k.into(), Value::String(s.clone()));
                }
                None => {
                    out.remove(k);
                }
            }
        }
        out
    }

    /// `date_format` or [`DEFAULT_DATE_FORMAT`].
    pub fn date_format_or_default(&self) -> &str {
        self.date_format.as_deref().unwrap_or(DEFAULT_DATE_FORMAT)
    }

    /// `time_format` or [`crate::DEFAULT_TIME_FORMAT`].
    pub fn time_format_or_default(&self) -> &str {
        self.time_format
            .as_deref()
            .unwrap_or(crate::DEFAULT_TIME_FORMAT)
    }
}

impl Vault {
    /// Read `.obsidian/<name>` as a JSON object. Missing or invalid → `None`.
    pub(crate) fn read_config_map(&self, name: &str) -> Result<Option<Map<String, Value>>> {
        let path = self.root.join(CONFIG_DIR).join(name);
        match std::fs::read_to_string(&path) {
            Ok(s) => Ok(parse_object(s.trim_start_matches('\u{feff}'))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            // Invalid UTF-8 is "invalid file" → defaults.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Atomically write `.obsidian/<name>` (pretty-printed, 2-space indent).
    pub(crate) fn write_config_map(&self, name: &str, map: Map<String, Value>) -> Result<()> {
        let path = self.root.join(CONFIG_DIR).join(name);
        let json = serde_json::to_string_pretty(&Value::Object(map))
            .map_err(|e| ors_core::Error::Parse(e.to_string()))?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    /// Write `.obsidian/daily-notes.json`, preserving unknown keys.
    pub fn save_daily_notes_config(&self, config: &DailyNotesConfig) -> Result<()> {
        let existing = self.read_config_map(DAILY_NOTES_JSON)?.unwrap_or_default();
        self.write_config_map(DAILY_NOTES_JSON, config.to_json_map(&existing))
    }

    /// Write `.obsidian/templates.json`, preserving unknown keys.
    pub fn save_templates_config(&self, config: &TemplatesConfig) -> Result<()> {
        let existing = self.read_config_map(TEMPLATES_JSON)?.unwrap_or_default();
        self.write_config_map(TEMPLATES_JSON, config.to_json_map(&existing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn app_config_parse() {
        let c = AppConfig::from_json_str(
            r#"{"newFileLocation":"folder","newFileFolderPath":"Inbox","useMarkdownLinks":true,
                "trashOption":"none","userIgnoreFilters":["Archive/"],"foo":{"bar":1}}"#,
        );
        assert_eq!(c.new_file_location.as_deref(), Some("folder"));
        assert_eq!(c.new_file_folder_path.as_deref(), Some("Inbox"));
        assert!(c.use_markdown_links);
        assert_eq!(c.trash_mode, TrashMode::Permanent);
        assert_eq!(c.user_ignore_filters, vec!["Archive/".to_string()]);
        assert_eq!(c.extra.get("foo"), Some(&json!({"bar": 1})));
        assert!(c.is_user_ignored("Archive/old.md"));
        assert!(!c.is_user_ignored("Inbox/x.md"));
        assert_eq!(c.new_note_folder(Some("Projets")), "Inbox");
    }

    #[test]
    fn app_config_invalid() {
        assert_eq!(AppConfig::from_json_str("not json"), AppConfig::default());
        assert_eq!(AppConfig::from_json_str("[1,2]"), AppConfig::default());
        let c = AppConfig::from_json_str(r#"{"useMarkdownLinks":"yes","trashOption":"bin"}"#);
        assert!(!c.use_markdown_links);
        assert_eq!(c.trash_mode, TrashMode::Local);
        // Wrongly typed values survive a round trip.
        let m = c.to_json_map();
        assert_eq!(m.get("useMarkdownLinks"), Some(&json!("yes")));
        assert_eq!(m.get("trashOption"), Some(&json!("bin")));
    }

    #[test]
    fn app_config_roundtrip_map() {
        let original = json!({
            "newFileLocation": "current",
            "useMarkdownLinks": false,
            "alwaysUpdateLinks": true,
            "trashOption": "system",
            "showLineNumber": true,
            "nested": {"a": [1, 2]}
        });
        let map = original.as_object().unwrap().clone();
        let c = AppConfig::from_json_map(&map);
        assert_eq!(Value::Object(c.to_json_map_preserving(&map)), original);
        // Without the existing map, default-valued keys are omitted.
        assert!(!c.to_json_map().contains_key("useMarkdownLinks"));
    }

    #[test]
    fn locations() {
        let mut c = AppConfig::default();
        assert_eq!(c.new_note_folder(Some("a")), "");
        c.new_file_location = Some("current".into());
        assert_eq!(c.new_note_folder(Some("a/b")), "a/b");
        assert_eq!(c.new_note_folder(None), "");
        let note = NotePath::new("Projets/Note.md").unwrap();
        assert_eq!(c.attachment_folder(Some(&note)), "");
        c.attachment_folder_path = Some("./".into());
        assert_eq!(c.attachment_folder(Some(&note)), "Projets");
        c.attachment_folder_path = Some("./img".into());
        assert_eq!(c.attachment_folder(Some(&note)), "Projets/img");
        assert_eq!(c.attachment_folder(None), "img");
        c.attachment_folder_path = Some("attachments".into());
        assert_eq!(c.attachment_folder(Some(&note)), "attachments");
    }

    #[test]
    fn daily_and_templates() {
        let d = DailyNotesConfig::from_json_str(
            r#"{ "folder": "Journal/", "format": "", "template": "Templates/Daily.md", "autorun": true }"#,
        );
        assert_eq!(d.folder, "Journal");
        assert_eq!(d.format, "YYYY-MM-DD");
        assert_eq!(d.template.as_deref(), Some("Templates/Daily"));
        assert_eq!(d.template_path().unwrap().as_str(), "Templates/Daily.md");
        assert_eq!(
            DailyNotesConfig::from_json_str("{"),
            DailyNotesConfig::default()
        );

        let t = TemplatesConfig::from_json_str(
            r#"{"folder":"Templates","dateFormat":"","timeFormat":"HH:mm:ss"}"#,
        );
        assert_eq!(t.folder.as_deref(), Some("Templates"));
        assert_eq!(t.date_format, None);
        assert_eq!(t.date_format_or_default(), "YYYY-MM-DD");
        assert_eq!(t.time_format_or_default(), "HH:mm:ss");
    }
}
