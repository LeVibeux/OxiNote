//! Interface of the local Git backup: settings page, "Historique de
//! version" side panel, status bar indicator and confirmation dialogs. The
//! logic lives in [`crate::git_backup`].

use std::hash::{Hash, Hasher};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Vec2};
use ors_core::NotePath;

use super::settings_ui::{heading, row, text_row, toggle_row};
use super::theme::Palette;
use super::{Action, icons};
use crate::commands::Command;
use crate::git_backup::{
    Availability, DiffKind, GitConfirm, Preset, diff_lines, format_timestamp, preset_active,
    relative_time, unix_now,
};
use crate::state::AppState;
use crate::ws_commands::WsCommand;

pub const GIT_BRANCH: &str = "\u{e278}";
pub const HISTORY: &str = "\u{e1a0}";
pub const CLOUD_CHECK: &str = "\u{e1b0}";

/// Requests coming from the explorer menu.
#[derive(Debug, Clone)]
pub enum GitAction {
    Ignore { path: String, is_dir: bool },
    Unignore { path: String, is_dir: bool },
}

pub fn apply(state: &mut AppState, action: GitAction) {
    match action {
        GitAction::Ignore { path, is_dir } => state.git_ignore_path(&path, is_dir),
        GitAction::Unignore { path, is_dir } => state.git_unignore_path(&path, is_dir),
    }
}

fn green(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0x4c, 0xc3, 0x71)
    } else {
        Color32::from_rgb(0x1e, 0x8a, 0x4c)
    }
}

fn red(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0xf0, 0x6c, 0x76)
    } else {
        Color32::from_rgb(0xc0, 0x30, 0x40)
    }
}

fn warning_color() -> Color32 {
    Color32::from_rgb(0xe0, 0x9a, 0x2b)
}

// ---------------------------------------------------------------------------
// Settings page
// ---------------------------------------------------------------------------

