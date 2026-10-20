//! Application state, independent of any drawing code so that it can be
//! unit-tested without a display.
//!
//! Time is passed in explicitly (seconds, monotonic) through
//! [`AppState::tick`], which makes the autosave debounce deterministic in
//! tests.

use std::collections::HashMap;

use ors_core::{Heading, NotePath, ParsedNote, VaultEvent, VaultRead};
use ors_index::{GraphOptions, ResolvedLink, SearchResult};

use crate::backend::{IndexBackend, LayoutEngine, VaultBackend};
use crate::commands::{Command, CommandRegistry};
use crate::settings::{LeftPane, Settings, ThemeChoice};
use crate::util::{today, valid_file_name};

pub type TabId = u64;

/// Builds the index for a vault (used on open and on "Reload vault").
pub type IndexFactory = fn(&dyn VaultRead) -> Box<dyn IndexBackend>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Source,
    Reading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenIn {
    CurrentTab,
    NewTab,
}

/// A note (or attachment) tab with its own navigation history.
#[derive(Debug, Clone)]
pub struct NoteTab {
    pub path: NotePath,
    pub mode: ViewMode,
    pub back: Vec<NotePath>,
    pub forward: Vec<NotePath>,
    /// Line to reveal on next draw (search result, heading click).
    pub pending_line: Option<usize>,
}

/// Graph view tab state.
pub struct GraphTab {
    /// Local graph around `center` instead of the whole vault.
    pub local: bool,
    pub center: Option<NotePath>,
    pub depth: usize,
    pub show_tags: bool,
    pub show_attachments: bool,
    pub show_unresolved: bool,
    pub show_orphans: bool,
    pub filter: String,
    pub data: ors_core::GraphData,
    pub layout: Option<Box<dyn LayoutEngine>>,
    /// Key of the last build (index generation + options), to rebuild lazily.
    built_key: Option<String>,
    pub pan: (f32, f32),
    pub zoom: f32,
    pub dragging: Option<usize>,
    pub energy: f32,
    pub show_filters: bool,
    /// The view was fitted to the graph bounds once.
    pub fitted: bool,
}

impl GraphTab {
    pub fn new(center: Option<NotePath>) -> Self {
        Self {
            local: center.is_some(),
            center,
            depth: 1,
            show_tags: false,
            show_attachments: false,
            show_unresolved: true,
            show_orphans: true,
            filter: String::new(),
            data: ors_core::GraphData::default(),
            layout: None,
            built_key: None,
            pan: (0.0, 0.0),
            zoom: 1.0,
            dragging: None,
            energy: 1.0,
            show_filters: true,
            fitted: false,
        }
    }

    fn key(&self, generation: u64) -> String {
        format!(
            "{generation}|{}|{:?}|{}|{}{}{}{}|{}",
            self.local,
            self.center,
            self.depth,
            self.show_tags,
            self.show_attachments,
            self.show_unresolved,
            self.show_orphans,
            self.filter
        )
    }

    /// Rebuild the displayed graph if the index or the options changed.
    /// Returns true when rebuilt.
    pub fn refresh(&mut self, index: &dyn IndexBackend, generation: u64) -> bool {
        let key = self.key(generation);
        if self.built_key.as_deref() == Some(key.as_str()) {
            return false;
        }
        let opts = GraphOptions {
            include_tags: self.show_tags,
            include_attachments: self.show_attachments,
            include_unresolved: self.show_unresolved,
            include_orphans: self.show_orphans || self.local,
        };
        let mut g = index.graph(&opts);
        if self.local
            && let Some(c) = &self.center
        {
            g = match g.node_index(c.as_str()) {
                Some(i) => ors_graph::local_graph(&g, i, self.depth),
                None => ors_core::GraphData::default(),
            };
        }
        let filter = self.filter.trim().to_lowercase();
        if !filter.is_empty() {
            g = ors_graph::filter_graph(&g, |n| {
                n.label.to_lowercase().contains(&filter) || n.id.to_lowercase().contains(&filter)
            });
        }
        match &mut self.layout {
            Some(layout) => layout.sync(&self.data, &g),
            None => self.layout = Some(index.new_layout(&g)),
        }
        self.data = g;
        self.built_key = Some(key);
        self.energy = 1.0;
        true
    }
}

pub enum TabContent {
    Note(NoteTab),
    Graph(Box<GraphTab>),
}

pub struct Tab {
    pub id: TabId,
    pub content: TabContent,
}

impl Tab {
    pub fn note(&self) -> Option<&NoteTab> {
        match &self.content {
            TabContent::Note(n) => Some(n),
            TabContent::Graph(_) => None,
        }
    }
    pub fn note_mut(&mut self) -> Option<&mut NoteTab> {
        match &mut self.content {
            TabContent::Note(n) => Some(n),
            TabContent::Graph(_) => None,
        }
    }
    pub fn title(&self) -> String {
        match &self.content {
            TabContent::Note(n) => n.path.stem().to_string(),
            TabContent::Graph(g) => match (&g.center, g.local) {
                (Some(c), true) => format!("Local graph: {}", c.stem()),
                _ => "Graph view".to_string(),
            },
        }
    }
}

/// An open note's text.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub text: String,
    /// Contents last read from / written to disk.
    pub saved: String,
    /// Time of the last unsaved edit (autosave debounce).
    pub last_edit: Option<f64>,
    /// Set when the file changed on disk while the buffer was dirty: holds
    /// the disk version until the user picks one.
    pub conflict: Option<String>,
}

