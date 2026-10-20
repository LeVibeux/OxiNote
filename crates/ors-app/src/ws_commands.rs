//! Commands of the workspace / file management / settings area. They are
//! wrapped in [`crate::commands::Command::Ws`] so that the shared command
//! enum only needs one variant.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

use crate::commands::{Command, CommandSpec};
use crate::plugins::CorePlugin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WsCommand {
    SplitRight,
    SplitDown,
    TogglePin,
    CloseOthers,
    CloseGroup,
    FocusNextGroup,
    OpenSettings,
    VaultSwitcher,
    InsertTemplate,
    InsertDate,
    InsertTime,
    PrevDailyNote,
    NextDailyNote,
    BookmarkFile,
    BookmarkHeading,
    BookmarkSearch,
    ShowBookmarks,
    RevealActiveFile,
    MoveFile,
    DuplicateFile,
    CopyPath,
}

impl WsCommand {
    pub const ALL: [WsCommand; 21] = [
        WsCommand::SplitRight,
        WsCommand::SplitDown,
        WsCommand::TogglePin,
        WsCommand::CloseOthers,
        WsCommand::CloseGroup,
        WsCommand::FocusNextGroup,
        WsCommand::OpenSettings,
        WsCommand::VaultSwitcher,
        WsCommand::InsertTemplate,
        WsCommand::InsertDate,
        WsCommand::InsertTime,
        WsCommand::PrevDailyNote,
        WsCommand::NextDailyNote,
        WsCommand::BookmarkFile,
        WsCommand::BookmarkHeading,
        WsCommand::BookmarkSearch,
        WsCommand::ShowBookmarks,
        WsCommand::RevealActiveFile,
        WsCommand::MoveFile,
        WsCommand::DuplicateFile,
        WsCommand::CopyPath,
    ];

    pub fn name(self) -> &'static str {
        use WsCommand::*;
        match self {
            SplitRight => "Split right",
            SplitDown => "Split down",
            TogglePin => "Toggle pin",
            CloseOthers => "Close all other tabs",
            CloseGroup => "Close this tab group",
            FocusNextGroup => "Focus on next tab group",
            OpenSettings => "Open settings",
            VaultSwitcher => "Open vault switcher (manage vaults)",
            InsertTemplate => "Templates: Insert template",
            InsertDate => "Templates: Insert current date",
            InsertTime => "Templates: Insert current time",
            PrevDailyNote => "Daily notes: Open previous daily note",
            NextDailyNote => "Daily notes: Open next daily note",
            BookmarkFile => "Bookmarks: Bookmark current file",
            BookmarkHeading => "Bookmarks: Bookmark heading…",
            BookmarkSearch => "Bookmarks: Bookmark current search",
            ShowBookmarks => "Bookmarks: Show bookmarks",
            RevealActiveFile => "Files: Reveal current file in navigation",
            MoveFile => "Move current file to another folder",
            DuplicateFile => "Make a copy of the current file",
            CopyPath => "Copy file path",
        }
    }

    /// Obsidian command id (used in `hotkeys.json`).
    pub fn id(self) -> &'static str {
        use WsCommand::*;
        match self {
            SplitRight => "workspace:split-vertical",
            SplitDown => "workspace:split-horizontal",
            TogglePin => "workspace:toggle-pin",
            CloseOthers => "workspace:close-others",
            CloseGroup => "workspace:close-tab-group",
            FocusNextGroup => "workspace:focus-on-next-tab-group",
            OpenSettings => "app:open-settings",
            VaultSwitcher => "app:open-vault",
            InsertTemplate => "insert-template",
            InsertDate => "insert-current-date",
            InsertTime => "insert-current-time",
            PrevDailyNote => "daily-notes:goto-prev",
            NextDailyNote => "daily-notes:goto-next",
            BookmarkFile => "bookmarks:bookmark-current-view",
            BookmarkHeading => "bookmarks:bookmark-current-heading",
            BookmarkSearch => "bookmarks:bookmark-current-search",
            ShowBookmarks => "bookmarks:open",
            RevealActiveFile => "file-explorer:reveal-active-file",
            MoveFile => "file-explorer:move-file",
            DuplicateFile => "file-explorer:duplicate-file",
            CopyPath => "workspace:copy-path",
        }
    }

    fn default_shortcut(self) -> Option<KeyboardShortcut> {
        match self {
            WsCommand::OpenSettings => Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma)),
            _ => None,
        }
    }

    /// Core plugin providing the command, if any.
    pub fn plugin(self) -> Option<CorePlugin> {
        use WsCommand::*;
        match self {
            InsertTemplate | InsertDate | InsertTime => Some(CorePlugin::Templates),
            PrevDailyNote | NextDailyNote => Some(CorePlugin::DailyNotes),
            BookmarkFile | BookmarkHeading | BookmarkSearch | ShowBookmarks => {
                Some(CorePlugin::Bookmarks)
            }
            _ => None,
        }
    }
}

/// Registry entries for [`WsCommand`]s.
pub fn specs() -> Vec<CommandSpec> {
    WsCommand::ALL
        .iter()
        .map(|&c| CommandSpec {
            command: Command::Ws(c),
            name: c.name(),
            shortcut: c.default_shortcut(),
        })
        .collect()
}

/// Core plugin owning any command (also the base ones).
pub fn plugin_of(cmd: Command) -> Option<CorePlugin> {
    match cmd {
        Command::Ws(c) => c.plugin(),
        Command::OpenDailyNote => Some(CorePlugin::DailyNotes),
        Command::OpenGraph | Command::OpenLocalGraph => Some(CorePlugin::Graph),
        Command::ShowTags => Some(CorePlugin::TagPane),
        _ => None,
    }
}
