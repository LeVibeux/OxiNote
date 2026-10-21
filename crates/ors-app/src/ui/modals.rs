//! Quick switcher, command palette and small dialogs.

use eframe::egui::{self, Key, Modifiers, RichText};

use super::Action;
use super::theme::Palette;
use crate::state::{AppState, FsTarget, Modal, OpenIn};

/// Draw the current modal. Returns the modal to keep open (or `None`).
pub fn show(
    ctx: &egui::Context,
    state: &mut AppState,
    modal: Modal,
    actions: &mut Vec<Action>,
) -> Option<Modal> {
    match modal {
        Modal::QuickSwitcher { query, selected } => {
            quick_switcher(ctx, state, query, selected, actions)
        }
        Modal::CommandPalette { query, selected } => palette(ctx, state, query, selected, actions),
        Modal::Rename { target, name } => {
            let title = match &target {
                FsTarget::File(p) => format!("Rename {}", p.file_name()),
                FsTarget::Folder(f) => format!("Rename folder {f}"),
            };
            match prompt(ctx, &title, name, "Rename", |name| Modal::Rename {
                target: target.clone(),
                name,
            }) {
                Some(PromptResult::Open(m)) => Some(m),
                Some(PromptResult::Confirm(m)) => {
                    actions.push(Action::Confirm(m));
                    None
                }
                None => None,
            }
        }
        Modal::NewFolder { parent, name } => {
            let title = if parent.is_empty() {
                "New folder".to_string()
            } else {
                format!("New folder in {parent}")
            };
            match prompt(ctx, &title, name, "Create", |name| Modal::NewFolder {
                parent: parent.clone(),
                name,
            }) {
                Some(PromptResult::Open(m)) => Some(m),
                Some(PromptResult::Confirm(m)) => {
                    actions.push(Action::Confirm(m));
                    None
                }
                None => None,
            }
        }
        Modal::ConfirmDelete { target } => confirm_delete(ctx, target, actions),
        Modal::Props(m) => super::properties_view::dialog(ctx, state, m, actions),
    }
}

enum PromptResult {
    Open(Modal),
    Confirm(Modal),
}

fn prompt(
    ctx: &egui::Context,
    title: &str,
    mut value: String,
    button: &str,
    make: impl Fn(String) -> Modal,
) -> Option<PromptResult> {
    let mut result = None;
    let resp = egui::Modal::new(egui::Id::unique("prompt")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(6.0);
        let te = ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY));
        if !te.has_focus() && !te.lost_focus() {
            te.request_focus();
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(button).clicked() || enter {
                result = Some(true);
            }
            if ui.button("Cancel").clicked() {
                result = Some(false);
            }
        });
    });
    match result {
        Some(true) => Some(PromptResult::Confirm(make(value))),
        Some(false) => None,
        None if resp.should_close() => None,
        None => Some(PromptResult::Open(make(value))),
    }
}

fn confirm_delete(
    ctx: &egui::Context,
    target: FsTarget,
    actions: &mut Vec<Action>,
) -> Option<Modal> {
    let mut result = None;
    let resp = egui::Modal::new(egui::Id::unique("confirm-delete")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new("Delete").strong().size(15.0));
        ui.add_space(6.0);
        ui.label(format!(
            "Are you sure you want to delete \"{}\"?",
            target.display()
        ));
        ui.label(RichText::new("It will be moved according to the vault's trash setting.").small());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let del = egui::Button::new(RichText::new("Delete").color(egui::Color32::WHITE))
                .fill(egui::Color32::from_rgb(0xc0, 0x30, 0x40));
            if ui.add(del).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                result = Some(true);
            }
            if ui.button("Cancel").clicked() {
                result = Some(false);
            }
        });
    });
    match result {
        Some(true) => {
            actions.push(Action::Confirm(Modal::ConfirmDelete { target }));
            None
        }
        Some(false) => None,
        None if resp.should_close() => None,
        None => Some(Modal::ConfirmDelete { target }),
    }
}

/// Up/Down/Enter handling shared by the switcher and the palette.
fn list_keys(ctx: &egui::Context, selected: &mut usize, len: usize) -> Option<Modifiers> {
    ctx.input_mut(|i| {
        if len > 0 {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                *selected = (*selected + 1) % len;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                *selected = (*selected + len - 1) % len;
            }
        }
        [Modifiers::SHIFT, Modifiers::COMMAND, Modifiers::NONE]
            .into_iter()
            .find(|&m| i.consume_key(m, Key::Enter))
    })
}

fn search_field(ui: &mut egui::Ui, query: &mut String, hint: &str) {
    let te = ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Heading),
    );
    te.request_focus();
}

