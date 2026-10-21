//! Editor logic independent of drawing: text transformations (formatting
//! shortcuts, list continuation, auto-pairing), autocompletion contexts,
//! Live Preview decorations and find/replace.
//!
//! Everything here works on byte offsets into the note text ([`Sel`]); the
//! UI converts to/from egui's char indices with [`char_to_byte`] and
//! [`byte_to_char`].

pub mod blocks;
pub mod completion;
pub mod embeds;
pub mod find;
pub mod fold;
pub mod format;
pub mod heading_nav;
pub mod live;

use std::collections::HashSet;

use oxi_core::NotePath;

use crate::commands::Command;
use crate::state::{AppState, TabId};

/// A selection as byte offsets, `start <= end` (empty = caret).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sel {
    pub start: usize,
    pub end: usize,
}

impl Sel {
    pub fn caret(at: usize) -> Self {
        Self { start: at, end: at }
    }
    pub fn new(a: usize, b: usize) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Byte offset of char index `ci` (clamped to the text length).
pub fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

/// Char index of byte offset `bi` (rounded down to a char boundary).
pub fn byte_to_char(text: &str, bi: usize) -> usize {
    let mut b = bi.min(text.len());
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    text[..b].chars().count()
}

/// Start of the line containing byte `at`.
pub fn line_start(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].rfind('\n').map_or(0, |i| i + 1)
}

/// End of the line containing byte `at` (position of its `\n`, or text end).
pub fn line_end(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// 0-based line number of byte `at`.
pub fn line_of(text: &str, at: usize) -> usize {
    text.as_bytes()[..at.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
}

/// Editor state kept in [`AppState`] (commands are executed by the state
/// but need the cursor, which only the editor widget knows).
#[derive(Debug, Clone, Default)]
pub struct EditorState {
    /// Editing commands waiting for the active editor to apply them.
    pub pending: Vec<Command>,
    /// Tabs whose editing mode differs from the default
    /// (`Settings::live_preview`).
    pub toggled_tabs: HashSet<TabId>,
    /// Folded headings / list items per tab.
    pub folds: std::collections::HashMap<TabId, Vec<fold::FoldKey>>,
    /// (tab, note) pairs whose `[!type]-` callouts were already folded on opening.
    pub callout_seeded: HashSet<(TabId, oxi_core::NotePath)>,
}

impl EditorState {
    /// Is the tab shown in Live Preview (vs. Source mode) when editing?
    pub fn is_live(&self, tab: TabId, default_live: bool) -> bool {
        default_live != self.toggled_tabs.contains(&tab)
    }
}

impl EditorState {
    /// Live Preview for `tab` given the user setting.
    pub fn live(&self, tab: TabId, mode: crate::settings::EditingMode) -> bool {
        self.is_live(tab, mode == crate::settings::EditingMode::LivePreview)
    }
}

/// The three ways Obsidian shows a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    LivePreview,
    Source,
    Reading,
}

impl DisplayMode {
    /// Name shown in the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Self::LivePreview => "Live Preview",
            Self::Source => "Source mode",
            Self::Reading => "Reading view",
        }
    }
}

/// Combine the tab's view mode (editing / reading) and its Live Preview flag.
pub fn display_mode(view: crate::state::ViewMode, live: bool) -> DisplayMode {
    match (view, live) {
        (crate::state::ViewMode::Reading, _) => DisplayMode::Reading,
        (_, true) => DisplayMode::LivePreview,
        (_, false) => DisplayMode::Source,
    }
}

/// Is `cmd` an editing command applied by the editor widget?
pub fn is_editor_command(cmd: Command) -> bool {
    use Command::*;
    matches!(
        cmd,
        ToggleBold
            | ToggleItalic
            | ToggleStrikethrough
            | ToggleHighlight
            | ToggleInlineCode
            | ToggleComment
            | InsertLink
            | InsertWikilink
            | ToggleChecklist
            | ToggleCheckbox
            | ToggleBulletList
            | ToggleNumberedList
            | IndentList
            | OutdentList
            | MoveLineUp
            | MoveLineDown
            | FindInNote
            | ReplaceInNote
            | FoldAll
            | UnfoldAll
    )
}

/// Apply a formatting command to the selection. `None` = not applicable
/// (text unchanged) or not a text command.
pub fn run_command(cmd: Command, text: &mut String, sel: Sel) -> Option<Sel> {
    use Command::*;
    Some(match cmd {
        ToggleBold => format::toggle_wrap(text, sel, "**"),
        ToggleItalic => format::toggle_wrap(text, sel, "*"),
        ToggleStrikethrough => format::toggle_wrap(text, sel, "~~"),
        ToggleHighlight => format::toggle_wrap(text, sel, "=="),
        ToggleInlineCode => format::toggle_wrap(text, sel, "`"),
        ToggleComment => format::toggle_wrap(text, sel, "%%"),
        InsertLink => format::insert_link(text, sel),
        InsertWikilink => format::insert_wikilink(text, sel),
        ToggleChecklist => format::toggle_checklist(text, sel),
        ToggleCheckbox => format::toggle_checkbox(text, sel),
        ToggleBulletList => format::toggle_list(text, sel, false),
        ToggleNumberedList => format::toggle_list(text, sel, true),
        IndentList => format::indent_lines(text, sel),
        OutdentList => return format::outdent_lines(text, sel),
        MoveLineUp => return format::move_lines(text, sel, true),
        MoveLineDown => return format::move_lines(text, sel, false),
        _ => return None,
    })
}

