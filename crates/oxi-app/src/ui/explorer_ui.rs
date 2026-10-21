//! File explorer and bookmarks panes (Obsidian style). The model lives in
//! [`crate::explorer`] and [`crate::bookmarks`]; this file only draws and
//! emits [`WsAction`]s.

use eframe::egui::{self, Align2, Color32, FontId, Key, RichText, Sense, Stroke, Vec2};
use oxi_core::NotePath;

use super::icons;
use super::theme::Palette;
use super::ws_ui::WsAction;
use super::{Action, UiCaches};
use crate::bookmarks::Bookmark;
use crate::commands::Command;
use crate::explorer::{ExplorerState, SortOrder, Tree, join};
use crate::state::{AppState, FsTarget, Modal, OpenIn};
use crate::workbench::Dialog;
use crate::ws_commands::WsCommand;

const ROW_H: f32 = 24.0;
const INDENT: f32 = 16.0;

/// Cached sorted tree (rebuilt when the vault, its index or the sort
/// order change).
#[derive(Default)]
pub struct ExplorerCache {
    key: Option<(usize, usize, u64, SortOrder)>,
    tree: Tree,
}

/// Small frameless icon button.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, tip: &str, active: bool) -> egui::Response {
    let pal = Palette::of(ui.ctx());
    let color = if active { pal.accent } else { pal.muted };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if resp.hovered() || active {
        ui.painter().rect_filled(
            rect,
            4.0,
            if active {
                pal.accent_soft
            } else {
                pal.border.gamma_multiply(0.7)
            },
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        super::icons::font(16.0),
        if resp.hovered() { pal.strong } else { color },
    );
    resp.on_hover_text(tip)
}

fn ws(actions: &mut Vec<Action>, a: WsAction) {
    actions.push(Action::Ws(a));
}

pub fn explorer(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let mut ex = std::mem::take(&mut state.ws.explorer);
    let active = state.active_note().cloned();

    // ----- toolbar -----
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let w = ui.available_width();
        ui.add_space(((w - 5.0 * 28.0) / 2.0).max(0.0));
        if icon_button(ui, icons::NOTE_PENCIL, "New note", false).clicked() {
            actions.push(Action::Exec(Command::NewNote));
        }
        if icon_button(ui, icons::FOLDER_PLUS, "New folder", false).clicked() {
            let parent = active
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or("")
                .to_string();
            actions.push(Action::Modal(Modal::NewFolder {
                parent,
                name: String::new(),
            }));
        }
        let r = icon_button(ui, icons::SORT_ASC, "Change sort order", false);
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(210.0);
            for o in SortOrder::ALL {
                if ui.selectable_label(ex.sort == o, o.label()).clicked() {
                    ex.sort = o;
                    ui.close();
                }
            }
        });
        if icon_button(
            ui,
            icons::CROSSHAIR,
            "Auto-reveal current file",
            ex.auto_reveal,
        )
        .clicked()
        {
            ex.auto_reveal = !ex.auto_reveal;
            if ex.auto_reveal
                && let Some(p) = &active
            {
                ex.reveal(p);
            }
        }
        if icon_button(ui, icons::COLLAPSE, "Collapse all", false).clicked() {
            ex.expanded.clear();
        }
    });
    ui.add_space(2.0);

    if ex.auto_reveal && ex.last_active != active {
        if let Some(p) = &active {
            ex.reveal(p);
        }
        ex.last_active = active.clone();
    }

    // ----- tree -----
    let key = (
        state.files.len(),
        state.folders.len(),
        state.generation,
        ex.sort,
    );
    if caches.explorer.key != Some(key) {
        caches.explorer.tree = Tree::build(state.explorer_entries(), &state.folders, ex.sort);
        caches.explorer.key = Some(key);
    }
    let tree = std::mem::take(&mut caches.explorer.tree);
    let mut visible = Vec::new();
    tree.visible("", &ex.expanded, &mut visible);
    let dirty: Vec<NotePath> = state
        .buffers
        .iter()
        .filter(|(_, b)| b.is_dirty())
        .map(|(p, _)| p.clone())
        .collect();
    let bookmarked = |p: &str| state.ws.bookmarks.has_file(p);
    let bookmarked_set: std::collections::HashSet<String> = state
        .files
        .iter()
        .filter(|f| bookmarked(f.as_str()))
        .map(|f| f.as_str().to_string())
        .collect();

    let pane_rect = ui.available_rect_before_wrap();
    if ui.input(|i| i.key_pressed(Key::F2))
        && ui.rect_contains_pointer(pane_rect)
        && ex.renaming.is_none()
        && ex.selection.len() == 1
        && let Some(t) = ex.selection.first().cloned()
    {
        start_rename(&mut ex, &t);
    }

    let mut cx = Cx {
        pal,
        active: active.as_ref(),
        dirty: &dirty,
        bookmarked: &bookmarked_set,
        visible: &visible,
        n_selected: ex.selection.len(),
        ignored: &state.git.ignored,
        git: state.git.usable(),
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            draw_tree(ui, &tree, "", 0, &mut ex, &mut cx, actions);
            // Empty area: root context menu and drop target.
            let rest = ui.allocate_response(
                ui.available_size().max(Vec2::new(10.0, 60.0)),
                Sense::click(),
            );
            if let Some(payload) = rest.dnd_release_payload::<Vec<FsTarget>>() {
                ws(actions, WsAction::Move((*payload).clone(), String::new()));
            }
            if rest.clicked() {
                ex.selection.clear();
            }
            rest.context_menu(|ui| folder_menu(ui, "", None, &mut ex, actions));
        });
    caches.explorer.tree = tree;
    state.ws.explorer = ex;
}

