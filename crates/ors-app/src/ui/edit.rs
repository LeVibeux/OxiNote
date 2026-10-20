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
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use eframe::egui::{
    self, Align, Color32, Event, FontFamily, FontId, Frame, Key, Margin, Modifiers, Pos2, Rect,
    Sense, Stroke, TextFormat,
    text::{CCursor, CCursorRange, LayoutJob},
};
use ors_core::NotePath;

use super::editor::{editor_font, format_for, layout_job, readable_column};
use super::theme::Palette;
use super::{Action, UiCaches};
use crate::backend::IndexBackend;
use crate::commands::Command;
use crate::editor::completion::{self, Context as CompCtx, Kind as CompKind};
use crate::editor::find::{find_all, next_match, replace_all};
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
    resolves: &'a dyn Fn(&str) -> bool,
}

fn heading_scale(kind: &LineKind) -> f32 {
    match kind {
        LineKind::Heading(1) => 1.6,
        LineKind::Heading(2) => 1.4,
        LineKind::Heading(3) => 1.25,
        LineKind::Heading(4) => 1.12,
        LineKind::Heading(_) => 1.05,
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
    let mut f = format_for(style, p.size, p.mono, pal);
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
    match conceal {
        Some(Conceal::Hide) => {
            f.font_id = FontId::new(0.1, FontFamily::Proportional);
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
    for (n, line) in decor.lines.iter().enumerate() {
        let r = line.range.clone();
        while si < spans.len() && spans[si].0.end <= r.start {
            si += 1;
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
                font_id: editor_font(p.size * heading_scale(&line.kind), p.mono),
                color: p.pal.text,
                ..TextFormat::default()
            };
            pieces.push((r.end..next, f));
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
        return layout_job(text, p.size, p.mono, &p.pal);
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

fn read_sel(ctx: &egui::Context, id: egui::Id, text: &str) -> Option<Sel> {
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
        CompKind::Block { .. } => Vec::new(),
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
            Command::FoldAll | Command::UnfoldAll => {}
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
    let active = {
        let r = read_sel(&ctx, id, text).unwrap_or(Sel::caret(0));
        if ctx.memory(|m| m.has_focus(id)) || read_sel(&ctx, id, text).is_some() {
            line_of(text, r.start)..line_of(text, r.end) + 1
        } else {
            0..0
        }
    };
    let find_params = caches
        .find
        .as_ref()
        .filter(|f| f.tab == tab_id && !f.query.is_empty())
        .map(|f| (f.query.clone(), f.case, f.current));
    let generation = state.generation;
    let index: &dyn IndexBackend = &*state.index;
    let from = path.clone();
    let resolves = |t: &str| index.resolve(t, &from).is_some();
    let decor_cache = &mut caches.decor;
    let job_cache = &mut caches.highlight;
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
        let key = hash_of(&(
            th,
            size.to_bits(),
            mono,
            pal.text,
            live,
            active_for_layout.start,
            active_for_layout.end,
            generation,
            &find_params,
        ));
        let mut job = match job_cache.get(&tab_id) {
            Some((k, job)) if *k == key => job.clone(),
            _ => {
                let marks = find_params
                    .as_ref()
                    .map(|(q, case, _)| find_all(t, q, *case))
                    .unwrap_or_default();
                let params = Params {
                    size,
                    mono,
                    pal,
                    live,
                    active: active_for_layout.clone(),
                    resolves: &resolves,
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

    let output = readable_column(ui, readable, |ui| {
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
        ui.scroll_to_rect(rect.expand(40.0), Some(Align::Center));
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
        if (a.min(b)..a.max(b) + 1) != active {
            ctx.request_repaint();
        }
    }

    // ---- Live Preview widgets ------------------------------------------
    let toggle_line = if live {
        paint_widgets(ui, &decor, text_ref, &output, &active, &pal, tab_id)
    } else {
        None
    };

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
                            let d = d.split('#').next().unwrap_or("");
                            let d = d.strip_suffix(".md").unwrap_or(d);
                            actions.push(Action::Follow {
                                target: d.to_string(),
                                from: path.clone(),
                                open_in: how,
                            });
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

/// Byte offset under screen position `pos`.
fn byte_at(galley: &egui::Galley, origin: Pos2, text: &str, pos: Pos2) -> usize {
    let c = galley.cursor_from_pos(pos - origin);
    char_to_byte(text, usize::from(c.index))
}

/// Paint bullets, checkboxes, rules, quote bars and callout tints over the
/// blank glyphs of lines that do not hold the caret. Returns the line of a
/// checkbox that was clicked.
fn paint_widgets(
    ui: &mut egui::Ui,
    decor: &Decor,
    text: &str,
    output: &egui::text_edit::TextEditOutput,
    active: &Range<usize>,
    pal: &Palette,
    tab_id: TabId,
) -> Option<usize> {
    let mut toggled = None;
    let galley = &output.galley;
    let off = output.galley_pos.to_vec2();
    let clip = ui.clip_rect();
    let width = output.response.rect.width();
    // Char index of the start of every line.
    let mut line_chars = Vec::with_capacity(decor.lines.len());
    let mut acc = 0;
    for l in &decor.lines {
        line_chars.push(acc);
        acc += text.get(l.range.clone()).map_or(0, |s| s.chars().count()) + 1;
    }
    let rect_at = |n: usize, byte: usize| -> Rect {
        let start = decor.lines[n].range.start;
        let c = line_chars[n] + text.get(start..byte).map_or(0, |s| s.chars().count());
        galley.pos_from_cursor(CCursor::new(c)).translate(off)
    };
    let painter = ui.painter().clone();
    for (n, l) in decor.lines.iter().enumerate() {
        if active.contains(&n) {
            continue;
        }
        let top = rect_at(n, l.range.start);
        if top.bottom() < clip.top() - 200.0 || top.top() > clip.bottom() + 200.0 {
            continue;
        }
        let bottom = rect_at(n, l.range.end);
        let (y0, y1) = (top.top(), bottom.bottom());
        match &l.kind {
            LineKind::Quote => {
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(off.x - 10.0, y0), Pos2::new(off.x - 7.0, y1)),
                    1.5,
                    pal.accent.gamma_multiply(0.8),
                );
            }
            LineKind::Callout { kind, .. } => {
                let col = pal.callout(kind);
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(off.x - 10.0, y0), Pos2::new(off.x + width, y1)),
                    0.0,
                    col.gamma_multiply(0.12),
                );
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(off.x - 10.0, y0), Pos2::new(off.x - 7.0, y1)),
                    1.5,
                    col,
                );
            }
            LineKind::Rule => {
                let y = (y0 + y1) / 2.0;
                painter.line_segment(
                    [Pos2::new(off.x, y), Pos2::new(off.x + width, y)],
                    Stroke::new(1.5, pal.border.gamma_multiply(1.6)),
                );
            }
            _ => {}
        }
    }
    for (w, n) in &decor.widgets {
        if active.contains(n) {
            continue;
        }
        match w {
            Widget::Bullet { at } => {
                let a = rect_at(*n, *at);
                let b = rect_at(*n, *at + 1);
                if a.bottom() < clip.top() || a.top() > clip.bottom() {
                    continue;
                }
                let c = Pos2::new((a.left() + b.left()) / 2.0, a.center().y);
                painter.circle_filled(c, (a.height() * 0.085).max(2.0), pal.text);
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

    fn params<'a>(active: Range<usize>, resolves: &'a dyn Fn(&str) -> bool) -> Params<'a> {
        Params {
            size: 15.0,
            mono: false,
            pal: Palette::dark(),
            live: true,
            active,
            resolves,
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
