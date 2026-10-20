//! On-disk Obsidian vault.
//!
//! [`Vault`] wraps a folder on disk and gives sandboxed, Obsidian-compatible
//! access to it:
//!
//! * listing / reading / writing files through [`VaultRead`] and [`VaultWrite`]
//!   (atomic writes, every path checked to stay inside the vault root, hidden
//!   entries such as `.obsidian`, `.trash` and `.git` skipped);
//! * Obsidian-style note creation (`Untitled.md`, `Untitled 1.md`, ...), folder
//!   management and deletion honoring the "Deleted files" setting
//!   ([`TrashMode`]);
//! * reading and writing `.obsidian/app.json`, `daily-notes.json` and
//!   `templates.json` while preserving unknown keys ([`AppConfig`],
//!   [`DailyNotesConfig`], [`TemplatesConfig`]);
//! * Moment.js-compatible date formatting ([`format_moment`]) and template
//!   expansion ([`expand_template`]);
//! * property types from `.obsidian/types.json` ([`PropertyType`],
//!   [`Vault::property_types`], [`Vault::set_property_type`]);
//! * daily notes ([`Vault::open_daily_note`]);
//! * a debounced filesystem watcher ([`Vault::watch`]).

#![warn(missing_docs)]

mod config;
mod daily;
mod fsutil;
mod moment;
mod naming;
mod ops;
mod types;
mod watch;

use std::path::{Path, PathBuf};

use ors_core::{Error, FileMeta, NotePath, Result, VaultRead, VaultWrite};
use serde::{Deserialize, Serialize};

pub use daily::daily_note_path;
pub use moment::{expand_template, format_moment};
pub use naming::{FORBIDDEN_NAME_CHARS, is_valid_file_name, sanitize_file_name};
pub use types::{BUILTIN_PROPERTY_TYPES, PropertyType, parse_date, parse_datetime};
pub use watch::VaultWatcher;

/// Default Moment.js format for `{{date}}` and daily note names.
pub const DEFAULT_DATE_FORMAT: &str = "YYYY-MM-DD";
/// Default Moment.js format for `{{time}}`.
pub const DEFAULT_TIME_FORMAT: &str = "HH:mm";
/// Name of the Obsidian configuration folder at the vault root.
pub const CONFIG_DIR: &str = ".obsidian";
/// Name of the local trash folder at the vault root.
pub const LOCAL_TRASH_DIR: &str = ".trash";

/// Where deleted files go (Obsidian's "Deleted files" setting).
///
/// Maps to `trashOption` in `app.json`: `"system"`, `"local"`, `"none"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TrashMode {
    /// OS trash. Falls back to [`TrashMode::Local`] if unavailable.
    System,
    /// `<vault>/.trash/`
    #[default]
    Local,
    /// Remove permanently.
    Permanent,
}

/// Subset of `.obsidian/app.json` we understand. Unknown keys are preserved on save.
///
/// The JSON mapping (camelCase keys) is implemented in
/// [`AppConfig::from_json_str`] / [`AppConfig::to_json_map`]; the serde derives
/// use Rust field names and are only meant for internal caching/serialization.
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
    /// `trashOption`. A missing key maps to [`TrashMode::Local`] (the
    /// contract default; Obsidian itself defaults to the system trash).
    pub trash_mode: TrashMode,
    /// All other keys from app.json, kept verbatim.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `.obsidian/daily-notes.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyNotesConfig {
    /// Moment.js format, default `"YYYY-MM-DD"`. May contain `/` to create
    /// subfolders (e.g. `"YYYY/MM/YYYY-MM-DD"`).
    pub format: String,
    /// Folder, default root (`""`).
    pub folder: String,
    /// Template note path (without `.md`), optional.
    pub template: Option<String>,
}

impl Default for DailyNotesConfig {
    fn default() -> Self {
        Self {
            format: DEFAULT_DATE_FORMAT.into(),
            folder: String::new(),
            template: None,
        }
    }
}

/// `.obsidian/templates.json`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TemplatesConfig {
    /// `folder`: folder holding the template notes.
    pub folder: Option<String>,
    /// Moment.js formats for `{{date}}` / `{{time}}`.
    pub date_format: Option<String>,
    /// Moment.js format for `{{time}}` (`timeFormat`).
    pub time_format: Option<String>,
}

