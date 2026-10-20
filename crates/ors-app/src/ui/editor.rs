//! Note views: source editor (with highlighting and `[[` completion),
//! reading view, and attachment preview.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use eframe::egui::{
    self, Color32, FontFamily, FontId, Frame, Margin, RichText, Sense,
    Stroke, TextFormat,
    text::LayoutJob,
};
use ors_core::{NotePath, Subpath};

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::highlight::{Style, highlight};
use crate::reading::{
    Block, Inline, InlineKind, LinkRef, Prepared, parse_blocks, prepare, ref_index,
};
use crate::state::{AppState, OpenIn};

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

pub(super) fn format_for(s: &Style, size: f32, mono: bool, pal: &Palette) -> TextFormat {
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

// ---------------------------------------------------------------------------
// Reading mode
// ---------------------------------------------------------------------------

pub struct Rendered {
    key: u64,
    pub prepared: Rc<Prepared>,
    pub blocks: Rc<Vec<Block>>,
}

fn rendered(
    state: &AppState,
    caches: &mut UiCaches,
    path: &NotePath,
    text: &str,
) -> (Rc<Prepared>, Rc<Vec<Block>>) {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    state.generation.hash(&mut h);
    let key = h.finish();
    if let Some(r) = caches.reading.get(path)
        && r.key == key
    {
        return (r.prepared.clone(), r.blocks.clone());
    }
    let parsed = state.parsed_with_text(path, text);
    let prepared = prepare(text, &parsed, path, &*state.index);
    let blocks = Rc::new(parse_blocks(&prepared.markdown));
    let prepared = Rc::new(prepared);
    caches.reading.insert(
        path.clone(),
        Rendered {
            key,
            prepared: prepared.clone(),
            blocks: blocks.clone(),
        },
    );
    (prepared, blocks)
}

struct Ctx<'a> {
    state: &'a AppState,
    path: &'a NotePath,
    prepared: &'a Prepared,
    pal: Palette,
    size: f32,
    depth: usize,
}

pub fn reading_view(
    ui: &mut egui::Ui,
    state: &AppState,
    caches: &mut UiCaches,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    let text = state.note_text(path).unwrap_or_default();
    let (prepared, blocks) = rendered(state, caches, path, &text);
    let pal = Palette::of(ui.ctx());
    let size = state.settings.font_size;
    readable_column(ui, state.settings.readable_line_length, |ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(path.stem())
                .size(size * 1.9)
                .color(pal.strong),
        );
        ui.add_space(4.0);
        if !prepared.frontmatter.is_empty() {
            properties(ui, &prepared.frontmatter, &pal, size);
        }
        let cx = Ctx {
            state,
            path,
            prepared: &prepared,
            pal,
            size,
            depth: 0,
        };
        draw_blocks(ui, &cx, caches, &blocks, actions);
        ui.add_space(40.0);
    });
}

fn properties(ui: &mut egui::Ui, props: &[(String, String)], pal: &Palette, size: f32) {
    Frame::new()
        .inner_margin(Margin::symmetric(8, 6))
        .corner_radius(6)
        .stroke(Stroke::new(1.0, pal.border))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("properties")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (k, v) in props {
                        ui.label(RichText::new(k).color(pal.muted).size(size * 0.9));
                        ui.label(RichText::new(v).size(size * 0.9));
                        ui.end_row();
                    }
                });
        });
    ui.add_space(8.0);
}

fn draw_blocks(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    blocks: &[Block],
    actions: &mut Vec<Action>,
) {
    for b in blocks {
        draw_block(ui, cx, caches, b, actions);
    }
}

