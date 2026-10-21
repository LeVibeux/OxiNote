//! Drawing helpers of the editor that sit around the text: line-number
//! gutter, fold arrows, folded-block marker and inline images. Geometry is
//! derived from the galley rows so that everything stays aligned with the
//! `TextEdit` whatever the wrapping or the font sizes.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Galley, Pos2, Rect, Sense, Stroke};
use oxi_core::{NotePath, VaultRead};

use super::theme::Palette;
use crate::backend::IndexBackend;
use crate::editor::embeds::{self, ImageRef};
use crate::editor::fold::FoldRange;
use crate::state::TabId;

/// Space kept above and below an inline image.
pub const IMAGE_PAD: f32 = 4.0;
/// Space reserved left of the text for the fold arrows.
pub const ARROW_GUTTER: f32 = 30.0;

/// Where a logical line sits in the galley (galley coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineRow {
    /// Top of its first row.
    pub top: f32,
    /// Height of its first row.
    pub height: f32,
    /// Bottom of its last row.
    pub bottom: f32,
    /// Right end of its last row.
    pub right: f32,
}

/// One entry per logical line (`\n`-separated), from the galley rows.
pub fn line_rows(galley: &Galley) -> Vec<LineRow> {
    let mut out: Vec<LineRow> = Vec::new();
    let mut new_line = true;
    for row in &galley.rows {
        let (top, bottom) = (row.pos.y, row.pos.y + row.size.y);
        let right = row.pos.x + row.size.x;
        if new_line {
            out.push(LineRow {
                top,
                height: row.size.y,
                bottom,
                right,
            });
        } else if let Some(l) = out.last_mut() {
            l.bottom = bottom;
            l.right = right;
        }
        new_line = row.ends_with_newline;
    }
    out
}

/// Width taken by the line numbers for `lines` lines.
pub fn number_gutter_width(lines: usize, size: f32) -> f32 {
    let digits = lines.max(10).to_string().len() as f32;
    digits * size * 0.6 + 10.0
}

/// Line numbers right-aligned in the margin, the caret line highlighted.
#[allow(clippy::too_many_arguments)]
pub fn paint_line_numbers(
    ui: &egui::Ui,
    rows: &[LineRow],
    origin: Pos2,
    hidden: &HashSet<usize>,
    caret_line: Option<usize>,
    size: f32,
    pal: &Palette,
) {
    let clip = ui.clip_rect();
    let painter = ui.painter();
    let font = FontId::new((size * 0.8).max(9.0), FontFamily::Proportional);
    let right = origin.x - ARROW_GUTTER;
    for (n, r) in rows.iter().enumerate() {
        let (y0, y1) = (origin.y + r.top, origin.y + r.bottom);
        if y1 < clip.top() - 50.0 {
            continue;
        }
        if y0 > clip.bottom() + 50.0 {
            break;
        }
        if hidden.contains(&n) {
            continue;
        }
        // Centered on the first row, but kept near the top of tall rows
        // (images).
        let cy = y0 + r.height.min(size * 1.7) / 2.0;
        let col = if caret_line == Some(n) {
            pal.text
        } else {
            pal.faint
        };
        painter.text(
            Pos2::new(right, cy),
            Align2::RIGHT_CENTER,
            (n + 1).to_string(),
            font.clone(),
            col,
        );
    }
}

