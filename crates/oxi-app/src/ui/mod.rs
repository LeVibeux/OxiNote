//! eframe application: layout, input routing and the vault picker. All
//! state changes go through [`AppState`] (via [`Action`]s collected while
//! drawing, applied at the end of the frame).

mod dialogs;
mod edit;
mod edit_blocks;
mod edit_deco;
mod editor;
mod explorer_ui;
#[cfg(test)]
mod git_tests;
mod git_ui;
mod graph;
mod groups;
mod heading_nav;
mod icons;
mod modals;
mod panes;
mod pdf_view;
mod properties_view;
mod reading_view;
mod settings_ui;
mod tags_view;
#[cfg(test)]
mod tests;
pub mod theme;
mod view_panes;
mod ws_ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Frame, Margin, RichText, text::LayoutJob};
use oxi_core::{NotePath, VaultRead};

use crate::backend::{DiskVault, IndexBackend, RealIndex};
use crate::commands::Command;
use crate::editor::DisplayMode;
use crate::fake::{FakeIndex, demo_vault};
use crate::settings::{LeftPane, RecentVaults, RightPane, Settings, ThemeChoice};
use crate::state::{AppState, Modal, OpenIn, TabId, ToastKind, ViewMode};
use theme::Palette;

/// Something the user asked for while a frame was drawn.
#[derive(Debug, Clone)]
pub enum Action {
    Open(NotePath, OpenIn),
    OpenLine(NotePath, usize, OpenIn),
    Follow {
        target: String,
        from: NotePath,
        open_in: OpenIn,
    },
    OpenOrCreate(String),
    SearchTag(String),
    Exec(Command),
    Modal(Modal),
    Confirm(Modal),
    NewNoteIn(String),
    Resolve(NotePath, bool),
    Edited(NotePath),
    ShowBacklinks,
    /// Requests of the graph / search / backlinks / outline / reading views.
    View(crate::views::ViewAction),
    Error(String),
    Props(crate::properties_model::PropAction),
    /// Workspace area (explorer, bookmarks, tab groups).
    Ws(ws_ui::WsAction),
    /// Open a PDF at a page (1-based).
    OpenPdf(NotePath, u32, OpenIn),
    /// Switch the active note between Live Preview, Source and Reading.
    SetMode(crate::editor::DisplayMode),
    /// Git backup (explorer menu).
    Git(git_ui::GitAction),
}

fn apply(state: &mut AppState, action: Action) {
    match action {
        Action::View(a) => crate::views::apply(state, a),
        Action::Error(e) => state.error(e),
        Action::Open(p, how) => state.open(&p, how),
        Action::OpenLine(p, line, how) => state.open_at_line(&p, line, how),
        Action::Follow {
            target,
            from,
            open_in,
        } => state.follow_link(&target, &from, open_in),
        Action::OpenOrCreate(name) => state.open_or_create(&name, OpenIn::CurrentTab),
        Action::SearchTag(t) => state.search_tag(&t),
        Action::Exec(c) => state.execute(c),
        Action::Modal(m) => state.modal = Some(m),
        Action::Confirm(m) => state.confirm_modal(m),
        Action::NewNoteIn(folder) => {
            state.create_note_in(&folder, "Untitled", OpenIn::NewTab);
        }
        Action::Resolve(p, keep) => state.resolve_conflict(&p, keep),
        Action::Edited(p) => state.buffer_edited(&p),
        Action::Props(a) => state.apply_prop_action(a),
        Action::Ws(a) => ws_ui::apply(state, a),
        Action::OpenPdf(p, page, how) => state.open_pdf_at(&p, page, how),
        Action::SetMode(m) => state.set_display_mode(m),
        Action::Git(a) => git_ui::apply(state, a),
        Action::ShowBacklinks => {
            state.settings.right_open = true;
            state.settings.right_pane = RightPane::Backlinks;
        }
    }
}