fn draw_block(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    block: &Block,
    actions: &mut Vec<Action>,
) {
    let pal = cx.pal;
    match block {
        Block::Heading { level, inlines } => {
            let scale = match level {
                1 => 1.6,
                2 => 1.4,
                3 => 1.25,
                4 => 1.12,
                _ => 1.05,
            };
            ui.add_space(cx.size * 0.6);
            inline_row(ui, cx, caches, inlines, cx.size * scale, true, actions);
            ui.add_space(cx.size * 0.2);
        }
        Block::Paragraph(inlines) => {
            inline_row(ui, cx, caches, inlines, cx.size, false, actions);
            ui.add_space(cx.size * 0.5);
        }
        Block::Code { lang, text } => {
            Frame::new()
                .fill(pal.code_bg)
                .corner_radius(4)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !lang.is_empty() {
                        ui.label(RichText::new(lang).small().color(pal.faint));
                    }
                    ui.label(
                        RichText::new(text)
                            .monospace()
                            .size(cx.size * 0.9)
                            .color(pal.text),
                    );
                });
            ui.add_space(cx.size * 0.5);
        }
        Block::Quote(inner) => {
            let resp = Frame::new()
                .inner_margin(Margin {
                    left: 14,
                    right: 4,
                    top: 2,
                    bottom: 2,
                })
                .show(ui, |ui| draw_blocks(ui, cx, caches, inner, actions));
            let r = resp.response.rect;
            ui.painter().line_segment(
                [r.left_top(), r.left_bottom()],
                Stroke::new(2.0, pal.accent),
            );
            ui.add_space(cx.size * 0.3);
        }
        Block::Callout {
            kind,
            title,
            blocks,
        } => {
            let color = pal.callout(kind);
            Frame::new()
                .fill(color.gamma_multiply(0.12))
                .corner_radius(6)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(title).color(color).size(cx.size).strong());
                    draw_blocks(ui, cx, caches, blocks, actions);
                });
            ui.add_space(cx.size * 0.5);
        }
        Block::List { start, items } => {
            for (n, item) in items.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.add_space(8.0);
                    let marker = match (item.task, start) {
                        (Some(true), _) => "☑".to_string(),
                        (Some(false), _) => "☐".to_string(),
                        (None, Some(s)) => format!("{}.", s + n as u64),
                        (None, None) => "•".to_string(),
                    };
                    let color = if item.task.is_some() {
                        pal.accent
                    } else {
                        pal.muted
                    };
                    ui.label(RichText::new(marker).size(cx.size).color(color));
                    ui.vertical(|ui| {
                        if item.task == Some(true) {
                            ui.style_mut().visuals.override_text_color = Some(pal.muted);
                        }
                        for b in &item.blocks {
                            match b {
                                Block::Paragraph(inl) => {
                                    inline_row(ui, cx, caches, inl, cx.size, false, actions)
                                }
                                other => draw_block(ui, cx, caches, other, actions),
                            }
                        }
                    });
                });
            }
            ui.add_space(cx.size * 0.5);
        }
        Block::Table { header, rows } => {
            egui::Grid::new(ui.next_auto_id())
                .striped(true)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    for cell in header {
                        inline_row(ui, cx, caches, cell, cx.size, true, actions);
                    }
                    ui.end_row();
                    for row in rows {
                        for cell in row {
                            inline_row(ui, cx, caches, cell, cx.size, false, actions);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(cx.size * 0.5);
        }
        Block::Rule => {
            ui.separator();
        }
    }
}

fn rich(text: &str, inline: &Inline, size: f32, strong: bool, pal: &Palette) -> RichText {
    let mut t = RichText::new(text.replace('\n', " "))
        .size(size)
        .color(pal.text);
    if strong || inline.style.bold {
        t = t.color(pal.strong);
    }
    if inline.style.italic {
        t = t.italics();
    }
    if inline.style.strike {
        t = t.strikethrough();
    }
    if inline.style.highlight {
        t = t.background_color(pal.highlight_bg);
    }
    if inline.style.code {
        t = t
            .font(FontId::monospace(size * 0.9))
            .color(pal.code_fg)
            .background_color(pal.code_bg);
    }
    t
}

fn inline_row(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    inlines: &[Inline],
    size: f32,
    strong: bool,
    actions: &mut Vec<Action>,
) {
    // Embeds are laid out as blocks between wrapped text rows.
    let mut run: Vec<&Inline> = Vec::new();
    for inline in inlines {
        if let InlineKind::Image { .. } = inline.kind {
            flush_run(ui, cx, caches, &mut run, size, strong, actions);
            draw_embed(ui, cx, caches, inline, actions);
        } else if !(run.is_empty()
            && inline.kind == InlineKind::Text
            && inline.text.trim().is_empty())
        {
            run.push(inline);
        }
    }
    flush_run(ui, cx, caches, &mut run, size, strong, actions);
}

