//! Reading view: draws the block tree of `crate::reading` with egui widgets
//! (callouts, task checkboxes, recursive transclusions, link previews).

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align, FontId, Frame, Margin, Rect, RichText, Sense, Stroke};
use ors_core::{NotePath, Subpath};

use super::editor::{is_image, readable_column};
use super::theme::Palette;
use super::{Action, UiCaches};
use crate::editor::find::find_all;
use crate::reading::{
    Block, EmbedKey, EmbedPlan, Inline, InlineKind, LinkRef, Prepared, callout_family,
    callout_icon, embed_plan, parse_blocks, prepare, ref_index,
};
use crate::state::{AppState, OpenIn};

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

pub struct Ctx<'a> {
    state: &'a AppState,
    path: &'a NotePath,
    prepared: &'a Prepared,
    pal: Palette,
    size: f32,
    /// Notes being drawn, outermost first (`path` is the last one).
    chain: Vec<EmbedKey>,
    /// Only the note itself is interactive (tasks, scroll tracking), not
    /// its transclusions and previews.
    interactive: bool,
    heading_n: Cell<usize>,
    task_n: Cell<usize>,
    callout_n: Cell<usize>,
    /// Index of the heading to scroll to.
    scroll_heading: Option<usize>,
    clip_top: f32,
    /// Line of the last heading scrolled past the top of the viewport.
    current_line: Cell<Option<usize>>,
    /// Inside a table cell: no wrapping (the grid sizes the columns).
    no_wrap: Cell<bool>,
    /// Screen `y` of the headings of the note (for the heading navigator).
    heading_ys: RefCell<Vec<Option<f32>>>,
    /// Find bar matches to highlight (the note itself only).
    find: Option<FindHl>,
    /// Matches drawn (or skipped) so far, in document order.
    find_n: Cell<usize>,
}

/// The find bar as the reading view sees it.
struct FindHl {
    query: String,
    case: bool,
    current: usize,
    /// Scroll to the current match (once).
    reveal: Cell<bool>,
}

impl<'a> Ctx<'a> {
    fn child(&self, path: &'a NotePath, prepared: &'a Prepared, chain: Vec<EmbedKey>) -> Self {
        Ctx {
            state: self.state,
            path,
            prepared,
            pal: self.pal,
            size: self.size,
            chain,
            interactive: false,
            heading_n: Cell::new(0),
            task_n: Cell::new(0),
            callout_n: Cell::new(0),
            scroll_heading: None,
            clip_top: 0.0,
            current_line: Cell::new(None),
            no_wrap: Cell::new(false),
            heading_ys: RefCell::new(Vec::new()),
            find: None,
            find_n: Cell::new(0),
        }
    }
}

/// (headings, tasks, callouts) in `blocks`, to keep the document-order
/// counters right when a folded callout is not drawn.
fn counts(blocks: &[Block]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    let mut add = |d: (usize, usize, usize)| {
        c.0 += d.0;
        c.1 += d.1;
        c.2 += d.2;
    };
    for b in blocks {
        match b {
            Block::Heading { .. } => add((1, 0, 0)),
            Block::Quote(inner) => add(counts(inner)),
            Block::Callout { blocks, .. } => {
                let d = counts(blocks);
                add((d.0, d.1, d.2 + 1));
            }
            Block::List { items, .. } => {
                for it in items {
                    let d = counts(&it.blocks);
                    add((d.0, d.1 + usize::from(it.task.is_some()), d.2));
                }
            }
            _ => {}
        }
    }
    c
}

/// Index of the heading to scroll to for `line`: the last one at or before it.
fn heading_for_line(heading_lines: &[usize], line: usize) -> Option<usize> {
    heading_lines.iter().rposition(|l| *l <= line)
}

