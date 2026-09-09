// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Where a flyout opens, ported from PopoverPlacement.swift and
// AttachedPanel.show(relativeTo:preferredEdge:on:).
//
// Screen pixels throughout, because that is what the taskbar, the monitor's
// work area and the strip's column rectangles are all reported in; the one
// DIP figure, the gap, is converted by the caller at the monitor's scale.
// Nothing here has a window handle, which is what lets the Mac tests that
// pin this behavior - a panel at every corner of every display stays on
// that display - run here without a display.

use barometer_core::taskbar::Edge;

use crate::settings_ui::geometry::PxRect;

/// Between the panel and the taskbar, and between the panel and the edge of
/// the work area. The Mac's 8, docs/ui-design.md section 10.
pub const GAP_DIP: f32 = 8.0;

fn width(rect: PxRect) -> i32 {
    rect.right - rect.left
}

fn height(rect: PxRect) -> i32 {
    rect.bottom - rect.top
}

fn inset(rect: PxRect, by: i32) -> PxRect {
    PxRect { left: rect.left + by, top: rect.top + by, right: rect.right - by, bottom: rect.bottom - by }
}

/// A frame moved and, if it must be, shrunk, until it lies inside `visible`.
///
/// The Mac's containedFrame: the size is capped at the visible area's first,
/// so a frame taller than the display does not get pushed off the top in
/// the course of keeping its bottom on. Origins may be negative; a second
/// monitor to the left of the first has them.
pub fn contained(frame: PxRect, visible: PxRect) -> PxRect {
    let w = width(frame).min(width(visible)).max(0);
    let h = height(frame).min(height(visible)).max(0);
    let left = frame.left.max(visible.left).min(visible.right - w);
    let top = frame.top.max(visible.top).min(visible.bottom - h);
    PxRect { left, top, right: left + w, bottom: top + h }
}

/// Where a panel of `size` opens for a strip column at `anchor`.
///
/// Centered on the column and `gap` away from the taskbar on the desktop
/// side: above a bottom taskbar, below a top one, beside a side one with
/// the panel centered on the column's height. Without a taskbar to ask -
/// Explorer mid-restart - it opens above the column, which is where a
/// taskbar nearly always is. Then it is held inside the work area inset by
/// the same gap, so a column near the screen's edge gets a panel that slid
/// along the taskbar rather than one hanging off the display.
pub fn place(
    anchor: PxRect,
    taskbar: Option<(PxRect, Edge)>,
    size: (i32, i32),
    work: PxRect,
    gap: i32,
) -> PxRect {
    let (w, h) = size;
    let center_x = (anchor.left + anchor.right) / 2;
    let center_y = (anchor.top + anchor.bottom) / 2;
    let (left, top) = match taskbar {
        Some((bar, Edge::Bottom)) => (center_x - w / 2, bar.top - gap - h),
        Some((bar, Edge::Top)) => (center_x - w / 2, bar.bottom + gap),
        Some((bar, Edge::Left)) => (bar.right + gap, center_y - h / 2),
        Some((bar, Edge::Right)) => (bar.left - gap - w, center_y - h / 2),
        None => (center_x - w / 2, anchor.top - gap - h),
    };
    contained(PxRect { left, top, right: left + w, bottom: top + h }, inset(work, gap))
}

