//! Live Preview block decorations painted around the `TextEdit` galley:
//! code block and callout backgrounds, table grids, list indentation guides,
//! callout icons and titles, external-link arrows, language labels.
//!
//! The text stays in the buffer; these are only shapes placed from the
//! positions of the laid-out galley.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Galley, Pos2, Rect, Shape, Stroke, Vec2,
    text::CCursor,
};

use super::editor::styled_font;
use super::theme::Palette;
use crate::editor::blocks::{BlockKind, bars};
use crate::editor::live::{Decor, LineKind, LinkTarget};
use crate::reading::callout_icon;

/// Positions in the galley for (line, byte) pairs.
pub struct Geo<'a> {
    pub galley: &'a Galley,
    pub off: Vec2,
    pub decor: &'a Decor,
    pub text: &'a str,
    line_chars: Vec<usize>,
}

impl<'a> Geo<'a> {
    pub fn new(galley: &'a Galley, off: Vec2, decor: &'a Decor, text: &'a str) -> Self {
        let mut line_chars = Vec::with_capacity(decor.lines.len());
        let mut acc = 0;
        for l in &decor.lines {
            line_chars.push(acc);
            acc += text.get(l.range.clone()).map_or(0, |s| s.chars().count()) + 1;
        }
        Self {
            galley,
            off,
            decor,
            text,
            line_chars,
        }
    }

    /// Rectangle of the char at `byte` of line `n` (screen coordinates).
    pub fn rect_at(&self, n: usize, byte: usize) -> Rect {
        let start = self.decor.lines[n].range.start;
        let c = self.line_chars[n] + self.text.get(start..byte).map_or(0, |s| s.chars().count());
        self.galley
            .pos_from_cursor(CCursor::new(c))
            .translate(self.off)
    }

    /// Top and bottom of line `n`.
    pub fn span(&self, n: usize) -> (f32, f32) {
        let l = &self.decor.lines[n];
        (
            self.rect_at(n, l.range.start).top(),
            self.rect_at(n, l.range.end).bottom(),
        )
    }
}

/// Drawing parameters.
pub struct Look {
    pub pal: Palette,
    pub size: f32,
    pub mono: bool,
    pub styled: bool,
    /// Width of the text column (screen points).
    pub width: f32,
    /// Visible area, to skip off-screen blocks.
    pub clip: Rect,
}

fn visible(look: &Look, y0: f32, y1: f32) -> bool {
    y1 >= look.clip.top() - 300.0 && y0 <= look.clip.bottom() + 300.0
}

