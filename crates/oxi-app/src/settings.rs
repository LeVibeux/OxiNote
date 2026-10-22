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
    Bookmarks,
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
    /// Git history of the active note.
    History,
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

/// The heading navigator on the right edge of note panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HeadingNavigator {
    Off,
    /// Ticks + a floating table of contents card on hover.
    Floating,
    /// Ticks magnified like the macOS Dock under the pointer.
    #[default]
    Loupe,
}

impl HeadingNavigator {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Floating => "Floating outline",
            Self::Loupe => "Magnifier",
        }
    }

    /// Off -> Floating outline -> Magnifier -> Off.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Floating,
            Self::Floating => Self::Loupe,
            Self::Loupe => Self::Off,
        }
    }
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
    // ----- workspace / appearance (settings window) ---------------------
    /// Custom accent colour (RGB); `None` = Obsidian purple.
    pub accent: Option<[u8; 3]>,
    /// Size of the interface (sidebars, tabs, menus) text.
    pub ui_font_size: f32,
    /// Proportional font: "Inter" (bundled) or "Default" (egui's).
    pub text_font: String,
    /// New tabs open in reading view ("defaultViewMode": "preview").
    pub default_reading_view: bool,
    pub show_line_numbers: bool,
    pub spellcheck: bool,
    pub left_width: f32,
    pub right_width: f32,
    /// Show the ribbon (left icon bar).
    pub show_ribbon: bool,
    /// Heading navigator on the right edge of note panels.
    pub heading_navigator: HeadingNavigator,
    /// Multiplier of trackpad and mouse wheel scrolling.
    pub scroll_speed: f32,
    /// Spread wheel steps over a few frames (off: apply them at once).
    pub smooth_scrolling: bool,
    /// What to do when a new release is published.
    pub update_mode: crate::update::UpdateMode,
    /// Last check for a new release (Unix seconds).
    pub update_checked_at: u64,
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
            accent: None,
            ui_font_size: 13.5,
            text_font: "Inter".into(),
            default_reading_view: false,
            show_line_numbers: false,
            spellcheck: false,
            left_width: 260.0,
            right_width: 280.0,
            show_ribbon: true,
            heading_navigator: HeadingNavigator::default(),
            scroll_speed: 1.0,
            smooth_scrolling: true,
            update_mode: crate::update::UpdateMode::default(),
            update_checked_at: 0,
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
        directories::ProjectDirs::from("", "", crate::APP_NAME)
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
        let dir = std::env::temp_dir().join(format!("oxi-app-test-{}", std::process::id()));
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