/// A simple calendar date (no external date crate in the contract).
///
/// See the helpers in this crate: [`Date::today`], [`Date::new`],
/// [`Date::parse_iso`], [`Date::add_days`], conversions to/from
/// [`chrono::NaiveDate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Full year, e.g. `2026`.
    pub year: i32,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
}

/// Values available when expanding a template.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// `{{title}}`: stem of the note being created.
    pub title: String,
    /// Local date/time used for `{{date}}`, `{{time}}`, `{{date:FORMAT}}`.
    pub date: Date,
    /// 0..=23
    pub hour: u32,
    /// 0..=59
    pub minute: u32,
    /// 0..=59
    pub second: u32,
}

/// An opened vault rooted at a folder on disk.
#[derive(Debug, Clone)]
pub struct Vault {
    /// Absolute root as given by the user (not canonicalized).
    root: PathBuf,
    /// Canonical root, used for containment checks and watcher events.
    canonical_root: PathBuf,
}

impl Vault {
    /// Open an existing folder as a vault (creating `.obsidian/` if missing).
    ///
    /// Fails with [`Error::InvalidPath`] if `root` does not exist or is not a
    /// directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let given = root.as_ref();
        let meta = std::fs::metadata(given)
            .map_err(|_| Error::InvalidPath(format!("{} does not exist", given.display())))?;
        if !meta.is_dir() {
            return Err(Error::InvalidPath(format!(
                "{} is not a directory",
                given.display()
            )));
        }
        let root = std::path::absolute(given)?;
        let canonical_root = std::fs::canonicalize(&root)?;
        let config = root.join(CONFIG_DIR);
        if !config.is_dir() {
            std::fs::create_dir_all(&config)?;
        }
        Ok(Self {
            root,
            canonical_root,
        })
    }

    /// The vault root folder (absolute).
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
    ///
    /// This is a plain join and performs no containment check; use it for
    /// display or for handing paths to other programs. All the I/O methods of
    /// [`Vault`] perform their own checks.
    pub fn absolute(&self, path: &NotePath) -> PathBuf {
        self.root.join(path.as_str())
    }

    /// Absolute path of the `.obsidian` configuration folder.
    pub fn config_dir(&self) -> PathBuf {
        self.root.join(CONFIG_DIR)
    }

    /// All folders (vault-relative, sorted), excluding hidden/config/trash.
    pub fn list_folders(&self) -> Result<Vec<String>> {
        let (_, mut folders) = self.walk()?;
        folders.sort();
        Ok(folders)
    }

    /// Create a folder (and its parents). Succeeds if it already exists as a
    /// folder; fails with [`Error::AlreadyExists`] if a file is in the way.
    pub fn create_folder(&self, folder: &str) -> Result<()> {
        self.create_folder_impl(folder)
    }

    /// Create a new note `"<name>.md"` in `folder` (`""` = root). If it
    /// exists, uses `"<name> 1.md"`, `"<name> 2.md"`, ... like Obsidian.
    ///
    /// `name` is sanitized with [`sanitize_file_name`]; an empty name becomes
    /// `"Untitled"`. A trailing `.md` in `name` is ignored. The folder is
    /// created if missing.
    pub fn create_note(&self, folder: &str, name: &str, contents: &str) -> Result<NotePath> {
        self.create_note_impl(folder, name, contents)
    }

    /// Rename/move a folder and everything inside it.
    ///
    /// Fails with [`Error::AlreadyExists`] if `to` exists (case-only renames
    /// are allowed) and with [`Error::InvalidPath`] when moving a folder into
    /// itself or renaming the root.
    pub fn rename_folder(&self, from: &str, to: &str) -> Result<()> {
        self.rename_folder_impl(from, to)
    }

    /// Delete a file honoring `mode`.
    pub fn delete_with(&self, path: &NotePath, mode: TrashMode) -> Result<()> {
        self.delete_file_impl(path, mode)
    }

    /// Read `.obsidian/app.json`. Missing or invalid files yield defaults.
    pub fn app_config(&self) -> Result<AppConfig> {
        Ok(self
            .read_config_map(config::APP_JSON)?
            .map(|m| AppConfig::from_json_map(&m))
            .unwrap_or_default())
    }

    /// Write `.obsidian/app.json`, preserving [`AppConfig::extra`].
    pub fn save_app_config(&self, config: &AppConfig) -> Result<()> {
        let existing = self.read_config_map(config::APP_JSON)?.unwrap_or_default();
        let map = config.to_json_map_preserving(&existing);
        self.write_config_map(config::APP_JSON, map)
    }

    /// Read `.obsidian/daily-notes.json`. Missing or invalid files yield defaults.
    pub fn daily_notes_config(&self) -> Result<DailyNotesConfig> {
        Ok(self
            .read_config_map(config::DAILY_NOTES_JSON)?
            .map(|m| DailyNotesConfig::from_json_map(&m))
            .unwrap_or_default())
    }

    /// Read `.obsidian/templates.json`. Missing or invalid files yield defaults.
    pub fn templates_config(&self) -> Result<TemplatesConfig> {
        Ok(self
            .read_config_map(config::TEMPLATES_JSON)?
            .map(|m| TemplatesConfig::from_json_map(&m))
            .unwrap_or_default())
    }

    /// Open today's (or `date`'s) daily note, creating it from the template if needed.
    ///
    /// The path is `<folder>/<format(date)>.md`; the template (if configured
    /// and present) is expanded with `title` = file stem, `{{date}}` in the
    /// daily note format, `{{time}}` = current time, and also supports
    /// `{{yesterday}}` / `{{tomorrow}}`.
    pub fn open_daily_note(&self, date: Date) -> Result<NotePath> {
        self.open_daily_note_impl(date)
    }

    /// List template notes from the configured templates folder (recursive,
    /// markdown only, sorted). Empty if no folder is configured.
    pub fn list_templates(&self) -> Result<Vec<NotePath>> {
        self.list_templates_impl()
    }

    /// Start watching the vault for external changes. Events are debounced and
    /// filtered (no `.obsidian`, `.trash`, hidden files). Dropping the returned
    /// watcher stops it.
    ///
    /// See [`VaultWatcher`] for the exact event semantics. Writes made through
    /// this [`Vault`] are reported too (usually as [`ors_core::VaultEvent::Modified`]
    /// or `Created`); callers should ignore echoes of their own writes, e.g. by
    /// comparing contents or timestamps.
    pub fn watch(
        &self,
    ) -> Result<(
        VaultWatcher,
        std::sync::mpsc::Receiver<ors_core::VaultEvent>,
    )> {
        watch::start(self)
    }
}

