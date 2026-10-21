//! Obsidian-like colours (purple accent) for both themes.

use eframe::egui::{self, Color32, Context, FontFamily as FontFamilyName, Stroke, Theme};

use crate::settings::ThemeChoice;

/// Semantic colours used by the custom drawing code.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub text: Color32,
    pub strong: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub unresolved: Color32,
    pub tag_bg: Color32,
    pub code_bg: Color32,
    pub code_fg: Color32,
    pub highlight_bg: Color32,
    pub bg_secondary: Color32,
    pub bg_primary: Color32,
    pub border: Color32,
    pub graph_node: Color32,
    pub graph_attachment: Color32,
    pub graph_tag: Color32,
    pub graph_line: Color32,
}

pub const ACCENT_DARK: Color32 = Color32::from_rgb(0x7f, 0x6d, 0xf2);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x70, 0x5d, 0xcf);

impl Palette {
    pub fn dark() -> Self {
        Self {
            text: Color32::from_rgb(0xdc, 0xdd, 0xde),
            strong: Color32::from_rgb(0xf2, 0xf2, 0xf2),
            muted: Color32::from_rgb(0xa0, 0xa0, 0xa0),
            faint: Color32::from_rgb(0x6c, 0x6c, 0x6c),
            accent: ACCENT_DARK,
            accent_soft: Color32::from_rgba_unmultiplied(0x7f, 0x6d, 0xf2, 40),
            unresolved: Color32::from_rgba_unmultiplied(0x7f, 0x6d, 0xf2, 150),
            tag_bg: Color32::from_rgba_unmultiplied(0x7f, 0x6d, 0xf2, 38),
            code_bg: Color32::from_rgb(0x2a, 0x2a, 0x2a),
            code_fg: Color32::from_rgb(0xe8, 0xa0, 0x8a),
            highlight_bg: Color32::from_rgba_unmultiplied(0xff, 0xd0, 0x00, 70),
            bg_secondary: Color32::from_rgb(0x26, 0x26, 0x26),
            bg_primary: Color32::from_rgb(0x1e, 0x1e, 0x1e),
            border: Color32::from_rgb(0x36, 0x36, 0x36),
            graph_node: Color32::from_rgb(0xb3, 0xb3, 0xb3),
            graph_attachment: Color32::from_rgb(0xd8, 0xc0, 0x6a),
            graph_tag: Color32::from_rgb(0x6c, 0xc4, 0x8f),
            graph_line: Color32::from_rgb(0x4a, 0x4a, 0x4a),
        }
    }

    pub fn light() -> Self {
        Self {
            text: Color32::from_rgb(0x2e, 0x33, 0x38),
            strong: Color32::from_rgb(0x10, 0x10, 0x10),
            muted: Color32::from_rgb(0x5c, 0x5c, 0x5c),
            faint: Color32::from_rgb(0xa0, 0xa0, 0xa0),
            accent: ACCENT_LIGHT,
            accent_soft: Color32::from_rgba_unmultiplied(0x70, 0x5d, 0xcf, 36),
            unresolved: Color32::from_rgba_unmultiplied(0x70, 0x5d, 0xcf, 150),
            tag_bg: Color32::from_rgba_unmultiplied(0x70, 0x5d, 0xcf, 30),
            code_bg: Color32::from_rgb(0xf3, 0xf1, 0xf6),
            code_fg: Color32::from_rgb(0xb0, 0x4a, 0x30),
            highlight_bg: Color32::from_rgba_unmultiplied(0xff, 0xd0, 0x00, 110),
            bg_secondary: Color32::from_rgb(0xf6, 0xf6, 0xf6),
            bg_primary: Color32::from_rgb(0xff, 0xff, 0xff),
            border: Color32::from_rgb(0xe0, 0xe0, 0xe0),
            graph_node: Color32::from_rgb(0x8a, 0x8a, 0x8a),
            graph_attachment: Color32::from_rgb(0xb8, 0x96, 0x20),
            graph_tag: Color32::from_rgb(0x3a, 0x9a, 0x60),
            graph_line: Color32::from_rgb(0xd0, 0xd0, 0xd0),
        }
    }

    pub fn of(ctx: &Context) -> Self {
        let base = match ctx.theme() {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::light(),
        };
        match accent_override(ctx) {
            Some(rgb) => base.with_accent(rgb),
            None => base,
        }
    }

    /// Same palette with a custom accent colour.
    pub fn with_accent(mut self, [r, g, b]: [u8; 3]) -> Self {
        self.accent = Color32::from_rgb(r, g, b);
        self.accent_soft = Color32::from_rgba_unmultiplied(r, g, b, 40);
        self.unresolved = Color32::from_rgba_unmultiplied(r, g, b, 150);
        self.tag_bg = Color32::from_rgba_unmultiplied(r, g, b, 38);
        self
    }

    pub fn callout(&self, kind: &str) -> Color32 {
        match kind {
            "tip" | "hint" | "important" => Color32::from_rgb(0x00, 0xbf, 0xbc),
            "success" | "check" | "done" => Color32::from_rgb(0x08, 0xb9, 0x4e),
            "question" | "help" | "faq" | "warning" | "caution" | "attention" => {
                Color32::from_rgb(0xec, 0x75, 0x00)
            }
            "failure" | "fail" | "missing" | "danger" | "error" | "bug" => {
                Color32::from_rgb(0xe9, 0x31, 0x47)
            }
            "example" => Color32::from_rgb(0x78, 0x52, 0xee),
            "quote" | "cite" => Color32::from_rgb(0x9e, 0x9e, 0x9e),
            _ => Color32::from_rgb(0x08, 0x6d, 0xdd),
        }
    }
}

