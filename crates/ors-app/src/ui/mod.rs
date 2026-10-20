//! eframe application: layout, input routing and the vault picker. All
//! state changes go through [`AppState`] (via [`Action`]s collected while
//! drawing, applied at the end of the frame).

mod editor;
mod graph;
mod modals;
mod panes;
mod reading_view;
#[cfg(test)]
mod tests;
pub mod theme;
mod view_panes;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Frame, Margin, RichText, text::LayoutJob};
use ors_core::{NotePath, VaultRead};

use crate::backend::{DiskVault, IndexBackend, RealIndex};
use crate::commands::Command;
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
    Activate(usize),
    CloseTab(TabId),
    MoveTab(usize, usize),
    Resolve(NotePath, bool),
    Edited(NotePath),
    ShowBacklinks,
    /// Requests of the graph / search / backlinks / outline / reading views.
    View(crate::views::ViewAction),
    Error(String),
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
        Action::Activate(i) => state.set_active(i),
        Action::CloseTab(id) => state.close_tab_by_id(id),
        Action::MoveTab(a, b) => state.move_tab(a, b),
        Action::Resolve(p, keep) => state.resolve_conflict(&p, keep),
        Action::Edited(p) => state.buffer_edited(&p),
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
    completion: Option<editor::Completion>,
    tree: panes::FolderNode,
    tree_key: Option<(usize, usize, u64)>,
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
}

pub struct ObsiApp {
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

fn fake_index(vault: &dyn VaultRead) -> Box<dyn IndexBackend> {
    Box::new(FakeIndex::build(vault))
}

impl ObsiApp {
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
                self.state = Some(AppState::new(
                    Box::new(v),
                    real_index,
                    self.settings.clone(),
                ));
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
        }
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
                    ui.label(RichText::new("ObsiRust").size(30.0).color(pal.strong));
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
    let pal = Palette::of(ui.ctx());
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        let items: [(&str, Command); 7] = [
            ("☰", Command::ToggleLeftSidebar),
            ("🔍", Command::QuickSwitcher),
            ("⎔", Command::OpenGraph),
            ("📅", Command::OpenDailyNote),
            ("📝", Command::NewNote),
            (">_", Command::CommandPalette),
            ("◐", Command::ToggleTheme),
        ];
        for (icon, cmd) in items {
            let name = state.commands.get(cmd).map_or("", |s| s.name);
            let tip = match state.commands.shortcut(cmd) {
                Some(k) => format!("{name} ({})", ui.ctx().format_shortcut(&k)),
                None => name.to_string(),
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(icon).size(16.0).color(pal.muted)).frame(false),
                )
                .on_hover_text(tip)
                .clicked()
            {
                actions.push(Action::Exec(cmd));
            }
            ui.add_space(6.0);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if ui
                .add(egui::Button::new(RichText::new("⇄").size(16.0).color(pal.muted)).frame(false))
                .on_hover_text("Open another vault")
                .clicked()
            {
                actions.push(Action::Exec(Command::SwitchVault));
            }
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
                let (icon, tip) = match note.mode {
                    ViewMode::Source => ("📖", "Reading view (Ctrl+E)"),
                    ViewMode::Reading => ("✏", "Editing view (Ctrl+E)"),
                };
                if ui
                    .add(egui::Button::new(RichText::new(icon).color(pal.muted)).frame(false))
                    .on_hover_text(tip)
                    .clicked()
                {
                    actions.push(Action::Exec(Command::ToggleMode));
                }
            }
        });
    });
}

fn central(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    Frame::new()
        .fill(pal.bg_secondary)
        .inner_margin(Margin {
            left: 4,
            right: 4,
            top: 4,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            panes::tab_bar(ui, state, actions);
        });
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
        None => graph::graph_view(ui, state, i, actions),
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
            let scroll_to = if mode == ViewMode::Reading {
                crate::views::take_pending_line(state)
            } else {
                None
            };
            egui::ScrollArea::vertical()
                .id_salt(("note-scroll", tab_id, mode == ViewMode::Reading))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if !path.is_markdown() {
                        editor::file_view(ui, state, caches, &path);
                    } else if mode == ViewMode::Reading {
                        reading_view::reading_view(ui, state, caches, &path, scroll_to, actions);
                    } else {
                        editor::source_view(ui, state, caches, tab_id, &path, actions);
                    }
                });
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

impl ObsiApp {
    fn workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let caches = &mut self.caches;
        state.tick(self.start.elapsed().as_secs_f64());
        state.poll_vault();
        let mut actions: Vec<Action> = Vec::new();

        // Shortcuts (shared with the palette through the registry).
        for (shortcut, cmd) in state.commands.by_specificity() {
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                actions.push(Action::Exec(cmd));
            }
        }
        let pal = Palette::of(&ctx);

        egui::Panel::left("ribbon")
            .exact_size(40.0)
            .resizable(false)
            .frame(Frame::new().fill(pal.bg_secondary))
            .show(ui, |ui| ribbon(ui, state, &mut actions));
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(
                Frame::new()
                    .fill(pal.bg_secondary)
                    .inner_margin(Margin::symmetric(12, 2)),
            )
            .show(ui, |ui| panes::status_bar(ui, state, &mut actions));
        if state.settings.left_open {
            egui::Panel::left("left-sidebar")
                .default_size(250.0)
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
        }
        if state.settings.right_open {
            egui::Panel::right("right-sidebar")
                .default_size(270.0)
                .size_range(160.0..=500.0)
                .resizable(true)
                .frame(
                    Frame::new()
                        .fill(pal.bg_secondary)
                        .inner_margin(Margin::symmetric(8, 4)),
                )
                .show(ui, |ui| panes::right_sidebar(ui, state, &mut actions));
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.bg_primary))
            .show(ui, |ui| central(ui, state, caches, &mut actions));

        if let Some(m) = state.modal.take() {
            let keep = modals::show(&ctx, state, m, &mut actions);
            if state.modal.is_none() {
                state.modal = keep;
            }
        }
        toasts(&ctx, state);

        for a in actions {
            apply(state, a);
        }

        // Scheduling.
        if let Some(t) = state.next_autosave_in() {
            ctx.request_repaint_after(Duration::from_secs_f64(t + 0.02));
        }
        if state.note_info_stale() || !state.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // Poll the file watcher regularly.
        ctx.request_repaint_after(Duration::from_secs(1));
        let title = match state.active_tab() {
            Some(t) => format!("{} - {} - ObsiRust", t.title(), state.vault.name()),
            None => format!("{} - ObsiRust", state.vault.name()),
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

impl eframe::App for ObsiApp {
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