struct Cx<'a> {
    pal: Palette,
    active: Option<&'a NotePath>,
    dirty: &'a [NotePath],
    bookmarked: &'a std::collections::HashSet<String>,
    visible: &'a [FsTarget],
    n_selected: usize,
    /// Paths git ignores (folders end with `/`), dimmed in the tree.
    ignored: &'a std::collections::HashSet<String>,
    /// Git backup is active: the menus offer "Ignorer dans Git".
    git: bool,
}

fn start_rename(ex: &mut ExplorerState, t: &FsTarget) {
    let name = match t {
        FsTarget::File(p) => p.stem().to_string(),
        FsTarget::Folder(f) => f.rsplit('/').next().unwrap_or(f).to_string(),
    };
    ex.rename_error = None;
    ex.renaming = Some((t.clone(), name));
}

fn draw_tree(
    ui: &mut egui::Ui,
    tree: &Tree,
    prefix: &str,
    depth: usize,
    ex: &mut ExplorerState,
    cx: &mut Cx<'_>,
    actions: &mut Vec<Action>,
) {
    for (name, child) in &tree.folders {
        let path = join(prefix, name);
        let target = FsTarget::Folder(path.clone());
        let open = ex.expanded.contains(&path);
        folder_row(ui, name, &path, &target, open, depth, ex, cx, actions);
        if open {
            draw_tree(ui, child, &path, depth + 1, ex, cx, actions);
        }
    }
    for f in &tree.files {
        file_row(ui, f, depth, ex, cx, actions);
    }
}

