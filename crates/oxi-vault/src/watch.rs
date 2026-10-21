//! Debounced filesystem watcher producing [`VaultEvent`]s.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use notify_debouncer_full::notify::event::{ModifyKind, RenameMode};
use notify_debouncer_full::notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use oxi_core::{Error, NotePath, Result, VaultEvent};

use crate::Vault;
use crate::fsutil::is_hidden_rel;

/// Debounce window: events for the same file within this delay are merged.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Keeps the file watcher alive. Dropping it stops the watcher and closes the
/// event channel.
///
/// Event semantics:
/// * paths are vault-relative files (never folders); hidden entries
///   (`.obsidian`, `.trash`, `.git`, dotfiles, our own temp files) are filtered;
/// * a folder created or moved into the vault yields `Created` for each file in
///   it; a folder deleted or moved out yields `Deleted` for each file the
///   watcher knew about; a folder renamed yields `Renamed` per file;
/// * renames are reported as `Renamed` when the backend reports both sides,
///   otherwise as `Deleted` + `Created`;
/// * moving a file to `.trash` / out of the vault is a `Deleted`;
/// * writes performed through [`Vault`] itself are reported too (atomic writes
///   show up as `Modified`, or `Created` for new files); callers must ignore
///   such echoes themselves;
/// * metadata-only changes (permissions, access times) are not reported.
pub struct VaultWatcher {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl fmt::Debug for VaultWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultWatcher").finish_non_exhaustive()
    }
}

/// Translates raw notify events into [`VaultEvent`]s, tracking known files so
/// folder-level events can be expanded to file-level ones.
struct Mapper {
    vault: Vault,
    known: BTreeSet<NotePath>,
    tx: Sender<VaultEvent>,
}

impl Mapper {
    fn send(&self, ev: VaultEvent) {
        // The receiver may be gone; nothing to do then.
        let _ = self.tx.send(ev);
    }

    /// Vault path of `abs` if it is inside the vault and not hidden.
    fn visible(&self, abs: &Path) -> Option<NotePath> {
        let rel = self.vault.rel_path(abs)?;
        (!is_hidden_rel(rel.as_str())).then_some(rel)
    }

    fn handle(&mut self, result: DebounceEventResult) {
        let Ok(events) = result else {
            return; // backend errors are not fatal; keep watching
        };
        for ev in events {
            let paths = &ev.event.paths;
            match ev.event.kind {
                EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_)) => {}
                EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                    paths.iter().for_each(|p| self.appeared(p));
                }
                EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                    paths.iter().for_each(|p| self.disappeared(p));
                }
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() == 2 => {
                    self.renamed(&paths[0], &paths[1]);
                }
                _ => {
                    for p in paths {
                        if p.exists() {
                            self.appeared(p);
                        } else {
                            self.disappeared(p);
                        }
                    }
                }
            }
        }
    }

    /// A file or folder exists at `abs` (created, modified or moved in).
    fn appeared(&mut self, abs: &Path) {
        let Some(rel) = self.visible(abs) else {
            return;
        };
        if abs.is_dir() {
            let Ok((files, _)) = self.vault.walk_from(abs, rel.as_str()) else {
                return;
            };
            let mut files = files;
            files.sort();
            for f in files {
                if self.known.insert(f.clone()) {
                    self.send(VaultEvent::Created(f));
                }
            }
        } else if abs.is_file() {
            if self.known.insert(rel.clone()) {
                self.send(VaultEvent::Created(rel));
            } else {
                self.send(VaultEvent::Modified(rel));
            }
        } else if !abs.exists() {
            self.disappeared(abs);
        }
    }

    /// Known files at or under `rel`.
    fn known_under(&self, rel: &NotePath) -> Vec<NotePath> {
        let prefix = format!("{rel}/");
        self.known
            .iter()
            .filter(|p| *p == rel || p.as_str().starts_with(&prefix))
            .cloned()
            .collect()
    }

    /// The file or folder at `abs` is gone.
    fn disappeared(&mut self, abs: &Path) {
        let Some(rel) = self.vault.rel_path(abs) else {
            return;
        };
        for p in self.known_under(&rel) {
            self.known.remove(&p);
            self.send(VaultEvent::Deleted(p));
        }
    }

    fn renamed(&mut self, from: &Path, to: &Path) {
        let from_rel = self.visible(from);
        let to_rel = self.visible(to);
        match (from_rel, to_rel) {
            (None, None) => {}
            (None, Some(_)) => self.appeared(to),
            (Some(_), None) => self.disappeared(from),
            (Some(from_rel), Some(to_rel)) => {
                if !to.exists() {
                    // Already moved again; later events will report the rest.
                    self.disappeared(from);
                    return;
                }
                let moved = self.known_under(&from_rel);
                if moved.is_empty() {
                    self.appeared(to);
                    return;
                }
                for old in moved {
                    let suffix = &old.as_str()[from_rel.as_str().len()..];
                    let Ok(new) = NotePath::new(format!("{to_rel}{suffix}")) else {
                        continue;
                    };
                    self.known.remove(&old);
                    if old == new {
                        continue;
                    }
                    self.known.insert(new.clone());
                    self.send(VaultEvent::Renamed { from: old, to: new });
                }
                // Files we did not know about inside a moved folder.
                if to.is_dir() {
                    self.appeared(to);
                }
            }
        }
    }
}

pub(crate) fn start(vault: &Vault) -> Result<(VaultWatcher, Receiver<VaultEvent>)> {
    let (tx, rx) = mpsc::channel();
    let snapshot = vault.clone();
    let known = snapshot.walk()?.0.into_iter().collect();
    let mut mapper = Mapper {
        vault: snapshot,
        known,
        tx,
    };
    let mut debouncer = new_debouncer(DEBOUNCE, None, move |res: DebounceEventResult| {
        mapper.handle(res)
    })
    .map_err(|e| Error::Other(format!("cannot start watcher: {e}")))?;
    debouncer
        .watch(&vault.root, RecursiveMode::Recursive)
        .map_err(|e| Error::Other(format!("cannot watch {}: {e}", vault.root.display())))?;
    Ok((
        VaultWatcher {
            _debouncer: debouncer,
        },
        rx,
    ))
}