impl Buffer {
    fn new(text: String) -> Self {
        Self {
            saved: text.clone(),
            text,
            last_edit: None,
            conflict: None,
        }
    }
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsTarget {
    File(NotePath),
    Folder(String),
}

impl FsTarget {
    pub fn display(&self) -> String {
        match self {
            FsTarget::File(p) => p.as_str().to_string(),
            FsTarget::Folder(f) => format!("{f}/"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    QuickSwitcher { query: String, selected: usize },
    CommandPalette { query: String, selected: usize },
    Rename { target: FsTarget, name: String },
    NewFolder { parent: String, name: String },
    ConfirmDelete { target: FsTarget },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub created: f64,
}

/// Information shown in the right sidebar and status bar for the active note.
#[derive(Debug, Clone, Default)]
pub struct NoteInfo {
    pub path: Option<NotePath>,
    /// Backlinks grouped by source note: (line, line text) per link.
    pub backlinks: Vec<(NotePath, Vec<(usize, String)>)>,
    pub backlink_count: usize,
    pub unlinked: Vec<NotePath>,
    pub outgoing: Vec<ResolvedLink>,
    pub headings: Vec<Heading>,
    pub word_count: usize,
}

pub struct AppState {
    pub vault: Box<dyn VaultBackend>,
    pub index: Box<dyn IndexBackend>,
    index_factory: IndexFactory,
    pub commands: CommandRegistry,
    pub settings: Settings,
    /// Vault files and folders (explorer).
    pub files: Vec<NotePath>,
    pub folders: Vec<String>,
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    pub buffers: HashMap<NotePath, Buffer>,
    pub modal: Option<Modal>,
    pub toasts: Vec<Toast>,
    /// Bumped whenever the index changes.
    pub generation: u64,
    pub now: f64,
    pub recent_files: Vec<NotePath>,
    pub search_query: String,
    pub search_results: Result<Vec<SearchResult>, String>,
    search_key: Option<(String, u64)>,
    info: NoteInfo,
    info_key: Option<(Option<NotePath>, u64)>,
    info_time: f64,
    next_tab_id: TabId,
    /// Set by the "Open another vault" command; the UI drops the state.
    pub switch_vault_requested: bool,
    /// Focus the search field on next frame.
    pub focus_search: bool,
}

impl AppState {
    pub fn new(
        mut vault: Box<dyn VaultBackend>,
        index_factory: IndexFactory,
        settings: Settings,
    ) -> Self {
        let index = index_factory(vault.as_read());
        let mut state = Self {
            index,
            index_factory,
            commands: CommandRegistry::default(),
            settings,
            files: Vec::new(),
            folders: Vec::new(),
            tabs: Vec::new(),
            active: None,
            buffers: HashMap::new(),
            modal: None,
            toasts: Vec::new(),
            generation: 0,
            now: 0.0,
            recent_files: Vec::new(),
            search_query: String::new(),
            search_results: Ok(Vec::new()),
            search_key: None,
            info: NoteInfo::default(),
            info_key: None,
            info_time: f64::MIN,
            next_tab_id: 1,
            switch_vault_requested: false,
            focus_search: false,
            vault: {
                if let Err(e) = vault.start_watching() {
                    eprintln!("file watcher unavailable: {e}");
                }
                vault
            },
        };
        state.refresh_files();
        state
    }

    // ----- notifications -------------------------------------------------

    pub fn info(&mut self, text: impl Into<String>) {
        self.toast(text.into(), ToastKind::Info);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toast(text.into(), ToastKind::Error);
    }

    fn toast(&mut self, text: String, kind: ToastKind) {
        if self.toasts.last().is_some_and(|t| t.text == text) {
            return;
        }
        self.toasts.push(Toast {
            text,
            kind,
            created: self.now,
        });
    }

    // ----- files ---------------------------------------------------------

    pub fn refresh_files(&mut self) {
        match self.vault.list_files() {
            Ok(f) => self.files = f,
            Err(e) => self.error(format!("Cannot list files: {e}")),
        }
        // Files known to the index but not listed by the vault (e.g. unsaved).
        for f in self.index.files() {
            if let Err(i) = self.files.binary_search(&f) {
                self.files.insert(i, f);
            }
        }
        self.folders = self.vault.list_folders().unwrap_or_default();
    }

    fn bump(&mut self) {
        self.generation += 1;
    }

    // ----- tabs ----------------------------------------------------------

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.active.and_then(|i| self.tabs.get_mut(i))
    }

    pub fn active_note(&self) -> Option<&NotePath> {
        self.active_tab().and_then(Tab::note).map(|n| &n.path)
    }

    fn push_tab(&mut self, content: TabContent) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let at = self.active.map_or(self.tabs.len(), |i| i + 1);
        self.tabs.insert(at, Tab { id, content });
        self.set_active(at);
        at
    }

    /// Activate tab `i`, autosaving what was being edited.
    pub fn set_active(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if self.active != Some(i) {
            self.save_all();
        }
        self.active = Some(i);
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(i);
        if let TabContent::Note(n) = &tab.content {
            self.release_buffer(&n.path);
        }
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.min(self.tabs.len() - 1)),
            other => other,
        };
    }

