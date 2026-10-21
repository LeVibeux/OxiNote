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
        look_applied: None,
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
    for left in [
        LeftPane::Files,
        LeftPane::Search,
        LeftPane::Tags,
        LeftPane::Properties,
    ] {
        for right in [
            RightPane::Backlinks,
            RightPane::Outgoing,
            RightPane::Outline,
            RightPane::LocalGraph,
            RightPane::FileProperties,
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
    assert!(state(&mut app).views.local_pane.is_some());
    state(&mut app).execute(Command::SwitchVault);
    frames(&mut app, &ctx, 1);
    assert!(app.state.is_none());
}

fn open_empty(app: &mut ObsiApp, ctx: &egui::Context) -> NotePath {
    let path = NotePath::new("Orpheline.md").expect("path");
    state(app).open_at_line(&path, 0, OpenIn::CurrentTab);
    state(app)
        .buffers
        .get_mut(&path)
        .expect("buffer")
        .text
        .clear();
    frames(app, ctx, 3);
    path
}

fn text_of(app: &mut ObsiApp, path: &NotePath) -> String {
    state(app).buffers[path].text.clone()
}

#[test]
fn enter_continues_lists_and_empty_item_ends_them() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = open_empty(&mut app, &ctx);
    frame(&mut app, &ctx, vec![Event::Text("- idée".into())]);
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 1);
    assert_eq!(text_of(&mut app, &path), "- idée\n- ");
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 1);
    assert_eq!(text_of(&mut app, &path), "- idée\n");
}

