//! Note views: source editor (with highlighting and `[[` completion),
//! reading view, and attachment preview.

use eframe::egui::{
    self, Color32, FontFamily, FontId, Frame, Margin, RichText, Stroke, TextFormat, text::LayoutJob,
};
use ors_core::NotePath;

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::highlight::{Style, highlight};
use crate::state::AppState;

const READABLE_WIDTH: f32 = 760.0;
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

pub fn is_image(path: &NotePath) -> bool {
    IMAGE_EXTS.contains(&path.extension().as_str())
}

/// Lay out `add` in a centered column of readable width.
pub fn readable_column<R>(
    ui: &mut egui::Ui,
    readable: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    readable_column_with(ui, readable, 0.0, add)
}

/// Like [`readable_column`], keeping `left_extra` points free on the left
/// of the text (line numbers, fold arrows).
pub fn readable_column_with<R>(
    ui: &mut egui::Ui,
    readable: bool,
    left_extra: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let avail = ui.available_width() - left_extra;
    let width = if readable {
        (avail - 32.0).min(READABLE_WIDTH)
    } else {
        avail - 32.0
    }
    .max(100.0);
    let pad = ((avail - width) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad + left_extra);
        ui.vertical(|ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner
    })
    .inner
}

// ---------------------------------------------------------------------------
// Source mode
// ---------------------------------------------------------------------------

pub fn editor_font(size: f32, mono: bool) -> FontId {
    FontId::new(
        size,
        if mono {
            FontFamily::Monospace
        } else {
            FontFamily::Proportional
        },
    )
}

/// Build the highlighted layout job for the source editor.
pub fn layout_job(text: &str, size: f32, mono: bool, styled: bool, pal: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, style) in highlight(text) {
        job.append(
            &text[range],
            0.0,
            format_for_mode(&style, size, mono, styled, pal, false),
        );
    }
    job
}

/// Font for text that is `bold` and/or `italic`: the real Inter faces when
/// `styled` (families installed) and the editor is proportional.
pub fn styled_font(size: f32, mono: bool, styled: bool, bold: bool, italic: bool) -> FontId {
    if mono || !styled || !(bold || italic) {
        return editor_font(size, mono);
    }
    let name = match (bold, italic) {
        (true, true) => super::theme::BOLD_ITALIC_FAMILY,
        (true, false) => super::theme::BOLD_FAMILY,
        _ => super::theme::ITALIC_FAMILY,
    };
    FontId::new(size, FontFamily::Name(name.into()))
}

/// Size factor of a heading line. Source mode never enlarges headings.
pub fn heading_scale(level: u8, enlarge: bool) -> f32 {
    if !enlarge {
        return 1.0;
    }
    match level {
        1 => 1.6,
        2 => 1.4,
        3 => 1.25,
        4 => 1.12,
        5 | 6 => 1.05,
        _ => 1.0,
    }
}

/// Format of a highlighted span. `live`: Live Preview (enlarged headings,
/// monospace frontmatter); otherwise Source mode (raw markdown in the editor
/// font, only colours and weights change).
pub(super) fn format_for_mode(
    s: &Style,
    size: f32,
    mono: bool,
    styled: bool,
    pal: &Palette,
    live: bool,
) -> TextFormat {
    let scale = heading_scale(s.heading, live);
    let mut fmt = TextFormat {
        font_id: styled_font(
            size * scale,
            mono,
            styled,
            s.heading > 0 || s.bold,
            s.italic,
        ),
        color: pal.text,
        ..TextFormat::default()
    };
    if s.heading > 0 || s.bold {
        fmt.color = pal.strong;
    }
    if s.italic && (mono || !styled) {
        fmt.italics = true;
    }
    if s.strike {
        fmt.strikethrough = Stroke::new(1.0, pal.muted);
    }
    if s.link {
        fmt.color = pal.accent;
    }
    if s.tag {
        fmt.color = pal.accent;
        fmt.background = pal.tag_bg;
    }
    if s.highlight {
        fmt.background = pal.highlight_bg;
    }
    if s.quote {
        fmt.color = pal.muted;
    }
    if s.code {
        fmt.font_id = FontId::new(size * scale * 0.92, FontFamily::Monospace);
        fmt.color = pal.code_fg;
        fmt.background = pal.code_bg;
    }
    if s.syntax {
        fmt.color = pal.faint;
    }
    if s.frontmatter || s.comment {
        fmt.color = pal.faint;
        fmt.font_id = if live {
            FontId::new(size * 0.92, FontFamily::Monospace)
        } else {
            editor_font(size, mono)
        };
    }
    fmt
}

// ---------------------------------------------------------------------------
// Attachments
// ---------------------------------------------------------------------------

pub fn file_view(ui: &mut egui::Ui, state: &AppState, caches: &mut UiCaches, path: &NotePath) {
    let pal = Palette::of(ui.ctx());
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(path.file_name()).size(18.0).color(pal.strong));
        ui.add_space(8.0);
        if is_image(path) {
            match caches.image_bytes(state, path) {
                Some(bytes) => {
                    ui.add(
                        egui::Image::from_bytes(format!("bytes://{}", path.as_str()), bytes)
                            .max_width(ui.available_width() - 32.0)
                            .corner_radius(4),
                    );
                }
                None => {
                    ui.label(RichText::new("Cannot load this image.").color(pal.unresolved));
                }
            }
        } else {
            ui.label(RichText::new("No preview available for this file type.").color(pal.muted));
        }
    });
}

/// Banner shown when the file changed on disk while being edited.
pub fn conflict_banner(ui: &mut egui::Ui, path: &NotePath, actions: &mut Vec<Action>) {
    let color = Color32::from_rgb(0xec, 0x75, 0x00);
    Frame::new()
        .fill(color.gamma_multiply(0.15))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠ This file was modified outside ObsiRust.").color(color));
                if ui.button("Reload from disk").clicked() {
                    actions.push(Action::Resolve(path.clone(), false));
                }
                if ui.button("Keep my version").clicked() {
                    actions.push(Action::Resolve(path.clone(), true));
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_job_covers_text() {
        let text = "# Title\nsome **bold** [[link]] #tag";
        let job = layout_job(text, 14.0, false, false, &Palette::dark());
        assert_eq!(job.text, text);
        assert!(job.sections.len() > 4);
    }
}
