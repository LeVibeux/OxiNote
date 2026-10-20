//! Central command registry: the command palette, keyboard shortcuts and
//! menus all go through it.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

use crate::util::fuzzy_score;
use crate::ws_commands::WsCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
pub enum Command {
    QuickSwitcher,
    CommandPalette,
    NewNote,
    NewFolder,
    OpenDailyNote,
    ToggleMode,
    OpenGraph,
    OpenLocalGraph,
    SearchVault,
    ShowTags,
    ShowFiles,
    RenameNote,
    DeleteNote,
    Save,
    CloseTab,
    NextTab,
    PrevTab,
    NavigateBack,
    NavigateForward,
    ToggleLeftSidebar,
    ToggleRightSidebar,
    ReloadVault,
    ToggleTheme,
    ToggleMonospace,
    SwitchVault,
    /// Workspace / file management / settings commands (see
    /// [`crate::ws_commands`]).
    Ws(WsCommand),
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub command: Command,
    pub name: &'static str,
    pub shortcut: Option<KeyboardShortcut>,
}

pub struct CommandRegistry {
    specs: Vec<CommandSpec>,
    /// Commands of disabled core plugins (not in the palette, no hotkey).
    hidden: std::collections::HashSet<Command>,
}

const fn sc(modifiers: Modifiers, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(modifiers, key))
}

const CMD_SHIFT: Modifiers = Modifiers {
    alt: false,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: true,
};

impl Default for CommandRegistry {
    fn default() -> Self {
        use Command::*;
        let c = Modifiers::COMMAND;
        let specs = vec![
            spec(
                QuickSwitcher,
                "Quick switcher: Open quick switcher",
                sc(c, Key::O),
            ),
            spec(
                CommandPalette,
                "Command palette: Open command palette",
                sc(c, Key::P),
            ),
            spec(NewNote, "Create new note", sc(c, Key::N)),
            spec(NewFolder, "Create new folder", None),
            spec(
                OpenDailyNote,
                "Daily notes: Open today's daily note",
                sc(c, Key::D),
            ),
            spec(ToggleMode, "Toggle reading view", sc(c, Key::E)),
            spec(OpenGraph, "Graph view: Open graph view", sc(c, Key::G)),
            spec(OpenLocalGraph, "Graph view: Open local graph", None),
            spec(
                SearchVault,
                "Search: Search in all files",
                sc(CMD_SHIFT, Key::F),
            ),
            spec(ShowTags, "Tags: Show tags", None),
            spec(ShowFiles, "Files: Show file explorer", None),
            spec(RenameNote, "Rename file", sc(Modifiers::NONE, Key::F2)),
            spec(DeleteNote, "Delete current file", None),
            spec(Save, "Save current file", sc(c, Key::S)),
            spec(CloseTab, "Close current tab", sc(c, Key::W)),
            spec(NextTab, "Go to next tab", sc(c, Key::Tab)),
            spec(PrevTab, "Go to previous tab", sc(CMD_SHIFT, Key::Tab)),
            spec(
                NavigateBack,
                "Navigate back",
                sc(Modifiers::ALT, Key::ArrowLeft),
            ),
            spec(
                NavigateForward,
                "Navigate forward",
                sc(Modifiers::ALT, Key::ArrowRight),
            ),
            spec(ToggleLeftSidebar, "Toggle left sidebar", None),
            spec(ToggleRightSidebar, "Toggle right sidebar", None),
            spec(ReloadVault, "Reload vault (rebuild index)", None),
            spec(ToggleTheme, "Appearance: Toggle dark/light mode", None),
            spec(ToggleMonospace, "Editor: Toggle monospace font", None),
            spec(SwitchVault, "Open another vault", None),
        ];
        let mut specs = specs;
        specs.extend(crate::ws_commands::specs());
        Self {
            specs,
            hidden: Default::default(),
        }
    }
}

