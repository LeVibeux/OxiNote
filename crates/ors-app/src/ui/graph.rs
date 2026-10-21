//! Graph view: force layout from `ors-graph`, pan/zoom, node dragging, hover
//! highlighting, colour groups and Obsidian's collapsible settings panel
//! (Filters / Groups / Display / Forces). Used by the graph tab and, in a
//! compact form, by the local graph in the right sidebar.

use eframe::egui::{
    self, Align2, Color32, FontId, Frame, Margin, Pos2, RichText, Sense, Stroke, Vec2,
};
use ors_core::{GraphNodeKind, NotePath};

use super::Action;
use super::theme::Palette;
use crate::backend::IndexBackend;
use crate::graph_model::{ColorGroup, GraphColor, GraphStore, GraphTab};
use crate::state::{AppState, OpenIn, TabContent};

const ENERGY_EPSILON: f32 = 0.05;

fn to_screen(center: Pos2, tab: &GraphTab, p: ors_graph::Vec2) -> Pos2 {
    center + Vec2::new(tab.pan.0, tab.pan.1) + Vec2::new(p.x, p.y) * tab.zoom
}

fn to_world(center: Pos2, tab: &GraphTab, p: Pos2) -> ors_graph::Vec2 {
    let v = (p - center - Vec2::new(tab.pan.0, tab.pan.1)) / tab.zoom;
    ors_graph::Vec2::new(v.x, v.y)
}

/// Opacity of the labels for a zoom level. `fade` is Obsidian's "text fade
/// threshold" in -3..=3: higher shows labels from further away.
pub fn label_alpha(zoom: f32, fade: f64) -> f32 {
    let start = 0.45 - fade as f32 * 0.12;
    ((zoom - start) / 0.5).clamp(0.0, 1.0)
}

/// The graph tab at `tab_index`.
pub fn graph_view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    tab_index: usize,
    actions: &mut Vec<Action>,
) {
    let active_note = state.recent_files.first().cloned();
    let generation = state.generation;
    let AppState {
        tabs, index, views, ..
    } = state;
    let Some(TabContent::Graph(tab)) = tabs.get_mut(tab_index).map(|t| &mut t.content) else {
        return;
    };
    let store = if tab.local {
        &mut views.local_graph
    } else {
        &mut views.graph
    };
    let env = Env {
        index: &**index,
        generation,
        active: active_note.as_ref(),
        compact: false,
    };
    show(ui, tab, store, &env, actions);
}

/// The local graph of the right sidebar, following the active note.
pub fn local_graph_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let active = state.active_note().cloned();
    let generation = state.generation;
    let AppState { index, views, .. } = state;
    let Some(active) = active else {
        ui.add_space(8.0);
        ui.label(RichText::new("No active note").weak());
        return;
    };
    let tab = views.local_pane.get_or_insert_with(|| {
        let mut t = Box::new(GraphTab::new(Some(active.clone())));
        t.settings = views.local_graph.settings.clone();
        t
    });
    if tab.center.as_ref() != Some(&active) {
        tab.center = Some(active.clone());
        tab.fitted = false;
    }
    let env = Env {
        index: &**index,
        generation,
        active: Some(&active),
        compact: true,
    };
    show(ui, tab, &mut views.local_graph, &env, actions);
}

struct Env<'a> {
    index: &'a dyn IndexBackend,
    generation: u64,
    active: Option<&'a NotePath>,
    compact: bool,
}

fn rgb(c: &GraphColor) -> Color32 {
    let [r, g, b] = c.rgb_bytes();
    Color32::from_rgb(r, g, b)
}

