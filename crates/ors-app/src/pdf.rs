//! PDF support: model and rendering (no egui here).
//!
//! * [`PdfRenderer`] hides the rendering engine. The default engine is
//!   [`HayroRenderer`] (`hayro`: pure Rust, MIT/Apache-2.0, CPU rasteriser,
//!   no system library), so a different engine can be plugged in later.
//! * [`PdfService`] owns a worker thread that opens the document and renders
//!   the pages the UI asks for, most important first, without ever blocking
//!   the interface.
//! * [`PageLayout`] is the continuous vertical layout (which pages are
//!   visible, where a page starts...).
//! * [`LruCache`] is the bounded cache the UI keeps its textures in.
//! * [`PdfParams`] parses `#page=3&height=400` and `|400` of links/embeds.
//! * [`PdfViewState`] is the per-tab page / zoom, saved with the workspace.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use ors_core::{NotePath, Subpath};

use crate::state::{AppState, OpenIn, TabId};

// ---------------------------------------------------------------------------
// Engine abstraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// Encrypted document and no (or a wrong) password.
    Password,
    /// Corrupted or unsupported file.
    Invalid(String),
    /// Rendering of one page failed.
    Render(String),
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfError::Password => write!(f, "This PDF is protected by a password."),
            PdfError::Invalid(m) => write!(f, "This PDF cannot be opened ({m})."),
            PdfError::Render(m) => write!(f, "Cannot render this page ({m})."),
        }
    }
}

impl std::error::Error for PdfError {}

/// A rendered page: opaque RGBA8 pixels, row by row.
#[derive(Debug, Clone)]
pub struct PdfBitmap {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl PdfBitmap {
    pub fn byte_size(&self) -> usize {
        self.rgba.len()
    }
}

/// A PDF engine. Implementations need not be `Send`: the service builds
/// one inside its worker thread.
pub trait PdfRenderer {
    fn page_count(&self) -> usize;
    /// Size of page `page` (0-based) in points, rotation applied.
    fn page_size(&self, page: usize) -> Option<(f32, f32)>;
    /// Render a page; `scale` = pixels per point.
    fn render(&self, page: usize, scale: f32) -> Result<PdfBitmap, PdfError>;
}

/// Largest side, in pixels, of a rendered page.
const MAX_SIDE_PX: f32 = 6000.0;

/// Engine based on the pure Rust `hayro` crate.
pub struct HayroRenderer {
    pdf: hayro::hayro_syntax::Pdf,
}

impl HayroRenderer {
    pub fn open(data: Vec<u8>, password: Option<&str>) -> Result<Self, PdfError> {
        use hayro::hayro_syntax::{LoadPdfError, Pdf, PdfData};
        let loaded = match password {
            Some(p) => Pdf::new_with_password(PdfData::from(data), p),
            None => Pdf::new(data),
        };
        match loaded {
            Ok(pdf) if pdf.pages().is_empty() => Err(PdfError::Invalid("no pages".into())),
            Ok(pdf) => Ok(Self { pdf }),
            Err(LoadPdfError::Decryption(e)) => {
                use hayro::hayro_syntax::DecryptionError as D;
                match e {
                    D::PasswordProtected => Err(PdfError::Password),
                    D::UnsupportedAlgorithm => {
                        Err(PdfError::Invalid("unsupported encryption".into()))
                    }
                    _ => Err(PdfError::Invalid("invalid encryption".into())),
                }
            }
            Err(LoadPdfError::Invalid) => Err(PdfError::Invalid("not a valid PDF file".into())),
        }
    }
}

impl PdfRenderer for HayroRenderer {
    fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    fn page_size(&self, page: usize) -> Option<(f32, f32)> {
        self.pdf.pages().get(page).map(|p| p.render_dimensions())
    }

