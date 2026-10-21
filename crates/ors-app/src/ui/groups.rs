//! Tab groups: the split layout of the workspace, one tab bar and one view
//! per group, drag & drop of tabs between groups.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Layout as UiLayout, Margin, RichText, Sense,
    Stroke, UiBuilder, Vec2,
};

use super::icons;
use super::theme::Palette;
use super::ws_ui::WsAction;
use super::{Action, UiCaches};
use crate::commands::Command;
use crate::state::{AppState, FsTarget, TabContent, TabId};
use crate::workspace::{GroupId, Node, SplitDir};
use crate::ws_commands::WsCommand;

/// Drag payload of a tab.
#[derive(Debug, Clone)]
pub struct TabDrag {
    pub id: TabId,
    pub title: String,
}

const TAB_BAR_H: f32 = 32.0;
const SEP: f32 = 5.0;

pub fn workspace_area(
    ui: &mut egui::Ui,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    let rect = ui.available_rect_before_wrap();
    let root = state.ws.layout.root.clone();
    draw_node(ui, rect, &root, &mut Vec::new(), state, caches, actions);
    // The whole rect is consumed.
    ui.allocate_rect(rect, Sense::hover());
}

fn split_rects(rect: egui::Rect, row: bool, ratio: f32) -> (egui::Rect, egui::Rect, egui::Rect) {
    let total = if row { rect.width() } else { rect.height() };
    let margin = 120.0_f32.min(total / 2.0);
    let first_len = (total * ratio).clamp(margin, total - margin);
    if row {
        let x = rect.left() + first_len;
        (
            egui::Rect::from_min_max(rect.min, egui::pos2(x - SEP / 2.0, rect.bottom())),
            egui::Rect::from_min_max(
                egui::pos2(x - SEP / 2.0, rect.top()),
                egui::pos2(x + SEP / 2.0, rect.bottom()),
            ),
            egui::Rect::from_min_max(egui::pos2(x + SEP / 2.0, rect.top()), rect.max),
        )
    } else {
        let y = rect.top() + first_len;
        (
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), y - SEP / 2.0)),
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), y - SEP / 2.0),
                egui::pos2(rect.right(), y + SEP / 2.0),
            ),
            egui::Rect::from_min_max(egui::pos2(rect.left(), y + SEP / 2.0), rect.max),
        )
    }
}

fn draw_node(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    node: &Node,
    path: &mut Vec<bool>,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    match node {
        Node::Leaf(g) => draw_group(ui, rect, *g, state, caches, actions),
        Node::Split {
            row,
            ratio,
            first,
            second,
        } => {
            let (r1, sep, r2) = split_rects(rect, *row, *ratio);
            path.push(false);
            draw_node(ui, r1, first, path, state, caches, actions);
            path.pop();
            path.push(true);
            draw_node(ui, r2, second, path, state, caches, actions);
            path.pop();
            let pal = Palette::of(ui.ctx());
            let id = ui.scope_id().with(("split-sep", path.clone()));
            let resp = ui.interact(sep, id, Sense::drag());
            let icon = if *row {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::ResizeVertical
            };
            let resp = resp.on_hover_cursor(icon);
            let color = if resp.hovered() || resp.dragged() {
                pal.accent
            } else {
                pal.border
            };
            let line = if *row {
                egui::Rect::from_center_size(sep.center(), Vec2::new(1.0, sep.height()))
            } else {
                egui::Rect::from_center_size(sep.center(), Vec2::new(sep.width(), 1.0))
            };
            ui.painter().rect_filled(line, 0.0, color);
            if resp.dragged()
                && let Some(p) = ui.input(|i| i.pointer.interact_pos())
            {
                let new = if *row {
                    (p.x - rect.left()) / rect.width()
                } else {
                    (p.y - rect.top()) / rect.height()
                };
                if let Some(Node::Split { ratio, .. }) = state.ws.layout.root.at_path_mut(path) {
                    *ratio = new.clamp(0.1, 0.9);
                }
            }
        }
    }
}

