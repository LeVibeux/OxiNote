//! Phosphor icon code points (the font is bundled and registered as the
//! `icons` egui font family, see `theme::install_fonts`).

use eframe::egui::{FontFamily, FontId, RichText};

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn text(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).font(font(size))
}

pub const FOLDER: &str = "\u{e24a}";
pub const FOLDER_OPEN: &str = "\u{e256}";
pub const FILE_TEXT: &str = "\u{e23a}";
pub const SEARCH: &str = "\u{e30c}";
pub const TAG: &str = "\u{e478}";
pub const STAR: &str = "\u{e46a}";
pub const BOOKMARK: &str = "\u{e0ea}";
pub const GEAR: &str = "\u{e270}";
pub const GRAPH: &str = "\u{eb58}";
pub const CALENDAR: &str = "\u{e10a}";
pub const NOTE_PENCIL: &str = "\u{e34c}";
pub const TERMINAL: &str = "\u{e47e}";
pub const CARET_RIGHT: &str = "\u{e13a}";
pub const CARET_DOWN: &str = "\u{e136}";
pub const PLUS: &str = "\u{e3d4}";
pub const SORT_ASC: &str = "\u{e444}";
pub const CROSSHAIR: &str = "\u{e1d6}";
pub const FOLDER_PLUS: &str = "\u{e258}";
pub const SIDEBAR: &str = "\u{ec24}";
pub const PIN: &str = "\u{e3e2}";
pub const LINK: &str = "\u{e2e2}";
pub const CIRCLE_HALF: &str = "\u{e18c}";
pub const KEYBOARD: &str = "\u{e2d8}";
pub const PLUG: &str = "\u{e946}";
pub const BRUSH: &str = "\u{e6f0}";
pub const CLOCK: &str = "\u{e19a}";
pub const COLLAPSE: &str = "\u{e532}";
pub const VAULT: &str = "\u{e76e}";
pub const NOTE: &str = "\u{e348}";
pub const BOOK_OPEN: &str = "\u{e0e6}";
pub const PENCIL: &str = "\u{e3b4}";
