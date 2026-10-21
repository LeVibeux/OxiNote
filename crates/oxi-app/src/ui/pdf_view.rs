//! PDF views: the tab (continuous vertical scroll with a toolbar) and the
//! embed of the reading view (`![[doc.pdf#page=3|400]]`).
//!
//! Documents are shared by path between the tabs and the embeds: each one
//! has a render thread ([`PdfService`]) and a bounded texture cache. Only
//! the visible pages plus one before and one after are requested; until a
//! page is ready a grey rectangle of the right size (or the same page at
//! another zoom, stretched) is drawn.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Rect, RichText, Sense, TextureHandle, TextureId,
    TextureOptions, pos2, vec2,
};
use oxi_core::NotePath;

use super::theme::Palette;
use super::{Action, UiCaches};
use crate::pdf::{
    LruCache, PT_TO_PX, PageLayout, PdfBitmap, PdfError, PdfEvent, PdfParams, PdfService,
    PdfViewState, RenderKey, fit_width_zoom, step_zoom,
};
use crate::state::{AppState, OpenIn, TabId};

/// Space between pages (and around them) in the tab.
const GAP: f32 = 14.0;
const MARGIN_X: f32 = 16.0;
const SCROLLBAR: f32 = 14.0;
const CACHE_ENTRIES: usize = 24;
const CACHE_BYTES: usize = 192 << 20;
/// A document nobody draws for this long is closed (thread and textures).
const KEEP_SECS: f64 = 60.0;
const EMBED_HEIGHT: f32 = 400.0;
const EMBED_MAX_WIDTH: f32 = 720.0;

enum Status {
    Loading,
    Ready(Vec<(f32, f32)>),
    Error(PdfError),
}

/// An open document.
pub struct PdfDoc {
    service: Option<PdfService>,
    status: Status,
    textures: LruCache<RenderKey, TextureHandle>,
    failed: HashSet<RenderKey>,
    req_pass: u64,
    req: Vec<RenderKey>,
    last_used: f64,
    name: String,
    /// Password typed in the error panel.
    password: String,
    unlock: Option<String>,
    tried_password: bool,
}

impl PdfDoc {
    fn open(
        ctx: &egui::Context,
        state: &AppState,
        path: &NotePath,
        password: Option<String>,
    ) -> Self {
        let tried_password = password.is_some();
        let (service, status) = match state.vault.read_bytes(path) {
            Ok(bytes) => {
                let c = ctx.clone();
                let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || c.request_repaint());
                (
                    Some(PdfService::spawn(bytes, password, Some(repaint))),
                    Status::Loading,
                )
            }
            Err(e) => (
                None,
                Status::Error(PdfError::Invalid(format!("cannot read the file: {e}"))),
            ),
        };
        Self {
            service,
            status,
            textures: LruCache::new(CACHE_ENTRIES, CACHE_BYTES),
            failed: HashSet::new(),
            req_pass: 0,
            req: Vec::new(),
            last_used: 0.0,
            name: path.as_str().to_string(),
            password: String::new(),
            unlock: None,
            tried_password,
        }
    }

    /// Take what the render thread produced.
    fn pump(&mut self, ctx: &egui::Context) {
        let Some(service) = &self.service else { return };
        for ev in service.poll() {
            match ev {
                PdfEvent::Opened(sizes) => self.status = Status::Ready(sizes),
                PdfEvent::Failed(e) => self.status = Status::Error(e),
                PdfEvent::Rendered(key, bmp) => {
                    let bytes = bmp.byte_size();
                    let tex = upload(ctx, &self.name, key, &bmp);
                    self.textures.insert(key, tex, bytes);
                }
                PdfEvent::RenderFailed(key) => {
                    self.failed.insert(key);
                }
            }
        }
    }

    fn sizes(&self) -> Option<&[(f32, f32)]> {
        match &self.status {
            Status::Ready(s) => Some(s),
            _ => None,
        }
    }

    /// Ask for pages (most important first). Calls made during the same
    /// pass add up, so several views of one document do not cancel each
    /// other.
    fn request(&mut self, ctx: &egui::Context, keys: Vec<RenderKey>) {
        let pass = ctx.cumulative_pass_nr();
        if pass != self.req_pass {
            self.req_pass = pass;
            self.req.clear();
        }
        for k in keys {
            if !self.req.contains(&k) {
                self.req.push(k);
            }
        }
        if let Some(s) = &self.service {
            s.set_wanted(self.req.clone());
        }
    }

    /// Texture for `key`, or the closest scale of the same page (stretched
    /// until the right one arrives). `exact` tells which one it is.
    fn texture(&mut self, key: RenderKey) -> Option<(TextureId, bool)> {
        if let Some(t) = self.textures.get(&key) {
            return Some((t.id(), true));
        }
        self.textures
            .best_by(|k| (k.page == key.page).then(|| k.scale_pct.abs_diff(key.scale_pct)))
            .map(|(_, t)| (t.id(), false))
    }

    /// Keys of `pages`, skipping what is cached or known to fail.
    fn missing(&self, pages: impl Iterator<Item = usize>, zoom: f32, ppp: f32) -> Vec<RenderKey> {
        pages
            .map(|p| RenderKey::new(p, zoom, ppp))
            .filter(|k| !self.textures.contains(k) && !self.failed.contains(k))
            .collect()
    }
}

