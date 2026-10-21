//! Sidebar panes (explorer, search, tags, backlinks, outgoing links,
//! outline), tab bar and status bar.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Frame, Margin, RichText, Sense, text::LayoutJob};

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::commands::Command;
use crate::settings::{LeftPane, RightPane};
use crate::state::{AppState, OpenIn, ViewMode};

fn open_in_from(ui: &egui::Ui, resp: &egui::Response) -> OpenIn {
    if resp.middle_clicked() || ui.input(|i| i.modifiers.command) {
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
    });
    ui.separator();
    match state.settings.left_pane {
        LeftPane::Files => super::explorer_ui::explorer(ui, state, caches, actions),
        LeftPane::Search => search_pane(ui, state, actions),
        LeftPane::Tags => tags_pane(ui, state, actions),
        LeftPane::Bookmarks => super::explorer_ui::bookmarks_pane(ui, state, actions),
    }
}

fn highlighted_line(text: &str, ranges: &[(usize, usize)], pal: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    let normal = egui::TextFormat {
        color: pal.muted,
        font_id: egui::FontId::proportional(12.5),
        ..Default::default()
    };
    let hit = egui::TextFormat {
        color: pal.strong,
        background: pal.highlight_bg,
        ..normal.clone()
    };
    let mut pos = 0;
    let mut sorted: Vec<(usize, usize)> = ranges.to_vec();
    sorted.sort_unstable();
    for (s, e) in sorted {
        if s < pos || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        job.append(&text[pos..s], 0.0, normal.clone());
        job.append(&text[s..e], 0.0, hit.clone());
        pos = e;
    }
    job.append(&text[pos..], 0.0, normal);
    job.wrap.max_rows = 3;
    job
}