/// Fold arrow (pointing right when folded, down otherwise) centered on `c`.
fn paint_arrow(ui: &egui::Ui, c: Pos2, folded: bool, color: Color32) {
    let s = 4.0;
    let pts = if folded {
        vec![
            Pos2::new(c.x - s * 0.6, c.y - s),
            Pos2::new(c.x + s * 0.9, c.y),
            Pos2::new(c.x - s * 0.6, c.y + s),
        ]
    } else {
        vec![
            Pos2::new(c.x - s, c.y - s * 0.5),
            Pos2::new(c.x + s, c.y - s * 0.5),
            Pos2::new(c.x, c.y + s * 0.8),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Fold arrows in the margin (shown for the hovered line and for folded
/// blocks) and the `…` after a folded header. Returns the start line of a
/// block to toggle.
#[allow(clippy::too_many_arguments)]
pub fn fold_controls(
    ui: &mut egui::Ui,
    tab: TabId,
    folds: &[FoldRange],
    folded: &[usize],
    hidden: &HashSet<usize>,
    rows: &[LineRow],
    origin: Pos2,
    width: f32,
    pal: &Palette,
) -> Option<usize> {
    let clip = ui.clip_rect();
    let hover = ui.input(|i| i.pointer.hover_pos());
    let mut toggled = None;
    for (i, f) in folds.iter().enumerate() {
        let Some(r) = rows.get(f.start).filter(|_| !hidden.contains(&f.start)) else {
            continue;
        };
        let (y0, y1) = (origin.y + r.top, origin.y + r.bottom);
        if y1 < clip.top() || y0 > clip.bottom() {
            continue;
        }
        let is_folded = folded.contains(&i);
        let over = hover.is_some_and(|p| {
            p.y >= y0
                && p.y <= y1
                && p.x >= origin.x - ARROW_GUTTER - 14.0
                && p.x <= origin.x + width
        });
        if !(over || is_folded) {
            continue;
        }
        let c = Pos2::new(origin.x - 19.0, y0 + r.height.min(26.0) / 2.0);
        let area = Rect::from_center_size(c, egui::vec2(18.0, 18.0));
        let resp = ui.interact(
            area,
            egui::Id::unique(("fold-arrow", tab, f.start)),
            Sense::click(),
        );
        let col = if resp.hovered() { pal.text } else { pal.muted };
        paint_arrow(ui, c, is_folded, if over { col } else { pal.faint });
        if resp.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            toggled = Some(f.start);
        }
        if is_folded {
            // "…" after the header line.
            let x = origin.x + r.right + 8.0;
            let rect = Rect::from_min_max(
                Pos2::new(x, y0 + r.height * 0.18),
                Pos2::new(x + 26.0, y0 + r.height * 0.82),
            );
            let dots = ui.interact(
                rect,
                egui::Id::unique(("fold-dots", tab, f.start)),
                Sense::click(),
            );
            ui.painter().rect_filled(
                rect,
                4.0,
                if dots.hovered() {
                    pal.accent_soft
                } else {
                    pal.tag_bg
                },
            );
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "…",
                FontId::proportional((r.height * 0.6).clamp(10.0, 18.0)),
                pal.muted,
            );
            if dots.clicked() {
                toggled = Some(f.start);
            }
        }
    }
    toggled
}

// ---------------------------------------------------------------------------
// Inline images
// ---------------------------------------------------------------------------

/// An image reserved under a line at the last layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub line: usize,
    pub path: NotePath,
    pub size: egui::Vec2,
}

/// Resolved paths and pixel sizes of the embedded images (valid for one
/// vault generation).
#[derive(Default)]
pub struct EmbedCache {
    generation: u64,
    resolved: HashMap<(NotePath, String), Option<NotePath>>,
    dims: HashMap<NotePath, Option<(u32, u32)>>,
    /// Images laid out for each tab (read when painting).
    pub placed: HashMap<TabId, Vec<Placed>>,
}

/// Pixel size of an encoded image without decoding it.
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

impl EmbedCache {
    pub fn sync(&mut self, generation: u64) {
        if self.generation != generation {
            self.generation = generation;
            self.resolved.clear();
            self.dims.clear();
        }
    }

    fn resolve(
        &mut self,
        index: &dyn IndexBackend,
        from: &NotePath,
        target: &str,
    ) -> Option<NotePath> {
        self.resolved
            .entry((from.clone(), target.to_string()))
            .or_insert_with(|| {
                index.resolve(target, from).filter(|p| {
                    embeds::IMAGE_EXTS.contains(&p.extension().to_ascii_lowercase().as_str())
                })
            })
            .clone()
    }

    fn dims(&mut self, vault: &dyn VaultRead, path: &NotePath) -> Option<(u32, u32)> {
        *self.dims.entry(path.clone()).or_insert_with(|| {
            vault
                .read_bytes(path)
                .ok()
                .and_then(|b| image_dimensions(&b))
        })
    }

    /// The images of the inactive lines of `images`, sized for a column
    /// `max_w` wide; remembered for the painting pass.
    #[allow(clippy::too_many_arguments)]
    pub fn layout(
        &mut self,
        tab: TabId,
        index: &dyn IndexBackend,
        vault: &dyn VaultRead,
        from: &NotePath,
        images: &[(usize, ImageRef)],
        skip: impl Fn(usize) -> bool,
        max_w: f32,
    ) -> Vec<Placed> {
        let mut out = Vec::new();
        for (line, img) in images {
            if skip(*line) {
                continue;
            }
            let Some(path) = self.resolve(index, from, &img.target) else {
                continue;
            };
            let Some(px) = self.dims(vault, &path) else {
                continue;
            };
            let (w, h) = embeds::display_size(px, img, max_w);
            out.push(Placed {
                line: *line,
                path,
                size: egui::vec2(w, h),
            });
        }
        self.placed.insert(tab, out.clone());
        out
    }
}

/// Row height reserved for an image of height `h`.
pub fn image_row_height(h: f32) -> f32 {
    h + 2.0 * IMAGE_PAD
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Context, text::LayoutJob};

    fn galley_of(text: &str, width: f32) -> std::sync::Arc<Galley> {
        let ctx = Context::default();
        let mut out = None;
        let mut o = ctx.run_ui(Default::default(), |ui| {
            let mut job = LayoutJob::simple(
                text.to_string(),
                FontId::proportional(14.0),
                Color32::WHITE,
                width,
            );
            job.wrap.max_width = width;
            out = Some(ui.fonts_mut(|f| f.layout_job(job)));
        });
        o.textures_delta.clear();
        out.expect("galley")
    }

    #[test]
    fn rows_follow_logical_lines_even_when_wrapped() {
        let long = "mot ".repeat(60);
        let t = format!("a\n{long}\n\nz");
        let g = galley_of(&t, 120.0);
        let rows = line_rows(&g);
        assert_eq!(rows.len(), 4);
        assert!(g.rows.len() > 4, "the long line wraps");
        // Increasing, and the wrapped line spans several rows.
        assert!(rows.windows(2).all(|w| w[0].top < w[1].top));
        assert!(rows[1].bottom - rows[1].top > rows[1].height * 1.5);
        // A trailing newline adds an empty last line.
        assert_eq!(line_rows(&galley_of("a\nb\n", 200.0)).len(), 3);
    }

    #[test]
    fn gutter_grows_with_digits() {
        assert!(number_gutter_width(5, 15.0) < number_gutter_width(5000, 15.0));
    }

    #[test]
    fn reads_png_dimensions() {
        let mut png = Vec::new();
        image::RgbaImage::new(7, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode");
        assert_eq!(image_dimensions(&png), Some((7, 3)));
        assert_eq!(image_dimensions(b"pas une image"), None);
    }
}