fn show(
    ui: &mut egui::Ui,
    tab: &mut GraphTab,
    store: &mut GraphStore,
    env: &Env<'_>,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    tab.sync_store(store);
    tab.refresh(env.index, env.generation);

    if env.compact {
        ui.horizontal(|ui| {
            ui.label("Depth");
            let mut d = tab.settings.depth();
            if ui.add(egui::Slider::new(&mut d, 1..=5)).changed() {
                tab.settings.local_jumps = Some(d);
            }
            if ui.small_button("⟲").on_hover_text("Fit to view").clicked() {
                tab.fitted = false;
            }
        });
    }
    let size = if env.compact {
        Vec2::new(ui.available_width(), ui.available_height().max(120.0))
    } else {
        ui.available_size()
    };
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = resp.rect;
    painter.rect_filled(rect, 0.0, pal.bg_primary);
    let painter = painter.with_clip_rect(rect);
    draw_canvas(ui, tab, env, &resp, &painter, &pal, actions);
    if !env.compact {
        settings_panel(ui, tab, env, rect, &pal);
    }
    tab.sync_store(store);
    let now = ui.input(|i| i.time);
    if let Err(e) = store.save_if_needed(now, 0.6) {
        actions.push(Action::Error(format!("Cannot save graph settings: {e}")));
    }
    if store.has_pending() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(700));
    }
}