pub fn reading_view(
    ui: &mut egui::Ui,
    state: &AppState,
    caches: &mut UiCaches,
    path: &NotePath,
    scroll_to: Option<usize>,
    actions: &mut Vec<Action>,
) {
    let text = state.note_text(path).unwrap_or_default();
    let (prepared, blocks) = rendered(state, caches, path, &text);
    let pal = Palette::of(ui.ctx());
    let size = state.settings.font_size;
    let clip_top = ui.clip_rect().top();
    let active = state.active_tab().map(|t| t.id);
    let find = caches
        .find
        .as_mut()
        .filter(|f| Some(f.tab) == active && !f.query.is_empty())
        .map(|f| FindHl {
            query: f.query.clone(),
            case: f.case,
            current: f.current,
            reveal: Cell::new(f.reveal.take().is_some()),
        });
    let mut current = None;
    readable_column(ui, state.settings.readable_line_length, |ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(path.stem())
                .size(size * 1.9)
                .color(pal.strong),
        );
        ui.add_space(4.0);
        if crate::properties_model::panel_visible(&state.settings) {
            super::properties_view::panel(ui, state, path, "doc", true, actions);
        } else if state.views.show_properties_table
            && !crate::properties_model::frontmatter_hidden(&state.settings)
            && !prepared.frontmatter.is_empty()
        {
            properties(ui, &prepared.frontmatter, &pal, size);
        }
        let cx = Ctx {
            state,
            path,
            prepared: &prepared,
            pal,
            size,
            chain: vec![(path.clone(), None)],
            interactive: true,
            heading_n: Cell::new(0),
            task_n: Cell::new(0),
            callout_n: Cell::new(0),
            scroll_heading: scroll_to.and_then(|l| heading_for_line(&prepared.heading_lines, l)),
            clip_top,
            current_line: Cell::new(None),
            no_wrap: Cell::new(false),
            heading_ys: RefCell::new(vec![None; prepared.heading_lines.len()]),
            find,
            find_n: Cell::new(0),
        };
        draw_blocks(ui, &cx, caches, &blocks, actions);
        caches.nav.ys = Some(cx.heading_ys.take());
        current = cx.current_line.get();
        ui.add_space(40.0);
    });
    let shown = state.views.current_line.as_ref().map(|(p, l)| (p, *l));
    let line = current.unwrap_or(0);
    if shown != Some((path, line)) {
        actions.push(Action::View(crate::views::ViewAction::SetCurrentLine(
            path.clone(),
            line,
        )));
    }
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
    if cx.interactive {
        return draw_blocks_culled(ui, cx, caches, blocks, actions);
    }
    for b in blocks {
        draw_block(ui, cx, caches, b, actions);
    }
}

/// Identity of a laid-out note: blocks (address, count), width, font size.
type HeightsKey = (usize, usize, u32, u32);