/// Paint the background of a row; returns the content rect.
fn row_frame(
    ui: &mut egui::Ui,
    resp: &egui::Response,
    selected: bool,
    active: bool,
    drop: bool,
    pal: &Palette,
) {
    let rect = resp.rect;
    let bg = if drop {
        pal.accent_soft.gamma_multiply(1.6)
    } else if active {
        pal.accent_soft
    } else if selected {
        pal.border
    } else if resp.hovered() {
        pal.border.gamma_multiply(0.6)
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 4.0, bg);
    }
    if drop {
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn folder_row(
    ui: &mut egui::Ui,
    name: &str,
    path: &str,
    target: &FsTarget,
    open: bool,
    depth: usize,
    ex: &mut ExplorerState,
    cx: &mut Cx<'_>,
    actions: &mut Vec<Action>,
) {
    let pal = cx.pal;
    let git_ignored = cx.ignored.contains(&format!("{path}/"));
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_H),
        Sense::click_and_drag(),
    );
    let resp = egui::Response { rect, ..resp };
    let drop = resp.dnd_hover_payload::<Vec<FsTarget>>().is_some_and(|p| {
        !p.iter().any(|t| match t {
            FsTarget::Folder(f) => path == f || path.starts_with(&format!("{f}/")),
            FsTarget::File(_) => false,
        })
    });
    row_frame(ui, &resp, ex.is_selected(target), false, drop, &pal);
    let x = rect.left() + 4.0 + depth as f32 * INDENT;
    ui.painter().text(
        egui::pos2(x + 6.0, rect.center().y),
        Align2::CENTER_CENTER,
        if open {
            icons::CARET_DOWN
        } else {
            icons::CARET_RIGHT
        },
        super::icons::font(11.0),
        pal.muted,
    );
    let renaming = matches!(&ex.renaming, Some((t, _)) if t == target);
    if renaming {
        rename_edit(ui, rect, x + 16.0, ex, target, actions);
    } else {
        let clip = ui
            .painter()
            .with_clip_rect(rect.shrink2(Vec2::new(0.0, 0.0)));
        clip.text(
            egui::pos2(x + 16.0, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(13.5),
            if git_ignored { pal.faint } else { pal.text },
        );
    }
    if ex.scroll_to.as_ref() == Some(target) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
        ex.scroll_to = None;
    }
    if resp.drag_started() {
        // Dragging an unselected row drags only that row.
        if !ex.is_selected(target) {
            ex.selection = vec![target.clone()];
        }
    }
    if resp.dragged() || resp.drag_started() {
        let payload: Vec<FsTarget> = if ex.is_selected(target) {
            crate::explorer::top_level(&ex.selection)
        } else {
            vec![target.clone()]
        };
        resp.dnd_set_drag_payload(payload);
    }
    if let Some(p) = resp.dnd_release_payload::<Vec<FsTarget>>() {
        ws(actions, WsAction::Move((*p).clone(), path.to_string()));
        ex.expanded.insert(path.to_string());
    }
    if resp.clicked() && !renaming {
        let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        if ex.click(target, ctrl, shift, cx.visible) && !ex.expanded.remove(path) {
            ex.expanded.insert(path.to_string());
        }
    }
    if resp.secondary_clicked() && !ex.is_selected(target) {
        ex.selection = vec![target.clone()];
    }
    resp.context_menu(|ui| {
        if ex.selection.len() > 1 && ex.is_selected(target) {
            multi_menu(ui, ex, actions);
        } else {
            folder_menu(ui, path, cx.git.then_some(git_ignored), ex, actions);
        }
    });
}

fn file_row(
    ui: &mut egui::Ui,
    f: &NotePath,
    depth: usize,
    ex: &mut ExplorerState,
    cx: &mut Cx<'_>,
    actions: &mut Vec<Action>,
) {
    let pal = cx.pal;
    let target = FsTarget::File(f.clone());
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_H),
        Sense::click_and_drag(),
    );
    let is_active = cx.active == Some(f);
    let git_ignored = cx.ignored.contains(f.as_str());
    row_frame(ui, &resp, ex.is_selected(&target), is_active, false, &pal);
    let x = rect.left() + 4.0 + depth as f32 * INDENT + 16.0;
    let renaming = matches!(&ex.renaming, Some((t, _)) if *t == target);
    // Extension is hidden for notes.
    let mut label = if f.is_markdown() {
        f.stem().to_string()
    } else {
        f.file_name().to_string()
    };
    if cx.dirty.contains(f) {
        label.push_str(" ●");
    }
    if renaming {
        rename_edit(ui, rect, x, ex, &target, actions);
    } else {
        let color = if git_ignored {
            pal.faint
        } else if is_active {
            pal.strong
        } else if f.is_markdown() {
            pal.text
        } else {
            pal.muted
        };
        let clip = ui.painter().with_clip_rect(rect);
        clip.text(
            egui::pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.5),
            color,
        );
    }
    if ex.scroll_to.as_ref() == Some(&target) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
        ex.scroll_to = None;
    }
    if resp.drag_started() && !ex.is_selected(&target) {
        ex.selection = vec![target.clone()];
    }
    if resp.dragged() || resp.drag_started() {
        let payload: Vec<FsTarget> = if ex.is_selected(&target) {
            crate::explorer::top_level(&ex.selection)
        } else {
            vec![target.clone()]
        };
        resp.dnd_set_drag_payload(payload);
    }
    // Dropping on a file moves into its folder.
    if let Some(p) = resp.dnd_release_payload::<Vec<FsTarget>>() {
        let folder = f.parent().unwrap_or("").to_string();
        ws(actions, WsAction::Move((*p).clone(), folder));
    }
    let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    if (resp.clicked() || resp.middle_clicked()) && !renaming {
        let plain = ex.click(&target, ctrl, shift, cx.visible);
        if plain || resp.middle_clicked() {
            let how = if resp.middle_clicked() {
                OpenIn::NewTab
            } else {
                OpenIn::CurrentTab
            };
            actions.push(Action::Open(f.clone(), how));
        }
    }
    if resp.double_clicked() && !renaming {
        start_rename(ex, &target);
    }
    if resp.secondary_clicked() && !ex.is_selected(&target) {
        ex.selection = vec![target.clone()];
    }
    let bm = cx.bookmarked.contains(f.as_str());
    resp.context_menu(|ui| {
        if ex.selection.len() > 1 && ex.is_selected(&target) {
            multi_menu(ui, ex, actions);
        } else {
            file_menu(ui, f, bm, cx.git.then_some(git_ignored), ex, actions);
        }
    });
    let _ = cx.n_selected;
}