    fn render(&self, page: usize, scale: f32) -> Result<PdfBitmap, PdfError> {
        use hayro::vello_cpu::color::palette::css::WHITE;
        use hayro::{PixmapSettings, RenderCache, RenderSettings};
        let p = self
            .pdf
            .pages()
            .get(page)
            .ok_or_else(|| PdfError::Render("no such page".into()))?;
        let (w, h) = p.render_dimensions();
        let scale = scale.min(MAX_SIDE_PX / w.max(h).max(1.0)).max(0.01);
        let settings = hayro::hayro_interpret::InterpreterSettings::default();
        let cache = RenderCache::new();
        // The engine may panic on exotic files: never take the app down.
        let pix = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hayro::render(
                p,
                &cache,
                &settings,
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: WHITE,
                },
            )
        }))
        .map_err(|_| PdfError::Render("the engine failed on this page".into()))?;
        Ok(PdfBitmap {
            width: usize::from(pix.width()),
            height: usize::from(pix.height()),
            rgba: pix.data_as_u8_slice().to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------
// Zoom
// ---------------------------------------------------------------------------

/// Zoom 1.0 shows a page at 96 dpi (like a browser): 1 pt = 4/3 px.
pub const PT_TO_PX: f32 = 96.0 / 72.0;
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 5.0;
const ZOOM_STEPS: [f32; 17] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

pub fn clamp_zoom(z: f32) -> f32 {
    if z.is_finite() {
        z.clamp(MIN_ZOOM, MAX_ZOOM)
    } else {
        1.0
    }
}

/// Next preset zoom above (`up`) or below the current one.
pub fn step_zoom(zoom: f32, up: bool) -> f32 {
    let z = clamp_zoom(zoom);
    if up {
        ZOOM_STEPS
            .iter()
            .copied()
            .find(|s| *s > z + 0.005)
            .unwrap_or(MAX_ZOOM)
    } else {
        ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|s| *s < z - 0.005)
            .unwrap_or(MIN_ZOOM)
    }
}

/// Zoom that makes a page `page_width_pt` wide fill `avail_px`.
pub fn fit_width_zoom(avail_px: f32, page_width_pt: f32) -> f32 {
    if page_width_pt <= 0.0 || avail_px <= 0.0 {
        return 1.0;
    }
    clamp_zoom(avail_px / (page_width_pt * PT_TO_PX))
}

/// Cache key of a rendered page: page and *rounded* render scale, so that a
/// small zoom change reuses the texture and Ctrl+wheel does not render
/// hundreds of bitmaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderKey {
    pub page: usize,
    /// Pixels per point x 100, rounded to a multiple of 10.
    pub scale_pct: u32,
}

impl RenderKey {
    /// `zoom` is the view zoom, `ppp` egui's pixels per point.
    pub fn new(page: usize, zoom: f32, ppp: f32) -> Self {
        let raw = clamp_zoom(zoom) * PT_TO_PX * ppp.max(0.5) * 100.0;
        let rounded = (raw / 10.0).round() * 10.0;
        Self {
            page,
            scale_pct: (rounded as u32).max(10),
        }
    }

    /// Pixels per PDF point.
    pub fn scale(&self) -> f32 {
        self.scale_pct as f32 / 100.0
    }
}

// ---------------------------------------------------------------------------
// Background service
// ---------------------------------------------------------------------------

pub enum PdfEvent {
    /// The document is open: page sizes in points.
    Opened(Vec<(f32, f32)>),
    Failed(PdfError),
    Rendered(RenderKey, PdfBitmap),
    RenderFailed(RenderKey),
}

#[derive(Default)]
struct Shared {
    /// Pages to render, most important first.
    wanted: Vec<RenderKey>,
    current: Option<RenderKey>,
    closed: bool,
}

type Repaint = Arc<dyn Fn() + Send + Sync>;
type Factory = Box<dyn FnOnce() -> Result<Box<dyn PdfRenderer>, PdfError> + Send>;

/// A document open in a worker thread.
pub struct PdfService {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    rx: Receiver<PdfEvent>,
    handle: Option<JoinHandle<()>>,
}

impl PdfService {
    /// Open `data` with the default engine.
    pub fn spawn(data: Vec<u8>, password: Option<String>, repaint: Option<Repaint>) -> Self {
        Self::spawn_with(
            Box::new(move || {
                HayroRenderer::open(data, password.as_deref())
                    .map(|r| Box::new(r) as Box<dyn PdfRenderer>)
            }),
            repaint,
        )
    }