    pub fn close_tab_by_id(&mut self, id: TabId) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
            self.close_tab(i);
        }
    }

    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let active_id = self.active_tab().map(|t| t.id);
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = active_id.and_then(|id| self.tabs.iter().position(|t| t.id == id));
    }

    pub fn cycle_tab(&mut self, delta: isize) {
        let n = self.tabs.len() as isize;
        if n == 0 {
            return;
        }
        let cur = self.active.unwrap_or(0) as isize;
        self.set_active((cur + delta).rem_euclid(n) as usize);
    }

    /// Save the buffer and drop it if no tab shows it anymore.
    fn release_buffer(&mut self, path: &NotePath) {
        self.save(path);
        let used = self
            .tabs
            .iter()
            .any(|t| t.note().is_some_and(|n| &n.path == path));
        let clean = self.buffers.get(path).is_some_and(|b| !b.is_dirty());
        if !used && clean {
            self.buffers.remove(path);
        }
    }

    fn ensure_buffer(&mut self, path: &NotePath) {
        if !path.is_markdown() || self.buffers.contains_key(path) {
            return;
        }
        match self.vault.read_to_string(path) {
            Ok(text) => {
                self.buffers.insert(path.clone(), Buffer::new(text));
            }
            Err(e) if self.vault.exists(path) => {
                self.error(format!("Cannot read {path}: {e}"));
            }
            Err(_) => {
                // New/unsaved or missing file: start empty.
                self.buffers
                    .insert(path.clone(), Buffer::new(String::new()));
            }
        }
    }

    /// Open a file. In the current tab this records navigation history.
    pub fn open(&mut self, path: &NotePath, open_in: OpenIn) {
        self.ensure_buffer(path);
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(30);
        let current_is_note = self.active_tab().is_some_and(|t| t.note().is_some());
        if open_in == OpenIn::NewTab || !current_is_note {
            self.push_tab(TabContent::Note(NoteTab {
                path: path.clone(),
                mode: ViewMode::Source,
                back: Vec::new(),
                forward: Vec::new(),
                pending_line: None,
            }));
            return;
        }
        let Some(tab) = self.active_tab_mut().and_then(Tab::note_mut) else {
            return;
        };
        if &tab.path == path {
            return;
        }
        let old = std::mem::replace(&mut tab.path, path.clone());
        tab.back.push(old.clone());
        tab.forward.clear();
        tab.pending_line = None;
        self.release_buffer(&old);
    }

    /// Open and scroll to a line.
    pub fn open_at_line(&mut self, path: &NotePath, line: usize, open_in: OpenIn) {
        self.open(path, open_in);
        if let Some(tab) = self.active_tab_mut().and_then(Tab::note_mut) {
            tab.pending_line = Some(line);
            tab.mode = ViewMode::Source;
        }
    }

    pub fn navigate(&mut self, forward: bool) {
        let Some(tab) = self.active_tab_mut().and_then(Tab::note_mut) else {
            return;
        };
        let target = if forward {
            tab.forward.pop()
        } else {
            tab.back.pop()
        };
        let Some(target) = target else { return };
        let old = std::mem::replace(&mut tab.path, target.clone());
        if forward {
            tab.back.push(old.clone());
        } else {
            tab.forward.push(old.clone());
        }
        self.ensure_buffer(&target);
        self.release_buffer(&old);
    }

    pub fn toggle_mode(&mut self) {
        if let Some(tab) = self.active_tab_mut().and_then(Tab::note_mut) {
            tab.mode = match tab.mode {
                ViewMode::Source => ViewMode::Reading,
                ViewMode::Reading => ViewMode::Source,
            };
        }
    }

    pub fn open_graph(&mut self, local: bool) {
        let center = if local {
            match self.active_note() {
                Some(p) => Some(p.clone()),
                None => {
                    self.info("Open a note to show its local graph");
                    return;
                }
            }
        } else {
            None
        };
        if !local
            && let Some(i) = self
                .tabs
                .iter()
                .position(|t| matches!(&t.content, TabContent::Graph(g) if !g.local))
        {
            self.set_active(i);
            return;
        }
        self.push_tab(TabContent::Graph(Box::new(GraphTab::new(center))));
    }

    // ----- editing & saving ---------------------------------------------

    /// Must be called after the UI modified `buffers[path].text`.
    pub fn buffer_edited(&mut self, path: &NotePath) {
        let now = self.now;
        if let Some(b) = self.buffers.get_mut(path) {
            b.last_edit = Some(now);
            let text = b.text.clone();
            self.index.update_from_source(path, &text);
            self.bump();
        }
    }

    /// Advance time: autosave debounced buffers and expire toasts.
    pub fn tick(&mut self, now: f64) {
        self.now = now;
        let delay = self.settings.autosave_delay;
        let due: Vec<NotePath> = self
            .buffers
            .iter()
            .filter(|(_, b)| {
                b.conflict.is_none()
                    && b.is_dirty()
                    && b.last_edit.is_some_and(|t| now - t >= delay)
            })
            .map(|(p, _)| p.clone())
            .collect();
        for p in due {
            self.save(&p);
        }
        self.toasts.retain(|t| now - t.created < 5.0);
    }

    /// Seconds until the next autosave is due (for scheduling a repaint).
    pub fn next_autosave_in(&self) -> Option<f64> {
        self.buffers
            .values()
            .filter(|b| b.conflict.is_none() && b.is_dirty())
            .filter_map(|b| b.last_edit)
            .map(|t| (t + self.settings.autosave_delay - self.now).max(0.0))
            .min_by(f64::total_cmp)
    }

    /// Write a dirty buffer to disk. Returns false on error.
    pub fn save(&mut self, path: &NotePath) -> bool {
        let Some(b) = self.buffers.get(path) else {
            return true;
        };
        if !b.is_dirty() || b.conflict.is_some() {
            return true;
        }
        let text = b.text.clone();
        match self.vault.write(path, &text) {
            Ok(()) => {
                if let Some(b) = self.buffers.get_mut(path) {
                    b.saved = text;
                    b.last_edit = None;
                }
                if let Err(i) = self.files.binary_search(path) {
                    self.files.insert(i, path.clone());
                }
                true
            }
            Err(e) => {
                let retry = self.now + 5.0;
                if let Some(b) = self.buffers.get_mut(path) {
                    b.last_edit = Some(retry);
                }
                self.error(format!("Cannot save {path}: {e}"));
                false
            }
        }
    }

    pub fn save_all(&mut self) -> bool {
        let dirty: Vec<NotePath> = self
            .buffers
            .iter()
            .filter(|(_, b)| b.is_dirty())
            .map(|(p, _)| p.clone())
            .collect();
        let mut ok = true;
        for p in dirty {
            ok &= self.save(&p);
        }
        ok
    }

    /// Resolve a conflict: keep the editor version (overwrite disk) or
    /// take the disk version.
    pub fn resolve_conflict(&mut self, path: &NotePath, keep_mine: bool) {
        let Some(b) = self.buffers.get_mut(path) else {
            return;
        };
        let Some(disk) = b.conflict.take() else {
            return;
        };
        if keep_mine {
            b.saved = disk;
            self.save(path);
        } else {
            b.text = disk.clone();
            b.saved = disk;
            b.last_edit = None;
            self.index
                .update_from_source(path, &self.buffers[path].text.clone());
            self.bump();
        }
    }

    // ----- external changes ---------------------------------------------

    pub fn poll_vault(&mut self) {
        let events = self.vault.poll_events();
        if events.is_empty() {
            return;
        }
        for ev in events {
            self.handle_vault_event(ev);
        }
        self.refresh_files();
        self.bump();
    }

    pub fn handle_vault_event(&mut self, ev: VaultEvent) {
        match ev {
            VaultEvent::Created(p) | VaultEvent::Modified(p) => {
                self.index.update_file(self.vault.as_read(), &p);
                self.on_disk_change(&p);
            }
            VaultEvent::Deleted(p) => {
                self.index.remove_file(&p);
                let dirty = self.buffers.get(&p).is_some_and(Buffer::is_dirty);
                if dirty {
                    self.info(format!("{p} was deleted on disk; your edits are kept"));
                } else {
                    self.close_tabs_of(&p);
                }
            }
            VaultEvent::Renamed { from, to } => {
                self.index.rename_file(&from, &to);
                self.index.update_file(self.vault.as_read(), &to);
                self.retarget(&from, &to);
            }
        }
    }

    fn on_disk_change(&mut self, path: &NotePath) {
        let Some(b) = self.buffers.get(path) else {
            return;
        };
        let Ok(disk) = self.vault.read_to_string(path) else {
            return;
        };
        if disk == b.saved || disk == b.text {
            // Echo of our own save (or identical content).
            if let Some(b) = self.buffers.get_mut(path) {
                b.saved = disk;
            }
        } else if !b.is_dirty() {
            if let Some(b) = self.buffers.get_mut(path) {
                b.text = disk.clone();
                b.saved = disk;
            }
        } else {
            let text = b.text.clone();
            if let Some(b) = self.buffers.get_mut(path) {
                b.conflict = Some(disk);
            }
            // Keep the index in sync with what the user sees.
            self.index.update_from_source(path, &text);
            self.info(format!(
                "{} changed on disk while you were editing it",
                path.stem()
            ));
        }
    }

    fn close_tabs_of(&mut self, path: &NotePath) {
        while let Some(i) = self
            .tabs
            .iter()
            .position(|t| t.note().is_some_and(|n| &n.path == path))
        {
            self.close_tab(i);
        }
        self.buffers.remove(path);
        self.recent_files.retain(|p| p != path);
    }

    /// Point tabs, buffers and history at a renamed file.
    fn retarget(&mut self, from: &NotePath, to: &NotePath) {
        for t in &mut self.tabs {
            if let Some(n) = t.note_mut() {
                if &n.path == from {
                    n.path = to.clone();
                }
                for p in n.back.iter_mut().chain(n.forward.iter_mut()) {
                    if p == from {
                        *p = to.clone();
                    }
                }
            }
            if let TabContent::Graph(g) = &mut t.content
                && g.center.as_ref() == Some(from)
            {
                g.center = Some(to.clone());
            }
        }
        if let Some(b) = self.buffers.remove(from) {
            self.buffers.insert(to.clone(), b);
        }
        for p in &mut self.recent_files {
            if p == from {
                *p = to.clone();
            }
        }
    }

    // ----- file operations ----------------------------------------------

    /// Folder for new notes according to `.obsidian/app.json`.
    pub fn new_note_folder(&self) -> String {
        let cfg = self.vault.app_config().unwrap_or_default();
        match cfg.new_file_location.as_deref() {
            Some("current") => self
                .active_note()
                .and_then(NotePath::parent)
                .unwrap_or("")
                .to_string(),
            Some("folder") => cfg
                .new_file_folder_path
                .unwrap_or_default()
                .trim_matches('/')
                .to_string(),
            _ => String::new(),
        }
    }

    /// Create a note named `name` in `folder` and open it.
    pub fn create_note_in(
        &mut self,
        folder: &str,
        name: &str,
        open_in: OpenIn,
    ) -> Option<NotePath> {
        if !folder.is_empty() && !self.folders.iter().any(|f| f == folder) {
            let _ = self.vault.create_folder(folder);
        }
        match self.vault.create_note(folder, name, "") {
            Ok(p) => {
                self.index.update_file(self.vault.as_read(), &p);
                if !self.index.files().contains(&p) {
                    self.index.update_from_source(&p, "");
                }
                self.refresh_files();
                self.bump();
                self.open(&p, open_in);
                Some(p)
            }
            Err(e) => {
                self.error(format!("Cannot create note: {e}"));
                None
            }
        }
    }

    pub fn new_note(&mut self) {
        let folder = self.new_note_folder();
        self.create_note_in(&folder, "Untitled", OpenIn::NewTab);
    }

    /// Open the note `name` (may contain folders), creating it if missing
    /// (quick switcher Shift+Enter, unresolved link click).
    pub fn open_or_create(&mut self, name: &str, open_in: OpenIn) {
        let name = name.trim().trim_end_matches(".md");
        if name.is_empty() {
            return;
        }
        let (folder, stem) = match name.rsplit_once('/') {
            Some((f, s)) => (f.trim_matches('/').to_string(), s),
            None => (self.new_note_folder(), name),
        };
        if !valid_file_name(stem) {
            self.error(format!("Invalid note name: {stem}"));
            return;
        }
        if let Ok(p) = NotePath::join(&folder, &format!("{stem}.md"))
            && (self.vault.exists(&p) || self.files.contains(&p))
        {
            self.open(&p, open_in);
            return;
        }
        self.create_note_in(&folder, stem, open_in);
    }

    /// Follow an internal link written in `from`.
    pub fn follow_link(&mut self, target: &str, from: &NotePath, open_in: OpenIn) {
        if target.is_empty() {
            return;
        }
        match self.index.resolve(target, from) {
            Some(p) => self.open(&p, open_in),
            None => self.open_or_create(target, open_in),
        }
    }

    pub fn open_daily_note(&mut self) {
        match self.vault.open_daily_note(today()) {
            Ok(p) => {
                self.index.update_file(self.vault.as_read(), &p);
                self.refresh_files();
                self.bump();
                self.open(&p, OpenIn::CurrentTab);
            }
            Err(e) => self.error(format!("Cannot open daily note: {e}")),
        }
    }

    pub fn create_folder(&mut self, parent: &str, name: &str) {
        if !valid_file_name(name) {
            self.error(format!("Invalid folder name: {name}"));
            return;
        }
        let path = if parent.is_empty() {
            name.trim().to_string()
        } else {
            format!("{parent}/{}", name.trim())
        };
        match self.vault.create_folder(&path) {
            Ok(()) => self.refresh_files(),
            Err(e) => self.error(format!("Cannot create folder: {e}")),
        }
    }

    /// Rename a note (keeping its extension) and update links to it.
    pub fn rename_file(&mut self, from: &NotePath, new_stem: &str) {
        let new_stem = new_stem.trim();
        if !valid_file_name(new_stem) {
            self.error(format!("Invalid name: {new_stem}"));
            return;
        }
        let ext = from.extension();
        let file_name = if ext.is_empty() {
            new_stem.to_string()
        } else {
            format!("{new_stem}.{ext}")
        };
        let Ok(to) = from.with_file_name(&file_name) else {
            self.error(format!("Invalid name: {new_stem}"));
            return;
        };
        if &to == from {
            return;
        }
        if self.vault.exists(&to) {
            self.error(format!("{to} already exists"));
            return;
        }
        if !self.save_all() {
            return;
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
            }
            Err(e) => self.error(format!("Cannot rename {from}: {e}")),
        }
    }

    pub fn rename_folder(&mut self, from: &str, new_name: &str) {
        if !valid_file_name(new_name) {
            self.error(format!("Invalid folder name: {new_name}"));
            return;
        }
        let to = match from.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{}", new_name.trim()),
            None => new_name.trim().to_string(),
        };
        if to == from {
            return;
        }
        if !self.save_all() {
            return;
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
        match self.vault.rename_folder(from, &to) {
            Ok(()) => {
                for (a, b) in &moved {
                    self.index.rename_file(a, b);
                    self.retarget(a, b);
                }
                self.refresh_files();
                self.bump();
            }
            Err(e) => self.error(format!("Cannot rename folder: {e}")),
        }
    }

    pub fn delete(&mut self, target: &FsTarget) {
        let files: Vec<NotePath> = match target {
            FsTarget::File(p) => vec![p.clone()],
            FsTarget::Folder(f) => {
                let prefix = format!("{f}/");
                self.files
                    .iter()
                    .filter(|p| p.as_str().starts_with(&prefix))
                    .cloned()
                    .collect()
            }
        };
        for p in files {
            match self.vault.delete(&p) {
                Ok(()) => {
                    self.index.remove_file(&p);
                    self.close_tabs_of(&p);
                }
                Err(e) => self.error(format!("Cannot delete {p}: {e}")),
            }
        }
        self.refresh_files();
        self.bump();
    }

    fn reload_clean_buffer(&mut self, path: &NotePath) {
        let Some(b) = self.buffers.get(path) else {
            return;
        };
        if b.is_dirty() {
            return;
        }
        if let Ok(text) = self.vault.read_to_string(path)
            && let Some(b) = self.buffers.get_mut(path)
        {
            b.text = text.clone();
            b.saved = text;
        }
    }

    /// Rebuild the index from disk.
    pub fn reload_vault(&mut self) {
        self.save_all();
        self.index = (self.index_factory)(self.vault.as_read());
        let open: Vec<NotePath> = self.buffers.keys().cloned().collect();
        for p in open {
            self.reload_clean_buffer(&p);
            if let Some(b) = self.buffers.get(&p).filter(|b| b.is_dirty()) {
                let text = b.text.clone();
                self.index.update_from_source(&p, &text);
            }
        }
        self.refresh_files();
        self.bump();
        self.info("Vault reloaded");
    }

    // ----- derived data --------------------------------------------------

    /// Parsed metadata for a note: from the cache, falling back to parsing
    /// the buffer with `ors_markdown`.
    pub fn parsed(&self, path: &NotePath) -> ParsedNote {
        match self.index.get(path) {
            Some(n) => n.clone(),
            None => {
                let text = self.note_text(path).unwrap_or_default();
                self.index.parse(&text)
            }
        }
    }

    /// Metadata for `text`, which is either the current contents of `path`
    /// (cached) or a fragment of it (parsed on the fly).
    pub fn parsed_with_text(&self, path: &NotePath, text: &str) -> ParsedNote {
        let is_current = self.buffers.get(path).is_none_or(|b| b.text == text);
        match self.index.get(path) {
            Some(n) if is_current => n.clone(),
            _ => self.index.parse(text),
        }
    }

    pub fn note_text(&self, path: &NotePath) -> Option<String> {
        match self.buffers.get(path) {
            Some(b) => Some(b.text.clone()),
            None => self.vault.read_to_string(path).ok(),
        }
    }

    /// Sidebar/status-bar data for the active note, recomputed at most
    /// twice per second while typing.
    pub fn note_info(&mut self) -> &NoteInfo {
        let path = self.active_note().cloned();
        let key = (path.clone(), self.generation);
        let stale = self.info_key.as_ref() != Some(&key);
        let path_changed = self.info_key.as_ref().map(|k| &k.0) != Some(&path);
        if stale && (path_changed || self.now - self.info_time >= 0.5) {
            self.info = self.compute_info(path);
            self.info_key = Some(key);
            self.info_time = self.now;
        }
        &self.info
    }

    pub fn note_info_stale(&self) -> bool {
        self.info_key.as_ref().map(|k| k.1) != Some(self.generation)
    }

    fn compute_info(&self, path: Option<NotePath>) -> NoteInfo {
        let Some(path) = path else {
            return NoteInfo::default();
        };
        let parsed = self.parsed(&path);
        let backlinks = self.index.backlinks(&path);
        let backlink_count = backlinks.len();
        let mut grouped: Vec<(NotePath, Vec<(usize, String)>)> = Vec::new();
        let mut source_cache: HashMap<NotePath, Option<String>> = HashMap::new();
        for bl in backlinks {
            let text = source_cache
                .entry(bl.source.clone())
                .or_insert_with(|| self.note_text(&bl.source))
                .as_deref()
                .and_then(|t| t.lines().nth(bl.link.line))
                .unwrap_or("")
                .trim()
                .to_string();
            match grouped.last_mut() {
                Some((p, v)) if *p == bl.source => v.push((bl.link.line, text)),
                _ => grouped.push((bl.source.clone(), vec![(bl.link.line, text)])),
            }
        }
        let word_count = match self.index.get(&path) {
            Some(n) => n.word_count,
            None => self
                .buffers
                .get(&path)
                .map_or(0, |b| b.text.split_whitespace().count()),
        };
        NoteInfo {
            unlinked: self.index.unlinked_mentions(self.vault.as_read(), &path),
            outgoing: self.index.outgoing(&path),
            headings: parsed.headings,
            backlinks: grouped,
            backlink_count,
            word_count,
            path: Some(path),
        }
    }

    /// Re-run the search if the query or the index changed.
    pub fn refresh_search(&mut self) {
        let key = (self.search_query.clone(), self.generation);
        if self.search_key.as_ref() == Some(&key) {
            return;
        }
        self.search_results = if self.search_query.trim().is_empty() {
            Ok(Vec::new())
        } else {
            self.index
                .search(&self.search_query)
                .map_err(|e| e.to_string())
        };
        self.search_key = Some(key);
    }

    pub fn search_tag(&mut self, tag: &str) {
        self.search_query = format!("tag:#{tag}");
        self.settings.left_pane = LeftPane::Search;
        self.settings.left_open = true;
    }

    // ----- commands -----------------------------------------------------

    pub fn execute(&mut self, cmd: Command) {
        match cmd {
            Command::QuickSwitcher => {
                self.modal = Some(Modal::QuickSwitcher {
                    query: String::new(),
                    selected: 0,
                });
            }
            Command::CommandPalette => {
                self.modal = Some(Modal::CommandPalette {
                    query: String::new(),
                    selected: 0,
                });
            }
            Command::NewNote => self.new_note(),
            Command::NewFolder => {
                let parent = self
                    .active_note()
                    .and_then(NotePath::parent)
                    .unwrap_or("")
                    .to_string();
                self.modal = Some(Modal::NewFolder {
                    parent,
                    name: String::new(),
                });
            }
            Command::OpenDailyNote => self.open_daily_note(),
            Command::ToggleMode => self.toggle_mode(),
            Command::OpenGraph => self.open_graph(false),
            Command::OpenLocalGraph => self.open_graph(true),
            Command::SearchVault => {
                self.settings.left_open = true;
                self.settings.left_pane = LeftPane::Search;
                self.focus_search = true;
            }
            Command::ShowTags => {
                self.settings.left_open = true;
                self.settings.left_pane = LeftPane::Tags;
            }
            Command::ShowFiles => {
                self.settings.left_open = true;
                self.settings.left_pane = LeftPane::Files;
            }
            Command::RenameNote => {
                if let Some(p) = self.active_note().cloned() {
                    self.modal = Some(Modal::Rename {
                        name: p.stem().to_string(),
                        target: FsTarget::File(p),
                    });
                }
            }
            Command::DeleteNote => {
                if let Some(p) = self.active_note().cloned() {
                    self.modal = Some(Modal::ConfirmDelete {
                        target: FsTarget::File(p),
                    });
                }
            }
            Command::Save => {
                if let Some(p) = self.active_note().cloned()
                    && self.save(&p)
                {
                    self.info(format!("Saved {}", p.stem()));
                }
            }
            Command::CloseTab => {
                if let Some(i) = self.active {
                    self.close_tab(i);
                }
            }
            Command::NextTab => self.cycle_tab(1),
            Command::PrevTab => self.cycle_tab(-1),
            Command::NavigateBack => self.navigate(false),
            Command::NavigateForward => self.navigate(true),
            Command::ToggleLeftSidebar => self.settings.left_open = !self.settings.left_open,
            Command::ToggleRightSidebar => self.settings.right_open = !self.settings.right_open,
            Command::ReloadVault => self.reload_vault(),
            Command::ToggleTheme => {
                self.settings.theme = match self.settings.theme {
                    ThemeChoice::Dark => ThemeChoice::Light,
                    _ => ThemeChoice::Dark,
                };
            }
            Command::ToggleMonospace => {
                self.settings.monospace_editor = !self.settings.monospace_editor;
            }
            Command::SwitchVault => {
                self.save_all();
                self.switch_vault_requested = true;
            }
        }
    }

    /// Apply a confirmed modal action.
    pub fn confirm_modal(&mut self, modal: Modal) {
        match modal {
            Modal::Rename {
                target: FsTarget::File(p),
                name,
            } => self.rename_file(&p, &name),
            Modal::Rename {
                target: FsTarget::Folder(f),
                name,
            } => self.rename_folder(&f, &name),
            Modal::NewFolder { parent, name } => self.create_folder(&parent, &name),
            Modal::ConfirmDelete { target } => self.delete(&target),
            Modal::QuickSwitcher { .. } | Modal::CommandPalette { .. } => {}
        }
    }

    /// Called before the app exits.
    pub fn shutdown(&mut self) {
        self.save_all();
    }
}