fn rename_edit(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    x: f32,
    ex: &mut ExplorerState,
    target: &FsTarget,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let Some((_, text)) = ex.renaming.as_mut() else {
        return;
    };
    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(x - 2.0, rect.top() + 1.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
    );
    let id = ui.make_persistent_id(("explorer-rename", format!("{target:?}")));
    let first = !ui.memory(|m| m.has_focus(id)) && ui.data(|d| d.get_temp::<bool>(id)).is_none();
    let resp = ui.put(
        edit_rect,
        egui::TextEdit::singleline(text)
            .id(id)
            .desired_width(edit_rect.width())
            .margin(Vec2::new(4.0, 2.0)),
    );
    if first {
        ui.data_mut(|d| d.insert_temp(id, true));
        resp.request_focus();
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
            st.store(ui.ctx(), id);
        }
    }
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let esc = ui.input(|i| i.key_pressed(Key::Escape));
    if let Some(err) = &ex.rename_error {
        let r = resp.rect;
        ui.painter().text(
            egui::pos2(r.left(), r.bottom() + 2.0),
            Align2::LEFT_TOP,
            err,
            FontId::proportional(11.0),
            Color32::from_rgb(0xe9, 0x31, 0x47),
        );
    }
    let _ = pal;
    if esc {
        ex.renaming = None;
        ex.rename_error = None;
        ui.data_mut(|d| d.remove::<bool>(id));
    } else if enter || (resp.lost_focus() && !resp.has_focus() && ex.rename_error.is_none()) {
        if let Some((t, name)) = ex.renaming.clone() {
            ws(actions, WsAction::Rename(t, name));
        }
        ui.data_mut(|d| d.remove::<bool>(id));
    }
}

// ----- context menus ------------------------------------------------------

