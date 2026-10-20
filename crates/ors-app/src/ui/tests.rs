//! Headless UI smoke tests: real egui frames (no window, no GPU) driving
//! the whole workspace with synthetic input.

use eframe::egui::{self, Event, Key, Modifiers, RawInput, Rect, pos2};

use super::*;

fn app() -> ObsiApp {
    let settings = Settings {
        autosave_delay: 0.0,
        ..Settings::default()
    };
    let state = AppState::new(Box::new(demo_vault()), fake_index, settings.clone());
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
        start: Instant::now(),
        title: String::new(),
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn frame(app: &mut ObsiApp, ctx: &egui::Context, events: Vec<Event>) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..RawInput::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.workspace(ui));
    out.textures_delta.clear();
}

fn frames(app: &mut ObsiApp, ctx: &egui::Context, n: usize) {
    for _ in 0..n {
        frame(app, ctx, Vec::new());
    }
}

fn state(app: &mut ObsiApp) -> &mut AppState {
    app.state.as_mut().expect("vault open")
}

#[test]
fn quick_switcher_opens_note() {
    let ctx = egui::Context::default();
    let mut app = app();
    frames(&mut app, &ctx, 2);
    frame(&mut app, &ctx, vec![key(Key::O, Modifiers::COMMAND)]);
    assert!(matches!(
        state(&mut app).modal,
        Some(Modal::QuickSwitcher { .. })
    ));
    frames(&mut app, &ctx, 1);
    frame(&mut app, &ctx, vec![Event::Text("rust notes".into())]);
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 1);
    let s = state(&mut app);
    assert!(s.modal.is_none());
    assert_eq!(
        s.active_note().map(|p| p.stem().to_string()).as_deref(),
        Some("Rust notes")
    );
}

#[test]
fn typing_edits_and_autosaves() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = NotePath::new("Orpheline.md").expect("path");
    state(&mut app).open_at_line(&path, 0, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    frame(&mut app, &ctx, vec![Event::Text("Hello ".into())]);
    assert!(state(&mut app).buffers[&path].text.starts_with("Hello "));
    frames(&mut app, &ctx, 2);
    let disk = state(&mut app).vault.read_to_string(&path).expect("read");
    assert!(disk.starts_with("Hello "), "not autosaved: {disk}");
    assert!(state(&mut app).note_info().word_count > 0);
}

#[test]
fn link_completion_inserts_link() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = NotePath::new("Orpheline.md").expect("path");
    state(&mut app).open_at_line(&path, 0, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    frame(&mut app, &ctx, vec![Event::Text("[[Rust no".into())]);
    assert!(app.caches.completion.is_some());
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 1);
    let text = state(&mut app).buffers[&path].text.clone();
    assert!(text.starts_with("[[Rust notes]]"), "{text}");
}

#[test]
fn palette_toggle_reading_graph_and_close() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = NotePath::new("Idées.md").expect("path");
    state(&mut app).open(&path, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 2);
    frame(&mut app, &ctx, vec![key(Key::E, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 2);
    let mode = state(&mut app)
        .active_tab()
        .and_then(|t| t.note())
        .map(|n| n.mode);
    assert_eq!(mode, Some(ViewMode::Reading));
    frame(&mut app, &ctx, vec![key(Key::P, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 1);
    frame(&mut app, &ctx, vec![Event::Text("open graph view".into())]);
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 3);
    assert_eq!(state(&mut app).tabs.len(), 2);
    assert!(state(&mut app).active_note().is_none());
    frame(&mut app, &ctx, vec![key(Key::W, Modifiers::COMMAND)]);
    frame(&mut app, &ctx, vec![key(Key::W, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 1);
    assert!(state(&mut app).tabs.is_empty());
}

#[test]
fn all_panes_render() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = NotePath::new("Accueil.md").expect("path");
    state(&mut app).open(&path, OpenIn::CurrentTab);
    for left in [LeftPane::Files, LeftPane::Search, LeftPane::Tags] {
        for right in [
            RightPane::Backlinks,
            RightPane::Outgoing,
            RightPane::Outline,
        ] {
            let s = state(&mut app);
            s.settings.left_pane = left;
            s.settings.right_pane = right;
            s.search_query = "rust".into();
            frames(&mut app, &ctx, 2);
        }
    }
    state(&mut app).execute(Command::OpenLocalGraph);
    frames(&mut app, &ctx, 2);
    state(&mut app).execute(Command::SwitchVault);
    frames(&mut app, &ctx, 1);
    assert!(app.state.is_none());
}
