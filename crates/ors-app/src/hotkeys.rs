//! Custom hotkeys: `.obsidian/hotkeys.json` in Obsidian's format
//!
//! ```json
//! {"command-id": [{"modifiers": ["Mod", "Shift"], "key": "F"}],
//!  "other-id": []}
//! ```
//! Only commands whose binding differs from the default are stored; an
//! empty array removes a default hotkey. Ids unknown to ObsiRust are kept.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Map, Value, json};

use crate::commands::{Command, CommandRegistry};

pub const HOTKEYS_JSON: &str = "hotkeys.json";

/// Obsidian-style command id of any command.
pub fn command_id(cmd: Command) -> String {
    use Command::*;
    let id = match cmd {
        Ws(c) => c.id(),
        QuickSwitcher => "switcher:open",
        CommandPalette => "command-palette:open",
        NewNote => "file-explorer:new-file",
        NewFolder => "file-explorer:new-folder",
        OpenDailyNote => "daily-notes",
        ToggleMode => "markdown:toggle-preview",
        OpenGraph => "graph:open",
        OpenLocalGraph => "graph:open-local",
        SearchVault => "global-search:open",
        ShowTags => "tag-pane:open",
        ShowFiles => "file-explorer:open",
        RenameNote => "workspace:edit-file-title",
        DeleteNote => "app:delete-file",
        Save => "editor:save-file",
        CloseTab => "workspace:close",
        NextTab => "workspace:next-tab",
        PrevTab => "workspace:previous-tab",
        NavigateBack => "app:go-back",
        NavigateForward => "app:go-forward",
        ToggleLeftSidebar => "app:toggle-left-sidebar",
        ToggleRightSidebar => "app:toggle-right-sidebar",
        ReloadVault => "app:reload",
        SwitchVault => "obsirust:close-vault",
        other => return generic_id(other),
    };
    id.to_string()
}

/// `obsirust:kebab-case-of-the-variant` for commands without a known id.
fn generic_id(cmd: Command) -> String {
    let dbg = format!("{cmd:?}");
    let mut out = String::from("obsirust:");
    for (i, c) in dbg.chars().filter(|c| c.is_alphanumeric()).enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn is_mac() -> bool {
    cfg!(target_os = "macos")
}

/// Normalize modifiers captured from a key press so that the platform
/// "command" key (Ctrl, or ⌘ on macOS) is stored as `command`.
pub fn normalize(m: Modifiers) -> Modifiers {
    let command = m.command || m.mac_cmd || (!is_mac() && m.ctrl);
    Modifiers {
        alt: m.alt,
        ctrl: is_mac() && m.ctrl,
        shift: m.shift,
        mac_cmd: false,
        command,
    }
}

fn key_to_obsidian(key: Key) -> String {
    match key {
        Key::ArrowLeft => "ArrowLeft".into(),
        Key::ArrowRight => "ArrowRight".into(),
        Key::ArrowUp => "ArrowUp".into(),
        Key::ArrowDown => "ArrowDown".into(),
        Key::Space => " ".into(),
        Key::Minus => "-".into(),
        k => k.symbol_or_name().to_string(),
    }
}

fn key_from_obsidian(s: &str) -> Option<Key> {
    match s {
        " " | "Space" => Some(Key::Space),
        "-" => Some(Key::Minus),
        _ => Key::from_name(s).or_else(|| Key::from_name(&s.to_uppercase())),
    }
}

pub fn to_obsidian(sc: &KeyboardShortcut) -> Value {
    let m = sc.modifiers;
    let mut mods = Vec::new();
    if m.command || m.mac_cmd {
        mods.push("Mod");
    }
    if m.ctrl && is_mac() {
        mods.push("Ctrl");
    }
    if m.alt {
        mods.push("Alt");
    }
    if m.shift {
        mods.push("Shift");
    }
    json!({"modifiers": mods, "key": key_to_obsidian(sc.logical_key)})
}

pub fn from_obsidian(v: &Value) -> Option<KeyboardShortcut> {
    let key = key_from_obsidian(v.get("key")?.as_str()?)?;
    let mut m = Modifiers::NONE;
    for name in v
        .get("modifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        match name {
            "Mod" => m.command = true,
            "Ctrl" if is_mac() => m.ctrl = true,
            "Ctrl" | "Meta" => m.command = true,
            "Alt" => m.alt = true,
            "Shift" => m.shift = true,
            _ => {}
        }
    }
    Some(KeyboardShortcut::new(m, key))
}

fn same(a: Option<KeyboardShortcut>, b: Option<KeyboardShortcut>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.logical_key == b.logical_key && normalize(a.modifiers) == normalize(b.modifiers)
        }
        _ => false,
    }
}

/// Apply `hotkeys.json` on top of the default bindings.
pub fn apply(json: &str, registry: &mut CommandRegistry) {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else {
        return;
    };
    let cmds: Vec<Command> = registry.specs().iter().map(|s| s.command).collect();
    for cmd in cmds {
        if let Some(Value::Array(list)) = map.get(&command_id(cmd)) {
            let sc = list.iter().find_map(from_obsidian);
            registry.set_shortcut(cmd, sc);
        }
    }
}