fn upload(ctx: &egui::Context, name: &str, key: RenderKey, bmp: &PdfBitmap) -> TextureHandle {
    let img = ColorImage::from_rgba_unmultiplied([bmp.width, bmp.height], &bmp.rgba);
    ctx.load_texture(
        format!("pdf:{name}:{}:{}", key.page, key.scale_pct),
        img,
        TextureOptions::LINEAR,
    )
}

/// Per-tab UI-only state.
#[derive(Default)]
struct ViewUi {
    last_zoom: f32,
    last_offset: f32,
    page_text: String,
}

#[derive(Default)]
pub struct PdfCaches {
    docs: HashMap<NotePath, PdfDoc>,
    views: HashMap<TabId, ViewUi>,
}

impl PdfCaches {
    /// Close documents nobody draws any more.
    pub fn gc(&mut self, now: f64) {
        self.docs.retain(|_, d| now - d.last_used < KEEP_SECS);
    }

    #[cfg(test)]
    pub fn doc_count(&self) -> usize {
        self.docs.len()
    }

    /// Cached textures of a document (tests).
    #[cfg(test)]
    pub fn texture_count(&self, path: &NotePath) -> usize {
        self.docs.get(path).map_or(0, |d| d.textures.len())
    }

    #[cfg(test)]
    pub fn page_count(&self, path: &NotePath) -> Option<usize> {
        self.docs.get(path).and_then(|d| d.sizes().map(<[_]>::len))
    }

    fn ensure(&mut self, ctx: &egui::Context, state: &AppState, path: &NotePath) -> &mut PdfDoc {
        let now = ctx.input(|i| i.time);
        let doc = self
            .docs
            .entry(path.clone())
            .or_insert_with(|| PdfDoc::open(ctx, state, path, None));
        doc.last_used = now;
        doc.pump(ctx);
        doc
    }
}

fn centered_message(ui: &mut egui::Ui, text: RichText) {
    ui.add_space((ui.available_height() * 0.3).min(160.0));
    ui.vertical_centered(|ui| {
        ui.label(text);
    });
}

// ---------------------------------------------------------------------------
// Page painting (shared by the tab and the embeds)
// ---------------------------------------------------------------------------

fn paint_page(
    ui: &egui::Ui,
    doc: &mut PdfDoc,
    pal: &Palette,
    rect: Rect,
    page: usize,
    zoom: f32,
    ppp: f32,
) {
    let painter = ui.painter();
    let shadow = egui::Shadow {
        offset: [0, 2],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(80),
    };
    painter.add(shadow.as_shape(rect, 2.0));
    match doc.texture(RenderKey::new(page, zoom, ppp)) {
        Some((id, _)) => {
            painter.image(
                id,
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(rect, 2.0, Color32::from_gray(0xe6));
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("Page {}", page + 1),
                FontId::proportional(14.0),
                pal.faint,
            );
        }
    }
}