impl dyn VaultBackend {
    pub fn as_read(&self) -> &dyn VaultRead {
        self
    }
    pub fn as_write(&self) -> &dyn ors_core::VaultWrite {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemVault;
    use crate::fake::FakeIndex;

    fn fake_factory(v: &dyn VaultRead) -> Box<dyn IndexBackend> {
        Box::new(FakeIndex::build(v))
    }

    fn p(s: &str) -> NotePath {
        NotePath::new(s).unwrap()
    }

    fn state_with(files: &[(&str, &str)]) -> AppState {
        let v = MemVault::with_files("test", files.iter().copied());
        AppState::new(Box::new(v), fake_factory, Settings::default())
    }

    fn sample() -> AppState {
        state_with(&[
            ("A.md", "# A\nlink to [[B]] and [[Missing]]"),
            ("B.md", "# B\nback to [[A]]"),
            ("dir/C.md", "c"),
        ])
    }

    fn disk(s: &AppState, path: &str) -> String {
        s.vault.read_to_string(&p(path)).unwrap()
    }

    #[test]
    fn open_creates_tab_and_buffer() {
        let mut s = sample();
        assert_eq!(s.files.len(), 3);
        s.open(&p("A.md"), OpenIn::CurrentTab);
        assert_eq!(s.tabs.len(), 1);
        assert_eq!(s.active, Some(0));
        assert!(s.buffers[&p("A.md")].text.starts_with("# A"));
        s.open(&p("B.md"), OpenIn::NewTab);
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(s.active_note(), Some(&p("B.md")));
    }

    #[test]
    fn history_back_forward() {
        let mut s = sample();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.open(&p("B.md"), OpenIn::CurrentTab);
        s.open(&p("dir/C.md"), OpenIn::CurrentTab);
        assert_eq!(s.tabs.len(), 1);
        s.execute(Command::NavigateBack);
        assert_eq!(s.active_note(), Some(&p("B.md")));
        s.execute(Command::NavigateBack);
        assert_eq!(s.active_note(), Some(&p("A.md")));
        s.execute(Command::NavigateBack); // nothing more
        assert_eq!(s.active_note(), Some(&p("A.md")));
        s.execute(Command::NavigateForward);
        assert_eq!(s.active_note(), Some(&p("B.md")));
        // Navigating somewhere new clears the forward stack.
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.execute(Command::NavigateForward);
        assert_eq!(s.active_note(), Some(&p("A.md")));
        // Buffers of notes no longer shown are released.
        assert_eq!(s.buffers.len(), 1);
    }

    #[test]
    fn close_and_reorder_tabs() {
        let mut s = sample();
        s.open(&p("A.md"), OpenIn::NewTab);
        s.open(&p("B.md"), OpenIn::NewTab);
        s.open(&p("dir/C.md"), OpenIn::NewTab);
        assert_eq!(s.active, Some(2));
        s.move_tab(2, 0);
        assert_eq!(s.active, Some(0));
        assert_eq!(s.tabs[0].title(), "C");
        s.set_active(1);
        s.close_tab(1);
        assert_eq!(s.active_note(), Some(&p("B.md")));
        assert!(!s.buffers.contains_key(&p("A.md")));
        s.execute(Command::CloseTab);
        s.execute(Command::CloseTab);
        assert!(s.tabs.is_empty());
        assert_eq!(s.active, None);
        s.execute(Command::CloseTab); // no panic
    }

    #[test]
    fn cycle_tabs() {
        let mut s = sample();
        s.open(&p("A.md"), OpenIn::NewTab);
        s.open(&p("B.md"), OpenIn::NewTab);
        s.execute(Command::NextTab);
        assert_eq!(s.active, Some(0));
        s.execute(Command::PrevTab);
        assert_eq!(s.active, Some(1));
    }

    #[test]
    fn autosave_is_debounced() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.tick(10.0);
        s.buffers.get_mut(&a).unwrap().text.push_str(" edit");
        s.buffer_edited(&a);
        s.tick(10.5);
        assert!(!disk(&s, "A.md").ends_with("edit"), "saved too early");
        assert!((s.next_autosave_in().unwrap() - 0.5).abs() < 1e-9);
        // Typing again postpones the save.
        s.buffers.get_mut(&a).unwrap().text.push('!');
        s.buffer_edited(&a);
        s.tick(11.2);
        assert!(!disk(&s, "A.md").ends_with('!'));
        s.tick(11.6);
        assert!(disk(&s, "A.md").ends_with("edit!"));
        assert!(!s.buffers[&a].is_dirty());
        assert!(s.next_autosave_in().is_none());
    }

