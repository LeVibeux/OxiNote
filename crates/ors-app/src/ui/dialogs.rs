//! Small dialogs of the workspace area ([`Dialog`]): move-to picker,
//! bulk delete confirmation, template and heading pickers.

use eframe::egui::{self, Key, RichText};

use super::theme::Palette;
use crate::bookmarks::{Bookmark, now_millis};
use crate::state::AppState;
use crate::util::fuzzy_score;
use crate::workbench::Dialog;

/// Result of one frame of a list picker.
enum Pick {
    None,
    Chosen(usize),
    Close,
}

/// Search box + list. `rows` are the already filtered labels.
fn picker(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    hint: &str,
    query: &mut String,
    selected: &mut usize,
    rows: &[String],
) -> Pick {
    let pal = Palette::of(ctx);
    let mut result = Pick::None;
    let n = rows.len();
    if n > 0 {
        *selected = (*selected).min(n - 1);
    }
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) && n > 0 {
            *selected = (*selected + 1) % n;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) && n > 0 {
            *selected = (*selected + n - 1) % n;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Enter) && n > 0 {
            result = Pick::Chosen(*selected);
        }
    });
    let resp = egui::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(6.0);
        let te = ui.add(
            egui::TextEdit::singleline(query)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        if !te.has_focus() {
            te.request_focus();
        }
        if te.changed() {
            *selected = 0;
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (i, row) in rows.iter().enumerate() {
                    let r = ui.add(
                        egui::Button::selectable(
                            i == *selected,
                            RichText::new(row).color(pal.text),
                        )
                        .min_size(egui::vec2(ui.available_width(), 24.0)),
                    );
                    if i == *selected {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        result = Pick::Chosen(i);
                    }
                }
                if rows.is_empty() {
                    ui.label(RichText::new("No match").color(pal.muted));
                }
            });
    });
    if resp.should_close() {
        return Pick::Close;
    }
    result
}

fn filter(items: &[String], query: &str) -> Vec<usize> {
    let mut scored: Vec<(i64, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| fuzzy_score(query, s).map(|sc| (sc, i)))
        .collect();
    if !query.trim().is_empty() {
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    }
    scored.into_iter().map(|(_, i)| i).collect()
}

/// Draw the dialog; it is kept open while it returns `Some`.
pub fn show(ctx: &egui::Context, state: &mut AppState, dialog: Dialog) -> Option<Dialog> {
    match dialog {
        Dialog::MoveTo {
            targets,
            mut query,
            mut selected,
        } => {
            let mut folders: Vec<String> = vec!["/".into()];
            folders.extend(state.folders.iter().cloned());
            let idx = filter(&folders, &query);
            let rows: Vec<String> = idx.iter().map(|&i| folders[i].clone()).collect();
            let what = if targets.len() == 1 {
                targets[0].display()
            } else {
                format!("{} items", targets.len())
            };
            match picker(
                ctx,
                "move-to",
                &format!("Move {what} to…"),
                "Type a folder name",
                &mut query,
                &mut selected,
                &rows,
            ) {
                Pick::Chosen(i) => {
                    let f = &folders[idx[i]];
                    let folder = if f == "/" { "" } else { f.as_str() };
                    state.move_targets(&targets, folder);
                    None
                }
                Pick::Close => None,
                Pick::None => Some(Dialog::MoveTo {
                    targets,
                    query,
                    selected,
                }),
            }
        }
        Dialog::ConfirmDeleteMany { targets } => {
            let mut result = None;
            let resp = egui::Modal::new(egui::Id::unique("confirm-delete-many")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Delete").strong().size(15.0));
                ui.add_space(6.0);
                ui.label(format!("Delete {} items?", targets.len()));
                ui.label(
                    RichText::new("They will be moved according to the vault's trash setting.")
                        .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let del =
                        egui::Button::new(RichText::new("Delete").color(egui::Color32::WHITE))
                            .fill(egui::Color32::from_rgb(0xc0, 0x30, 0x40));
                    if ui.add(del).clicked() {
                        result = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        result = Some(false);
                    }
                });
            });
            match result {
                Some(true) => {
                    state.delete_targets(&targets);
                    None
                }
                Some(false) => None,
                None if resp.should_close() => None,
                None => Some(Dialog::ConfirmDeleteMany { targets }),
            }
        }
        Dialog::TemplatePicker {
            mut query,
            mut selected,
        } => {
            let templates = state.templates();
            let names: Vec<String> = templates
                .iter()
                .map(|t| t.without_extension().to_string())
                .collect();
            let idx = filter(&names, &query);
            let rows: Vec<String> = idx.iter().map(|&i| names[i].clone()).collect();
            match picker(
                ctx,
                "template-picker",
                "Insert template",
                "Type a template name",
                &mut query,
                &mut selected,
                &rows,
            ) {
                Pick::Chosen(i) => {
                    let t = templates[idx[i]].clone();
                    state.insert_template(&t);
                    None
                }
                Pick::Close => None,
                Pick::None => Some(Dialog::TemplatePicker { query, selected }),
            }
        }
        Dialog::HeadingPicker {
            path,
            mut query,
            mut selected,
        } => {
            let headings = state.parsed(&path).headings;
            let mut names: Vec<String> = vec!["(whole file)".into()];
            names.extend(headings.iter().map(|h| h.text.clone()));
            let idx = filter(&names, &query);
            let rows: Vec<String> = idx.iter().map(|&i| names[i].clone()).collect();
            match picker(
                ctx,
                "heading-picker",
                "Bookmark heading",
                "Type a heading",
                &mut query,
                &mut selected,
                &rows,
            ) {
                Pick::Chosen(i) => {
                    let n = idx[i];
                    let b = if n == 0 {
                        Bookmark::file(path.as_str(), now_millis())
                    } else {
                        Bookmark::heading(path.as_str(), &names[n], now_millis())
                    };
                    state.add_bookmark(b);
                    None
                }
                Pick::Close => None,
                Pick::None => Some(Dialog::HeadingPicker {
                    path,
                    query,
                    selected,
                }),
            }
        }
        Dialog::VaultSwitcher => {
            state.switch_vault_requested = true;
            None
        }
    }
}
