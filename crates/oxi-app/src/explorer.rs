//! File explorer model (sort order, tree, selection) and the file
//! operations it triggers (move, copy, bulk delete), all on [`AppState`].

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use oxi_core::NotePath;
use serde::{Deserialize, Serialize};

use crate::state::{AppState, FsTarget};

/// Obsidian's file explorer sort orders (ids as in `workspace.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortOrder {
    #[default]
    #[serde(rename = "alphabetical")]
    NameAsc,
    #[serde(rename = "alphabeticalReverse")]
    NameDesc,
    #[serde(rename = "byModifiedTime")]
    ModifiedNewest,
    #[serde(rename = "byModifiedTimeReverse")]
    ModifiedOldest,
    #[serde(rename = "byCreatedTime")]
    CreatedNewest,
    #[serde(rename = "byCreatedTimeReverse")]
    CreatedOldest,
}

impl SortOrder {
    pub const ALL: [SortOrder; 6] = [
        SortOrder::NameAsc,
        SortOrder::NameDesc,
        SortOrder::ModifiedNewest,
        SortOrder::ModifiedOldest,
        SortOrder::CreatedNewest,
        SortOrder::CreatedOldest,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::NameAsc => "File name (A to Z)",
            SortOrder::NameDesc => "File name (Z to A)",
            SortOrder::ModifiedNewest => "Modified time (new to old)",
            SortOrder::ModifiedOldest => "Modified time (old to new)",
            SortOrder::CreatedNewest => "Created time (new to old)",
            SortOrder::CreatedOldest => "Created time (old to new)",
        }
    }
}

/// One file as the explorer sorts it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: NotePath,
    pub modified: SystemTime,
    pub created: SystemTime,
}

/// Natural-ish, case-insensitive name key ("note 2" < "note 10").
fn name_key(s: &str) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut digits = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_digit() != digits && !cur.is_empty() {
            out.push(chunk(&cur, digits));
            cur.clear();
        }
        digits = c.is_ascii_digit();
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(chunk(&cur, digits));
    }
    out
}

fn chunk(s: &str, digits: bool) -> (u8, String) {
    if digits {
        let trimmed = s.trim_start_matches('0');
        (0, format!("{:0>20}", trimmed))
    } else {
        (1, s.to_string())
    }
}

fn display_name(p: &NotePath) -> &str {
    if p.is_markdown() {
        p.stem()
    } else {
        p.file_name()
    }
}

pub fn sort_files(files: &mut [Entry], order: SortOrder) {
    let by_name = |a: &Entry, b: &Entry| {
        name_key(display_name(&a.path)).cmp(&name_key(display_name(&b.path)))
    };
    match order {
        SortOrder::NameAsc => files.sort_by(by_name),
        SortOrder::NameDesc => files.sort_by(|a, b| by_name(b, a)),
        SortOrder::ModifiedNewest => {
            files.sort_by(|a, b| b.modified.cmp(&a.modified).then(by_name(a, b)));
        }
        SortOrder::ModifiedOldest => {
            files.sort_by(|a, b| a.modified.cmp(&b.modified).then(by_name(a, b)));
        }
        SortOrder::CreatedNewest => {
            files.sort_by(|a, b| b.created.cmp(&a.created).then(by_name(a, b)));
        }
        SortOrder::CreatedOldest => {
            files.sort_by(|a, b| a.created.cmp(&b.created).then(by_name(a, b)));
        }
    }
}

/// Folder names: alphabetical, reversed only for "Z to A" (like Obsidian).
pub fn sort_folders(names: &mut [String], order: SortOrder) {
    names.sort_by_key(|a| name_key(a));
    if order == SortOrder::NameDesc {
        names.reverse();
    }
}

/// Explorer tree with children already sorted.
#[derive(Debug, Default, Clone)]
pub struct Tree {
    pub folders: Vec<(String, Tree)>,
    pub files: Vec<NotePath>,
}

#[derive(Default)]
struct RawTree {
    folders: BTreeMap<String, RawTree>,
    files: Vec<Entry>,
}