/// Top-level blocks of the note, with the ones far outside the viewport
/// replaced by empty space of the height they had when last drawn: laying
/// out every block of a long note on each frame made scrolling slow.
fn draw_blocks_culled(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    blocks: &[Block],
    actions: &mut Vec<Action>,
) {
    let id = egui::Id::unique(("reading-heights", cx.path.as_str()));
    // Same blocks (the Rc'd parse), width and font size: heights still hold.
    let key: HeightsKey = (
        blocks.as_ptr() as usize,
        blocks.len(),
        ui.available_width().to_bits(),
        cx.size.to_bits(),
    );
    let known: Option<Vec<f32>> = ui
        .data(|d| d.get_temp::<(HeightsKey, Vec<f32>)>(id))
        .filter(|(k, h)| *k == key && h.len() == blocks.len())
        .map(|(_, h)| h);
    let clip = ui.clip_rect();
    let (lo, hi) = (clip.top() - clip.height(), clip.bottom() + clip.height());
    let mut heights = Vec::with_capacity(blocks.len());
    for (i, b) in blocks.iter().enumerate() {
        let top = ui.cursor().top();
        let (headings, tasks, callouts) = counts(std::slice::from_ref(b));
        let first = cx.heading_n.get();
        let found = find_count(cx, std::slice::from_ref(b));
        let first_found = cx.find_n.get();
        let wanted = cx
            .scroll_heading
            .is_some_and(|h| (first..first + headings).contains(&h))
            || cx.find.as_ref().is_some_and(|f| {
                f.reveal.get() && (first_found..first_found + found).contains(&f.current)
            });
        if let Some(h) = known.as_ref().and_then(|k| k.get(i).copied())
            && !wanted
            && (top + h < lo || top > hi)
        {
            // Skipped headings sit at the top of their block, close enough
            // for the heading navigator and the current-line tracking.
            for n in first..first + headings {
                if let Some(slot) = cx.heading_ys.borrow_mut().get_mut(n) {
                    *slot = Some(top);
                }
                if top <= cx.clip_top + 24.0
                    && let Some(l) = cx.prepared.heading_lines.get(n)
                {
                    cx.current_line.set(Some(*l));
                }
            }
            cx.heading_n.set(first + headings);
            cx.task_n.set(cx.task_n.get() + tasks);
            cx.callout_n.set(cx.callout_n.get() + callouts);
            cx.find_n.set(first_found + found);
            ui.add_space(h);
            heights.push(h);
            continue;
        }
        draw_block(ui, cx, caches, b, actions);
        heights.push(ui.cursor().top() - top);
    }
    ui.data_mut(|d| d.insert_temp(id, (key, heights)));
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
            let r = ui.scope(|ui| {
                inline_row(ui, cx, caches, inlines, cx.size * scale, true, actions);
            });
            if cx.interactive {
                let n = cx.heading_n.get();
                cx.heading_n.set(n + 1);
                if let Some(slot) = cx.heading_ys.borrow_mut().get_mut(n) {
                    *slot = Some(r.response.rect.top());
                }
                if cx.scroll_heading == Some(n) {
                    ui.scroll_to_rect(r.response.rect, Some(Align::TOP));
                }
                if r.response.rect.top() <= cx.clip_top + 24.0
                    && let Some(l) = cx.prepared.heading_lines.get(n)
                {
                    cx.current_line.set(Some(*l));
                }
            }
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
                    let marks = find_marks(cx, text);
                    if marks.is_empty() {
                        ui.label(
                            RichText::new(text)
                                .monospace()
                                .size(cx.size * 0.9)
                                .color(pal.text),
                        );
                    } else {
                        let format = TextFormat {
                            font_id: FontId::monospace(cx.size * 0.9),
                            color: pal.text,
                            ..TextFormat::default()
                        };
                        let mut job = LayoutJob::default();
                        let mut pos = 0;
                        for (r, cur) in &marks {
                            job.append(&text[pos..r.start], 0.0, format.clone());
                            let background = mark_color(&pal, *cur);
                            job.append(
                                &text[r.clone()],
                                0.0,
                                TextFormat {
                                    background,
                                    ..format.clone()
                                },
                            );
                            pos = r.end;
                        }
                        job.append(&text[pos..], 0.0, format);
                        let rect = ui.label(job).rect;
                        reveal(ui, cx, &marks, Some(rect));
                    }
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
            fold,
            blocks,
        } => {
            let color = pal.callout(callout_family(kind));
            let n = cx.callout_n.get();
            cx.callout_n.set(n + 1);
            let id = ui.make_persistent_id(("callout", cx.path.as_str(), n, title));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                *fold != Some(true),
            );
            // The current find match is in the folded body: open it.
            if let Some(f) = cx.find.as_ref().filter(|f| f.reveal.get()) {
                let start = cx.find_n.get() + find_all(title, &f.query, f.case).len();
                if (start..start + find_count(cx, blocks)).contains(&f.current) {
                    state.set_open(true);
                }
            }
            Frame::new()
                .fill(color.gamma_multiply(0.12))
                .corner_radius(6)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let foldable = fold.is_some();
                    let head = ui.horizontal(|ui| {
                        ui.label(RichText::new(callout_icon(kind)).color(color).size(cx.size));
                        let marks = find_marks(cx, title);
                        let rect = marked_labels(ui, title, &marks, &pal, |t| {
                            RichText::new(t).color(color).size(cx.size).strong()
                        });
                        reveal(ui, cx, &marks, rect);
                        if foldable {
                            let arrow = if state.is_open() { "⏷" } else { "⏵" };
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| ui.label(RichText::new(arrow).color(color)),
                            );
                        }
                    });
                    if foldable {
                        let r = ui
                            .interact(head.response.rect, id.with("head"), Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.clicked() {
                            state.toggle(ui);
                        }
                    }
                    if !foldable || state.is_open() {
                        draw_blocks(ui, cx, caches, blocks, actions);
                    } else {
                        let d = counts(blocks);
                        cx.find_n.set(cx.find_n.get() + find_count(cx, blocks));
                        cx.heading_n.set(cx.heading_n.get() + d.0);
                        cx.task_n.set(cx.task_n.get() + d.1);
                        cx.callout_n.set(cx.callout_n.get() + d.2);
                    }
                });
            state.store(ui.ctx());
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
                    let mark = ui.add(
                        egui::Label::new(RichText::new(marker).size(cx.size).color(color)).sense(
                            if item.task.is_some() && cx.interactive {
                                Sense::click()
                            } else {
                                Sense::hover()
                            },
                        ),
                    );
                    if item.task.is_some() {
                        let t = cx.task_n.get();
                        cx.task_n.set(t + 1);
                        if cx.interactive {
                            let mark = mark.on_hover_cursor(egui::CursorIcon::PointingHand);
                            if mark.clicked()
                                && let Some(line) = cx.prepared.task_lines.get(t)
                            {
                                actions.push(Action::View(crate::views::ViewAction::ToggleTask {
                                    path: cx.path.clone(),
                                    line: *line,
                                }));
                            }
                        }
                    }
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
            cx.no_wrap.set(true);
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
            cx.no_wrap.set(false);
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
    let body = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for inline in run.drain(..) {
            draw_inline(ui, cx, caches, inline, size, strong, actions);
        }
    };
    if cx.no_wrap.get() {
        ui.horizontal(body);
    } else {
        ui.horizontal_wrapped(body);
    }
}