/// Per-frame caches that only matter for drawing.
#[derive(Default)]
pub struct UiCaches {
    images: HashMap<NotePath, Option<egui::load::Bytes>>,
    highlight: HashMap<TabId, (u64, LayoutJob)>,
    reading: HashMap<NotePath, reading_view::Rendered>,
    decor: HashMap<TabId, (u64, Rc<crate::editor::live::Decor>)>,
    completion: Option<edit::CompletionUi>,
    find: Option<edit::FindUi>,
    explorer: explorer_ui::ExplorerCache,
    embeds: edit_deco::EmbedCache,
    pdf: pdf_view::PdfCaches,
    nav: heading_nav::NavCache,
}

impl UiCaches {
    fn image_bytes(&mut self, state: &AppState, path: &NotePath) -> Option<egui::load::Bytes> {
        self.images
            .entry(path.clone())
            .or_insert_with(|| {
                state
                    .vault
                    .read_bytes(path)
                    .ok()
                    .map(egui::load::Bytes::from)
            })
            .clone()
    }
}

/// Command-line options, including hidden debugging flags used to capture
/// screenshots headlessly.
#[derive(Debug, Clone, Default)]
pub struct Startup {
    pub vault: Option<PathBuf>,
    pub demo: bool,
    pub screenshot: Option<PathBuf>,
    pub frames: u64,
    pub open: Vec<String>,
    pub reading: bool,
    /// Open notes in Source mode (`--source`, screenshots).
    pub source: bool,
    /// Heading navigator design (`--nav off|a|b`) and forced hover position
    /// (`--nav-hover FRACTION`), for screenshots.
    pub nav: Option<crate::settings::HeadingNavigator>,
    pub nav_hover: Option<f32>,
    pub graph: bool,
    /// Open the graph settings panel (screenshots).
    pub graph_panel: bool,
    pub palette: bool,
    pub switcher: Option<String>,
    pub search: Option<String>,
    pub theme: Option<ThemeChoice>,
    pub left: Option<LeftPane>,
    pub right: Option<RightPane>,
    /// Ignore persisted settings.
    pub fresh: bool,
    /// Open the settings window (`--settings [page]`).
    pub settings: Option<String>,
    /// Split the workspace and open the next notes in the right group.
    pub split: bool,
    pub split_down: bool,
    /// Fold every heading and list on opening (`--fold-all`, screenshots).
    pub fold_all: bool,
}

pub struct OxinoteApp {
    state: Option<AppState>,
    settings: Settings,
    recent: RecentVaults,
    recent_file: Option<PathBuf>,
    picker_input: String,
    picker_error: Option<String>,
    caches: UiCaches,
    startup: Startup,
    frame: u64,
    shot_done: Arc<AtomicBool>,
    shot_requested: bool,
    theme_applied: Option<ThemeChoice>,
    look_applied: Option<(Option<[u8; 3]>, u32, bool)>,
    start: Instant,
    title: String,
}

pub(crate) fn real_index(vault: &dyn VaultRead) -> Box<dyn IndexBackend> {
    match RealIndex::build(vault) {
        Ok(i) => Box::new(i),
        Err(e) => {
            eprintln!("cannot index vault: {e}");
            Box::new(FakeIndex::default())
        }
    }
}

pub(crate) fn fake_index(vault: &dyn VaultRead) -> Box<dyn IndexBackend> {
    Box::new(FakeIndex::build(vault))
}

