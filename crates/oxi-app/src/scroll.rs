//! Scroll speed and smoothing (Settings > Editor), applied to the raw wheel
//! events before egui sees them.
//!
//! egui moves trackpad deltas (points) as they come, multiplies wheel deltas
//! (lines) by `line_scroll_speed`, and spreads every large or line-based step
//! over ~0.1 s. That smoothing suits a notched mouse wheel but adds latency
//! to a trackpad, which many Linux and Windows drivers report in lines.

use eframe::egui::{Event, MouseWheelUnit, TouchPhase};

/// Slowest and fastest scroll speed offered in the settings.
pub const SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.25..=4.0;

/// Scale the wheel deltas of `events` by `speed`, and with `smooth` off,
/// make egui apply them at once: lines become points (`line_points` each)
/// and every step is preceded by a `Start` phase, which egui treats as
/// fingers on a trackpad (no smoothing). Zoom (Ctrl/Cmd + wheel) is left
/// as is.
pub fn adjust_wheel_events(events: &mut Vec<Event>, speed: f32, smooth: bool, line_points: f32) {
    let speed = speed.clamp(*SPEED_RANGE.start(), *SPEED_RANGE.end());
    if smooth && speed == 1.0 {
        return;
    }
    let mut out = Vec::with_capacity(events.len());
    for ev in events.drain(..) {
        match ev {
            Event::MouseWheel {
                unit,
                delta,
                phase: TouchPhase::Move,
                source,
                modifiers,
            } if !(modifiers.command || modifiers.ctrl || modifiers.mac_cmd) => {
                let (unit, delta) = match unit {
                    MouseWheelUnit::Line if !smooth => {
                        (MouseWheelUnit::Point, delta * line_points * speed)
                    }
                    _ => (unit, delta * speed),
                };
                if !smooth {
                    out.push(Event::MouseWheel {
                        unit,
                        delta: eframe::egui::Vec2::ZERO,
                        phase: TouchPhase::Start,
                        source,
                        modifiers,
                    });
                }
                out.push(Event::MouseWheel {
                    unit,
                    delta,
                    phase: TouchPhase::Move,
                    source,
                    modifiers,
                });
            }
            other => out.push(other),
        }
    }
    *events = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Modifiers, MouseWheelSource, RawInput, Vec2, vec2};

    fn wheel(unit: MouseWheelUnit, dy: f32, modifiers: Modifiers) -> Event {
        Event::MouseWheel {
            unit,
            delta: vec2(0.0, dy),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Unknown,
            modifiers,
        }
    }

    fn deltas(events: &[Event]) -> Vec<(MouseWheelUnit, TouchPhase, f32)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::MouseWheel {
                    unit, delta, phase, ..
                } => Some((*unit, *phase, delta.y)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn default_settings_change_nothing() {
        let mut ev = vec![wheel(MouseWheelUnit::Line, -3.0, Modifiers::NONE)];
        let before = ev.clone();
        adjust_wheel_events(&mut ev, 1.0, true, 40.0);
        assert_eq!(ev, before);
    }

    #[test]
    fn speed_scales_wheel_and_trackpad_but_not_zoom() {
        let mut ev = vec![
            wheel(MouseWheelUnit::Point, -10.0, Modifiers::NONE),
            wheel(MouseWheelUnit::Line, -1.0, Modifiers::NONE),
            wheel(MouseWheelUnit::Line, -1.0, Modifiers::COMMAND),
            Event::Text("x".into()),
        ];
        adjust_wheel_events(&mut ev, 2.0, true, 40.0);
        assert_eq!(
            deltas(&ev),
            vec![
                (MouseWheelUnit::Point, TouchPhase::Move, -20.0),
                (MouseWheelUnit::Line, TouchPhase::Move, -2.0),
                (MouseWheelUnit::Line, TouchPhase::Move, -1.0),
            ]
        );
        assert_eq!(ev.last(), Some(&Event::Text("x".into())));
        // Out-of-range speeds are clamped.
        let mut ev = vec![wheel(MouseWheelUnit::Point, -10.0, Modifiers::NONE)];
        adjust_wheel_events(&mut ev, 100.0, true, 40.0);
        assert_eq!(deltas(&ev)[0].2, -40.0);
    }

    #[test]
    fn direct_mode_turns_lines_into_points_after_a_start() {
        let mut ev = vec![wheel(MouseWheelUnit::Line, -1.0, Modifiers::NONE)];
        adjust_wheel_events(&mut ev, 1.5, false, 40.0);
        assert_eq!(
            deltas(&ev),
            vec![
                (MouseWheelUnit::Point, TouchPhase::Start, 0.0),
                (MouseWheelUnit::Point, TouchPhase::Move, -60.0),
            ]
        );
    }

    /// The scroll delta egui hands to scroll areas on the first frame.
    fn first_frame_delta(events: Vec<Event>) -> Vec2 {
        let ctx = egui::Context::default();
        let mut got = Vec2::ZERO;
        let mut out = ctx.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| got = ui.input(|i| i.smooth_scroll_delta()),
        );
        out.textures_delta.clear();
        got
    }

    #[test]
    fn egui_applies_a_direct_step_at_once_and_smooths_the_default() {
        let line = egui::Context::default().options(|o| o.input_options.line_scroll_speed);
        let step = || vec![wheel(MouseWheelUnit::Line, -3.0, Modifiers::NONE)];
        let smooth = first_frame_delta(step());
        let mut direct = step();
        adjust_wheel_events(&mut direct, 1.0, false, line);
        let direct = first_frame_delta(direct);
        // Direct: the whole step in the first frame; smoothed: only a part.
        assert_eq!(direct.y, -3.0 * line);
        assert!(smooth.y.abs() < direct.y.abs(), "{smooth:?} vs {direct:?}");
    }
}
