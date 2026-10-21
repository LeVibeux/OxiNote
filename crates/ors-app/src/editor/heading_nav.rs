//! Pure model of the heading navigator (a thin strip of ticks, one per
//! heading, placed proportionally to the heading's position in the content).
//! Not wired into the UI yet: the design is being validated first.
#![allow(dead_code)]

/// A heading with its vertical position (content coordinates), `None` when
/// it is not laid out (inside a folded section).
#[derive(Debug, Clone, PartialEq)]
pub struct NavHeading {
    pub level: u8,
    pub text: String,
    pub line: usize,
    pub y: Option<f32>,
}

/// Fraction (0..=1) of the content height at which each heading sits.
pub fn fractions(headings: &[NavHeading], total: f32) -> Vec<Option<f32>> {
    headings
        .iter()
        .map(|h| {
            h.y.filter(|_| total > 0.0)
                .map(|y| (y / total).clamp(0.0, 1.0))
        })
        .collect()
}

/// Tick length for `level` given the strip's `max` length: H1 longest.
pub fn tick_len(level: u8, max: f32) -> f32 {
    let k = f32::from(level.clamp(1, 6) - 1);
    (max * (1.0 - 0.14 * k)).max(3.0)
}

/// Tick thickness for `level`: H1 thickest.
pub fn tick_thickness(level: u8) -> f32 {
    3.0 - 0.4 * f32::from(level.clamp(1, 6) - 1)
}

/// Index of the current heading: the last one at or above `scroll + margin`;
/// the first one when none is; at the bottom of the content, the last one
/// that is visible.
pub fn current(headings: &[NavHeading], scroll: f32, viewport: f32, total: f32) -> Option<usize> {
    let mut best = None;
    let at_bottom = scroll + viewport >= total - 1.0;
    for (i, h) in headings.iter().enumerate() {
        let Some(y) = h.y else { continue };
        let limit = if at_bottom {
            scroll + viewport
        } else {
            scroll + 24.0
        };
        if y <= limit {
            best = Some(i);
        }
    }
    best.or_else(|| headings.iter().position(|h| h.y.is_some()))
}

/// Next (`delta > 0`) or previous heading from `cur`, skipping hidden ones.
pub fn step(headings: &[NavHeading], cur: Option<usize>, delta: i32) -> Option<usize> {
    let visible: Vec<usize> = (0..headings.len())
        .filter(|i| headings[*i].y.is_some())
        .collect();
    let pos = cur.and_then(|c| visible.iter().position(|v| *v == c));
    let next = match (pos, delta.signum()) {
        (Some(p), 1) => p + 1,
        (Some(p), -1) => p.checked_sub(1)?,
        (None, 1) => 0,
        (None, -1) => visible.len().checked_sub(1)?,
        _ => return cur,
    };
    visible.get(next).copied()
}

/// Tick nearest to `y` (strip coordinates) within `tolerance`.
pub fn nearest(tick_ys: &[Option<f32>], y: f32, tolerance: f32) -> Option<usize> {
    tick_ys
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.map(|t| (i, (t - y).abs())))
        .filter(|(_, d)| *d <= tolerance)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
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
    fn fractions_are_proportional_to_y_not_index() {
        let hs = [
            h(1, Some(0.0)),
            h(2, Some(100.0)),
            h(2, None),
            h(3, Some(900.0)),
        ];
        let f = fractions(&hs, 1000.0);
        assert_eq!(f, vec![Some(0.0), Some(0.1), None, Some(0.9)]);
        assert_eq!(fractions(&hs, 0.0), vec![None; 4]);
    }

    #[test]
    fn ticks_shrink_with_level() {
        assert!(tick_len(1, 14.0) > tick_len(6, 14.0));
        assert!(tick_thickness(1) > tick_thickness(6));
        assert!(tick_len(6, 14.0) >= 3.0);
    }

    #[test]
    fn current_follows_scroll() {
        let hs = [
            h(1, Some(50.0)),
            h(2, Some(400.0)),
            h(2, None),
            h(2, Some(900.0)),
        ];
        assert_eq!(current(&hs, 0.0, 300.0, 1200.0), Some(0));
        assert_eq!(current(&hs, 390.0, 300.0, 1200.0), Some(1));
        assert_eq!(current(&hs, 900.0, 300.0, 1200.0), Some(3));
        assert_eq!(current(&hs, 20.0, 300.0, 1200.0), Some(0));
    }

    #[test]
    fn step_skips_hidden_and_stops_at_ends() {
        let hs = [h(1, Some(0.0)), h(2, None), h(2, Some(500.0))];
        assert_eq!(step(&hs, Some(0), 1), Some(2));
        assert_eq!(step(&hs, Some(2), -1), Some(0));
        assert_eq!(step(&hs, Some(2), 1), None);
        assert_eq!(step(&hs, None, 1), Some(0));
    }

    #[test]
    fn nearest_picks_closest_within_tolerance() {
        let t = [Some(10.0), None, Some(14.0)];
        assert_eq!(nearest(&t, 13.0, 3.0), Some(2));
        assert_eq!(nearest(&t, 40.0, 3.0), None);
    }
}
