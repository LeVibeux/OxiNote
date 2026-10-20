//! Sidebar panes (explorer, search, tags, backlinks, outgoing links,
//! outline), tab bar and status bar.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Frame, Margin, RichText, Sense, Stroke};
use ors_core::NotePath;

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::commands::Command;
use crate::settings::{LeftPane, RightPane};
use crate::state::{AppState, FsTarget, Modal, OpenIn, TabContent, ViewMode};

pub(super) fn open_in_from(ui: &egui::Ui, resp: &egui::Response) -> OpenIn {
    if resp.middle_clicked() || ui.input(|i| i.modifiers.command) {
        OpenIn::NewTab
    } else {
        OpenIn::CurrentTab
    }
}

fn pane_tab(ui: &mut egui::Ui, selected: bool, icon: &str, tip: &str) -> bool {
    let pal = Palette::of(ui.ctx());
    let color = if selected { pal.accent } else { pal.muted };
    ui.add(egui::Button::new(RichText::new(icon).size(15.0).color(color)).frame(selected))
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
        if pane_tab(ui, cur == LeftPane::Files, "🗀", "Files") {
            state.settings.left_pane = LeftPane::Files;
        }
        if pane_tab(ui, cur == LeftPane::Search, "🔍", "Search") {
            state.settings.left_pane = LeftPane::Search;
        }
        if pane_tab(ui, cur == LeftPane::Tags, "#", "Tags") {
            state.settings.left_pane = LeftPane::Tags;
        }
    });
    ui.separator();
    match state.settings.left_pane {
        LeftPane::Files => explorer(ui, state, caches, actions),
        LeftPane::Search => search_pane(ui, state, actions),
        LeftPane::Tags => tags_pane(ui, state, actions),
    }
}

#[derive(Default)]
pub struct FolderNode {
    folders: BTreeMap<String, FolderNode>,
    files: Vec<NotePath>,
}

pub fn build_tree(files: &[NotePath], folders: &[String]) -> FolderNode {
    let mut root = FolderNode::default();
    for f in folders {
        let mut node = &mut root;
        for part in f.split('/') {
            node = node.folders.entry(part.to_string()).or_default();
        }
    }
    for f in files {
        let mut node = &mut root;
        if let Some(parent) = f.parent() {
            for part in parent.split('/') {
                node = node.folders.entry(part.to_string()).or_default();
            }
        }
        node.files.push(f.clone());
    }
    root
}

fn explorer(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        let root = state.vault.root().map_or_else(
            || "in-memory vault".to_string(),
            |r| r.display().to_string(),
        );
        ui.label(RichText::new(state.vault.name()).strong().color(pal.strong))
            .on_hover_text(root);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("🗁+").on_hover_text("New folder").clicked() {
                actions.push(Action::Modal(Modal::NewFolder {
                    parent: String::new(),
                    name: String::new(),
                }));
            }
            if ui.small_button("📝").on_hover_text("New note").clicked() {
                actions.push(Action::Exec(Command::NewNote));
            }
        });
    });
    ui.add_space(2.0);
    let key = (state.files.len(), state.folders.len(), state.generation);
    if caches.tree_key != Some(key) {
        caches.tree = build_tree(&state.files, &state.folders);
        caches.tree_key = Some(key);
    }
    let active = state.active_note().cloned();
    let dirty: Vec<NotePath> = state
        .buffers
        .iter()
        .filter(|(_, b)| b.is_dirty())
        .map(|(p, _)| p.clone())
        .collect();
    let tree = std::mem::take(&mut caches.tree);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            folder_contents(ui, &tree, "", active.as_ref(), &dirty, &pal, actions);
            // Empty area: context menu for the vault root.
            let rest = ui.allocate_response(
                ui.available_size().max(egui::vec2(10.0, 40.0)),
                Sense::click(),
            );
            rest.context_menu(|ui| folder_menu(ui, "", actions));
        });
    caches.tree = tree;
}

