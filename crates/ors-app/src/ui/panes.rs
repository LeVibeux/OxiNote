//! Sidebar panes (explorer, search, tags, backlinks, outgoing links,
//! outline), tab bar and status bar.

use eframe::egui::{self, RichText, Sense};

use ors_core::NotePath;

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::commands::Command;
use crate::settings::{LeftPane, RightPane};
use crate::state::{AppState, OpenIn, ViewMode};

pub(super) fn open_in_from(ui: &egui::Ui, resp: &egui::Response) -> OpenIn {
    let m = ui.input(|i| i.modifiers);
    if m.command && m.alt && m.shift {
        OpenIn::SplitDown
    } else if m.command && m.alt {
        OpenIn::SplitRight
    } else if resp.middle_clicked() || m.command {
        OpenIn::NewTab
    } else {
        OpenIn::CurrentTab
    }
}

fn pane_tab(ui: &mut egui::Ui, selected: bool, icon: &str, tip: &str) -> bool {
    let pal = Palette::of(ui.ctx());
    let color = if selected { pal.accent } else { pal.muted };
    ui.add(egui::Button::new(super::icons::text(icon, 16.0).color(color)).frame(selected))
        .on_hover_text(tip)
        .clicked()
}

// ---------------------------------------------------------------------------
// Left sidebar
// ---------------------------------------------------------------------------

pub fn left_sidebar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let cur = state.settings.left_pane;
        let plugins = &state.ws.plugins;
        let (bookmarks, tags) = (
            plugins.is_enabled(crate::plugins::CorePlugin::Bookmarks),
            plugins.is_enabled(crate::plugins::CorePlugin::TagPane),
        );
        if pane_tab(ui, cur == LeftPane::Files, super::icons::FOLDER, "Files") {
            state.settings.left_pane = LeftPane::Files;
        }
        if pane_tab(ui, cur == LeftPane::Search, super::icons::SEARCH, "Search") {
            state.settings.left_pane = LeftPane::Search;
        }
        if bookmarks
            && pane_tab(
                ui,
                cur == LeftPane::Bookmarks,
                super::icons::BOOKMARK,
                "Bookmarks",
            )
        {
            state.settings.left_pane = LeftPane::Bookmarks;
        }
        if tags && pane_tab(ui, cur == LeftPane::Tags, super::icons::TAG, "Tags") {
            state.settings.left_pane = LeftPane::Tags;
        }
        if pane_tab(
            ui,
            cur == LeftPane::Properties,
            super::icons::LIST_DASHES,
            "All properties",
        ) {
            state.settings.left_pane = LeftPane::Properties;
        }
    });
    ui.separator();
    match state.settings.left_pane {
        LeftPane::Files => super::explorer_ui::explorer(ui, state, caches, actions),
        LeftPane::Search => search_pane(ui, state, actions),
        LeftPane::Tags => super::tags_view::tags_pane(ui, state, actions),
        LeftPane::Properties => super::properties_view::all_properties_pane(ui, state, actions),
        LeftPane::Bookmarks => super::explorer_ui::bookmarks_pane(ui, state, actions),
    }
}

fn search_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    super::view_panes::search_pane(ui, state, actions);
}

// ---------------------------------------------------------------------------
// Right sidebar
// ---------------------------------------------------------------------------

pub fn right_sidebar(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let cur = state.settings.right_pane;
        if pane_tab(
            ui,
            cur == RightPane::Backlinks,
            super::icons::LINK,
            "Backlinks",
        ) {
            state.settings.right_pane = RightPane::Backlinks;
        }
        if pane_tab(
            ui,
            cur == RightPane::Outgoing,
            super::icons::LINK_SIMPLE,
            "Outgoing links",
        ) {
            state.settings.right_pane = RightPane::Outgoing;
        }
        if pane_tab(
            ui,
            cur == RightPane::Outline,
            super::icons::LIST_BULLETS,
            "Outline",
        ) {
            state.settings.right_pane = RightPane::Outline;
        }
        if pane_tab(
            ui,
            cur == RightPane::LocalGraph,
            super::icons::GRAPH,
            "Local graph",
        ) {
            state.settings.right_pane = RightPane::LocalGraph;
        }
        if pane_tab(
            ui,
            cur == RightPane::FileProperties,
            super::icons::ARTICLE,
            "File properties",
        ) {
            state.settings.right_pane = RightPane::FileProperties;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("↗")
                .on_hover_text("Open local graph in a tab")
                .clicked()
            {
                actions.push(Action::Exec(Command::OpenLocalGraph));
            }
        });
    });
    ui.separator();
    let pane = state.settings.right_pane;
    if pane == RightPane::LocalGraph {
        super::graph::local_graph_pane(ui, state, actions);
        return;
    }
    let Some(path) = side_note(state) else {
        ui.label(RichText::new("No file is open.").color(pal.muted));
        return;
    };
    match pane {
        RightPane::Backlinks => super::view_panes::backlinks_pane(ui, state, &path, actions),
        RightPane::Outgoing => super::view_panes::outgoing_pane(ui, state, &path, actions),
        RightPane::Outline => super::view_panes::outline_pane(ui, state, &path, actions),
        RightPane::FileProperties => {
            super::properties_view::panel(ui, state, &path, "side", false, actions);
        }
        RightPane::LocalGraph => {}
    }
}

// ---------------------------------------------------------------------------
// Tab bar & status bar
// ---------------------------------------------------------------------------

/// Note shown by the right sidebar: the active note, or, when a graph tab is
/// active, the last active note that is still open in a tab (like Obsidian).
fn side_note(state: &AppState) -> Option<NotePath> {
    if let Some(p) = state.active_note() {
        return Some(p.clone());
    }
    state.active_tab()?;
    state
        .recent_files
        .iter()
        .find(|p| {
            state
                .tabs
                .iter()
                .any(|t| t.note().is_some_and(|n| &n.path == *p))
        })
        .cloned()
}

pub fn status_bar(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let mode = state.active_tab().and_then(|t| t.note()).map(|n| n.mode);
    let path = state.active_note().cloned();
    let dirty = path
        .as_ref()
        .and_then(|p| state.buffers.get(p))
        .map(|b| b.is_dirty());
    let chars = path
        .as_ref()
        .and_then(|p| state.buffers.get(p))
        .map_or(0, |b| b.text.chars().count());
    let info = state.note_info().clone();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let small = |t: String| RichText::new(t).size(12.0).color(pal.muted);
        if let Some(mode) = mode {
            let label = match mode {
                ViewMode::Source => "✏ Editing",
                ViewMode::Reading => "📖 Reading",
            };
            if ui
                .add(egui::Label::new(small(label.into())).sense(Sense::click()))
                .on_hover_text("Toggle reading view (Ctrl+E)")
                .clicked()
            {
                actions.push(Action::Exec(Command::ToggleMode));
            }
        }
        if path.is_some() {
            ui.label(small(format!("{chars} characters")));
            ui.label(small(format!("{} words", info.word_count)));
            let r = ui.add(
                egui::Label::new(small(format!("{} backlinks", info.backlink_count)))
                    .sense(Sense::click()),
            );
            if r.clicked() {
                actions.push(Action::ShowBacklinks);
            }
            match dirty {
                Some(true) => ui.label(small("● unsaved".into())),
                Some(false) => ui.label(small("✔ saved".into())),
                None => ui.label(small(String::new())),
            };
        }
    });
}
