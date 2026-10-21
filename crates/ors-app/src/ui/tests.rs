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
    egui::Id::unique(("prop", "doc1", path, key, what))
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
fn folded_callouts_are_reapplied_when_the_tab_changes_note() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("callout-nav");
    open_with_text(&mut app, &ctx, "Un.md", "just text\n");
    let tab = state(&mut app).active_tab().expect("tab").id;
    // Same tab, other note.
    open_with_text(&mut app, &ctx, "Deux.md", "> [!note]- Titre\n> corps\n");
    assert_eq!(state(&mut app).active_tab().expect("tab").id, tab);
    assert_eq!(
        state(&mut app).editor.folds.get(&tab).map(Vec::len),
        Some(1),
        "the callout of the second note starts folded"
    );
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

#[test]
fn modes_switch_per_tab_and_the_status_bar_follows() {
    use crate::editor::DisplayMode;
    let ctx = egui::Context::default();
    let mut app = app();
    let path = NotePath::new("Orpheline.md").expect("path");
    state(&mut app).open_at_line(&path, 0, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    let label = |app: &mut ObsiApp| state(app).display_mode().map(DisplayMode::label);
    assert_eq!(label(&mut app), Some("Live Preview"));
    // Command: Live Preview <-> Source.
    state(&mut app).execute(Command::ToggleLivePreview);
    assert_eq!(label(&mut app), Some("Source mode"));
    // Ctrl+E: editing <-> reading, the Source choice is remembered.
    frame(&mut app, &ctx, vec![key(Key::E, Modifiers::COMMAND)]);
    assert_eq!(label(&mut app), Some("Reading view"));
    frame(&mut app, &ctx, vec![key(Key::E, Modifiers::COMMAND)]);
    assert_eq!(label(&mut app), Some("Source mode"));
    // The header menu actions.
    apply(state(&mut app), Action::SetMode(DisplayMode::Reading));
    assert_eq!(label(&mut app), Some("Reading view"));
    apply(state(&mut app), Action::SetMode(DisplayMode::LivePreview));
    assert_eq!(label(&mut app), Some("Live Preview"));
    // Another tab keeps its own mode.
    state(&mut app).open(&NotePath::new("Accueil.md").expect("path"), OpenIn::NewTab);
    frames(&mut app, &ctx, 2);
    state(&mut app).execute(Command::ToggleLivePreview);
    assert_eq!(label(&mut app), Some("Source mode"));
    state(&mut app).active = Some(0);
    assert_eq!(label(&mut app), Some("Live Preview"));
}

#[test]
fn source_mode_shows_frontmatter_and_hides_no_marker() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("source");
    let text =
        "---\ntitle: T\n---\n# Titre\n\n**gras** et [[Accueil]]\n\n```rust\nlet x = 1;\n```\n";
    open_with_text(&mut app, &ctx, "Src.md", text);
    let tab = state(&mut app).active_tab().expect("tab").id;
    let hidden_sections = |app: &ObsiApp| {
        let (_, job) = &app.caches.highlight[&tab];
        assert_eq!(job.text, text);
        job.sections
            .iter()
            .filter(|s| s.format.font_id.size < 1.0)
            .count()
    };
    // Live Preview hides the YAML and the markers.
    assert!(hidden_sections(&app) > 0);
    state(&mut app).set_display_mode(crate::editor::DisplayMode::Source);
    frames(&mut app, &ctx, 3);
    assert_eq!(hidden_sections(&app), 0);
    // Headings are not enlarged in Source mode.
    let (_, job) = &app.caches.highlight[&tab];
    let max = job
        .sections
        .iter()
        .map(|s| s.format.font_id.size)
        .fold(0.0, f32::max);
    assert!(max <= state(&mut app).settings.font_size + 0.01, "{max}");
    assert_eq!(
        text_of(&mut app, &NotePath::new("Src.md").expect("p")),
        text
    );
}

#[test]
fn live_preview_hides_the_frontmatter_when_the_caret_is_on_line_one() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("fm-line1");
    let text = "---\ntitle: T\ntags: [a]\n---\n# Titre\n\ntexte\n";
    // `open_at_line(.., 0, ..)` puts the caret at the very start (line 1).
    open_with_text(&mut app, &ctx, "Fm.md", text);
    frames(&mut app, &ctx, 4);
    let tab = state(&mut app).active_tab().expect("tab").id;
    let (_, job) = &app.caches.highlight[&tab];
    let yaml_end = text.find("# Titre").expect("heading");
    let visible_yaml = job
        .sections
        .iter()
        .filter(|s| usize::from(s.byte_range.start) < yaml_end)
        .filter(|s| s.format.font_id.size >= 1.0 && s.format.color != egui::Color32::TRANSPARENT)
        .count();
    assert_eq!(visible_yaml, 0, "the YAML must stay hidden: {job:?}");
}

