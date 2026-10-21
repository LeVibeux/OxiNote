//! Headless egui tests of the PDF tab and embeds.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use eframe::egui::{Event, Key, Modifiers, RawInput};

use super::*;
use crate::fake::demo_vault;
use crate::settings::{RecentVaults, Settings};
use crate::ui::{ObsiApp, Startup, fake_index};

const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/test.pdf");

fn pdf_path() -> NotePath {
    NotePath::new("test.pdf").expect("path")
}

fn app() -> ObsiApp {
    let vault = demo_vault();
    vault.add_binary(&pdf_path(), FIXTURE.to_vec());
    let _ = ors_core::VaultWrite::write(
        &vault,
        &NotePath::new("Embeds.md").expect("path"),
        "# Embeds\n\n![[test.pdf#page=2|300]]\n\nand ![[test.pdf]]\n",
    );
    let settings = Settings::default();
    let state = AppState::new(Box::new(vault), fake_index, settings.clone());
    ObsiApp {
        state: Some(state),
        settings,
        recent: RecentVaults::default(),
        recent_file: None,
        picker_input: String::new(),
        picker_error: None,
        caches: UiCaches::default(),
        startup: Startup::default(),
        frame: 0,
        shot_done: Arc::new(AtomicBool::new(false)),
        shot_requested: false,
        theme_applied: None,
        look_applied: None,
        start: Instant::now(),
        title: String::new(),
    }
}

fn frame(app: &mut ObsiApp, ctx: &egui::Context, events: Vec<Event>) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0))),
        events,
        ..RawInput::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.workspace(ui));
    out.textures_delta.clear();
}

/// Run frames until `done` holds (the render thread is asynchronous).
fn until(app: &mut ObsiApp, ctx: &egui::Context, done: impl Fn(&ObsiApp) -> bool) {
    let t = Instant::now();
    while !done(app) {
        assert!(t.elapsed() < Duration::from_secs(30), "timeout");
        frame(app, ctx, Vec::new());
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn st(app: &mut ObsiApp) -> &mut AppState {
    app.state.as_mut().expect("vault")
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn opens_in_a_tab_and_goes_to_page_two() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = pdf_path();
    st(&mut app).open(&path, OpenIn::CurrentTab);
    frame(&mut app, &ctx, Vec::new());
    until(&mut app, &ctx, |a| {
        a.caches.pdf.page_count(&pdf_path()) == Some(3)
    });
    // The first page gets rendered in the background.
    until(&mut app, &ctx, |a| {
        a.caches.pdf.texture_count(&pdf_path()) >= 1
    });
    let id = st(&mut app).active_tab().map(|t| t.id).expect("tab");
    assert_eq!(st(&mut app).pdf_view(id, &path).page, 1);

    frame(&mut app, &ctx, vec![key(Key::PageDown)]);
    frame(&mut app, &ctx, Vec::new());
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(st(&mut app).pdf_view(id, &path).page, 2);
    // Page 2 gets rendered once it is in view, and stays the current page.
    until(&mut app, &ctx, |a| {
        a.caches.pdf.texture_count(&pdf_path()) >= 2
    });
    assert_eq!(st(&mut app).pdf_view(id, &path).page, 2);
}

#[test]
fn link_to_a_page_opens_the_tab_there() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = pdf_path();
    st(&mut app).open_pdf_at(&path, 3, OpenIn::NewTab);
    let id = st(&mut app).active_tab().map(|t| t.id).expect("tab");
    until(&mut app, &ctx, |a| {
        a.caches.pdf.page_count(&pdf_path()) == Some(3)
    });
    for _ in 0..3 {
        frame(&mut app, &ctx, Vec::new());
    }
    assert_eq!(st(&mut app).pdf_view(id, &path).page, 3);
    assert!(st(&mut app).pdf_view(id, &path).goto.is_none());
}

#[test]
fn corrupted_pdf_shows_an_error_without_panic() {
    let ctx = egui::Context::default();
    let mut app = app();
    let bad = NotePath::new("broken.pdf").expect("path");
    if let Some(s) = app.state.as_mut() {
        let _ = s.vault.write(&bad, "this is not a pdf");
        s.refresh_files();
        s.open(&bad, OpenIn::CurrentTab);
    }
    until(&mut app, &ctx, |a| {
        a.caches
            .pdf
            .docs
            .get(&NotePath::new("broken.pdf").expect("path"))
            .is_some_and(|d| matches!(d.status, Status::Error(_)))
    });
    frame(&mut app, &ctx, Vec::new());
}

#[test]
fn reading_view_embeds_render_pages() {
    let ctx = egui::Context::default();
    let mut app = app();
    let note = NotePath::new("Embeds.md").expect("path");
    st(&mut app).open(&note, OpenIn::CurrentTab);
    st(&mut app).toggle_mode();
    until(&mut app, &ctx, |a| {
        a.caches.pdf.page_count(&pdf_path()) == Some(3)
    });
    until(&mut app, &ctx, |a| {
        a.caches.pdf.texture_count(&pdf_path()) >= 2
    });
}

#[test]
fn unused_documents_are_closed() {
    let ctx = egui::Context::default();
    let mut app = app();
    st(&mut app).open(&pdf_path(), OpenIn::CurrentTab);
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(app.caches.pdf.doc_count(), 1);
    app.caches.pdf.gc(1e9);
    assert_eq!(app.caches.pdf.doc_count(), 0);
}