fn file_menu(
    ui: &mut egui::Ui,
    f: &NotePath,
    bookmarked: bool,
    git: Option<bool>,
    ex: &mut ExplorerState,
    actions: &mut Vec<Action>,
) {
    let target = FsTarget::File(f.clone());
    if ui.button("Open in new tab").clicked() {
        actions.push(Action::Open(f.clone(), OpenIn::NewTab));
        ui.close();
    }
    if ui.button("Open to the right").clicked() {
        actions.push(Action::Open(f.clone(), OpenIn::SplitRight));
        ui.close();
    }
    if ui.button("Open below").clicked() {
        actions.push(Action::Open(f.clone(), OpenIn::SplitDown));
        ui.close();
    }
    ui.separator();
    if f.is_markdown() && ui.button("Make a copy").clicked() {
        ws(actions, WsAction::Duplicate(f.clone()));
        ui.close();
    }
    if ui.button("Move file to…").clicked() {
        ws(
            actions,
            WsAction::Dialog(Dialog::MoveTo {
                targets: vec![target.clone()],
                query: String::new(),
                selected: 0,
            }),
        );
        ui.close();
    }
    if f.is_markdown() {
        let label = if bookmarked {
            "Remove bookmark"
        } else {
            "Bookmark…"
        };
        if ui.button(label).clicked() {
            ws(actions, WsAction::ToggleBookmark(target.clone()));
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Copy path").clicked() {
        ws(actions, WsAction::CopyText(f.as_str().to_string()));
        ui.close();
    }
    if ui.button("Reveal in system file manager").clicked() {
        ws(actions, WsAction::RevealSystem(target.clone()));
        ui.close();
    }
    git_items(ui, f.as_str(), false, git, actions);
    ui.separator();
    if ui.button("Rename…").clicked() {
        start_rename(ex, &target);
        ui.close();
    }
    if ui
        .button(RichText::new("Delete").color(Color32::from_rgb(0xe9, 0x31, 0x47)))
        .clicked()
    {
        actions.push(Action::Modal(Modal::ConfirmDelete { target }));
        ui.close();
    }
}

/// "Ignorer dans Git" / "Ne plus ignorer" (`git`: `Some(is_ignored)` when
/// the Git backup is active).
fn git_items(
    ui: &mut egui::Ui,
    path: &str,
    is_dir: bool,
    git: Option<bool>,
    actions: &mut Vec<Action>,
) {
    let Some(ignored) = git else {
        return;
    };
    ui.separator();
    let (label, make): (_, fn(String, bool) -> super::git_ui::GitAction) = if ignored {
        ("Ne plus ignorer", |path, is_dir| {
            super::git_ui::GitAction::Unignore { path, is_dir }
        })
    } else {
        ("Ignorer dans Git", |path, is_dir| {
            super::git_ui::GitAction::Ignore { path, is_dir }
        })
    };
    if ui.button(label).clicked() {
        actions.push(Action::Git(make(path.to_string(), is_dir)));
        ui.close();
    }
}

fn folder_menu(
    ui: &mut egui::Ui,
    folder: &str,
    git: Option<bool>,
    ex: &mut ExplorerState,
    actions: &mut Vec<Action>,
) {
    if ui.button("New note").clicked() {
        actions.push(Action::NewNoteIn(folder.to_string()));
        ui.close();
    }
    if ui.button("New folder").clicked() {
        actions.push(Action::Modal(Modal::NewFolder {
            parent: folder.to_string(),
            name: String::new(),
        }));
        ui.close();
    }
    if folder.is_empty() {
        return;
    }
    let target = FsTarget::Folder(folder.to_string());
    ui.separator();
    if ui.button("Move folder to…").clicked() {
        ws(
            actions,
            WsAction::Dialog(Dialog::MoveTo {
                targets: vec![target.clone()],
                query: String::new(),
                selected: 0,
            }),
        );
        ui.close();
    }
    if ui.button("Bookmark…").clicked() {
        ws(actions, WsAction::ToggleBookmark(target.clone()));
        ui.close();
    }
    if ui.button("Copy path").clicked() {
        ws(actions, WsAction::CopyText(folder.to_string()));
        ui.close();
    }
    if ui.button("Reveal in system file manager").clicked() {
        ws(actions, WsAction::RevealSystem(target.clone()));
        ui.close();
    }
    git_items(ui, folder, true, git, actions);
    ui.separator();
    if ui.button("Rename…").clicked() {
        start_rename(ex, &target);
        ui.close();
    }
    if ui
        .button(RichText::new("Delete").color(Color32::from_rgb(0xe9, 0x31, 0x47)))
        .clicked()
    {
        actions.push(Action::Modal(Modal::ConfirmDelete { target }));
        ui.close();
    }
}

fn multi_menu(ui: &mut egui::Ui, ex: &ExplorerState, actions: &mut Vec<Action>) {
    let targets = crate::explorer::top_level(&ex.selection);
    if ui
        .button(format!("Move {} items to…", targets.len()))
        .clicked()
    {
        ws(
            actions,
            WsAction::Dialog(Dialog::MoveTo {
                targets: targets.clone(),
                query: String::new(),
                selected: 0,
            }),
        );
        ui.close();
    }
    ui.separator();
    if ui
        .button(
            RichText::new(format!("Delete {} items", targets.len()))
                .color(Color32::from_rgb(0xe9, 0x31, 0x47)),
        )
        .clicked()
    {
        ws(
            actions,
            WsAction::Dialog(Dialog::ConfirmDeleteMany { targets }),
        );
        ui.close();
    }
}

// ---------------------------------------------------------------------------
// Bookmarks pane
// ---------------------------------------------------------------------------

pub fn bookmarks_pane(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let w = ui.available_width();
        ui.add_space(((w - 2.0 * 28.0) / 2.0).max(0.0));
        if icon_button(ui, icons::STAR, "Bookmark current file", false).clicked() {
            actions.push(Action::Exec(Command::Ws(WsCommand::BookmarkFile)));
        }
        if icon_button(ui, icons::FOLDER_PLUS, "New bookmark group", false).clicked() {
            ws(actions, WsAction::NewBookmarkGroup);
        }
    });
    ui.add_space(4.0);
    if state.ws.bookmarks.items.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new("No bookmarks yet. Right-click a file in the explorer or use \"Bookmark current file\".")
                .color(pal.muted),
        );
        return;
    }
    let items = state.ws.bookmarks.items.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            bookmark_list(ui, &items, &mut Vec::new(), 0, actions);
        });
}

