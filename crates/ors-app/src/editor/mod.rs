//! Editor logic independent of drawing: text transformations (formatting
//! shortcuts, list continuation, auto-pairing), autocompletion contexts,
//! Live Preview decorations and find/replace.
//!
//! Everything here works on byte offsets into the note text ([`Sel`]); the
//! UI converts to/from egui's char indices with [`char_to_byte`] and
//! [`byte_to_char`].

pub mod completion;
pub mod embeds;
pub mod find;
pub mod fold;
pub mod format;
pub mod live;

use std::collections::HashSet;

use ors_core::NotePath;

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
    /// Handle an editor command from the palette/shortcuts.
    pub fn editor_command(&mut self, cmd: Command) {
        match cmd {
            Command::ToggleLivePreview => {
                if let Some(id) = self
                    .active_tab()
                    .filter(|t| t.note().is_some())
                    .map(|t| t.id)
                    && !self.editor.toggled_tabs.remove(&id)
                {
                    self.editor.toggled_tabs.insert(id);
                }
            }
            Command::ToggleLineNumbers => {
                self.settings.show_line_numbers = !self.settings.show_line_numbers;
                self.save_vault_settings();
            }
            c if is_editor_command(c) => {
                let editing = self
                    .active_tab()
                    .and_then(|t| t.note())
                    .is_some_and(|n| n.mode == crate::state::ViewMode::Source);
                if editing {
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
                ors_markdown::encode_markdown_target(&dest)
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
