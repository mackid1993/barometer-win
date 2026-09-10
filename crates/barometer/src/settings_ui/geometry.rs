// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Rectangles in device-independent pixels.
//
// Everything in the settings window is laid out in DIPs and converted at the
// point of drawing, which is the only arrangement that survives the window
// being dragged between a 100% and a 150% monitor: the layout is the same
// numbers, and only the scale they are multiplied by changes.

/// A rectangle in DIPs.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center_y(&self) -> f32 {
        self.y + self.h / 2.0
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Shrunk by `d` on every side. A negative `d` grows it.
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The device pixels this covers at a scale.
    ///
    /// Edges are rounded independently rather than the origin and size, so
    /// two rectangles that share an edge in DIPs share it in pixels too and
    /// neither a gap nor an overlap appears between them at 125%.
    pub fn px(&self, scale: f32) -> PxRect {
        PxRect {
            left: to_px(self.x, scale),
            top: to_px(self.y, scale),
            right: to_px(self.right(), scale),
            bottom: to_px(self.bottom(), scale),
        }
    }
}

/// A rectangle in device pixels.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

pub fn to_px(dip: f32, scale: f32) -> i32 {
    (dip * scale).round() as i32
}

pub fn to_dip(px: i32, scale: f32) -> f32 {
    px as f32 / scale
}

/// A DIP offset held to a whole device pixel.
///
/// For offsets that move a whole picture: a page's scroll, a chart's
/// sideways position. Every element converts itself to pixels on its own,
/// edge by edge, through `to_px`'s round - so an offset carrying a fraction
/// rounds each element independently, and two rows an exact distance apart
/// in DIPs land 21 pixels apart on one frame and 22 on the next. Measured on
/// a card's caption and the value under it: the gap alternates on every
/// frame of an overscroll's spring-back, and on every wheel notch from a
/// touchpad, whose deltas are fractions of a notch rather than whole ones.
/// Snapped once, before anything is offset by it, the whole picture moves by
/// a whole number of pixels and nothing inside it can move against anything
/// else.
pub fn snap_dip(dip: f32, scale: f32) -> f32 {
    (dip * scale).round() / scale
}

/// A one-DIP stroke in device pixels: one pixel up to 150%, two from 200%.
///
/// The design document's rule (section 9.8). At 150% a 1.5-pixel line drawn
/// as two pixels reads heavy beside the one-pixel strokes Windows itself
/// draws at that scale, and drawn as one it matches them.
pub fn hairline_px(scale: f32) -> i32 {
    if scale >= 2.0 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_is_half_open() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(39.9, 59.9));
        assert!(!r.contains(40.0, 30.0));
        assert!(!r.contains(20.0, 60.0));
        assert!(!r.contains(9.9, 30.0));
    }

    #[test]
    fn inset_shrinks_every_side_and_a_negative_inset_grows() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(r.inset(2.0), Rect::new(2.0, 2.0, 6.0, 6.0));
        assert_eq!(r.inset(-1.0), Rect::new(-1.0, -1.0, 12.0, 12.0));
    }

    #[test]
    fn adjacent_rectangles_share_their_pixel_edge_at_a_fractional_scale() {
        // At 125% a row 52 DIP tall is 65 pixels; the next row must start
        // exactly where this one ends, with no seam and no double line.
        let a = Rect::new(0.0, 0.0, 100.0, 52.0).px(1.25);
        let b = Rect::new(0.0, 52.0, 100.0, 52.0).px(1.25);
        assert_eq!(a.bottom, b.top);
        assert_eq!(a.height(), 65);
    }

    #[test]
    fn intersection_needs_overlap_not_touch() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(a.intersects(&Rect::new(5.0, 5.0, 10.0, 10.0)));
        assert!(!a.intersects(&Rect::new(10.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn a_snapped_offset_moves_every_row_of_a_card_by_the_same_whole_pixels() {
        // The measurement this exists for. A caption at 112 DIP with its
        // value 21.5 below it, shifted by a touchpad's fractional notches:
        // unsnapped the gap between them alternates between 21 and 22
        // pixels from frame to frame, which is the shimmer. Snapped, the
        // gap is whatever it is and it never changes.
        let (caption, value) = (112.0f32, 133.5f32);
        let mut gaps = Vec::new();
        let mut raw_gaps = Vec::new();
        let mut offset = 0.0f32;
        for delta in [-7.0f32, -13.0, -9.0, -11.0, -8.0, -14.0, -6.0, -12.0] {
            offset += delta / 120.0 * 48.0;
            let snapped = snap_dip(offset, 1.0);
            gaps.push(to_px(value + snapped, 1.0) - to_px(caption + snapped, 1.0));
            raw_gaps.push(to_px(value + offset, 1.0) - to_px(caption + offset, 1.0));
        }
        assert!(gaps.windows(2).all(|pair| pair[0] == pair[1]), "the gap breathed: {gaps:?}");
        assert!(raw_gaps.windows(2).any(|pair| pair[0] != pair[1]), "nothing to fix: {raw_gaps:?}");
    }

    #[test]
    fn snapping_lands_on_whole_pixels_at_every_scale_and_keeps_the_sign() {
        for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
            for dip in [-37.4f32, -0.2, 0.0, 3.2, 48.0, 121.9] {
                let snapped = snap_dip(dip, scale);
                let pixels = snapped * scale;
                assert!((pixels - pixels.round()).abs() < 1e-3, "{dip} at {scale} gave {pixels}px");
                assert!((snapped - dip).abs() <= 0.5 / scale + 1e-3, "{dip} at {scale} moved to {snapped}");
            }
        }
    }

    #[test]
    fn hairlines_stay_one_pixel_until_two_hundred_percent() {
        assert_eq!(hairline_px(1.0), 1);
        assert_eq!(hairline_px(1.5), 1);
        assert_eq!(hairline_px(2.0), 2);
    }

    #[test]
    fn dip_and_pixel_conversions_round_to_nearest() {
        assert_eq!(to_px(12.0, 1.25), 15);
        assert_eq!(to_px(9.0, 1.5), 14);
        assert!((to_dip(48, 1.5) - 32.0).abs() < f32::EPSILON);
    }
}
