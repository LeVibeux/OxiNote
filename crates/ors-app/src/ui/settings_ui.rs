//! Settings window (Ctrl+,), laid out like Obsidian's: a list of
//! categories on the left, the options of the selected page on the right.

use eframe::egui::{self, Align, Color32, Key, Layout, RichText, Sense, Stroke, Vec2};
use ors_vault::TrashMode;

use super::icons;
use super::theme::Palette;
use crate::commands::{Command, CommandRegistry};
use crate::hotkeys;
use crate::plugins::CorePlugin;
use crate::settings::ThemeChoice;
use crate::state::AppState;
use crate::workbench::{SettingsPage, SettingsWindow};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 520.0;

/// Draw the window if it is open. Persists every change immediately.
pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut win) = state.ws.settings_window.take() else {
        return;
    };
    let pal = Palette::of(ctx);
    let mut close = false;

    // Hotkey capture happens before anything else sees the keys.
    if let Some(cmd) = win.capture {
        capture_hotkey(ctx, state, &mut win, cmd);
    }

    let modal = egui::Modal::new(egui::Id::unique("settings-window"))
        .backdrop_color(Color32::from_black_alpha(150))
        .frame(
            egui::Frame::new()
                .fill(pal.bg_primary)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(10),
        )
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.set_height(HEIGHT);
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
            ui.horizontal_top(|ui| {
                // ----- categories -----
                egui::Frame::new()
                    .fill(pal.bg_secondary)
                    .corner_radius(egui::CornerRadius {
                        nw: 10,
                        sw: 10,
                        ne: 0,
                        se: 0,
                    })
                    .inner_margin(egui::Margin::symmetric(8, 10))
                    .show(ui, |ui| {
                        ui.set_width(176.0);
                        ui.set_height(HEIGHT - 20.0);
                        ui.vertical(|ui| categories(ui, state, &mut win));
                    });
                // ----- page -----
                ui.vertical(|ui| {
                    ui.set_width(WIDTH - 200.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(page_title(win.page))
                                .size(18.0)
                                .strong()
                                .color(pal.strong),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if super::explorer_ui::icon_button(ui, "×", "Close (Esc)", false)
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt(("settings-page", win.page as u8))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            page(ui, state, &mut win);
                        });
                });
            });
        });
    if (modal.should_close() && win.capture.is_none()) || close {
        // Flush anything still pending, then drop the window.
        return;
    }
    state.ws.settings_window = Some(win);
}

fn page_title(p: SettingsPage) -> &'static str {
    match p {
        SettingsPage::Editor => "Editor",
        SettingsPage::FilesLinks => "Files and links",
        SettingsPage::Appearance => "Appearance",
        SettingsPage::Hotkeys => "Hotkeys",
        SettingsPage::CorePlugins => "Core plugins",
        SettingsPage::DailyNotes => "Daily notes",
        SettingsPage::Templates => "Templates",
        SettingsPage::About => "About",
    }
}

