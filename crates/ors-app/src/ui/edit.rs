//! The editing view: Live Preview and Source mode on top of egui's
//! `TextEdit`, with formatting shortcuts, list continuation, autocompletion,
//! find/replace and ctrl+click on links.
//!
//! Live Preview keeps the markdown in the buffer (so the caret and the
//! selection stay correct) and only *styles* it: markers of lines that do not
//! hold the caret are laid out at a tiny size in a transparent colour, and
//! the bullets / checkboxes / rules are painted over blank glyphs. All text
//! transformations live in [`crate::editor`] and are unit-tested there.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use eframe::egui::{
    self, Align, Color32, Event, FontFamily, FontId, Frame, Key, Margin, Modifiers, Pos2, Rect,
    Sense, Stroke, TextFormat,
    text::{CCursor, CCursorRange, LayoutJob},
};
use ors_core::NotePath;

use super::edit_blocks;
use super::edit_deco;
use super::editor::{
    editor_font, format_for_mode, heading_scale, layout_job, readable_column_with, styled_font,
};
use super::theme::{Palette, has_styled_fonts};
use super::{Action, UiCaches};
use crate::backend::IndexBackend;
use crate::commands::Command;
use crate::editor::blocks::{self, BlockKind, Pad};
use crate::editor::completion::{self, Context as CompCtx, Kind as CompKind};
use crate::editor::find::{find_all, next_match, replace_all};
use crate::editor::fold;
use crate::editor::format as fmt;
use crate::editor::live::{Conceal, Decor, LineInfo, LineKind, LinkTarget, Widget, decorate};
use crate::editor::{Sel, byte_to_char, char_to_byte, line_of, run_command};
use crate::highlight::{Style, highlight};
use crate::state::{AppState, OpenIn, TabId};
use crate::util::line_start_byte;

// ---------------------------------------------------------------------------
// UI state kept in `UiCaches`
// ---------------------------------------------------------------------------

/// One suggestion of the completion popup.
#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub detail: String,
    /// Text put between the brackets (or after `#`).
    pub insert: String,
    /// The note, for markdown-link output.
    pub target: Option<NotePath>,
}

/// Autocompletion popup state.
#[derive(Debug, Clone)]
pub struct CompletionUi {
    pub tab: TabId,
    pub ctx: CompCtx,
    pub items: Vec<Item>,
    pub selected: usize,
}

/// Find / replace bar state.
#[derive(Debug, Clone, Default)]
pub struct FindUi {
    pub tab: TabId,
    pub query: String,
    pub replace: String,
    pub show_replace: bool,
    pub case: bool,
    pub current: usize,
    /// Focus the query field on the next frame.
    pub focus: bool,
    /// Match to select in the editor on the next frame.
    pub reveal: Option<Range<usize>>,
}

// ---------------------------------------------------------------------------
// Layout: Live Preview styling
// ---------------------------------------------------------------------------

struct Params<'a> {
    size: f32,
    mono: bool,
    pal: Palette,
    live: bool,
    /// 0-based lines that show raw markdown (caret / selection).
    active: Range<usize>,
    /// Hide the raw YAML block (the Properties panel replaces it).
    hide_frontmatter: bool,
    resolves: &'a dyn Fn(&str) -> bool,
    /// The bold / italic Inter families are installed.
    styled: bool,
    /// Lines hidden by folds.
    hidden: &'a HashSet<usize>,
    /// Lines replaced by an inline image: row height reserved for it.
    images: &'a HashMap<usize, f32>,
    /// Table alignment of the blocks shown as a grid.
    table: &'a TableLook,
}

/// Column padding, header and row lines of the tables shown as a grid.
#[derive(Default)]
struct TableLook {
    pads: Vec<Pad>,
    rows: HashSet<usize>,
    headers: HashSet<usize>,
}

/// Split the pieces so that `r` is covered by pieces of its own, and apply
/// `f` to those.
fn restyle(
    pieces: &mut Vec<(Range<usize>, TextFormat)>,
    r: Range<usize>,
    f: impl Fn(&mut TextFormat),
) {
    if r.is_empty() {
        return;
    }
    let mut i = pieces.partition_point(|(p, _)| p.end <= r.start);
    while i < pieces.len() && pieces[i].0.start < r.end {
        let (pr, fm) = pieces[i].clone();
        let (a, b) = (pr.start.max(r.start), pr.end.min(r.end));
        let mut repl = Vec::new();
        if pr.start < a {
            repl.push((pr.start..a, fm.clone()));
        }
        let mut mid = fm.clone();
        f(&mut mid);
        repl.push((a..b, mid));
        if b < pr.end {
            repl.push((b..pr.end, fm));
        }
        let n = repl.len();
        pieces.splice(i..=i, repl);
        i += n;
    }
}

/// Measure the cells of the tables shown as grids and plan their padding.
fn table_look(
    ui: &egui::Ui,
    decor: &Decor,
    text: &str,
    active: &Range<usize>,
    size: f32,
    mono: bool,
    styled: bool,
) -> TableLook {
    let mut look = TableLook::default();
    let digit = ui.fonts_mut(|f| {
        f.layout_no_wrap("0".into(), editor_font(size, mono), Color32::WHITE)
            .size()
            .x
    });
    for (at, missing) in blocks::ordered_runs(text, &decor.lines) {
        look.pads.push(Pad {
            at,
            amount: digit * missing as f32,
        });
    }
    // `> ` of callout headers makes room for the icon.
    for (n, l) in decor.lines.iter().enumerate() {
        if matches!(l.kind, LineKind::Callout { header: true, .. }) && !active.contains(&n) {
            let src = &text[l.range.clone()];
            let gt = src.len() - src.trim_start().len();
            look.pads.push(Pad {
                at: l.range.start + gt + 1,
                amount: 7.0,
            });
        }
    }
    for b in &decor.blocks {
        let BlockKind::Table { aligns } = &b.kind else {
            continue;
        };
        if active.contains(&b.start) || active.contains(&b.end) {
            continue;
        }
        look.headers.insert(b.start);
        let mut rows = Vec::new();
        let mut widths = Vec::new();
        for n in b.start..=b.end {
            if n == b.start + 1 {
                continue;
            }
            look.rows.insert(n);
            let cs = blocks::cells(text, decor.lines[n].range.clone());
            let font = if n == b.start {
                styled_font(size, mono, styled, true, false)
            } else {
                editor_font(size, mono)
            };
            let w: Vec<f32> = cs
                .iter()
                .map(|c| {
                    let shown = blocks::shown_text(&text[c.content.clone()]);
                    ui.fonts_mut(|f| {
                        f.layout_no_wrap(shown, font.clone(), Color32::WHITE)
                            .size()
                            .x
                    })
                })
                .collect();
            rows.push(cs);
            widths.push(w);
        }
        look.pads
            .extend(blocks::table_pads(&rows, &widths, aligns, text));
    }
    look
}

/// Size of the font used for hidden markers.
const HIDDEN_SIZE: f32 = 0.1;