impl OxinoteApp {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::install(&cc.egui_ctx);
        let mut settings: Settings = if startup.fresh {
            Settings::default()
        } else {
            cc.storage
                .and_then(|s| eframe::get_value(s, "settings"))
                .unwrap_or_default()
        };
        if let Some(t) = startup.theme {
            settings.theme = t;
        }
        let recent_file = RecentVaults::default_file();
        let recent = recent_file
            .as_deref()
            .map(RecentVaults::load)
            .unwrap_or_default();
        let mut app = Self {
            state: None,
            settings,
            recent,
            recent_file,
            picker_input: String::new(),
            picker_error: None,
            caches: UiCaches::default(),
            startup: startup.clone(),
            frame: 0,
            shot_done: Arc::new(AtomicBool::new(false)),
            shot_requested: false,
            theme_applied: None,
            look_applied: None,
            start: Instant::now(),
            title: String::new(),
        };
        if startup.demo {
            app.open_demo();
        } else if let Some(v) = &startup.vault {
            app.open_vault(v.clone());
        }
        app.apply_startup();
        app
    }

    fn open_demo(&mut self) {
        let state = AppState::new(Box::new(demo_vault()), fake_index, self.settings.clone());
        self.state = Some(state);
        self.caches = UiCaches::default();
    }

    fn open_vault(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        match DiskVault::open(&path) {
            Ok(v) => {
                let mut state = AppState::new(Box::new(v), real_index, self.settings.clone());
                state.updates.enabled = self.startup.screenshot.is_none();
                self.state = Some(state);
                self.caches = UiCaches::default();
                self.picker_error = None;
                self.recent.push(path);
                if let Some(f) = &self.recent_file
                    && let Err(e) = self.recent.save(f)
                {
                    eprintln!("cannot save recent vaults: {e}");
                }
            }
            Err(e) => {
                self.picker_error = Some(format!("Cannot open {}: {e}", path.display()));
            }
        }
    }

    /// Apply debugging flags (`--open`, `--reading`, ...).
    fn apply_startup(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let s = &self.startup;
        state.ws.read_only = s.screenshot.is_some();
        if s.open.is_empty() && !s.fresh && !state.ws.read_only {
            state.restore_workspace();
        }
        for (i, name) in s.open.iter().enumerate() {
            let how = if i == 0 {
                OpenIn::CurrentTab
            } else {
                OpenIn::NewTab
            };
            match NotePath::new(name) {
                Ok(p) if state.files.contains(&p) => state.open(&p, how),
                _ => match state.index.fuzzy_find(name, 1).into_iter().next() {
                    Some((p, _, _)) => state.open(&p, how),
                    None => state.error(format!("No such note: {name}")),
                },
            }
            if s.reading {
                state.toggle_mode();
            }
            if s.source {
                state.set_display_mode(DisplayMode::Source);
            }
            if s.fold_all {
                state.editor.pending.push(Command::FoldAll);
            }
        }
        if let Some(p) = state.active_note().cloned() {
            state.ws.explorer.reveal(&p);
        }
        if let Some(m) = s.nav {
            state.settings.heading_navigator = m;
        }
        self.caches.nav.force = s.nav_hover;
        if let Some(l) = s.left {
            state.settings.left_pane = l;
        }
        if let Some(r) = s.right {
            state.settings.right_pane = r;
        }
        if let Some(q) = &s.search {
            state.search_query = q.clone();
            state.settings.left_pane = LeftPane::Search;
        }
        if s.graph_panel {
            state.views.graph.override_unsaved(|g| {
                g.close = false;
                g.collapse_filter = false;
                g.collapse_color_groups = false;
                g.collapse_display = false;
                g.collapse_forces = false;
            });
        }
        if s.graph {
            state.open_graph(false);
        }
        if s.split_down {
            state.exec_ws(crate::ws_commands::WsCommand::SplitDown);
        }
        if s.split {
            state.exec_ws(crate::ws_commands::WsCommand::SplitRight);
        }
        if let Some(page) = &s.settings {
            use crate::workbench::SettingsPage as P;
            let page = match page.as_str() {
                "files" => P::FilesLinks,
                "appearance" => P::Appearance,
                "hotkeys" => P::Hotkeys,
                "plugins" => P::CorePlugins,
                "daily" => P::DailyNotes,
                "templates" => P::Templates,
                "git" => P::GitBackup,
                "git-ignore" => {
                    state.git.scroll_gitignore = true;
                    P::GitBackup
                }
                "about" => P::About,
                _ => P::Editor,
            };
            state.open_settings(page);
        }
        if s.palette {
            state.modal = Some(Modal::CommandPalette {
                query: String::new(),
                selected: 0,
            });
        }
        if let Some(q) = &s.switcher {
            state.modal = Some(Modal::QuickSwitcher {
                query: q.clone(),
                selected: 0,
            });
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.startup.screenshot.clone() else {
            return;
        };
        if self.shot_done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
        if self.frame >= self.startup.frames.max(2) && !self.shot_requested {
            self.shot_requested = true;
            let done = self.shot_done.clone();
            let ctx2 = ctx.clone();
            ctx.request_screenshot(move |image| {
                let [w, h] = image.size;
                let mut buf = Vec::with_capacity(w * h * 4);
                for p in &image.pixels {
                    buf.extend_from_slice(&p.to_array());
                }
                match image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                    Some(img) => match img.save(&path) {
                        Ok(()) => eprintln!("screenshot saved to {}", path.display()),
                        Err(e) => eprintln!("cannot save screenshot: {e}"),
                    },
                    None => eprintln!("invalid screenshot buffer"),
                }
                done.store(true, Ordering::SeqCst);
                ctx2.request_repaint();
            });
        }
    }

    fn picker(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let mut open: Option<PathBuf> = None;
        let mut remove: Option<PathBuf> = None;
        let mut demo = false;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.bg_primary))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.12);
                    ui.label(RichText::new("◆").size(44.0).color(pal.accent));
                    ui.label(RichText::new("Oxinote").size(30.0).color(pal.strong));
                    ui.label(RichText::new("A Rust reimplementation of Obsidian").color(pal.muted));
                    ui.add_space(24.0);
                    ui.set_max_width(460.0);
                    Frame::new()
                        .fill(pal.bg_secondary)
                        .corner_radius(8)
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(440.0);
                            ui.label(RichText::new("Open a vault").strong().color(pal.strong));
                            ui.add_space(6.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("📂  Open folder as vault")
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(pal.accent),
                                )
                                .clicked()
                                && let Some(dir) = rfd::FileDialog::new()
                                    .set_title("Open folder as vault")
                                    .pick_folder()
                            {
                                open = Some(dir);
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let te = ui.add(
                                    egui::TextEdit::singleline(&mut self.picker_input)
                                        .hint_text("/path/to/vault")
                                        .desired_width(330.0),
                                );
                                let enter = te.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if (ui.button("Open").clicked() || enter)
                                    && !self.picker_input.trim().is_empty()
                                {
                                    open = Some(PathBuf::from(self.picker_input.trim()));
                                }
                            });
                            if ui.link("Try the demo vault").clicked() {
                                demo = true;
                            }
                            if let Some(e) = &self.picker_error {
                                ui.colored_label(egui::Color32::from_rgb(0xe9, 0x31, 0x47), e);
                            }
                            if !self.recent.vaults.is_empty() {
                                ui.add_space(10.0);
                                ui.separator();
                                ui.label(RichText::new("Recent vaults").small().color(pal.muted));
                                for v in &self.recent.vaults {
                                    ui.horizontal(|ui| {
                                        let name = v
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| v.display().to_string());
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(name).color(pal.strong),
                                                )
                                                .frame(false),
                                            )
                                            .on_hover_text(v.display().to_string())
                                            .clicked()
                                        {
                                            open = Some(v.clone());
                                        }
                                        ui.label(
                                            RichText::new(v.display().to_string())
                                                .small()
                                                .color(pal.faint),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .small_button("×")
                                                    .on_hover_text("Remove from list")
                                                    .clicked()
                                                {
                                                    remove = Some(v.clone());
                                                }
                                            },
                                        );
                                    });
                                }
                            }
                        });
                });
            });
        if let Some(r) = remove {
            self.recent.remove(&r);
            if let Some(f) = &self.recent_file {
                let _ = self.recent.save(f);
            }
        }
        if demo {
            self.open_demo();
        } else if let Some(p) = open {
            self.open_vault(p);
        }
    }
}