impl VaultRead for Vault {
    fn list_files(&self) -> Result<Vec<NotePath>> {
        let (mut files, _) = self.walk()?;
        files.sort();
        Ok(files)
    }

    fn read_to_string(&self, path: &NotePath) -> Result<String> {
        let bytes = self.read_bytes(path)?;
        String::from_utf8(bytes).map_err(|e| Error::Parse(format!("{path}: {e}")))
    }

    fn read_bytes(&self, path: &NotePath) -> Result<Vec<u8>> {
        let abs = self.resolve(path)?;
        std::fs::read(&abs).map_err(|e| fsutil::map_io(e, path))
    }

    fn metadata(&self, path: &NotePath) -> Result<FileMeta> {
        let abs = self.resolve(path)?;
        let meta = std::fs::metadata(&abs).map_err(|e| fsutil::map_io(e, path))?;
        if !meta.is_file() {
            return Err(Error::NotFound(path.clone()));
        }
        let modified = meta.modified()?;
        let created = meta.created().unwrap_or(modified);
        Ok(FileMeta {
            size: meta.len(),
            modified,
            created,
        })
    }

    /// `true` if `path` is an existing file inside the vault (folders → `false`,
    /// see [`Vault::folder_exists`]).
    fn exists(&self, path: &NotePath) -> bool {
        self.resolve(path).is_ok_and(|p| p.is_file())
    }
}

impl VaultWrite for Vault {
    /// Atomic write: the contents go to a hidden temporary file in the same
    /// folder, which is then renamed over the target. Parent folders are created.
    fn write(&self, path: &NotePath, contents: &str) -> Result<()> {
        self.write_bytes(path, contents.as_bytes())
    }

    fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        self.rename_file_impl(from, to)
    }

    /// Uses the trash mode from `app_config()`.
    fn delete(&self, path: &NotePath) -> Result<()> {
        let mode = self.app_config()?.trash_mode;
        self.delete_with(path, mode)
    }
}