fn hidden_format() -> TextFormat {
    TextFormat {
        font_id: FontId::new(HIDDEN_SIZE, FontFamily::Proportional),
        color: Color32::TRANSPARENT,
        // The first glyph of a section is not affected; the following ones
        // are pulled back so that long markers leave no gap.
        extra_letter_spacing: -HIDDEN_SIZE,
        ..TextFormat::default()
    }
}

fn line_scale(kind: &LineKind, live: bool) -> f32 {
    match kind {
        LineKind::Heading(n) => heading_scale(*n, live),
        _ => 1.0,
    }
}

fn piece_format(
    style: &Style,
    line: &LineInfo,
    at: usize,
    conceal: Option<Conceal>,
    concealed: bool,
    unresolved: bool,
    p: &Params,
) -> TextFormat {
    let pal = &p.pal;
    let mut f = format_for_mode(style, p.size, p.mono, p.styled, pal, p.live);
    if concealed {
        match &line.kind {
            LineKind::Quote => f.color = pal.muted,
            LineKind::Callout { kind, header } => {
                if *header {
                    f.color = pal.callout(kind);
                } else if !style.link && !style.tag {
                    f.color = pal.text;
                }
            }
            _ => {}
        }
        if line.done && at >= line.content {
            f.color = pal.muted;
            f.strikethrough = Stroke::new(1.0, pal.muted);
        }
        if style.comment {
            f.font_id = editor_font(p.size * 0.95, p.mono);
            f.color = pal.faint;
        }
        if (style.link || style.tag) && !style.code {
            f.color = pal.accent;
        }
        if unresolved && style.link {
            f.color = pal.unresolved;
        }
    }
    if p.live && matches!(line.kind, LineKind::Code | LineKind::CodeFence) {
        // Code blocks: plain monospace text on the block background.
        f.font_id = FontId::new(p.size * 0.92, FontFamily::Monospace);
        f.background = Color32::TRANSPARENT;
        f.color = if line.kind == LineKind::Code {
            pal.text
        } else {
            pal.faint
        };
    }
    match conceal {
        Some(Conceal::Hide) => {
            f.font_id = FontId::new(HIDDEN_SIZE, FontFamily::Proportional);
            f.extra_letter_spacing = -HIDDEN_SIZE;
            f.color = Color32::TRANSPARENT;
            f.background = Color32::TRANSPARENT;
            f.strikethrough = Stroke::NONE;
            f.underline = Stroke::NONE;
        }
        Some(Conceal::Blank) => {
            f.color = Color32::TRANSPARENT;
            f.background = Color32::TRANSPARENT;
            f.strikethrough = Stroke::NONE;
            f.underline = Stroke::NONE;
        }
        None => {}
    }
    f
}

/// The styled text of the whole note. `marks` are find matches (sorted).
fn build_job(
    text: &str,
    decor: &Decor,
    p: &Params,
    marks: &[Range<usize>],
    current: usize,
) -> LayoutJob {
    let spans = highlight(text);
    let unres: Vec<&Range<usize>> = decor
        .links
        .iter()
        .filter_map(|l| match &l.target {
            LinkTarget::Wiki { target, .. } if !target.is_empty() && !(p.resolves)(target) => {
                Some(&l.range)
            }
            _ => None,
        })
        .collect();
    let mut pieces: Vec<(Range<usize>, TextFormat)> = Vec::new();
    let (mut si, mut ci, mut ui_) = (0usize, 0usize, 0usize);
    // The YAML block stays in the buffer but is invisible, unless the caret
    // is inside it.
    let hide_fm = p.live
        && p.hide_frontmatter
        && !decor
            .lines
            .iter()
            .enumerate()
            .any(|(n, l)| l.kind == LineKind::Frontmatter && p.active.contains(&n));
    for (n, line) in decor.lines.iter().enumerate() {
        let r = line.range.clone();
        while si < spans.len() && spans[si].0.end <= r.start {
            si += 1;
        }
        if hide_fm && line.kind == LineKind::Frontmatter {
            let next = decor.lines.get(n + 1).map_or(text.len(), |l| l.range.start);
            if next > r.start {
                pieces.push((r.start..next, hidden_format()));
            }
            continue;
        }
        // Lines of a folded block, and the image of an image-only line, take
        // (almost) no room / a fixed row height.
        let hidden_by_fold = p.hidden.contains(&n);
        let image_h = (p.live && !p.active.contains(&n))
            .then(|| p.images.get(&n).copied())
            .flatten();
        if hidden_by_fold || image_h.is_some() {
            let next = decor.lines.get(n + 1).map_or(text.len(), |l| l.range.start);
            if next > r.start {
                let mut f = hidden_format();
                f.line_height = image_h;
                pieces.push((r.start..next, f));
            }
            continue;
        }
        let concealed = p.live && !p.active.contains(&n);
        let line_spans = spans[si..]
            .iter()
            .take_while(|(s, _)| s.start < r.end)
            .map(|(s, st)| (s.start.max(r.start)..s.end.min(r.end), st))
            .filter(|(s, _)| !s.is_empty());
        for (rng, style) in line_spans {
            let mut push = |range: Range<usize>, conceal: Option<Conceal>| {
                while ui_ < unres.len() && unres[ui_].end <= range.start {
                    ui_ += 1;
                }
                let unresolved = unres
                    .get(ui_)
                    .is_some_and(|u| u.start <= range.start && range.start < u.end);
                let f = piece_format(style, line, range.start, conceal, concealed, unresolved, p);
                pieces.push((range, f));
            };
            if !concealed {
                push(rng, None);
                continue;
            }
            let mut pos = rng.start;
            while pos < rng.end {
                while ci < decor.conceals.len() && decor.conceals[ci].0.end <= pos {
                    ci += 1;
                }
                match decor.conceals.get(ci) {
                    Some((cr, kind, _)) if cr.start < rng.end => {
                        if cr.start > pos {
                            push(pos..cr.start, None);
                            pos = cr.start;
                        }
                        let end = cr.end.min(rng.end);
                        push(pos..end, Some(*kind));
                        pos = end;
                    }
                    _ => {
                        push(pos..rng.end, None);
                        pos = rng.end;
                    }
                }
            }
        }
        // Line break(s) up to the next line.
        let next = decor.lines.get(n + 1).map_or(text.len(), |l| l.range.start);
        if next > r.end {
            let f = TextFormat {
                font_id: editor_font(p.size * line_scale(&line.kind, p.live), p.mono),
                color: p.pal.text,
                ..TextFormat::default()
            };
            pieces.push((r.end..next, f));
        }
    }
    if p.live {
        let line_of_byte = |b: usize| decor.lines.partition_point(|l| l.range.end < b);
        let skip = |b: usize| p.hidden.contains(&line_of_byte(b));
        for r in &decor.math {
            if !skip(r.start) {
                restyle(&mut pieces, r.clone(), |f| {
                    f.font_id = FontId::new(p.size * 0.92, FontFamily::Monospace);
                    f.color = p.pal.muted;
                });
            }
        }
        for r in &decor.footnotes {
            let n = line_of_byte(r.start);
            if !p.active.contains(&n) && !p.hidden.contains(&n) {
                restyle(&mut pieces, r.start + 2..r.end - 1, |f| {
                    f.font_id = FontId::new(p.size * 0.75, FontFamily::Proportional);
                    f.valign = Align::TOP;
                    f.color = p.pal.accent;
                });
            }
        }
        for n in &p.table.rows {
            if let Some(l) = decor.lines.get(*n) {
                let bold = p.table.headers.contains(n);
                let r = l.range.clone();
                restyle(&mut pieces, r, |f| {
                    f.line_height = Some(p.size * 1.8);
                    if bold {
                        f.font_id = styled_font(p.size, p.mono, p.styled, true, false);
                        if f.color != Color32::TRANSPARENT {
                            f.color = p.pal.strong;
                        }
                    }
                });
            }
        }
        for pad in &p.table.pads {
            if skip(pad.at) {
                continue;
            }
            let len = text[pad.at..].chars().next().map_or(0, char::len_utf8);
            restyle(&mut pieces, pad.at..pad.at + len, |f| {
                f.extra_letter_spacing += pad.amount;
            });
        }
    }
    let mut job = LayoutJob::default();
    let mut k = 0;
    let mut cover = 0;
    for (r, f) in pieces {
        let mut pos = r.start;
        while pos < r.end {
            while k < marks.len() && marks[k].end <= pos {
                k += 1;
            }
            match marks.get(k) {
                Some(m) if m.start < r.end => {
                    if m.start > pos {
                        append(&mut job, text, pos..m.start, f.clone());
                        pos = m.start;
                    }
                    let end = m.end.min(r.end);
                    let mut ff = f.clone();
                    ff.background = if k == current {
                        p.pal.accent.gamma_multiply(0.55)
                    } else {
                        p.pal.highlight_bg
                    };
                    append(&mut job, text, pos..end, ff);
                    pos = end;
                }
                _ => {
                    append(&mut job, text, pos..r.end, f.clone());
                    pos = r.end;
                }
            }
        }
        cover += r.len();
    }
    if cover != text.len() || job.text.len() != text.len() {
        // Safety net: the layout text must equal the buffer.
        return layout_job(text, p.size, p.mono, p.styled, &p.pal);
    }
    job
}