    /// Open a document with any engine (built inside the worker thread).
    pub fn spawn_with(factory: Factory, repaint: Option<Repaint>) -> Self {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let (tx, rx) = channel();
        let worker_shared = shared.clone();
        let handle = std::thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || worker(factory, worker_shared, tx, repaint))
            .ok();
        Self { shared, rx, handle }
    }

    /// Replace the list of pages to render (most important first). What
    /// is not listed any more is dropped, so fast scrolling does not pile
    /// up stale work.
    pub fn set_wanted(&self, keys: Vec<RenderKey>) {
        let (lock, cv) = &*self.shared;
        if let Ok(mut s) = lock.lock() {
            let current = s.current;
            s.wanted = keys.into_iter().filter(|k| Some(*k) != current).collect();
            cv.notify_one();
        }
    }

    /// Events produced since the last call.
    pub fn poll(&self) -> Vec<PdfEvent> {
        self.rx.try_iter().collect()
    }

    /// Wait for the next event (tests).
    #[cfg(test)]
    pub fn wait(&self) -> Option<PdfEvent> {
        self.rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .ok()
    }
}

impl Drop for PdfService {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        if let Ok(mut s) = lock.lock() {
            s.closed = true;
            s.wanted.clear();
        }
        cv.notify_one();
        // The worker exits on its own after the page it is rendering; do
        // not block the UI waiting for it.
        self.handle.take();
    }
}

fn worker(
    factory: Factory,
    shared: Arc<(Mutex<Shared>, Condvar)>,
    tx: Sender<PdfEvent>,
    repaint: Option<Repaint>,
) {
    let notify = || {
        if let Some(r) = &repaint {
            r();
        }
    };
    let renderer = match factory() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(PdfEvent::Failed(e));
            notify();
            return;
        }
    };
    let sizes: Vec<(f32, f32)> = (0..renderer.page_count())
        .map(|i| renderer.page_size(i).unwrap_or((612.0, 792.0)))
        .collect();
    if tx.send(PdfEvent::Opened(sizes)).is_err() {
        return;
    }
    notify();
    let (lock, cv) = &*shared;
    loop {
        let key = {
            let Ok(mut s) = lock.lock() else { return };
            loop {
                if s.closed {
                    return;
                }
                if !s.wanted.is_empty() {
                    let k = s.wanted.remove(0);
                    s.current = Some(k);
                    break k;
                }
                s = match cv.wait(s) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let event = match renderer.render(key.page, key.scale()) {
            Ok(bmp) => PdfEvent::Rendered(key, bmp),
            Err(_) => PdfEvent::RenderFailed(key),
        };
        if let Ok(mut s) = lock.lock() {
            s.current = None;
            if s.closed {
                return;
            }
        }
        if tx.send(event).is_err() {
            return;
        }
        notify();
    }
}

// ---------------------------------------------------------------------------
// Layout of the continuous view
// ---------------------------------------------------------------------------

/// Vertical stack of pages (in screen points).
#[derive(Debug, Clone, Default)]
pub struct PageLayout {
    pub tops: Vec<f32>,
    pub widths: Vec<f32>,
    pub heights: Vec<f32>,
    pub total_height: f32,
    pub max_width: f32,
    pub gap: f32,
}

#[allow(clippy::len_without_is_empty)]
impl PageLayout {
    pub fn new(sizes: &[(f32, f32)], zoom: f32, gap: f32) -> Self {
        let k = clamp_zoom(zoom) * PT_TO_PX;
        let mut l = PageLayout {
            gap,
            ..Self::default()
        };
        let mut y = gap;
        for (w, h) in sizes {
            l.tops.push(y);
            l.widths.push(w * k);
            l.heights.push(h * k);
            y += h * k + gap;
            l.max_width = l.max_width.max(w * k);
        }
        l.total_height = y;
        l
    }

    pub fn len(&self) -> usize {
        self.tops.len()
    }

    /// Pages intersecting `[top, bottom]`.
    pub fn visible(&self, top: f32, bottom: f32) -> Range<usize> {
        let n = self.len();
        let first = (0..n)
            .find(|&i| self.tops[i] + self.heights[i] >= top)
            .unwrap_or(n);
        let end = (first..n)
            .find(|&i| self.tops[i] > bottom)
            .unwrap_or(n)
            .max(first);
        first..end
    }

    /// Visible pages plus one before and one after.
    pub fn prefetch(&self, visible: &Range<usize>) -> Range<usize> {
        if visible.is_empty() {
            return 0..0;
        }
        visible.start.saturating_sub(1)..(visible.end + 1).min(self.len())
    }

    /// The page "being read" when the viewport starts at `top`: the first
    /// one whose bottom is still below a small margin.
    pub fn current_page(&self, top: f32) -> usize {
        let probe = top + 24.0;
        (0..self.len())
            .find(|&i| self.tops[i] + self.heights[i] > probe)
            .unwrap_or_else(|| self.len().saturating_sub(1))
    }

    /// Scroll offset showing `page` at the top of the view.
    pub fn offset_of(&self, page: usize) -> f32 {
        self.tops
            .get(page)
            .map_or(0.0, |t| (t - self.gap * 0.5).max(0.0))
    }

    /// `(page, fraction of its height)` at scroll offset `y`.
    pub fn anchor_at(&self, y: f32) -> (usize, f32) {
        let p = self.current_page(y);
        let h = self.heights.get(p).copied().unwrap_or(1.0).max(1.0);
        let top = self.tops.get(p).copied().unwrap_or(0.0);
        (p, ((y - top) / h).clamp(-1.0, 1.0))
    }

    /// Inverse of [`PageLayout::anchor_at`].
    pub fn offset_at(&self, anchor: (usize, f32)) -> f32 {
        let (p, frac) = anchor;
        match (self.tops.get(p), self.heights.get(p)) {
            (Some(t), Some(h)) => (t + h * frac).max(0.0),
            _ => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Bounded cache
// ---------------------------------------------------------------------------

/// Least-recently-used cache bounded by entry count and byte size.
pub struct LruCache<K, V> {
    map: HashMap<K, (V, usize, u64)>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
    tick: u64,
}

#[allow(clippy::len_without_is_empty)]
impl<K: std::hash::Hash + Eq + Copy, V> LruCache<K, V> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            bytes: 0,
            max_bytes,
            max_entries: max_entries.max(1),
            tick: 0,
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    /// Fetch and mark as recently used.
    pub fn get(&mut self, k: &K) -> Option<&V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.2 = t;
            &e.0
        })
    }

    /// Insert, then evict the least recently used entries (never the one
    /// just inserted) until the limits hold.
    pub fn insert(&mut self, k: K, v: V, bytes: usize) {
        self.tick += 1;
        if let Some((_, old, _)) = self.map.insert(k, (v, bytes, self.tick)) {
            self.bytes -= old;
        }
        self.bytes += bytes;
        while self.map.len() > 1
            && (self.map.len() > self.max_entries || self.bytes > self.max_bytes)
        {
            let Some(oldest) = self
                .map
                .iter()
                .filter(|(key, _)| **key != k)
                .min_by_key(|(_, e)| e.2)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some((_, b, _)) = self.map.remove(&oldest) {
                self.bytes -= b;
            }
        }
    }

    /// Entry satisfying `pick` that is the best match according to `score`
    /// (lowest wins); does not touch the LRU order.
    pub fn best_by<S: Ord>(&self, pick: impl Fn(&K) -> Option<S>) -> Option<(&K, &V)> {
        self.map
            .iter()
            .filter_map(|(k, e)| pick(k).map(|s| (s, k, &e.0)))
            .min_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, k, v)| (k, v))
    }
}