fn draw_group(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    g: GroupId,
    state: &mut AppState,
    caches: &mut UiCaches,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let Some(group) = state.ws.layout.group(g).cloned() else {
        return;
    };
    let focused = state.ws.layout.focused == g;
    let multi = state.ws.layout.groups.len() > 1;

    // Focus follows clicks anywhere in the group.
    if !focused
        && ui.input(|i| i.pointer.any_pressed())
        && ui.rect_contains_pointer(rect)
        && !egui::DragAndDrop::has_any_payload(ui.ctx())
    {
        actions.push(Action::Ws(WsAction::FocusGroup(g)));
    }

    // ----- tab bar -----
    let bar_rect = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), TAB_BAR_H));
    ui.painter().rect_filled(bar_rect, 0.0, pal.bg_secondary);
    {
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    bar_rect.min + Vec2::new(4.0, 5.0),
                    bar_rect.max - Vec2::new(4.0, 0.0),
                ))
                .layout(UiLayout::left_to_right(Align::Min)),
        );
        bar.set_clip_rect(bar_rect.intersect(ui.clip_rect()));
        let rightmost = is_rightmost(&state.ws.layout.root, g);
        tab_bar(&mut bar, state, &group, g, focused, rightmost, actions);
    }
    ui.painter().line_segment(
        [bar_rect.left_bottom(), bar_rect.right_bottom()],
        Stroke::new(1.0, pal.border),
    );

    // ----- view -----
    let body = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + TAB_BAR_H), rect.max);
    let active_idx = group.active.and_then(|id| state.tab_index(id));
    let saved_active = state.active;
    state.active = active_idx;
    {
        let mut view = ui.new_child(
            UiBuilder::new()
                .max_rect(body)
                .layout(UiLayout::top_down(Align::Min)),
        );
        view.set_clip_rect(body.intersect(ui.clip_rect()));
        view.painter().rect_filled(body, 0.0, pal.bg_primary);
        view.set_min_size(body.size());
        super::leaf_content(&mut view, state, caches, actions);
    }
    state.active = saved_active;
    if multi && focused {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            Stroke::new(1.0, pal.accent.gamma_multiply(0.45)),
            egui::StrokeKind::Inside,
        );
    }

    drop_zones(ui, rect, body, g, actions);
}

fn tab_bar(
    ui: &mut egui::Ui,
    state: &AppState,
    group: &crate::workspace::Group,
    g: GroupId,
    focused: bool,
    rightmost: bool,
    actions: &mut Vec<Action>,
) {
    let pal = Palette::of(ui.ctx());
    let bar_w = ui.available_width();
    // Room for the "+" button and, on the rightmost group, the sidebar toggle.
    let reserved = if rightmost { 64.0 } else { 34.0 };
    egui::ScrollArea::horizontal()
        .id_salt(("tabbar", g))
        .max_width((bar_w - reserved).max(40.0))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (n, tab_id) in group.tabs.iter().enumerate() {
                    let Some(tab) = state.tabs.iter().find(|t| t.id == *tab_id) else {
                        continue;
                    };
                    let is_active = group.active == Some(*tab_id);
                    let pinned = state.ws.layout.is_pinned(*tab_id);
                    let dirty = tab
                        .note()
                        .and_then(|n| state.buffers.get(&n.path))
                        .is_some_and(|b| b.is_dirty());
                    let title = tab.title();
                    let fill = if is_active {
                        pal.bg_primary
                    } else {
                        Color32::TRANSPARENT
                    };
                    let frame = Frame::new()
                        .fill(fill)
                        .corner_radius(CornerRadius {
                            nw: 8,
                            ne: 8,
                            sw: 0,
                            se: 0,
                        })
                        .inner_margin(Margin::symmetric(10, 4))
                        .stroke(if is_active {
                            Stroke::new(1.0, pal.border)
                        } else {
                            Stroke::NONE
                        });
                    let inner = frame.show(ui, |ui| {
                        ui.set_min_width(80.0);
                        ui.set_max_width(190.0);
                        ui.horizontal(|ui| {
                            let color = if is_active && focused {
                                pal.strong
                            } else if is_active {
                                pal.text
                            } else {
                                pal.muted
                            };
                            if matches!(tab.content, TabContent::Graph(_)) {
                                ui.label(icons::text(icons::GRAPH, 14.0).color(color));
                            }
                            if pinned {
                                ui.label(icons::text(icons::PIN, 13.0).color(color));
                            }
                            let label = ui.add(
                                egui::Label::new(RichText::new(&title).color(color))
                                    .truncate()
                                    .selectable(false)
                                    .sense(Sense::click_and_drag()),
                            );
                            let close = ui.add(
                                egui::Button::new(
                                    RichText::new(if dirty { "●" } else { "×" }).color(pal.faint),
                                )
                                .frame(false)
                                .small(),
                            );
                            (label, close.on_hover_text("Close (Ctrl+W)"))
                        })
                        .inner
                    });
                    let (label, close) = inner.inner;
                    let tab_resp = inner.response.interact(Sense::click_and_drag());
                    // `tab_resp` is registered after the children and covers them, so
                    // it receives the click on "×": route it back to the close button.
                    let on_close = click_on_close(
                        tab_resp.clicked(),
                        tab_resp.interact_pointer_pos(),
                        close.rect,
                    );
                    if (label.clicked() || tab_resp.clicked()) && !on_close {
                        actions.push(Action::Ws(WsAction::ActivateTab(*tab_id)));
                    }
                    if label.middle_clicked()
                        || tab_resp.middle_clicked()
                        || close.clicked()
                        || on_close
                    {
                        actions.push(Action::Ws(WsAction::CloseTab(*tab_id)));
                    }
                    let payload = TabDrag {
                        id: *tab_id,
                        title: title.clone(),
                    };
                    if label.dragged() || label.drag_started() {
                        label.dnd_set_drag_payload(payload.clone());
                    }
                    if tab_resp.dragged() || tab_resp.drag_started() {
                        tab_resp.dnd_set_drag_payload(payload);
                    }
                    let dropped = tab_resp
                        .dnd_release_payload::<TabDrag>()
                        .or_else(|| label.dnd_release_payload::<TabDrag>());
                    if let Some(d) = dropped {
                        actions.push(Action::Ws(WsAction::MoveTabTo(d.id, g, Some(n))));
                    }
                    if tab_resp.dnd_hover_payload::<TabDrag>().is_some()
                        || label.dnd_hover_payload::<TabDrag>().is_some()
                    {
                        ui.painter().line_segment(
                            [
                                inner.response.rect.left_top(),
                                inner.response.rect.left_bottom(),
                            ],
                            Stroke::new(2.0, pal.accent),
                        );
                    }
                    let note_path = tab.note().map(|n| n.path.clone());
                    tab_resp.context_menu(|ui| {
                        tab_menu(ui, *tab_id, pinned, note_path.as_ref(), actions);
                    });
                }
            });
        });
    // New tab button + drop area at the end of the bar.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, pal.border.gamma_multiply(0.7));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icons::PLUS,
        icons::font(15.0),
        pal.muted,
    );
    if resp.on_hover_text("New note (Ctrl+N)").clicked() {
        actions.push(Action::Exec(Command::NewNote));
    }
    let toggle_w = if rightmost { 30.0 } else { 0.0 };
    let rest = ui.allocate_response(
        Vec2::new((ui.available_width() - toggle_w).max(1.0), TAB_BAR_H),
        Sense::hover(),
    );
    if let Some(d) = rest.dnd_release_payload::<TabDrag>() {
        actions.push(Action::Ws(WsAction::MoveTabTo(d.id, g, None)));
    }
    if rightmost {
        let open = state.settings.right_open;
        let tip = if open {
            "Collapse right sidebar"
        } else {
            "Expand right sidebar"
        };
        if super::explorer_ui::icon_button(ui, icons::SIDEBAR, tip, false).clicked() {
            actions.push(Action::Exec(Command::ToggleRightSidebar));
        }
    }
}