fn ribbon(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    let button = |ui: &mut egui::Ui, icon: &str, cmd: Command, actions: &mut Vec<Action>| {
        let name = state.commands.get(cmd).map_or("", |s| s.name);
        let tip = match state.commands.shortcut(cmd) {
            Some(k) => format!("{name} ({})", ui.ctx().format_shortcut(&k)),
            None => name.to_string(),
        };
        if explorer_ui::icon_button(ui, icon, &tip, false).clicked() {
            actions.push(Action::Exec(cmd));
        }
    };
    ui.vertical_centered(|ui| {
        let items: [(&str, Command); 7] = [
            (icons::SIDEBAR, Command::ToggleLeftSidebar),
            (icons::SEARCH, Command::QuickSwitcher),
            (icons::GRAPH, Command::OpenGraph),
            (icons::CALENDAR, Command::OpenDailyNote),
            (icons::NOTE_PENCIL, Command::NewNote),
            (icons::TERMINAL, Command::CommandPalette),
            (icons::CIRCLE_HALF, Command::ToggleTheme),
        ];
        for (icon, cmd) in items {
            if state.commands.is_hidden(cmd) {
                continue;
            }
            button(ui, icon, cmd, actions);
            ui.add_space(2.0);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            button(
                ui,
                icons::GEAR,
                Command::Ws(crate::ws_commands::WsCommand::OpenSettings),
                actions,
            );
            ui.add_space(2.0);
            button(ui, icons::VAULT, Command::SwitchVault, actions);
        });
    });
}