fn flush_run(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    run: &mut Vec<&Inline>,
    size: f32,
    strong: bool,
    actions: &mut Vec<Action>,
) {
    if run.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for inline in run.drain(..) {
            draw_inline(ui, cx, caches, inline, size, strong, actions);
        }
    });
}

fn draw_inline(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    _caches: &mut UiCaches,
    inline: &Inline,
    size: f32,
    strong: bool,
    actions: &mut Vec<Action>,
) {
    let pal = cx.pal;
    match &inline.kind {
        InlineKind::Text => {
            ui.label(rich(&inline.text, inline, size, strong, &pal));
        }
        InlineKind::LineBreak => ui.end_row(),
        InlineKind::Link(url) => match ref_index(url).and_then(|i| cx.prepared.refs.get(i)) {
            Some(LinkRef::Internal {
                target,
                resolved,
                subpath,
            }) => {
                let color = if resolved.is_some() {
                    pal.accent
                } else {
                    pal.unresolved
                };
                let r = ui
                    .add(
                        egui::Label::new(
                            rich(&inline.text, inline, size, strong, &pal).color(color),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(match resolved {
                        Some(p) => p.as_str().to_string(),
                        None => format!("{target} (not created yet — click to create)"),
                    });
                if r.hovered() {
                    let rect = r.rect;
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        Stroke::new(1.0, color),
                    );
                }
                if r.clicked() || r.middle_clicked() {
                    let open_in = if r.middle_clicked() || ui.input(|i| i.modifiers.command) {
                        OpenIn::NewTab
                    } else {
                        OpenIn::CurrentTab
                    };
                    let line = heading_line(cx, resolved.as_ref(), subpath.as_ref());
                    match (resolved, line) {
                        (Some(p), Some(line)) if target.is_empty() || p != cx.path => {
                            actions.push(Action::OpenLine(p.clone(), line, open_in));
                        }
                        (Some(p), Some(line)) => {
                            actions.push(Action::OpenLine(p.clone(), line, open_in))
                        }
                        _ => actions.push(Action::Follow {
                            target: target.clone(),
                            from: cx.path.clone(),
                            open_in,
                        }),
                    }
                }
            }
            Some(LinkRef::Tag(tag)) => {
                let r = ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!("#{tag}"))
                                .size(size * 0.92)
                                .color(pal.accent)
                                .background_color(pal.tag_bg),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if r.clicked() {
                    actions.push(Action::SearchTag(tag.clone()));
                }
            }
            _ => {
                ui.hyperlink_to(
                    rich(&inline.text, inline, size, strong, &pal).color(pal.accent),
                    url,
                );
            }
        },
        InlineKind::Image { url } => {
            ui.hyperlink_to(&inline.text, url);
        }
    }
}

/// Line of the heading/block a subpath points to, if it can be found.
fn heading_line(
    cx: &Ctx<'_>,
    target: Option<&NotePath>,
    subpath: Option<&Subpath>,
) -> Option<usize> {
    let target = target?;
    let parsed = cx.state.parsed(target);
    match subpath? {
        Subpath::Heading(h) => {
            let last = h.last()?;
            parsed
                .headings
                .iter()
                .find(|x| x.text.eq_ignore_ascii_case(last))
                .map(|x| x.line)
        }
        Subpath::Block(id) => parsed
            .block_ids
            .iter()
            .find(|b| &b.id == id)
            .map(|b| b.line),
    }
}