impl Tree {
    pub fn build(entries: Vec<Entry>, folders: &[String], order: SortOrder) -> Tree {
        let mut root = RawTree::default();
        for f in folders {
            let mut node = &mut root;
            for part in f.split('/') {
                node = node.folders.entry(part.to_string()).or_default();
            }
        }
        for e in entries {
            let mut node = &mut root;
            if let Some(parent) = e.path.parent() {
                for part in parent.split('/') {
                    node = node.folders.entry(part.to_string()).or_default();
                }
            }
            node.files.push(e);
        }
        Self::finish(root, order)
    }

    fn finish(raw: RawTree, order: SortOrder) -> Tree {
        let mut names: Vec<String> = raw.folders.keys().cloned().collect();
        sort_folders(&mut names, order);
        let mut map = raw.folders;
        let folders = names
            .into_iter()
            .filter_map(|n| map.remove(&n).map(|t| (n, Self::finish(t, order))))
            .collect();
        let mut files = raw.files;
        sort_files(&mut files, order);
        Tree {
            folders,
            files: files.into_iter().map(|e| e.path).collect(),
        }
    }

    /// Visible rows in order (for Shift+click ranges), honoring `expanded`.
    pub fn visible(&self, prefix: &str, expanded: &BTreeSet<String>, out: &mut Vec<FsTarget>) {
        for (name, child) in &self.folders {
            let path = join(prefix, name);
            out.push(FsTarget::Folder(path.clone()));
            if expanded.contains(&path) {
                child.visible(&path, expanded, out);
            }
        }
        out.extend(self.files.iter().cloned().map(FsTarget::File));
    }
}

pub fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Explorer UI state that outlives a frame.
#[derive(Debug, Clone, Default)]
pub struct ExplorerState {
    pub sort: SortOrder,
    pub expanded: BTreeSet<String>,
    pub selection: Vec<FsTarget>,
    pub anchor: Option<FsTarget>,
    /// Inline rename in progress: target, edited name, validation error.
    pub renaming: Option<(FsTarget, String)>,
    pub rename_error: Option<String>,
    /// Scroll the explorer to this row on the next frame.
    pub scroll_to: Option<FsTarget>,
    pub auto_reveal: bool,
    /// Last active file seen (auto reveal).
    pub last_active: Option<NotePath>,
}

impl ExplorerState {
    /// Expand all ancestors of `path` and scroll to it.
    pub fn reveal(&mut self, path: &NotePath) {
        let mut parent = path.parent();
        while let Some(p) = parent {
            self.expanded.insert(p.to_string());
            parent = p.rfind('/').map(|i| &p[..i]);
        }
        self.selection = vec![FsTarget::File(path.clone())];
        self.anchor = self.selection.first().cloned();
        self.scroll_to = Some(FsTarget::File(path.clone()));
    }

    /// Click on a row with modifiers: returns true when it is a plain click
    /// (the caller then opens the file).
    pub fn click(
        &mut self,
        target: &FsTarget,
        ctrl: bool,
        shift: bool,
        visible: &[FsTarget],
    ) -> bool {
        if shift && let Some(anchor) = &self.anchor {
            let a = visible.iter().position(|t| t == anchor);
            let b = visible.iter().position(|t| t == target);
            if let (Some(a), Some(b)) = (a, b) {
                let (lo, hi) = (a.min(b), a.max(b));
                self.selection = visible[lo..=hi].to_vec();
                return false;
            }
        }
        if ctrl {
            if let Some(i) = self.selection.iter().position(|t| t == target) {
                self.selection.remove(i);
            } else {
                self.selection.push(target.clone());
            }
            self.anchor = Some(target.clone());
            return false;
        }
        self.selection = vec![target.clone()];
        self.anchor = Some(target.clone());
        true
    }

    pub fn is_selected(&self, t: &FsTarget) -> bool {
        self.selection.contains(t)
    }