/// A note with `n` sections of a few lines each.
fn long_note(n: usize) -> String {
    let mut t = String::new();
    for i in 0..n {
        t.push_str(&format!("## Section {i}\n\n"));
        for j in 0..6 {
            t.push_str(&format!("ligne {i}.{j}\n\n"));
        }
    }
    t
}

/// Screen position of the tick of heading `i` (of `n`) at rest.
fn nav_tick_pos(ctx: &egui::Context, tab: u64, n: usize, i: usize) -> egui::Pos2 {
    let zone = ctx
        .read_response(heading_nav::strip_id(tab))
        .expect("navigator drawn")
        .rect;
    let pitch = (zone.height() - 12.0) / n as f32;
    egui::pos2(zone.center().x, zone.top() + 6.0 + (i as f32 + 0.5) * pitch)
}

#[test]
fn clicking_a_heading_tick_scrolls_to_the_heading_and_moves_the_caret() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("nav-click");
    let text = long_note(20);
    let path = open_with_text(&mut app, &ctx, "Long.md", &text);
    let tab = state(&mut app).active_tab().expect("tab").id;
    frames(&mut app, &ctx, 3);
    assert_eq!(app.caches.nav.offsets.get(&tab).copied(), Some(0.0));
    let target = 12;
    let pos = nav_tick_pos(&ctx, tab, 20, target);
    let button = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    // Hover for a few frames so that the magnifier is fully open, then click.
    frame(&mut app, &ctx, vec![Event::PointerMoved(pos)]);
    frames(&mut app, &ctx, 12);
    frame(&mut app, &ctx, vec![button(true)]);
    frame(&mut app, &ctx, vec![button(false)]);
    frames(&mut app, &ctx, 30);
    let offset = app.caches.nav.offsets[&tab];
    assert!(offset > 200.0, "not scrolled: {offset}");
    // The caret is at the start of the heading.
    let at = text.find("## Section 12").expect("heading");
    let id = egui::Id::unique(("editor", tab));
    let sel = edit::read_sel(&ctx, id, &text).expect("selection");
    assert_eq!((sel.start, sel.end), (at, at));
    assert_eq!(text_of(&mut app, &path), text);
}

#[test]
fn hovering_the_ticks_opens_the_floating_outline() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("nav-hover");
    state(&mut app).settings.heading_navigator = crate::settings::HeadingNavigator::Floating;
    open_with_text(&mut app, &ctx, "Long.md", &long_note(8));
    let tab = state(&mut app).active_tab().expect("tab").id;
    frames(&mut app, &ctx, 3);
    assert!(!app.caches.nav.card_open);
    let pos = nav_tick_pos(&ctx, tab, 8, 3);
    frame(&mut app, &ctx, vec![Event::PointerMoved(pos)]);
    frames(&mut app, &ctx, 2);
    assert!(app.caches.nav.card_open, "card not opened");
    // Moving away closes it.
    frame(
        &mut app,
        &ctx,
        vec![Event::PointerMoved(egui::pos2(500.0, 400.0))],
    );
    frames(&mut app, &ctx, 2);
    assert!(!app.caches.nav.card_open, "card still open");
}

#[test]
fn heading_tick_click_also_scrolls_the_reading_view() {
    let ctx = egui::Context::default();
    let (mut app, _dir) = real_app("nav-reading");
    state(&mut app).settings.heading_navigator = crate::settings::HeadingNavigator::Floating;
    open_with_text(&mut app, &ctx, "Long.md", &long_note(20));
    state(&mut app).set_display_mode(crate::editor::DisplayMode::Reading);
    let tab = state(&mut app).active_tab().expect("tab").id;
    frames(&mut app, &ctx, 4);
    let pos = nav_tick_pos(&ctx, tab, 20, 15);
    let button = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(pos)]);
    frame(&mut app, &ctx, vec![button(true)]);
    frame(&mut app, &ctx, vec![button(false)]);
    frames(&mut app, &ctx, 30);
    assert!(app.caches.nav.offsets[&tab] > 200.0);
}

fn column_right(active: bool) -> f32 {
    let ctx = egui::Context::default();
    let mut right = 0.0;
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(800.0, 600.0),
        )),
        ..RawInput::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        editor::set_nav_reserve(active);
        editor::readable_column(ui, false, |ui| {
            right = ui.max_rect().right();
        });
    });
    out.textures_delta.clear();
    editor::set_nav_reserve(false);
    right
}