fn categories(ui: &mut egui::Ui, state: &AppState, win: &mut SettingsWindow) {
    let pal = Palette::of(ui.ctx());
    let section = |ui: &mut egui::Ui, text: &str| {
        ui.add_space(6.0);
        ui.label(RichText::new(text).small().color(pal.faint));
    };
    let item = |ui: &mut egui::Ui, win: &mut SettingsWindow, p: SettingsPage, icon: &str| {
        let selected = win.page == p;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
        if selected {
            ui.painter().rect_filled(rect, 5.0, pal.accent_soft);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 5.0, pal.border.gamma_multiply(0.6));
        }
        ui.painter().text(
            egui::pos2(rect.left() + 14.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            icons::font(15.0),
            if selected { pal.accent } else { pal.muted },
        );
        ui.painter().text(
            egui::pos2(rect.left() + 30.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            page_title(p),
            egui::FontId::proportional(13.5),
            if selected { pal.strong } else { pal.text },
        );
        if resp.clicked() {
            win.page = p;
            win.capture = None;
        }
    };
    section(ui, "Options");
    item(ui, win, SettingsPage::Editor, icons::NOTE_PENCIL);
    item(ui, win, SettingsPage::FilesLinks, icons::LINK);
    item(ui, win, SettingsPage::Appearance, icons::BRUSH);
    item(ui, win, SettingsPage::Hotkeys, icons::KEYBOARD);
    item(ui, win, SettingsPage::CorePlugins, icons::PLUG);
    if state.plugin_enabled(CorePlugin::DailyNotes) || state.plugin_enabled(CorePlugin::Templates) {
        section(ui, "Core plugins");
    }
    if state.plugin_enabled(CorePlugin::DailyNotes) {
        item(ui, win, SettingsPage::DailyNotes, icons::CALENDAR);
    }
    if state.plugin_enabled(CorePlugin::Templates) {
        item(ui, win, SettingsPage::Templates, icons::FILE_TEXT);
    }
    section(ui, "");
    item(ui, win, SettingsPage::About, icons::NOTE);
}

fn page(ui: &mut egui::Ui, state: &mut AppState, win: &mut SettingsWindow) {
    // A hidden plugin's page cannot stay selected.
    if (win.page == SettingsPage::DailyNotes && !state.plugin_enabled(CorePlugin::DailyNotes))
        || (win.page == SettingsPage::Templates && !state.plugin_enabled(CorePlugin::Templates))
    {
        win.page = SettingsPage::Editor;
    }
    match win.page {
        SettingsPage::Editor => editor_page(ui, state),
        SettingsPage::FilesLinks => files_page(ui, state, win),
        SettingsPage::Appearance => appearance_page(ui, state),
        SettingsPage::Hotkeys => hotkeys_page(ui, state, win),
        SettingsPage::CorePlugins => plugins_page(ui, state),
        SettingsPage::DailyNotes => daily_page(ui, state, win),
        SettingsPage::Templates => templates_page(ui, state, win),
        SettingsPage::About => about_page(ui, state),
    }
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

/// A setting: name and description on the left, control on the right.
fn row(ui: &mut egui::Ui, name: &str, desc: &str, control: impl FnOnce(&mut egui::Ui)) {
    let pal = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() * 0.58);
            ui.label(RichText::new(name).color(pal.strong));
            if !desc.is_empty() {
                ui.label(RichText::new(desc).small().color(pal.muted));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(2.0);
    ui.separator();
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong().size(15.0).color(pal.strong));
    ui.separator();
}

/// Obsidian-style toggle switch. Returns true when changed.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> bool {
    let pal = Palette::of(ui.ctx());
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    let mut changed = false;
    if resp.clicked() {
        *on = !*on;
        changed = true;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = if *on { pal.accent } else { pal.border };
    ui.painter().rect_filled(rect, 11.0, bg);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    changed
}

fn toggle_row(ui: &mut egui::Ui, name: &str, desc: &str, on: &mut bool) -> bool {
    let mut changed = false;
    row(ui, name, desc, |ui| changed = toggle(ui, on));
    changed
}

fn text_row(ui: &mut egui::Ui, name: &str, desc: &str, text: &mut String, hint: &str) -> bool {
    let mut changed = false;
    row(ui, name, desc, |ui| {
        changed = ui
            .add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .desired_width(210.0),
            )
            .changed();
    });
    changed
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

fn editor_page(ui: &mut egui::Ui, state: &mut AppState) {
    let mut changed = false;
    let s = &mut state.settings;
    row(
        ui,
        "Default editing mode",
        "Live Preview shows formatting while you edit; Source mode shows the raw Markdown.",
        |ui| {
            egui::ComboBox::from_id_salt("edit-mode")
                .selected_text(if s.live_preview {
                    "Live Preview"
                } else {
                    "Source mode"
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut s.live_preview, true, "Live Preview")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut s.live_preview, false, "Source mode")
                        .changed();
                });
        },
    );
    row(
        ui,
        "Default view for new tabs",
        "Whether notes open in editing or in reading view.",
        |ui| {
            egui::ComboBox::from_id_salt("view-mode")
                .selected_text(if s.default_reading_view {
                    "Reading view"
                } else {
                    "Editing view"
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut s.default_reading_view, false, "Editing view")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut s.default_reading_view, true, "Reading view")
                        .changed();
                });
        },
    );
    changed |= toggle_row(
        ui,
        "Readable line length",
        "Limit the width of lines so that text is easier to read.",
        &mut s.readable_line_length,
    );
    changed |= toggle_row(
        ui,
        "Show line numbers",
        "Show line numbers in the gutter of the editor.",
        &mut s.show_line_numbers,
    );
    changed |= toggle_row(
        ui,
        "Spellcheck",
        "Underline misspelled words while you type.",
        &mut s.spellcheck,
    );
    row(
        ui,
        "Autosave delay",
        "Seconds of inactivity before a note is saved.",
        |ui| {
            ui.add(egui::Slider::new(&mut s.autosave_delay, 0.5..=10.0).suffix(" s"));
        },
    );
    if changed {
        state.save_vault_settings();
    }
}