fn view_header(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let Some(note) = state.active_tab().and_then(|t| t.note()) else {
        return;
    };
    ui.horizontal(|ui| {
        let back = ui.add_enabled(!note.back.is_empty(), egui::Button::new("←").frame(false));
        if back.on_hover_text("Navigate back (Alt+←)").clicked() {
            actions.push(Action::Exec(Command::NavigateBack));
        }
        let fwd = ui.add_enabled(
            !note.forward.is_empty(),
            egui::Button::new("→").frame(false),
        );
        if fwd.on_hover_text("Navigate forward (Alt+→)").clicked() {
            actions.push(Action::Exec(Command::NavigateForward));
        }
        ui.add_space(8.0);
        let crumbs = note.path.without_extension().replace('/', "  /  ");
        ui.label(RichText::new(crumbs).color(pal.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if note.path.is_markdown() {
                let mode = state.display_mode().unwrap_or(DisplayMode::LivePreview);
                let more = ui.add(
                    egui::Button::new(icons::text(icons::DOTS_THREE, 16.0).color(pal.muted))
                        .frame(false),
                );
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(170.0);
                    if ui
                        .selectable_label(mode == DisplayMode::Source, "Source mode")
                        .clicked()
                    {
                        let to = if mode == DisplayMode::Source {
                            DisplayMode::LivePreview
                        } else {
                            DisplayMode::Source
                        };
                        actions.push(Action::SetMode(to));
                        ui.close();
                    }
                    if ui
                        .selectable_label(mode == DisplayMode::Reading, "Reading view")
                        .clicked()
                    {
                        let to = if mode == DisplayMode::Reading {
                            DisplayMode::LivePreview
                        } else {
                            DisplayMode::Reading
                        };
                        actions.push(Action::SetMode(to));
                        ui.close();
                    }
                });
                more.on_hover_text("More options");
                let (icon, tip) = match mode {
                    DisplayMode::Reading => (icons::PENCIL, "Current view: reading. Edit (Ctrl+E)"),
                    _ => (icons::BOOK_OPEN, "Current view: editing. Read (Ctrl+E)"),
                };
                if ui
                    .add(egui::Button::new(icons::text(icon, 16.0).color(pal.muted)).frame(false))
                    .on_hover_text(tip)
                    .clicked()
                {
                    actions.push(Action::Exec(Command::ToggleMode));
                }
            }
        });
    });
}

