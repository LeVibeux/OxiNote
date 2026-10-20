//! Graph view tab: force layout from `ors-graph`, pan/zoom, node dragging,
//! hover highlighting and a filters panel.

use std::collections::HashSet;

use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Margin, Pos2, RichText, Sense, Stroke, Vec2,
};
use ors_core::GraphNodeKind;

use super::Action;
use super::theme::Palette;
use crate::state::{AppState, GraphTab, OpenIn, TabContent};

const ENERGY_EPSILON: f32 = 0.05;

fn to_screen(center: Pos2, tab: &GraphTab, p: ors_graph::Vec2) -> Pos2 {
    center + Vec2::new(tab.pan.0, tab.pan.1) + Vec2::new(p.x, p.y) * tab.zoom
}

fn to_world(center: Pos2, tab: &GraphTab, p: Pos2) -> ors_graph::Vec2 {
    let v = (p - center - Vec2::new(tab.pan.0, tab.pan.1)) / tab.zoom;
    ors_graph::Vec2::new(v.x, v.y)
}

pub fn graph_view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    tab_index: usize,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let generation = state.generation;
    let active_note = state.recent_files.first().map(|p| p.as_str().to_string());
    let index = &*state.index;
    let Some(TabContent::Graph(tab)) = state.tabs.get_mut(tab_index).map(|t| &mut t.content) else {
        return;
    };
    let tab: &mut GraphTab = tab;
    tab.refresh(index, generation);

    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, pal.bg_primary);
    let center = rect.center();

    // Simulation.
    let dt = ui.input(|i| i.stable_dt).min(0.05);
    if let Some(layout) = tab.layout.as_mut() {
        if tab.energy > ENERGY_EPSILON || tab.dragging.is_some() {
            tab.energy = layout.step(&tab.data, dt * 60.0);
            ui.ctx().request_repaint();
        }
        if !tab.fitted && !tab.data.nodes.is_empty() && tab.energy <= ENERGY_EPSILON * 20.0 {
            let (min, max) = layout.bounds();
            let w = (max.x - min.x).max(1.0);
            let h = (max.y - min.y).max(1.0);
            tab.zoom = ((rect.width() - 120.0) / w)
                .min((rect.height() - 120.0) / h)
                .clamp(0.2, 2.5);
            tab.pan = (
                -(min.x + max.x) / 2.0 * tab.zoom,
                -(min.y + max.y) / 2.0 * tab.zoom,
            );
            tab.fitted = true;
        }
    }
    let Some(mut layout) = tab.layout.take() else {
        return;
    };
    let positions: Vec<ors_graph::Vec2> = layout.positions().to_vec();
    let degrees = tab.data.degrees();
    let radius = |i: usize| ors_graph::node_radius(degrees.get(i).copied().unwrap_or(0));

    // Hover.
    let pointer = resp.hover_pos();
    let hovered = pointer.and_then(|p| {
        let w = to_world(center, tab, p);
        layout.hit_test(w, 14.0 / tab.zoom.max(0.05))
    });
    let hovered = tab.dragging.or(hovered).filter(|i| *i < positions.len());

    // Interaction.
    if resp.drag_started() {
        tab.dragging = hovered;
    }
    if resp.dragged() {
        match tab.dragging {
            Some(i) => {
                if let Some(p) = resp.interact_pointer_pos() {
                    layout.pin(i, Some(to_world(center, tab, p)));
                    tab.energy = 1.0;
                }
            }
            None => {
                let d = resp.drag_delta();
                tab.pan.0 += d.x;
                tab.pan.1 += d.y;
            }
        }
    }
    if resp.drag_stopped()
        && let Some(i) = tab.dragging.take()
    {
        layout.pin(i, None);
    }
    if resp.hovered() {
        let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll * 0.0025).exp() * zoom_delta;
        if (factor - 1.0).abs() > 1e-4 {
            let new_zoom = (tab.zoom * factor).clamp(0.05, 8.0);
            if let Some(p) = pointer {
                // Keep the point under the cursor fixed.
                let world = to_world(center, tab, p);
                tab.zoom = new_zoom;
                let after = to_screen(center, tab, world);
                tab.pan.0 += p.x - after.x;
                tab.pan.1 += p.y - after.y;
            } else {
                tab.zoom = new_zoom;
            }
        }
    }
    if (resp.clicked() || resp.middle_clicked())
        && let Some(i) = hovered
        && let Some(node) = tab.data.nodes.get(i)
    {
        let open_in = if resp.middle_clicked() || ui.input(|inp| inp.modifiers.command) {
            OpenIn::NewTab
        } else {
            OpenIn::CurrentTab
        };
        match (&node.path, node.kind) {
            (Some(p), _) => actions.push(Action::Open(p.clone(), open_in)),
            (None, GraphNodeKind::Unresolved) => {
                actions.push(Action::OpenOrCreate(node.id.clone()))
            }
            (None, GraphNodeKind::Tag) => {
                actions.push(Action::SearchTag(
                    node.id.trim_start_matches('#').to_string(),
                ));
            }
            _ => {}
        }
    }
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // Neighbors of the hovered node.
    let neighbors: HashSet<usize> = match hovered {
        Some(h) => tab
            .data
            .edges
            .iter()
            .filter_map(|e| {
                if e.source == h {
                    Some(e.target)
                } else if e.target == h {
                    Some(e.source)
                } else {
                    None
                }
            })
            .chain(std::iter::once(h))
            .collect(),
        None => HashSet::new(),
    };
    let faded = |i: usize| hovered.is_some() && !neighbors.contains(&i);

    // Edges.
    for e in &tab.data.edges {
        let (Some(a), Some(b)) = (positions.get(e.source), positions.get(e.target)) else {
            continue;
        };
        let hot = hovered.is_some_and(|h| e.source == h || e.target == h);
        let color = if hot {
            pal.accent
        } else if hovered.is_some() {
            pal.graph_line.gamma_multiply(0.35)
        } else {
            pal.graph_line
        };
        painter.line_segment(
            [to_screen(center, tab, *a), to_screen(center, tab, *b)],
            Stroke::new(if hot { 1.6 } else { 1.0 }, color),
        );
    }

    // Nodes and labels.
    let label_alpha = ((tab.zoom - 0.45) / 0.5).clamp(0.0, 1.0);
    for (i, node) in tab.data.nodes.iter().enumerate() {
        let Some(p) = positions.get(i) else { continue };
        let pos = to_screen(center, tab, *p);
        if !rect.expand(50.0).contains(pos) {
            continue;
        }
        let r = (radius(i) * tab.zoom.sqrt()).max(2.0);
        let is_active = active_note.as_deref() == Some(node.id.as_str());
        let mut color = match node.kind {
            GraphNodeKind::Note => pal.graph_node,
            GraphNodeKind::Attachment => pal.graph_attachment,
            GraphNodeKind::Tag => pal.graph_tag,
            GraphNodeKind::Unresolved => pal.faint,
        };
        if is_active || hovered == Some(i) {
            color = pal.accent;
        }
        if faded(i) {
            color = color.gamma_multiply(0.3);
        }
        if node.kind == GraphNodeKind::Unresolved {
            painter.circle_stroke(pos, r, Stroke::new(1.2, color));
        } else {
            painter.circle_filled(pos, r, color);
        }
        let alpha = if neighbors.contains(&i) {
            1.0
        } else if faded(i) {
            label_alpha * 0.3
        } else {
            label_alpha
        };
        if alpha > 0.02 {
            painter.text(
                pos + Vec2::new(0.0, r + 3.0),
                Align2::CENTER_TOP,
                &node.label,
                FontId::proportional(if hovered == Some(i) { 13.5 } else { 12.0 }),
                pal.text.gamma_multiply(alpha),
            );
        }
    }
    if tab.data.nodes.is_empty() {
        painter.text(
            center,
            Align2::CENTER_CENTER,
            "No notes to display",
            FontId::proportional(15.0),
            pal.muted,
        );
    }

    tab.layout = Some(layout);
    filters_panel(ui, tab, rect, &pal);
}