impl AppState {
    /// Display mode of the active note tab.
    pub fn display_mode(&self) -> Option<DisplayMode> {
        let t = self.active_tab()?;
        let n = t.note()?;
        // Modes only exist for Markdown notes (not PDFs, images, ...).
        if !n.path.is_markdown() {
            return None;
        }
        Some(display_mode(
            n.mode,
            self.editor.live(t.id, self.settings.default_editing_mode),
        ))
    }

    /// Put the active note tab in `mode` (remembered per tab).
    pub fn set_display_mode(&mut self, mode: DisplayMode) {
        let default_live =
            self.settings.default_editing_mode == crate::settings::EditingMode::LivePreview;
        let Some(t) = self.active_tab_mut() else {
            return;
        };
        let id = t.id;
        let Some(n) = t.note_mut() else { return };
        if !n.path.is_markdown() {
            return;
        }
        match mode {
            DisplayMode::Reading => n.mode = crate::state::ViewMode::Reading,
            DisplayMode::LivePreview | DisplayMode::Source => {
                n.mode = crate::state::ViewMode::Source;
                if (mode == DisplayMode::LivePreview) == default_live {
                    self.editor.toggled_tabs.remove(&id);
                } else {
                    self.editor.toggled_tabs.insert(id);
                }
            }
        }
    }

    /// Handle an editor command from the palette/shortcuts.
    pub fn editor_command(&mut self, cmd: Command) {
        match cmd {
            Command::ToggleLivePreview => match self.display_mode() {
                Some(DisplayMode::LivePreview) => self.set_display_mode(DisplayMode::Source),
                Some(_) => self.set_display_mode(DisplayMode::LivePreview),
                None => {}
            },
            Command::CycleHeadingNavigator => {
                self.settings.heading_navigator = self.settings.heading_navigator.next();
                self.info(format!(
                    "Heading navigator: {}",
                    self.settings.heading_navigator.label()
                ));
            }
            Command::ToggleLineNumbers => {
                self.settings.show_line_numbers = !self.settings.show_line_numbers;
                self.save_vault_settings();
            }
            c if is_editor_command(c) => {
                // Only notes take editor commands: nothing drains them in a
                // PDF or image tab, even one in reading mode.
                let mode = self
                    .active_tab()
                    .and_then(|t| t.note())
                    .filter(|n| n.path.is_markdown())
                    .map(|n| n.mode);
                // The reading view only takes the find bar.
                let find = matches!(c, Command::FindInNote | Command::ReplaceInNote);
                if mode == Some(crate::state::ViewMode::Source)
                    || (find && mode == Some(crate::state::ViewMode::Reading))
                {
                    self.editor.pending.push(c);
                }
            }
            _ => {}
        }
    }

    /// `useMarkdownLinks` from `.obsidian/app.json`.
    pub fn use_markdown_links(&self) -> bool {
        self.vault
            .app_config()
            .map(|c| c.use_markdown_links)
            .unwrap_or(false)
    }

    /// Copy an attachment (pasted or dropped) into the attachment folder
    /// configured for `note` and index it. Returns its vault path.
    pub fn add_attachment(
        &mut self,
        note: &NotePath,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<NotePath, String> {
        let path = self
            .vault
            .attachment_path(Some(note), file_name)
            .map_err(|e| e.to_string())?;
        self.vault
            .write_bytes(&path, bytes)
            .map_err(|e| e.to_string())?;
        self.index.update_file(self.vault.as_read(), &path);
        self.refresh_files();
        self.generation += 1;
        Ok(path)
    }

    /// Text to insert to link (`embed`) `target` from `from`, honouring
    /// `useMarkdownLinks`.
    pub fn link_markup(&self, target: &NotePath, from: &NotePath, embed: bool) -> String {
        let text = self.index.link_text_for(target, from);
        let bang = if embed { "!" } else { "" };
        if self.use_markdown_links() {
            let dest = if target.is_markdown() {
                format!("{text}.md")
            } else {
                text.clone()
            };
            let name = if target.is_markdown() {
                target.stem().to_string()
            } else {
                target.file_name().to_string()
            };
            format!(
                "{bang}[{name}]({})",
                oxi_markdown::encode_markdown_target(&dest)
            )
        } else {
            format!("{bang}[[{text}]]")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_byte_conversions() {
        let t = "héllo\nwörld";
        assert_eq!(char_to_byte(t, 2), 3);
        assert_eq!(byte_to_char(t, 3), 2);
        assert_eq!(byte_to_char(t, 2), 1); // inside 'é'
        assert_eq!(char_to_byte(t, 100), t.len());
        assert_eq!(line_start(t, 10), 7);
        assert_eq!(line_end(t, 1), 6);
        assert_eq!(line_of(t, 10), 1);
    }

    #[test]
    fn run_command_wraps_selection() {
        let mut t = "un mot".to_string();
        let s = run_command(Command::ToggleBold, &mut t, Sel::new(3, 6));
        assert_eq!(t, "un **mot**");
        assert_eq!(s, Some(Sel::new(5, 8)));
        assert_eq!(run_command(Command::Save, &mut t, Sel::caret(0)), None);
    }

    #[test]
    fn live_preview_toggle_per_tab() {
        let mut e = EditorState::default();
        assert!(e.is_live(1, true));
        e.toggled_tabs.insert(1);
        assert!(!e.is_live(1, true));
        assert!(e.is_live(1, false));
    }
}