/// Serialize the non-default bindings of `registry`, keeping entries of
/// `existing` that ObsiRust does not know.
pub fn to_json(registry: &CommandRegistry, existing: &str) -> String {
    let defaults = CommandRegistry::default();
    let mut map: Map<String, Value> = match serde_json::from_str::<Value>(existing) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    for spec in registry.specs() {
        let id = command_id(spec.command);
        map.remove(&id);
        if !same(spec.shortcut, defaults.shortcut(spec.command)) {
            let list: Vec<Value> = spec.shortcut.iter().map(to_obsidian).collect();
            map.insert(id, Value::Array(list));
        }
    }
    serde_json::to_string_pretty(&map).unwrap_or_else(|_| "{}".into())
}

/// Shortcut text like Obsidian shows it ("Ctrl + Shift + F").
pub fn display(sc: &KeyboardShortcut) -> String {
    let m = sc.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if m.command || m.mac_cmd {
        parts.push(if is_mac() { "⌘" } else { "Ctrl" }.into());
    }
    if m.ctrl && is_mac() {
        parts.push("Ctrl".into());
    }
    if m.alt {
        parts.push("Alt".into());
    }
    if m.shift {
        parts.push("Shift".into());
    }
    parts.push(match sc.logical_key {
        Key::ArrowLeft => "←".into(),
        Key::ArrowRight => "→".into(),
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        k => k.symbol_or_name().to_string(),
    });
    parts.join(" + ")
}

/// Commands bound to the same shortcut as `cmd` (conflicts).
pub fn conflicts(registry: &CommandRegistry, cmd: Command) -> Vec<Command> {
    let Some(sc) = registry.shortcut(cmd) else {
        return Vec::new();
    };
    registry
        .specs()
        .iter()
        .filter(|s| s.command != cmd && same(s.shortcut, Some(sc)))
        .map(|s| s.command)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws_commands::WsCommand;

    #[test]
    fn ids_are_unique() {
        let r = CommandRegistry::default();
        let mut seen = std::collections::HashSet::new();
        for s in r.specs() {
            assert!(seen.insert(command_id(s.command)), "{:?}", s.command);
        }
        assert_eq!(command_id(Command::SearchVault), "global-search:open");
        assert_eq!(
            command_id(Command::Ws(WsCommand::SplitRight)),
            "workspace:split-vertical"
        );
        assert_eq!(command_id(Command::ToggleTheme), "obsirust:toggle-theme");
    }

    #[test]
    fn obsidian_format_round_trip() {
        let src = r#"{
          "global-search:open": [{"modifiers": ["Mod", "Shift"], "key": "G"}],
          "switcher:open": [],
          "workspace:split-vertical": [{"modifiers": ["Alt"], "key": "ArrowRight"}],
          "some-plugin:cmd": [{"modifiers": ["Mod"], "key": "K"}]
        }"#;
        let mut r = CommandRegistry::default();
        apply(src, &mut r);
        assert_eq!(
            r.shortcut(Command::SearchVault),
            Some(KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::G
            ))
        );
        assert_eq!(r.shortcut(Command::QuickSwitcher), None);
        assert_eq!(
            r.shortcut(Command::Ws(WsCommand::SplitRight)),
            Some(KeyboardShortcut::new(Modifiers::ALT, Key::ArrowRight))
        );
        // Untouched commands keep their default.
        assert_eq!(
            r.shortcut(Command::ToggleMode),
            Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::E))
        );
        let out: Value = serde_json::from_str(&to_json(&r, src)).unwrap();
        let orig: Value = serde_json::from_str(src).unwrap();
        assert_eq!(out, orig);
        // Resetting to the default removes the entry.
        r.set_shortcut(Command::QuickSwitcher, CommandRegistry::default().shortcut(Command::QuickSwitcher));
        let out: Value = serde_json::from_str(&to_json(&r, src)).unwrap();
        assert!(out.get("switcher:open").is_none());
    }

    #[test]
    fn keys_and_display() {
        for key in [Key::Comma, Key::F2, Key::Tab, Key::Space, Key::A, Key::ArrowUp] {
            let sc = KeyboardShortcut::new(Modifiers::COMMAND, key);
            assert_eq!(from_obsidian(&to_obsidian(&sc)), Some(sc), "{key:?}");
        }
        let sc = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
        if !is_mac() {
            assert_eq!(display(&sc), "Ctrl + Shift + F");
        }
        let captured = Modifiers {
            ctrl: !is_mac(),
            command: true,
            ..Modifiers::NONE
        };
        assert_eq!(normalize(captured), Modifiers::COMMAND);
    }

    #[test]
    fn detects_conflicts() {
        let mut r = CommandRegistry::default();
        r.set_shortcut(
            Command::ShowTags,
            Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
        );
        assert_eq!(
            conflicts(&r, Command::ShowTags),
            vec![Command::QuickSwitcher]
        );
    }
}