fn folder_menu(ui: &mut egui::Ui, folder: &str, actions: &mut Vec<Action>) {
    if ui.button("New note").clicked() {
        actions.push(Action::NewNoteIn(folder.to_string()));
        ui.close();
    }
    if ui.button("New folder").clicked() {
        actions.push(Action::Modal(Modal::NewFolder {
            parent: folder.to_string(),
            name: String::new(),
        }));
        ui.close();
    }
    if !folder.is_empty() {
        ui.separator();
        if ui.button("Rename…").clicked() {
            actions.push(Action::Modal(Modal::Rename {
                target: FsTarget::Folder(folder.to_string()),
                name: folder.rsplit('/').next().unwrap_or(folder).to_string(),
            }));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            actions.push(Action::Modal(Modal::ConfirmDelete {
                target: FsTarget::Folder(folder.to_string()),
            }));
            ui.close();
        }
    }
}

fn folder_contents(
    ui: &mut egui::Ui,
    node: &FolderNode,
    prefix: &str,
    active: Option<&NotePath>,
    dirty: &[NotePath],
    pal: &Palette,
    actions: &mut Vec<Action>,
) {
    let mut folders: Vec<(&String, &FolderNode)> = node.folders.iter().collect();
    folders.sort_by_key(|(n, _)| n.to_lowercase());
    for (name, child) in folders {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let id = ui.make_persistent_id(("folder", &path));
        let header =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    let r = ui.add(
                        egui::Label::new(RichText::new(name).color(pal.text)).sense(Sense::click()),
                    );
                    r.context_menu(|ui| folder_menu(ui, &path, actions));
                    r
                });
        let (_, header_resp, _) =
            header.body(|ui| folder_contents(ui, child, &path, active, dirty, pal, actions));
        if header_resp.inner.clicked() {
            let mut s = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            );
            s.toggle(ui);
            s.store(ui.ctx());
        }
    }
    let mut files: Vec<&NotePath> = node.files.iter().collect();
    files.sort_by_key(|f| f.file_name().to_lowercase());
    for f in files {
        let selected = active == Some(f);
        let mut label = if f.is_markdown() {
            f.stem().to_string()
        } else {
            f.file_name().to_string()
        };
        if dirty.contains(f) {
            label.push_str(" ●");
        }
        let mut text = RichText::new(label);
        if !f.is_markdown() {
            text = text.color(pal.muted);
        }
        let r = ui.add(egui::Button::selectable(selected, text).frame_when_inactive(false));
        let r = r.on_hover_text(f.as_str());
        if r.clicked() || r.middle_clicked() {
            actions.push(Action::Open(f.clone(), open_in_from(ui, &r)));
        }
        r.context_menu(|ui| {
            if ui.button("Open in new tab").clicked() {
                actions.push(Action::Open(f.clone(), OpenIn::NewTab));
                ui.close();
            }
            ui.separator();
            if ui.button("Rename…").clicked() {
                actions.push(Action::Modal(Modal::Rename {
                    target: FsTarget::File(f.clone()),
                    name: f.stem().to_string(),
                }));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                actions.push(Action::Modal(Modal::ConfirmDelete {
                    target: FsTarget::File(f.clone()),
                }));
                ui.close();
            }
        });
    }
}

fn search_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    super::view_panes::search_pane(ui, state, actions);
}

#[derive(Default)]
struct TagNode {
    count: usize,
    children: BTreeMap<String, TagNode>,
}

fn tags_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let tags = state.index.tags();
    if tags.is_empty() {
        ui.label(RichText::new("No tags found.").color(pal.muted));
        return;
    }
    let mut root = TagNode::default();
    for (tag, count) in &tags {
        let mut node = &mut root;
        for part in tag.split('/') {
            node = node.children.entry(part.to_string()).or_default();
        }
        node.count = *count;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            tag_nodes(ui, &root, "", &pal, actions);
        });
}

fn tag_nodes(
    ui: &mut egui::Ui,
    node: &TagNode,
    prefix: &str,
    pal: &Palette,
    actions: &mut Vec<Action>,
) {
    for (name, child) in &node.children {
        let full = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let row = |ui: &mut egui::Ui, actions: &mut Vec<Action>| {
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::Label::new(RichText::new(format!("#{name}")).color(pal.accent))
                        .sense(Sense::click()),
                );
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    actions.push(Action::SearchTag(full.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(child.count.to_string())
                            .small()
                            .color(pal.faint),
                    );
                });
            });
        };
        if child.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.vertical(|ui| row(ui, actions));
            });
        } else {
            let id = ui.make_persistent_id(("tag", &full));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| row(ui, actions))
                .body(|ui| tag_nodes(ui, child, &full, pal, actions));
        }
    }
}

