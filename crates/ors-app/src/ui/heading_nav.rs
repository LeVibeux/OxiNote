//! Heading navigator: a compact stack of ticks on the right edge of a note
//! panel (Live Preview, Source and Reading), one per heading.
//!
//! * Floating outline (A): hovering the ticks opens a card listing the
//!   headings.
//! * Magnifier (B): the ticks near the pointer grow like the macOS Dock and
//!   show their titles.
//!
//! The layout maths live in [`crate::editor::heading_nav`]; this file draws
//! and handles the pointer.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Margin, Pos2, Rect, Sense, Shadow, Stroke, Vec2,
};
use ors_core::NotePath;

use super::editor::styled_font;
use super::theme::{Palette, has_styled_fonts};
use crate::editor::DisplayMode;
use crate::editor::heading_nav::{
    self as model, NavHeading, STRIP_WIDTH, Tick, card_indent, current, dash_len, hit,
    loupe_layout, pitch, rest_layout,
};
use crate::settings::HeadingNavigator;
use crate::state::{AppState, TabId};

/// Distance between the panel's right edge and the ticks' right edge.
const EDGE: f32 = 14.0;

/// State shared between the note views and the navigator.
#[derive(Default)]
pub struct NavCache {
    heads: HashMap<TabId, (u64, Rc<Vec<NavHeading>>)>,
    /// Screen `y` of every heading drawn this frame (indexed like the
    /// headings), reported by the edit and reading views.
    pub ys: Option<Vec<Option<f32>>>,
    /// Line of the heading to scroll to (and put the caret on) in the editor.
    pub jump: Option<(TabId, usize)>,
    /// Scroll offset of the last frame of each tab.
    pub offsets: HashMap<TabId, f32>,
    /// Pointer position over the strip, remembered for the fade-out.
    last_hy: f32,
    /// Is the floating card open, and where it was drawn.
    pub card_open: bool,
    card_rect: Option<Rect>,
    /// Force the hover at this fraction of the strip (screenshots).
    pub force: Option<f32>,
}

impl NavCache {
    /// The headings of the note of `tab` (parsed again when the text changed).
    pub fn headings(
        &mut self,
        tab: TabId,
        state: &AppState,
        path: &NotePath,
    ) -> Rc<Vec<NavHeading>> {
        let text = state.note_text(path).unwrap_or_default();
        let mut h = DefaultHasher::new();
        text.hash(&mut h);
        let key = h.finish();
        if let Some((k, v)) = self.heads.get(&tab)
            && *k == key
        {
            return v.clone();
        }
        let parsed = state.parsed_with_text(path, &text);
        let v = Rc::new(NavHeading::from_parsed(&parsed.headings));
        self.heads.insert(tab, (key, v.clone()));
        v
    }

    /// Headings of `tab` as last computed.
    pub fn cached(&self, tab: TabId) -> Option<Rc<Vec<NavHeading>>> {
        self.heads.get(&tab).map(|(_, v)| v.clone())
    }

    /// The pending editor jump for `tab` (a line).
    pub fn take_jump(&mut self, tab: TabId) -> Option<usize> {
        match self.jump {
            Some((t, line)) if t == tab => {
                self.jump = None;
                Some(line)
            }
            _ => None,
        }
    }
}

/// Geometry of the strip in a panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strip {
    /// Top of the ticks at rest.
    pub top: f32,
    pub pitch: f32,
    /// Right edge of the ticks.
    pub right: f32,
    /// Area that keeps the strip "hovered".
    pub zone: Rect,
}

/// Where the strip of `n` headings sits in `panel`: centred vertically.
pub fn strip(panel: Rect, n: usize) -> Strip {
    let p = pitch(n, panel.height());
    let h = n as f32 * p;
    let top = panel.center().y - h / 2.0;
    let right = panel.right() - EDGE;
    Strip {
        top,
        pitch: p,
        right,
        zone: Rect::from_min_max(
            Pos2::new(right - STRIP_WIDTH + 6.0, top - 6.0),
            Pos2::new(right + 8.0, top + h + 6.0),
        ),
    }
}

/// Id of the strip's interaction area.
pub fn strip_id(tab: TabId) -> egui::Id {
    egui::Id::unique(("heading-nav", tab))
}

fn lighter(c: Color32) -> Color32 {
    c.lerp_to_gamma(Color32::WHITE, 0.3)
}

