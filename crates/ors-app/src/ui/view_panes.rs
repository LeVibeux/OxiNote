//! Search pane, backlinks, outgoing links and outline (logic in
//! `crate::views`).

use eframe::egui::{
    self, Color32, Frame, Margin, RichText, Sense,
    collapsing_header::CollapsingState,
    text::{LayoutJob, TextFormat},
};
use ors_core::NotePath;

use super::theme::Palette;
use super::{Action, panes};
use crate::state::{AppState, OpenIn};
use crate::views::{
    self, ContextItem, Grouped, OutlineNode, SEARCH_OPERATORS, SortOrder, ViewAction,
};

const DEBOUNCE: f64 = 0.3;
const MAX_FILES: usize = 300;

fn open_in(ui: &egui::Ui, resp: &egui::Response) -> OpenIn {
    panes::open_in_from(ui, resp)
}

fn highlighted(text: &str, ranges: &[(usize, usize)], pal: &Palette, rows: usize) -> LayoutJob {
    let mut job = LayoutJob::default();
    let normal = TextFormat {
        color: pal.muted,
        font_id: egui::FontId::proportional(12.5),
        ..Default::default()
    };
    let hit = TextFormat {
        color: pal.strong,
        background: pal.highlight_bg,
        ..normal.clone()
    };
    let mut pos = 0;
    let mut sorted: Vec<(usize, usize)> = ranges.to_vec();
    sorted.sort_unstable();
    for (s, e) in sorted {
        if s < pos || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        job.append(&text[pos..s], 0.0, normal.clone());
        job.append(&text[s..e], 0.0, hit.clone());
        pos = e;
    }
    job.append(&text[pos..], 0.0, normal);
    job.wrap.max_rows = rows;
    job
}

fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str, tip: &str) -> bool {
    let pal = Palette::of(ui.ctx());
    let color = if *on { pal.accent } else { pal.muted };
    let r = ui
        .add(egui::Button::new(RichText::new(label).color(color)).frame(*on))
        .on_hover_text(tip);
    if r.clicked() {
        *on = !*on;
    }
    r.clicked()
}

fn sort_combo(ui: &mut egui::Ui, id: &str, sort: &mut SortOrder) -> bool {
    let before = *sort;
    egui::ComboBox::from_id_salt(id)
        .selected_text(sort.label())
        .width(ui.available_width() - 8.0)
        .show_ui(ui, |ui| {
            for o in SortOrder::ALL {
                ui.selectable_value(sort, o, o.label());
            }
        });
    *sort != before
}

fn section_header(ui: &mut egui::Ui, text: &str, count: usize, pal: &Palette) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).strong().color(pal.muted));
        ui.label(
            RichText::new(count.to_string())
                .small()
                .color(pal.faint)
                .background_color(pal.bg_primary),
        );
    });
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