/// Capitalised callout type (`warning` -> `Warning`).
pub fn default_title(kind: &str) -> String {
    let mut c = kind.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

/// Shapes drawn *behind* the text: code blocks, callouts, quote bars, table
/// grids.
pub fn backdrop(
    geo: &Geo,
    active: &Range<usize>,
    fold_hidden: &HashSet<usize>,
    look: &Look,
) -> Vec<Shape> {
    let pal = &look.pal;
    let d = geo.decor;
    let x0 = geo.off.x;
    let right = x0 + look.width;
    let mut out = Vec::new();
    // Code blocks.
    for b in &d.blocks {
        match &b.kind {
            BlockKind::Code { .. } => {
                if fold_hidden.contains(&b.start) {
                    continue;
                }
                let raw = active.contains(&b.start) || active.contains(&b.end);
                let (first, last) = if raw {
                    (b.start, b.end)
                } else {
                    (b.start + 1, b.end.saturating_sub(1))
                };
                if first > last || (!raw && b.end <= b.start + 1) {
                    continue;
                }
                let (y0, _) = geo.span(first);
                let (_, y1) = geo.span(last);
                if !visible(look, y0, y1) {
                    continue;
                }
                let pad = if raw { 2.0 } else { 6.0 };
                out.push(Shape::rect_filled(
                    Rect::from_min_max(Pos2::new(x0 - 8.0, y0 - pad), Pos2::new(right, y1 + pad)),
                    6.0,
                    pal.code_bg,
                ));
            }
            BlockKind::Table { .. } => {
                let raw = active.contains(&b.start) || active.contains(&b.end);
                if raw || fold_hidden.contains(&b.start) {
                    continue;
                }
                table_grid(geo, b.start, b.end, look, &mut out);
            }
        }
    }
    // Callouts and quote bars.
    let mut n = 0;
    while n < d.lines.len() {
        match &d.lines[n].kind {
            LineKind::Callout { kind, .. } => {
                let mut end = n;
                while end + 1 < d.lines.len()
                    && matches!(
                        &d.lines[end + 1].kind,
                        LineKind::Callout { header: false, .. }
                    )
                {
                    end += 1;
                }
                // Last line that is not hidden by a fold.
                let mut last = end;
                while last > n && fold_hidden.contains(&last) {
                    last -= 1;
                }
                if !fold_hidden.contains(&n) {
                    let (y0, _) = geo.span(n);
                    let (_, y1) = geo.span(last);
                    if visible(look, y0, y1) {
                        let col = pal.callout(kind);
                        out.push(Shape::rect_filled(
                            Rect::from_min_max(
                                Pos2::new(x0 - 10.0, y0 - 3.0),
                                Pos2::new(right, y1 + 3.0),
                            ),
                            6.0,
                            col.gamma_multiply(0.12),
                        ));
                    }
                }
                n = end + 1;
            }
            LineKind::Quote => {
                if !fold_hidden.contains(&n) {
                    let (y0, y1) = geo.span(n);
                    if visible(look, y0, y1) {
                        out.push(Shape::rect_filled(
                            Rect::from_min_max(Pos2::new(x0 - 10.0, y0), Pos2::new(x0 - 7.0, y1)),
                            1.5,
                            pal.accent.gamma_multiply(0.8),
                        ));
                    }
                }
                n += 1;
            }
            _ => n += 1,
        }
    }
    out
}

fn table_grid(geo: &Geo, start: usize, end: usize, look: &Look, out: &mut Vec<Shape>) {
    let pal = &look.pal;
    let d = geo.decor;
    let hl = &d.lines[start];
    let header_bars = bars(&geo.text[hl.range.clone()], hl.range.start);
    if header_bars.len() < 2 {
        return;
    }
    let bar_x = |n: usize, b: usize| {
        let a = geo.rect_at(n, b);
        let c = geo.rect_at(n, b + 1);
        (a.left() + c.left()) / 2.0
    };
    let xs: Vec<f32> = header_bars.iter().map(|b| bar_x(start, *b)).collect();
    let (y0, _) = geo.span(start);
    let (_, y1) = geo.span(end);
    if !visible(look, y0, y1) {
        return;
    }
    let (left, right) = (xs[0], xs[xs.len() - 1]);
    let (top, bottom) = (y0 - 2.0, y1 + 2.0);
    let frame = Rect::from_min_max(Pos2::new(left - 4.0, top), Pos2::new(right + 4.0, bottom));
    let (_, hy1) = geo.span(start);
    out.push(Shape::rect_filled(
        Rect::from_min_max(frame.min, Pos2::new(frame.max.x, hy1 + 2.0)),
        4.0,
        pal.bg_secondary,
    ));
    let line = Stroke::new(1.0, pal.border.gamma_multiply(1.8));
    out.push(Shape::rect_stroke(
        frame,
        4.0,
        line,
        egui::StrokeKind::Inside,
    ));
    for x in &xs[1..xs.len() - 1] {
        out.push(Shape::line_segment(
            [Pos2::new(*x, top), Pos2::new(*x, bottom)],
            line,
        ));
    }
    for n in start..=end {
        if n == start + 1 {
            continue;
        }
        let (_, by) = geo.span(n);
        if n < end {
            out.push(Shape::line_segment(
                [
                    Pos2::new(left - 4.0, by + 2.0),
                    Pos2::new(right + 4.0, by + 2.0),
                ],
                line,
            ));
        }
    }
}

/// Shapes drawn over the (blank) glyphs: list guides, callout icons and
/// default titles, external-link arrows, code block language labels.
pub fn overlays(
    ui: &egui::Ui,
    geo: &Geo,
    active: &Range<usize>,
    hidden: &HashSet<usize>,
    look: &Look,
) {
    let pal = &look.pal;
    let d = geo.decor;
    let painter = ui.painter();
    let shown = |n: usize| !active.contains(&n) && !hidden.contains(&n);
    // Indentation guides.
    let guide = Stroke::new(1.0, pal.border.gamma_multiply(2.2));
    for g in &d.guides {
        if !shown(g.line) {
            continue;
        }
        let (y0, y1) = geo.span(g.line);
        if !visible(look, y0, y1) {
            continue;
        }
        let a = geo.rect_at(g.line, g.at);
        let b = geo.rect_at(g.line, g.at + 1);
        let x = (a.left() + b.left()) / 2.0;
        painter.line_segment([Pos2::new(x, y0), Pos2::new(x, y1)], guide);
    }
    // Callout headers.
    for (n, l) in d.lines.iter().enumerate() {
        let LineKind::Callout { kind, header: true } = &l.kind else {
            continue;
        };
        if !shown(n) {
            continue;
        }
        let (y0, y1) = geo.span(n);
        if !visible(look, y0, y1) {
            continue;
        }
        let col = pal.callout(kind);
        let r = geo.rect_at(n, l.range.start + indent_of(geo.text, l.range.clone()));
        painter.text(
            Pos2::new(r.left() + 1.0, (y0 + y1) / 2.0),
            Align2::LEFT_CENTER,
            callout_icon(kind),
            FontId::new(look.size * 0.95, FontFamily::Proportional),
            col,
        );
        let src = &geo.text[l.range.clone()];
        let no_title = src
            .find(']')
            .is_some_and(|i| src[i + 1..].trim_matches(['-', '+', ' ']).is_empty());
        if no_title {
            let at = geo.rect_at(n, l.content - tag_len(src));
            painter.text(
                Pos2::new(at.left(), (y0 + y1) / 2.0),
                Align2::LEFT_CENTER,
                default_title(kind),
                styled_font(look.size, look.mono, look.styled, true, false),
                col,
            );
        }
    }
    // Language label and external-link arrows.
    for b in &d.blocks {
        if let BlockKind::Code { lang } = &b.kind
            && !lang.is_empty()
            && !active.contains(&b.start)
            && !active.contains(&b.end)
            && b.end > b.start + 1
            && !hidden.contains(&b.start)
            && !hidden.contains(&(b.start + 1))
        {
            let (y0, _) = geo.span(b.start + 1);
            if visible(look, y0, y0) {
                painter.text(
                    Pos2::new(geo.off.x + look.width - 8.0, y0 - 3.0),
                    Align2::RIGHT_TOP,
                    lang,
                    FontId::new(look.size * 0.75, FontFamily::Proportional),
                    pal.muted,
                );
            }
        }
    }
    for link in &d.links {
        let LinkTarget::Markdown(dest) = &link.target else {
            continue;
        };
        if !dest.contains("://") {
            continue;
        }
        let n = d.lines.partition_point(|l| l.range.end < link.range.start);
        if n >= d.lines.len() || !shown(n) {
            continue;
        }
        let at = link.range.end.saturating_sub(dest.len() + 3);
        let r = geo.rect_at(n, at);
        if !visible(look, r.top(), r.bottom()) {
            continue;
        }
        painter.text(
            Pos2::new(r.left() + 1.0, r.center().y),
            Align2::LEFT_CENTER,
            "↗",
            FontId::new(look.size * 0.8, FontFamily::Proportional),
            pal.accent,
        );
    }
}

/// Bullet look by nesting depth: filled disc, ring, small square.
pub fn paint_bullet(painter: &egui::Painter, c: Pos2, r: f32, depth: usize, color: Color32) {
    match depth % 3 {
        0 => {
            painter.circle_filled(c, r, color);
        }
        1 => {
            painter.circle_stroke(c, r, Stroke::new(1.2, color));
        }
        _ => {
            painter.rect_filled(Rect::from_center_size(c, Vec2::splat(r * 1.6)), 1.0, color);
        }
    }
}

/// Depth of each list item line.
pub fn depth_map(d: &Decor) -> HashMap<usize, usize> {
    d.depths.iter().copied().collect()
}

fn indent_of(text: &str, line: Range<usize>) -> usize {
    let s = &text[line];
    s.len() - s.trim_start().len()
}

/// Length of the `[!kind]` + marker part of a callout header (from `[`).
fn tag_len(line: &str) -> usize {
    line.find(']').map_or(0, |i| {
        let mut e = i + 1;
        if matches!(line.as_bytes().get(e), Some(b'-' | b'+')) {
            e += 1;
        }
        let start = line.find("[!").unwrap_or(0);
        e - start + usize::from(line[e..].starts_with(' '))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_tag_lengths() {
        assert_eq!(default_title("warning"), "Warning");
        assert_eq!(default_title(""), "");
        assert_eq!(tag_len("> [!tip]"), 6);
        assert_eq!(tag_len("> [!tip]- "), 8);
    }
}