/// Draw the navigator over the panel `panel` (the scroll area's viewport) and
/// handle its input. `ys` were reported by the view during this frame.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut super::UiCaches,
    tab_id: TabId,
    tab_index: usize,
    mode: DisplayMode,
    panel: Rect,
    offset: f32,
    heads: &[NavHeading],
) {
    let kind = state.settings.heading_navigator;
    caches.nav.offsets.insert(tab_id, offset);
    if kind == HeadingNavigator::Off || heads.is_empty() || panel.height() < 80.0 {
        caches.nav.card_open = false;
        return;
    }
    let ys = caches.nav.ys.take().unwrap_or_default();
    let hs: Vec<NavHeading> = heads
        .iter()
        .enumerate()
        .map(|(i, h)| NavHeading {
            y: ys
                .get(i)
                .copied()
                .flatten()
                .map(|y| y - panel.top() + offset),
            ..h.clone()
        })
        .collect();
    let cur = current(&hs, offset);
    let pal = Palette::of(ui.ctx());
    let n = hs.len();
    let st = strip(panel, n);
    let resp = ui.interact(st.zone, strip_id(tab_id), Sense::click());
    let forced = caches
        .nav
        .force
        .map(|f| Pos2::new(st.zone.center().x, st.top + f * n as f32 * st.pitch));
    let pointer = forced.or_else(|| ui.input(|i| i.pointer.hover_pos()));
    let over_strip = forced.is_some() || resp.hovered();
    let over_card = kind == HeadingNavigator::Floating
        && caches
            .nav
            .card_rect
            .zip(pointer)
            .is_some_and(|(r, p)| r.contains(p));
    let hovered = over_strip || over_card;
    if over_strip {
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
    }

    let mut target: Option<usize> = None;
    match kind {
        HeadingNavigator::Off => {}
        HeadingNavigator::Floating => {
            let ticks = rest_layout(n, st.pitch);
            paint_ticks(ui, &hs, &ticks, &st, cur, None, &pal);
            if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                target = hit(&ticks, p.y - st.top);
            }
            caches.nav.card_open = hovered;
            if hovered {
                let just_opened = caches.nav.card_rect.is_none();
                let card = card(ui, &hs, cur, &st, panel, just_opened, &pal);
                caches.nav.card_rect = Some(card.0);
                target = target.or(card.1);
            } else {
                caches.nav.card_rect = None;
            }
        }
        HeadingNavigator::Loupe => {
            caches.nav.card_open = false;
            caches.nav.card_rect = None;
            if over_strip && let Some(p) = pointer {
                caches.nav.last_hy = p.y - st.top;
            }
            let amount = ui.ctx().animate_bool_with_time(
                egui::Id::unique(("heading-nav-amount", tab_id)),
                over_strip,
                0.12,
            );
            let hy = caches.nav.last_hy;
            let ticks = loupe_layout(n, st.pitch, hy, amount);
            let near = (amount > 0.0).then(|| model::focus(n, st.pitch, hy).round() as usize);
            paint_ticks(ui, &hs, &ticks, &st, cur, near, &pal);
            if amount > 0.0 {
                paint_labels(ui, &hs, &ticks, &st, cur, near, panel, &pal);
            }
            if amount > 0.0 && amount < 1.0 {
                ui.ctx().request_repaint();
            }
            if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                target = hit(&ticks, p.y - st.top);
            }
        }
    }

    if let Some(i) = target.filter(|i| *i < n) {
        jump(state, caches, tab_id, tab_index, mode, hs[i].line);
        ui.ctx().request_repaint();
    }
}

/// Scroll the note to the heading at `line`.
fn jump(
    state: &mut AppState,
    caches: &mut super::UiCaches,
    tab_id: TabId,
    tab_index: usize,
    mode: DisplayMode,
    line: usize,
) {
    if mode == DisplayMode::Reading {
        if let Some(n) = state.tabs.get_mut(tab_index).and_then(|t| t.note_mut()) {
            n.pending_line = Some(line);
        }
    } else {
        caches.nav.jump = Some((tab_id, line));
    }
}