pub fn settings_page(ui: &mut egui::Ui, state: &mut AppState) {
    let pal = Palette::of(ui.ctx());
    if state.vault.root().is_none() {
        ui.label(
            RichText::new("Ce vault n'est pas sur le disque : pas de sauvegarde Git.")
                .color(pal.muted),
        );
        return;
    }
    match state.git.avail {
        Availability::Missing => {
            ui.label(
                RichText::new(
                    "Git est introuvable sur cet ordinateur. Installe-le (https://git-scm.com) \
                     puis relance ObsiRust : la sauvegarde Git est désactivée.",
                )
                .color(warning_color()),
            );
            return;
        }
        Availability::Unknown => {
            ui.label(RichText::new("Recherche de Git…").color(pal.muted));
            return;
        }
        Availability::Ready => {}
    }
    if let Some(e) = state.git.init_error.clone() {
        ui.label(RichText::new(e).color(red(ui)));
    }
    let Some(info) = state.git.repo.clone() else {
        if !state.git.checked {
            ui.label(RichText::new("Détection du dépôt…").color(pal.muted));
            return;
        }
        ui.label(
            RichText::new(
                "Ce vault n'est pas encore un dépôt Git. L'initialisation crée un dépôt local \
                 (sans serveur distant), un .gitignore adapté à Obsidian et un premier commit.",
            )
            .color(pal.muted),
        );
        ui.add_space(8.0);
        let busy = state.git.running > 0;
        if ui
            .add_enabled(
                !busy,
                egui::Button::new("Initialiser le dépôt Git du vault"),
            )
            .clicked()
        {
            state.git_init();
        }
        if busy {
            ui.label(RichText::new("Initialisation…").small().color(pal.muted));
        }
        return;
    };

    ui.label(
        RichText::new(format!(
            "{} ({})",
            info.toplevel.display(),
            state.git.version
        ))
        .small()
        .color(pal.muted),
    );
    if info.is_nested() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Le dépôt Git est dans un dossier parent du vault. Seuls les fichiers du vault \
                 (« {} ») sont ajoutés et enregistrés par les sauvegardes.",
                info.prefix
            ))
            .color(warning_color()),
        );
    }

    heading(ui, "Sauvegarde");
    let mut changed = false;
    changed |= toggle_row(
        ui,
        "Sauvegarde automatique",
        "Crée un commit à intervalle régulier s'il y a des modifications.",
        &mut state.git.settings.auto_backup,
    );
    row(
        ui,
        "Intervalle (minutes)",
        "Délai entre deux sauvegardes automatiques.",
        |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut state.git.settings.interval_minutes)
                        .range(1..=1440)
                        .suffix(" min"),
                )
                .changed();
        },
    );
    changed |= toggle_row(
        ui,
        "Sauvegarder à la fermeture",
        "Crée un dernier commit en quittant ObsiRust.",
        &mut state.git.settings.backup_on_exit,
    );
    changed |= text_row(
        ui,
        "Message de commit",
        "{{date}} = YYYY-MM-DD HH:mm, {{numFiles}} = nombre de fichiers modifiés.",
        &mut state.git.settings.message_template,
        crate::git_backup::DEFAULT_TEMPLATE,
    );
    if changed {
        state.git_save_settings();
    }
    let status = status_text(state);
    row(ui, "Sauvegarder maintenant", &status, |ui| {
        let busy = state.git.is_backing_up();
        if ui
            .add_enabled(!busy, egui::Button::new("Sauvegarder"))
            .clicked()
        {
            state.git_backup_now();
        }
    });

    heading(ui, ".gitignore");
    if std::mem::take(&mut state.git.scroll_gitignore) {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    state.git_load_gitignore();
    for p in Preset::ALL {
        let mut on = preset_active(&state.git.gitignore_text, p);
        if ui.checkbox(&mut on, p.label()).changed() {
            let text = crate::git_backup::set_preset(&state.git.gitignore_text, p, on);
            state.git_write_gitignore(text);
        }
    }
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(pal.code_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(4)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut state.git.gitignore_text)
                    .font(egui::TextStyle::Monospace)
                    .frame(egui::Frame::NONE)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .hint_text("Un motif par ligne, comme dans un .gitignore"),
            );
        });
    let dirty = state.git.gitignore_text != state.git.gitignore_saved;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(dirty, egui::Button::new("Enregistrer"))
            .clicked()
        {
            let text = state.git.gitignore_text.clone();
            state.git_write_gitignore(text);
        }
        if ui
            .add_enabled(dirty, egui::Button::new("Annuler les modifications"))
            .clicked()
        {
            state.git.gitignore_text = state.git.gitignore_saved.clone();
        }
        if dirty {
            ui.label(
                RichText::new("modifications non enregistrées")
                    .small()
                    .color(pal.muted),
            );
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "Astuce : clic droit sur un fichier ou un dossier de l'explorateur > « Ignorer dans Git ».",
        )
        .small()
        .color(pal.muted),
    );
}

