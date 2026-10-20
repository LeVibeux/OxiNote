mod common;

use std::fs;

use common::{empty, p, sample};
use ors_core::{Error, VaultRead, VaultWrite};
use ors_vault::{AppConfig, Date, TrashMode, Vault};

#[test]
fn open_requires_existing_dir_and_creates_config() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(matches!(
        Vault::open(tmp.path().join("missing")),
        Err(Error::InvalidPath(_))
    ));
    let file = tmp.path().join("file.txt");
    fs::write(&file, "x").unwrap();
    assert!(matches!(Vault::open(&file), Err(Error::InvalidPath(_))));

    let (_t, v) = empty();
    assert!(v.root().join(".obsidian").is_dir());
    assert_eq!(v.name(), "vault");
    assert!(v.root().is_absolute());
}

#[test]
fn list_files_of_sample_vault() {
    let (_t, v) = sample();
    // Hidden stuff that must be skipped.
    fs::create_dir_all(v.root().join(".git/objects")).unwrap();
    fs::write(v.root().join(".git/HEAD"), "ref").unwrap();
    fs::write(v.root().join(".DS_Store"), "").unwrap();
    fs::create_dir_all(v.root().join(".trash")).unwrap();
    fs::write(v.root().join(".trash/old.md"), "").unwrap();
    fs::create_dir_all(v.root().join("Projets/.hidden")).unwrap();
    fs::write(v.root().join("Projets/.hidden/x.md"), "").unwrap();

    let files: Vec<String> = v
        .list_files()
        .unwrap()
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        files,
        vec![
            "Accueil.md",
            "Idées.md",
            "Journal/2026-10-20.md",
            "Orpheline.md",
            "Projets/ObsiRust.md",
            "Projets/Rust notes.md",
            "Templates/Daily.md",
            "attachments/diagramme.png",
        ]
    );
    let notes = v.list_notes().unwrap();
    assert_eq!(notes.len(), 7);
    assert_eq!(
        v.list_folders().unwrap(),
        vec!["Journal", "Projets", "Templates", "attachments"]
    );
}