fn search_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let resp = ui.add(
        egui::TextEdit::singleline(&mut state.search_query)
            .hint_text("Search…  (tag:#x  path:  \"phrase\"  -not)")
            .desired_width(f32::INFINITY),
    );
    if state.focus_search {
        resp.request_focus();
        state.focus_search = false;
    }
    state.refresh_search();
    ui.add_space(4.0);
    let results = match &state.search_results {
        Ok(r) => r,
        Err(e) => {
            ui.label(
                RichText::new(format!("Invalid query: {e}"))
                    .color(Color32::from_rgb(0xe9, 0x31, 0x47)),
            );
            return;
        }
    };
    if !state.search_query.trim().is_empty() {
        ui.label(
            RichText::new(format!("{} results", results.len()))
                .small()
                .color(pal.faint),
        );
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for r in results {
                let id = ui.make_persistent_id(("search", r.path.as_str()));
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    true,
                )
                .show_header(ui, |ui| {
                    let l = ui
                        .add(
                            egui::Label::new(RichText::new(r.path.stem()).color(pal.strong))
                                .sense(Sense::click()),
                        )
                        .on_hover_text(r.path.as_str());
                    if l.clicked() || l.middle_clicked() {
                        actions.push(Action::Open(r.path.clone(), open_in_from(ui, &l)));
                    }
                    if !r.matches.is_empty() {
                        ui.label(
                            RichText::new(r.matches.len().to_string())
                                .small()
                                .color(pal.faint),
                        );
                    }
                })
                .body(|ui| {
                    for m in &r.matches {
                        let job = highlighted_line(&m.line_text, &m.ranges, &pal);
                        let resp = Frame::new()
                            .inner_margin(Margin::symmetric(6, 3))
                            .corner_radius(4)
                            .fill(pal.bg_primary)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(job).wrap().sense(Sense::click()))
                            });
                        let l = resp.inner.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if l.clicked() || l.middle_clicked() {
                            actions.push(Action::OpenLine(
                                r.path.clone(),
                                m.line,
                                open_in_from(ui, &l),
                            ));
                        }
                        ui.add_space(2.0);
                    }
                });
            }
        });
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("⎔")
                .on_hover_text("Open local graph")
                .clicked()
            {
                actions.push(Action::Exec(Command::OpenLocalGraph));
            }
        });
    });
    ui.separator();
    let pane = state.settings.right_pane;
    let info = state.note_info().clone();
    let Some(path) = info.path.clone() else {
        ui.label(RichText::new("No file is open.").color(pal.muted));
        return;
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match pane {
            RightPane::Backlinks => {
                section_title(
                    ui,
                    &format!("Linked mentions ({})", info.backlink_count),
                    &pal,
                );
                if info.backlinks.is_empty() {
                    ui.label(RichText::new("No backlinks found.").color(pal.faint));
                }
                for (src, lines) in &info.backlinks {
                    let l = ui
                        .add(
                            egui::Label::new(RichText::new(src.stem()).color(pal.strong))
                                .sense(Sense::click()),
                        )
                        .on_hover_text(src.as_str());
                    if l.clicked() || l.middle_clicked() {
                        actions.push(Action::Open(src.clone(), open_in_from(ui, &l)));
                    }
                    for (line, text) in lines {
                        let resp = Frame::new()
                            .inner_margin(Margin::symmetric(6, 3))
                            .corner_radius(4)
                            .fill(pal.bg_primary)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(text).size(12.5).color(pal.muted),
                                    )
                                    .wrap()
                                    .sense(Sense::click()),
                                )
                            });
                        let r = resp.inner.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.clicked() || r.middle_clicked() {
                            actions.push(Action::OpenLine(
                                src.clone(),
                                *line,
                                open_in_from(ui, &r),
                            ));
                        }
                    }
                    ui.add_space(6.0);
                }
                ui.add_space(8.0);
                section_title(
                    ui,
                    &format!("Unlinked mentions ({})", info.unlinked.len()),
                    &pal,
                );
                for src in &info.unlinked {
                    let l = ui.add(
                        egui::Label::new(RichText::new(src.stem()).color(pal.text))
                            .sense(Sense::click()),
                    );
                    if l.clicked() || l.middle_clicked() {
                        actions.push(Action::Open(src.clone(), open_in_from(ui, &l)));
                    }
                }
            }
            RightPane::Outgoing => {
                let (resolved, unresolved): (Vec<_>, Vec<_>) =
                    info.outgoing.iter().partition(|l| l.target.is_some());
                section_title(ui, &format!("Links ({})", resolved.len()), &pal);
                let mut seen = std::collections::HashSet::new();
                for l in resolved {
                    let Some(t) = &l.target else { continue };
                    if !seen.insert(t.clone()) {
                        continue;
                    }
                    let r = ui
                        .add(
                            egui::Label::new(RichText::new(t.stem()).color(pal.accent))
                                .sense(Sense::click()),
                        )
                        .on_hover_text(t.as_str());
                    if r.clicked() || r.middle_clicked() {
                        actions.push(Action::Open(t.clone(), open_in_from(ui, &r)));
                    }
                }
                ui.add_space(8.0);
                section_title(ui, &format!("Unresolved ({})", unresolved.len()), &pal);
                for l in unresolved {
                    let r = ui
                        .add(
                            egui::Label::new(RichText::new(&l.link.target).color(pal.unresolved))
                                .sense(Sense::click()),
                        )
                        .on_hover_text("Click to create");
                    if r.clicked() {
                        actions.push(Action::Follow {
                            target: l.link.target.clone(),
                            from: path.clone(),
                            open_in: OpenIn::CurrentTab,
                        });
                    }
                }
            }
            RightPane::Outline => {
                if info.headings.is_empty() {
                    ui.label(RichText::new("No headings.").color(pal.faint));
                }
                let min_level = info.headings.iter().map(|h| h.level).min().unwrap_or(1);
                for h in &info.headings {
                    ui.horizontal(|ui| {
                        ui.add_space(f32::from(h.level - min_level) * 14.0);
                        let r = ui.add(
                            egui::Label::new(RichText::new(&h.text).color(pal.text))
                                .sense(Sense::click()),
                        );
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            actions.push(Action::OpenLine(
                                path.clone(),
                                h.line,
                                OpenIn::CurrentTab,
                            ));
                        }
                    });
                }
            }
        });
}

fn section_title(ui: &mut egui::Ui, text: &str, pal: &Palette) {
    ui.label(RichText::new(text).small().strong().color(pal.muted));
    ui.add_space(2.0);
}

// ---------------------------------------------------------------------------
// Tab bar & status bar
// ---------------------------------------------------------------------------

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
