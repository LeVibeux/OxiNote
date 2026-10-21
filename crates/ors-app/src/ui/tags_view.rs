//! The Tags pane: hierarchical tag tree with counts, sorting, expand /
//! collapse all and a "Rename tag" dialog entry.

use eframe::egui::{self, Align, Id, Layout, Popup, RichText, Sense};

use super::Action;
use super::theme::Palette;
use crate::properties_model::{PropsModal, SortMode, TagRow, tag_rows};
use crate::state::{AppState, Modal};

#[derive(Clone, Copy)]
struct TagView {
    nested: bool,
    sort: SortMode,
}

impl Default for TagView {
    fn default() -> Self {
        Self {
            nested: true,
            sort: SortMode::NameAsc,
        }
    }
}

fn node_id(full: &str) -> Id {
    Id::unique(("tag-node", full.to_lowercase()))
}

fn set_all(ctx: &egui::Context, rows: &[TagRow], open: bool) {
    for r in rows {
        if !r.children.is_empty() {
            let mut s = egui::collapsing_header::CollapsingState::load_with_default_open(
                ctx,
                node_id(&r.full_name),
                true,
            );
            s.set_open(open);
            s.store(ctx);
            set_all(ctx, &r.children, open);
        }
    }
}

pub fn tags_pane(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let view_id = Id::unique("tags-view");
    let mut view: TagView = ui.data(|d| d.get_temp(view_id)).unwrap_or_default();
    let tree = state.index.tag_tree();
    let rows = tag_rows(&tree, view.nested, view.sort);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Tags ({})", count_tags(&rows)))
                .small()
                .strong()
                .color(pal.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let sort = ui
                .add(egui::Button::new("⇅").frame(false))
                .on_hover_text("Sort");
            Popup::menu(&sort).show(|ui| {
                for m in SortMode::ALL {
                    if ui.selectable_label(view.sort == m, m.label()).clicked() {
                        view.sort = m;
                        ui.close();
                    }
                }
            });
            let nested = ui
                .add(egui::Button::selectable(view.nested, "⛓").frame_when_inactive(false))
                .on_hover_text("Show nested tags as a tree");
            if nested.clicked() {
                view.nested = !view.nested;
            }
            if ui
                .add(egui::Button::new("⏶").frame(false))
                .on_hover_text("Collapse all")
                .clicked()
            {
                set_all(ui.ctx(), &rows, false);
            }
            if ui
                .add(egui::Button::new("⏷").frame(false))
                .on_hover_text("Expand all")
                .clicked()
            {
                set_all(ui.ctx(), &rows, true);
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(view_id, view));
    if rows.is_empty() {
        ui.label(RichText::new("No tags found.").color(pal.muted));
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| draw_rows(ui, &rows, &pal, actions));
}

fn count_tags(rows: &[TagRow]) -> usize {
    rows.iter().map(|r| 1 + count_tags(&r.children)).sum()
}

fn draw_rows(ui: &mut egui::Ui, rows: &[TagRow], pal: &Palette, actions: &mut Vec<Action>) {
    for r in rows {
        if r.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.vertical(|ui| tag_line(ui, r, pal, actions));
            });
        } else {
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                node_id(&r.full_name),
                true,
            )
            .show_header(ui, |ui| tag_line(ui, r, pal, actions))
            .body(|ui| draw_rows(ui, &r.children, pal, actions));
        }
    }
}

fn tag_line(ui: &mut egui::Ui, r: &TagRow, pal: &Palette, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::Label::new(RichText::new(format!("#{}", r.name)).color(pal.accent))
                .truncate()
                .sense(Sense::click()),
        );
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("#{}", r.full_name));
        if resp.clicked() {
            actions.push(Action::SearchTag(r.full_name.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Search tag").clicked() {
                actions.push(Action::SearchTag(r.full_name.clone()));
                ui.close();
            }
            if ui.button("Rename tag…").clicked() {
                actions.push(Action::Modal(Modal::Props(PropsModal::RenameTag {
                    old: r.full_name.clone(),
                    new: r.full_name.clone(),
                })));
                ui.close();
            }
            if ui.button("Copy tag").clicked() {
                ui.ctx().copy_text(format!("#{}", r.full_name));
                ui.close();
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(r.count.to_string()).small().color(pal.faint));
        });
    });
}