fn append(job: &mut LayoutJob, text: &str, r: Range<usize>, f: TextFormat) {
    if let Some(s) = text.get(r) {
        job.append(s, 0.0, f);
    }
}

fn hash_of(t: &impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------
// Selection helpers
// ---------------------------------------------------------------------------

pub(super) fn read_sel(ctx: &egui::Context, id: egui::Id, text: &str) -> Option<Sel> {
    let st = egui::TextEdit::load_state(ctx, id)?;
    let r = st.cursor.char_range()?;
    Some(Sel::new(
        char_to_byte(text, usize::from(r.primary.index)),
        char_to_byte(text, usize::from(r.secondary.index)),
    ))
}

fn write_sel(ctx: &egui::Context, id: egui::Id, text: &str, sel: Sel) {
    let (a, b) = (byte_to_char(text, sel.start), byte_to_char(text, sel.end));
    let range = if a == b {
        CCursorRange::one(CCursor::new(a))
    } else {
        CCursorRange::two(CCursor::new(a), CCursor::new(b))
    };
    let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(range));
    egui::TextEdit::store_state(ctx, id, st);
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

fn items_for(state: &AppState, text: &str, from: &NotePath, ctx: &CompCtx) -> Vec<Item> {
    match &ctx.kind {
        CompKind::Link => state
            .index
            .fuzzy_find_files(&ctx.query, 8, ctx.embed)
            .into_iter()
            .map(|(p, alias, _)| {
                let link = state.index.link_text_for(&p, from);
                let (label, insert) = match alias {
                    Some(a) => (format!("{a}  →  {}", p.stem()), format!("{link}|{a}")),
                    None if p.is_markdown() => (p.stem().to_string(), link),
                    None => (p.file_name().to_string(), link),
                };
                Item {
                    label,
                    detail: p.as_str().to_string(),
                    insert,
                    target: Some(p),
                }
            })
            .collect(),
        CompKind::Heading { note } => {
            let target = if note.is_empty() {
                Some(from.clone())
            } else {
                state.index.resolve(note, from)
            };
            let Some(target) = target else {
                return Vec::new();
            };
            let parsed = if &target == from {
                state.parsed_with_text(from, text)
            } else {
                state.parsed(&target)
            };
            let q = ctx.query.to_lowercase();
            parsed
                .headings
                .iter()
                .filter(|h| h.text.to_lowercase().contains(&q))
                .take(10)
                .map(|h| Item {
                    label: h.text.clone(),
                    detail: format!("H{}", h.level),
                    insert: completion::heading_for_link(&h.text),
                    target: None,
                })
                .collect()
        }
        CompKind::Block { note } => {
            let target = if note.is_empty() {
                Some(from.clone())
            } else {
                state.index.resolve(note, from)
            };
            let Some(target) = target else {
                return Vec::new();
            };
            let (parsed, source) = if &target == from {
                (state.parsed_with_text(from, text), text.to_string())
            } else {
                (
                    state.parsed(&target),
                    state.note_text(&target).unwrap_or_default(),
                )
            };
            let blocks: Vec<(String, usize)> = parsed
                .block_ids
                .iter()
                .map(|b| (b.id.clone(), b.line))
                .collect();
            completion::block_items(&source, &blocks, &ctx.query, 10)
                .into_iter()
                .map(|b| Item {
                    label: if b.excerpt.is_empty() {
                        format!("^{}", b.id)
                    } else {
                        format!("{}  ^{}", b.excerpt, b.id)
                    },
                    detail: format!("^{}", b.id),
                    insert: b.id,
                    target: None,
                })
                .collect()
        }
        CompKind::Tag => state
            .index
            .tag_suggestions(&ctx.query, 10)
            .into_iter()
            .map(|(t, n)| {
                let t = t.trim_start_matches('#').to_string();
                Item {
                    label: format!("#{t}"),
                    detail: n.to_string(),
                    insert: t,
                    target: None,
                }
            })
            .collect(),
    }
}

/// Insert the selected suggestion; returns the new caret.
fn accept(text: &mut String, state: &AppState, from: &NotePath, c: &CompletionUi) -> Option<Sel> {
    let item = c
        .items
        .get(c.selected.min(c.items.len().saturating_sub(1)))?;
    Some(match &c.ctx.kind {
        CompKind::Tag => completion::accept_tag(text, &c.ctx, &item.insert),
        CompKind::Link if state.use_markdown_links() && item.target.is_some() => {
            let markup = state.link_markup(item.target.as_ref()?, from, c.ctx.embed);
            completion::accept_replacing_link(text, &c.ctx, &markup)
        }
        _ => completion::accept_in_link(text, &c.ctx, &item.insert),
    })
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// Percent-decode a markdown link destination.
fn decode_target(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn edit_view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    tab_id: TabId,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    // The body works on the text taken out of the buffer, so that `state`
    // stays freely usable (index lookups, attachments...).
    let Some(mut text) = state
        .buffers
        .get_mut(path)
        .map(|b| std::mem::take(&mut b.text))
    else {
        ui.label("Cannot read this file.");
        return;
    };
    let edited = edit_body(ui, state, caches, tab_id, path, &mut text, actions);
    if let Some(b) = state.buffers.get_mut(path) {
        b.text = text;
    }
    if edited {
        actions.push(Action::Edited(path.clone()));
    }
}

fn edit_body(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    tab_id: TabId,
    path: &NotePath,
    text: &mut String,
    actions: &mut Vec<Action>,
) -> bool {
    let ctx = ui.ctx().clone();
    let pal = Palette::of(&ctx);
    let size = state.settings.font_size;
    let mono = state.settings.monospace_editor;
    let readable = state.settings.readable_line_length;
    let live = state
        .editor
        .live(tab_id, state.settings.default_editing_mode);
    let id = egui::Id::unique(("editor", tab_id));
    let mut edited = false;
    let mut scroll = false;
    let mut want_focus = false;

    let pending_line = state
        .active_tab_mut()
        .and_then(|t| t.note_mut())
        .and_then(|n| n.pending_line.take());

    let sel = read_sel(&ctx, id, text);
    let mut sel_changed = false;
    let mut cur = sel.unwrap_or_default();

    // ---- dropped files become attachments -----------------------------
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    for f in dropped {
        let name = f
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "file".to_string());
        let bytes = f.bytes().ok();
        let Some(bytes) = bytes else {
            state.error(format!("Cannot read dropped file {name}"));
            continue;
        };
        match state.add_attachment(path, &name, &bytes) {
            Ok(p) => {
                let markup = state.link_markup(&p, path, true);
                cur = fmt::insert(text, cur, &format!("{markup}\n"));
                sel_changed = true;
                edited = true;
            }
            Err(e) => state.error(format!("Cannot add attachment: {e}")),
        }
    }

    // ---- pending editing commands -------------------------------------
    for cmd in std::mem::take(&mut state.editor.pending) {
        match cmd {
            Command::FindInNote | Command::ReplaceInNote => {
                let selected = text.get(cur.start..cur.end).unwrap_or("");
                let mut f = match caches.find.take() {
                    Some(f) if f.tab == tab_id => f,
                    _ => FindUi {
                        tab: tab_id,
                        ..FindUi::default()
                    },
                };
                if !selected.is_empty() && !selected.contains('\n') {
                    f.query = selected.to_string();
                }
                f.focus = true;
                f.show_replace = cmd == Command::ReplaceInNote || f.show_replace;
                caches.find = Some(f);
            }
            Command::FoldAll => {
                let d = decorate(text);
                let keys = fold::all_keys(text, &d.lines, &d.folds);
                state.editor.folds.insert(tab_id, keys);
                // Folded lines cannot hold the caret.
                cur = Sel::caret(fold_safe_caret(text, &d, cur.start));
                sel_changed = true;
            }
            Command::UnfoldAll => {
                state.editor.folds.remove(&tab_id);
            }
            c => {
                if let Some(new) = run_command(c, text, cur) {
                    cur = new;
                    sel_changed = true;
                    edited = true;
                    want_focus = true;
                }
            }
        }
    }

    // ---- find: reveal a match ------------------------------------------
    if let Some(f) = caches.find.as_mut().filter(|f| f.tab == tab_id)
        && let Some(r) = f.reveal.take()
        && r.end <= text.len()
    {
        cur = Sel::new(r.start, r.end);
        sel_changed = true;
        scroll = true;
    }

    if let Some(line) = pending_line {
        let b = line_start_byte(text, line);
        cur = Sel::caret(b);
        sel_changed = true;
        want_focus = true;
        scroll = true;
    }

    // A click on the heading navigator: caret at the heading, scrolled to
    // the top.
    let mut align = Align::Center;
    if let Some(line) = caches.nav.take_jump(tab_id) {
        cur = Sel::caret(line_start_byte(text, line));
        sel_changed = true;
        want_focus = true;
        scroll = true;
        align = Align::TOP;
    }

    // ---- keys that must be handled before the TextEdit sees them -------
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut comp_accepted = false;
    if focused {
        if let Some(c) = caches.completion.as_mut().filter(|c| c.tab == tab_id)
            && !c.items.is_empty()
        {
            let n = c.items.len();
            let (mut down, mut up, mut ok, mut esc) = (false, false, false, false);
            ui.input_mut(|i| {
                down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
                ok = i.consume_key(Modifiers::NONE, Key::Enter)
                    || i.consume_key(Modifiers::NONE, Key::Tab);
                esc = i.consume_key(Modifiers::NONE, Key::Escape);
            });
            if down {
                c.selected = (c.selected + 1) % n;
            }
            if up {
                c.selected = (c.selected + n - 1) % n;
            }
            if ok {
                let c = c.clone();
                if let Some(new) = accept(text, state, path, &c) {
                    cur = new;
                    sel_changed = true;
                    edited = true;
                    comp_accepted = true;
                }
                caches.completion = None;
            } else if esc {
                caches.completion = None;
            }
        }
        if !comp_accepted {
            let mut enter = false;
            let mut tab = false;
            let mut shift_tab = false;
            ui.input(|i| {
                enter = i
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.is_none()));
                tab = i
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Key { key: Key::Tab, pressed: true, modifiers, .. } if modifiers.is_none()));
                shift_tab = i
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Key { key: Key::Tab, pressed: true, modifiers, .. } if modifiers.shift && !modifiers.command && !modifiers.alt));
            });
            if enter && let Some(new) = fmt::on_enter(text, cur) {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                cur = new;
                sel_changed = true;
                edited = true;
            } else if tab && fmt::in_list(text, cur) {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
                cur = fmt::indent_lines(text, cur);
                sel_changed = true;
                edited = true;
            } else if shift_tab && let Some(new) = fmt::outdent_lines(text, cur) {
                ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab));
                cur = new;
                sel_changed = true;
                edited = true;
            }
            // Auto-pairing of brackets/quotes and paired backspace; only
            // when this frame holds a single, unambiguous input event.
            let (typed, backspace) = ctx.input(|i| {
                let texts: Vec<&String> = i
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        Event::Text(s) => Some(s),
                        _ => None,
                    })
                    .collect();
                let edits_text = i.events.iter().any(|e| {
                    matches!(
                        e,
                        Event::Key {
                            key: Key::Delete
                                | Key::Enter
                                | Key::Tab
                                | Key::ArrowLeft
                                | Key::ArrowRight
                                | Key::ArrowUp
                                | Key::ArrowDown
                                | Key::Home
                                | Key::End,
                            pressed: true,
                            ..
                        } | Event::Paste(_)
                            | Event::Cut
                            | Event::Ime(_)
                    )
                });
                let bs = i.events.iter().any(|e| {
                    matches!(
                        e,
                        Event::Key { key: Key::Backspace, pressed: true, modifiers, .. }
                            if modifiers.is_none()
                    )
                });
                let typed = match texts.as_slice() {
                    [t] if !edits_text && !bs && t.chars().count() == 1 => t.chars().next(),
                    _ => None,
                };
                (typed, bs && texts.is_empty() && !edits_text)
            });
            if let Some(ch) = typed
                && let Some(new) = fmt::auto_pair(text, cur, ch)
            {
                ui.input_mut(|i| i.events.retain(|e| !matches!(e, Event::Text(_))));
                cur = new;
                sel_changed = true;
                edited = true;
            } else if backspace && let Some(new) = fmt::backspace_pair(text, cur) {
                ui.input_mut(|i| {
                    i.events.retain(|e| {
                        !matches!(
                            e,
                            Event::Key {
                                key: Key::Backspace,
                                ..
                            }
                        )
                    })
                });
                cur = new;
                sel_changed = true;
                edited = true;
            }
            // Smart paste: a URL pasted over a selection makes a link.
            let pasted = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    Event::Paste(s) => Some(s.clone()),
                    _ => None,
                })
            });
            if let Some(p) = pasted
                && let Some(new) = fmt::smart_paste(text, cur, &p)
            {
                ui.input_mut(|i| i.events.retain(|e| !matches!(e, Event::Paste(_))));
                cur = new;
                sel_changed = true;
                edited = true;
            }
        }
    }

    if sel_changed {
        write_sel(&ctx, id, text, cur);
    }
    if want_focus || scroll {
        ctx.memory_mut(|m| m.request_focus(id));
    }

    // ---- layout ---------------------------------------------------------
    let active_raw = {
        let r = read_sel(&ctx, id, text).unwrap_or(Sel::caret(0));
        if ctx.memory(|m| m.has_focus(id)) || read_sel(&ctx, id, text).is_some() {
            line_of(text, r.start)..line_of(text, r.end) + 1
        } else {
            0..0
        }
    };
    // Code blocks and tables show their raw markdown as a whole when the
    // caret is inside; the initial caret at the very start does not reveal
    // the frontmatter.
    let mut active = active_raw.clone();
    if live && let Some((_, d)) = caches.decor.get(&tab_id) {
        active = blocks::expand_active(active, &d.blocks);
        if active_raw == (0..1)
            && read_sel(&ctx, id, text) == Some(Sel::caret(0))
            && d.lines
                .first()
                .is_some_and(|l| l.kind == LineKind::Frontmatter)
        {
            active = 0..0;
        }
    }
    let find_params = caches
        .find
        .as_ref()
        .filter(|f| f.tab == tab_id && !f.query.is_empty())
        .map(|f| (f.query.clone(), f.case, f.current));
    let generation = state.generation;
    let hide_frontmatter = crate::properties_model::frontmatter_hidden(&state.settings);
    let styled = has_styled_fonts(&ctx);
    let show_numbers = state.settings.show_line_numbers;
    // A caret inside a folded block opens it.
    if !active.is_empty()
        && let Some(keys) = state.editor.folds.get(&tab_id).filter(|k| !k.is_empty())
        && let Some((_, d)) = caches.decor.get(&tab_id)
    {
        let mut left = keys.clone();
        for line in [active.start, active.end.saturating_sub(1)] {
            left = fold::reveal_line(text, &d.lines, &d.folds, &left, line);
        }
        if left.len() != keys.len() {
            state.editor.folds.insert(tab_id, left);
        }
    }
    // `> [!type]-` callouts start folded.
    if live && state.editor.callout_seeded.insert((tab_id, path.clone())) {
        let d = decorate(text);
        let keys: Vec<fold::FoldKey> = d
            .folds
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.kind == fold::FoldKind::Callout
                    && d.lines
                        .get(f.start)
                        .and_then(|l| text.get(l.range.clone()))
                        .and_then(fold::callout_marker)
                        == Some('-')
            })
            .map(|(i, _)| fold::key_of(text, &d.lines, &d.folds, i))
            .collect();
        if !keys.is_empty() {
            state.editor.folds.entry(tab_id).or_default().extend(keys);
        }
    }
    let fold_keys: Vec<fold::FoldKey> =
        state.editor.folds.get(&tab_id).cloned().unwrap_or_default();
    caches.embeds.sync(generation);
    let index: &dyn IndexBackend = &*state.index;
    let vault: &dyn ors_core::VaultRead = &*state.vault;
    let from = path.clone();
    let resolves = |t: &str| index.resolve(t, &from).is_some();
    let decor_cache = &mut caches.decor;
    let job_cache = &mut caches.highlight;
    let embed_cache = &mut caches.embeds;
    let active_for_layout = active.clone();
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let t = buf.as_str();
        let th = hash_of(&t);
        let decor = match decor_cache.get(&tab_id) {
            Some((k, d)) if *k == th => d.clone(),
            _ => {
                let d = Rc::new(decorate(t));
                decor_cache.insert(tab_id, (th, d.clone()));
                d
            }
        };
        let mut hidden: HashSet<usize> = if fold_keys.is_empty() {
            HashSet::new()
        } else {
            let idx = fold::folded_indices(t, &decor.lines, &decor.folds, &fold_keys);
            fold::hidden_lines(&decor.folds, &idx)
        };
        if live {
            hidden.extend(blocks::collapsed_lines(&decor.blocks, &active_for_layout));
        }
        let mut images: HashMap<usize, f32> = HashMap::new();
        if live {
            let placed = embed_cache.layout(
                tab_id,
                index,
                vault,
                &from,
                &decor.images,
                |l| active_for_layout.contains(&l) || hidden.contains(&l),
                wrap_width,
            );
            for pl in placed {
                images.insert(pl.line, edit_deco::image_row_height(pl.size.y));
            }
        } else {
            embed_cache.placed.remove(&tab_id);
        }
        let mut image_key: Vec<(usize, u32)> =
            images.iter().map(|(l, h)| (*l, h.to_bits())).collect();
        image_key.sort_unstable();
        let key = hash_of(&(
            (
                th,
                size.to_bits(),
                mono,
                styled,
                pal.text,
                live,
                hide_frontmatter,
            ),
            (active_for_layout.start, active_for_layout.end, generation),
            (&find_params, &fold_keys, &image_key),
        ));
        let mut job = match job_cache.get(&tab_id) {
            Some((k, job)) if *k == key => job.clone(),
            _ => {
                let marks = find_params
                    .as_ref()
                    .map(|(q, case, _)| find_all(t, q, *case))
                    .unwrap_or_default();
                let table = if live {
                    table_look(ui, &decor, t, &active_for_layout, size, mono, styled)
                } else {
                    TableLook::default()
                };
                let params = Params {
                    size,
                    mono,
                    pal,
                    live,
                    active: active_for_layout.clone(),
                    hide_frontmatter,
                    resolves: &resolves,
                    styled,
                    hidden: &hidden,
                    images: &images,
                    table: &table,
                };
                let cur = find_params.as_ref().map_or(usize::MAX, |p| p.2);
                let job = build_job(t, &decor, &params, &marks, cur);
                job_cache.insert(tab_id, (key, job.clone()));
                job
            }
        };
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };

    let line_count = text.matches('\n').count() + 1;
    let left_extra = edit_deco::ARROW_GUTTER
        + if show_numbers {
            edit_deco::number_gutter_width(line_count, size)
        } else {
            0.0
        };
    let backdrop_idx = ui.painter().add(egui::Shape::Noop);
    let output = readable_column_with(ui, readable, left_extra, |ui| {
        ui.add_space(8.0);
        egui::TextEdit::multiline(text)
            .id(id)
            .frame(Frame::NONE)
            .desired_width(f32::INFINITY)
            .desired_rows(30)
            .lock_focus(true)
            .font(editor_font(size, mono))
            .layouter(&mut layouter)
            .show(ui)
    });
    let changed = output.response.changed();
    edited |= changed;

    let text_ref: &str = text;
    let decor: Rc<Decor> = caches
        .decor
        .get(&tab_id)
        .map(|(_, d)| d.clone())
        .unwrap_or_default();
    let galley = output.galley.clone();
    let origin = output.galley_pos;
    let cursor_rect = output.cursor_range.map(|r| {
        galley
            .pos_from_cursor(r.primary)
            .translate(origin.to_vec2())
    });
    if scroll && let Some(rect) = cursor_rect {
        ui.scroll_to_rect(rect.expand(40.0), Some(align));
    }

    // The caret moved to another line: restyle on the next frame.
    if let Some(r) = output.cursor_range {
        let (a, b) = (
            line_of(
                text_ref,
                char_to_byte(text_ref, usize::from(r.primary.index)),
            ),
            line_of(
                text_ref,
                char_to_byte(text_ref, usize::from(r.secondary.index)),
            ),
        );
        if (a.min(b)..a.max(b) + 1) != active_raw {
            ctx.request_repaint();
        }
    }

    let folded_idx = fold::folded_indices(text_ref, &decor.lines, &decor.folds, &fold_keys);
    let fold_hidden = fold::hidden_lines(&decor.folds, &folded_idx);
    // The decorations were computed for this very text (else skip them).
    let decor_fresh = caches
        .decor
        .get(&tab_id)
        .is_some_and(|(k, _)| *k == hash_of(&text_ref));
    let mut hidden = fold_hidden.clone();
    if live && hide_frontmatter && decor_fresh {
        for (n, l) in decor.lines.iter().enumerate() {
            if l.kind == LineKind::Frontmatter && !active.contains(&n) {
                hidden.insert(n);
            }
        }
    }
    if live && decor_fresh {
        hidden.extend(blocks::collapsed_lines(&decor.blocks, &active));
    }
    // ---- Live Preview widgets ------------------------------------------
    let toggle_line = if live && decor_fresh {
        let geo = edit_blocks::Geo::new(&galley, origin.to_vec2(), &decor, text_ref);
        let look = edit_blocks::Look {
            pal,
            size,
            mono,
            styled,
            width: output.response.rect.width(),
            clip: ui.clip_rect(),
        };
        let shapes = edit_blocks::backdrop(&geo, &active, &fold_hidden, &look);
        ui.painter().set(backdrop_idx, egui::Shape::Vec(shapes));
        edit_blocks::overlays(ui, &geo, &active, &fold_hidden, &look);
        paint_widgets(ui, &geo, &output, &active, &hidden, &pal, tab_id)
    } else {
        None
    };

    // ---- gutter, folds and inline images --------------------------------
    {
        let rows = edit_deco::line_rows(&galley);
        if let Some(hs) = caches.nav.cached(tab_id) {
            caches.nav.ys = Some(
                hs.iter()
                    .map(|h| {
                        rows.get(h.line)
                            .filter(|_| !hidden.contains(&h.line))
                            .map(|r| origin.y + r.top)
                    })
                    .collect(),
            );
        }
        if show_numbers {
            let caret_line = output.cursor_range.map(|r| {
                line_of(
                    text_ref,
                    char_to_byte(text_ref, usize::from(r.primary.index)),
                )
            });
            edit_deco::paint_line_numbers(ui, &rows, origin, &hidden, caret_line, size, &pal);
        }
        if let Some(start) = edit_deco::fold_controls(
            ui,
            tab_id,
            &decor.folds,
            &folded_idx,
            &hidden,
            &rows,
            origin,
            output.response.rect.width(),
            &pal,
        ) {
            let keys = fold::toggle(text_ref, &decor.lines, &decor.folds, &fold_keys, start);
            state.editor.folds.insert(tab_id, keys);
            ctx.request_repaint();
        }
        let placed = caches
            .embeds
            .placed
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        let clip = ui.clip_rect();
        for pl in placed {
            if active.contains(&pl.line) || hidden.contains(&pl.line) {
                continue;
            }
            let Some(r) = rows.get(pl.line) else { continue };
            let top = origin.y + r.top + edit_deco::IMAGE_PAD;
            let rect = Rect::from_min_size(Pos2::new(origin.x, top), pl.size);
            if rect.bottom() < clip.top() || rect.top() > clip.bottom() {
                continue;
            }
            if let Some(bytes) = caches.image_bytes(state, &pl.path) {
                egui::Image::from_bytes(format!("bytes://{}", pl.path.as_str()), bytes)
                    .corner_radius(4)
                    .paint_at(ui, rect);
            }
        }
    }

    // ---- ctrl+click on links ---------------------------------------------
    let mods = ui.input(|i| i.modifiers);
    if mods.command {
        if let Some(pos) = output.response.hover_pos() {
            let byte = byte_at(&galley, origin, text_ref, pos);
            if decor.link_at(byte).is_some() {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
            }
        }
        if output.response.clicked()
            && let Some(pos) = output.response.interact_pointer_pos()
        {
            let byte = byte_at(&galley, origin, text_ref, pos);
            if let Some(link) = decor.link_at(byte) {
                let how = if mods.alt {
                    OpenIn::NewTab
                } else {
                    OpenIn::CurrentTab
                };
                match &link.target {
                    LinkTarget::Wiki { target, subpath } => {
                        if target.is_empty() {
                            // `[[#Heading]]`: jump inside this note.
                            if let Some(h) = subpath {
                                let parsed = state.parsed_with_text(path, text_ref);
                                if let Some(hd) = parsed
                                    .headings
                                    .iter()
                                    .find(|x| x.text.eq_ignore_ascii_case(h))
                                {
                                    actions.push(Action::OpenLine(path.clone(), hd.line, how));
                                }
                            }
                        } else if let Some((pdf, page)) =
                            state.pdf_link(target, subpath.as_deref(), path)
                        {
                            actions.push(Action::OpenPdf(pdf, page, how));
                        } else {
                            actions.push(Action::Follow {
                                target: target.clone(),
                                from: path.clone(),
                                open_in: how,
                            });
                        }
                    }
                    LinkTarget::Markdown(dest) => {
                        if dest.contains("://") || dest.starts_with("mailto:") {
                            ctx.open_url(egui::OpenUrl::new_tab(dest.clone()));
                        } else {
                            let d = decode_target(dest);
                            let (d, frag) = match d.split_once('#') {
                                Some((d, f)) => (d, Some(f)),
                                None => (&*d, None),
                            };
                            let d = d.strip_suffix(".md").unwrap_or(d);
                            if let Some((pdf, page)) = state.pdf_link(d, frag, path) {
                                actions.push(Action::OpenPdf(pdf, page, how));
                            } else {
                                actions.push(Action::Follow {
                                    target: d.to_string(),
                                    from: path.clone(),
                                    open_in: how,
                                });
                            }
                        }
                    }
                    LinkTarget::Tag(t) => actions.push(Action::SearchTag(t.clone())),
                }
            }
        }
    }

    // ---- completion popup ------------------------------------------------
    let has_focus = output.response.has_focus();
    if changed && !comp_accepted {
        caches.completion = output
            .cursor_range
            .filter(|r| r.is_empty())
            .and_then(|r| {
                completion::detect(
                    text_ref,
                    char_to_byte(text_ref, usize::from(r.primary.index)),
                )
            })
            .and_then(|c| {
                let items = items_for(state, text_ref, path, &c);
                (!items.is_empty()).then_some(CompletionUi {
                    tab: tab_id,
                    ctx: c,
                    items,
                    selected: 0,
                })
            });
    } else if caches.completion.as_ref().is_some_and(|c| c.tab == tab_id) {
        let moved = output.cursor_range.is_none_or(|r| {
            let b = char_to_byte(text_ref, usize::from(r.primary.index));
            caches
                .completion
                .as_ref()
                .is_some_and(|c| b < c.ctx.start || b > c.ctx.end)
        });
        if !has_focus || moved {
            caches.completion = None;
        }
    }
    if let (Some(c), Some(rect)) = (caches.completion.as_mut(), cursor_rect)
        && c.tab == tab_id
        && !c.items.is_empty()
    {
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("link-completion"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.left_bottom() + egui::vec2(0.0, 4.0))
            .show(&ctx, |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(260.0);
                    for (i, it) in c.items.iter().enumerate() {
                        let r = ui.selectable_label(i == c.selected, &it.label);
                        if r.clicked() {
                            clicked = Some(i);
                        }
                        r.on_hover_text(&it.detail);
                    }
                    ui.label(
                        egui::RichText::new("↑↓ to navigate · ↵ to insert · esc to dismiss")
                            .small()
                            .color(pal.faint),
                    );
                });
            });
        if let Some(i) = clicked {
            c.selected = i;
            let c = c.clone();
            if let Some(new) = accept(text, state, path, &c) {
                write_sel(&ctx, id, text, new);
                ctx.memory_mut(|m| m.request_focus(id));
                edited = true;
            }
            caches.completion = None;
        }
    }
    if let Some(line) = toggle_line {
        *text = fmt::toggle_task_at(text, line);
        edited = true;
    }
    edited
}