#[test]
fn read_metadata_exists() {
    let (_t, v) = sample();
    let note = p("Projets/Rust notes.md");
    assert!(v.exists(&note));
    assert!(!v.exists(&p("Projets"))); // folders are not files
    assert!(v.folder_exists("Projets"));
    let text = v.read_to_string(&note).unwrap();
    let meta = v.metadata(&note).unwrap();
    assert_eq!(meta.size, text.len() as u64);
    assert_eq!(v.read_bytes(&note).unwrap(), text.as_bytes());
    assert!(matches!(
        v.read_to_string(&p("nope.md")),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(v.metadata(&p("nope.md")), Err(Error::NotFound(_))));
    // Invalid UTF-8 is a parse error, bytes still readable.
    fs::write(v.root().join("bin.md"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(
        v.read_to_string(&p("bin.md")),
        Err(Error::Parse(_))
    ));
    assert_eq!(v.read_bytes(&p("bin.md")).unwrap().len(), 3);
}

#[test]
fn write_is_atomic_and_creates_parents() {
    let (_t, v) = empty();
    let note = p("a/b/c.md");
    v.write(&note, "hello").unwrap();
    assert_eq!(v.read_to_string(&note).unwrap(), "hello");
    v.write(&note, "world").unwrap();
    assert_eq!(v.read_to_string(&note).unwrap(), "world");
    // No temp files left behind.
    let leftovers: Vec<_> = fs::read_dir(v.root().join("a/b"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["c.md".to_string()]);
    v.write_bytes(&p("img/x.png"), &[1, 2, 3]).unwrap();
    assert_eq!(v.read_bytes(&p("img/x.png")).unwrap(), vec![1, 2, 3]);
    // Writing over a folder fails.
    assert!(v.write(&p("a"), "x").is_err());
}

#[test]
fn rename_files() {
    let (_t, v) = empty();
    v.write(&p("a.md"), "A").unwrap();
    v.write(&p("b.md"), "B").unwrap();
    assert!(matches!(
        v.rename(&p("a.md"), &p("b.md")),
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        v.rename(&p("zzz.md"), &p("c.md")),
        Err(Error::NotFound(_))
    ));
    v.rename(&p("a.md"), &p("sub/dir/a2.md")).unwrap();
    assert!(!v.exists(&p("a.md")));
    assert_eq!(v.read_to_string(&p("sub/dir/a2.md")).unwrap(), "A");
    // Case-only rename.
    v.rename(&p("b.md"), &p("B.md")).unwrap();
    let names: Vec<String> = v
        .list_files()
        .unwrap()
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(names, vec!["B.md", "sub/dir/a2.md"]);
    assert_eq!(v.read_to_string(&p("B.md")).unwrap(), "B");
}

#[test]
fn create_note_untitled_numbering() {
    let (_t, v) = empty();
    let a = v.create_note("", "", "").unwrap();
    let b = v.create_note("", "Untitled", "").unwrap();
    let c = v.create_note("", "Untitled.md", "body").unwrap();
    assert_eq!(a.as_str(), "Untitled.md");
    assert_eq!(b.as_str(), "Untitled 1.md");
    assert_eq!(c.as_str(), "Untitled 2.md");
    assert_eq!(v.read_to_string(&c).unwrap(), "body");
    let d = v.create_note("New/Folder", "What: is [this]?", "").unwrap();
    assert_eq!(d.as_str(), "New/Folder/What is this.md");
    // Case-insensitive collision check, like Obsidian.
    let e = v.create_note("New/Folder", "what is THIS", "").unwrap();
    assert_eq!(e.as_str(), "New/Folder/what is THIS 1.md");
}

#[test]
fn create_untitled_note_honors_new_file_location() {
    let (_t, v) = sample();
    // sample app.json: newFileLocation = folder, newFileFolderPath = Inbox
    let n = v.create_untitled_note(Some("Projets"), "").unwrap();
    assert_eq!(n.as_str(), "Inbox/Untitled.md");
    assert_eq!(v.new_note_folder(None).unwrap(), "Inbox");
    let mut cfg = v.app_config().unwrap();
    cfg.new_file_location = Some("current".into());
    v.save_app_config(&cfg).unwrap();
    assert_eq!(v.new_note_folder(Some("Projets")).unwrap(), "Projets");
}

#[test]
fn folders() {
    let (_t, v) = empty();
    v.create_folder("x/y").unwrap();
    v.create_folder("x/y").unwrap(); // idempotent
    assert_eq!(v.list_folders().unwrap(), vec!["x", "x/y"]);
    v.write(&p("x/y/n.md"), "n").unwrap();
    v.write(&p("file"), "f").unwrap();
    assert!(matches!(
        v.create_folder("file"),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(v.create_untitled_folder("").unwrap(), "Untitled");
    assert_eq!(v.create_untitled_folder("").unwrap(), "Untitled 1");

    v.rename_folder("x", "z/w").unwrap();
    assert_eq!(v.read_to_string(&p("z/w/y/n.md")).unwrap(), "n");
    assert!(!v.folder_exists("x"));
    assert!(matches!(
        v.rename_folder("z", "Untitled"),
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        v.rename_folder("z", "z/inner"),
        Err(Error::InvalidPath(_))
    ));
    assert!(matches!(
        v.rename_folder("missing", "q"),
        Err(Error::NotFound(_))
    ));
    v.rename_folder("z", "Z").unwrap(); // case-only
    assert!(v.list_folders().unwrap().contains(&"Z".to_string()));

    v.delete_folder_with("Z", TrashMode::Permanent).unwrap();
    assert!(!v.folder_exists("Z"));
    v.delete_folder_with("Untitled", TrashMode::Local).unwrap();
    assert!(v.root().join(".trash/Untitled").is_dir());
    assert!(v.delete_folder_with("", TrashMode::Permanent).is_err());
}

#[test]
fn delete_modes() {
    let (_t, v) = empty();
    for name in ["a.md", "b.md", "c.md"] {
        v.write(&p(name), name).unwrap();
    }
    v.delete_with(&p("a.md"), TrashMode::Permanent).unwrap();
    assert!(!v.exists(&p("a.md")));
    assert!(!v.root().join(".trash").exists());

    v.delete_with(&p("b.md"), TrashMode::Local).unwrap();
    v.write(&p("b.md"), "again").unwrap();
    v.delete_with(&p("b.md"), TrashMode::Local).unwrap();
    assert_eq!(
        fs::read_to_string(v.root().join(".trash/b.md")).unwrap(),
        "b.md"
    );
    assert_eq!(
        fs::read_to_string(v.root().join(".trash/b 1.md")).unwrap(),
        "again"
    );
    // Trash content is not listed.
    assert_eq!(v.list_files().unwrap(), vec![p("c.md")]);

    // `delete` uses app.json; default (no trashOption) is local.
    v.delete(&p("c.md")).unwrap();
    assert!(v.root().join(".trash/c.md").exists());
    assert!(matches!(
        v.delete_with(&p("c.md"), TrashMode::Permanent),
        Err(Error::NotFound(_))
    ));

    // `trashOption: none` → permanent.
    let cfg = AppConfig {
        trash_mode: TrashMode::Permanent,
        ..AppConfig::default()
    };
    v.save_app_config(&cfg).unwrap();
    v.write(&p("d.md"), "d").unwrap();
    v.delete(&p("d.md")).unwrap();
    assert!(!v.root().join(".trash/d.md").exists());
}

#[test]
fn path_traversal_is_rejected() {
    let (tmp, v) = empty();
    let outside = tmp.path().join("secret.md");
    fs::write(&outside, "secret").unwrap();

    // `..` cannot even be expressed as a NotePath.
    assert!(ors_core::NotePath::new("../secret.md").is_err());
    // Folder arguments are validated too.
    assert!(v.create_folder("../evil").is_err());
    assert!(v.create_note("../", "x", "").is_err());
    assert!(v.rename_folder("..", "x").is_err());
    assert!(v.list_files().unwrap().is_empty());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // A symlink pointing outside the vault is neither listed nor readable
        // nor writable through.
        symlink(&outside, v.root().join("link.md")).unwrap();
        symlink(tmp.path(), v.root().join("outdir")).unwrap();
        assert!(v.list_files().unwrap().is_empty());
        assert!(matches!(
            v.read_to_string(&p("link.md")),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            v.read_to_string(&p("outdir/secret.md")),
            Err(Error::InvalidPath(_))
        ));
        assert!(v.write(&p("outdir/new.md"), "x").is_err());
        assert!(!tmp.path().join("new.md").exists());
        assert!(!v.exists(&p("link.md")));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "secret");
    }
}

#[cfg(unix)]
#[test]
fn symlinks_inside_vault_and_loops() {
    use std::os::unix::fs::symlink;
    let (_t, v) = empty();
    v.write(&p("real/n.md"), "n").unwrap();
    // Loop: real/loop -> real
    symlink(v.root().join("real"), v.root().join("real/loop")).unwrap();
    // In-vault file symlink is fine.
    symlink(v.root().join("real/n.md"), v.root().join("alias.md")).unwrap();
    let files = v.list_files().unwrap();
    assert!(files.contains(&p("real/n.md")));
    assert!(files.contains(&p("alias.md")));
    assert!(!files.iter().any(|f| f.as_str().contains("loop/loop")));
    assert_eq!(v.read_to_string(&p("alias.md")).unwrap(), "n");
}

#[test]
fn sample_config_parsing() {
    let (_t, v) = sample();
    let app = v.app_config().unwrap();
    assert_eq!(app.new_file_location.as_deref(), Some("folder"));
    assert_eq!(app.new_file_folder_path.as_deref(), Some("Inbox"));
    assert_eq!(app.attachment_folder_path.as_deref(), Some("attachments"));
    assert!(!app.use_markdown_links);
    assert_eq!(app.new_link_format.as_deref(), Some("shortest"));
    assert!(app.always_update_links);
    assert_eq!(app.trash_mode, TrashMode::Local);
    assert_eq!(
        app.extra.get("showLineNumber"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(app.extra.len(), 1);

    let daily = v.daily_notes_config().unwrap();
    assert_eq!(daily.folder, "Journal");
    assert_eq!(daily.format, "YYYY-MM-DD");
    assert_eq!(daily.template.as_deref(), Some("Templates/Daily"));

    let t = v.templates_config().unwrap();
    assert_eq!(t.folder.as_deref(), Some("Templates"));
    assert_eq!(t.date_format.as_deref(), Some("YYYY-MM-DD"));
    assert_eq!(t.time_format.as_deref(), Some("HH:mm"));

    assert_eq!(v.list_templates().unwrap(), vec![p("Templates/Daily.md")]);
    let note = p("Projets/ObsiRust.md");
    assert_eq!(v.attachment_folder_for(Some(&note)).unwrap(), "attachments");
    assert_eq!(
        v.available_attachment_path(Some(&note), "diagramme.png")
            .unwrap()
            .as_str(),
        "attachments/diagramme 1.png"
    );
    assert_eq!(
        v.available_attachment_path(None, "new.png")
            .unwrap()
            .as_str(),
        "attachments/new.png"
    );
}

#[test]
fn app_config_roundtrip_preserves_file() {
    let (_t, v) = sample();
    let path = v.root().join(".obsidian/app.json");
    let before: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let cfg = v.app_config().unwrap();
    v.save_app_config(&cfg).unwrap();
    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(before, after);

    // Modify a known key; unknown keys survive.
    let mut cfg = cfg;
    cfg.use_markdown_links = true;
    cfg.trash_mode = TrashMode::System;
    cfg.user_ignore_filters = vec!["Archive/".into()];
    v.save_app_config(&cfg).unwrap();
    let reread = v.app_config().unwrap();
    assert_eq!(reread, cfg);
    let raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["showLineNumber"], serde_json::Value::Bool(true));
    assert_eq!(raw["trashOption"], "system");
    assert_eq!(raw["useMarkdownLinks"], true);
}

#[test]
fn other_configs_roundtrip_and_invalid_files() {
    let (_t, v) = sample();
    let daily_path = v.root().join(".obsidian/daily-notes.json");
    fs::write(
        &daily_path,
        r#"{ "folder": "Journal", "format": "YYYY-MM-DD", "template": "Templates/Daily", "autorun": true }"#,
    )
    .unwrap();
    let mut d = v.daily_notes_config().unwrap();
    d.format = "YYYY/MM/DD".into();
    v.save_daily_notes_config(&d).unwrap();
    assert_eq!(v.daily_notes_config().unwrap(), d);
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&daily_path).unwrap()).unwrap();
    assert_eq!(raw["autorun"], true);

    let mut t = v.templates_config().unwrap();
    t.time_format = Some("HH:mm:ss".into());
    v.save_templates_config(&t).unwrap();
    assert_eq!(v.templates_config().unwrap(), t);

    // Garbage → defaults, never an error or panic.
    fs::write(v.root().join(".obsidian/app.json"), "{ nope").unwrap();
    fs::write(&daily_path, [0xff, 0x00]).unwrap();
    fs::write(v.root().join(".obsidian/templates.json"), "42").unwrap();
    assert_eq!(v.app_config().unwrap(), AppConfig::default());
    assert_eq!(v.daily_notes_config().unwrap(), Default::default());
    assert_eq!(v.templates_config().unwrap(), Default::default());
    // Missing files too.
    let (_t2, e) = empty();
    assert_eq!(e.app_config().unwrap(), AppConfig::default());
    assert!(e.list_templates().unwrap().is_empty());
}

#[test]
fn daily_note_from_template() {
    let (_t, v) = sample();
    // Existing note is returned untouched.
    let existing = v.open_daily_note(Date::new(2026, 10, 20).unwrap()).unwrap();
    assert_eq!(existing.as_str(), "Journal/2026-10-20.md");
    assert!(
        v.read_to_string(&existing)
            .unwrap()
            .starts_with("# 2026-10-20\n")
    );

    let date = Date::new(2026, 3, 1).unwrap();
    let path = v.open_daily_note(date).unwrap();
    assert_eq!(path.as_str(), "Journal/2026-03-01.md");
    let text = v.read_to_string(&path).unwrap();
    assert!(text.starts_with("# 2026-03-01\n"), "{text}");
    assert!(
        text.contains("Créé le 2026-03-01 à "),
        "date placeholder: {text}"
    );
    assert!(text.contains("(Sunday 1 March 2026)"), "{text}");
    assert!(!text.contains("{{"), "{text}");
    // Opening again does not overwrite.
    v.write(&path, "edited").unwrap();
    assert_eq!(v.open_daily_note(date).unwrap(), path);
    assert_eq!(v.read_to_string(&path).unwrap(), "edited");
}

#[test]
fn daily_note_subfolders_and_missing_template() {
    let (_t, v) = empty();
    let mut d = v.daily_notes_config().unwrap();
    d.folder = "Daily".into();
    d.format = "YYYY/MM-MMMM/YYYY-MM-DD".into();
    d.template = Some("Nope/Missing".into());
    v.save_daily_notes_config(&d).unwrap();
    let date = Date::new(2026, 12, 25).unwrap();
    assert_eq!(
        v.daily_note_path(date).unwrap().as_str(),
        "Daily/2026/12-December/2026-12-25.md"
    );
    let path = v.open_daily_note(date).unwrap();
    assert_eq!(v.read_to_string(&path).unwrap(), "");
    let today = v.open_today_note().unwrap();
    assert!(v.exists(&today));
}

#[test]
fn render_template_uses_templates_config() {
    let (_t, v) = sample();
    let out = v
        .render_template(&p("Templates/Daily.md"), "Ma note")
        .unwrap();
    assert!(out.starts_with("# Ma note\n"));
    assert!(!out.contains("{{"));
}