/// Draw the visible pages of `layout` inside a scroll area whose content
/// rectangle is `origin`; returns the page clicked, if `clickable`.
#[allow(clippy::too_many_arguments)]
fn paint_visible(
    ui: &mut egui::Ui,
    doc: &mut PdfDoc,
    pal: &Palette,
    layout: &PageLayout,
    viewport: Rect,
    origin: Rect,
    zoom: f32,
    clickable: bool,
) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let vis = layout.visible(viewport.min.y, viewport.max.y);
    let pre = layout.prefetch(&vis);
    // Visible pages first, then the neighbours.
    let order = vis
        .clone()
        .chain(pre.filter(|p| !vis.contains(p)))
        .collect::<Vec<_>>();
    let wanted = doc.missing(order.into_iter(), zoom, ppp);
    if !wanted.is_empty() {
        doc.request(&ctx, wanted);
    }
    let mut clicked = None;
    for i in vis {
        let w = layout.widths[i];
        let x = origin.min.x + (origin.width() - w) / 2.0;
        let rect = Rect::from_min_size(
            pos2(x.max(origin.min.x), origin.min.y + layout.tops[i]),
            vec2(w, layout.heights[i]),
        );
        paint_page(ui, doc, pal, rect, i, zoom, ppp);
        if clickable {
            let r = ui
                .interact(rect, ui.scope_id().with(("pdf-page", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.clicked() {
                clicked = Some(i);
            }
        }
    }
    clicked
}

// ---------------------------------------------------------------------------
// Tab
// ---------------------------------------------------------------------------

/// Toolbar; changes `view` and returns whether to open the system app.
fn toolbar(
    ui: &mut egui::Ui,
    pal: &Palette,
    view: &mut PdfViewState,
    vs: &mut ViewUi,
    tab_id: TabId,
    pages: usize,
    zoom: f32,
) -> bool {
    let mut external = false;
    egui::Frame::new()
        .fill(pal.bg_secondary)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let page_id = ui.make_persistent_id(("pdf-page-field", tab_id));
                if !ui.memory(|m| m.has_focus(page_id)) {
                    vs.page_text = view.page.to_string();
                }
                if ui
                    .add_enabled(view.page > 1, egui::Button::new("↑").frame(false))
                    .on_hover_text("Previous page (PageUp)")
                    .clicked()
                {
                    view.go_to(view.page - 1, pages);
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut vs.page_text)
                        .id(page_id)
                        .desired_width(34.0)
                        .align(Align2::CENTER_CENTER),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Ok(n) = vs.page_text.trim().parse::<u32>() {
                        view.go_to(n, pages);
                    }
                    vs.page_text = view.page.to_string();
                }
                ui.label(RichText::new(format!("/ {pages}")).color(pal.muted));
                if ui
                    .add_enabled(
                        (view.page as usize) < pages,
                        egui::Button::new("↓").frame(false),
                    )
                    .on_hover_text("Next page (PageDown)")
                    .clicked()
                {
                    view.go_to(view.page + 1, pages);
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("−").frame(false))
                    .on_hover_text("Zoom out (Ctrl+wheel)")
                    .clicked()
                {
                    view.set_zoom(step_zoom(zoom, false));
                }
                ui.label(RichText::new(format!("{:.0}%", zoom * 100.0)).color(pal.text));
                if ui
                    .add(egui::Button::new("+").frame(false))
                    .on_hover_text("Zoom in (Ctrl+wheel)")
                    .clicked()
                {
                    view.set_zoom(step_zoom(zoom, true));
                }
                if ui
                    .add(egui::Button::selectable(view.fit_width, "Fit to width"))
                    .on_hover_text("Zoom so that the page fills the width")
                    .clicked()
                {
                    view.fit_width = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Open in default app")
                        .on_hover_text("Open this file with the system's PDF viewer")
                        .clicked()
                    {
                        external = true;
                    }
                });
            });
        });
    external
}

fn error_panel(ui: &mut egui::Ui, pal: &Palette, doc: &mut PdfDoc) -> bool {
    let mut external = false;
    ui.add_space((ui.available_height() * 0.25).min(140.0));
    ui.vertical_centered(|ui| {
        let Status::Error(e) = &doc.status else {
            return;
        };
        ui.label(
            RichText::new("This PDF cannot be displayed")
                .size(18.0)
                .color(pal.strong),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(e.to_string()).color(pal.unresolved));
        if *e == PdfError::Password {
            if doc.tried_password {
                ui.label(RichText::new("The password is wrong.").color(pal.muted));
            }
            ui.add_space(8.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut doc.password)
                    .password(true)
                    .hint_text("Password")
                    .desired_width(220.0),
            );
            let go = ui.button("Unlock").clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go && !doc.password.is_empty() {
                doc.unlock = Some(std::mem::take(&mut doc.password));
            }
        }
        ui.add_space(10.0);
        if ui.button("Open in default app").clicked() {
            external = true;
        }
    });
    external
}