fn quick_switcher(
    ctx: &egui::Context,
    state: &mut AppState,
    mut query: String,
    mut selected: usize,
    actions: &mut Vec<Action>,
) -> Option<Modal> {
    let pal = Palette::of(ctx);
    let results: Vec<(ors_core::NotePath, Option<String>)> = if query.trim().is_empty() {
        state
            .recent_files
            .iter()
            .chain(
                state
                    .files
                    .iter()
                    .filter(|f| f.is_markdown() || crate::pdf::is_pdf(f)),
            )
            .fold(Vec::new(), |mut acc: Vec<ors_core::NotePath>, p| {
                if !acc.contains(p) {
                    acc.push(p.clone());
                }
                acc
            })
            .into_iter()
            .take(30)
            .map(|p| (p, None))
            .collect()
    } else {
        state
            .index
            .fuzzy_find_files(&query, 200, true)
            .into_iter()
            .filter(|(p, _, _)| p.is_markdown() || crate::pdf::is_pdf(p))
            .take(30)
            .map(|(p, a, _)| (p, a))
            .collect()
    };
    selected = selected.min(results.len().saturating_sub(1));
    let pressed = list_keys(ctx, &mut selected, results.len());
    let mut chosen: Option<(usize, OpenIn)> = None;
    let mut create = false;
    match pressed {
        Some(m) if m == Modifiers::SHIFT => create = true,
        Some(m) if m == Modifiers::COMMAND => chosen = Some((selected, OpenIn::NewTab)),
        Some(_) if results.is_empty() => create = true,
        Some(_) => chosen = Some((selected, OpenIn::CurrentTab)),
        None => {}
    }
    let resp = egui::Modal::new(egui::Id::unique("quick-switcher"))
        .area(
            egui::Modal::default_area(egui::Id::unique("quick-switcher-area"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0]),
        )
        .show(ctx, |ui| {
            ui.set_width(560.0);
            search_field(ui, &mut query, "Find or create a note…");
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(380.0)
                .show(ui, |ui| {
                    for (i, (p, alias)) in results.iter().enumerate() {
                        let text = match alias {
                            Some(a) => format!("{a}  ⟶  {}", p.stem()),
                            None if p.is_markdown() => p.stem().to_string(),
                            None => p.file_name().to_string(),
                        };
                        let r = ui.add(
                            egui::Button::selectable(
                                i == selected,
                                egui::Atoms::new((
                                    RichText::new(text).color(pal.strong),
                                    egui::Atom::grow(),
                                    RichText::new(p.parent().unwrap_or(""))
                                        .small()
                                        .color(pal.faint),
                                )),
                            )
                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                        );
                        if i == selected && pressed.is_none() {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            chosen = Some((i, OpenIn::CurrentTab));
                        }
                    }
                    if results.is_empty() && !query.trim().is_empty() {
                        ui.label(
                            RichText::new("No match. Press ↵ to create this note.")
                                .color(pal.muted),
                        );
                    }
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "↑↓ navigate   ↵ open   ctrl ↵ open in new tab   shift ↵ create   esc dismiss",
                )
                .small()
                .color(pal.faint),
            );
        });
    if create {
        actions.push(Action::OpenOrCreate(query));
        return None;
    }
    if let Some((i, open_in)) = chosen {
        if let Some((p, _)) = results.get(i) {
            actions.push(Action::Open(p.clone(), open_in));
        }
        return None;
    }
    if resp.should_close() {
        return None;
    }
    Some(Modal::QuickSwitcher { query, selected })
}

fn palette(
    ctx: &egui::Context,
    state: &mut AppState,
    mut query: String,
    mut selected: usize,
    actions: &mut Vec<Action>,
) -> Option<Modal> {
    let pal = Palette::of(ctx);
    let results: Vec<(
        crate::commands::Command,
        &'static str,
        Option<egui::KeyboardShortcut>,
    )> = state
        .commands
        .filter(&query)
        .into_iter()
        .map(|s| (s.command, s.name, s.shortcut))
        .collect();
    selected = selected.min(results.len().saturating_sub(1));
    let pressed = list_keys(ctx, &mut selected, results.len());
    let mut chosen = pressed.and_then(|_| results.get(selected).map(|r| r.0));
    let resp = egui::Modal::new(egui::Id::unique("command-palette"))
        .area(
            egui::Modal::default_area(egui::Id::unique("command-palette-area"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0]),
        )
        .show(ctx, |ui| {
            ui.set_width(560.0);
            search_field(ui, &mut query, "Type a command…");
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(380.0)
                .show(ui, |ui| {
                    for (i, (cmd, name, shortcut)) in results.iter().enumerate() {
                        let keys = shortcut
                            .map(|k| ctx.format_shortcut(&k))
                            .unwrap_or_default();
                        let r = ui.add(
                            egui::Button::selectable(
                                i == selected,
                                egui::Atoms::new((
                                    RichText::new(*name).color(pal.strong),
                                    egui::Atom::grow(),
                                    RichText::new(keys).small().color(pal.faint).monospace(),
                                )),
                            )
                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                        );
                        if i == selected && pressed.is_none() {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            chosen = Some(*cmd);
                        }
                    }
                    if results.is_empty() {
                        ui.label(RichText::new("No matching command.").color(pal.muted));
                    }
                });
        });
    if let Some(cmd) = chosen {
        actions.push(Action::Exec(cmd));
        return None;
    }
    if resp.should_close() {
        return None;
    }
    Some(Modal::CommandPalette { query, selected })
}