fn draw_canvas(
    ui: &mut egui::Ui,
    tab: &mut GraphTab,
    env: &Env<'_>,
    resp: &egui::Response,
    painter: &egui::Painter,
    pal: &Palette,
    actions: &mut Vec<Action>,
) {
    let rect = resp.rect;
    let center = rect.center();
    // Simulation.
    let dt = ui.input(|i| i.stable_dt).min(0.05);
    if let Some(layout) = tab.layout.as_mut() {
        if tab.energy > ENERGY_EPSILON || tab.dragging.is_some() {
            tab.energy = layout.step(&tab.data, dt * 60.0);
            ui.ctx().request_repaint();
        }
        // Keep the graph framed while it settles, until the user moves the view.
        if !tab.fitted && !tab.data.nodes.is_empty() {
            let (min, max) = layout.bounds();
            let w = (max.x - min.x).max(1.0);
            let h = (max.y - min.y).max(1.0);
            let margin = if env.compact { 90.0 } else { 120.0 };
            tab.zoom = ((rect.width() - margin) / w)
                .min((rect.height() - margin) / h)
                .clamp(0.2, 2.5);
            tab.pan = (
                -(min.x + max.x) / 2.0 * tab.zoom,
                -(min.y + max.y) / 2.0 * tab.zoom,
            );
            if tab.energy <= ENERGY_EPSILON {
                tab.fitted = true;
            }
        }
    }
    let Some(mut layout) = tab.layout.take() else {
        return;
    };
    let positions: Vec<ors_graph::Vec2> = layout.positions().to_vec();
    let node_mul = tab.settings.node_size_multiplier as f32;
    let line_mul = tab.settings.line_size_multiplier as f32;
    let zoom = tab.zoom;
    let radius = |i: usize| {
        let d = tab.degrees.get(i).copied().unwrap_or(0);
        (ors_graph::node_radius(d) * node_mul * zoom.sqrt()).max(2.0)
    };

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
                    layout.reheat(0.3);
                    tab.energy = 1.0;
                    tab.fitted = true;
                }
            }
            None => {
                tab.fitted = true;
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
            tab.fitted = true;
            let new_zoom = (tab.zoom * factor).clamp(0.05, 8.0);
            if let Some(p) = pointer {
                // Zoom towards the cursor: keep the point under it fixed.
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

    // The hovered node and its neighbours stay lit, the others fade.
    let lit = |i: usize| match hovered {
        None => true,
        Some(h) => {
            i == h
                || tab
                    .adjacency
                    .get(h)
                    .is_some_and(|a| a.binary_search(&i).is_ok())
        }
    };

    // Edges.
    let arrows = tab.settings.show_arrow;
    for e in &tab.data.edges {
        let (Some(a), Some(b)) = (positions.get(e.source), positions.get(e.target)) else {
            continue;
        };
        let hot = hovered.is_some_and(|h| e.source == h || e.target == h);
        let color = if hot {
            pal.accent
        } else if hovered.is_some() {
            pal.graph_line.gamma_multiply(0.3)
        } else {
            pal.graph_line
        };
        let width = (if hot { 1.6 } else { 1.0 }) * line_mul;
        let pa = to_screen(center, tab, *a);
        let pb = to_screen(center, tab, *b);
        painter.line_segment([pa, pb], Stroke::new(width, color));
        if arrows {
            let dir = (pb - pa).normalized();
            if dir.length() > 0.0 {
                let tip = pb - dir * (radius(e.target) + 1.0);
                let n = Vec2::new(-dir.y, dir.x);
                let size = 5.0 + width;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        tip,
                        tip - dir * size + n * size * 0.5,
                        tip - dir * size - n * size * 0.5,
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
    }

    // Nodes and labels.
    let alpha = label_alpha(tab.zoom, tab.settings.text_fade_multiplier);
    for (i, node) in tab.data.nodes.iter().enumerate() {
        let Some(p) = positions.get(i) else { continue };
        let pos = to_screen(center, tab, *p);
        if !rect.expand(50.0).contains(pos) {
            continue;
        }
        let r = radius(i);
        let is_active = env.active.is_some_and(|a| node.path.as_ref() == Some(a));
        let group = tab
            .groups
            .get(i)
            .copied()
            .flatten()
            .and_then(|g| tab.settings.color_groups.get(g));
        let mut color = match (node.kind, group) {
            (_, Some(g)) => rgb(&g.color),
            (GraphNodeKind::Note, _) => pal.graph_node,
            (GraphNodeKind::Attachment, _) => pal.graph_attachment,
            (GraphNodeKind::Tag, _) => pal.graph_tag,
            (GraphNodeKind::Unresolved, _) => pal.faint,
        };
        if is_active || hovered == Some(i) {
            color = pal.accent;
        }
        let dim = !lit(i);
        if dim {
            color = color.gamma_multiply(0.25);
        }
        if node.kind == GraphNodeKind::Unresolved {
            painter.circle_stroke(pos, r, Stroke::new(1.2, color));
        } else {
            painter.circle_filled(pos, r, color);
        }
        let a = if hovered.is_some() && !dim {
            1.0
        } else if dim {
            alpha * 0.25
        } else {
            alpha
        };
        if a > 0.02 {
            painter.text(
                pos + Vec2::new(0.0, r + 3.0),
                Align2::CENTER_TOP,
                &node.label,
                FontId::proportional(if hovered == Some(i) { 13.5 } else { 12.0 }),
                pal.text.gamma_multiply(a),
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
}

/// A collapsible section; the collapsed state is stored in the settings
/// (`collapse-filter`, ...) like Obsidian.
fn section(ui: &mut egui::Ui, title: &str, collapsed: &mut bool, body: impl FnOnce(&mut egui::Ui)) {
    let arrow = if *collapsed { "⏵" } else { "⏷" };
    if ui
        .add(
            egui::Label::new(RichText::new(format!("{arrow} {title}")).strong())
                .sense(Sense::click()),
        )
        .clicked()
    {
        *collapsed = !*collapsed;
    }
    if !*collapsed {
        ui.add_space(2.0);
        body(ui);
        ui.add_space(4.0);
    }
    ui.separator();
}

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>) {
    ui.label(label);
    ui.add(egui::Slider::new(v, range).show_value(true));
}

/// Default colours offered for new groups.
const GROUP_COLORS: [[u8; 3]; 6] = [
    [0xe0, 0x6c, 0x75],
    [0x61, 0xaf, 0xef],
    [0xe5, 0xc0, 0x7b],
    [0x98, 0xc3, 0x79],
    [0xc6, 0x78, 0xdd],
    [0x56, 0xb6, 0xc2],
];

fn settings_panel(
    ui: &mut egui::Ui,
    tab: &mut GraphTab,
    env: &Env<'_>,
    rect: egui::Rect,
    pal: &Palette,
) {
    let id = ui.make_persistent_id(("graph-settings", tab.local));
    let mut animate = false;
    egui::Area::new(id)
        .order(egui::Order::Middle)
        .fixed_pos(rect.right_top() + Vec2::new(-250.0, 10.0))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(pal.bg_secondary)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(6)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(220.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} nodes · {} links",
                                tab.data.nodes.len(),
                                tab.data.edges.len()
                            ))
                            .small()
                            .color(pal.faint),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .selectable_label(!tab.settings.close, "⚙")
                                .on_hover_text("Graph settings")
                                .clicked()
                            {
                                tab.settings.close = !tab.settings.close;
                            }
                            if ui.button("⟲").on_hover_text("Fit to view").clicked() {
                                tab.fitted = false;
                            }
                        });
                    });
                    if tab.settings.close {
                        return;
                    }
                    ui.separator();
                    let h = (rect.height() - 70.0).clamp(120.0, 640.0);
                    egui::ScrollArea::vertical()
                        .max_height(h)
                        .min_scrolled_height(h)
                        .show(ui, |ui| {
                            animate = settings_sections(ui, &mut tab.settings);
                        });
                });
        });
    if animate {
        tab.reset_layout(env.index);
    }
}