pub fn search_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let now = ui.input(|i| i.time);
    let mut opts = state.views.search.clone();
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = ui.available_width() - 90.0;
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.search_query)
                .hint_text("Search…")
                .desired_width(w),
        );
        if resp.changed() {
            state.views.search_edited = Some(now);
        }
        if state.focus_search {
            resp.request_focus();
            state.focus_search = false;
        }
        changed |= toggle(ui, &mut opts.match_case, "Aa", "Match case");
        toggle(ui, &mut opts.show_settings, "⚙", "Search settings");
        toggle(ui, &mut opts.show_help, "?", "Search syntax");
    });
    if opts.show_settings {
        Frame::new()
            .fill(pal.bg_primary)
            .corner_radius(4)
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.checkbox(&mut opts.collapse, "Collapse results");
                ui.checkbox(&mut opts.more_context, "Show more context");
                ui.checkbox(&mut opts.explain, "Explain search term");
                changed |= sort_combo(ui, "search-sort", &mut opts.sort);
            });
    }
    if opts.show_help {
        Frame::new()
            .fill(pal.bg_primary)
            .corner_radius(4)
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Search options").strong());
                egui::Grid::new("search-help").num_columns(2).show(ui, |ui| {
                    for (op, desc) in SEARCH_OPERATORS {
                        ui.label(RichText::new(*op).monospace().color(pal.accent));
                        ui.label(RichText::new(*desc).small().color(pal.muted));
                        ui.end_row();
                    }
                });
            });
    }
    state.views.search = opts.clone();

    // Live search, debounced.
    if views::debounce_ready(state.views.search_edited, now, DEBOUNCE) {
        state.views.search_edited = None;
        state.refresh_search();
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    if changed {
        state.refresh_search();
    }
    ui.add_space(4.0);

    let query = views::effective_query(&state.search_query, &opts);
    if let Err(e) = &state.search_results {
        ui.label(RichText::new(format!("Invalid query: {e}")).color(Color32::from_rgb(0xe9, 0x31, 0x47)));
        return;
    }
    if opts.explain && !state.search_query.trim().is_empty() {
        if let Ok(q) = ors_index::parse_query(&query) {
            for (depth, line) in views::explain_query(&q) {
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 12.0);
                    ui.label(RichText::new(line).small().color(pal.muted));
                });
            }
        }
        ui.add_space(4.0);
    }
    if query.is_empty() {
        return;
    }

    // Sorted copy of the results.
    let key = (query.clone(), state.generation, opts.sort);
    if state.views.sorted.key.as_ref() != Some(&key) {
        let mut r = state.search_results.clone().unwrap_or_default();
        views::sort_results(state, &mut r, opts.sort);
        state.views.sorted.value = r;
        state.views.sorted.key = Some(key);
    }
    let results = std::mem::take(&mut state.views.sorted.value);
    let matches: usize = results.iter().map(|r| r.matches.len().max(1)).sum();
    ui.label(
        RichText::new(format!(
            "{matches} result{} in {} file{}",
            if matches == 1 { "" } else { "s" },
            results.len(),
            if results.len() == 1 { "" } else { "s" }
        ))
        .small()
        .color(pal.faint),
    );
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for r in results.iter().take(MAX_FILES) {
                let id = ui.make_persistent_id(("search", r.path.as_str(), opts.collapse));
                CollapsingState::load_with_default_open(ui.ctx(), id, !opts.collapse)
                    .show_header(ui, |ui| {
                        let l = ui
                            .add(
                                egui::Label::new(RichText::new(r.path.stem()).color(pal.strong))
                                    .sense(Sense::click()),
                            )
                            .on_hover_text(r.path.as_str());
                        if l.clicked() || l.middle_clicked() {
                            actions.push(Action::Open(r.path.clone(), open_in(ui, &l)));
                        }
                        if !r.matches.is_empty() {
                            ui.label(
                                RichText::new(r.matches.len().to_string())
                                    .small()
                                    .color(pal.faint),
                            );
                        }
                    })
                    .body(|ui| {
                        let source = if opts.more_context {
                            state.note_text(&r.path)
                        } else {
                            None
                        };
                        for m in &r.matches {
                            let (text, ranges) = match &source {
                                Some(src) => {
                                    let c = views::context_for(src, m.line, &[], true);
                                    let terms: Vec<&str> = m
                                        .ranges
                                        .iter()
                                        .filter_map(|&(s, e)| m.line_text.get(s..e))
                                        .collect();
                                    let ranges = views::highlight_terms(&c.text, &terms);
                                    (c.text, ranges)
                                }
                                None => (m.line_text.clone(), m.ranges.clone()),
                            };
                            let rows = if opts.more_context { 8 } else { 3 };
                            let job = highlighted(&text, &ranges, &pal, rows);
                            let resp = Frame::new()
                                .inner_margin(Margin::symmetric(6, 3))
                                .corner_radius(4)
                                .fill(pal.bg_primary)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add(egui::Label::new(job).wrap().sense(Sense::click()))
                                });
                            let l = resp.inner.on_hover_cursor(egui::CursorIcon::PointingHand);
                            if l.clicked() || l.middle_clicked() {
                                actions.push(Action::OpenLine(
                                    r.path.clone(),
                                    m.line,
                                    open_in(ui, &l),
                                ));
                            }
                            ui.add_space(2.0);
                        }
                    });
            }
            if results.len() > MAX_FILES {
                ui.label(
                    RichText::new(format!("{} more files not shown", results.len() - MAX_FILES))
                        .small()
                        .color(pal.faint),
                );
            }
        });
    state.views.sorted.value = results;
}

