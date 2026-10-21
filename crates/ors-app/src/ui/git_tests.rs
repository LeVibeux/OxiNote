//! Headless UI tests of the Git backup (real repositories in temporary
//! folders). Skipped when `git` is not installed.

use std::time::Duration;

use eframe::egui::{self, Event, Modifiers, RawInput, Rect, epaint::Shape, pos2};

use super::*;
use crate::git_backup::{self, BackupOutcome};
use crate::settings::RightPane;
use crate::workbench::SettingsPage;

fn git_available() -> bool {
    git_backup::git_version().is_ok()
}

fn app_on(dir: &std::path::Path) -> ObsiApp {
    let settings = Settings {
        autosave_delay: 60.0,
        ..Settings::default()
    };
    let vault = DiskVault::open(dir).expect("open vault");
    let state = AppState::new(Box::new(vault), real_index, settings.clone());
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

fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
        Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

/// Run one frame; returns the texts drawn with their rectangles.
fn frame(app: &mut ObsiApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
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
    let mut texts = Vec::new();
    for s in &out.shapes {
        collect(&s.shape, &mut texts);
    }
    texts
}

/// Draw frames (letting the background git jobs run) until `done`.
fn wait_for(
    app: &mut ObsiApp,
    ctx: &egui::Context,
    what: &str,
    done: impl Fn(&AppState) -> bool,
) -> Vec<(String, Rect)> {
    let mut texts = Vec::new();
    for _ in 0..200 {
        texts = frame(app, ctx, Vec::new());
        if done(app.state.as_ref().expect("vault open")) {
            // One more frame to draw the result.
            return frame(app, ctx, Vec::new());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = texts;
    panic!("timeout waiting for {what}");
}

fn click_at(app: &mut ObsiApp, ctx: &egui::Context, pos: egui::Pos2) {
    let button = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(app, ctx, vec![Event::PointerMoved(pos)]);
    frame(app, ctx, vec![button(true)]);
    frame(app, ctx, vec![button(false)]);
}

fn find(texts: &[(String, Rect)], needle: &str) -> Option<Rect> {
    texts
        .iter()
        .find(|(t, _)| t.contains(needle))
        .map(|(_, r)| *r)
}

fn vault_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.md"), "# A\nv1\n").expect("write");
    std::fs::write(dir.path().join("b.md"), "# B\n").expect("write");
    dir
}

#[test]
fn settings_page_init_button_creates_the_repository() {
    if !git_available() {
        eprintln!("git absent: test ignoré");
        return;
    }
    let dir = vault_dir();
    let mut app = app_on(dir.path());
    let ctx = egui::Context::default();
    app.state
        .as_mut()
        .expect("vault")
        .open_settings(SettingsPage::GitBackup);
    wait_for(&mut app, &ctx, "repository detection", |s| s.git.checked);
    assert!(!dir.path().join(".git").exists());
    let texts = frame(&mut app, &ctx, Vec::new());
    assert!(
        find(&texts, "Sauvegarde (Git)").is_some(),
        "page title: {texts:?}"
    );
    let button = find(&texts, "Initialiser le dépôt Git du vault").expect("init button drawn");
    click_at(&mut app, &ctx, button.center());
    wait_for(&mut app, &ctx, "git init", |s| s.git.repo.is_some());
    assert!(dir.path().join(".git").exists());
    assert!(dir.path().join(".gitignore").exists());
    let s = app.state.as_ref().expect("vault");
    assert!(s.git.settings.auto_backup, "enabled after the init");
    assert_eq!(s.git.settings.interval_minutes, 10);
    assert!(
        dir.path().join(".obsidian/obsirust-git.json").exists(),
        "per-vault settings saved"
    );
    // The page now shows the settings and the .gitignore presets.
    let texts = frame(&mut app, &ctx, Vec::new());
    assert!(find(&texts, "Sauvegarde automatique").is_some());
    assert!(find(&texts, "Ignorer la corbeille").is_some());
    assert_eq!(
        git_backup::status_count(dir.path()).expect("status"),
        0,
        "first commit done"
    );
}

#[test]
fn history_panel_lists_commits_and_shows_a_diff() {
    if !git_available() {
        eprintln!("git absent: test ignoré");
        return;
    }
    let dir = vault_dir();
    git_backup::init_repo(dir.path()).expect("init");
    let info = git_backup::detect_repo(dir.path())
        .expect("detect")
        .expect("repo");
    std::fs::write(dir.path().join("a.md"), "# A\nv2\n").expect("write");
    assert!(matches!(
        git_backup::backup(dir.path(), &info, "second change"),
        Ok(BackupOutcome::Committed { .. })
    ));
    std::fs::write(dir.path().join("a.md"), "# A\nv3\n").expect("write");
    git_backup::backup(dir.path(), &info, "third change").expect("commit");
    // An edit that is not committed yet.
    std::fs::write(dir.path().join("a.md"), "# A\nv4\n").expect("write");

    let mut app = app_on(dir.path());
    let ctx = egui::Context::default();
    {
        let s = app.state.as_mut().expect("vault");
        let path = ors_core::NotePath::new("a.md").expect("path");
        s.open(&path, OpenIn::CurrentTab);
        s.settings.right_open = true;
        s.settings.right_pane = RightPane::History;
    }
    let texts = wait_for(&mut app, &ctx, "history and diff", |s| {
        s.git.history.entries.len() == 3 && s.git.history.version.is_some()
    });
    assert!(find(&texts, "third change").is_some(), "{texts:?}");
    assert!(find(&texts, "second change").is_some());
    assert!(find(&texts, "Initial backup").is_some());
    assert!(find(&texts, "Restaurer cette version").is_some());
    // The newest commit is selected: its diff with the current text.
    assert!(find(&texts, "- v3").is_some(), "{texts:?}");
    assert!(find(&texts, "+ v4").is_some(), "{texts:?}");

    // Select the oldest commit: diff with its predecessor (empty file).
    let s = app.state.as_mut().expect("vault");
    s.git.history.against_previous = true;
    s.git_select_commit(2);
    let texts = wait_for(&mut app, &ctx, "old version", |s| {
        s.git.history.version.is_some()
    });
    assert!(find(&texts, "+ # A").is_some(), "{texts:?}");
}

#[test]
fn restoring_a_version_replaces_the_buffer() {
    if !git_available() {
        eprintln!("git absent: test ignoré");
        return;
    }
    let dir = vault_dir();
    git_backup::init_repo(dir.path()).expect("init");
    let mut app = app_on(dir.path());
    let ctx = egui::Context::default();
    let path = ors_core::NotePath::new("a.md").expect("path");
    {
        let s = app.state.as_mut().expect("vault");
        s.open(&path, OpenIn::CurrentTab);
        s.buffers.get_mut(&path).expect("buffer").text = "# A\nedited\n".into();
        s.git_restore_text(&path, "# A\nv1\n".into());
        assert_eq!(s.buffers[&path].text, "# A\nv1\n");
        assert!(!s.buffers[&path].is_dirty(), "same as the saved file");
    }
    frame(&mut app, &ctx, Vec::new());
}

#[test]
fn restoring_a_version_only_touches_the_history_note() {
    let dir = vault_dir();
    let mut app = app_on(dir.path());
    let a = ors_core::NotePath::new("a.md").expect("path");
    let b = ors_core::NotePath::new("b.md").expect("path");
    let s = app.state.as_mut().expect("vault");
    s.open(&a, OpenIn::CurrentTab);
    s.open(&b, OpenIn::CurrentTab);
    assert_eq!(s.active_note(), Some(&b), "B is the active note");
    s.git_restore_text(&a, "# A\nold\n".into());
    assert_eq!(s.buffers[&a].text, "# A\nold\n");
    assert_eq!(s.buffers[&b].text, "# B\n", "B untouched");
}

#[test]
fn explorer_dims_ignored_files() {
    if !git_available() {
        eprintln!("git absent: test ignoré");
        return;
    }
    let dir = vault_dir();
    std::fs::write(dir.path().join("big.pdf"), "x").expect("write");
    std::fs::write(dir.path().join(".gitignore"), "*.pdf\n").expect("write");
    git_backup::init_repo(dir.path()).expect("init");
    let mut app = app_on(dir.path());
    let ctx = egui::Context::default();
    wait_for(&mut app, &ctx, "ignored files", |s| {
        s.git.ignored.contains("big.pdf")
    });
    let s = app.state.as_ref().expect("vault");
    assert!(!s.git.ignored.contains("a.md"));
}

/// Texts drawn in the status bar (bottom strip) of a 1000x700 window.
fn status_texts(app: &mut ObsiApp, ctx: &egui::Context) -> Vec<(String, Rect)> {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(1000.0, 700.0),
        )),
        ..RawInput::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.workspace(ui));
    out.textures_delta.clear();
    let mut texts = Vec::new();
    for s in &out.shapes {
        collect(&s.shape, &mut texts);
    }
    texts.retain(|(_, r)| r.center().y > 675.0);
    texts
}