// ---------------------------------------------------------------------------
// Right sidebar
// ---------------------------------------------------------------------------

pub fn right_sidebar(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let cur = state.settings.right_pane;
        if pane_tab(ui, cur == RightPane::Backlinks, "⮪", "Backlinks") {
            state.settings.right_pane = RightPane::Backlinks;
        }
        if pane_tab(ui, cur == RightPane::Outgoing, "⮫", "Outgoing links") {
            state.settings.right_pane = RightPane::Outgoing;
        }
        if pane_tab(ui, cur == RightPane::Outline, "☰", "Outline") {
            state.settings.right_pane = RightPane::Outline;
        }
        if pane_tab(ui, cur == RightPane::LocalGraph, "⎔", "Local graph") {
            state.settings.right_pane = RightPane::LocalGraph;
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
    let Some(path) = state.active_note().cloned() else {
        ui.label(RichText::new("No file is open.").color(pal.muted));
        return;
    };
    match pane {
        RightPane::Backlinks => super::view_panes::backlinks_pane(ui, state, &path, actions),
        RightPane::Outgoing => super::view_panes::outgoing_pane(ui, state, &path, actions),
        RightPane::Outline => super::view_panes::outline_pane(ui, state, &path, actions),
        RightPane::LocalGraph => {}
    }
}

// ---------------------------------------------------------------------------
// Tab bar & status bar
// ---------------------------------------------------------------------------

pub fn tab_bar(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    egui::ScrollArea::horizontal()
        .id_salt("tabbar")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, tab) in state.tabs.iter().enumerate() {
                    let active = state.active == Some(i);
                    let dirty = tab
                        .note()
                        .and_then(|n| state.buffers.get(&n.path))
                        .is_some_and(|b| b.is_dirty());
                    let icon = match &tab.content {
                        TabContent::Graph(_) => "⎔ ",
                        TabContent::Note(_) => "",
                    };
                    let fill = if active {
                        pal.bg_primary
                    } else {
                        Color32::TRANSPARENT
                    };
                    let frame = Frame::new()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius {
                            nw: 6,
                            ne: 6,
                            sw: 0,
                            se: 0,
                        })
                        .inner_margin(Margin::symmetric(10, 5))
                        .stroke(if active {
                            Stroke::new(1.0, pal.border)
                        } else {
                            Stroke::NONE
                        });
                    let inner = frame.show(ui, |ui| {
                        ui.set_min_width(90.0);
                        ui.horizontal(|ui| {
                            let color = if active { pal.strong } else { pal.muted };
                            let title = format!("{icon}{}", tab.title());
                            let label = ui.add(
                                egui::Label::new(RichText::new(title).color(color))
                                    .truncate()
                                    .sense(Sense::click_and_drag()),
                            );
                            let close = if dirty { "●" } else { "×" };
                            let close_resp = ui
                                .add(
                                    egui::Button::new(RichText::new(close).color(pal.faint))
                                        .frame(false)
                                        .small(),
                                )
                                .on_hover_text("Close (Ctrl+W)");
                            (label, close_resp)
                        })
                        .inner
                    });
                    let (label, close_resp) = inner.inner;
                    let tab_resp = inner.response.interact(Sense::click());
                    if label.clicked() || tab_resp.clicked() {
                        actions.push(Action::Activate(i));
                    }
                    if label.middle_clicked() || tab_resp.middle_clicked() || close_resp.clicked() {
                        actions.push(Action::CloseTab(tab.id));
                    }
                    label.dnd_set_drag_payload(i);
                    if let Some(from) = tab_resp
                        .dnd_release_payload::<usize>()
                        .or_else(|| label.dnd_release_payload::<usize>())
                    {
                        actions.push(Action::MoveTab(*from, i));
                    }
                    if tab_resp.dnd_hover_payload::<usize>().is_some()
                        || label.dnd_hover_payload::<usize>().is_some()
                    {
                        ui.painter().line_segment(
                            [
                                inner.response.rect.left_top(),
                                inner.response.rect.left_bottom(),
                            ],
                            Stroke::new(2.0, pal.accent),
                        );
                    }
                }
                if ui
                    .add(egui::Button::new(RichText::new("+").color(pal.muted)).frame(false))
                    .on_hover_text("New note (Ctrl+N)")
                    .clicked()
                {
                    actions.push(Action::Exec(Command::NewNote));
                }
            });
        });
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