/// Start of the outermost header line hiding the caret (all blocks folded).
fn fold_safe_caret(text: &str, d: &Decor, at: usize) -> usize {
    let mut line = line_of(text, at).min(d.lines.len().saturating_sub(1));
    for f in &d.folds {
        if f.start < line && line <= f.end {
            line = f.start;
        }
    }
    d.lines
        .get(line)
        .map_or(at, |l| l.range.start.min(at.max(l.range.start)))
}

/// Byte offset under screen position `pos`.
fn byte_at(galley: &egui::Galley, origin: Pos2, text: &str, pos: Pos2) -> usize {
    let c = galley.cursor_from_pos(pos - origin);
    char_to_byte(text, usize::from(c.index))
}

/// Paint bullets, checkboxes, rules, quote bars and callout tints over the
/// blank glyphs of lines that do not hold the caret. Returns the line of a
/// checkbox that was clicked.
#[allow(clippy::too_many_arguments)]
fn paint_widgets(
    ui: &mut egui::Ui,
    geo: &edit_blocks::Geo,
    output: &egui::text_edit::TextEditOutput,
    active: &Range<usize>,
    hidden: &HashSet<usize>,
    pal: &Palette,
    tab_id: TabId,
) -> Option<usize> {
    let mut toggled = None;
    let decor = geo.decor;
    let off = geo.off;
    let clip = ui.clip_rect();
    let width = output.response.rect.width();
    let rect_at = |n: usize, byte: usize| geo.rect_at(n, byte);
    // Cheap check first: exact glyph positions walk the whole galley.
    let on_screen = |n: usize| {
        let (y0, y1) = geo.span(n);
        y1 >= clip.top() - 200.0 && y0 <= clip.bottom() + 200.0
    };
    let painter = ui.painter().clone();
    for (n, l) in decor.lines.iter().enumerate() {
        if l.kind != LineKind::Rule || active.contains(&n) || hidden.contains(&n) {
            continue;
        }
        if !on_screen(n) {
            continue;
        }
        let (y0, y1) = geo.span(n);
        {
            let y = (y0 + y1) / 2.0;
            painter.line_segment(
                [Pos2::new(off.x, y), Pos2::new(off.x + width, y)],
                Stroke::new(1.5, pal.border.gamma_multiply(1.6)),
            );
        }
    }
    let depths = edit_blocks::depth_map(decor);
    for (w, n) in &decor.widgets {
        if active.contains(n) || hidden.contains(n) {
            continue;
        }
        match w {
            Widget::Bullet { .. } | Widget::Checkbox { .. } if !on_screen(*n) => {}
            Widget::Bullet { at } => {
                let a = rect_at(*n, *at);
                let b = rect_at(*n, *at + 1);
                if a.bottom() < clip.top() || a.top() > clip.bottom() {
                    continue;
                }
                let c = Pos2::new((a.left() + b.left()) / 2.0, a.center().y);
                edit_blocks::paint_bullet(
                    &painter,
                    c,
                    (a.height() * 0.085).max(2.0),
                    depths.get(n).copied().unwrap_or(0),
                    pal.text,
                );
            }
            Widget::Checkbox {
                range,
                checked,
                status_at: _,
            } => {
                let a = rect_at(*n, range.start);
                let b = rect_at(*n, range.end);
                if a.bottom() < clip.top() || a.top() > clip.bottom() {
                    continue;
                }
                let s = (a.height() * 0.7).clamp(10.0, 20.0);
                let cx = (a.left() + b.left()) / 2.0;
                let r = Rect::from_center_size(Pos2::new(cx, a.center().y), egui::vec2(s, s));
                let resp = ui.interact(
                    r.expand(3.0),
                    egui::Id::unique(("task-box", tab_id, *n)),
                    Sense::click(),
                );
                if *checked {
                    painter.rect_filled(r, 4.0, pal.accent);
                    let p = |fx: f32, fy: f32| {
                        Pos2::new(r.left() + r.width() * fx, r.top() + r.height() * fy)
                    };
                    painter.line_segment(
                        [p(0.25, 0.52), p(0.43, 0.70)],
                        Stroke::new(1.8, Color32::WHITE),
                    );
                    painter.line_segment(
                        [p(0.43, 0.70), p(0.76, 0.30)],
                        Stroke::new(1.8, Color32::WHITE),
                    );
                } else {
                    let col = if resp.hovered() {
                        pal.accent
                    } else {
                        pal.muted
                    };
                    painter.rect_stroke(r, 4.0, Stroke::new(1.4, col), egui::StrokeKind::Inside);
                }
                if resp.hovered() {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    toggled = Some(*n);
                }
            }
            Widget::Embed { .. } => {}
        }
    }
    toggled
}