    /// Keep the expanded set and selection valid after a folder rename.
    pub fn folder_renamed(&mut self, from: &str, to: &str) {
        let prefix = format!("{from}/");
        self.expanded = std::mem::take(&mut self.expanded)
            .into_iter()
            .map(|f| {
                if f == from {
                    to.to_string()
                } else if let Some(rest) = f.strip_prefix(&prefix) {
                    format!("{to}/{rest}")
                } else {
                    f
                }
            })
            .collect();
        self.selection.clear();
    }
}

/// Selection without entries already covered by a selected parent folder.
pub fn top_level(targets: &[FsTarget]) -> Vec<FsTarget> {
    let folders: Vec<&String> = targets
        .iter()
        .filter_map(|t| match t {
            FsTarget::Folder(f) => Some(f),
            FsTarget::File(_) => None,
        })
        .collect();
    let inside = |p: &str| folders.iter().any(|f| p.starts_with(&format!("{f}/")));
    let mut out: Vec<FsTarget> = Vec::new();
    for t in targets {
        let covered = match t {
            FsTarget::File(p) => inside(p.as_str()),
            FsTarget::Folder(f) => inside(f),
        };
        if !covered && !out.contains(t) {
            out.push(t.clone());
        }
    }
    out
}

impl AppState {
    /// Files with their timestamps, for sorting.
    pub fn explorer_entries(&self) -> Vec<Entry> {
        self.files
            .iter()
            .map(|p| {
                let meta = self.vault.metadata(p).ok();
                Entry {
                    path: p.clone(),
                    modified: meta.as_ref().map_or(SystemTime::UNIX_EPOCH, |m| m.modified),
                    created: meta.map_or(SystemTime::UNIX_EPOCH, |m| m.created),
                }
            })
            .collect()
    }

    /// Move a file into `folder` (`""` = root), updating links.
    pub fn move_file_to(&mut self, from: &NotePath, folder: &str) -> bool {
        let Ok(to) = NotePath::join(folder, from.file_name()) else {
            self.error(format!("Invalid folder: {folder}"));
            return false;
        };
        if &to == from {
            return true;
        }
        if self.vault.exists(&to) {
            self.error(format!("{to} already exists"));
            return false;
        }
        if !self.save_all() {
            return false;
        }
        match self
            .index
            .rename_with_links(self.vault.as_write(), from, &to)
        {
            Ok(changed) => {
                self.retarget(from, &to);
                for p in changed {
                    self.reload_clean_buffer(&p);
                }
                self.refresh_files();
                self.bump();
                true
            }
            Err(e) => {
                self.error(format!("Cannot move {from}: {e}"));
                false
            }
        }
    }

    /// Move folder `from` into `parent` (`""` = root), updating links.
    pub fn move_folder_to(&mut self, from: &str, parent: &str) -> bool {
        let name = from.rsplit('/').next().unwrap_or(from);
        let to = join(parent, name);
        if to == from {
            return true;
        }
        if parent == from || parent.starts_with(&format!("{from}/")) {
            self.error("Cannot move a folder into itself");
            return false;
        }
        if self.folders.contains(&to) {
            self.error(format!("{to} already exists"));
            return false;
        }
        if !self.save_all() {
            return false;
        }
        let prefix = format!("{from}/");
        let moved: Vec<(NotePath, NotePath)> = self
            .files
            .iter()
            .filter_map(|f| {
                let rest = f.as_str().strip_prefix(&prefix)?;
                Some((f.clone(), NotePath::join(&to, rest).ok()?))
            })
            .collect();
        // Sub-folders without files are recreated at the destination.
        let empty_subfolders: Vec<String> = self
            .folders
            .iter()
            .filter_map(|f| f.strip_prefix(&prefix).map(|rest| join(&to, rest)))
            .collect();
        match self
            .index
            .rename_folder_with_links(self.vault.as_write(), from, &to)
        {
            Ok(changed) => {
                let _ = self.vault.create_folder(&to);
                for f in empty_subfolders {
                    let _ = self.vault.create_folder(&f);
                }
                for (a, b) in &moved {
                    self.retarget(a, b);
                }
                for p in changed {
                    self.reload_clean_buffer(&p);
                }
                let still_used = self
                    .vault
                    .list_files()
                    .is_ok_and(|fs| fs.iter().any(|f| f.as_str().starts_with(&prefix)));
                if !still_used {
                    let _ = self.vault.delete_folder(from);
                }
                self.ws.bookmarks.rename_folder(from, &to);
                self.ws.explorer.folder_renamed(from, &to);
                self.ws.save_bookmarks_to(&*self.vault);
                self.refresh_files();
                self.bump();
                true
            }
            Err(e) => {
                self.error(format!("Cannot move folder: {e}"));
                false
            }
        }
    }