fn draw_inline(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    inline: &Inline,
    size: f32,
    strong: bool,
    actions: &mut Vec<Action>,
) {
    let pal = cx.pal;
    match &inline.kind {
        InlineKind::Text => {
            let text = inline.text.replace('\n', " ");
            let marks = find_marks(cx, &text);
            let rect = marked_labels(ui, &text, &marks, &pal, |t| {
                rich(t, inline, size, strong, &pal)
            });
            reveal(ui, cx, &marks, rect);
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
                let marks = find_marks(cx, &inline.text.replace('\n', " "));
                let mut label = rich(&inline.text, inline, size, strong, &pal).color(color);
                if let Some(bg) = link_mark(&pal, &marks) {
                    label = label.background_color(bg);
                }
                let r = ui
                    .add(egui::Label::new(label).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                reveal(ui, cx, &marks, Some(r.rect));
                // Ctrl held: preview of the note; otherwise a plain tooltip.
                let preview = ui
                    .input(|i| i.modifiers.command)
                    .then_some(resolved.as_ref())
                    .flatten()
                    .filter(|p| p.is_markdown());
                match preview {
                    Some(p) => {
                        r.clone().on_hover_ui(|ui| {
                            link_preview(ui, cx, caches, p, subpath.as_ref(), actions)
                        });
                    }
                    None => {
                        r.clone().on_hover_text(match resolved {
                            Some(p) => p.as_str().to_string(),
                            None => format!("{target} (not created yet — click to create)"),
                        });
                    }
                }
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
                    if let Some(p) = resolved.as_ref().filter(|p| crate::pdf::is_pdf(p)) {
                        let page = crate::pdf::params_from_link(subpath.as_ref(), None).page;
                        actions.push(Action::OpenPdf(p.clone(), page.unwrap_or(1), open_in));
                        return;
                    }
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
                let marks = find_marks(cx, &inline.text.replace('\n', " "));
                let mut label = rich(&inline.text, inline, size, strong, &pal).color(pal.accent);
                if let Some(bg) = link_mark(&pal, &marks) {
                    label = label.background_color(bg);
                }
                let r = ui.hyperlink_to(label, url);
                reveal(ui, cx, &marks, Some(r.rect));
            }
        },
        InlineKind::Image { url } => {
            ui.hyperlink_to(&inline.text, url);
        }
    }
}

