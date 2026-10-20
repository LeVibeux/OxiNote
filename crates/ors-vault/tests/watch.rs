mod common;

use std::fs;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use common::{empty, p};
use ors_core::{VaultEvent, VaultWrite};

const TIMEOUT: Duration = Duration::from_secs(15);

/// Collect events until `pred` matches one of them (returns all collected
/// events), or panic after a generous timeout.
fn wait_for(
    rx: &Receiver<VaultEvent>,
    what: &str,
    pred: impl Fn(&VaultEvent) -> bool,
) -> Vec<VaultEvent> {
    let deadline = Instant::now() + TIMEOUT;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            panic!("timed out waiting for {what}; got {seen:?}");
        }
        if let Ok(ev) = rx.recv_timeout(left.min(Duration::from_millis(200))) {
            let hit = pred(&ev);
            seen.push(ev);
            if hit {
                return seen;
            }
        }
    }
}

/// Drain whatever arrives within `d`.
fn drain(rx: &Receiver<VaultEvent>, d: Duration) -> Vec<VaultEvent> {
    let deadline = Instant::now() + d;
    let mut out = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(ev) => out.push(ev),
            Err(_) => break,
        }
    }
    out
}

#[test]
fn watcher_reports_external_changes() {
    let (_t, v) = empty();
    fs::create_dir_all(v.root().join("dir")).unwrap();
    fs::write(v.root().join("dir/old.md"), "x").unwrap();
    let (_w, rx) = v.watch().unwrap();
    // Give the backend a moment to install its watches.
    std::thread::sleep(Duration::from_millis(300));

    fs::write(v.root().join("a.md"), "hello").unwrap();
    wait_for(&rx, "Created(a.md)", |e| {
        *e == VaultEvent::Created(p("a.md"))
    });
    drain(&rx, Duration::from_millis(600));

    fs::write(v.root().join("a.md"), "hello again").unwrap();
    wait_for(&rx, "Modified(a.md)", |e| {
        *e == VaultEvent::Modified(p("a.md"))
    });
    drain(&rx, Duration::from_millis(600));

    fs::rename(v.root().join("a.md"), v.root().join("b.md")).unwrap();
    let evs = wait_for(&rx, "rename a.md -> b.md", |e| {
        matches!(e, VaultEvent::Renamed { to, .. } if *to == p("b.md"))
            || *e == VaultEvent::Created(p("b.md"))
    });
    match evs.last() {
        Some(VaultEvent::Renamed { from, .. }) => assert_eq!(*from, p("a.md")),
        // inotify reports both sides, so Linux must give a real rename.
        other if cfg!(target_os = "linux") => panic!("expected Renamed, got {other:?} in {evs:?}"),
        _ => {}
    }
    drain(&rx, Duration::from_millis(600));

    // Folder rename → per-file events.
    fs::rename(v.root().join("dir"), v.root().join("dir2")).unwrap();
    let evs = wait_for(&rx, "dir/old.md moved", |e| {
        matches!(e, VaultEvent::Renamed { to, .. } if *to == p("dir2/old.md"))
            || *e == VaultEvent::Created(p("dir2/old.md"))
    });
    if cfg!(target_os = "linux") {
        assert!(
            evs.contains(&VaultEvent::Renamed {
                from: p("dir/old.md"),
                to: p("dir2/old.md")
            }),
            "{evs:?}"
        );
    }
    drain(&rx, Duration::from_millis(600));

    fs::remove_file(v.root().join("b.md")).unwrap();
    wait_for(&rx, "Deleted(b.md)", |e| {
        *e == VaultEvent::Deleted(p("b.md"))
    });
    drain(&rx, Duration::from_millis(600));

    // Folder deletion → Deleted for each known file.
    fs::remove_dir_all(v.root().join("dir2")).unwrap();
    wait_for(&rx, "Deleted(dir2/old.md)", |e| {
        *e == VaultEvent::Deleted(p("dir2/old.md"))
    });
}

#[test]
fn watcher_filters_hidden_and_reports_own_writes() {
    let (_t, v) = empty();
    let (_w, rx) = v.watch().unwrap();
    std::thread::sleep(Duration::from_millis(300));

    fs::write(v.root().join(".obsidian/workspace.json"), "{}").unwrap();
    fs::create_dir_all(v.root().join(".git")).unwrap();
    fs::write(v.root().join(".git/HEAD"), "x").unwrap();
    fs::write(v.root().join(".hidden.md"), "x").unwrap();
    // A visible marker so we know the hidden events had their chance.
    v.write(&p("visible.md"), "x").unwrap();
    let evs = wait_for(&rx, "visible.md", |e| {
        *e == VaultEvent::Created(p("visible.md"))
    });
    let mut all = evs;
    all.extend(drain(&rx, Duration::from_millis(800)));
    for e in &all {
        let path = match e {
            VaultEvent::Created(p) | VaultEvent::Modified(p) | VaultEvent::Deleted(p) => p,
            VaultEvent::Renamed { to, .. } => to,
        };
        assert_eq!(path.as_str(), "visible.md", "unexpected event {e:?}");
    }

    // Moving to local trash is a deletion.
    v.delete_with(&p("visible.md"), ors_vault::TrashMode::Local)
        .unwrap();
    wait_for(&rx, "Deleted(visible.md)", |e| {
        *e == VaultEvent::Deleted(p("visible.md"))
    });
}

#[test]
fn dropping_watcher_closes_channel() {
    let (_t, v) = empty();
    let (w, rx) = v.watch().unwrap();
    drop(w);
    // The sender lives in the debouncer thread's handler; once dropped the
    // channel disconnects.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            _ if Instant::now() > deadline => panic!("channel not closed"),
            _ => {}
        }
    }
}
