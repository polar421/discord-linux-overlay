//! Monitor-aware overlay placement.
//!
//! The overlay can either be dragged freely (the compositor remembers the
//! position) or "anchored" to a monitor corner/edge with an offset. This module
//! is pure arithmetic so the anchoring behaviour can be tested on any machine,
//! including headless CI.
//!
//! Coordinates are physical pixels, exactly like the values the windowing layer
//! reports, so multi-monitor setups with negative origins and fractional scale
//! factors are handled without surprises.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor::BottomRight
    }
}

impl Anchor {
    /// `(horizontal, vertical)` where 0 = start, 1 = middle, 2 = end.
    fn axes(self) -> (u8, u8) {
        match self {
            Anchor::TopLeft => (0, 0),
            Anchor::TopCenter => (1, 0),
            Anchor::TopRight => (2, 0),
            Anchor::MiddleLeft => (0, 1),
            Anchor::Center => (1, 1),
            Anchor::MiddleRight => (2, 1),
            Anchor::BottomLeft => (0, 2),
            Anchor::BottomCenter => (1, 2),
            Anchor::BottomRight => (2, 2),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl MonitorRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn contains(self, placement: WindowPlacement) -> bool {
        let left = self.x as i64;
        let top = self.y as i64;
        let right = left + self.width as i64;
        let bottom = top + self.height as i64;

        let window_left = placement.x as i64;
        let window_top = placement.y as i64;
        let window_right = window_left + placement.width as i64;
        let window_bottom = window_top + placement.height as i64;

        window_right > left && window_left < right && window_bottom > top && window_top < bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Compute the physical window placement for an anchor.
///
/// `window` is the logical size the web view wants, `scale` the monitor's scale
/// factor, and `offset` the logical margin from the anchored edges.
pub fn compute_placement(
    monitor: MonitorRect,
    window: (f64, f64),
    scale: f64,
    anchor: Anchor,
    offset: (f64, f64),
) -> Result<WindowPlacement> {
    let (logical_width, logical_height) = window;

    if !logical_width.is_finite()
        || !logical_height.is_finite()
        || logical_width <= 0.0
        || logical_height <= 0.0
        || !scale.is_finite()
        || scale <= 0.0
        || monitor.width == 0
        || monitor.height == 0
    {
        return Err(Error::Settings(
            "invalid placement dimensions or monitor bounds".to_string(),
        ));
    }

    let width = (logical_width * scale)
        .ceil()
        .clamp(1.0, f64::from(monitor.width)) as u32;
    let height = (logical_height * scale)
        .ceil()
        .clamp(1.0, f64::from(monitor.height)) as u32;

    let (horizontal, vertical) = anchor.axes();
    let offset_x = if offset.0.is_finite() {
        offset.0.max(0.0)
    } else {
        0.0
    };
    let offset_y = if offset.1.is_finite() {
        offset.1.max(0.0)
    } else {
        0.0
    };

    Ok(WindowPlacement {
        x: axis(
            monitor.x,
            monitor.width,
            width,
            offset_x * scale,
            horizontal,
        ),
        y: axis(
            monitor.y,
            monitor.height,
            height,
            offset_y * scale,
            vertical,
        ),
        width,
        height,
    })
}

fn axis(start: i32, length: u32, size: u32, offset: f64, align: u8) -> i32 {
    let available = f64::from(length.saturating_sub(size));
    let relative = match align {
        0 => offset,
        1 => available / 2.0,
        _ => available - offset,
    };
    (f64::from(start) + relative.clamp(0.0, available)).round() as i32
}

/// True when the window does not overlap any known monitor, i.e. it would be
/// invisible after a monitor change.
pub fn is_offscreen(placement: WindowPlacement, monitors: &[MonitorRect]) -> bool {
    if monitors.is_empty() {
        return false;
    }
    !monitors.iter().any(|monitor| monitor.contains(placement))
}

/// Nudge a placement so at least its top-left corner is on the primary monitor.
/// Used by "reset position" and when a saved position belongs to a monitor that
/// is no longer connected.
pub fn clamp_into_monitor(placement: WindowPlacement, monitor: MonitorRect) -> WindowPlacement {
    let width = placement.width.min(monitor.width).max(1);
    let height = placement.height.min(monitor.height).max(1);
    let max_x = monitor.x + (monitor.width - width) as i32;
    let max_y = monitor.y + (monitor.height - height) as i32;

    WindowPlacement {
        x: placement.x.clamp(monitor.x, max_x.max(monitor.x)),
        y: placement.y.clamp(monitor.y, max_y.max(monitor.y)),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offset() -> (f64, f64) {
        (16.0, 16.0)
    }

    #[test]
    fn all_anchors_land_inside_the_monitor_at_2x_scale() {
        let monitor = MonitorRect::new(-1920, 0, 1920, 1080);
        let cases = [
            (Anchor::TopLeft, -1888, 32),
            (Anchor::TopCenter, -1160, 32),
            (Anchor::TopRight, -432, 32),
            (Anchor::MiddleLeft, -1888, 440),
            (Anchor::Center, -1160, 440),
            (Anchor::MiddleRight, -432, 440),
            (Anchor::BottomLeft, -1888, 848),
            (Anchor::BottomCenter, -1160, 848),
            (Anchor::BottomRight, -432, 848),
        ];

        for (anchor, expected_x, expected_y) in cases {
            let placement =
                compute_placement(monitor, (200.0, 100.0), 2.0, anchor, offset()).unwrap();
            assert_eq!(
                (placement.x, placement.y),
                (expected_x, expected_y),
                "{anchor:?}"
            );
            assert_eq!((placement.width, placement.height), (400, 200));
        }
    }

    #[test]
    fn fractional_scale_rounds_up_and_keeps_the_anchored_corner() {
        let monitor = MonitorRect::new(1920, -500, 1280, 720);
        let placement = compute_placement(
            monitor,
            (201.0, 101.0),
            1.25,
            Anchor::BottomRight,
            (24.0, 24.0),
        )
        .unwrap();

        assert_eq!(placement.width, 252);
        assert_eq!(placement.height, 127);
        assert_eq!(placement.x + placement.width as i32, 1920 + 1280 - 30);
        assert_eq!(placement.y + placement.height as i32, -500 + 720 - 30);
    }

    #[test]
    fn oversized_windows_and_offsets_are_clamped_on_screen() {
        let monitor = MonitorRect::new(0, 0, 1280, 720);
        let placement = compute_placement(
            monitor,
            (5000.0, 5000.0),
            1.0,
            Anchor::BottomRight,
            (9000.0, 9000.0),
        )
        .unwrap();

        assert_eq!((placement.x, placement.y), (0, 0));
        assert_eq!((placement.width, placement.height), (1280, 720));
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        let monitor = MonitorRect::new(0, 0, 1920, 1080);
        for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                compute_placement(monitor, (width, 100.0), 1.0, Anchor::Center, offset()).is_err()
            );
        }
        for scale in [0.0, -2.0, f64::NAN] {
            assert!(
                compute_placement(monitor, (200.0, 100.0), scale, Anchor::Center, offset())
                    .is_err()
            );
        }
        assert!(compute_placement(
            MonitorRect::new(0, 0, 0, 0),
            (200.0, 100.0),
            1.0,
            Anchor::Center,
            offset()
        )
        .is_err());
    }

    #[test]
    fn offscreen_detection_and_clamping() {
        let primary = MonitorRect::new(0, 0, 1920, 1080);
        let secondary = MonitorRect::new(1920, 0, 1600, 900);
        let monitors = [primary, secondary];

        let visible = WindowPlacement {
            x: 100,
            y: 100,
            width: 320,
            height: 440,
        };
        assert!(!is_offscreen(visible, &monitors));

        let stale = WindowPlacement {
            x: 5000,
            y: 5000,
            width: 320,
            height: 440,
        };
        assert!(is_offscreen(stale, &monitors));
        assert!(
            !is_offscreen(stale, &[]),
            "unknown monitor layout must not hide the window"
        );

        let clamped = clamp_into_monitor(stale, primary);
        assert_eq!((clamped.x, clamped.y), (1600, 640));
        assert!(!is_offscreen(clamped, &monitors));

        let oversized = clamp_into_monitor(
            WindowPlacement {
                x: -999,
                y: -999,
                width: 4000,
                height: 3000,
            },
            secondary,
        );
        assert_eq!((oversized.x, oversized.y), (1920, 0));
        assert_eq!((oversized.width, oversized.height), (1600, 900));
    }

    #[test]
    fn default_anchor_is_bottom_right() {
        assert_eq!(Anchor::default(), Anchor::BottomRight);
        let serialized = serde_json::to_string(&Anchor::BottomRight).unwrap();
        assert_eq!(serialized, "\"bottom-right\"");
        let parsed: Anchor = serde_json::from_str("\"top-center\"").unwrap();
        assert_eq!(parsed, Anchor::TopCenter);
    }
}