/// How tall the panel is for its content, in DIPs.
///
/// The content plus the footer, capped at the Mac's maximum and at the work
/// area less a margin at each end; taller content scrolls inside.
pub fn panel_height(content: f32, footer: f32, work_h: f32, gap: f32, maximum: f32) -> f32 {
    (content + footer).min(maximum).min((work_h - 2.0 * gap).max(footer + 80.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(outer: PxRect, inner: PxRect) -> bool {
        inner.left >= outer.left && inner.top >= outer.top && inner.right <= outer.right && inner.bottom <= outer.bottom
    }

    /// The Mac's screenBounds test: oversized and off-screen frames fit
    /// displays with positive and negative origins.
    #[test]
    fn oversized_and_off_screen_frames_fit_displays_with_positive_and_negative_origins() {
        for visible in [
            PxRect { left: 0, top: 30, right: 1280, bottom: 720 },
            PxRect { left: -1920, top: -1080, right: 0, bottom: -30 },
        ] {
            for (left, top) in [(visible.left - 500, visible.top - 500), (visible.right + 500, visible.bottom + 500)] {
                let frame = PxRect { left, top, right: left + 380, bottom: top + 1500 };
                let fitted = contained(frame, visible);
                assert!(contains(visible, fitted), "{fitted:?} outside {visible:?}");
                assert_eq!(width(fitted), 380);
                assert_eq!(height(fitted), height(visible));
            }
        }
    }

    #[test]
    fn a_frame_already_inside_is_left_exactly_where_it_was() {
        let visible = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        let frame = PxRect { left: 700, top: 200, right: 1080, bottom: 900 };
        assert_eq!(contained(frame, visible), frame);
    }

    /// The Mac's corner test: a panel anchored at every corner of the work
    /// area stays on it, no taller than it asked to be.
    #[test]
    fn a_panel_anchored_at_every_corner_stays_on_the_display() {
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        let bar = PxRect { left: 0, top: 1032, right: 1920, bottom: 1080 };
        for x in [0, 1920 - 24] {
            for y in [0, 1032 - 24] {
                let anchor = PxRect { left: x, top: y, right: x + 24, bottom: y + 24 };
                for h in [300, 720, 1500] {
                    let frame = place(anchor, Some((bar, Edge::Bottom)), (380, h), work, 8);
                    assert!(contains(inset(work, 8), frame), "{frame:?} for anchor {anchor:?}");
                    assert!(height(frame) <= h);
                    assert!(frame.bottom <= bar.top - 8, "the panel never covers the taskbar");
                }
            }
        }
    }

    #[test]
    fn the_panel_is_centered_on_its_column_and_a_gap_off_the_taskbar() {
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        let anchor = PxRect { left: 1500, top: 1040, right: 1560, bottom: 1072 };
        let bottom = PxRect { left: 0, top: 1032, right: 1920, bottom: 1080 };
        let frame = place(anchor, Some((bottom, Edge::Bottom)), (380, 600), work, 8);
        assert_eq!((frame.left + frame.right) / 2, 1530);
        assert_eq!(frame.bottom, 1032 - 8);
        // A top taskbar hangs the panel under it.
        let top = PxRect { left: 0, top: 0, right: 1920, bottom: 48 };
        let work_below = PxRect { left: 0, top: 48, right: 1920, bottom: 1080 };
        let anchor = PxRect { left: 1500, top: 8, right: 1560, bottom: 40 };
        let frame = place(anchor, Some((top, Edge::Top)), (380, 600), work_below, 8);
        assert_eq!(frame.top, 48 + 8);
        assert_eq!((frame.left + frame.right) / 2, 1530);
    }

    #[test]
    fn a_column_near_the_edge_slides_the_panel_along_the_taskbar() {
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        let bar = PxRect { left: 0, top: 1032, right: 1920, bottom: 1080 };
        let anchor = PxRect { left: 1880, top: 1040, right: 1910, bottom: 1072 };
        let frame = place(anchor, Some((bar, Edge::Bottom)), (380, 600), work, 8);
        assert_eq!(frame.right, 1920 - 8);
        assert_eq!(frame.bottom, 1032 - 8);
    }

    #[test]
    fn a_side_taskbar_puts_the_panel_beside_it() {
        let work = PxRect { left: 62, top: 0, right: 1920, bottom: 1080 };
        let bar = PxRect { left: 0, top: 0, right: 62, bottom: 1080 };
        let anchor = PxRect { left: 10, top: 500, right: 52, bottom: 540 };
        let frame = place(anchor, Some((bar, Edge::Left)), (380, 600), work, 8);
        assert_eq!(frame.left, 62 + 8);
        assert_eq!((frame.top + frame.bottom) / 2, 520);
    }

    #[test]
    fn without_a_taskbar_to_ask_the_panel_opens_above_the_column() {
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        let anchor = PxRect { left: 1500, top: 1040, right: 1560, bottom: 1072 };
        let frame = place(anchor, None, (380, 600), work, 8);
        assert!(frame.bottom <= 1032 - 8);
        assert_eq!((frame.left + frame.right) / 2, 1530);
    }

    #[test]
    fn the_panel_is_as_tall_as_its_content_until_something_stops_it() {
        assert_eq!(panel_height(400.0, 40.0, 1000.0, 8.0, 720.0), 440.0);
        assert_eq!(panel_height(1400.0, 40.0, 1000.0, 8.0, 720.0), 720.0);
        assert_eq!(panel_height(1400.0, 40.0, 600.0, 8.0, 720.0), 584.0);
        // A comically short work area still leaves room for something.
        assert_eq!(panel_height(1400.0, 40.0, 50.0, 8.0, 720.0), 120.0);
    }
}