// ---------------------------------------------------------------------------
// Find in the reading view
// ---------------------------------------------------------------------------

/// The searchable texts of `blocks`, in the order the view draws them (the
/// find bar counts its matches in them; drawing highlights the same ones).
fn find_pieces(prepared: &Prepared, blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                inline_pieces(prepared, inlines, out);
            }
            Block::Code { text, .. } => out.push(text.clone()),
            Block::Quote(inner) => find_pieces(prepared, inner, out),
            Block::Callout { title, blocks, .. } => {
                out.push(title.clone());
                find_pieces(prepared, blocks, out);
            }
            Block::List { items, .. } => {
                for it in items {
                    find_pieces(prepared, &it.blocks, out);
                }
            }
            Block::Table { header, rows } => {
                for cell in header.iter().chain(rows.iter().flatten()) {
                    inline_pieces(prepared, cell, out);
                }
            }
            Block::Rule => {}
        }
    }
}

/// Mirrors [`inline_row`]: embeds and leading blank texts are not drawn as
/// text, tags are not searched.
fn inline_pieces(prepared: &Prepared, inlines: &[Inline], out: &mut Vec<String>) {
    let mut run_empty = true;
    for inline in inlines {
        match &inline.kind {
            InlineKind::Image { .. } => {
                run_empty = true;
                continue;
            }
            InlineKind::Text if run_empty && inline.text.trim().is_empty() => continue,
            InlineKind::Text => out.push(inline.text.replace('\n', " ")),
            InlineKind::Link(url) => {
                if !matches!(
                    ref_index(url).and_then(|i| prepared.refs.get(i)),
                    Some(LinkRef::Tag(_))
                ) {
                    out.push(inline.text.replace('\n', " "));
                }
            }
            InlineKind::LineBreak => {}
        }
        run_empty = false;
    }
}

/// The text the find bar searches in the reading view: the pieces, kept
/// apart so that no match spans two of them.
pub fn search_text(state: &AppState, caches: &mut UiCaches, path: &NotePath) -> String {
    let text = state.note_text(path).unwrap_or_default();
    let (prepared, blocks) = rendered(state, caches, path, &text);
    let mut pieces = Vec::new();
    find_pieces(&prepared, &blocks, &mut pieces);
    pieces.join("\u{0}")
}

/// Number of find matches in `blocks`.
fn find_count(cx: &Ctx<'_>, blocks: &[Block]) -> usize {
    let Some(f) = &cx.find else {
        return 0;
    };
    let mut pieces = Vec::new();
    find_pieces(cx.prepared, blocks, &mut pieces);
    pieces
        .iter()
        .map(|p| find_all(p, &f.query, f.case).len())
        .sum()
}

/// The next find matches, in `text` (which is drawn now): ranges, and
/// whether each is the current one.
fn find_marks(cx: &Ctx<'_>, text: &str) -> Vec<(Range<usize>, bool)> {
    let Some(f) = &cx.find else {
        return Vec::new();
    };
    let first = cx.find_n.get();
    let found = find_all(text, &f.query, f.case);
    cx.find_n.set(first + found.len());
    found
        .into_iter()
        .enumerate()
        .map(|(i, r)| (r, first + i == f.current))
        .collect()
}