fn open_a(app: &mut ObsiApp) {
    let s = app.state.as_mut().expect("vault");
    let path = ors_core::NotePath::new("a.md").expect("path");
    s.open(&path, OpenIn::CurrentTab);
}

#[test]
fn status_bar_git_item_does_not_overlap_and_is_hidden_without_a_repo() {
    if !git_available() {
        eprintln!("git absent: test ignoré");
        return;
    }
    // Not a repository: nothing about Git in the status bar.
    let dir = vault_dir();
    let mut app = app_on(dir.path());
    let ctx = egui::Context::default();
    open_a(&mut app);
    wait_for(&mut app, &ctx, "repository detection", |s| s.git.checked);
    let texts = status_texts(&mut app, &ctx);
    assert!(find(&texts, "words").is_some(), "{texts:?}");
    for t in ["Git", "modification", "Sauvegard"] {
        assert!(find(&texts, t).is_none(), "{t} in {texts:?}");
    }

    // A repository with an uncommitted edit: the item shows and nothing
    // overlaps at 1000 px.
    let dir = vault_dir();
    git_backup::init_repo(dir.path()).expect("init");
    std::fs::write(dir.path().join("a.md"), "# A\nv2\n").expect("write");
    let mut app = app_on(dir.path());
    let ctx = egui::Context::default();
    open_a(&mut app);
    wait_for(&mut app, &ctx, "pending changes", |s| {
        s.git.changes.is_some_and(|n| n > 0)
    });
    let texts = status_texts(&mut app, &ctx);
    assert!(find(&texts, "modification").is_some(), "{texts:?}");
    assert!(find(&texts, "words").is_some(), "{texts:?}");
    assert!(find(&texts, "Live Preview").is_some(), "{texts:?}");
    for (i, (a, ra)) in texts.iter().enumerate() {
        for (b, rb) in texts.iter().skip(i + 1) {
            assert!(
                !ra.shrink(0.5).intersects(rb.shrink(0.5)),
                "{a:?} {ra:?} overlaps {b:?} {rb:?}"
            );
        }
    }
}