/// Body of a PDF tab (below the usual view header).
pub fn pdf_tab(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    tab_id: TabId,
    path: &NotePath,
    _actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    let pal = Palette::of(&ctx);
    let pdf = &mut caches.pdf;
    let mut external = false;
    {
        let doc = pdf.ensure(&ctx, state, path);
        match &doc.status {
            Status::Loading => {
                centered_message(ui, RichText::new("Loading PDF…").color(pal.muted));
                return;
            }
            Status::Error(_) => {
                external = error_panel(ui, &pal, doc);
            }
            Status::Ready(_) => {}
        }
    }
    if let Some(pw) = pdf.docs.get_mut(path).and_then(|d| d.unlock.take()) {
        pdf.docs
            .insert(path.clone(), PdfDoc::open(&ctx, state, path, Some(pw)));
        ctx.request_repaint();
    }
    if external {
        state.open_in_system_app(path);
    }
    let PdfCaches { docs, views } = pdf;
    let Some(doc) = docs.get_mut(path) else {
        return;
    };
    let Some(sizes) = doc.sizes().map(<[_]>::to_vec) else {
        return;
    };
    if sizes.is_empty() {
        return;
    }
    let pages = sizes.len();
    let vs = views.entry(tab_id).or_default();
    let mut view = state.pdf_view(tab_id, path).clone();
    view.page = view.page.clamp(1, pages as u32);
    view.page_count = pages;

    // Zoom in effect.
    let avail_w = ui.available_width();
    let ref_w = sizes.iter().map(|s| s.0).fold(1.0_f32, f32::max);
    let mut zoom = if view.fit_width {
        fit_width_zoom(avail_w - 2.0 * MARGIN_X - SCROLLBAR, ref_w)
    } else {
        view.zoom
    };
    view.zoom = zoom;

    if toolbar(ui, &pal, &mut view, vs, tab_id, pages, zoom) {
        external = true;
    }
    if !view.fit_width {
        zoom = view.zoom;
    }

    // Ctrl+wheel / pinch zoom and keys.
    let area = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(area) {
        let z = ui.input(|i| i.zoom_delta());
        if (z - 1.0).abs() > 1e-3 {
            view.set_zoom(zoom * z);
            zoom = view.zoom;
        }
    }
    if state.modal.is_none() && !ctx.egui_wants_keyboard_input() {
        let (up, down, home, end) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::PageUp),
                i.key_pressed(egui::Key::PageDown),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
            )
        });
        if up {
            view.go_to(view.page.saturating_sub(1), pages);
        } else if down {
            view.go_to(view.page + 1, pages);
        } else if home {
            view.go_to(1, pages);
        } else if end {
            view.go_to(pages as u32, pages);
        }
    }

    let layout = PageLayout::new(&sizes, zoom, GAP);
    let mut forced: Option<f32> = None;
    if let Some(g) = view.goto.take() {
        forced = Some(layout.offset_of((g as usize).saturating_sub(1)));
    } else if vs.last_zoom > 0.0 && (zoom - vs.last_zoom).abs() > 1e-4 {
        // Keep what is on screen when the zoom changes.
        let old = PageLayout::new(&sizes, vs.last_zoom, GAP);
        forced = Some(layout.offset_at(old.anchor_at(vs.last_offset)));
    }
    vs.last_zoom = zoom;

    let needs_h = layout.max_width + 2.0 * MARGIN_X > avail_w - SCROLLBAR;
    let mut sa = if needs_h {
        egui::ScrollArea::both()
    } else {
        egui::ScrollArea::vertical()
    }
    .id_salt(("pdf-scroll", tab_id))
    .auto_shrink([false, false]);
    if let Some(y) = forced {
        sa = sa.vertical_scroll_offset(y);
    }
    let out = sa.show_viewport(ui, |ui, viewport| {
        let content_w = (layout.max_width + 2.0 * MARGIN_X).max(avail_w - SCROLLBAR);
        let (origin, _) =
            ui.allocate_exact_size(vec2(content_w, layout.total_height), Sense::hover());
        paint_visible(ui, doc, &pal, &layout, viewport, origin, zoom, false);
    });
    let offset = out.state.clamped_offset().y;
    if forced.is_some() {
        vs.last_offset = offset;
    } else if (offset - vs.last_offset).abs() > 0.5 {
        vs.last_offset = offset;
        view.page = layout.current_page(offset) as u32 + 1;
    }

    *state.pdf_view(tab_id, path) = view;
    if external {
        state.open_in_system_app(path);
    }
}