#[test]
fn formatting_shortcuts_apply_to_the_word_at_the_caret() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = open_empty(&mut app, &ctx);
    frame(&mut app, &ctx, vec![Event::Text("un mot".into())]);
    frame(&mut app, &ctx, vec![key(Key::B, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 2);
    assert_eq!(text_of(&mut app, &path), "un **mot**");
    frame(&mut app, &ctx, vec![key(Key::L, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 2);
    assert_eq!(text_of(&mut app, &path), "- [ ] un **mot**");
    frame(&mut app, &ctx, vec![key(Key::L, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 2);
    assert_eq!(text_of(&mut app, &path), "- [x] un **mot**");
}

#[test]
fn ctrl_f_opens_find_bar_and_hash_completes_tags() {
    let ctx = egui::Context::default();
    let mut app = app();
    let _ = open_empty(&mut app, &ctx);
    frame(&mut app, &ctx, vec![key(Key::F, Modifiers::COMMAND)]);
    frames(&mut app, &ctx, 3);
    assert!(app.caches.find.is_some());
    // Close it again with Escape, then complete a tag.
    frame(&mut app, &ctx, vec![key(Key::Escape, Modifiers::NONE)]);
    frames(&mut app, &ctx, 2);
    assert!(app.caches.find.is_none());
    frame(&mut app, &ctx, vec![Event::Text("#r".into())]);
    frames(&mut app, &ctx, 1);
    let c = app.caches.completion.as_ref().map(|c| c.items.len());
    assert!(c.is_some_and(|n| n > 0), "no tag suggestions");
}

#[test]
fn typing_brackets_auto_pairs_and_backspace_removes_the_pair() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = open_empty(&mut app, &ctx);
    frame(&mut app, &ctx, vec![Event::Text("[".into())]);
    frames(&mut app, &ctx, 1);
    assert_eq!(text_of(&mut app, &path), "[]");
    frame(&mut app, &ctx, vec![Event::Text("[".into())]);
    frames(&mut app, &ctx, 1);
    assert_eq!(text_of(&mut app, &path), "[[]]");
    frame(&mut app, &ctx, vec![key(Key::Backspace, Modifiers::NONE)]);
    frames(&mut app, &ctx, 1);
    assert_eq!(text_of(&mut app, &path), "[]");
}

// ---------------------------------------------------------------------------
// Properties and tags (on a temporary copy of the sample vault)
// ---------------------------------------------------------------------------

struct TempVault(std::path::PathBuf);

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for e in std::fs::read_dir(from).expect("read_dir").flatten() {
        let target = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).expect("copy");
        }
    }
}

fn real_app(tag: &str) -> (ObsiApp, TempVault) {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-vault");
    let dir = std::env::temp_dir().join(format!("ors-app-props-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&src, &dir);
    let mut app = app();
    let settings = Settings {
        autosave_delay: 60.0,
        ..Settings::default()
    };
    let vault = DiskVault::open(&dir).expect("open vault");
    app.state = Some(AppState::new(Box::new(vault), real_index, settings));
    (app, TempVault(dir))
}

fn click(ctx: &egui::Context, app: &mut ObsiApp, id: egui::Id) {
    let rect = ctx.read_response(id).expect("widget drawn").rect;
    let pos = rect.center();
    let button = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(app, ctx, vec![Event::PointerMoved(pos)]);
    frame(app, ctx, vec![button(true)]);
    frame(app, ctx, vec![button(false)]);
    frames(app, ctx, 2);
}

fn prop_id(path: &str, key: &str, what: &str) -> egui::Id {
    egui::Id::unique(("prop", "doc", path, key, what))
}

#[test]
fn properties_panel_adds_a_property_with_the_command() {
    let (mut app, _dir) = real_app("add");
    let ctx = egui::Context::default();
    let path = NotePath::new("Idées.md").expect("path");
    state(&mut app).open(&path, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    frame(
        &mut app,
        &ctx,
        vec![key(Key::Semicolon, Modifiers::COMMAND)],
    );
    frames(&mut app, &ctx, 3);
    assert!(!state(&mut app).add_property_requested, "flag consumed");
    frame(&mut app, &ctx, vec![Event::Text("rating".into())]);
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 3);
    let text = state(&mut app).buffers[&path].text.clone();
    assert!(text.contains("\nrating:"), "{text}");
    assert!(text.contains("status: brouillon"), "other properties kept");
    assert!(state(&mut app).index.properties().contains_key("rating"));
}

#[test]
fn properties_panel_edits_a_text_value() {
    let (mut app, _dir) = real_app("edit");
    let ctx = egui::Context::default();
    let path = NotePath::new("Idées.md").expect("path");
    state(&mut app).open(&path, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    ctx.memory_mut(|m| m.request_focus(prop_id("Idées.md", "status", "text")));
    frames(&mut app, &ctx, 2);
    frame(&mut app, &ctx, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&mut app, &ctx, vec![Event::Text("terminé".into())]);
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 3);
    let text = state(&mut app).buffers[&path].text.clone();
    assert!(text.contains("status: terminé"), "{text}");
    assert!(state(&mut app).buffers[&path].is_dirty());
    state(&mut app).settings.autosave_delay = 0.0;
    frames(&mut app, &ctx, 3);
    let disk = state(&mut app).vault.read_to_string(&path).expect("read");
    assert!(disk.contains("status: terminé"), "{disk}");
}

#[test]
fn properties_panel_removes_a_tag_pill() {
    let (mut app, _dir) = real_app("pill");
    let ctx = egui::Context::default();
    let path = NotePath::new("Idées.md").expect("path");
    state(&mut app).open(&path, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    let x = prop_id("Idées.md", "tags", "pill0").with("x");
    click(&ctx, &mut app, x);
    let text = state(&mut app).buffers[&path].text.clone();
    assert!(!text.contains("- idée"), "{text}");
    assert!(text.contains("projet/obsirust"), "{text}");
}

#[test]
fn properties_views_render_in_all_modes() {
    let (mut app, _dir) = real_app("modes");
    let ctx = egui::Context::default();
    let path = NotePath::new("Idées.md").expect("path");
    state(&mut app).open(&path, OpenIn::CurrentTab);
    for left in [LeftPane::Properties, LeftPane::Tags] {
        state(&mut app).settings.left_pane = left;
        state(&mut app).settings.right_pane = RightPane::FileProperties;
        frames(&mut app, &ctx, 2);
    }
    state(&mut app).toggle_mode();
    frames(&mut app, &ctx, 2);
    for mode in [
        crate::settings::PropertiesInDocument::Hidden,
        crate::settings::PropertiesInDocument::Source,
    ] {
        state(&mut app).settings.properties_in_document = mode;
        frames(&mut app, &ctx, 2);
    }
    let bad = NotePath::new("Orpheline.md").expect("path");
    state(&mut app)
        .vault
        .write(&bad, "---\na: [unclosed\n---\nbody\n")
        .expect("write");
    state(&mut app).settings.properties_in_document =
        crate::settings::PropertiesInDocument::Visible;
    state(&mut app).open(&bad, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
}

#[test]
fn renaming_a_tag_from_the_dialog_updates_notes_and_buffers() {
    let (mut app, _dir) = real_app("tagrename");
    let ctx = egui::Context::default();
    let idees = NotePath::new("Idées.md").expect("path");
    state(&mut app).open(&idees, OpenIn::CurrentTab);
    state(&mut app).settings.left_pane = LeftPane::Tags;
    frames(&mut app, &ctx, 3);
    let users = state(&mut app).index.files_with_tag("idée");
    assert!(!users.is_empty());
    state(&mut app).modal = Some(Modal::Props(
        crate::properties_model::PropsModal::RenameTag {
            old: "idée".into(),
            new: "idée".into(),
        },
    ));
    frames(&mut app, &ctx, 3);
    frame(&mut app, &ctx, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&mut app, &ctx, vec![Event::Text("pensée".into())]);
    frames(&mut app, &ctx, 2);
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 3);
    assert!(state(&mut app).modal.is_none());
    let s = state(&mut app);
    assert!(s.index.files_with_tag("idée").is_empty());
    assert_eq!(s.index.files_with_tag("pensée").len(), users.len());
    let disk = s.vault.read_to_string(&idees).expect("read");
    assert!(disk.contains("pensée"), "{disk}");
    assert!(s.buffers[&idees].text.contains("pensée"));
    assert!(!s.buffers[&idees].is_dirty(), "clean buffer reloaded");
}

// ---------------------------------------------------------------------------
// Editor: block completion, folding, inline images, line numbers
// ---------------------------------------------------------------------------

fn open_with_text(app: &mut ObsiApp, ctx: &egui::Context, name: &str, text: &str) -> NotePath {
    let path = NotePath::new(name).expect("path");
    state(app).open_at_line(&path, 0, OpenIn::CurrentTab);
    if let Some(b) = state(app).buffers.get_mut(&path) {
        b.text = text.to_string();
    }
    frames(app, ctx, 4);
    path
}

#[test]
fn block_completion_lists_the_blocks_of_the_note() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("blocks");
    let path = open_with_text(
        &mut app,
        &ctx,
        "Blocs.md",
        "Un paragraphe important ^imp\n\n",
    );
    frame(&mut app, &ctx, vec![Event::Text("[[#^".into())]);
    frames(&mut app, &ctx, 1);
    let c = app.caches.completion.as_ref().expect("completion");
    assert_eq!(c.items.len(), 1, "{:?}", c.items);
    assert_eq!(c.items[0].insert, "imp");
    assert!(c.items[0].label.contains("Un paragraphe important"));
    frame(&mut app, &ctx, vec![key(Key::Enter, Modifiers::NONE)]);
    frames(&mut app, &ctx, 1);
    assert!(text_of(&mut app, &path).starts_with("[[#^imp]]"));
}

#[test]
fn fold_all_hides_sections_and_unfold_all_restores_them() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("folds");
    let text = "# Titre\nligne un\nligne deux\n## Sous\nsuite\n";
    let path = open_with_text(&mut app, &ctx, "Plis.md", text);
    state(&mut app).editor.pending.push(Command::FoldAll);
    frames(&mut app, &ctx, 3);
    let tab = state(&mut app).active_tab().expect("tab").id;
    assert_eq!(
        state(&mut app).editor.folds.get(&tab).map(Vec::len),
        Some(2)
    );
    // The text itself is untouched.
    assert_eq!(text_of(&mut app, &path), text);
    state(&mut app).editor.pending.push(Command::UnfoldAll);
    frames(&mut app, &ctx, 2);
    assert!(!state(&mut app).editor.folds.contains_key(&tab));
}

#[test]
fn images_and_line_numbers_render_without_touching_the_text() {
    let ctx = egui::Context::default();
    let (mut app, dir) = real_app("images");
    let mut png = Vec::new();
    image::RgbaImage::new(40, 20)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png");
    std::fs::write(dir.0.join("pic.png"), &png).expect("write png");
    {
        let s = state(&mut app);
        s.index = real_index(&*s.vault);
        s.settings.show_line_numbers = true;
    }
    let text = "avant\n![[pic.png|30]]\naprès\n";
    let path = open_with_text(&mut app, &ctx, "Img.md", text);
    let placed = app
        .caches
        .embeds
        .placed
        .values()
        .next()
        .cloned()
        .unwrap_or_default();
    assert_eq!(placed.len(), 1, "image not laid out");
    assert_eq!((placed[0].line, placed[0].size.x), (1, 30.0));
    assert_eq!(placed[0].size.y, 15.0);
    assert_eq!(text_of(&mut app, &path), text);
}