fn files_page(ui: &mut egui::Ui, state: &mut AppState, win: &mut SettingsWindow) {
    let mut changed = false;
    let app = &mut win.app;
    row(
        ui,
        "Default location for new notes",
        "Where newly created notes are placed.",
        |ui| {
            let cur = app
                .new_file_location
                .clone()
                .unwrap_or_else(|| "root".into());
            let label = match cur.as_str() {
                "current" => "Same folder as current file",
                "folder" => "In the folder specified below",
                _ => "Vault folder",
            };
            egui::ComboBox::from_id_salt("new-file-loc")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (v, l) in [
                        ("root", "Vault folder"),
                        ("current", "Same folder as current file"),
                        ("folder", "In the folder specified below"),
                    ] {
                        if ui.selectable_label(cur == v, l).clicked() {
                            app.new_file_location = Some(v.into());
                            changed = true;
                        }
                    }
                });
        },
    );
    if app.new_file_location.as_deref() == Some("folder") {
        let mut folder = app.new_file_folder_path.clone().unwrap_or_default();
        if text_row(
            ui,
            "Folder to create new notes in",
            "Example: Inbox",
            &mut folder,
            "Folder name",
        ) {
            app.new_file_folder_path = Some(folder);
            changed = true;
        }
    }
    let mut att = app.attachment_folder_path.clone().unwrap_or_default();
    if text_row(
        ui,
        "Default location for new attachments",
        "A folder name, \"/\" for the vault root, or \"./\" for the folder of the current note.",
        &mut att,
        "/",
    ) {
        app.attachment_folder_path = if att.is_empty() { None } else { Some(att) };
        changed = true;
    }
    row(
        ui,
        "New link format",
        "What path is written when a link is inserted.",
        |ui| {
            let cur = app
                .new_link_format
                .clone()
                .unwrap_or_else(|| "shortest".into());
            let label = match cur.as_str() {
                "relative" => "Relative path to file",
                "absolute" => "Absolute path in vault",
                _ => "Shortest path when possible",
            };
            egui::ComboBox::from_id_salt("link-format")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (v, l) in [
                        ("shortest", "Shortest path when possible"),
                        ("relative", "Relative path to file"),
                        ("absolute", "Absolute path in vault"),
                    ] {
                        if ui.selectable_label(cur == v, l).clicked() {
                            app.new_link_format = Some(v.into());
                            changed = true;
                        }
                    }
                });
        },
    );
    let mut wiki = !app.use_markdown_links;
    if toggle_row(
        ui,
        "Use [[Wikilinks]]",
        "Create [[Wikilinks]] instead of [Markdown](links).",
        &mut wiki,
    ) {
        app.use_markdown_links = !wiki;
        changed = true;
    }
    changed |= toggle_row(
        ui,
        "Automatically update internal links",
        "Update links when a file is renamed or moved.",
        &mut app.always_update_links,
    );
    row(
        ui,
        "Deleted files",
        "What happens to a file you delete.",
        |ui| {
            let label = match app.trash_mode {
                TrashMode::System => "Move to system trash",
                TrashMode::Local => "Move to Obsidian trash (.trash)",
                TrashMode::Permanent => "Permanently delete",
            };
            egui::ComboBox::from_id_salt("trash-mode")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (v, l) in [
                        (TrashMode::System, "Move to system trash"),
                        (TrashMode::Local, "Move to Obsidian trash (.trash)"),
                        (TrashMode::Permanent, "Permanently delete"),
                    ] {
                        if ui.selectable_label(app.trash_mode == v, l).clicked() {
                            app.trash_mode = v;
                            changed = true;
                        }
                    }
                });
        },
    );
    let pal = Palette::of(ui.ctx());
    ui.label(RichText::new("Excluded files").color(pal.strong));
    ui.label(
        RichText::new(
            "One folder or path per line. Excluded files are hidden from search and the graph.",
        )
        .small()
        .color(pal.muted),
    );
    if ui
        .add(
            egui::TextEdit::multiline(&mut win.excluded)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        win.app.user_ignore_filters = win
            .excluded
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        changed = true;
    }
    if changed && let Err(e) = state.vault.save_app_config(&win.app) {
        state.error(format!("Cannot save settings: {e}"));
    }
}