// ---------------------------------------------------------------------------
// Backlinks
// ---------------------------------------------------------------------------

/// Recompute `cache` for `key` (rate limited while only the generation
/// changed, e.g. while typing).
fn refresh<K: PartialEq + Clone, V>(
    cache: &mut views::Cached<K, V>,
    key: K,
    now: f64,
    same_note: impl Fn(&K, &K) -> bool,
    compute: impl FnOnce() -> V,
) {
    let stale = cache.key.as_ref() != Some(&key);
    let urgent = cache.key.as_ref().is_none_or(|k| !same_note(k, &key));
    if stale && (urgent || now - cache.time >= 0.5) {
        cache.value = compute();
        cache.key = Some(key);
        cache.time = now;
    }
}

fn count_items(g: &Grouped) -> usize {
    g.iter().map(|(_, v)| v.len()).sum()
}

fn filter_grouped(g: &Grouped, filter: &str) -> Grouped {
    let f = filter.trim().to_lowercase();
    if f.is_empty() {
        return g.clone();
    }
    g.iter()
        .filter_map(|(p, items)| {
            if p.stem().to_lowercase().contains(&f) {
                return Some((p.clone(), items.clone()));
            }
            let kept: Vec<ContextItem> = items
                .iter()
                .filter(|i| i.text.to_lowercase().contains(&f))
                .cloned()
                .collect();
            (!kept.is_empty()).then(|| (p.clone(), kept))
        })
        .collect()
}

pub fn backlinks_pane(
    ui: &mut egui::Ui,
    state: &mut AppState,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let now = ui.input(|i| i.time);
    let mut opts = state.views.backlinks.clone();
    let mut reset = false;
    ui.horizontal(|ui| {
        reset |= toggle(ui, &mut opts.more_context, "☰", "Show more context");
        toggle(ui, &mut opts.collapse, "⇕", "Collapse results");
        toggle(ui, &mut opts.show_filter, "🔍", "Show search filter");
    });
    if opts.show_filter {
        ui.add(
            egui::TextEdit::singleline(&mut opts.filter)
                .hint_text("Search results…")
                .desired_width(f32::INFINITY),
        );
    }
    let before = opts.sort;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Sort").small().color(pal.faint));
        sort_combo(ui, "backlinks-sort", &mut opts.sort);
    });
    reset |= before != opts.sort;
    if reset {
        state.views.linked.key = None;
        state.views.unlinked.key = None;
    }

    let generation = state.generation;
    let key = (path.clone(), generation, opts.more_context);
    let more = opts.more_context;
    let sort = opts.sort;
    let mut linked = std::mem::take(&mut state.views.linked);
    refresh(&mut linked, key.clone(), now, |a, b| a.0 == b.0, || {
        let mut g = views::linked_mentions(state, path, more);
        views::sort_by_path(&mut g, |x| &x.0, sort, |p| views::file_times(state, p));
        g
    });
    state.views.linked = linked;
    let mut unlinked = std::mem::take(&mut state.views.unlinked);
    if opts.show_unlinked {
        refresh(&mut unlinked, key, now, |a, b| a.0 == b.0, || {
            let mut g = views::unlinked_mentions(state, path, more);
            views::sort_by_path(&mut g, |x| &x.0, sort, |p| views::file_times(state, p));
            g
        });
    }
    state.views.unlinked = unlinked;
    state.views.backlinks = opts.clone();

    let linked = filter_grouped(&state.views.linked.value, &opts.filter);
    let unlinked = filter_grouped(&state.views.unlinked.value, &opts.filter);
    let mut show_unlinked = opts.show_unlinked;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            section_header(ui, "Linked mentions", count_items(&linked), &pal);
            if linked.is_empty() {
                ui.label(RichText::new("No backlinks found.").color(pal.faint));
            }
            mention_groups(ui, &linked, path, &opts, "linked", &pal, actions);
            ui.add_space(10.0);
            let arrow = if show_unlinked { "⏷" } else { "⏵" };
            let count = if show_unlinked {
                count_items(&unlinked).to_string()
            } else {
                String::new()
            };
            let head = ui
                .horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{arrow} Unlinked mentions"))
                            .strong()
                            .color(pal.muted),
                    );
                    ui.label(RichText::new(count).small().color(pal.faint));
                })
                .response
                .interact(Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if head.clicked() {
                show_unlinked = !show_unlinked;
            }
            if show_unlinked {
                if unlinked.is_empty() {
                    ui.label(RichText::new("No unlinked mentions found.").color(pal.faint));
                }
                mention_groups(ui, &unlinked, path, &opts, "unlinked", &pal, actions);
            }
        });
    state.views.backlinks.show_unlinked = show_unlinked;
}