fn accent_id() -> egui::Id {
    egui::Id::unique("obsirust-accent")
}

fn accent_override(ctx: &Context) -> Option<[u8; 3]> {
    ctx.data(|d| d.get_temp::<Option<[u8; 3]>>(accent_id()))
        .flatten()
}

const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.otf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.otf");
const INTER_ITALIC: &[u8] = include_bytes!("../../assets/fonts/Inter-Italic.otf");
const INTER_BOLD_ITALIC: &[u8] = include_bytes!("../../assets/fonts/Inter-BoldItalic.otf");
const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");

/// Names of the extra proportional families holding the Inter faces.
pub const BOLD_FAMILY: &str = "bold";
pub const ITALIC_FAMILY: &str = "italic";
pub const BOLD_ITALIC_FAMILY: &str = "bold-italic";

/// Are the bold / italic families installed in this context? (Contexts of
/// headless tests do not install fonts, and egui panics on unknown families.)
pub fn has_styled_fonts(ctx: &Context) -> bool {
    let want = FontFamilyName::Name(BOLD_ITALIC_FAMILY.into());
    ctx.fonts(|f| f.families().contains(&want))
}

/// Register Inter (primary proportional font when `inter`) and the
/// Phosphor icon font (its own `icons` family).
pub fn install_fonts(ctx: &Context, inter: bool) {
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("Inter".into(), Arc::new(FontData::from_static(INTER)));
    fonts
        .font_data
        .insert("Phosphor".into(), Arc::new(FontData::from_static(PHOSPHOR)));
    for (name, bytes) in [
        ("Inter-Bold", INTER_BOLD),
        ("Inter-Italic", INTER_ITALIC),
        ("Inter-BoldItalic", INTER_BOLD_ITALIC),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // Fallbacks (symbols, emoji) of the default proportional family.
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if inter && let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, "Inter".into());
    }
    // Real bold / italic faces for the editor (`bold`, `italic`,
    // `bold-italic` families), see `ui::editor::styled_font`.
    for (family, face) in [
        (BOLD_FAMILY, "Inter-Bold"),
        (ITALIC_FAMILY, "Inter-Italic"),
        (BOLD_ITALIC_FAMILY, "Inter-BoldItalic"),
    ] {
        let mut list = vec![face.to_string()];
        list.extend(fallback.iter().filter(|f| *f != "Inter").cloned());
        fonts.families.insert(FontFamily::Name(family.into()), list);
    }
    fonts
        .families
        .insert(FontFamily::Name("icons".into()), vec!["Phosphor".into()]);
    ctx.set_fonts(fonts);
}

/// Apply the user's appearance settings (accent colour, interface size).
pub fn apply_settings(ctx: &Context, accent: Option<[u8; 3]>, ui_font_size: f32) {
    ctx.data_mut(|d| d.insert_temp(accent_id(), accent));
    install(ctx);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            use egui::{FontFamily::Proportional, FontId, TextStyle};
            let s = ui_font_size.clamp(10.0, 24.0);
            style
                .text_styles
                .insert(TextStyle::Body, FontId::new(s, Proportional));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::new(s, Proportional));
            style
                .text_styles
                .insert(TextStyle::Small, FontId::new(s * 0.85, Proportional));
            style
                .text_styles
                .insert(TextStyle::Heading, FontId::new(s * 1.45, Proportional));
        });
    }
}

/// Install the accent colours into both egui themes.
pub fn install(ctx: &Context) {
    let accent = accent_override(ctx);
    for (theme, p) in [
        (Theme::Dark, Palette::dark()),
        (Theme::Light, Palette::light()),
    ] {
        let p = match accent {
            Some(rgb) => p.with_accent(rgb),
            None => p,
        };
        ctx.style_mut_of(theme, |style| {
            let v = &mut style.visuals;
            v.hyperlink_color = p.accent;
            v.selection.bg_fill = p.accent_soft.gamma_multiply(2.5);
            v.selection.stroke = Stroke::new(1.0, p.accent);
            v.panel_fill = p.bg_secondary;
            v.window_fill = p.bg_secondary;
            v.extreme_bg_color = p.bg_primary;
            v.code_bg_color = p.code_bg;
            v.faint_bg_color = p.bg_primary;
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.6));
            v.widgets.active.bg_fill = p.accent;
            v.widgets.active.weak_bg_fill = p.accent;
            v.text_cursor.stroke = Stroke::new(2.0, p.accent);
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.button_padding = egui::vec2(6.0, 3.0);
            let r = egui::CornerRadius::same(4);
            v.widgets.noninteractive.corner_radius = r;
            v.widgets.inactive.corner_radius = r;
            v.widgets.hovered.corner_radius = r;
            v.widgets.active.corner_radius = r;
            v.widgets.open.corner_radius = r;
            v.window_corner_radius = egui::CornerRadius::same(8);
            v.menu_corner_radius = egui::CornerRadius::same(6);
        });
    }
}

pub fn apply_choice(ctx: &Context, choice: ThemeChoice) {
    let pref = match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(pref);
}
