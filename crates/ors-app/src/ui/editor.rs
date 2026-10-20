//! Note views: source editor (with highlighting and `[[` completion),
//! reading view, and attachment preview.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Frame, Key, Margin, Modifiers, RichText, Stroke,
    TextFormat,
    text::{CCursor, CCursorRange, LayoutJob},
};
use ors_core::NotePath;

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::highlight::{Style, highlight};
use crate::state::{AppState, TabId};
use crate::util::line_start_byte;

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
    let avail = ui.available_width();
    let width = if readable {
        avail.min(READABLE_WIDTH)
    } else {
        avail - 32.0
    }
    .max(100.0);
    let pad = ((avail - width) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
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
pub fn layout_job(text: &str, size: f32, mono: bool, pal: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, style) in highlight(text) {
        job.append(&text[range], 0.0, format_for(&style, size, mono, pal));
    }
    job
}

fn format_for(s: &Style, size: f32, mono: bool, pal: &Palette) -> TextFormat {
    let scale = match s.heading {
        1 => 1.6,
        2 => 1.4,
        3 => 1.25,
        4 => 1.12,
        5 | 6 => 1.05,
        _ => 1.0,
    };
    let mut fmt = TextFormat {
        font_id: editor_font(size * scale, mono),
        color: pal.text,
        ..TextFormat::default()
    };
    if s.heading > 0 || s.bold {
        fmt.color = pal.strong;
    }
    if s.italic {
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
        fmt.font_id = FontId::new(size * 0.92, FontFamily::Monospace);
    }
    fmt
}

/// `[[` link completion popup state.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub tab: TabId,
    /// Char index right after `[[`.
    pub start: usize,
    pub query: String,
    pub selected: usize,
}

/// Text between an unclosed `[[` and the cursor, if any.
pub fn completion_query(text: &str, cursor_char: usize) -> Option<(usize, String)> {
    let byte = text
        .char_indices()
        .nth(cursor_char)
        .map_or(text.len(), |(b, _)| b);
    let before = &text[..byte];
    let open = before.rfind("[[")?;
    let query = &before[open + 2..];
    if query.contains(['\n', ']', '|', '#']) || query.len() > 80 {
        return None;
    }
    Some((before[..open + 2].chars().count(), query.to_string()))
}