/// Source notes with their context lines. Unlinked items get a "Link"
/// button.
fn mention_groups(
    ui: &mut egui::Ui,
    groups: &Grouped,
    target: &NotePath,
    opts: &views::BacklinkOptions,
    kind: &str,
    pal: &Palette,
    actions: &mut Vec<Action>,
) {
    for (src, items) in groups {
        let id = ui.make_persistent_id((kind, src.as_str(), opts.collapse));
        CollapsingState::load_with_default_open(ui.ctx(), id, !opts.collapse)
            .show_header(ui, |ui| {
                let l = ui
                    .add(
                        egui::Label::new(RichText::new(src.stem()).color(pal.strong))
                            .sense(Sense::click()),
                    )
                    .on_hover_text(src.as_str());
                if l.clicked() || l.middle_clicked() {
                    actions.push(Action::Open(src.clone(), open_in(ui, &l)));
                }
                ui.label(RichText::new(items.len().to_string()).small().color(pal.faint));
            })
            .body(|ui| {
                for item in items {
                    let job = highlighted(&item.text, &item.ranges, pal, 8);
                    ui.horizontal(|ui| {
                        let width = ui.available_width()
                            - if item.mention.is_some() { 46.0 } else { 0.0 };
                        let resp = Frame::new()
                            .inner_margin(Margin::symmetric(6, 3))
                            .corner_radius(4)
                            .fill(pal.bg_primary)
                            .show(ui, |ui| {
                                ui.set_width(width.max(60.0) - 12.0);
                                ui.add(egui::Label::new(job).wrap().sense(Sense::click()))
                            });
                        let l = resp.inner.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if l.clicked() || l.middle_clicked() {
                            actions.push(Action::View(ViewAction::RevealLine {
                                path: src.clone(),
                                line: item.line,
                                open_in: open_in(ui, &l),
                            }));
                        }
                        if let Some(m) = &item.mention
                            && ui
                                .small_button("Link")
                                .on_hover_text("Replace this mention with a link")
                                .clicked()
                        {
                            actions.push(Action::View(ViewAction::LinkMention {
                                source: src.clone(),
                                mention: m.clone(),
                                target: target.clone(),
                            }));
                        }
                    });
                    ui.add_space(2.0);
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Outgoing links
// ---------------------------------------------------------------------------

pub fn outgoing_pane(
    ui: &mut egui::Ui,
    state: &mut AppState,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let now = ui.input(|i| i.time);
    let info = state.note_info().clone();
    let key = (path.clone(), state.generation);
    let mut cache = std::mem::take(&mut state.views.outgoing_mentions);
    refresh(&mut cache, key, now, |a, b| a.0 == b.0, || {
        views::outgoing_mentions(state, path)
    });
    state.views.outgoing_mentions = cache;
    let mentions = state.views.outgoing_mentions.value.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (resolved, unresolved): (Vec<_>, Vec<_>) =
                info.outgoing.iter().partition(|l| l.target.is_some());
            let mut seen = std::collections::HashSet::new();
            let resolved: Vec<_> = resolved
                .into_iter()
                .filter_map(|l| l.target.clone())
                .filter(|t| seen.insert(t.clone()))
                .collect();
            section_header(ui, "Links", resolved.len(), &pal);
            for t in &resolved {
                let r = ui
                    .add(
                        egui::Label::new(RichText::new(t.stem()).color(pal.accent))
                            .sense(Sense::click()),
                    )
                    .on_hover_text(t.as_str());
                if r.clicked() || r.middle_clicked() {
                    actions.push(Action::Open(t.clone(), open_in(ui, &r)));
                }
            }
            ui.add_space(8.0);
            section_header(ui, "Unresolved links", unresolved.len(), &pal);
            for l in unresolved {
                let r = ui
                    .add(
                        egui::Label::new(RichText::new(&l.link.target).color(pal.unresolved))
                            .sense(Sense::click()),
                    )
                    .on_hover_text("Click to create");
                if r.clicked() {
                    actions.push(Action::Follow {
                        target: l.link.target.clone(),
                        from: path.clone(),
                        open_in: OpenIn::CurrentTab,
                    });
                }
            }
            ui.add_space(8.0);
            section_header(ui, "Unlinked mentions", mentions.len(), &pal);
            for (m, target) in &mentions {
                ui.horizontal(|ui| {
                    let r = ui
                        .add(
                            egui::Label::new(RichText::new(&m.text).color(pal.text))
                                .sense(Sense::click()),
                        )
                        .on_hover_text(format!("{target} — line {}", m.line + 1));
                    if r.clicked() {
                        actions.push(Action::View(ViewAction::RevealLine {
                            path: path.clone(),
                            line: m.line,
                            open_in: OpenIn::CurrentTab,
                        }));
                    }
                    ui.label(RichText::new(format!("→ {}", target.stem())).small().color(pal.faint));
                    if ui.small_button("Link").clicked() {
                        actions.push(Action::View(ViewAction::LinkMention {
                            source: path.clone(),
                            mention: m.clone(),
                            target: target.clone(),
                        }));
                    }
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Outline
// ---------------------------------------------------------------------------

pub fn outline_pane(
    ui: &mut egui::Ui,
    state: &mut AppState,
    path: &NotePath,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let headings = state.note_info().headings.clone();
    ui.add(
        egui::TextEdit::singleline(&mut state.views.outline_filter)
            .hint_text("Filter headings…")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    if headings.is_empty() {
        ui.label(RichText::new("No headings found.").color(pal.faint));
        return;
    }
    let levels: Vec<u8> = headings.iter().map(|h| h.level).collect();
    let tree = views::outline_tree(&levels);
    let texts: Vec<&str> = headings.iter().map(|h| h.text.as_str()).collect();
    let filter = state.views.outline_filter.trim().to_lowercase();
    // The current section: last heading at or before the current line.
    let current = state
        .views
        .current_line
        .as_ref()
        .filter(|(p, _)| p == path)
        .and_then(|(_, line)| headings.iter().rposition(|h| h.line <= *line));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for node in &tree {
                outline_node(ui, node, &headings, &texts, &filter, current, path, &pal, actions);
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn outline_node(
    ui: &mut egui::Ui,
    node: &OutlineNode,
    headings: &[ors_core::Heading],
    texts: &[&str],
    filter: &str,
    current: Option<usize>,
    path: &NotePath,
    pal: &Palette,
    actions: &mut Vec<Action>,
) {
    if !filter.is_empty() && !views::outline_matches(node, texts, filter) {
        return;
    }
    let h = &headings[node.index];
    let row = |ui: &mut egui::Ui, actions: &mut Vec<Action>| {
        let is_current = current == Some(node.index);
        let mut text = RichText::new(&h.text).color(if is_current { pal.accent } else { pal.text });
        if is_current {
            text = text.strong().background_color(pal.accent_soft);
        }
        let r = ui
            .add(egui::Label::new(text).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if r.clicked() {
            actions.push(Action::View(ViewAction::RevealLine {
                path: path.clone(),
                line: h.line,
                open_in: OpenIn::CurrentTab,
            }));
        }
    };
    if node.children.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            row(ui, actions);
        });
    } else {
        let id = ui.make_persistent_id(("outline", path.as_str(), h.line));
        CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| row(ui, actions))
            .body(|ui| {
                for c in &node.children {
                    outline_node(ui, c, headings, texts, filter, current, path, pal, actions);
                }
            });
    }
}
