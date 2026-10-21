//! Actions emitted by the workspace UI (explorer, bookmarks, tab groups)
//! and their application to [`AppState`].

use ors_core::NotePath;

use crate::bookmarks::{Bookmark, now_millis};
use crate::state::{AppState, FsTarget, OpenIn, TabId};
use crate::workbench::Dialog;
use crate::workspace::{GroupId, SplitDir};

#[derive(Debug, Clone)]
pub enum WsAction {
    // explorer
    Move(Vec<FsTarget>, String),
    Rename(FsTarget, String),
    Duplicate(NotePath),
    Dialog(Dialog),
    CopyText(String),
    RevealSystem(FsTarget),
    // bookmarks
    ToggleBookmark(FsTarget),
    NewBookmarkGroup,
    OpenBookmark(Bookmark, OpenIn),
    MoveBookmark(Vec<usize>, Vec<usize>),
    ShiftBookmark(Vec<usize>, isize),
    RemoveBookmark(Vec<usize>),
    // tab groups
    ActivateTab(TabId),
    CloseTab(TabId),
    CloseOthers(TabId),
    TogglePin(TabId),
    SplitTab(TabId, SplitDir),
    MoveTabTo(TabId, GroupId, Option<usize>),
    MoveTabSplit(TabId, GroupId, SplitDir),
    FocusGroup(GroupId),
    OpenInGroup(NotePath, GroupId),
}

pub fn apply(state: &mut AppState, action: WsAction) {
    match action {
        WsAction::Move(targets, folder) => state.move_targets(&targets, &folder),
        WsAction::Rename(target, name) => match state.rename_target(&target, &name) {
            Ok(()) => {
                state.ws.explorer.renaming = None;
                state.ws.explorer.rename_error = None;
            }
            Err(e) => state.ws.explorer.rename_error = Some(e),
        },
        WsAction::Duplicate(p) => {
            state.duplicate_file(&p);
        }
        WsAction::Dialog(d) => state.ws.dialog = Some(d),
        WsAction::CopyText(t) => state.ws.clipboard = Some(t),
        WsAction::RevealSystem(t) => state.reveal_in_system(&t),
        WsAction::ToggleBookmark(FsTarget::File(p)) => state.toggle_file_bookmark(&p),
        WsAction::ToggleBookmark(FsTarget::Folder(f)) => {
            state.add_bookmark(Bookmark::folder(&f, now_millis()));
        }
        WsAction::NewBookmarkGroup => {
            let n = state
                .ws
                .bookmarks
                .items
                .iter()
                .filter(|b| b.is_group())
                .count()
                + 1;
            state.add_bookmark(Bookmark::group(&format!("Group {n}"), now_millis()));
        }
        WsAction::OpenBookmark(b, how) => state.open_bookmark(&b, how),
        WsAction::MoveBookmark(from, to) => {
            if state.ws.bookmarks.move_into(&from, &to) {
                state.bookmarks_changed();
            }
        }
        WsAction::ShiftBookmark(path, d) => {
            if state.ws.bookmarks.shift(&path, d) {
                state.bookmarks_changed();
            }
        }
        WsAction::RemoveBookmark(path) => {
            state.ws.bookmarks.remove(&path);
            state.bookmarks_changed();
        }
        WsAction::ActivateTab(id) => {
            if let Some(i) = state.tab_index(id) {
                state.set_active(i);
            }
        }
        WsAction::CloseTab(id) => state.close_tab_by_id(id),
        WsAction::CloseOthers(id) => state.close_others(id),
        WsAction::TogglePin(id) => state.ws.layout.toggle_pin(id),
        WsAction::SplitTab(id, dir) => {
            if let Some(i) = state.tab_index(id) {
                state.set_active(i);
            }
            state.split_active(dir);
        }
        WsAction::MoveTabTo(id, g, idx) => state.move_tab_to_group(id, g, idx),
        WsAction::MoveTabSplit(id, g, dir) => state.move_tab_to_split(id, g, dir),
        WsAction::FocusGroup(g) => state.focus_group(g),
        WsAction::OpenInGroup(p, g) => state.open_in_group(&p, g),
    }
}

/// Apply what the state asked the UI to do: clipboard text and text to
/// insert at the cursor of the focused editor.
pub fn flush_pending(ctx: &eframe::egui::Context, state: &mut AppState) {
    use eframe::egui::{self, text::CCursor, text::CCursorRange};
    if let Some(text) = state.ws.clipboard.take() {
        ctx.copy_text(text);
    }
    let Some(text) = state.ws.pending_insert.take() else {
        return;
    };
    let Some((tab_id, path)) = state
        .active_tab()
        .and_then(|t| t.note().map(|n| (t.id, n.path.clone())))
    else {
        return;
    };
    let id = egui::Id::unique(("editor", tab_id));
    let Some(buf) = state.buffers.get_mut(&path) else {
        return;
    };
    let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let total = buf.text.chars().count();
    let cursor = st
        .cursor
        .char_range()
        .map_or(total, |r| usize::from(r.primary.index).min(total));
    let byte = buf
        .text
        .char_indices()
        .nth(cursor)
        .map_or(buf.text.len(), |(b, _)| b);
    buf.text.insert_str(byte, &text);
    st.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(
            cursor + text.chars().count(),
        ))));
    egui::TextEdit::store_state(ctx, id, st);
    state.buffer_edited(&path);
}
