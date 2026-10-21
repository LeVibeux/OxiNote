//! Pure model of the heading navigator: a compact stack of short horizontal
//! ticks (one per heading) on the right edge of a note panel.
//!
//! Two designs share it: the floating table of contents (A, ticks at rest +
//! a card on hover) and the magnifier (B, a macOS Dock effect on the ticks).

use oxi_core::Heading;

/// A heading of the note with its vertical position in the content
/// (`None` while it is not laid out: folded section, collapsed callout).
#[derive(Debug, Clone, PartialEq)]
pub struct NavHeading {
    pub level: u8,
    pub text: String,
    /// 0-based line of the heading.
    pub line: usize,
    pub y: Option<f32>,
}

impl NavHeading {
    pub fn from_parsed(h: &[Heading]) -> Vec<Self> {
        h.iter()
            .map(|h| Self {
                level: h.level,
                text: h.text.clone(),
                line: h.line,
                y: None,
            })
            .collect()
    }
}

/// Spacing of the ticks at rest.
pub const BASE_PITCH: f32 = 10.0;
/// Width of the nav strip.
pub const STRIP_WIDTH: f32 = 46.0;
/// How many neighbours the magnifier enlarges.
pub const REACH: f32 = 2.6;
/// Extra slot height of the tick under the pointer.
pub const ZOOM: f32 = 2.2;
/// Tick thickness at rest / when magnified.
pub const THICK: f32 = 2.0;
pub const THICK_ZOOM: f32 = 3.0;

/// Tick length by heading level: H1 longest.
pub fn dash_len(level: u8) -> f32 {
    match level {
        0 | 1 => 20.0,
        2 => 14.0,
        _ => 8.0,
    }
}

/// Spacing of the ticks: [`BASE_PITCH`], reduced so that the stack of `n`
/// ticks never exceeds 70 % of the panel height.
pub fn pitch(n: usize, panel_height: f32) -> f32 {
    if n == 0 {
        return BASE_PITCH;
    }
    (0.7 * panel_height / n as f32).clamp(3.0, BASE_PITCH)
}

/// Index of the current heading: the last one above the top of the visible
/// area (`scroll` + a small margin). The first heading while none has been
/// scrolled past.
pub fn current(headings: &[NavHeading], scroll: f32) -> Option<usize> {
    let mut best = None;
    for (i, h) in headings.iter().enumerate() {
        if h.y.is_some_and(|y| y <= scroll + 24.0) {
            best = Some(i);
        }
    }
    best.or_else(|| headings.iter().position(|h| h.y.is_some()))
        .or((!headings.is_empty()).then_some(0))
}

/// One tick of the strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    /// Top of the slot, relative to the top of the strip at rest.
    pub top: f32,
    pub slot: f32,
    /// Magnification factor 0..=1.
    pub g: f32,
}

impl Tick {
    pub fn center(&self) -> f32 {
        self.top + self.slot / 2.0
    }
}

/// Ticks at rest: `n` slots of `pitch`.
pub fn rest_layout(n: usize, pitch: f32) -> Vec<Tick> {
    (0..n)
        .map(|i| Tick {
            top: i as f32 * pitch,
            slot: pitch,
            g: 0.0,
        })
        .collect()
}

/// Fractional index under the pointer (`hy`: distance from the top of the
/// strip at rest).
pub fn focus(n: usize, pitch: f32, hy: f32) -> f32 {
    (hy / pitch - 0.5).clamp(0.0, n.saturating_sub(1) as f32)
}

/// Magnifier layout: every tick gets `g = max(0, 1 - |i - focus| / REACH)`,
/// its slot grows by `1 + ZOOM * g`, positions are accumulated and shifted
/// so that the nearest tick stays under the pointer. `amount` (0..=1)
/// blends between rest and fully magnified (smooth in/out).
pub fn loupe_layout(n: usize, pitch: f32, hy: f32, amount: f32) -> Vec<Tick> {
    if n == 0 {
        return Vec::new();
    }
    let f = focus(n, pitch, hy);
    let g: Vec<f32> = (0..n)
        .map(|i| (1.0 - (i as f32 - f).abs() / REACH).max(0.0) * amount)
        .collect();
    let slots: Vec<f32> = g.iter().map(|g| pitch * (1.0 + ZOOM * g)).collect();
    let mut cum = Vec::with_capacity(n);
    let mut acc = 0.0;
    for s in &slots {
        cum.push(acc);
        acc += s;
    }
    let c = (f.round() as usize).min(n - 1);
    // Shift so that the tick under the pointer stays under it (no jump);
    // `amount` blends from the rest layout (no shift).
    let offset = amount * (hy - (cum[c] + slots[c] / 2.0));
    (0..n)
        .map(|i| Tick {
            top: cum[i] + offset,
            slot: slots[i],
            g: g[i],
        })
        .collect()
}