/// A click on a tab counts as a click on its "×" when the pointer was inside
/// the close button's rectangle.
fn click_on_close(clicked: bool, pos: Option<egui::Pos2>, close: egui::Rect) -> bool {
    clicked && pos.is_some_and(|p| close.contains(p))
}

/// Whether group `g` touches the right edge of the workspace.
fn is_rightmost(node: &Node, g: GroupId) -> bool {
    match node {
        Node::Leaf(id) => *id == g,
        Node::Split {
            row: true, second, ..
        } => is_rightmost(second, g),
        Node::Split { first, second, .. } => is_rightmost(first, g) || is_rightmost(second, g),
    }
}

fn tab_menu(
    ui: &mut egui::Ui,
    id: TabId,
    pinned: bool,
    path: Option<&ors_core::NotePath>,
    actions: &mut Vec<Action>,
) {
    if ui.button("Close").clicked() {
        actions.push(Action::Ws(WsAction::CloseTab(id)));
        ui.close();
    }
    if ui.button("Close others").clicked() {
        actions.push(Action::Ws(WsAction::CloseOthers(id)));
        ui.close();
    }
    if ui.button(if pinned { "Unpin" } else { "Pin" }).clicked() {
        actions.push(Action::Ws(WsAction::TogglePin(id)));
        ui.close();
    }
    ui.separator();
    if ui.button("Split right").clicked() {
        actions.push(Action::Ws(WsAction::SplitTab(id, SplitDir::Right)));
        ui.close();
    }
    if ui.button("Split down").clicked() {
        actions.push(Action::Ws(WsAction::SplitTab(id, SplitDir::Down)));
        ui.close();
    }
    if let Some(p) = path {
        ui.separator();
        if ui.button("Copy path").clicked() {
            actions.push(Action::Ws(WsAction::CopyText(p.as_str().to_string())));
            ui.close();
        }
        if ui.button("Reveal in navigation").clicked() {
            actions.push(Action::Ws(WsAction::ActivateTab(id)));
            actions.push(Action::Exec(Command::Ws(WsCommand::RevealActiveFile)));
            ui.close();
        }
        if ui.button("Bookmark").clicked() {
            actions.push(Action::Ws(WsAction::ToggleBookmark(FsTarget::File(
                p.clone(),
            ))));
            ui.close();
        }
    }
}