fn appearance_page(ui: &mut egui::Ui, state: &mut AppState) {
    let mut changed = false;
    let pal = Palette::of(ui.ctx());
    let s = &mut state.settings;
    row(
        ui,
        "Base color scheme",
        "Choose Obsidian's default color scheme.",
        |ui| {
            egui::ComboBox::from_id_salt("theme")
                .selected_text(match s.theme {
                    ThemeChoice::Dark => "Dark",
                    ThemeChoice::Light => "Light",
                    ThemeChoice::System => "Adapt to system",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut s.theme, ThemeChoice::Dark, "Dark")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut s.theme, ThemeChoice::Light, "Light")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut s.theme, ThemeChoice::System, "Adapt to system")
                        .changed();
                });
        },
    );
    row(
        ui,
        "Accent color",
        "Used for links, selection and highlights.",
        |ui| {
            if s.accent.is_some() && ui.button("Reset").clicked() {
                s.accent = None;
                changed = true;
            }
            let current = s.accent.unwrap_or_else(|| {
                let c = pal.accent;
                [c.r(), c.g(), c.b()]
            });
            let mut rgb = current;
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                s.accent = Some(rgb);
                changed = true;
            }
        },
    );
    heading(ui, "Font");
    row(ui, "Font size", "Base size of the text in notes.", |ui| {
        changed |= ui
            .add(egui::Slider::new(&mut s.font_size, 10.0..=30.0).step_by(1.0))
            .changed();
    });
    row(
        ui,
        "Interface font size",
        "Size of the text in sidebars, tabs and menus.",
        |ui| {
            changed |= ui
                .add(egui::Slider::new(&mut s.ui_font_size, 11.0..=20.0).step_by(0.5))
                .changed();
        },
    );
    row(
        ui,
        "Text font",
        "Font of the interface and of notes.",
        |ui| {
            egui::ComboBox::from_id_salt("text-font")
                .selected_text(if s.text_font == "Inter" {
                    "Inter"
                } else {
                    "System default"
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(s.text_font == "Inter", "Inter")
                        .clicked()
                    {
                        s.text_font = "Inter".into();
                        changed = true;
                    }
                    if ui
                        .selectable_label(s.text_font != "Inter", "System default")
                        .clicked()
                    {
                        s.text_font = "Default".into();
                        changed = true;
                    }
                });
        },
    );
    changed |= toggle_row(
        ui,
        "Monospace font in the editor",
        "Use a monospace font when editing notes.",
        &mut s.monospace_editor,
    );
    heading(ui, "Interface");
    changed |= toggle_row(
        ui,
        "Show ribbon",
        "Show the icon bar on the left edge of the window.",
        &mut s.show_ribbon,
    );
    if changed {
        state.save_vault_settings();
    }
}