/// Tick whose slot contains `y` (same coordinates as [`Tick::top`]); the
/// nearest one when `y` falls in no slot.
pub fn hit(ticks: &[Tick], y: f32) -> Option<usize> {
    ticks
        .iter()
        .position(|t| y >= t.top && y < t.top + t.slot)
        .or_else(|| {
            ticks
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    (a.1.center() - y)
                        .abs()
                        .total_cmp(&(b.1.center() - y).abs())
                })
                .map(|(i, _)| i)
        })
}

/// Left padding of a card row (floating table of contents).
pub fn card_indent(level: u8) -> f32 {
    8.0 + f32::from(level.clamp(1, 6) - 1) * 14.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(level: u8, y: Option<f32>) -> NavHeading {
        NavHeading {
            level,
            text: format!("H{level}"),
            line: 0,
            y,
        }
    }

    #[test]
    fn dash_lengths_by_level() {
        assert_eq!(dash_len(1), 20.0);
        assert_eq!(dash_len(2), 14.0);
        assert_eq!(dash_len(3), 8.0);
        assert_eq!(dash_len(6), 8.0);
    }

    #[test]
    fn pitch_shrinks_for_long_notes() {
        assert_eq!(pitch(10, 600.0), BASE_PITCH);
        // 100 ticks in 600 px: 70 % = 420 px => 4.2 px each.
        assert!((pitch(100, 600.0) - 4.2).abs() < 1e-4);
        assert_eq!(pitch(1000, 600.0), 3.0);
        assert_eq!(pitch(0, 600.0), BASE_PITCH);
    }

    #[test]
    fn current_is_the_last_heading_above_the_top() {
        let hs = [
            h(1, Some(50.0)),
            h(2, Some(400.0)),
            h(2, None),
            h(2, Some(900.0)),
        ];
        assert_eq!(current(&hs, 0.0), Some(0));
        assert_eq!(current(&hs, 380.0), Some(1));
        assert_eq!(current(&hs, 399.0), Some(1));
        assert_eq!(current(&hs, 5000.0), Some(3));
        // Before the first heading: the first one.
        let hs = [h(1, Some(500.0))];
        assert_eq!(current(&hs, 0.0), Some(0));
        assert_eq!(current(&[], 0.0), None);
        // Not laid out yet: first heading.
        assert_eq!(current(&[h(1, None), h(2, None)], 0.0), Some(0));
    }

    #[test]
    fn rest_layout_is_uniform() {
        let t = rest_layout(3, 10.0);
        assert_eq!(t[2].top, 20.0);
        assert!(t.iter().all(|t| t.slot == 10.0 && t.g == 0.0));
        assert_eq!(hit(&t, 14.0), Some(1));
        assert_eq!(hit(&t, 500.0), Some(2));
    }

    #[test]
    fn focus_is_fractional_and_clamped() {
        assert_eq!(focus(10, 10.0, 55.0), 5.0);
        assert_eq!(focus(10, 10.0, 0.0), 0.0);
        assert_eq!(focus(10, 10.0, 999.0), 9.0);
    }

    #[test]
    fn loupe_factors_follow_the_formula() {
        let t = loupe_layout(12, 10.0, 65.0, 1.0);
        let f = focus(12, 10.0, 65.0); // 6.0
        assert_eq!(f, 6.0);
        assert!((t[6].g - 1.0).abs() < 1e-5);
        assert!((t[7].g - (1.0 - 1.0 / REACH)).abs() < 1e-5);
        assert_eq!(t[0].g, 0.0); // beyond REACH
        assert!((t[6].slot - 10.0 * (1.0 + ZOOM)).abs() < 1e-4);
        assert!(t[0].slot == 10.0);
    }

    #[test]
    fn loupe_keeps_the_point_under_the_pointer() {
        for hy in [5.0, 33.0, 65.0, 98.0] {
            let t = loupe_layout(10, 10.0, hy, 1.0);
            let i = hit(&t, hy).expect("hit");
            // The pointer is inside the slot of the tick it designates.
            assert!(t[i].top <= hy + 1e-3 && hy <= t[i].top + t[i].slot + 1e-3);
        }
        // Not magnified: rest layout.
        let t = loupe_layout(5, 10.0, 25.0, 0.0);
        assert_eq!(t, rest_layout(5, 10.0));
        // Positions are contiguous.
        let t = loupe_layout(8, 10.0, 40.0, 1.0);
        for w in t.windows(2) {
            assert!((w[0].top + w[0].slot - w[1].top).abs() < 1e-3);
        }
    }

    #[test]
    fn card_rows_indent_by_level() {
        assert_eq!(card_indent(1), 8.0);
        assert_eq!(card_indent(3), 36.0);
    }
}