/// Dropping a tab (or a file) on the view: centre = move into this group,
/// edges = new split.
fn drop_zones(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    body: egui::Rect,
    g: GroupId,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    let tab = egui::DragAndDrop::payload::<TabDrag>(&ctx);
    let file = egui::DragAndDrop::payload::<Vec<FsTarget>>(&ctx);
    if tab.is_none() && file.is_none() {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    if !body.contains(p) || !rect.contains(p) {
        return;
    }
    let pal = Palette::of(&ctx);
    let fx = (p.x - body.left()) / body.width();
    let fy = (p.y - body.top()) / body.height();
    let (zone, dir) = if fx < 0.25 {
        (
            egui::Rect::from_min_max(body.min, egui::pos2(body.center().x, body.bottom())),
            Some(SplitDir::Left),
        )
    } else if fx > 0.75 {
        (
            egui::Rect::from_min_max(egui::pos2(body.center().x, body.top()), body.max),
            Some(SplitDir::Right),
        )
    } else if fy < 0.25 {
        (
            egui::Rect::from_min_max(body.min, egui::pos2(body.right(), body.center().y)),
            Some(SplitDir::Up),
        )
    } else if fy > 0.75 {
        (
            egui::Rect::from_min_max(egui::pos2(body.left(), body.center().y), body.max),
            Some(SplitDir::Down),
        )
    } else {
        (body, None)
    };
    ui.painter().rect_filled(zone, 0.0, pal.accent_soft);
    ui.painter().rect_stroke(
        zone,
        0.0,
        Stroke::new(2.0, pal.accent),
        egui::StrokeKind::Inside,
    );
    if !ctx.input(|i| i.pointer.any_released()) {
        return;
    }
    if let Some(t) = tab {
        actions.push(Action::Ws(match dir {
            Some(d) => WsAction::MoveTabSplit(t.id, g, d),
            None => WsAction::MoveTabTo(t.id, g, None),
        }));
    } else if let Some(files) = file
        && let Some(FsTarget::File(f)) = files.first()
    {
        // A note dropped on the view opens in this group.
        actions.push(Action::Ws(WsAction::OpenInGroup(f.clone(), g)));
    }
}

/// Floating label following the pointer while something is dragged.
pub fn drag_preview(ctx: &egui::Context) {
    let label = if let Some(t) = egui::DragAndDrop::payload::<TabDrag>(ctx) {
        t.title.clone()
    } else if let Some(f) = egui::DragAndDrop::payload::<Vec<FsTarget>>(ctx) {
        match f.as_slice() {
            [one] => one.display(),
            many => format!("{} items", many.len()),
        }
    } else {
        return;
    };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let pal = Palette::of(ctx);
    egui::Area::new(egui::Id::unique("drag-preview"))
        .order(egui::Order::Tooltip)
        .fixed_pos(p + Vec2::new(12.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(pal.bg_secondary)
                .stroke(Stroke::new(1.0, pal.accent))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(RichText::new(label).color(pal.strong));
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2};

    #[test]
    fn click_on_close_needs_a_click_inside_the_button() {
        let close = Rect::from_min_max(pos2(10.0, 10.0), pos2(20.0, 20.0));
        assert!(click_on_close(true, Some(pos2(15.0, 15.0)), close));
        assert!(!click_on_close(true, Some(pos2(40.0, 15.0)), close));
        assert!(!click_on_close(true, None, close));
        assert!(!click_on_close(false, Some(pos2(15.0, 15.0)), close));
    }

    #[test]
    fn rightmost_group_follows_the_split_tree() {
        let leaf = |g| Box::new(Node::Leaf(g));
        let row = Node::Split {
            row: true,
            ratio: 0.5,
            first: leaf(1),
            second: Box::new(Node::Split {
                row: false,
                ratio: 0.5,
                first: leaf(2),
                second: leaf(3),
            }),
        };
        assert!(!is_rightmost(&row, 1));
        assert!(is_rightmost(&row, 2));
        assert!(is_rightmost(&row, 3));
        assert!(is_rightmost(&Node::Leaf(1), 1));
    }
}