fn hotkeys_page(ui: &mut egui::Ui, state: &mut AppState, win: &mut SettingsWindow) {
    let pal = Palette::of(ui.ctx());
    ui.add(
        egui::TextEdit::singleline(&mut win.hotkey_filter)
            .hint_text("Filter commands…")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    let filter = win.hotkey_filter.to_lowercase();
    let defaults = CommandRegistry::default();
    let rows: Vec<(Command, &'static str)> = state
        .commands
        .specs()
        .iter()
        .filter(|s| !state.commands.is_hidden(s.command))
        .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
        .map(|s| (s.command, s.name))
        .collect();
    let mut set: Option<(Command, Option<egui::KeyboardShortcut>)> = None;
    for (cmd, name) in rows {
        let current = state.commands.shortcut(cmd);
        let default = defaults.shortcut(cmd);
        let capturing = win.capture == Some(cmd);
        let conflicts = hotkeys::conflicts(&state.commands, cmd);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width() - 260.0).max(100.0), 22.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(name).color(pal.text)).truncate());
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let differs =
                    current.map(|c| hotkeys::display(&c)) != default.map(|d| hotkeys::display(&d));
                if differs
                    && super::explorer_ui::icon_button(ui, icons::CLOCK, "Restore default", false)
                        .clicked()
                {
                    set = Some((cmd, default));
                }
                if current.is_some()
                    && super::explorer_ui::icon_button(ui, "×", "Remove hotkey", false).clicked()
                {
                    set = Some((cmd, None));
                }
                let label = if capturing {
                    "Press a hotkey…".to_string()
                } else {
                    current
                        .map(|c| hotkeys::display(&c))
                        .unwrap_or_else(|| "Blank".into())
                };
                let color = if capturing {
                    pal.accent
                } else if current.is_some() {
                    pal.strong
                } else {
                    pal.faint
                };
                let b = egui::Button::new(RichText::new(label).color(color))
                    .fill(pal.bg_secondary)
                    .stroke(Stroke::new(
                        1.0,
                        if capturing { pal.accent } else { pal.border },
                    ));
                if ui.add(b).clicked() {
                    win.capture = if capturing { None } else { Some(cmd) };
                }
            });
        });
        if !conflicts.is_empty() {
            let names: Vec<&str> = conflicts
                .iter()
                .filter_map(|c| state.commands.get(*c).map(|s| s.name))
                .collect();
            ui.label(
                RichText::new(format!("Also used by: {}", names.join(", ")))
                    .small()
                    .color(Color32::from_rgb(0xe9, 0x31, 0x47)),
            );
        }
        ui.separator();
    }
    if let Some((cmd, sc)) = set {
        state.commands.set_shortcut(cmd, sc);
        state.save_hotkeys();
    }
}

/// While a hotkey is being captured: the next non-modifier key press (with
/// the modifiers held) becomes the shortcut; Escape cancels.
fn capture_hotkey(
    ctx: &egui::Context,
    state: &mut AppState,
    win: &mut SettingsWindow,
    cmd: Command,
) {
    let mut result: Option<Option<egui::KeyboardShortcut>> = None;
    ctx.input_mut(|i| {
        let found = i.events.iter().find_map(|e| match e {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => Some((*key, *modifiers)),
            _ => None,
        });
        if let Some((key, mods)) = found {
            if key == Key::Escape {
                result = Some(None);
            } else {
                let mods = hotkeys::normalize(mods);
                result = Some(Some(egui::KeyboardShortcut::new(mods, key)));
            }
            i.events
                .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
        }
    });
    match result {
        Some(Some(sc)) => {
            state.commands.set_shortcut(cmd, Some(sc));
            state.save_hotkeys();
            win.capture = None;
        }
        Some(None) => win.capture = None,
        None => {}
    }
}