fn mark_color(pal: &Palette, current: bool) -> egui::Color32 {
    if current {
        pal.accent.gamma_multiply(0.55)
    } else {
        pal.highlight_bg
    }
}

/// Background of a link label holding matches (links stay one widget).
fn link_mark(pal: &Palette, marks: &[(Range<usize>, bool)]) -> Option<egui::Color32> {
    (!marks.is_empty()).then(|| mark_color(pal, marks.iter().any(|m| m.1)))
}

/// `text` as labels, the matches highlighted; the rect of the current one.
fn marked_labels(
    ui: &mut egui::Ui,
    text: &str,
    marks: &[(Range<usize>, bool)],
    pal: &Palette,
    make: impl Fn(&str) -> RichText,
) -> Option<Rect> {
    if marks.is_empty() {
        ui.label(make(text));
        return None;
    }
    let mut pos = 0;
    let mut current = None;
    for (r, cur) in marks {
        if r.start > pos {
            ui.label(make(&text[pos..r.start]));
        }
        let rect = ui
            .label(make(&text[r.clone()]).background_color(mark_color(pal, *cur)))
            .rect;
        if *cur {
            current = Some(rect);
        }
        pos = r.end;
    }
    if pos < text.len() {
        ui.label(make(&text[pos..]));
    }
    current
}

/// Scroll to the current match once, when the find bar asked for it.
fn reveal(ui: &egui::Ui, cx: &Ctx<'_>, marks: &[(Range<usize>, bool)], rect: Option<Rect>) {
    if let (Some(f), Some(rect)) = (&cx.find, rect)
        && f.reveal.get()
        && marks.iter().any(|m| m.1)
    {
        ui.scroll_to_rect(rect, Some(Align::Center));
        f.reveal.set(false);
    }
}

/// Hover preview of a note (or of the section a link points to), drawn like
/// a transclusion; at most 30 lines.
fn link_preview(
    ui: &mut egui::Ui,
    cx: &Ctx<'_>,
    caches: &mut UiCaches,
    path: &NotePath,
    subpath: Option<&Subpath>,
    actions: &mut Vec<Action>,
) {
    ui.set_max_width(440.0);
    let Some(full) = cx.state.note_text(path) else {
        return;
    };
    let section = section_of(cx.state, path, &full, subpath);
    let text: String = section.lines().take(30).collect::<Vec<_>>().join("\n");
    let parsed = cx.state.parsed_with_text(path, &text);
    let prepared = prepare(&text, &parsed, path, &*cx.state.index);
    let blocks = parse_blocks(&prepared.markdown);
    let mut chain = cx.chain.clone();
    chain.push((path.clone(), subpath.cloned()));
    let inner = cx.child(path, &prepared, chain);
    ui.label(RichText::new(path.stem()).strong().color(cx.pal.accent));
    ui.separator();
    draw_blocks(ui, &inner, caches, &blocks, actions);
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
        display,
        ..
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
    if crate::pdf::is_pdf(path) {
        let params = crate::pdf::params_from_link(subpath.as_ref(), display.as_deref());
        super::pdf_view::pdf_embed(ui, cx.state, caches, path, &params, actions);
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
            match embed_plan(&cx.chain, path, subpath.as_ref()) {
                EmbedPlan::Render => {}
                EmbedPlan::Cycle => {
                    ui.label(RichText::new("↻ circular embed").color(pal.faint));
                    return;
                }
                EmbedPlan::TooDeep => {
                    ui.label(RichText::new("…").color(pal.faint));
                    return;
                }
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
            let mut chain = cx.chain.clone();
            chain.push((path.clone(), subpath.clone()));
            let inner = cx.child(path, &prepared, chain);
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