// ---------------------------------------------------------------------------
// Links and embeds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfParams {
    /// 1-based page.
    pub page: Option<u32>,
    /// Height of an embed, in points.
    pub height: Option<f32>,
}

/// Parse `page=3`, `#page=3&height=400`, `height=300`...
pub fn parse_params(s: &str) -> PdfParams {
    let mut p = PdfParams::default();
    for part in s.trim().trim_start_matches('#').split(['&', '#']) {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "page" => p.page = v.trim().parse::<u32>().ok().filter(|n| *n >= 1),
            "height" => p.height = parse_size(v),
            _ => {}
        }
    }
    p
}

fn parse_size(v: &str) -> Option<f32> {
    v.trim()
        .parse::<f32>()
        .ok()
        .filter(|h| h.is_finite() && *h >= 20.0)
        .map(|h| h.min(4000.0))
}

/// Parameters of a link/embed: subpath (`#page=3`) and display text
/// (`|400` or `|400x300`: the height is what counts).
pub fn params_from_link(subpath: Option<&Subpath>, display: Option<&str>) -> PdfParams {
    let mut p = match subpath {
        Some(Subpath::Heading(h)) => parse_params(&h.join("#")),
        _ => PdfParams::default(),
    };
    if let Some(d) = display {
        let d = d.trim();
        let h = match d.split_once(['x', 'X']) {
            Some((_, h)) => parse_size(h),
            None => parse_size(d),
        };
        if h.is_some() {
            p.height = h;
        } else {
            let q = parse_params(d);
            p.page = p.page.or(q.page);
            p.height = p.height.or(q.height);
        }
    }
    p
}

