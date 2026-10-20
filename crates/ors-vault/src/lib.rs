//! On-disk Obsidian vault.
//!
//! STUB — public signatures are the contract; the bodies are placeholders.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use ors_core::{Error, FileMeta, NotePath, Result, VaultEvent, VaultRead, VaultWrite};
use serde::{Deserialize, Serialize};

/// Where deleted files go (Obsidian's "Deleted files" setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TrashMode {
    /// OS trash. Falls back to [`TrashMode::Local`] if unavailable.
    System,
    /// `<vault>/.trash/`
    #[default]
    Local,
    Permanent,
}

/// Subset of `.obsidian/app.json` we understand. Unknown keys are preserved on save.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// `newFileLocation`: "root" | "current" | "folder"
    pub new_file_location: Option<String>,
    /// `newFileFolderPath`
    pub new_file_folder_path: Option<String>,
    /// `attachmentFolderPath`
    pub attachment_folder_path: Option<String>,
    /// `useMarkdownLinks`
    pub use_markdown_links: bool,
    /// `newLinkFormat`: "shortest" | "relative" | "absolute"
    pub new_link_format: Option<String>,
    /// `alwaysUpdateLinks`
    pub always_update_links: bool,
    /// `userIgnoreFilters` (folders/paths excluded from search & graph)
    pub user_ignore_filters: Vec<String>,
    pub trash_mode: TrashMode,
    /// All other keys from app.json, kept verbatim.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `.obsidian/daily-notes.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyNotesConfig {
    /// Moment.js format, default `"YYYY-MM-DD"`.
    pub format: String,
    /// Folder, default root (`""`).
    pub folder: String,
    /// Template note path (without `.md`), optional.
    pub template: Option<String>,
}

impl Default for DailyNotesConfig {
    fn default() -> Self {
        Self {
            format: "YYYY-MM-DD".into(),
            folder: String::new(),
            template: None,
        }
    }
}

/// `.obsidian/templates.json`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TemplatesConfig {
    pub folder: Option<String>,
    /// Moment.js formats for `{{date}}` / `{{time}}`.
    pub date_format: Option<String>,
    pub time_format: Option<String>,
}

/// A simple calendar date (no external date crate in the contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// Values available when expanding a template.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// `{{title}}`: stem of the note being created.
    pub title: String,
    /// Local date/time used for `{{date}}`, `{{time}}`, `{{date:FORMAT}}`.
    pub date: Date,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// An opened vault rooted at a folder on disk.
#[derive(Debug)]
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    /// Open an existing folder as a vault (creating `.obsidian/` if missing).
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            root: root.as_ref().to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Vault name (root folder name).
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Absolute path on disk of a vault path.
    pub fn absolute(&self, path: &NotePath) -> PathBuf {
        self.root.join(path.as_str())
    }

    /// All folders (vault-relative, sorted), excluding hidden/config/trash.
    pub fn list_folders(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn create_folder(&self, folder: &str) -> Result<()> {
        let _ = folder;
        Ok(())
    }

    /// Create a new empty note `"<name>.md"` in `folder` (`""` = root). If it
    /// exists, uses `"<name> 1.md"`, `"<name> 2.md"`, ... like Obsidian.
    pub fn create_note(&self, folder: &str, name: &str, contents: &str) -> Result<NotePath> {
        let _ = contents;
        NotePath::join(folder, &format!("{name}.md"))
    }

    /// Rename/move a folder and everything inside it.
    pub fn rename_folder(&self, from: &str, to: &str) -> Result<()> {
        let _ = (from, to);
        Ok(())
    }

    /// Delete a file honoring `mode`.
    pub fn delete_with(&self, path: &NotePath, mode: TrashMode) -> Result<()> {
        let _ = (path, mode);
        Ok(())
    }

    pub fn app_config(&self) -> Result<AppConfig> {
        Ok(AppConfig::default())
    }

    pub fn save_app_config(&self, config: &AppConfig) -> Result<()> {
        let _ = config;
        Ok(())
    }

    pub fn daily_notes_config(&self) -> Result<DailyNotesConfig> {
        Ok(DailyNotesConfig::default())
    }

    pub fn templates_config(&self) -> Result<TemplatesConfig> {
        Ok(TemplatesConfig::default())
    }

    /// Open today's (or `date`'s) daily note, creating it from the template if needed.
    pub fn open_daily_note(&self, date: Date) -> Result<NotePath> {
        let _ = date;
        Err(Error::Other("not implemented".into()))
    }

    /// List template notes from the configured templates folder.
    pub fn list_templates(&self) -> Result<Vec<NotePath>> {
        Ok(Vec::new())
    }

    /// Start watching the vault for external changes. Events are debounced and
    /// filtered (no `.obsidian`, `.trash`, hidden files). Dropping the returned
    /// watcher stops it.
    pub fn watch(&self) -> Result<(VaultWatcher, Receiver<VaultEvent>)> {
        let (_tx, rx) = std::sync::mpsc::channel();
        Ok((VaultWatcher { _private: () }, rx))
    }
}

/// Keeps the file watcher alive.
#[derive(Debug)]
pub struct VaultWatcher {
    _private: (),
}

/// Expand `{{title}}`, `{{date}}`, `{{time}}`, `{{date:FORMAT}}`, `{{time:FORMAT}}`
/// (Moment.js formats) in a template.
pub fn expand_template(template: &str, ctx: &TemplateContext, config: &TemplatesConfig) -> String {
    let _ = (ctx, config);
    template.to_string()
}

/// Format a date/time with a Moment.js format string (YYYY, YY, MM, M, DD, D,
/// ddd, dddd, MMM, MMMM, HH, H, mm, ss, A, `[literal]`).
pub fn format_moment(format: &str, ctx: &TemplateContext) -> String {
    let _ = ctx;
    format.to_string()
}

impl VaultRead for Vault {
    fn list_files(&self) -> Result<Vec<NotePath>> {
        Ok(Vec::new())
    }

    fn read_to_string(&self, path: &NotePath) -> Result<String> {
        Err(Error::NotFound(path.clone()))
    }

    fn read_bytes(&self, path: &NotePath) -> Result<Vec<u8>> {
        Err(Error::NotFound(path.clone()))
    }

    fn metadata(&self, path: &NotePath) -> Result<FileMeta> {
        Err(Error::NotFound(path.clone()))
    }

    fn exists(&self, path: &NotePath) -> bool {
        let _ = path;
        false
    }
}

impl VaultWrite for Vault {
    fn write(&self, path: &NotePath, contents: &str) -> Result<()> {
        let _ = (path, contents);
        Ok(())
    }

    fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        let _ = (from, to);
        Ok(())
    }

    /// Uses the trash mode from `app_config()`.
    fn delete(&self, path: &NotePath) -> Result<()> {
        let _ = path;
        Ok(())
    }
}