fn status_text(state: &AppState) -> String {
    let g = &state.git;
    let last = match g.last_commit {
        Some(t) => format!("Dernière sauvegarde : {}.", relative_time(unix_now(), t)),
        None => "Aucune sauvegarde pour l'instant.".to_string(),
    };
    match g.changes {
        Some(0) | None => last,
        Some(n) => format!("{n} modification(s) en attente. {last}"),
    }
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

/// Discreet indicator: "Sauvegardé il y a 3 min" / "5 modifications".
pub fn status_item(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let g = &state.git;
    if !g.usable() {
        return;
    }
    let pal = Palette::of(ui.ctx());
    let now = unix_now();
    let (icon, text, color) = if g.is_backing_up() {
        (GIT_BRANCH, "Sauvegarde…".to_string(), pal.muted)
    } else if let Some(n) = g.changes.filter(|n| *n > 0) {
        let s = if n == 1 {
            "1 modification".to_string()
        } else {
            format!("{n} modifications")
        };
        (GIT_BRANCH, s, warning_color())
    } else if let Some(t) = g.last_commit {
        (
            CLOUD_CHECK,
            format!("Sauvegardé {}", relative_time(now, t)),
            pal.muted,
        )
    } else {
        (GIT_BRANCH, "Git".to_string(), pal.muted)
    };
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        // The parent lays out right to left: the text goes first.
        ui.label(RichText::new(text).size(12.0).color(color));
        ui.label(icons::text(icon, 13.0).color(color));
    });
    let tip = match g.last_commit {
        Some(t) => format!(
            "Dernier commit : {}\nClic : sauvegarder maintenant",
            format_timestamp(t)
        ),
        None => "Aucun commit\nClic : sauvegarder maintenant".to_string(),
    };
    let resp = ui
        .interact(
            inner.response.rect,
            egui::Id::unique("git-status-item"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    if resp.clicked() {
        actions.push(Action::Exec(Command::Ws(WsCommand::GitBackupNow)));
    }
}

// ---------------------------------------------------------------------------
// History panel
// ---------------------------------------------------------------------------

fn hash_of(parts: &[&str], flag: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    flag.hash(&mut h);
    for p in parts {
        p.hash(&mut h);
    }
    h.finish()
}

pub fn history_pane(ui: &mut egui::Ui, state: &mut AppState, path: &NotePath) {
    let pal = Palette::of(ui.ctx());
    match state.git.avail {
        Availability::Missing => {
            ui.label(
                RichText::new("Git est introuvable : historique indisponible.").color(pal.muted),
            );
            return;
        }
        Availability::Unknown => {
            ui.label(RichText::new("Recherche de Git…").color(pal.muted));
            return;
        }
        Availability::Ready => {}
    }
    if state.git.repo.is_none() {
        if !state.git.checked {
            ui.label(RichText::new("Détection du dépôt…").color(pal.muted));
            return;
        }
        ui.label(RichText::new("Ce vault n'est pas un dépôt Git.").color(pal.muted));
        if ui.button("Initialiser la sauvegarde Git…").clicked() {
            state.open_settings(crate::workbench::SettingsPage::GitBackup);
        }
        return;
    }
    state.git_request_history(path);

    ui.horizontal(|ui| {
        ui.label(RichText::new(path.stem()).strong().color(pal.strong));
        if state.git.history.loading {
            ui.label(RichText::new("chargement…").small().color(pal.muted));
        }
    });
    if let Some(e) = &state.git.history.error {
        ui.label(RichText::new(e).small().color(red(ui)));
    }
    if state.git.history.entries.is_empty() {
        if !state.git.history.loading && state.git.history.error.is_none() {
            ui.label(
                RichText::new(
                    "Aucun commit pour cette note. Elle apparaîtra après la prochaine sauvegarde.",
                )
                .color(pal.muted),
            );
        }
        return;
    }

    // ----- commits -----
    let now = unix_now();
    let mut clicked = None;
    let list_h = (ui.available_height() * 0.38).clamp(90.0, 260.0);
    egui::ScrollArea::vertical()
        .id_salt("git-history-list")
        .max_height(list_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let h = &state.git.history;
            for (i, c) in h.entries.iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
                let selected = h.selected == Some(i);
                if selected {
                    ui.painter().rect_filled(rect, 4.0, pal.accent_soft);
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, 4.0, pal.border.gamma_multiply(0.6));
                }
                let clip = ui
                    .painter()
                    .with_clip_rect(rect.shrink2(Vec2::new(4.0, 0.0)));
                clip.text(
                    rect.left_top() + Vec2::new(8.0, 8.0),
                    Align2::LEFT_CENTER,
                    &c.subject,
                    FontId::proportional(13.0),
                    if selected { pal.strong } else { pal.text },
                );
                clip.text(
                    rect.left_top() + Vec2::new(8.0, 26.0),
                    Align2::LEFT_CENTER,
                    format!("{} · {}", relative_time(now, c.time), c.short()),
                    FontId::proportional(11.0),
                    pal.muted,
                );
                resp.clone().on_hover_text(format!(
                    "{}\n{}\n{}",
                    c.subject,
                    format_timestamp(c.time),
                    c.sha
                ));
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    if let Some(i) = clicked {
        state.git_select_commit(i);
    }
    ui.separator();

    // ----- diff -----
    let Some(sel) = state.git.history.selected else {
        return;
    };
    let Some(commit) = state.git.history.entries.get(sel).cloned() else {
        return;
    };
    let mut against_previous = state.git.history.against_previous;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut against_previous, false, "Avec la version actuelle");
        ui.selectable_value(&mut against_previous, true, "Avec le commit précédent");
    });
    state.git.history.against_previous = against_previous;

    let Some((text, prev)) = state.git.history.version.clone() else {
        ui.label(
            RichText::new("Chargement de la version…")
                .small()
                .color(pal.muted),
        );
        return;
    };
    ui.horizontal(|ui| {
        if ui.button("Restaurer cette version").clicked() {
            state.git.confirm = Some(GitConfirm::Restore {
                path: path.clone(),
                short: commit.short().to_string(),
                text: text.clone(),
            });
        }
    });
    let current = state
        .buffers
        .get(path)
        .map(|b| b.text.clone())
        .unwrap_or_default();
    let key = hash_of(&[&commit.sha, &text, &prev, &current], against_previous);
    if state.git.history.diff.as_ref().map(|d| d.0) != Some(key) {
        let lines = if against_previous {
            diff_lines(&prev, &text, 3)
        } else {
            diff_lines(&text, &current, 3)
        };
        state.git.history.diff = Some((key, lines));
    }
    let legend = if against_previous {
        "Changements apportés par ce commit"
    } else {
        "Changements depuis cette version jusqu'à la version actuelle"
    };
    ui.label(RichText::new(legend).small().color(pal.muted));
    let lines = state
        .git
        .history
        .diff
        .as_ref()
        .map(|d| d.1.as_slice())
        .unwrap_or(&[]);
    if lines.is_empty() {
        ui.label(RichText::new("Aucune différence.").color(pal.muted));
        return;
    }
    let (add_fg, del_fg) = (green(ui), red(ui));
    egui::ScrollArea::both()
        .id_salt("git-history-diff")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for l in lines {
                let (prefix, fg, bg) = match l.kind {
                    DiffKind::Added => ("+ ", add_fg, add_fg.gamma_multiply(0.16)),
                    DiffKind::Removed => ("- ", del_fg, del_fg.gamma_multiply(0.16)),
                    DiffKind::Context => ("  ", pal.muted, Color32::TRANSPARENT),
                    DiffKind::Gap => ("⋯ ", pal.faint, Color32::TRANSPARENT),
                };
                let galley = ui.painter().layout_no_wrap(
                    format!("{prefix}{}", l.text),
                    FontId::monospace(12.0),
                    fg,
                );
                let w = ui.available_width().max(galley.size().x + 8.0);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(w, galley.size().y + 2.0), Sense::hover());
                if bg != Color32::TRANSPARENT {
                    ui.painter().rect_filled(rect, 0.0, bg);
                }
                ui.painter()
                    .galley(rect.left_top() + Vec2::new(4.0, 1.0), galley, fg);
            }
        });
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// Confirmation dialogs ("Ne plus suivre", "Restaurer").
pub fn dialogs(ctx: &egui::Context, state: &mut AppState) {
    let Some(confirm) = state.git.confirm.clone() else {
        return;
    };
    let (title, body, ok_label) = match &confirm {
        GitConfirm::Untrack { path, is_dir } => (
            "Ignorer dans Git".to_string(),
            format!(
                "« {path} » est déjà suivi par Git. Le retirer de l'index (git rm --cached) ? \
                 {} reste sur le disque ; il n'apparaîtra plus dans les prochains commits \
                 (l'historique garde ses anciennes versions).",
                if *is_dir { "Le dossier" } else { "Le fichier" }
            ),
            "Retirer de l'index".to_string(),
        ),
        GitConfirm::Restore { short, .. } => (
            "Restaurer cette version".to_string(),
            format!(
                "Remplacer le contenu de la note par la version {short} ? Le texte actuel sera \
                 écrasé (il reste dans l'historique s'il a été sauvegardé)."
            ),
            "Restaurer".to_string(),
        ),
    };
    let mut answer = None;
    let resp = egui::Modal::new(egui::Id::unique("git-confirm")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(6.0);
        ui.label(body);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(ok_label).clicked() {
                answer = Some(true);
            }
            if ui.button("Annuler").clicked() {
                answer = Some(false);
            }
        });
    });
    if answer.is_none() && resp.should_close() {
        answer = Some(false);
    }
    let Some(yes) = answer else {
        return;
    };
    state.git.confirm = None;
    if !yes {
        return;
    }
    match confirm {
        GitConfirm::Untrack { path, .. } => state.git_untrack(&path),
        GitConfirm::Restore { path, text, .. } => state.git_restore_text(&path, text),
    }
}