/// View of the active tab of the group being drawn (`state.active`).
fn leaf_content(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let Some(i) = state.active else {
        ui.add_space(ui.available_height() * 0.3);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No file is open").size(20.0).color(pal.muted));
            ui.add_space(8.0);
            for (label, cmd) in [
                ("Create new note", Command::NewNote),
                ("Go to file", Command::QuickSwitcher),
                ("Open graph view", Command::OpenGraph),
                ("Open today's daily note", Command::OpenDailyNote),
            ] {
                let keys = state
                    .commands
                    .shortcut(cmd)
                    .map(|k| format!("  ({})", ui.ctx().format_shortcut(&k)))
                    .unwrap_or_default();
                if ui
                    .link(RichText::new(format!("{label}{keys}")).color(pal.accent))
                    .clicked()
                {
                    actions.push(Action::Exec(cmd));
                }
            }
        });
        return;
    };
    let (tab_id, note) = match state.tabs.get(i) {
        Some(t) => (t.id, t.note().map(|n| (n.path.clone(), n.mode))),
        None => return,
    };
    match note {
        None => {
            ui.push_id(("graph-tab", tab_id), |ui| {
                graph::graph_view(ui, state, i, actions)
            });
        }
        Some((path, mode)) => {
            Frame::new()
                .inner_margin(Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    view_header(ui, state, actions);
                });
            if state
                .buffers
                .get(&path)
                .is_some_and(|b| b.conflict.is_some())
            {
                editor::conflict_banner(ui, &path, actions);
            }
            if crate::pdf::is_pdf(&path) {
                pdf_view::pdf_tab(ui, state, caches, tab_id, &path, actions);
                return;
            }
            let scroll_to = if mode == ViewMode::Reading {
                crate::views::take_pending_line(state)
            } else {
                None
            };
            if path.is_markdown() {
                let reading = mode == ViewMode::Reading;
                edit::find_bar(ui, state, caches, tab_id, &path, reading, actions);
            }
            let heads = (path.is_markdown()
                && state.settings.heading_navigator != crate::settings::HeadingNavigator::Off)
                .then(|| caches.nav.headings(tab_id, state, &path));
            caches.nav.ys = None;
            editor::set_nav_reserve(heads.as_ref().is_some_and(|h| !h.is_empty()));
            let scroll_out = egui::ScrollArea::vertical()
                .id_salt(("note-scroll", tab_id, mode == ViewMode::Reading))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if !path.is_markdown() {
                        editor::file_view(ui, state, caches, &path);
                    } else if mode == ViewMode::Reading {
                        reading_view::reading_view(ui, state, caches, &path, scroll_to, actions);
                    } else {
                        if crate::properties_model::panel_visible(&state.settings)
                            && state.display_mode() != Some(DisplayMode::Source)
                        {
                            editor::readable_column(
                                ui,
                                state.settings.readable_line_length,
                                |ui| {
                                    let scope = format!("doc{tab_id}");
                                    properties_view::panel(ui, state, &path, &scope, true, actions);
                                },
                            );
                        }
                        edit::edit_view(ui, state, caches, tab_id, &path, actions);
                    }
                });
            if let Some(heads) = heads {
                let dm = if mode == ViewMode::Reading {
                    DisplayMode::Reading
                } else {
                    DisplayMode::LivePreview
                };
                heading_nav::show(
                    ui,
                    state,
                    caches,
                    tab_id,
                    i,
                    dm,
                    scroll_out.inner_rect,
                    scroll_out.state.clamped_offset().y,
                    &heads,
                );
            }
        }
    }
}

fn toasts(ctx: &egui::Context, state: &AppState) {
    if state.toasts.is_empty() {
        return;
    }
    let pal = Palette::of(ctx);
    egui::Area::new(egui::Id::unique("toasts"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -36.0])
        .interactable(false)
        .show(ctx, |ui| {
            for t in state.toasts.iter().rev().take(4) {
                let color = match t.kind {
                    ToastKind::Info => pal.accent,
                    ToastKind::Error => egui::Color32::from_rgb(0xe9, 0x31, 0x47),
                };
                Frame::new()
                    .fill(pal.bg_secondary)
                    .stroke(egui::Stroke::new(1.0, color))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(RichText::new(&t.text).color(pal.text));
                    });
                ui.add_space(6.0);
            }
        });
}

impl OxinoteApp {
    fn workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let caches = &mut self.caches;
        state.tick(self.start.elapsed().as_secs_f64());
        state.poll_vault();
        caches.pdf.gc(ctx.input(|i| i.time));
        let mut actions: Vec<Action> = Vec::new();