fn paint_ticks(
    ui: &egui::Ui,
    hs: &[NavHeading],
    ticks: &[Tick],
    st: &Strip,
    cur: Option<usize>,
    near: Option<usize>,
    pal: &Palette,
) {
    let painter = ui.painter();
    for (i, (h, t)) in hs.iter().zip(ticks).enumerate() {
        let w = dash_len(h.level) * (1.0 + 1.1 * t.g);
        let thick = if t.g > 0.6 {
            model::THICK_ZOOM
        } else {
            model::THICK
        };
        let y = st.top + t.center();
        let color = if Some(i) == cur {
            pal.accent
        } else if Some(i) == near {
            pal.strong
        } else {
            pal.faint
        };
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(st.right - w, y - thick / 2.0),
                Pos2::new(st.right, y + thick / 2.0),
            ),
            1.0,
            color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_labels(
    ui: &egui::Ui,
    hs: &[NavHeading],
    ticks: &[Tick],
    st: &Strip,
    cur: Option<usize>,
    near: Option<usize>,
    panel: Rect,
    pal: &Palette,
) {
    let painter = ui.painter();
    let styled = has_styled_fonts(ui.ctx());
    for (i, (h, t)) in hs.iter().zip(ticks).enumerate() {
        if t.g <= 0.08 || h.text.is_empty() {
            continue;
        }
        let opacity = (0.2 + t.g).min(1.0);
        let size = 10.0 + 4.0 * t.g;
        let font = if Some(i) == near {
            styled_font(size, false, styled, true, false)
        } else {
            FontId::proportional(size)
        };
        let color = if Some(i) == cur {
            lighter(pal.accent)
        } else if Some(i) == near {
            pal.strong
        } else {
            pal.text
        };
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(h.text.clone(), font, color));
        let w = dash_len(h.level) * (1.0 + 1.1 * t.g);
        let right = st.right - w - 10.0;
        let y = st.top + t.center();
        let size_g = galley.size();
        let rect = Rect::from_min_max(
            Pos2::new(
                (right - size_g.x - 14.0).max(panel.left() + 4.0),
                y - size_g.y / 2.0 - 2.0,
            ),
            Pos2::new(right, y + size_g.y / 2.0 + 2.0),
        );
        painter.rect_filled(rect, 5.0, pal.bg_secondary.gamma_multiply(0.94 * opacity));
        let clipped = painter.with_clip_rect(rect.shrink2(Vec2::new(3.0, 0.0)));
        clipped.galley_with_override_text_color(
            Pos2::new(
                rect.right() - 7.0 - size_g.x,
                rect.center().y - size_g.y / 2.0,
            ),
            galley,
            color.gamma_multiply(opacity),
        );
    }
}

/// Truncate `text` with an ellipsis so that it fits `max_w`.
fn fit(ui: &egui::Ui, text: &str, font: &FontId, max_w: f32) -> String {
    let width = |s: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if width(text) <= max_w {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && width(&format!("{s}…")) > max_w {
        s.pop();
    }
    format!("{s}…")
}

/// The floating card; returns its rectangle and the clicked heading.
fn card(
    ui: &mut egui::Ui,
    hs: &[NavHeading],
    cur: Option<usize>,
    st: &Strip,
    panel: Rect,
    just_opened: bool,
    pal: &Palette,
) -> (Rect, Option<usize>) {
    let mut clicked = None;
    let styled = has_styled_fonts(ui.ctx());
    let max_h = (panel.height() - 24.0).clamp(80.0, 400.0);
    let out = egui::Area::new(egui::Id::unique("heading-card"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(st.zone.left(), panel.center().y))
        .pivot(Align2::RIGHT_CENTER)
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(pal.bg_secondary)
                .stroke(Stroke::new(1.0, pal.border.gamma_multiply(1.4)))
                .corner_radius(10)
                .shadow(Shadow {
                    offset: [0, 10],
                    blur: 30,
                    spread: 0,
                    color: Color32::from_black_alpha(115),
                })
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(234.0);
                    egui::ScrollArea::vertical()
                        .max_height(max_h - 16.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for (i, h) in hs.iter().enumerate() {
                                let (rect, r) =
                                    ui.allocate_exact_size(Vec2::new(234.0, 26.0), Sense::click());
                                if r.hovered() {
                                    ui.painter().rect_filled(rect, 6.0, pal.border);
                                }
                                let is_cur = Some(i) == cur;
                                let color = if is_cur {
                                    lighter(pal.accent)
                                } else if h.level == 1 {
                                    pal.strong
                                } else {
                                    pal.text
                                };
                                let font = if is_cur || h.level == 1 {
                                    styled_font(13.0, false, styled, true, false)
                                } else {
                                    FontId::proportional(13.0)
                                };
                                let pad = card_indent(h.level);
                                let label = fit(ui, &h.text, &font, 234.0 - pad - 8.0);
                                ui.painter().text(
                                    Pos2::new(rect.left() + pad, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    label,
                                    font,
                                    color,
                                );
                                if is_cur && just_opened {
                                    r.scroll_to_me(Some(Align::Center));
                                }
                                if r.clicked() {
                                    clicked = Some(i);
                                }
                            }
                        });
                });
        });
    (out.response.rect, clicked)
}