fn draw_embed(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    inline: &Inline,
    actions: &mut Vec<Action>,
) {
    let pal = cx.pal;
    let InlineKind::Image { url } = &inline.kind else {
        return;
    };
    let Some(LinkRef::Embed {
        target,
        subpath,
        resolved,
    }) = ref_index(url).and_then(|i| cx.prepared.refs.get(i))
    else {
        // External image: not fetched (no network access from the app).
        ui.hyperlink_to(format!("🖼 {}", inline.text), url);
        return;
    };
    let Some(path) = resolved else {
        let r = ui.add(
            egui::Label::new(
                RichText::new(format!("⚠ {target} (missing embed)")).color(pal.unresolved),
            )
            .sense(Sense::click()),
        );
        if r.clicked() {
            actions.push(Action::Follow {
                target: target.clone(),
                from: cx.path.clone(),
                open_in: OpenIn::CurrentTab,
            });
        }
        return;
    };
    if is_image(path) {
        match caches.image_bytes(cx.state, path) {
            Some(bytes) => {
                let max_w = ui.available_width().min(700.0);
                let resp = ui.add(
                    egui::Image::from_bytes(format!("bytes://{}", path.as_str()), bytes)
                        .fit_to_original_size(1.0)
                        .max_width(max_w)
                        .corner_radius(4)
                        .sense(Sense::click()),
                );
                if resp.clicked() {
                    actions.push(Action::Open(path.clone(), OpenIn::NewTab));
                }
            }
            None => {
                ui.label(RichText::new(format!("⚠ cannot load {path}")).color(pal.unresolved));
            }
        }
        return;
    }
    if !path.is_markdown() {
        if ui.link(format!("📎 {}", path.file_name())).clicked() {
            actions.push(Action::Open(path.clone(), OpenIn::CurrentTab));
        }
        return;
    }
    // Note transclusion (one level deep).
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 8,
            top: 6,
            bottom: 6,
        })
        .fill(pal.accent_soft.gamma_multiply(0.4))
        .corner_radius(4)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = match subpath {
                Some(Subpath::Heading(h)) => format!("{} › {}", path.stem(), h.join(" › ")),
                Some(Subpath::Block(b)) => format!("{} › ^{b}", path.stem()),
                None => path.stem().to_string(),
            };
            let r = ui.add(
                egui::Label::new(RichText::new(title).color(pal.accent).small())
                    .sense(Sense::click()),
            );
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                actions.push(Action::Open(path.clone(), OpenIn::CurrentTab));
            }
            if cx.depth >= 1 || path == cx.path && subpath.is_none() {
                ui.label(RichText::new("…").color(pal.faint));
                return;
            }
            let Some(full) = cx.state.note_text(path) else {
                return;
            };
            let text = section_of(cx.state, path, &full, subpath.as_ref());
            let parsed = if subpath.is_some() {
                cx.state.index.parse(&text)
            } else {
                cx.state.parsed_with_text(path, &text)
            };
            let prepared = prepare(&text, &parsed, path, &*cx.state.index);
            let blocks = parse_blocks(&prepared.markdown);
            let inner = Ctx {
                state: cx.state,
                path,
                prepared: &prepared,
                pal,
                size: cx.size,
                depth: cx.depth + 1,
            };
            draw_blocks(ui, &inner, caches, &blocks, actions);
        });
    ui.add_space(cx.size * 0.4);
}

/// The part of `text` a subpath refers to (whole note if none / not found).
fn section_of(state: &AppState, path: &NotePath, text: &str, subpath: Option<&Subpath>) -> String {
    let Some(sub) = subpath else {
        return text.to_string();
    };
    let parsed = state.parsed_with_text(path, text);
    let lines: Vec<&str> = text.lines().collect();
    match sub {
        Subpath::Heading(h) => {
            let Some(last) = h.last() else {
                return text.to_string();
            };
            let Some(pos) = parsed
                .headings
                .iter()
                .position(|x| x.text.eq_ignore_ascii_case(last))
            else {
                return text.to_string();
            };
            let head = &parsed.headings[pos];
            let end = parsed.headings[pos + 1..]
                .iter()
                .find(|x| x.level <= head.level)
                .map_or(lines.len(), |x| x.line);
            lines
                .get(head.line..end.min(lines.len()))
                .map(|l| l.join("\n"))
                .unwrap_or_default()
        }
        Subpath::Block(id) => parsed
            .block_ids
            .iter()
            .find(|b| &b.id == id)
            .and_then(|b| lines.get(b.line))
            .map(|l| l.replace(&format!("^{id}"), ""))
            .unwrap_or_else(|| text.to_string()),
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
    fn layout_job_covers_text() {
        let text = "# Title\nsome **bold** [[link]] #tag";
        let job = layout_job(text, 14.0, false, &Palette::dark());
        assert_eq!(job.text, text);
        assert!(job.sections.len() > 4);
    }
}