fn spec(command: Command, name: &'static str, shortcut: Option<KeyboardShortcut>) -> CommandSpec {
    CommandSpec {
        command,
        name,
        shortcut,
    }
}

impl CommandRegistry {
    #[cfg(test)]
    pub fn all(&self) -> &[CommandSpec] {
        &self.specs
    }

    /// Every registered command (including hidden ones).
    pub fn specs(&self) -> &[CommandSpec] {
        &self.specs
    }

    /// Rebind (or unbind with `None`) a command's shortcut.
    pub fn set_shortcut(&mut self, command: Command, shortcut: Option<KeyboardShortcut>) {
        if let Some(s) = self.specs.iter_mut().find(|s| s.command == command) {
            s.shortcut = shortcut;
        }
    }

    pub fn set_hidden(&mut self, hidden: impl IntoIterator<Item = Command>) {
        self.hidden = hidden.into_iter().collect();
    }

    pub fn is_hidden(&self, command: Command) -> bool {
        self.hidden.contains(&command)
    }

    pub fn get(&self, command: Command) -> Option<&CommandSpec> {
        self.specs.iter().find(|s| s.command == command)
    }

    pub fn shortcut(&self, command: Command) -> Option<KeyboardShortcut> {
        self.get(command).and_then(|s| s.shortcut)
    }

    /// Commands matching `query`, best first.
    pub fn filter(&self, query: &str) -> Vec<&CommandSpec> {
        let mut scored: Vec<(i64, usize, &CommandSpec)> = self
            .specs
            .iter()
            .enumerate()
            .filter(|(_, s)| !self.hidden.contains(&s.command))
            .filter_map(|(i, s)| fuzzy_score(query, s.name).map(|sc| (sc, i, s)))
            .collect();
        if !query.trim().is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        scored.into_iter().map(|(_, _, s)| s).collect()
    }

    /// Commands whose shortcut matches the pressed key. More specific
    /// shortcuts (more modifiers) are listed first so that Ctrl+Shift+Tab
    /// wins over Ctrl+Tab.
    pub fn by_specificity(&self) -> Vec<(KeyboardShortcut, Command)> {
        let mut v: Vec<(KeyboardShortcut, Command)> = self
            .specs
            .iter()
            .filter(|s| !self.hidden.contains(&s.command))
            .filter_map(|s| s.shortcut.map(|k| (k, s.command)))
            .collect();
        v.sort_by_key(|(k, _)| {
            std::cmp::Reverse(
                k.modifiers.shift as u8 + k.modifiers.alt as u8 + k.modifiers.command as u8,
            )
        });
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_unique_commands_and_shortcuts() {
        let r = CommandRegistry::default();
        let mut seen = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for s in r.all() {
            assert!(seen.insert(s.command), "duplicate {:?}", s.command);
            if let Some(k) = s.shortcut {
                assert!(keys.insert(format!("{k:?}")), "duplicate shortcut {k:?}");
            }
        }
        assert_eq!(
            r.shortcut(Command::ToggleMode),
            Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::E))
        );
    }

    #[test]
    fn filtering() {
        let r = CommandRegistry::default();
        let res = r.filter("daily");
        assert!(
            res.iter()
                .take(3)
                .any(|s| s.command == Command::OpenDailyNote)
        );
        assert_eq!(r.filter("").len(), r.all().len());
        assert!(r.filter("zzzzqq").is_empty());
        let res = r.filter("graph");
        assert!(
            res.iter()
                .take(2)
                .all(|s| matches!(s.command, Command::OpenGraph | Command::OpenLocalGraph))
        );
    }

    #[test]
    fn specificity_order() {
        let r = CommandRegistry::default();
        let v = r.by_specificity();
        let prev = v.iter().position(|(_, c)| *c == Command::PrevTab).unwrap();
        let next = v.iter().position(|(_, c)| *c == Command::NextTab).unwrap();
        assert!(prev < next);
    }
}