/// The four sections. Returns true when "Animate" was clicked.
fn settings_sections(ui: &mut egui::Ui, s: &mut crate::graph_model::GraphSettings) -> bool {
    let mut animate = false;
    let local = s.is_local();
    let mut c_filter = s.collapse_filter;
    let mut c_groups = s.collapse_color_groups;
    let mut c_display = s.collapse_display;
    let mut c_forces = s.collapse_forces;
    section(ui, "Filters", &mut c_filter, |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut s.search)
                .hint_text("Search files…")
                .desired_width(f32::INFINITY),
        );
        ui.checkbox(&mut s.show_tags, "Tags");
        ui.checkbox(&mut s.show_attachments, "Attachments");
        ui.checkbox(&mut s.hide_unresolved, "Existing files only");
        ui.add_enabled(!local, egui::Checkbox::new(&mut s.show_orphans, "Orphans"));
        if local {
            let mut d = s.depth();
            ui.horizontal(|ui| {
                ui.label("Depth");
                if ui.add(egui::Slider::new(&mut d, 1..=5)).changed() {
                    s.local_jumps = Some(d);
                }
            });
            for (label, v) in [
                ("Incoming links", &mut s.local_backlinks),
                ("Outgoing links", &mut s.local_forelinks),
                ("Neighbor links", &mut s.local_interlinks),
            ] {
                let mut b = v.unwrap_or(false);
                if ui.checkbox(&mut b, label).changed() {
                    *v = Some(b);
                }
            }
        }
    });
    section(ui, "Groups", &mut c_groups, |ui| {
        let mut remove = None;
        for (i, g) in s.color_groups.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let mut c = g.color.rgb_bytes();
                if ui.color_edit_button_srgb(&mut c).changed() {
                    g.color = GraphColor::from_rgb(c[0], c[1], c[2]);
                }
                ui.add(
                    egui::TextEdit::singleline(&mut g.query)
                        .hint_text("tag:#x, path:dir…")
                        .desired_width(110.0),
                );
                if ui.small_button("✕").on_hover_text("Delete group").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            s.color_groups.remove(i);
        }
        if ui.button("New group").clicked() {
            let c = GROUP_COLORS[s.color_groups.len() % GROUP_COLORS.len()];
            s.color_groups.push(ColorGroup {
                query: String::new(),
                color: GraphColor::from_rgb(c[0], c[1], c[2]),
            });
        }
    });
    section(ui, "Display", &mut c_display, |ui| {
        ui.checkbox(&mut s.show_arrow, "Arrows");
        slider(
            ui,
            "Text fade threshold",
            &mut s.text_fade_multiplier,
            -3.0..=3.0,
        );
        slider(ui, "Node size", &mut s.node_size_multiplier, 0.1..=5.0);
        slider(ui, "Link thickness", &mut s.line_size_multiplier, 0.1..=5.0);
        animate = ui.button("Animate").clicked();
    });
    section(ui, "Forces", &mut c_forces, |ui| {
        slider(ui, "Center force", &mut s.center_strength, 0.0..=1.0);
        slider(ui, "Repel force", &mut s.repel_strength, 0.0..=20.0);
        slider(ui, "Link force", &mut s.link_strength, 0.0..=1.0);
        slider(ui, "Link distance", &mut s.link_distance, 30.0..=500.0);
    });
    s.collapse_filter = c_filter;
    s.collapse_color_groups = c_groups;
    s.collapse_display = c_display;
    s.collapse_forces = c_forces;
    animate
}