    /// Move files and folders into `folder`.
    pub fn move_targets(&mut self, targets: &[FsTarget], folder: &str) {
        for t in top_level(targets) {
            match &t {
                FsTarget::File(p) => {
                    if p.parent().unwrap_or("") != folder {
                        self.move_file_to(p, folder);
                    }
                }
                FsTarget::Folder(f) => {
                    let parent = f.rsplit_once('/').map_or("", |(p, _)| p);
                    if parent != folder {
                        self.move_folder_to(f, folder);
                    }
                }
            }
        }
        self.ws.explorer.selection.clear();
    }

    pub fn delete_targets(&mut self, targets: &[FsTarget]) {
        for t in top_level(targets) {
            self.delete(&t);
        }
        self.ws.explorer.selection.clear();
    }

    /// "Make a copy": `<name> 1.md` next to the original, opened.
    pub fn duplicate_file(&mut self, path: &NotePath) -> Option<NotePath> {
        if !path.is_markdown() {
            self.error("Only notes can be copied");
            return None;
        }
        self.save(path);
        let text = self.note_text(path).unwrap_or_default();
        let folder = path.parent().unwrap_or("");
        match self.vault.create_note(folder, path.stem(), &text) {
            Ok(p) => {
                self.index.update_file(self.vault.as_read(), &p);
                self.refresh_files();
                self.bump();
                self.open(&p, crate::state::OpenIn::CurrentTab);
                Some(p)
            }
            Err(e) => {
                self.error(format!("Cannot copy {path}: {e}"));
                None
            }
        }
    }

    /// Inline rename from the explorer; returns an error message if the
    /// name is invalid (rename not attempted).
    pub fn rename_target(&mut self, target: &FsTarget, name: &str) -> Result<(), String> {
        let name = name.trim();
        if !oxi_vault::is_valid_file_name(name) {
            return Err(format!(
                "File name cannot be empty or contain any of: {}",
                oxi_vault::FORBIDDEN_NAME_CHARS.iter().collect::<String>()
            ));
        }
        match target {
            FsTarget::File(p) => {
                let ext = p.extension();
                let file_name = if ext.is_empty() {
                    name.to_string()
                } else {
                    format!("{name}.{ext}")
                };
                if let Ok(to) = p.with_file_name(&file_name)
                    && &to != p
                    && self.vault.exists(&to)
                {
                    return Err("A file with this name already exists".into());
                }
                let stem = name.to_string();
                self.rename_file(p, &stem);
            }
            FsTarget::Folder(f) => {
                let parent = f.rsplit_once('/').map_or("", |(p, _)| p);
                let to = join(parent, name);
                if to != *f && self.folders.contains(&to) {
                    return Err("A folder with this name already exists".into());
                }
                self.rename_folder(f, name);
                self.ws.bookmarks.rename_folder(f, &to);
                self.ws.explorer.folder_renamed(f, &to);
                self.ws.save_bookmarks_to(&*self.vault);
            }
        }
        Ok(())
    }

    /// Absolute path on disk, for "Reveal in system file manager".
    pub fn absolute_path(&self, target: &FsTarget) -> Option<std::path::PathBuf> {
        let root = self.vault.root()?;
        Some(match target {
            FsTarget::File(p) => root.join(p.as_str()),
            FsTarget::Folder(f) => root.join(f),
        })
    }