    #[test]
    fn switching_and_closing_tabs_saves() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::NewTab);
        s.open(&p("B.md"), OpenIn::NewTab);
        s.set_active(0);
        s.buffers.get_mut(&a).unwrap().text = "changed".into();
        s.buffer_edited(&a);
        s.set_active(1);
        assert_eq!(disk(&s, "A.md"), "changed");
        s.set_active(0);
        s.buffers.get_mut(&a).unwrap().text = "changed again".into();
        s.buffer_edited(&a);
        s.close_tab(0);
        assert_eq!(disk(&s, "A.md"), "changed again");
    }

    #[test]
    fn external_change_reloads_clean_buffer() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.vault.as_write().write(&a, "external").unwrap();
        s.handle_vault_event(VaultEvent::Modified(a.clone()));
        assert_eq!(s.buffers[&a].text, "external");
        assert!(s.buffers[&a].conflict.is_none());
    }

    #[test]
    fn external_change_on_dirty_buffer_is_a_conflict() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.buffers.get_mut(&a).unwrap().text = "mine".into();
        s.buffer_edited(&a);
        s.vault.as_write().write(&a, "theirs").unwrap();
        s.handle_vault_event(VaultEvent::Modified(a.clone()));
        assert_eq!(s.buffers[&a].conflict.as_deref(), Some("theirs"));
        assert_eq!(s.buffers[&a].text, "mine");
        assert!(!s.toasts.is_empty());
        // Autosave must not clobber the disk while in conflict.
        s.tick(100.0);
        assert_eq!(disk(&s, "A.md"), "theirs");
        s.resolve_conflict(&a, true);
        assert_eq!(disk(&s, "A.md"), "mine");
        assert!(s.buffers[&a].conflict.is_none());
    }

    #[test]
    fn conflict_take_theirs() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.buffers.get_mut(&a).unwrap().text = "mine".into();
        s.buffer_edited(&a);
        s.vault.as_write().write(&a, "theirs").unwrap();
        s.handle_vault_event(VaultEvent::Modified(a.clone()));
        s.resolve_conflict(&a, false);
        assert_eq!(s.buffers[&a].text, "theirs");
        assert!(!s.buffers[&a].is_dirty());
    }

    #[test]
    fn own_save_echo_is_ignored() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.buffers.get_mut(&a).unwrap().text = "x".into();
        s.save(&a);
        s.handle_vault_event(VaultEvent::Modified(a.clone()));
        assert!(s.buffers[&a].conflict.is_none());
        assert_eq!(s.buffers[&a].text, "x");
    }

    #[test]
    fn deleted_on_disk_closes_clean_tabs() {
        let mut s = sample();
        let b = p("B.md");
        s.open(&b, OpenIn::CurrentTab);
        s.vault.as_write().delete(&b).unwrap();
        s.handle_vault_event(VaultEvent::Deleted(b.clone()));
        assert!(s.tabs.is_empty());
    }

    #[test]
    fn follow_unresolved_link_creates_note() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.follow_link("B", &a, OpenIn::CurrentTab);
        assert_eq!(s.active_note(), Some(&p("B.md")));
        s.follow_link("Missing", &a, OpenIn::NewTab);
        assert_eq!(s.active_note(), Some(&p("Missing.md")));
        assert!(s.vault.exists(&p("Missing.md")));
        assert_eq!(s.tabs.len(), 2);
    }

    #[test]
    fn new_note_honors_config_folder() {
        let mut v = MemVault::with_files("t", [("a.md", "")]);
        v.config.new_file_location = Some("folder".into());
        v.config.new_file_folder_path = Some("Inbox".into());
        let mut s = AppState::new(Box::new(v), fake_factory, Settings::default());
        s.execute(Command::NewNote);
        assert_eq!(s.active_note(), Some(&p("Inbox/Untitled.md")));
        s.execute(Command::NewNote);
        assert_eq!(s.active_note(), Some(&p("Inbox/Untitled 1.md")));
        assert!(s.folders.contains(&"Inbox".to_string()));
    }

    #[test]
    fn quick_switcher_create() {
        let mut s = sample();
        s.open_or_create("dir/New one", OpenIn::CurrentTab);
        assert_eq!(s.active_note(), Some(&p("dir/New one.md")));
        // Opening an existing name does not create a duplicate.
        s.open_or_create("dir/New one", OpenIn::CurrentTab);
        assert!(!s.files.contains(&p("dir/New one 1.md")));
        s.open_or_create("bad[name]", OpenIn::CurrentTab);
        assert!(!s.toasts.is_empty());
    }

    #[test]
    fn rename_updates_tabs_and_history() {
        let mut s = sample();
        let a = p("A.md");
        s.open(&a, OpenIn::CurrentTab);
        s.open(&p("B.md"), OpenIn::CurrentTab);
        s.navigate(false);
        s.execute(Command::RenameNote);
        let Some(Modal::Rename { name, .. }) = &s.modal else {
            panic!("rename modal not opened");
        };
        assert_eq!(name, "A");
        let modal = s.modal.take().unwrap();
        let Modal::Rename { target, .. } = modal else {
            unreachable!()
        };
        s.confirm_modal(Modal::Rename {
            target,
            name: "Alpha".into(),
        });
        assert_eq!(s.active_note(), Some(&p("Alpha.md")));
        assert!(s.vault.exists(&p("Alpha.md")));
        assert!(!s.files.contains(&a));
        assert!(s.buffers.contains_key(&p("Alpha.md")));
        // Renaming onto an existing file fails gracefully.
        s.rename_file(&p("Alpha.md"), "B");
        assert!(s.toasts.iter().any(|t| t.kind == ToastKind::Error));
        assert_eq!(s.active_note(), Some(&p("Alpha.md")));
    }

    #[test]
    fn rename_and_delete_folder() {
        let mut s = sample();
        s.open(&p("dir/C.md"), OpenIn::CurrentTab);
        s.rename_folder("dir", "renamed");
        assert_eq!(s.active_note(), Some(&p("renamed/C.md")));
        s.delete(&FsTarget::Folder("renamed".into()));
        assert!(s.tabs.is_empty());
        assert!(!s.files.iter().any(|f| f.as_str().starts_with("renamed/")));
    }

    #[test]
    fn delete_note_command() {
        let mut s = sample();
        s.open(&p("B.md"), OpenIn::CurrentTab);
        s.execute(Command::DeleteNote);
        let modal = s.modal.take().unwrap();
        s.confirm_modal(modal);
        assert!(!s.vault.exists(&p("B.md")));
        assert!(s.tabs.is_empty());
    }

    #[test]
    fn note_info_and_search() {
        let mut s = sample();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        let info = s.note_info().clone();
        assert_eq!(info.backlink_count, 1);
        assert_eq!(info.backlinks[0].0, p("B.md"));
        assert_eq!(info.backlinks[0].1[0].1, "back to [[A]]");
        assert_eq!(info.outgoing.len(), 2);
        assert_eq!(info.headings.len(), 1);
        s.search_query = "back".into();
        s.refresh_search();
        assert_eq!(s.search_results.as_ref().unwrap().len(), 1);
        s.search_tag("x");
        assert_eq!(s.search_query, "tag:#x");
    }

    #[test]
    fn graph_tab_builds_and_local_mode() {
        let mut s = sample();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.execute(Command::OpenLocalGraph);
        assert_eq!(s.tabs.len(), 2);
        let generation = s.generation;
        let index = &*s.index;
        let Some(TabContent::Graph(g)) = s.tabs.get_mut(1).map(|t| &mut t.content) else {
            panic!("no graph tab");
        };
        assert!(g.refresh(index, generation));
        assert!(!g.refresh(index, generation));
        g.depth = 2;
        assert!(g.refresh(index, generation));
        // Global graph tab is reused.
        s.execute(Command::OpenGraph);
        s.execute(Command::OpenGraph);
        assert_eq!(s.tabs.len(), 3);
        let generation = s.generation;
        let index = &*s.index;
        let Some(TabContent::Graph(g)) = s.tabs.get_mut(2).map(|t| &mut t.content) else {
            panic!("no graph tab");
        };
        assert!(!g.local);
        g.refresh(index, generation);
        assert!(g.data.node_index("A.md").is_some());
        assert!(g.layout.is_some());
    }

    #[test]
    fn modes_and_misc_commands() {
        let mut s = sample();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.execute(Command::ToggleMode);
        assert_eq!(
            s.active_tab().unwrap().note().unwrap().mode,
            ViewMode::Reading
        );
        s.execute(Command::ToggleMode);
        assert_eq!(
            s.active_tab().unwrap().note().unwrap().mode,
            ViewMode::Source
        );
        s.execute(Command::ToggleLeftSidebar);
        assert!(!s.settings.left_open);
        s.execute(Command::ToggleTheme);
        assert_eq!(s.settings.theme, ThemeChoice::Dark);
        s.execute(Command::QuickSwitcher);
        assert!(matches!(s.modal, Some(Modal::QuickSwitcher { .. })));
        s.execute(Command::OpenDailyNote);
        assert!(s.active_note().unwrap().as_str().starts_with("Journal/"));
        s.execute(Command::ReloadVault);
        assert!(s.files.iter().any(|f| f.as_str().starts_with("Journal/")));
    }

    #[test]
    fn poll_vault_picks_up_external_files() {
        let v = MemVault::with_files("t", [("a.md", "")]);
        v.external_write(&p("new.md"), "hello [[a]]").unwrap();
        let mut s = AppState::new(Box::new(v), fake_factory, Settings::default());
        let g0 = s.generation;
        s.poll_vault();
        assert!(s.generation > g0);
        assert!(s.files.contains(&p("new.md")));
        assert_eq!(s.index.backlinks(&p("a.md")).len(), 1);
    }
}