fn filters_panel(ui: &mut egui::Ui, tab: &mut GraphTab, rect: egui::Rect, pal: &Palette) {
    let id = egui::Id::unique(("graph-filters", tab as *const GraphTab as usize));
    egui::Area::new(id)
        .order(egui::Order::Middle)
        .fixed_pos(rect.right_top() + Vec2::new(-230.0, 10.0))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(pal.bg_secondary)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(6)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(200.0);
                    ui.horizontal(|ui| {
                        let arrow = if tab.show_filters { "⏷" } else { "⏵" };
                        if ui
                            .add(
                                egui::Label::new(
                                    RichText::new(format!("{arrow} Filters")).strong(),
                                )
                                .sense(Sense::click()),
                            )
                            .clicked()
                        {
                            tab.show_filters = !tab.show_filters;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("⟲").on_hover_text("Fit to view").clicked() {
                                tab.fitted = false;
                                tab.energy = 1.0;
                            }
                        });
                    });
                    if !tab.show_filters {
                        return;
                    }
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut tab.filter)
                            .hint_text("Search files…")
                            .desired_width(f32::INFINITY),
                    );
                    ui.checkbox(&mut tab.show_tags, "Tags");
                    ui.checkbox(&mut tab.show_attachments, "Attachments");
                    ui.checkbox(&mut tab.show_unresolved, "Unresolved links");
                    ui.add_enabled(
                        !tab.local,
                        egui::Checkbox::new(&mut tab.show_orphans, "Orphans"),
                    );
                    ui.separator();
                    let can_local = tab.center.is_some();
                    ui.add_enabled(
                        can_local,
                        egui::Checkbox::new(&mut tab.local, "Local graph"),
                    );
                    if tab.local {
                        ui.horizontal(|ui| {
                            ui.label("Depth");
                            ui.add(egui::Slider::new(&mut tab.depth, 1..=5));
                        });
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} nodes · {} links",
                            tab.data.nodes.len(),
                            tab.data.edges.len()
                        ))
                        .small()
                        .color(pal.faint),
                    );
                });
        });
    let _ = Color32::TRANSPARENT;
}
