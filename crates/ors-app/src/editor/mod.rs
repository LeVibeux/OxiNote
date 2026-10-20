//! Editor logic independent of drawing: text transformations (formatting
//! shortcuts, list continuation, auto-pairing), autocompletion contexts,
//! Live Preview decorations and find/replace.
//!
//! Everything here works on byte offsets into the note text ([`Sel`]); the
//! UI converts to/from egui's char indices with [`char_to_byte`] and
//! [`byte_to_char`].

pub mod completion;
pub mod find;
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
    /// Line-number gutter; `None` = read `showLineNumber` from app.json.
    pub line_numbers: Option<bool>,
}

impl EditorState {
    /// Is the tab shown in Live Preview (vs. Source mode) when editing?
    pub fn is_live(&self, tab: TabId, default_live: bool) -> bool {
        default_live != self.toggled_tabs.contains(&tab)
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

impl AppState {
    /// Handle an editor command from the palette/shortcuts.
    pub fn editor_command(&mut self, cmd: Command) {
        match cmd {
            Command::ToggleLivePreview => {
                if let Some(id) = self.active_tab().filter(|t| t.note().is_some()).map(|t| t.id)
                    && !self.editor.toggled_tabs.remove(&id)
                {
                    self.editor.toggled_tabs.insert(id);
                }
            }
            Command::ToggleLineNumbers => {
                let on = self.show_line_numbers();
                self.editor.line_numbers = Some(!on);
            }
            c if is_editor_command(c) => {
                if self.active_note().is_some() {
                    self.editor.pending.push(c);
                }
            }
            _ => {}
        }
    }

    /// Line numbers on? (`showLineNumber` in `.obsidian/app.json` unless
    /// toggled by the user.)
    pub fn show_line_numbers(&mut self) -> bool {
        if let Some(v) = self.editor.line_numbers {
            return v;
        }
        let v = self
            .vault
            .app_config()
            .ok()
            .and_then(|c| c.extra.get("showLineNumber").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        self.editor.line_numbers = Some(v);
        v
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
        assert_eq!(line_start(t, 9), 7);
        assert_eq!(line_end(t, 2), 6);
        assert_eq!(line_of(t, 9), 1);
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