    /// Open the system file manager on the folder containing `target`.
    pub fn reveal_in_system(&mut self, target: &FsTarget) {
        let Some(abs) = self.absolute_path(target) else {
            self.info("This vault is not on disk");
            return;
        };
        let dir = match target {
            FsTarget::File(_) => abs.parent().map(std::path::Path::to_path_buf),
            FsTarget::Folder(_) => Some(abs),
        };
        let Some(dir) = dir else { return };
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(program).arg(&dir).spawn() {
            self.error(format!("Cannot open the file manager: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn e(p: &str, m: u64, c: u64) -> Entry {
        Entry {
            path: NotePath::new(p).unwrap(),
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(m),
            created: SystemTime::UNIX_EPOCH + Duration::from_secs(c),
        }
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.path.file_name()).collect()
    }

    #[test]
    fn sort_orders() {
        let mut v = vec![
            e("b.md", 3, 1),
            e("Note 10.md", 1, 2),
            e("a.png", 2, 3),
            e("note 2.md", 4, 4),
        ];
        sort_files(&mut v, SortOrder::NameAsc);
        assert_eq!(names(&v), ["a.png", "b.md", "note 2.md", "Note 10.md"]);
        sort_files(&mut v, SortOrder::NameDesc);
        assert_eq!(names(&v), ["Note 10.md", "note 2.md", "b.md", "a.png"]);
        sort_files(&mut v, SortOrder::ModifiedNewest);
        assert_eq!(names(&v), ["note 2.md", "b.md", "a.png", "Note 10.md"]);
        sort_files(&mut v, SortOrder::ModifiedOldest);
        assert_eq!(names(&v)[0], "Note 10.md");
        sort_files(&mut v, SortOrder::CreatedNewest);
        assert_eq!(names(&v)[0], "note 2.md");
        sort_files(&mut v, SortOrder::CreatedOldest);
        assert_eq!(names(&v)[0], "b.md");
    }

    #[test]
    fn tree_build_and_visible_rows() {
        let t = Tree::build(
            vec![e("z.md", 0, 0), e("B/x.md", 0, 0), e("a/y.md", 0, 0)],
            &["a".into(), "B".into(), "a/empty".into()],
            SortOrder::NameAsc,
        );
        assert_eq!(
            t.folders.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
            ["a", "B"]
        );
        assert_eq!(t.folders[0].1.folders[0].0, "empty");
        let mut expanded = BTreeSet::new();
        let mut rows = Vec::new();
        t.visible("", &expanded, &mut rows);
        assert_eq!(rows.len(), 3);
        expanded.insert("a".to_string());
        rows.clear();
        t.visible("", &expanded, &mut rows);
        assert_eq!(rows[1], FsTarget::Folder("a/empty".into()));
        assert_eq!(rows[2], FsTarget::File(NotePath::new("a/y.md").unwrap()));
        let z = Tree::build(vec![], &["a".into(), "b".into()], SortOrder::NameDesc);
        assert_eq!(z.folders[0].0, "b");
    }

    #[test]
    fn selection_with_modifiers() {
        let rows: Vec<FsTarget> = ["a.md", "b.md", "c.md", "d.md"]
            .iter()
            .map(|p| FsTarget::File(NotePath::new(*p).unwrap()))
            .collect();
        let mut s = ExplorerState::default();
        assert!(s.click(&rows[1], false, false, &rows));
        assert!(!s.click(&rows[3], false, true, &rows));
        assert_eq!(s.selection.len(), 3);
        assert!(!s.click(&rows[2], true, false, &rows));
        assert_eq!(s.selection, vec![rows[1].clone(), rows[3].clone()]);
        s.reveal(&NotePath::new("x/y/z.md").unwrap());
        assert!(s.expanded.contains("x") && s.expanded.contains("x/y"));
        s.folder_renamed("x", "w");
        assert!(s.expanded.contains("w/y"));
    }

    #[test]
    fn top_level_skips_nested() {
        let t = vec![
            FsTarget::Folder("a".into()),
            FsTarget::File(NotePath::new("a/b.md").unwrap()),
            FsTarget::Folder("a/c".into()),
            FsTarget::File(NotePath::new("c.md").unwrap()),
        ];
        assert_eq!(top_level(&t).len(), 2);
    }
}