pub fn is_pdf(path: &NotePath) -> bool {
    path.extension() == "pdf"
}

// ---------------------------------------------------------------------------
// Per-tab view state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PdfViewState {
    pub path: NotePath,
    /// Current page, 1-based.
    pub page: u32,
    pub zoom: f32,
    /// Zoom follows the width of the view.
    pub fit_width: bool,
    /// Scroll to this page (1-based) on next draw.
    pub goto: Option<u32>,
}

impl PdfViewState {
    pub fn new(path: NotePath) -> Self {
        Self {
            path,
            page: 1,
            zoom: 1.0,
            fit_width: true,
            goto: None,
        }
    }

    /// Restored from the workspace file.
    pub fn restored(path: NotePath, page: Option<u32>, zoom: Option<f32>) -> Self {
        let mut s = Self::new(path);
        s.page = page.unwrap_or(1).max(1);
        s.goto = Some(s.page);
        if let Some(z) = zoom {
            s.zoom = clamp_zoom(z);
            s.fit_width = false;
        }
        s
    }

    /// Zoom to save: `None` while fitting the width.
    pub fn saved_zoom(&self) -> Option<f32> {
        (!self.fit_width).then_some(self.zoom)
    }

    pub fn go_to(&mut self, page: u32, page_count: usize) {
        let max = u32::try_from(page_count).unwrap_or(u32::MAX).max(1);
        let page = page.clamp(1, max);
        self.page = page;
        self.goto = Some(page);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = clamp_zoom(zoom);
        self.fit_width = false;
    }
}

impl AppState {
    /// View state of tab `tab`, reset when it now shows another file.
    pub fn pdf_view(&mut self, tab: TabId, path: &NotePath) -> &mut PdfViewState {
        let e = self
            .pdf_views
            .entry(tab)
            .or_insert_with(|| PdfViewState::new(path.clone()));
        if &e.path != path {
            *e = PdfViewState::new(path.clone());
        }
        e
    }

    /// Open a PDF at a page (1-based).
    pub fn open_pdf_at(&mut self, path: &NotePath, page: u32, open_in: OpenIn) {
        self.open(path, open_in);
        let Some(id) = self.active_tab().map(|t| t.id) else {
            return;
        };
        let v = self.pdf_view(id, path);
        v.page = page.max(1);
        v.goto = Some(v.page);
    }