pub fn source_view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    tab_id: TabId,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let size = state.settings.font_size;
    let mono = state.settings.monospace_editor;
    let readable = state.settings.readable_line_length;
    let pending_line = state
        .active_tab_mut()
        .and_then(|t| t.note_mut())
        .and_then(|n| n.pending_line.take());
    let suggestions: Vec<(NotePath, Option<String>, i64)> = match &caches.completion {
        Some(c) if c.tab == tab_id => state.index.fuzzy_find(&c.query, 8),
        _ => Vec::new(),
    };
    // Keyboard handling for the completion popup must happen before the
    // TextEdit sees the keys.
    let mut accept: Option<NotePath> = None;
    if let Some(c) = caches.completion.as_mut().filter(|c| c.tab == tab_id) {
        if suggestions.is_empty() {
            c.selected = 0;
        } else {
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    c.selected = (c.selected + 1) % suggestions.len();
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    c.selected = (c.selected + suggestions.len() - 1) % suggestions.len();
                }
                if i.consume_key(Modifiers::NONE, Key::Enter)
                    || i.consume_key(Modifiers::NONE, Key::Tab)
                {
                    accept = suggestions
                        .get(c.selected.min(suggestions.len() - 1))
                        .map(|s| s.0.clone());
                }
            });
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            caches.completion = None;
        }
    }

    let Some(buffer) = state.buffers.get_mut(path) else {
        ui.label("Cannot read this file.");
        return;
    };
    let id = egui::Id::unique(("editor", tab_id));

    // Apply an accepted completion.
    if let (Some(target), Some(c)) = (accept, caches.completion.take()) {
        let link = state.index.link_text_for(&target, path);
        let start_b = buffer
            .text
            .char_indices()
            .nth(c.start)
            .map_or(buffer.text.len(), |(b, _)| b);
        let end_char = c.start + c.query.chars().count();
        let end_b = buffer
            .text
            .char_indices()
            .nth(end_char)
            .map_or(buffer.text.len(), |(b, _)| b);
        let closing = if buffer.text[end_b..].starts_with("]]") {
            ""
        } else {
            "]]"
        };
        let insert = format!("{link}{closing}");
        buffer.text.replace_range(start_b..end_b, &insert);
        let cursor = c.start + link.chars().count() + 2;
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
            st.cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(cursor))));
            egui::TextEdit::store_state(ui.ctx(), id, st);
        }
        actions.push(Action::Edited(path.clone()));
    }

    if let Some(line) = pending_line {
        let byte = line_start_byte(&buffer.text, line);
        let ch = buffer.text[..byte].chars().count();
        let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        st.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(ch))));
        egui::TextEdit::store_state(ui.ctx(), id, st);
        ui.memory_mut(|m| m.request_focus(id));
    }

    let cache = &mut caches.highlight;
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut h = DefaultHasher::new();
        buf.as_str().hash(&mut h);
        size.to_bits().hash(&mut h);
        mono.hash(&mut h);
        pal.text.hash(&mut h);
        let key = h.finish();
        let mut job = match cache.get(&tab_id) {
            Some((k, job)) if *k == key => job.clone(),
            _ => {
                let job = layout_job(buf.as_str(), size, mono, &pal);
                cache.insert(tab_id, (key, job.clone()));
                job
            }
        };
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };

    let output = readable_column(ui, readable, |ui| {
        ui.add_space(8.0);
        egui::TextEdit::multiline(&mut buffer.text)
            .id(id)
            .frame(Frame::NONE)
            .desired_width(f32::INFINITY)
            .desired_rows(30)
            .lock_focus(true)
            .font(editor_font(size, mono))
            .layouter(&mut layouter)
            .show(ui)
    });

    if output.response.changed() {
        actions.push(Action::Edited(path.clone()));
        // Update `[[` completion.
        caches.completion = output
            .cursor_range
            .filter(|r| r.is_empty())
            .and_then(|r| completion_query(&buffer.text, r.primary.index.into()))
            .map(|(start, query)| Completion {
                tab: tab_id,
                start,
                query,
                selected: 0,
            });
    } else if caches.completion.as_ref().is_some_and(|c| c.tab == tab_id)
        && !output.response.has_focus()
    {
        caches.completion = None;
    }

    let cursor_rect = output.cursor_range.map(|r| {
        output
            .galley
            .pos_from_cursor(r.primary)
            .translate(output.galley_pos.to_vec2())
    });
    if pending_line.is_some()
        && let Some(rect) = cursor_rect
    {
        ui.scroll_to_rect(rect.expand(40.0), Some(Align::Center));
    }

    // Completion popup.
    if let (Some(c), Some(rect)) = (caches.completion.as_mut(), cursor_rect)
        && c.tab == tab_id
        && !suggestions.is_empty()
    {
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("link-completion"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.left_bottom() + egui::vec2(0.0, 4.0))
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(260.0);
                    for (i, (p, alias, _)) in suggestions.iter().enumerate() {
                        let label = match alias {
                            Some(a) => format!("{a}  →  {}", p.stem()),
                            None => p.stem().to_string(),
                        };
                        let r = ui.selectable_label(i == c.selected, label);
                        let r = r.on_hover_text(p.as_str());
                        if r.clicked() {
                            clicked = Some(i);
                        }
                    }
                    ui.label(
                        RichText::new("↑↓ to navigate · ↵ to insert · esc to dismiss")
                            .small()
                            .color(pal.faint),
                    );
                });
            });
        if let Some(i) = clicked {
            c.selected = i;
            // Re-run next frame through the keyboard path.
            ui.input_mut(|inp| {
                inp.events.push(egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            });
        }
    }
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
    fn completion_query_detection() {
        assert_eq!(completion_query("see [[Ab", 8), Some((6, "Ab".into())));
        assert_eq!(completion_query("see [[", 6), Some((6, String::new())));
        assert_eq!(completion_query("see [[x]] y", 11), None);
        assert_eq!(completion_query("[[a\nb", 5), None);
        assert_eq!(completion_query("no link", 3), None);
        // Char indices, not bytes.
        assert_eq!(completion_query("é [[ü", 5), Some((4, "ü".into())));
    }

    #[test]
    fn layout_job_covers_text() {
        let text = "# Title\nsome **bold** [[link]] #tag";
        let job = layout_job(text, 14.0, false, &Palette::dark());
        assert_eq!(job.text, text);
        assert!(job.sections.len() > 4);
    }
}
