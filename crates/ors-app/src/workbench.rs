//! Workspace-level state that lives next to [`AppState`]: tab groups,
//! bookmarks, core plugins, explorer state, settings window, workspace
//! dialogs, and the persistence of all of it in `.obsidian/`.

use ors_core::NotePath;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::backend::VaultBackend;
use crate::bookmarks::{BOOKMARKS_JSON, Bookmark, Bookmarks, now_millis};
use crate::commands::Command;
use crate::explorer::{ExplorerState, SortOrder};
use crate::plugins::{CORE_PLUGINS_JSON, CorePlugin, CorePlugins};
use crate::settings::{LeftPane, RightPane, Settings};
use crate::state::{AppState, FsTarget, GraphTab, NoteTab, OpenIn, Tab, TabContent, ViewMode};
use crate::workspace::{GroupId, Layout, Node, SplitDir};
use crate::ws_commands::{WsCommand, plugin_of};

/// Our own layout file (Obsidian's `workspace.json` is never touched).
pub const WORKSPACE_JSON: &str = "workspace-obsirust.json";
pub const APPEARANCE_JSON: &str = "appearance.json";

/// Small dialogs owned by the workspace area.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    MoveTo {
        targets: Vec<FsTarget>,
        query: String,
        selected: usize,
    },
    ConfirmDeleteMany {
        targets: Vec<FsTarget>,
    },
    TemplatePicker {
        query: String,
        selected: usize,
    },
    HeadingPicker {
        path: NotePath,
        query: String,
        selected: usize,
    },
    VaultSwitcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Editor,
    FilesLinks,
    Appearance,
    Hotkeys,
    CorePlugins,
    DailyNotes,
    Templates,
    About,
}

/// Settings window: page and the vault config being edited.
#[derive(Debug, Clone)]
pub struct SettingsWindow {
    pub page: SettingsPage,
    pub hotkey_filter: String,
    /// Command whose new hotkey is being captured.
    pub capture: Option<Command>,
    pub app: ors_vault::AppConfig,
    pub daily: ors_vault::DailyNotesConfig,
    pub templates: ors_vault::TemplatesConfig,
    /// `userIgnoreFilters`, one per line.
    pub excluded: String,
}

#[derive(Default)]
pub struct Workbench {
    pub layout: Layout,
    pub bookmarks: Bookmarks,
    pub plugins: CorePlugins,
    pub explorer: ExplorerState,
    pub dialog: Option<Dialog>,
    pub settings_window: Option<SettingsWindow>,
    /// Text to insert at the cursor of the active editor (UI applies it).
    pub pending_insert: Option<String>,
    /// Text to put on the clipboard (UI applies it).
    pub clipboard: Option<String>,
    /// Last saved workspace JSON (to save only on change).
    saved_layout: String,
    last_layout_check: f64,
}