#[test]
fn the_column_keeps_a_right_margin_when_the_navigator_is_active() {
    let off = column_right(false);
    let on = column_right(true);
    assert!(
        off - on >= editor::NAV_RESERVE / 2.0,
        "off {off} on {on}: column must shrink away from the strip"
    );
    assert!(800.0 - on >= editor::NAV_RESERVE);
}

fn wheel(dy: f32) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos2(700.0, 400.0)),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn long_note_scrolls_in_reading_and_live_preview() {
    let ctx = egui::Context::default();
    let mut app = app();
    let path = open_empty(&mut app, &ctx);
    let text: String = (0..150)
        .map(|i| format!("## Part {i}\n\nSome text with a [[link]].\n\n- [ ] task {i}\n\n"))
        .collect();
    state(&mut app).buffers.get_mut(&path).expect("buffer").text = text;
    state(&mut app).buffer_edited(&path);
    for mode in [DisplayMode::Reading, DisplayMode::LivePreview] {
        state(&mut app).set_display_mode(mode);
        frames(&mut app, &ctx, 3);
        for _ in 0..80 {
            frame(&mut app, &ctx, wheel(-300.0));
        }
    }
    // Reading view: far-off blocks are skipped, the heading tracking still
    // follows the scroll.
    state(&mut app).set_display_mode(DisplayMode::Reading);
    frames(&mut app, &ctx, 3);
    for _ in 0..40 {
        frame(&mut app, &ctx, wheel(-300.0));
    }
    let line = state(&mut app)
        .views
        .current_line
        .as_ref()
        .map_or(0, |(_, l)| *l);
    assert!(line > 100, "current line {line}");
}

/// One frame like `frame`, returning the texts drawn with their rectangles.
fn frame_texts(app: &mut ObsiApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    fn collect(shape: &egui::epaint::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::epaint::Shape::Text(t) => {
                out.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
            egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..RawInput::default()
    };
    let out = ctx.run_ui(input, |ui| app.workspace(ui));
    let mut texts = Vec::new();
    for s in &out.shapes {
        collect(&s.shape, &mut texts);
    }
    texts
}

fn click_pos(app: &mut ObsiApp, ctx: &egui::Context, pos: egui::Pos2) {
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

#[test]
fn clicking_the_cross_of_a_tab_closes_only_that_tab() {
    let ctx = egui::Context::default();
    let mut app = app();
    let a = NotePath::new("Orpheline.md").expect("path");
    let b = NotePath::new("Rust notes.md").expect("path");
    state(&mut app).open(&a, OpenIn::CurrentTab);
    state(&mut app).open(&b, OpenIn::NewTab);
    frames(&mut app, &ctx, 3);
    let before = state(&mut app).tabs.len();
    assert!(before >= 2);
    let active_before = state(&mut app).active_note().cloned();
    let crosses: Vec<Rect> = frame_texts(&mut app, &ctx, Vec::new())
        .into_iter()
        .filter(|(t, _)| t == "×")
        .map(|(_, r)| r)
        .collect();
    assert_eq!(crosses.len(), before);
    // Close the first (inactive) tab: it must not be activated on the way.
    click_pos(&mut app, &ctx, crosses[0].center());
    let s = state(&mut app);
    assert_eq!(s.tabs.len(), before - 1);
    assert!(s.tabs.iter().all(|t| t.note().is_none_or(|n| n.path != a)));
    assert_eq!(s.active_note().cloned(), active_before);
}

#[test]
fn right_sidebar_collapses_from_its_header_and_reopens_from_the_tab_bar() {
    let ctx = egui::Context::default();
    let mut app = app();
    state(&mut app).settings.right_open = true;
    let note = NotePath::new("Orpheline.md").expect("path");
    state(&mut app).open(&note, OpenIn::CurrentTab);
    frames(&mut app, &ctx, 3);
    // The right-most sidebar glyph is the collapse button of the sidebar.
    let toggle = |texts: Vec<(String, Rect)>| {
        texts
            .into_iter()
            .filter(|(t, r)| t == icons::SIDEBAR && r.center().x > 100.0)
            .map(|(_, r)| r)
            .max_by(|a, b| a.center().x.total_cmp(&b.center().x))
    };
    let r = toggle(frame_texts(&mut app, &ctx, Vec::new())).expect("collapse button");
    click_pos(&mut app, &ctx, r.center());
    assert!(!state(&mut app).settings.right_open);
    let r = toggle(frame_texts(&mut app, &ctx, Vec::new())).expect("tab bar toggle");
    click_pos(&mut app, &ctx, r.center());
    assert!(state(&mut app).settings.right_open);
}