fn plugins_page(ui: &mut egui::Ui, state: &mut AppState) {
    let pal = Palette::of(ui.ctx());
    ui.label(
        RichText::new("Core plugins can be turned off to hide their commands and views.")
            .color(pal.muted),
    );
    ui.add_space(4.0);
    for p in CorePlugin::ALL {
        let mut on = state.plugin_enabled(p);
        row(ui, p.name(), p.description(), |ui| {
            if toggle(ui, &mut on) {
                state.set_plugin(p, on);
            }
        });
    }
}

fn daily_page(ui: &mut egui::Ui, state: &mut AppState, win: &mut SettingsWindow) {
    let mut changed = false;
    let d = &mut win.daily;
    changed |= text_row(
        ui,
        "Date format",
        "Moment.js format of the note name, e.g. YYYY-MM-DD.",
        &mut d.format,
        "YYYY-MM-DD",
    );
    changed |= text_row(
        ui,
        "New file location",
        "Folder where new daily notes are created.",
        &mut d.folder,
        "Vault root",
    );
    let mut tpl = d.template.clone().unwrap_or_default();
    if text_row(
        ui,
        "Template file location",
        "Note whose content is used for new daily notes.",
        &mut tpl,
        "Templates/Daily",
    ) {
        d.template = if tpl.trim().is_empty() {
            None
        } else {
            Some(tpl)
        };
        changed = true;
    }
    if changed && let Err(e) = state.vault.save_daily_notes_config(&win.daily) {
        state.error(format!("Cannot save daily notes settings: {e}"));
    }
}

fn templates_page(ui: &mut egui::Ui, state: &mut AppState, win: &mut SettingsWindow) {
    let mut changed = false;
    let t = &mut win.templates;
    let mut folder = t.folder.clone().unwrap_or_default();
    if text_row(
        ui,
        "Template folder location",
        "Notes in this folder are offered by \"Insert template\".",
        &mut folder,
        "Templates",
    ) {
        t.folder = if folder.trim().is_empty() {
            None
        } else {
            Some(folder)
        };
        changed = true;
    }
    let mut date = t.date_format.clone().unwrap_or_default();
    if text_row(
        ui,
        "Date format",
        "Format of {{date}} (Moment.js).",
        &mut date,
        "YYYY-MM-DD",
    ) {
        t.date_format = if date.trim().is_empty() {
            None
        } else {
            Some(date)
        };
        changed = true;
    }
    let mut time = t.time_format.clone().unwrap_or_default();
    if text_row(
        ui,
        "Time format",
        "Format of {{time}} (Moment.js).",
        &mut time,
        "HH:mm",
    ) {
        t.time_format = if time.trim().is_empty() {
            None
        } else {
            Some(time)
        };
        changed = true;
    }
    if changed && let Err(e) = state.vault.save_templates_config(&win.templates) {
        state.error(format!("Cannot save templates settings: {e}"));
    }
}

fn about_page(ui: &mut egui::Ui, state: &AppState) {
    let pal = Palette::of(ui.ctx());
    ui.label(
        RichText::new("ObsiRust")
            .size(20.0)
            .strong()
            .color(pal.strong),
    );
    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(pal.muted));
    ui.add_space(6.0);
    ui.label("A Rust reimplementation of Obsidian, compatible with existing vaults.");
    ui.add_space(6.0);
    ui.label(RichText::new(format!("Vault: {}", state.vault.name())).color(pal.text));
    if let Some(root) = state.vault.root() {
        ui.label(
            RichText::new(root.display().to_string())
                .small()
                .color(pal.faint),
        );
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new("Fonts: Inter (SIL OFL), Phosphor icons (MIT).")
            .small()
            .color(pal.faint),
    );
}