impl Workbench {
    pub fn save_bookmarks_to(&self, vault: &dyn VaultBackend) {
        if let Err(e) = vault.write_config_file(BOOKMARKS_JSON, &self.bookmarks.to_json()) {
            eprintln!("cannot save bookmarks: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Saved layout (`workspace-obsirust.json`), shaped like Obsidian's
// `workspace.json`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SavedNode {
    Split {
        /// "vertical" = side by side (Obsidian's naming).
        direction: String,
        #[serde(default = "half")]
        ratio: f32,
        children: Vec<SavedNode>,
    },
    Tabs {
        #[serde(default, rename = "currentTab")]
        current_tab: usize,
        children: Vec<SavedLeaf>,
    },
}

fn half() -> f32 {
    0.5
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLeaf {
    #[serde(rename = "type", default = "leaf")]
    pub kind: String,
    pub state: SavedView,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

fn leaf() -> String {
    "leaf".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    /// "markdown", "file", "graph" or "localgraph".
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub state: SavedViewState,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SavedViewState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// "source" or "preview".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSidebar {
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub pane: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedExplorer {
    #[serde(default)]
    pub sort_order: SortOrder,
    #[serde(default)]
    pub expanded: Vec<String>,
    #[serde(default)]
    pub auto_reveal: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkspace {
    pub main: SavedNode,
    /// Index (tree order) of the focused tab group.
    #[serde(default)]
    pub active_group: usize,
    pub left: SavedSidebar,
    pub right: SavedSidebar,
    pub file_explorer: SavedExplorer,
    #[serde(default)]
    pub last_open_files: Vec<String>,
}

fn left_pane_name(p: LeftPane) -> &'static str {
    match p {
        LeftPane::Files => "file-explorer",
        LeftPane::Search => "search",
        LeftPane::Tags => "tag",
        LeftPane::Bookmarks => "bookmarks",
    }
}

fn left_pane_from(s: &str) -> LeftPane {
    match s {
        "search" => LeftPane::Search,
        "tag" => LeftPane::Tags,
        "bookmarks" => LeftPane::Bookmarks,
        _ => LeftPane::Files,
    }
}

fn right_pane_name(p: RightPane) -> &'static str {
    match p {
        RightPane::Backlinks => "backlink",
        RightPane::Outgoing => "outgoing-link",
        RightPane::Outline => "outline",
    }
}

fn right_pane_from(s: &str) -> RightPane {
    match s {
        "outgoing-link" => RightPane::Outgoing,
        "outline" => RightPane::Outline,
        _ => RightPane::Backlinks,
    }
}

fn save_tab(tab: &Tab, pinned: bool) -> SavedLeaf {
    let state = match &tab.content {
        TabContent::Note(n) => SavedView {
            kind: if n.path.is_markdown() {
                "markdown".into()
            } else {
                "file".into()
            },
            state: SavedViewState {
                file: Some(n.path.as_str().to_string()),
                mode: Some(
                    match n.mode {
                        ViewMode::Reading => "preview",
                        _ => "source",
                    }
                    .into(),
                ),
            },
        },
        TabContent::Graph(g) => SavedView {
            kind: if g.local {
                "localgraph".into()
            } else {
                "graph".into()
            },
            state: SavedViewState {
                file: g.center.as_ref().map(|c| c.as_str().to_string()),
                mode: None,
            },
        },
    };
    SavedLeaf {
        kind: leaf(),
        state,
        pinned,
    }
}

// ---------------------------------------------------------------------------
// AppState integration
// ---------------------------------------------------------------------------

impl AppState {
    /// Load bookmarks, core plugins, hotkeys and the vault's editor and
    /// appearance settings. Called once when the vault is opened.
    pub fn load_workbench(&mut self) {
        let v = &*self.vault;
        if let Some(s) = v.read_config_file(BOOKMARKS_JSON) {
            self.ws.bookmarks = Bookmarks::from_json(&s);
        }
        if let Some(s) = v.read_config_file(CORE_PLUGINS_JSON) {
            self.ws.plugins = CorePlugins::from_json(&s);
        }
        if let Some(s) = v.read_config_file(crate::hotkeys::HOTKEYS_JSON) {
            crate::hotkeys::apply(&s, &mut self.commands);
        }
        self.apply_plugins();
        let app = v.read_config_file("app.json");
        let appearance = v.read_config_file(APPEARANCE_JSON);
        apply_vault_settings(&mut self.settings, app.as_deref(), appearance.as_deref());
    }

    /// Hide commands of disabled core plugins.
    pub fn apply_plugins(&mut self) {
        let hidden: Vec<Command> = self
            .commands
            .specs()
            .iter()
            .map(|s| s.command)
            .filter(|c| plugin_of(*c).is_some_and(|p| !self.ws.plugins.is_enabled(p)))
            .collect();
        self.commands.set_hidden(hidden);
        if !self.ws.plugins.is_enabled(CorePlugin::Bookmarks)
            && self.settings.left_pane == LeftPane::Bookmarks
        {
            self.settings.left_pane = LeftPane::Files;
        }
        if !self.ws.plugins.is_enabled(CorePlugin::TagPane)
            && self.settings.left_pane == LeftPane::Tags
        {
            self.settings.left_pane = LeftPane::Files;
        }
    }

    pub fn plugin_enabled(&self, p: CorePlugin) -> bool {
        self.ws.plugins.is_enabled(p)
    }

    pub fn set_plugin(&mut self, p: CorePlugin, enabled: bool) {
        self.ws.plugins.set(p, enabled);
        if let Err(e) = self
            .vault
            .write_config_file(CORE_PLUGINS_JSON, &self.ws.plugins.to_json())
        {
            self.error(format!("Cannot save core plugins: {e}"));
        }
        self.apply_plugins();
    }

    pub fn save_hotkeys(&mut self) {
        let existing = self
            .vault
            .read_config_file(crate::hotkeys::HOTKEYS_JSON)
            .unwrap_or_default();
        let json = crate::hotkeys::to_json(&self.commands, &existing);
        if let Err(e) = self
            .vault
            .write_config_file(crate::hotkeys::HOTKEYS_JSON, &json)
        {
            self.error(format!("Cannot save hotkeys: {e}"));
        }
    }

    /// Write the editor/appearance settings to `app.json` and
    /// `appearance.json`, keeping all other keys.
    pub fn save_vault_settings(&mut self) {
        let s = &self.settings;
        let mut app = parse_map(self.vault.read_config_file("app.json").as_deref());
        app.insert("readableLineLength".into(), s.readable_line_length.into());
        app.insert("showLineNumber".into(), s.show_line_numbers.into());
        app.insert("spellcheck".into(), s.spellcheck.into());
        app.insert("livePreview".into(), s.live_preview.into());
        app.insert(
            "defaultViewMode".into(),
            if s.default_reading_view {
                "preview"
            } else {
                "source"
            }
            .into(),
        );
        let mut look = parse_map(self.vault.read_config_file(APPEARANCE_JSON).as_deref());
        look.insert("baseFontSize".into(), (s.font_size.round() as i64).into());
        look.insert(
            "theme".into(),
            match s.theme {
                crate::settings::ThemeChoice::Dark => "obsidian",
                crate::settings::ThemeChoice::Light => "moonstone",
                crate::settings::ThemeChoice::System => "system",
            }
            .into(),
        );
        match s.accent {
            Some([r, g, b]) => {
                look.insert("accentColor".into(), format!("#{r:02x}{g:02x}{b:02x}").into());
            }
            None => {
                look.insert("accentColor".into(), "".into());
            }
        }
        look.insert("interfaceFontFamily".into(), s.text_font.clone().into());
        for (name, map) in [("app.json", app), (APPEARANCE_JSON, look)] {
            let json = serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default();
            if let Err(e) = self.vault.write_config_file(name, &json) {
                self.error(format!("Cannot save {name}: {e}"));
            }
        }
    }

    // ----- layout persistence --------------------------------------------

    fn save_node(&self, node: &Node) -> SavedNode {
        match node {
            Node::Leaf(g) => {
                let group = self.ws.layout.group(*g);
                let ids = group.map(|g| g.tabs.clone()).unwrap_or_default();
                let children: Vec<SavedLeaf> = ids
                    .iter()
                    .filter_map(|id| self.tabs.iter().find(|t| t.id == *id))
                    .map(|t| save_tab(t, self.ws.layout.is_pinned(t.id)))
                    .collect();
                let current_tab = group
                    .and_then(|g| g.active)
                    .and_then(|a| ids.iter().position(|t| *t == a))
                    .unwrap_or(0);
                SavedNode::Tabs {
                    current_tab,
                    children,
                }
            }
            Node::Split {
                row,
                ratio,
                first,
                second,
            } => SavedNode::Split {
                direction: if *row { "vertical" } else { "horizontal" }.into(),
                ratio: *ratio,
                children: vec![self.save_node(first), self.save_node(second)],
            },
        }
    }

    pub fn saved_workspace(&self) -> SavedWorkspace {
        let s = &self.settings;
        let order = self.ws.layout.ordered_groups();
        SavedWorkspace {
            main: self.save_node(&self.ws.layout.root),
            active_group: order
                .iter()
                .position(|g| *g == self.ws.layout.focused)
                .unwrap_or(0),
            left: SavedSidebar {
                collapsed: !s.left_open,
                width: s.left_width,
                pane: left_pane_name(s.left_pane).into(),
            },
            right: SavedSidebar {
                collapsed: !s.right_open,
                width: s.right_width,
                pane: right_pane_name(s.right_pane).into(),
            },
            file_explorer: SavedExplorer {
                sort_order: self.ws.explorer.sort,
                expanded: self.ws.explorer.expanded.iter().cloned().collect(),
                auto_reveal: self.ws.explorer.auto_reveal,
            },
            last_open_files: self
                .recent_files
                .iter()
                .take(10)
                .map(|p| p.as_str().to_string())
                .collect(),
        }
    }

    /// Save the layout if it changed. Returns true if written.
    pub fn save_workspace(&mut self) -> bool {
        let json = serde_json::to_string_pretty(&self.saved_workspace()).unwrap_or_default();
        if json == self.ws.saved_layout {
            return false;
        }
        match self.vault.write_config_file(WORKSPACE_JSON, &json) {
            Ok(()) => {
                self.ws.saved_layout = json;
                true
            }
            Err(e) => {
                eprintln!("cannot save workspace: {e}");
                false
            }
        }
    }

    /// Called every frame; saves the layout at most every 2 seconds.
    pub fn maybe_save_workspace(&mut self) {
        if self.now - self.ws.last_layout_check >= 2.0 {
            self.ws.last_layout_check = self.now;
            self.save_workspace();
        }
    }

    /// Restore `workspace-obsirust.json` (open tabs, splits, sidebars).
    pub fn restore_workspace(&mut self) -> bool {
        let Some(json) = self.vault.read_config_file(WORKSPACE_JSON) else {
            return false;
        };
        let Ok(saved) = serde_json::from_str::<SavedWorkspace>(&json) else {
            return false;
        };
        self.apply_saved_workspace(&saved);
        self.ws.saved_layout = json;
        true
    }

    pub fn apply_saved_workspace(&mut self, saved: &SavedWorkspace) {
        let s = &mut self.settings;
        s.left_open = !saved.left.collapsed;
        s.right_open = !saved.right.collapsed;
        if saved.left.width > 50.0 {
            s.left_width = saved.left.width;
        }
        if saved.right.width > 50.0 {
            s.right_width = saved.right.width;
        }
        s.left_pane = left_pane_from(&saved.left.pane);
        s.right_pane = right_pane_from(&saved.right.pane);
        self.ws.explorer.sort = saved.file_explorer.sort_order;
        self.ws.explorer.expanded = saved.file_explorer.expanded.iter().cloned().collect();
        self.ws.explorer.auto_reveal = saved.file_explorer.auto_reveal;
        // Close what is open, then rebuild the tree.
        while !self.tabs.is_empty() {
            self.close_tab(0);
        }
        self.ws.layout = Layout::default();
        let root = self.ws.layout.focused;
        self.restore_node(&saved.main, root);
        set_ratios(&mut self.ws.layout.root, &saved.main);
        // Drop groups that ended up empty (missing files).
        let alive: Vec<_> = self.tabs.iter().map(|t| t.id).collect();
        self.ws.layout.retain_tabs(&alive);
        let order = self.ws.layout.ordered_groups();
        let focus = order
            .get(saved.active_group)
            .or(order.first())
            .copied()
            .unwrap_or(root);
        self.ws.layout.focus_group(focus);
        self.sync_active_from_layout();
        let recent: Vec<NotePath> = saved
            .last_open_files
            .iter()
            .filter_map(|p| NotePath::new(p).ok())
            .filter(|p| self.files.contains(p))
            .collect();
        for p in recent.into_iter().rev() {
            if !self.recent_files.contains(&p) {
                self.recent_files.push(p);
            }
        }
        self.apply_plugins();
    }

    fn restore_node(&mut self, node: &SavedNode, group: GroupId) {
        match node {
            SavedNode::Split {
                direction,
                children,
                ..
            } => {
                let dir = if direction == "horizontal" {
                    SplitDir::Down
                } else {
                    SplitDir::Right
                };
                match children.as_slice() {
                    [a] => self.restore_node(a, group),
                    [a, rest @ ..] => {
                        let new = self.ws.layout.split(group, dir);
                        self.restore_node(a, group);
                        // More than two children: nest the rest.
                        let tail = if rest.len() == 1 {
                            rest[0].clone()
                        } else {
                            SavedNode::Split {
                                direction: direction.clone(),
                                ratio: 0.5,
                                children: rest.to_vec(),
                            }
                        };
                        self.restore_node(&tail, new);
                    }
                    [] => {}
                }
            }
            SavedNode::Tabs {
                current_tab,
                children,
            } => {
                self.ws.layout.focus_group(group);
                let mut ids = Vec::new();
                for leaf in children {
                    if let Some(id) = self.restore_leaf(leaf) {
                        if leaf.pinned {
                            self.ws.layout.toggle_pin(id);
                        }
                        ids.push(id);
                    }
                }
                if let Some(id) = ids.get(*current_tab).or(ids.first()) {
                    self.ws.layout.activate(*id);
                }
            }
        }
    }

    fn restore_leaf(&mut self, leaf: &SavedLeaf) -> Option<crate::state::TabId> {
        let file = leaf
            .state
            .state
            .file
            .as_deref()
            .and_then(|f| NotePath::new(f).ok());
        let content = match leaf.state.kind.as_str() {
            "graph" => TabContent::Graph(Box::new(GraphTab::new(None))),
            "localgraph" => {
                let c = file.filter(|f| self.files.contains(f))?;
                TabContent::Graph(Box::new(GraphTab::new(Some(c))))
            }
            _ => {
                let path = file.filter(|f| self.files.contains(f))?;
                self.ensure_buffer(&path);
                TabContent::Note(NoteTab {
                    path,
                    mode: if leaf.state.state.mode.as_deref() == Some("preview") {
                        ViewMode::Reading
                    } else {
                        ViewMode::Source
                    },
                    back: Vec::new(),
                    forward: Vec::new(),
                    pending_line: None,
                })
            }
        };
        let i = self.push_tab(content);
        self.tabs.get(i).map(|t| t.id)
    }

    /// Point `active` at the focused group's active tab.
    pub fn sync_active_from_layout(&mut self) {
        self.active = self
            .ws
            .layout
            .focused_active()
            .and_then(|id| self.tabs.iter().position(|t| t.id == id));
    }

    pub fn tab_index(&self, id: crate::state::TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    // ----- tab groups ------------------------------------------------------

    /// Split the focused group, showing the current tab's view in the new
    /// group (like Obsidian's "Split right" / "Split down").
    pub fn split_active(&mut self, dir: SplitDir) {
        let focused = self.ws.layout.focused;
        let copy = self.active_tab().map(|t| match &t.content {
            TabContent::Note(n) => TabContent::Note(NoteTab {
                path: n.path.clone(),
                mode: n.mode,
                back: Vec::new(),
                forward: Vec::new(),
                pending_line: None,
            }),
            TabContent::Graph(g) => TabContent::Graph(Box::new(GraphTab::new(if g.local {
                g.center.clone()
            } else {
                None
            }))),
        });
        self.save_all();
        self.ws.layout.split(focused, dir);
        match copy {
            Some(c) => {
                self.push_tab(c);
            }
            None => self.active = None,
        }
    }

    /// Move tab `id` to `group` at `index` (drag & drop).
    pub fn move_tab_to_group(
        &mut self,
        id: crate::state::TabId,
        group: GroupId,
        index: Option<usize>,
    ) {
        self.ws.layout.move_tab(id, group, index);
        self.sync_active_from_layout();
    }

    /// Move tab `id` into a new split of `group` (drop on an edge).
    pub fn move_tab_to_split(&mut self, id: crate::state::TabId, group: GroupId, dir: SplitDir) {
        self.ws.layout.move_to_split(id, group, dir);
        self.sync_active_from_layout();
    }

    /// Open `path` in a new split of `group` (file dropped on an edge).
    pub fn open_in_split(&mut self, path: &NotePath, group: GroupId, dir: SplitDir) {
        self.ws.layout.focus_group(group);
        self.ws.layout.split(group, dir);
        self.open(path, OpenIn::NewTab);
    }

    pub fn open_in_group(&mut self, path: &NotePath, group: GroupId) {
        self.ws.layout.focus_group(group);
        self.sync_active_from_layout();
        self.open(path, OpenIn::NewTab);
    }

    pub fn focus_group(&mut self, group: GroupId) {
        if self.ws.layout.focused != group {
            self.save_all();
            self.ws.layout.focus_group(group);
            self.sync_active_from_layout();
        }
    }

    pub fn close_tabs(&mut self, ids: &[crate::state::TabId]) {
        for id in ids {
            self.close_tab_by_id(*id);
        }
    }

    pub fn close_others(&mut self, keep: crate::state::TabId) {
        let Some(g) = self.ws.layout.group_of(keep) else {
            return;
        };
        let ids: Vec<_> = self
            .ws
            .layout
            .group(g)
            .map(|g| g.tabs.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|t| *t != keep && !self.ws.layout.is_pinned(*t))
            .collect();
        self.close_tabs(&ids);
    }

    // ----- bookmarks -------------------------------------------------------

    pub fn add_bookmark(&mut self, b: Bookmark) {
        let label = b.label();
        if self.ws.bookmarks.add(b) {
            self.info(format!("Bookmarked {label}"));
        }
        self.ws.save_bookmarks_to(&*self.vault);
    }

    pub fn toggle_file_bookmark(&mut self, path: &NotePath) {
        if self.ws.bookmarks.has_file(path.as_str()) {
            self.ws.bookmarks.remove_file(path.as_str());
            self.ws.save_bookmarks_to(&*self.vault);
        } else {
            self.add_bookmark(Bookmark::file(path.as_str(), now_millis()));
        }
    }

    pub fn bookmarks_changed(&mut self) {
        self.ws.save_bookmarks_to(&*self.vault);
    }

    /// Open a bookmark (file, heading, folder, search).
    pub fn open_bookmark(&mut self, b: &Bookmark, open_in: OpenIn) {
        match b.kind.as_str() {
            "file" => {
                let Some(path) = b.path.as_deref().and_then(|p| NotePath::new(p).ok()) else {
                    return;
                };
                if !self.files.contains(&path) {
                    self.error(format!("{path} does not exist"));
                    return;
                }
                let line = b.subpath.as_deref().and_then(|s| {
                    let h = s.trim_start_matches('#');
                    self.parsed(&path)
                        .headings
                        .iter()
                        .find(|x| x.text == h)
                        .map(|x| x.line)
                });
                match line {
                    Some(l) => self.open_at_line(&path, l, open_in),
                    None => self.open(&path, open_in),
                }
            }
            "folder" => {
                if let Some(f) = &b.path {
                    let mut p = Some(f.as_str());
                    while let Some(x) = p {
                        self.ws.explorer.expanded.insert(x.to_string());
                        p = x.rfind('/').map(|i| &x[..i]);
                    }
                    self.ws.explorer.scroll_to = Some(FsTarget::Folder(f.clone()));
                    self.ws.explorer.selection = vec![FsTarget::Folder(f.clone())];
                    self.settings.left_pane = LeftPane::Files;
                    self.settings.left_open = true;
                }
            }
            "search" => {
                self.search_query = b.query.clone().unwrap_or_default();
                self.settings.left_pane = LeftPane::Search;
                self.settings.left_open = true;
            }
            _ => self.info("This bookmark type is not supported"),
        }
    }

    // ----- daily notes & templates -----------------------------------------

    /// Previous/next existing daily note relative to the active one (or
    /// today), like Obsidian's "Open previous/next daily note".
    pub fn adjacent_daily_note(&self, forward: bool) -> Option<NotePath> {
        let config = self.vault.daily_notes_config().ok()?;
        let today = crate::util::today();
        let path_of = |d: ors_vault::Date| ors_vault::daily_note_path(&config, d).ok();
        const SPAN: i64 = 3 * 366;
        // Date of the active note, if it is a daily note.
        let current = self.active_note().and_then(|active| {
            (0..=SPAN).find_map(|k| {
                [k, -k]
                    .into_iter()
                    .map(|o| today.add_days(o))
                    .find(|d| path_of(*d).as_ref() == Some(active))
            })
        });
        let start = current.unwrap_or(today);
        let step = if forward { 1 } else { -1 };
        (1..=SPAN)
            .map(|k| start.add_days(k * step))
            .filter_map(path_of)
            .find(|p| self.files.contains(p))
    }

    pub fn open_adjacent_daily(&mut self, forward: bool) {
        match self.adjacent_daily_note(forward) {
            Some(p) => self.open(&p, OpenIn::CurrentTab),
            None => self.info(if forward {
                "No next daily note"
            } else {
                "No previous daily note"
            }),
        }
    }

    /// Templates found in the configured templates folder.
    pub fn templates(&self) -> Vec<NotePath> {
        self.vault.list_templates().unwrap_or_default()
    }

    pub fn insert_template(&mut self, template: &NotePath) {
        let title = self
            .active_note()
            .map(|p| p.stem().to_string())
            .unwrap_or_default();
        match self.vault.render_template(template, &title) {
            Ok(text) => self.ws.pending_insert = Some(text),
            Err(e) => self.error(format!("Cannot read template: {e}")),
        }
    }

    fn insert_now(&mut self, time: bool) {
        let config = self.vault.templates_config().unwrap_or_default();
        let fmt = if time {
            config.time_format_or_default().to_string()
        } else {
            config.date_format_or_default().to_string()
        };
        let ctx = ors_vault::TemplateContext::now("");
        self.ws.pending_insert = Some(ors_vault::format_moment(&fmt, &ctx));
    }

    fn require_editor(&mut self) -> bool {
        let ok = self
            .active_tab()
            .and_then(Tab::note)
            .is_some_and(|n| n.path.is_markdown() && n.mode != ViewMode::Reading);
        if !ok {
            self.info("Open a note in editing view first");
        }
        ok
    }

    pub fn open_settings(&mut self, page: SettingsPage) {
        let app = self.vault.app_config().unwrap_or_default();
        self.ws.settings_window = Some(SettingsWindow {
            page,
            hotkey_filter: String::new(),
            capture: None,
            excluded: app.user_ignore_filters.join("\n"),
            app,
            daily: self.vault.daily_notes_config().unwrap_or_default(),
            templates: self.vault.templates_config().unwrap_or_default(),
        });
    }

    // ----- commands ----------------------------------------------------------

    pub fn exec_ws(&mut self, cmd: WsCommand) {
        use WsCommand::*;
        if let Some(p) = cmd.plugin()
            && !self.plugin_enabled(p)
        {
            return;
        }
        match cmd {
            SplitRight => self.split_active(SplitDir::Right),
            SplitDown => self.split_active(SplitDir::Down),
            TogglePin => {
                if let Some(id) = self.active_tab().map(|t| t.id) {
                    self.ws.layout.toggle_pin(id);
                }
            }
            CloseOthers => {
                if let Some(id) = self.active_tab().map(|t| t.id) {
                    self.close_others(id);
                }
            }
            CloseGroup => {
                let ids = self
                    .ws
                    .layout
                    .group(self.ws.layout.focused)
                    .map(|g| g.tabs.clone())
                    .unwrap_or_default();
                self.close_tabs(&ids);
            }
            FocusNextGroup => {
                self.save_all();
                self.ws.layout.cycle_group(1);
                self.sync_active_from_layout();
            }
            OpenSettings => self.open_settings(SettingsPage::Editor),
            VaultSwitcher => self.ws.dialog = Some(Dialog::VaultSwitcher),
            InsertTemplate => {
                if !self.require_editor() {
                    return;
                }
                if self
                    .vault
                    .templates_config()
                    .ok()
                    .and_then(|c| c.folder)
                    .is_none()
                {
                    self.error("Template folder not set. Configure it in Settings → Templates.");
                    self.open_settings(SettingsPage::Templates);
                    return;
                }
                self.ws.dialog = Some(Dialog::TemplatePicker {
                    query: String::new(),
                    selected: 0,
                });
            }
            InsertDate | InsertTime => {
                if self.require_editor() {
                    self.insert_now(cmd == InsertTime);
                }
            }
            PrevDailyNote => self.open_adjacent_daily(false),
            NextDailyNote => self.open_adjacent_daily(true),
            BookmarkFile => {
                if let Some(p) = self.active_note().cloned() {
                    self.toggle_file_bookmark(&p);
                }
            }
            BookmarkHeading => {
                if let Some(p) = self.active_note().cloned() {
                    self.ws.dialog = Some(Dialog::HeadingPicker {
                        path: p,
                        query: String::new(),
                        selected: 0,
                    });
                }
            }
            BookmarkSearch => {
                let q = self.search_query.trim().to_string();
                if q.is_empty() {
                    self.info("Search for something first");
                } else {
                    self.add_bookmark(Bookmark::search(&q, now_millis()));
                }
            }
            ShowBookmarks => {
                self.settings.left_open = true;
                self.settings.left_pane = LeftPane::Bookmarks;
            }
            RevealActiveFile => {
                if let Some(p) = self.active_note().cloned() {
                    self.ws.explorer.reveal(&p);
                    self.settings.left_open = true;
                    self.settings.left_pane = LeftPane::Files;
                }
            }
            MoveFile => {
                if let Some(p) = self.active_note().cloned() {
                    self.ws.dialog = Some(Dialog::MoveTo {
                        targets: vec![FsTarget::File(p)],
                        query: String::new(),
                        selected: 0,
                    });
                }
            }
            DuplicateFile => {
                if let Some(p) = self.active_note().cloned() {
                    self.duplicate_file(&p);
                }
            }
            CopyPath => {
                if let Some(p) = self.active_note() {
                    self.ws.clipboard = Some(p.as_str().to_string());
                }
            }
        }
    }
}

fn set_ratios(node: &mut Node, saved: &SavedNode) {
    if let (
        Node::Split {
            ratio,
            first,
            second,
            ..
        },
        SavedNode::Split {
            ratio: r, children, ..
        },
    ) = (node, saved)
    {
        *ratio = r.clamp(0.1, 0.9);
        if let Some(a) = children.first() {
            set_ratios(first, a);
        }
        if children.len() == 2 {
            set_ratios(second, &children[1]);
        }
    }
}

fn parse_map(s: Option<&str>) -> Map<String, Value> {
    match s.map(serde_json::from_str::<Value>) {
        Some(Ok(Value::Object(m))) => m,
        _ => Map::new(),
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    Some([b(0)?, b(2)?, b(4)?])
}

/// Read Obsidian's editor (`app.json`) and appearance (`appearance.json`)
/// keys into `settings`. Missing keys leave the current values.
pub fn apply_vault_settings(settings: &mut Settings, app: Option<&str>, appearance: Option<&str>) {
    let app = parse_map(app);
    let b = |k: &str| app.get(k).and_then(Value::as_bool);
    if let Some(v) = b("readableLineLength") {
        settings.readable_line_length = v;
    }
    if let Some(v) = b("showLineNumber") {
        settings.show_line_numbers = v;
    }
    if let Some(v) = b("spellcheck") {
        settings.spellcheck = v;
    }
    if let Some(v) = b("livePreview") {
        settings.live_preview = v;
    }
    if let Some(v) = app.get("defaultViewMode").and_then(Value::as_str) {
        settings.default_reading_view = v == "preview";
    }
    let look = parse_map(appearance);
    if let Some(v) = look.get("baseFontSize").and_then(Value::as_f64) {
        settings.font_size = (v as f32).clamp(10.0, 30.0);
    }
    if let Some(v) = look.get("accentColor").and_then(Value::as_str) {
        settings.accent = parse_hex(v);
    }
    if let Some(v) = look.get("theme").and_then(Value::as_str) {
        settings.theme = match v {
            "obsidian" => crate::settings::ThemeChoice::Dark,
            "moonstone" => crate::settings::ThemeChoice::Light,
            _ => crate::settings::ThemeChoice::System,
        };
    }
    if let Some(v) = look.get("interfaceFontFamily").and_then(Value::as_str)
        && matches!(v, "Inter" | "Default")
    {
        settings.text_font = v.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemVault;
    use crate::fake::FakeIndex;
    use crate::settings::ThemeChoice;

    fn state() -> AppState {
        let v = MemVault::with_files(
            "t",
            [
                ("A.md", "# A\n## Sub\ntext"),
                ("B.md", "b"),
                ("dir/C.md", "c"),
                ("Journal/2026-01-01.md", ""),
                ("Journal/2026-01-05.md", ""),
                ("Templates/T.md", "Hello {{title}}"),
            ],
        );
        AppState::new(
            Box::new(v),
            |v| Box::new(FakeIndex::build(v)),
            Settings::default(),
        )
    }

    fn p(s: &str) -> NotePath {
        NotePath::new(s).unwrap()
    }

    #[test]
    fn split_close_and_focus() {
        let mut s = state();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.exec_ws(WsCommand::SplitRight);
        assert_eq!(s.ws.layout.groups.len(), 2);
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(s.active_note(), Some(&p("A.md")));
        s.open(&p("B.md"), OpenIn::CurrentTab);
        // Navigating in the right pane leaves the left one alone.
        let left = s.ws.layout.ordered_groups()[0];
        let left_tab = s.ws.layout.group(left).unwrap().active.unwrap();
        let i = s.tab_index(left_tab).unwrap();
        assert_eq!(s.tabs[i].note().unwrap().path, p("A.md"));
        s.focus_group(left);
        assert_eq!(s.active_note(), Some(&p("A.md")));
        s.exec_ws(WsCommand::FocusNextGroup);
        assert_eq!(s.active_note(), Some(&p("B.md")));
        s.execute(Command::CloseTab);
        assert_eq!(s.ws.layout.groups.len(), 1);
        assert_eq!(s.active_note(), Some(&p("A.md")));
    }

    #[test]
    fn open_in_split_right_and_pinned_tabs() {
        let mut s = state();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.exec_ws(WsCommand::TogglePin);
        s.open(&p("B.md"), OpenIn::CurrentTab);
        // A pinned tab is not navigated away: B opens in a new tab.
        assert_eq!(s.tabs.len(), 2);
        s.open(&p("dir/C.md"), OpenIn::SplitRight);
        assert_eq!(s.ws.layout.groups.len(), 2);
        assert_eq!(s.active_note(), Some(&p("dir/C.md")));
        // Ctrl+W does not close a pinned tab.
        let a = s.tabs.iter().find(|t| t.title() == "A").unwrap().id;
        s.ws.layout.activate(a);
        s.sync_active_from_layout();
        s.execute(Command::CloseTab);
        assert!(s.tab_index(a).is_some());
    }

    #[test]
    fn workspace_round_trip() {
        let mut s = state();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.open(&p("B.md"), OpenIn::NewTab);
        s.toggle_mode();
        s.exec_ws(WsCommand::SplitDown);
        s.open(&p("dir/C.md"), OpenIn::CurrentTab);
        s.exec_ws(WsCommand::TogglePin);
        s.open_graph(false);
        s.settings.left_pane = LeftPane::Bookmarks;
        s.settings.left_width = 333.0;
        s.ws.explorer.sort = SortOrder::ModifiedNewest;
        s.ws.explorer.expanded.insert("dir".into());
        let saved = s.saved_workspace();
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("\"direction\":\"horizontal\""), "{json}");
        assert!(json.contains("\"sortOrder\":\"byModifiedTime\""), "{json}");
        assert!(s.save_workspace());
        assert!(!s.save_workspace(), "unchanged layout is not rewritten");

        let mut t = state();
        // Same vault config store: copy the saved file over.
        t.vault
            .write_config_file(
                WORKSPACE_JSON,
                &s.vault.read_config_file(WORKSPACE_JSON).unwrap(),
            )
            .unwrap();
        assert!(t.restore_workspace());
        assert_eq!(t.ws.layout.groups.len(), 2);
        assert_eq!(t.tabs.len(), 4);
        assert_eq!(t.settings.left_pane, LeftPane::Bookmarks);
        assert_eq!(t.settings.left_width, 333.0);
        assert_eq!(t.ws.explorer.sort, SortOrder::ModifiedNewest);
        assert!(t.active_tab().is_some_and(|x| x.note().is_none()), "graph focused");
        let b = t.tabs.iter().find(|x| x.title() == "B").unwrap();
        assert_eq!(b.note().unwrap().mode, ViewMode::Reading);
        assert_eq!(t.ws.layout.pinned.len(), 1);
        assert_eq!(t.saved_workspace(), saved);
    }

    #[test]
    fn restore_skips_missing_files() {
        let mut s = state();
        let saved: SavedWorkspace = serde_json::from_str(
            r#"{"main":{"type":"split","direction":"vertical","children":[
                {"type":"tabs","children":[{"state":{"type":"markdown","state":{"file":"gone.md"}}}]},
                {"type":"tabs","currentTab":1,"children":[
                    {"state":{"type":"markdown","state":{"file":"A.md"}}},
                    {"state":{"type":"markdown","state":{"file":"B.md","mode":"preview"}}}]}]},
              "left":{"collapsed":true,"width":200,"pane":"search"},
              "right":{"collapsed":false,"width":300,"pane":"outline"},
              "fileExplorer":{}}"#,
        )
        .unwrap();
        s.apply_saved_workspace(&saved);
        assert_eq!(s.ws.layout.groups.len(), 1);
        assert_eq!(s.active_note(), Some(&p("B.md")));
        assert!(!s.settings.left_open);
        assert_eq!(s.settings.right_pane, RightPane::Outline);
    }

    #[test]
    fn bookmarks_persist_and_follow_renames() {
        let mut s = state();
        s.open(&p("A.md"), OpenIn::CurrentTab);
        s.exec_ws(WsCommand::BookmarkFile);
        assert!(s.ws.bookmarks.has_file("A.md"));
        let json = s.vault.read_config_file(BOOKMARKS_JSON).unwrap();
        assert!(json.contains("\"type\": \"file\""), "{json}");
        s.rename_file(&p("A.md"), "Alpha");
        assert!(s.ws.bookmarks.has_file("Alpha.md"));
        s.search_query = "tag:#x".into();
        s.exec_ws(WsCommand::BookmarkSearch);
        let b = s.ws.bookmarks.items[1].clone();
        s.search_query.clear();
        s.open_bookmark(&b, OpenIn::CurrentTab);
        assert_eq!(s.search_query, "tag:#x");
        s.add_bookmark(Bookmark::heading("Alpha.md", "Sub", 1));
        let h = s.ws.bookmarks.items[2].clone();
        s.open_bookmark(&h, OpenIn::CurrentTab);
        let line = s.active_tab().and_then(Tab::note).and_then(|n| n.pending_line);
        assert_eq!(line, Some(1));
        s.exec_ws(WsCommand::BookmarkFile);
        assert!(!s.ws.bookmarks.has_file("Alpha.md"));
    }

    #[test]
    fn plugins_hide_commands() {
        let mut s = state();
        assert!(!s.commands.filter("daily").is_empty());
        s.set_plugin(CorePlugin::DailyNotes, false);
        assert!(s.commands.filter("daily notes").is_empty());
        assert!(s.commands.is_hidden(Command::OpenDailyNote));
        let mut t = state();
        t.vault
            .write_config_file(
                CORE_PLUGINS_JSON,
                &s.vault.read_config_file(CORE_PLUGINS_JSON).unwrap(),
            )
            .unwrap();
        t.load_workbench();
        assert!(!t.plugin_enabled(CorePlugin::DailyNotes));
        s.set_plugin(CorePlugin::Bookmarks, false);
        s.exec_ws(WsCommand::ShowBookmarks);
        assert_ne!(s.settings.left_pane, LeftPane::Bookmarks);
    }

    #[test]
    fn daily_navigation_and_templates() {
        let mut s = state();
        s.open(&p("Journal/2026-01-05.md"), OpenIn::CurrentTab);
        assert_eq!(
            s.adjacent_daily_note(false),
            Some(p("Journal/2026-01-01.md"))
        );
        s.exec_ws(WsCommand::PrevDailyNote);
        assert_eq!(s.active_note(), Some(&p("Journal/2026-01-01.md")));
        s.exec_ws(WsCommand::NextDailyNote);
        assert_eq!(s.active_note(), Some(&p("Journal/2026-01-05.md")));
        s.open(&p("A.md"), OpenIn::CurrentTab);
        // No templates folder yet: opens the settings.
        s.exec_ws(WsCommand::InsertTemplate);
        assert!(s.ws.settings_window.is_some());
        s.vault
            .save_templates_config(&ors_vault::TemplatesConfig {
                folder: Some("Templates".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.templates(), vec![p("Templates/T.md")]);
        s.exec_ws(WsCommand::InsertTemplate);
        assert!(matches!(s.ws.dialog, Some(Dialog::TemplatePicker { .. })));
        s.insert_template(&p("Templates/T.md"));
        assert_eq!(s.ws.pending_insert.as_deref(), Some("Hello A"));
        s.exec_ws(WsCommand::InsertDate);
        assert_eq!(s.ws.pending_insert.as_deref().map(str::len), Some(10));
    }

    #[test]
    fn vault_settings_round_trip() {
        let mut s = state();
        s.settings.show_line_numbers = true;
        s.settings.accent = Some([0x12, 0xab, 0xef]);
        s.settings.theme = ThemeChoice::Light;
        s.settings.default_reading_view = true;
        s.vault
            .write_config_file("app.json", r#"{"foo": 1, "trashOption": "local"}"#)
            .unwrap();
        s.save_vault_settings();
        let app = s.vault.read_config_file("app.json").unwrap();
        assert!(app.contains("\"foo\": 1") && app.contains("showLineNumber"), "{app}");
        let look = s.vault.read_config_file(APPEARANCE_JSON).unwrap();
        assert!(look.contains("#12abef") && look.contains("moonstone"), "{look}");
        let mut fresh = Settings::default();
        apply_vault_settings(&mut fresh, Some(&app), Some(&look));
        assert!(fresh.show_line_numbers && fresh.default_reading_view);
        assert_eq!(fresh.accent, Some([0x12, 0xab, 0xef]));
        assert_eq!(fresh.theme, ThemeChoice::Light);
        // New tabs follow the default view mode.
        s.settings.default_reading_view = true;
        s.open(&p("B.md"), OpenIn::NewTab);
        assert_eq!(s.active_tab().unwrap().note().unwrap().mode, ViewMode::Reading);
    }

    #[test]
    fn move_copy_and_bulk_ops() {
        let mut s = state();
        s.open(&p("B.md"), OpenIn::CurrentTab);
        assert!(s.move_file_to(&p("B.md"), "dir"));
        assert_eq!(s.active_note(), Some(&p("dir/B.md")));
        let c = s.duplicate_file(&p("dir/B.md")).unwrap();
        assert_eq!(c, p("dir/B 1.md"));
        assert!(s.move_folder_to("dir", "Journal"));
        assert!(s.files.contains(&p("Journal/dir/C.md")));
        assert!(!s.move_folder_to("Journal", "Journal/dir"));
        s.delete_targets(&[
            FsTarget::Folder("Journal/dir".into()),
            FsTarget::File(p("Journal/dir/C.md")),
            FsTarget::File(p("A.md")),
        ]);
        assert!(!s.files.contains(&p("A.md")));
        assert!(!s.files.iter().any(|f| f.as_str().starts_with("Journal/dir/")));
        assert!(s.rename_target(&FsTarget::File(p("Journal/2026-01-01.md")), "bad/name").is_err());
        assert!(s.rename_target(&FsTarget::File(p("Journal/2026-01-01.md")), "First").is_ok());
        assert!(s.files.contains(&p("Journal/First.md")));
    }
}