    /// Open a file with the system's default application.
    pub fn open_in_system_app(&mut self, path: &NotePath) {
        let Some(abs) = self.absolute_path(&crate::state::FsTarget::File(path.clone())) else {
            self.info("This vault is not on disk");
            return;
        };
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(program).arg(&abs).spawn() {
            self.error(format!("Cannot open {path}: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/test.pdf");

    fn p(s: &str) -> NotePath {
        NotePath::new(s).unwrap()
    }

    #[test]
    fn page_count_and_sizes() {
        let r = HayroRenderer::open(FIXTURE.to_vec(), None).unwrap();
        assert_eq!(r.page_count(), 3);
        assert_eq!(r.page_size(0), Some((612.0, 792.0)));
        assert_eq!(r.page_size(1), Some((595.0, 842.0)));
        assert_eq!(r.page_size(2), Some((792.0, 612.0)));
        assert_eq!(r.page_size(3), None);
    }

    #[test]
    fn renders_a_page() {
        let r = HayroRenderer::open(FIXTURE.to_vec(), None).unwrap();
        let b = r.render(0, 0.5).unwrap();
        assert_eq!((b.width, b.height), (306, 396));
        assert_eq!(b.rgba.len(), 306 * 396 * 4);
        // Not blank: the page is tinted and carries dark text.
        let dark = b.rgba.chunks(4).filter(|px| px[0] < 100).count();
        let tinted = b
            .rgba
            .chunks(4)
            .filter(|px| px != &[255, 255, 255, 255])
            .count();
        assert!(dark > 50, "dark pixels: {dark}");
        assert!(tinted > b.width * b.height / 2);
        assert!(r.render(9, 1.0).is_err());
    }

    #[test]
    fn corrupted_files_are_errors() {
        assert!(matches!(
            HayroRenderer::open(b"not a pdf".to_vec(), None),
            Err(PdfError::Invalid(_))
        ));
        let cut = &FIXTURE[..FIXTURE.len() / 3];
        // A truncated file is repaired or refused, never a panic.
        let _ = HayroRenderer::open(cut.to_vec(), None);
        assert!(HayroRenderer::open(Vec::new(), None).is_err());
    }

    #[test]
    fn service_opens_and_renders_in_background() {
        let s = PdfService::spawn(FIXTURE.to_vec(), None, None);
        let Some(PdfEvent::Opened(sizes)) = s.wait() else {
            panic!("expected Opened");
        };
        assert_eq!(sizes.len(), 3);
        let k = RenderKey::new(1, 0.5, 1.0);
        s.set_wanted(vec![k]);
        let Some(PdfEvent::Rendered(got, bmp)) = s.wait() else {
            panic!("expected Rendered");
        };
        assert_eq!(got, k);
        assert!(bmp.width > 100 && bmp.height > bmp.width);
    }

    #[test]
    fn service_reports_a_broken_file() {
        let s = PdfService::spawn(b"%PDF-garbage".to_vec(), None, None);
        assert!(matches!(s.wait(), Some(PdfEvent::Failed(_))));
    }

    #[test]
    fn render_keys_round_the_zoom() {
        let a = RenderKey::new(0, 1.0, 1.0);
        let b = RenderKey::new(0, 1.01, 1.0);
        let c = RenderKey::new(0, 1.5, 1.0);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.scale_pct, 130);
        assert!(RenderKey::new(0, 1.0, 2.0).scale() > a.scale());
    }

    #[test]
    fn zoom_steps_and_fit() {
        assert_eq!(step_zoom(1.0, true), 1.1);
        assert_eq!(step_zoom(1.0, false), 0.9);
        assert_eq!(step_zoom(MAX_ZOOM, true), MAX_ZOOM);
        assert_eq!(step_zoom(MIN_ZOOM, false), MIN_ZOOM);
        assert_eq!(clamp_zoom(f32::NAN), 1.0);
        // 816 px wide at 100%.
        assert!((fit_width_zoom(816.0, 612.0) - 1.0).abs() < 1e-3);
        assert!((fit_width_zoom(408.0, 612.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn layout_visibility() {
        let sizes = vec![(600.0, 750.0); 10];
        let l = PageLayout::new(&sizes, 0.75, 10.0); // 600 x 750 px
        assert_eq!(l.len(), 10);
        assert_eq!(l.heights[0], 750.0);
        assert_eq!(l.tops[1], 10.0 + 750.0 + 10.0);
        assert_eq!(l.visible(0.0, 100.0), 0..1);
        assert_eq!(l.visible(0.0, 800.0), 0..2);
        let v = l.visible(1600.0, 2400.0);
        assert_eq!(v, 2..4);
        assert_eq!(l.prefetch(&v), 1..5);
        assert_eq!(l.prefetch(&(0..1)), 0..2);
        assert_eq!(l.prefetch(&(9..10)), 8..10);
        assert!(l.visible(1e9, 2e9).is_empty());
        assert_eq!(l.current_page(0.0), 0);
        assert_eq!(l.current_page(l.offset_of(4)), 4);
        let a = l.anchor_at(l.tops[3] + l.heights[3] * 0.5);
        assert_eq!(a.0, 3);
        assert!((a.1 - 0.5).abs() < 1e-3);
        assert!((l.offset_at(a) - (l.tops[3] + l.heights[3] * 0.5)).abs() < 1e-2);
    }

    #[test]
    fn cache_is_bounded() {
        let mut c: LruCache<u32, &str> = LruCache::new(3, 1000);
        for i in 0..5 {
            c.insert(i, "x", 10);
        }
        assert_eq!(c.len(), 3);
        assert!(!c.contains(&0) && !c.contains(&1) && c.contains(&4));
        // Touching an entry protects it.
        assert!(c.get(&2).is_some());
        c.insert(9, "y", 10);
        assert!(c.contains(&2) && !c.contains(&3));
        // Byte budget.
        let mut b: LruCache<u32, ()> = LruCache::new(100, 100);
        for i in 0..10 {
            b.insert(i, (), 30);
        }
        assert!(b.bytes() <= 100 && b.len() == 3);
        // A single oversized entry is kept (it is what is on screen).
        b.insert(99, (), 500);
        assert_eq!(b.len(), 1);
        assert_eq!(b.bytes(), 500);
        // Replacing does not double count.
        b.insert(99, (), 40);
        assert_eq!(b.bytes(), 40);
    }

    #[test]
    fn parses_page_and_height() {
        assert_eq!(
            parse_params("page=3"),
            PdfParams {
                page: Some(3),
                height: None
            }
        );
        assert_eq!(
            parse_params("#page=2&height=500"),
            PdfParams {
                page: Some(2),
                height: Some(500.0)
            }
        );
        assert_eq!(parse_params("page=0").page, None);
        assert_eq!(parse_params("page=abc").page, None);
        assert_eq!(parse_params("").page, None);
        let sub = Subpath::Heading(vec!["page=4".into()]);
        assert_eq!(params_from_link(Some(&sub), None).page, Some(4));
        let h = params_from_link(Some(&sub), Some("300"));
        assert_eq!((h.page, h.height), (Some(4), Some(300.0)));
        assert_eq!(params_from_link(None, Some("200x350")).height, Some(350.0));
        assert_eq!(params_from_link(None, Some("my doc")).height, None);
    }

    #[test]
    fn view_state_goto_and_restore() {
        let mut v = PdfViewState::new(p("a.pdf"));
        assert!(v.fit_width && v.saved_zoom().is_none());
        v.go_to(99, 3);
        assert_eq!((v.page, v.goto), (3, Some(3)));
        v.go_to(0, 3);
        assert_eq!(v.page, 1);
        v.set_zoom(2.0);
        assert_eq!(v.saved_zoom(), Some(2.0));
        let r = PdfViewState::restored(p("a.pdf"), Some(2), Some(1.5));
        assert_eq!((r.page, r.zoom, r.fit_width), (2, 1.5, false));
        assert_eq!(PdfViewState::restored(p("a.pdf"), None, None).page, 1);
    }

    #[test]
    fn is_pdf_by_extension() {
        assert!(is_pdf(&p("docs/A.PDF")));
        assert!(!is_pdf(&p("a.md")));
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::backend::MemVault;
    use crate::fake::FakeIndex;
    use crate::settings::Settings;

    fn fake(v: &dyn ors_core::VaultRead) -> Box<dyn crate::backend::IndexBackend> {
        Box::new(FakeIndex::build(v))
    }

    fn state() -> AppState {
        let v = MemVault::with_files("t", [("A.md", "# A")]);
        v.add_binary(
            &NotePath::new("d.pdf").unwrap(),
            include_bytes!("../tests/fixtures/test.pdf").to_vec(),
        );
        AppState::new(Box::new(v), fake, Settings::default())
    }

    #[test]
    fn page_and_zoom_survive_the_workspace() {
        let mut s = state();
        let path = NotePath::new("d.pdf").unwrap();
        s.open_pdf_at(&path, 2, OpenIn::NewTab);
        let id = s.active_tab().unwrap().id;
        s.pdf_view(id, &path).set_zoom(1.5);
        let saved = s.saved_workspace();
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("\"type\":\"pdf\""), "{json}");
        assert!(json.contains("\"page\":2"), "{json}");
        assert!(json.contains("\"zoom\":1.5"), "{json}");
        let back: crate::workbench::SavedWorkspace = serde_json::from_str(&json).unwrap();
        s.apply_saved_workspace(&back);
        let id = s.active_tab().unwrap().id;
        let v = s.pdf_view(id, &path).clone();
        assert_eq!((v.page, v.zoom, v.fit_width), (2, 1.5, false));
        assert_eq!(v.goto, Some(2));
        // Fit width is saved as "no zoom".
        s.pdf_view(id, &path).fit_width = true;
        let json = serde_json::to_string(&s.saved_workspace()).unwrap();
        assert!(!json.contains("\"zoom\""), "{json}");
    }

    #[test]
    fn closing_the_tab_forgets_the_view() {
        let mut s = state();
        let path = NotePath::new("d.pdf").unwrap();
        s.open_pdf_at(&path, 2, OpenIn::NewTab);
        assert_eq!(s.pdf_views.len(), 1);
        let i = s.active.unwrap();
        s.close_tab(i);
        assert!(s.pdf_views.is_empty());
    }
}