// ---------------------------------------------------------------------------
// Embed of the reading view
// ---------------------------------------------------------------------------

/// `![[doc.pdf]]` (scrollable, about 400 px high) or `![[doc.pdf#page=3]]`
/// (that page only); `|height` sets the height. A click opens the PDF in a
/// tab at that page.
pub fn pdf_embed(
    ui: &mut egui::Ui,
    state: &AppState,
    caches: &mut UiCaches,
    path: &NotePath,
    params: &PdfParams,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    let pal = Palette::of(&ctx);
    let width = ui.available_width().min(EMBED_MAX_WIDTH);
    let doc = caches.pdf.ensure(&ctx, state, path);
    let sizes: Vec<(f32, f32)> = match &doc.status {
        Status::Ready(s) if !s.is_empty() => s.clone(),
        Status::Ready(_) => Vec::new(),
        Status::Loading => {
            let (rect, _) = ui.allocate_exact_size(
                vec2(width, params.height.unwrap_or(EMBED_HEIGHT)),
                Sense::hover(),
            );
            ui.painter().rect_filled(rect, 4.0, pal.bg_secondary);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("Loading {}…", path.file_name()),
                FontId::proportional(14.0),
                pal.muted,
            );
            return;
        }
        Status::Error(e) => {
            let r = ui.add(
                egui::Label::new(
                    RichText::new(format!("⚠ {}: {e}", path.file_name())).color(pal.unresolved),
                )
                .sense(Sense::click()),
            );
            if r.clicked() {
                actions.push(Action::OpenPdf(path.clone(), 1, OpenIn::NewTab));
            }
            return;
        }
    };
    if sizes.is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    if let Some(p) = params.page {
        // One page.
        let idx = (p as usize - 1).min(sizes.len() - 1);
        let (w, h) = sizes[idx];
        let zoom = match params.height {
            Some(target) => target / (h * PT_TO_PX),
            None => width / (w * PT_TO_PX),
        };
        let zoom = zoom.min(width / (w * PT_TO_PX)).max(0.05);
        let size = vec2(w * PT_TO_PX * zoom, h * PT_TO_PX * zoom);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let wanted = doc.missing(std::iter::once(idx), zoom, ppp);
        if !wanted.is_empty() {
            doc.request(&ctx, wanted);
        }
        paint_page(ui, doc, &pal, rect, idx, zoom, ppp);
        let r = ui
            .interact(rect, ui.scope_id().with(("pdf-embed", idx)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("Open {} at page {}", path.file_name(), idx + 1));
        if r.clicked() {
            actions.push(Action::OpenPdf(
                path.clone(),
                idx as u32 + 1,
                OpenIn::NewTab,
            ));
        }
        return;
    }
    // Whole document, scrollable.
    let height = params.height.unwrap_or(EMBED_HEIGHT);
    let zoom = fit_width_zoom(width - 2.0 * 8.0 - SCROLLBAR, sizes[0].0);
    let layout = PageLayout::new(&sizes, zoom, 8.0);
    let mut clicked = None;
    egui::Frame::new()
        .fill(pal.bg_secondary)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(4)
        .show(ui, |ui| {
            ui.set_width(width - 2.0);
            egui::ScrollArea::vertical()
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    let (origin, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), layout.total_height),
                        Sense::hover(),
                    );
                    clicked = paint_visible(ui, doc, &pal, &layout, viewport, origin, zoom, true);
                });
        });
    if let Some(i) = clicked {
        actions.push(Action::OpenPdf(path.clone(), i as u32 + 1, OpenIn::NewTab));
    }
}

#[cfg(test)]
mod tests;