// ---------------------------------------------------------------------------
// Find / replace bar
// ---------------------------------------------------------------------------

/// The find bar above the editor (no-op when closed).
pub fn find_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    tab_id: TabId,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    let Some(f) = caches.find.as_mut() else {
        return;
    };
    if f.tab != tab_id {
        caches.find = None;
        return;
    }
    let pal = Palette::of(ui.ctx());
    let Some(text) = state.buffers.get(path).map(|b| b.text.clone()) else {
        return;
    };
    let mut matches = find_all(&text, &f.query, f.case);
    if f.current >= matches.len() {
        f.current = 0;
    }
    let find_id = egui::Id::unique(("find-input", tab_id));
    let mut close = false;
    let mut step: Option<bool> = None;
    let mut reveal_current = false;
    let mut new_text: Option<String> = None;
    Frame::new()
        .fill(pal.bg_secondary)
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut f.query)
                        .id(find_id)
                        .hint_text("Find")
                        .desired_width(240.0),
                );
                if f.focus {
                    resp.request_focus();
                    f.focus = false;
                }
                if resp.changed() {
                    matches = find_all(&text, &f.query, f.case);
                    f.current = 0;
                    reveal_current = true;
                }
                if resp.has_focus() || resp.lost_focus() {
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    } else if ui.input(|i| i.key_pressed(Key::Enter)) {
                        step = Some(!ui.input(|i| i.modifiers.shift));
                        resp.request_focus();
                    }
                }
                let label = if f.query.is_empty() {
                    String::new()
                } else if matches.is_empty() {
                    "No matches".to_string()
                } else {
                    format!("{} of {}", f.current + 1, matches.len())
                };
                ui.label(egui::RichText::new(label).color(pal.muted));
                if ui
                    .small_button("↑")
                    .on_hover_text("Previous match")
                    .clicked()
                {
                    step = Some(false);
                }
                if ui.small_button("↓").on_hover_text("Next match").clicked() {
                    step = Some(true);
                }
                if ui
                    .selectable_label(f.case, "Aa")
                    .on_hover_text("Match case")
                    .clicked()
                {
                    f.case = !f.case;
                    matches = find_all(&text, &f.query, f.case);
                    f.current = 0;
                }
                if ui
                    .selectable_label(f.show_replace, "Replace")
                    .on_hover_text("Search & replace (Ctrl+H)")
                    .clicked()
                {
                    f.show_replace = !f.show_replace;
                }
                if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                    close = true;
                }
            });
            if f.show_replace {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut f.replace)
                            .hint_text("Replace with")
                            .desired_width(240.0),
                    );
                    if ui.button("Replace").clicked()
                        && let Some(m) = matches.get(f.current)
                    {
                        let mut t = text.clone();
                        t.replace_range(m.clone(), &f.replace);
                        new_text = Some(t);
                    }
                    if ui.button("Replace all").clicked() {
                        let mut t = text.clone();
                        let n = replace_all(&mut t, &f.query, &f.replace, f.case);
                        if n > 0 {
                            new_text = Some(t);
                            state.info(format!("Replaced {n} occurrence(s)"));
                        }
                    }
                });
            }
        });
    if let Some(fwd) = step
        && !matches.is_empty()
    {
        let from = matches.get(f.current).map_or(0, |m| m.start);
        f.current = if fwd {
            (f.current + 1) % matches.len()
        } else {
            (f.current + matches.len() - 1) % matches.len()
        };
        let _ = (from, next_match(&matches, 0, true));
        reveal_current = true;
    }
    if reveal_current {
        f.reveal = matches.get(f.current).cloned();
    }
    if let Some(t) = new_text
        && let Some(b) = state.buffers.get_mut(path)
    {
        b.text = t;
        actions.push(Action::Edited(path.clone()));
    }
    if close {
        caches.find = None;
        ui.ctx()
            .memory_mut(|m| m.request_focus(egui::Id::unique(("editor", tab_id))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_targets() {
        assert_eq!(decode_target("Mes%20notes%C3%A9"), "Mes notesé");
        assert_eq!(decode_target("100%"), "100%");
    }

    static NO_LINES: std::sync::LazyLock<HashSet<usize>> = std::sync::LazyLock::new(HashSet::new);
    static NO_IMAGES: std::sync::LazyLock<HashMap<usize, f32>> =
        std::sync::LazyLock::new(HashMap::new);
    static NO_TABLE: std::sync::LazyLock<TableLook> = std::sync::LazyLock::new(TableLook::default);

    fn params<'a>(active: Range<usize>, resolves: &'a dyn Fn(&str) -> bool) -> Params<'a> {
        Params {
            size: 15.0,
            mono: false,
            pal: Palette::dark(),
            live: true,
            active,
            hide_frontmatter: false,
            resolves,
            styled: false,
            hidden: &NO_LINES,
            images: &NO_IMAGES,
            table: &NO_TABLE,
        }
    }

    #[test]
    fn job_text_equals_buffer_and_hides_markers() {
        let t = "# Idées\n\n**gras** et [[Note|alias]] #tag\n- [ ] tâche\n> cité\n---\n";
        let d = decorate(t);
        let yes = |_: &str| true;
        let job = build_job(t, &d, &params(0..0, &yes), &[], 0);
        assert_eq!(job.text, t);
        // The `# ` of the heading is hidden (tiny, transparent) ...
        let first = &job.sections[0];
        assert_eq!(first.format.color, Color32::TRANSPARENT);
        assert!(first.format.font_id.size < 1.0);
        // ... but shown when the caret is on that line.
        let job = build_job(t, &d, &params(0..1, &yes), &[], 0);
        assert_eq!(job.text, t);
        assert_ne!(job.sections[0].format.color, Color32::TRANSPARENT);
    }

    #[test]
    fn frontmatter_is_invisible_when_hidden() {
        let t = "---\ntags: [a]\n---\n# Titre\n";
        let d = decorate(t);
        let yes = |_: &str| true;
        let mut p = params(3..3, &yes);
        p.hide_frontmatter = true;
        let job = build_job(t, &d, &p, &[], 0);
        assert_eq!(job.text, t);
        let fm_end = t.find("# Titre").unwrap_or(0);
        for s in job
            .sections
            .iter()
            .filter(|s| usize::from(s.byte_range.start) < fm_end)
        {
            assert_eq!(s.format.color, Color32::TRANSPARENT);
            assert!(s.format.font_id.size < 1.0);
        }
        // The heading after the block stays visible.
        assert!(
            job.sections
                .last()
                .is_some_and(|s| s.format.font_id.size > 1.0)
        );
        // Shown when the caret is inside the block, or when not hidden.
        p.active = 1..2;
        let job = build_job(t, &d, &p, &[], 0);
        assert_ne!(job.sections[0].format.color, Color32::TRANSPARENT);
        p.active = 3..3;
        p.hide_frontmatter = false;
        let job = build_job(t, &d, &p, &[], 0);
        assert_ne!(job.sections[0].format.color, Color32::TRANSPARENT);
    }

    #[test]
    fn job_with_marks_covers_text() {
        let t = "été **été** été";
        let d = decorate(t);
        let yes = |_: &str| true;
        let marks = find_all(t, "été", false);
        let job = build_job(t, &d, &params(0..0, &yes), &marks, 1);
        assert_eq!(job.text, t);
        assert!(
            job.sections
                .iter()
                .any(|s| s.format.background != Color32::TRANSPARENT)
        );
    }
}