fn bookmark_list(
    ui: &mut egui::Ui,
    items: &[Bookmark],
    path: &mut Vec<usize>,
    depth: usize,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    for (i, b) in items.iter().enumerate() {
        path.push(i);
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ROW_H),
            Sense::click_and_drag(),
        );
        row_frame(ui, &resp, false, false, false, &pal);
        let x = rect.left() + 6.0 + depth as f32 * INDENT;
        let id = ui.make_persistent_id(("bm-group", path.clone()));
        let open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
        let glyph = match b.kind.as_str() {
            "group" => {
                if open {
                    icons::FOLDER_OPEN
                } else {
                    icons::FOLDER
                }
            }
            "folder" => icons::FOLDER,
            "search" => icons::SEARCH,
            "graph" => icons::GRAPH,
            _ => icons::FILE_TEXT,
        };
        ui.painter().text(
            egui::pos2(x + 8.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            super::icons::font(14.0),
            pal.muted,
        );
        let rename_id = ui.make_persistent_id(("bm-rename", path.clone()));
        let renaming = ui.data(|d| d.get_temp::<String>(rename_id));
        if let Some(mut text) = renaming {
            let edit_rect = egui::Rect::from_min_max(
                egui::pos2(x + 22.0, rect.top() + 1.0),
                egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
            );
            let edit = ui.put(
                edit_rect,
                egui::TextEdit::singleline(&mut text).font(FontId::proportional(13.5)),
            );
            let (enter, esc) =
                ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if esc {
                ui.data_mut(|d| d.remove::<String>(rename_id));
            } else if enter || edit.lost_focus() {
                ws(actions, WsAction::RenameBookmark(path.clone(), text));
                ui.data_mut(|d| d.remove::<String>(rename_id));
            } else {
                ui.data_mut(|d| d.insert_temp(rename_id, text));
            }
        } else {
            ui.painter().with_clip_rect(rect).text(
                egui::pos2(x + 22.0, rect.center().y),
                Align2::LEFT_CENTER,
                b.label(),
                FontId::proportional(13.5),
                pal.text,
            );
        }
        if resp.double_clicked() {
            ui.data_mut(|d| d.insert_temp(rename_id, b.label()));
        } else if resp.clicked() || resp.middle_clicked() {
            if b.is_group() {
                ui.data_mut(|d| d.insert_temp(id, !open));
            } else {
                let how = if resp.middle_clicked() || ui.input(|i| i.modifiers.command) {
                    OpenIn::NewTab
                } else {
                    OpenIn::CurrentTab
                };
                ws(actions, WsAction::OpenBookmark(b.clone(), how));
            }
        }
        let this = path.clone();
        resp.dnd_set_drag_payload(this.clone());
        if b.is_group()
            && let Some(from) = resp.dnd_release_payload::<Vec<usize>>()
        {
            ws(
                actions,
                WsAction::MoveBookmark((*from).clone(), this.clone()),
            );
        }
        resp.context_menu(|ui| {
            if !b.is_group() && ui.button("Open in new tab").clicked() {
                ws(actions, WsAction::OpenBookmark(b.clone(), OpenIn::NewTab));
                ui.close();
            }
            if ui.button("Rename").clicked() {
                ui.data_mut(|d| d.insert_temp(rename_id, b.label()));
                ui.close();
            }
            if ui.button("Move up").clicked() {
                ws(actions, WsAction::ShiftBookmark(this.clone(), -1));
                ui.close();
            }
            if ui.button("Move down").clicked() {
                ws(actions, WsAction::ShiftBookmark(this.clone(), 1));
                ui.close();
            }
            if depth > 0 && ui.button("Move out of group").clicked() {
                ws(actions, WsAction::MoveBookmark(this.clone(), Vec::new()));
                ui.close();
            }
            ui.separator();
            if b.is_group() && ui.button("Delete group (keep items)").clicked() {
                ws(actions, WsAction::UngroupBookmark(this.clone()));
                ui.close();
            }
            if ui
                .button(if b.is_group() {
                    "Delete group and items"
                } else {
                    "Remove bookmark"
                })
                .clicked()
            {
                ws(actions, WsAction::RemoveBookmark(this.clone()));
                ui.close();
            }
        });
        if b.is_group()
            && open
            && let Some(children) = &b.items
        {
            bookmark_list(ui, children, path, depth + 1, actions);
        }
        path.pop();
    }
}
