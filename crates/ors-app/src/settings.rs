//! User settings (persisted through eframe storage) and the recent-vaults
//! list (JSON in the platform config dir).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LeftPane {
    #[default]
    Files,
    Search,
    Tags,
    /// "All properties" view.
    Properties,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RightPane {
    #[default]
    Backlinks,
    Outgoing,
    Outline,
    LocalGraph,
    /// Properties of the active note.
    FileProperties,
}

/// How the frontmatter is shown in notes (Obsidian's "Properties in
/// document").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PropertiesInDocument {
    /// The Properties panel replaces the raw YAML.
    #[default]
    Visible,
    /// Neither panel nor raw YAML.
    Hidden,
    /// The raw YAML block.
    Source,
}

/// How notes open for editing (Obsidian's "Default editing mode").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EditingMode {
    #[default]
    LivePreview,
    Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub monospace_editor: bool,
    pub font_size: f32,
    pub left_open: bool,
    pub right_open: bool,
    pub left_pane: LeftPane,
    pub right_pane: RightPane,
    pub readable_line_length: bool,
    pub properties_in_document: PropertiesInDocument,
    /// Autosave delay after the last keystroke, in seconds.
    pub autosave_delay: f64,
    /// Live Preview or Source mode when editing.
    pub default_editing_mode: EditingMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            monospace_editor: false,
            font_size: 15.0,
            left_open: true,
            right_open: true,
            left_pane: LeftPane::Files,
            right_pane: RightPane::Backlinks,
            readable_line_length: true,
            properties_in_document: PropertiesInDocument::Visible,
            autosave_delay: 1.0,
            default_editing_mode: EditingMode::LivePreview,
        }
    }
}

/// Recently opened vault folders, most recent first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentVaults {
    pub vaults: Vec<PathBuf>,
}

impl RecentVaults {
    const MAX: usize = 12;

    pub fn default_file() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "ObsiRust")
            .map(|d| d.config_dir().join("recent_vaults.json"))
    }

    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(file, json)
    }

    pub fn push(&mut self, path: PathBuf) {
        self.vaults.retain(|p| p != &path);
        self.vaults.insert(0, path);
        self.vaults.truncate(Self::MAX);
    }

    pub fn remove(&mut self, path: &Path) {
        self.vaults.retain(|p| p != path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_vaults_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ors-app-test-{}", std::process::id()));
        let file = dir.join("sub/recent.json");
        let mut r = RecentVaults::default();
        r.push("/a".into());
        r.push("/b".into());
        r.push("/a".into());
        assert_eq!(r.vaults, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        r.save(&file).unwrap();
        assert_eq!(RecentVaults::load(&file), r);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(
            RecentVaults::load(Path::new("/nonexistent/x.json")),
            RecentVaults::default()
        );
    }

    #[test]
    fn settings_deserialize_with_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"monospace_editor":true}"#).unwrap();
        assert!(s.monospace_editor);
        assert_eq!(s.font_size, 15.0);
    }
}