        // Shortcuts (shared with the palette through the registry).
        let capturing = state
            .ws
            .settings_window
            .as_ref()
            .is_some_and(|w| w.capture.is_some());
        if !capturing {
            for (shortcut, cmd) in state.commands.by_specificity() {
                if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                    actions.push(Action::Exec(cmd));
                }
            }
        }
        let pal = Palette::of(&ctx);

        if state.settings.show_ribbon {
            egui::Panel::left("ribbon")
                .exact_size(40.0)
                .resizable(false)
                .frame(Frame::new().fill(pal.bg_secondary))
                .show(ui, |ui| ribbon(ui, state, &mut actions));
        }
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(
                Frame::new()
                    .fill(pal.bg_secondary)
                    .inner_margin(Margin::symmetric(12, 2)),
            )
            .show(ui, |ui| panes::status_bar(ui, state, &mut actions));
        if state.settings.left_open {
            let w = state.settings.left_width;
            let r = egui::Panel::left("left-sidebar")
                .default_size(w)
                .size_range(160.0..=500.0)
                .resizable(true)
                .frame(
                    Frame::new()
                        .fill(pal.bg_secondary)
                        .inner_margin(Margin::symmetric(8, 4)),
                )
                .show(ui, |ui| {
                    panes::left_sidebar(ui, state, caches, &mut actions)
                });
            state.settings.left_width = r.response.rect.width();
        }
        if state.settings.right_open {
            let w = state.settings.right_width;
            let r = egui::Panel::right("right-sidebar")
                .default_size(w)
                .size_range(160.0..=500.0)
                .resizable(true)
                .frame(
                    Frame::new()
                        .fill(pal.bg_secondary)
                        .inner_margin(Margin::symmetric(8, 4)),
                )
                .show(ui, |ui| panes::right_sidebar(ui, state, &mut actions));
            state.settings.right_width = r.response.rect.width();
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.bg_primary))
            .show(ui, |ui| {
                groups::workspace_area(ui, state, caches, &mut actions)
            });

        if let Some(m) = state.modal.take() {
            let keep = modals::show(&ctx, state, m, &mut actions);
            if state.modal.is_none() {
                state.modal = keep;
            }
        }
        if let Some(d) = state.ws.dialog.take() {
            let keep = dialogs::show(&ctx, state, d);
            if state.ws.dialog.is_none() {
                state.ws.dialog = keep;
            }
        }
        settings_ui::show(&ctx, state);
        git_ui::dialogs(&ctx, state);
        groups::drag_preview(&ctx);
        toasts(&ctx, state);

        for a in actions {
            apply(state, a);
        }
        if !state.editor.pending.is_empty() {
            ctx.request_repaint();
        }
        ws_ui::flush_pending(&ctx, state);
        state.maybe_save_workspace();

        // Scheduling.
        if let Some(t) = state.next_autosave_in() {
            ctx.request_repaint_after(Duration::from_secs_f64(t + 0.02));
        }
        if state.note_info_stale() || !state.toasts.is_empty() || state.git.running > 0 {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // Poll the file watcher regularly.
        ctx.request_repaint_after(Duration::from_secs(1));
        let title = match state.active_tab() {
            Some(t) => format!("{} - {} - Oxinote", t.title(), state.vault.name()),
            None => format!("{} - Oxinote", state.vault.name()),
        };
        if self.title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        if state.switch_vault_requested {
            state.shutdown();
            self.settings = state.settings.clone();
            self.state = None;
        }
    }
}

impl eframe::App for OxinoteApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        let theme = self
            .state
            .as_ref()
            .map_or(self.settings.theme, |s| s.settings.theme);
        if self.theme_applied != Some(theme) {
            theme::apply_choice(&ctx, theme);
            self.theme_applied = Some(theme);
        }
        let look = self.state.as_ref().map_or(&self.settings, |s| &s.settings);
        let look_key = (
            look.accent,
            look.ui_font_size.to_bits(),
            look.text_font == "Inter",
        );
        if self.look_applied != Some(look_key) {
            if self.look_applied.is_none_or(|k| k.2 != look_key.2) {
                theme::install_fonts(&ctx, look_key.2);
            }
            theme::apply_settings(&ctx, look_key.0, look.ui_font_size);
            self.look_applied = Some(look_key);
        }
        if self.state.is_some() {
            self.workspace(ui);
        } else {
            self.picker(ui);
        }
        self.handle_screenshot(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = self.state.as_ref().map_or(&self.settings, |s| &s.settings);
        eframe::set_value(storage, "settings", settings);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(s) = self.state.as_mut() {
            s.shutdown();
        }
    }
}
