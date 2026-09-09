// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The module marks, drawn rather than typed.
//
// These are ports of the symbols the macOS app uses - cpu, square.stack.3d.up,
// memorychip, internaldrive, network, thermometer.medium, cloud.sun,
// rectangle.3.group - redrawn here as vectors. Ports, not copies: SF Symbols
// are Apple's artwork under a license that covers Apple platforms, so not one
// byte of them is in this program. What is ported is the idea of each mark and
// the visual language they share, which is the part that makes the two apps
// look like one product.
//
// Drawn rather than set from a font, and that was the whole reason to write
// this. Segoe Fluent Icons was the alternative and it decides what a mark can
// be: there is no cloud-with-sun in it, so the weather mark had to be composed
// from two glyphs at offsets tuned for the strip, and in a window that centers
// its glyphs those offsets put the sun on top of the cloud. Two glyphs also
// cannot be two colors. A drawing has neither problem - it is exactly the shape
// we want, in exactly the colors we want, at any size.
//
// Everything is specified in a unit square and scaled to the box it is given,
// so one definition serves the 16-point mark in a list row and the 28-point one
// in a pane header, and neither is a separate asset to keep in step.

use barometer_core::module::ModuleId;

use super::gdi::Canvas;
use super::geometry::Rect;
use super::theme::Color;

/// Stroke weight, as a fraction of the mark's box.
///
/// One number for every mark, which is what optical consistency means here: a
/// row of symbols drawn at different weights reads as a row of symbols from
/// different families, however carefully each one is drawn on its own.
/// Heavier than it looks like it should be on paper. SF Symbols at 16 point
/// carry about a pixel and a half, which is a tenth of the box, and anything
/// lighter goes spidery the moment it is drawn small and antialiased.
const STROKE: f32 = 0.095;

/// A point in the unit square, placed inside a box.
fn at(box_: Rect, x: f32, y: f32) -> (f32, f32) {
    (box_.x + x * box_.w, box_.y + y * box_.h)
}

/// A rectangle in the unit square, placed inside a box.
fn sub(box_: Rect, x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(box_.x + x * box_.w, box_.y + y * box_.h, w * box_.w, h * box_.h)
}

/// The stroke width for a mark of this size.
fn stroke(box_: Rect) -> f32 {
    box_.w * STROKE
}

/// Draws a module's mark, filling the box it is given.
///
/// `accent` is the module's own hue and `ink` is what a detail drawn over it
/// should be. Most marks use one; the weather uses both, because a sun behind a
/// cloud is the one mark here that is genuinely two things.
pub fn module(canvas: &Canvas, box_: Rect, id: ModuleId, accent: Color) {
    match id {
        ModuleId::Cpu => cpu(canvas, box_, accent),
        ModuleId::Gpu => gpu(canvas, box_, accent),
        ModuleId::Memory => memory(canvas, box_, accent),
        ModuleId::Disks => disks(canvas, box_, accent),
        ModuleId::Network => network(canvas, box_, accent),
        ModuleId::Sensors => sensors(canvas, box_, accent),
        ModuleId::Weather => weather(canvas, box_, accent),
    }
}

/// A processor package: a square die inside a square carrier, with pins out of
/// all four sides. The Mac's "cpu".
pub fn cpu(canvas: &Canvas, box_: Rect, color: Color) {
    let w = stroke(box_);
    canvas.stroke_round_icon(sub(box_, 0.22, 0.22, 0.56, 0.56), box_.w * 0.06, STROKE, color);
    canvas.stroke_round_icon(sub(box_, 0.38, 0.38, 0.24, 0.24), box_.w * 0.03, STROKE, color);

    // Three pins a side, at a third and two thirds and the middle of the edge.
    // Drawn as separate strokes rather than one path: a pin is a stub with two
    // round ends, and a path would join them around the corners.
    for t in [0.34_f32, 0.5, 0.66] {
        // Top and bottom.
        canvas.stroke_poly(&[at(box_, t, 0.10), at(box_, t, 0.22)], STROKE, color, false);
        canvas.stroke_poly(&[at(box_, t, 0.78), at(box_, t, 0.90)], STROKE, color, false);
        // Left and right.
        canvas.stroke_poly(&[at(box_, 0.10, t), at(box_, 0.22, t)], STROKE, color, false);
        canvas.stroke_poly(&[at(box_, 0.78, t), at(box_, 0.90, t)], STROKE, color, false);
    }
    let _ = w;
}

/// Three planes stacked in perspective. The Mac's "square.stack.3d.up", and
/// deliberately not a monitor: the module measures the card, not the display.
pub fn gpu(canvas: &Canvas, box_: Rect, color: Color) {
    // The top plane, a square seen at an angle, so a rhombus.
    canvas.stroke_poly(
        &[
            at(box_, 0.50, 0.10),
            at(box_, 0.90, 0.32),
            at(box_, 0.50, 0.54),
            at(box_, 0.10, 0.32),
        ],
        STROKE,
        color,
        true,
    );
    // Two planes beneath it, each only as much as would be visible: the near
    // edges, meeting under the point of the one above.
    for drop in [0.19_f32, 0.36] {
        canvas.stroke_poly(
            &[
                at(box_, 0.10, 0.32 + drop),
                at(box_, 0.50, 0.54 + drop),
                at(box_, 0.90, 0.32 + drop),
            ],
            STROKE,
            color,
            false,
        );
    }
}

/// A memory chip: a body with contacts down two sides and rows inside. The
/// Mac's "memorychip".
///
/// Kept visibly apart from the processor above, which is the same idea drawn
/// the same way. Pins on two sides rather than four, and rows instead of a
/// die, so the two never read as each other in a list.
pub fn memory(canvas: &Canvas, box_: Rect, color: Color) {
    canvas.stroke_round_icon(sub(box_, 0.24, 0.20, 0.52, 0.60), box_.w * 0.07, STROKE, color);

    // Two rows inside, the chip's own markings.
    for y in [0.38_f32, 0.54] {
        canvas.stroke_poly(&[at(box_, 0.36, y), at(box_, 0.64, y)], STROKE * 0.85, color, false);
    }

    // Contacts down the left and right, three a side.
    for t in [0.32_f32, 0.50, 0.68] {
        canvas.stroke_poly(&[at(box_, 0.10, t), at(box_, 0.24, t)], STROKE, color, false);
        canvas.stroke_poly(&[at(box_, 0.76, t), at(box_, 0.90, t)], STROKE, color, false);
    }
}

/// A drive: a wide body with a spindle at one end and a light at the other.
/// The Mac's "internaldrive".
pub fn disks(canvas: &Canvas, box_: Rect, color: Color) {
    let body = sub(box_, 0.10, 0.28, 0.80, 0.44);
    canvas.stroke_round_icon(body, box_.w * 0.10, STROKE, color);
    // The spindle, left of center as it sits in a real drive.
    canvas.stroke_circle_width(
        box_.x + 0.34 * box_.w,
        box_.y + 0.50 * box_.h,
        box_.w * 0.09,
        STROKE,
        color,
    );
    // The activity light: filled, because a ring this small closes up into a
    // smudge and a dot is what the eye expects there anyway.
    canvas.fill_circle(
        box_.x + 0.68 * box_.w,
        box_.y + 0.50 * box_.h,
        box_.w * 0.045,
        color,
    );
}

/// A globe with a meridian and a parallel. The Mac's "network".
pub fn network(canvas: &Canvas, box_: Rect, color: Color) {
    let r = 0.36;
    canvas.stroke_circle_width(
        box_.x + 0.5 * box_.w,
        box_.y + 0.5 * box_.h,
        box_.w * r,
        STROKE,
        color,
    );
    // The meridian: a narrow ellipse, drawn as its two halves so it is two
    // arcs of a sphere rather than one flat oval sitting on top of it.
    let meridian = sub(box_, 0.5 - 0.155, 0.5 - r, 0.31, r * 2.0);
    canvas.stroke_arcs(&[(meridian, 90.0, 180.0)], STROKE * 0.9, color, false);
    canvas.stroke_arcs(&[(meridian, 270.0, 180.0)], STROKE * 0.9, color, false);
    // The equator, straight because the globe is seen edge on.
    canvas.stroke_poly(
        &[at(box_, 0.5 - r, 0.5), at(box_, 0.5 + r, 0.5)],
        STROKE * 0.9,
        color,
        false,
    );
}

/// A thermometer with the mercury up and graduations beside it. The Mac's
/// "thermometer.medium".
pub fn sensors(canvas: &Canvas, box_: Rect, color: Color) {
    // The stem, a capsule. Its radius is half its width, so the ends are
    // semicircles and the bulb below can meet it without a seam.
    let stem = sub(box_, 0.38, 0.10, 0.24, 0.58);
    canvas.stroke_round_icon(stem, stem.w / 2.0, STROKE, color);

    // The bulb, filled, and the column of mercury reaching it. Filled because
    // a thermometer reads as a thermometer from the weight at the bottom, and
    // an outlined bulb at sixteen pixels is a small empty circle.
    let cx = box_.x + 0.50 * box_.w;
    canvas.fill_circle(cx, box_.y + 0.78 * box_.h, box_.w * 0.155, color);
    canvas.stroke_poly(
        &[at(box_, 0.50, 0.42), at(box_, 0.50, 0.74)],
        STROKE * 1.1,
        color,
        false,
    );

    // Graduations, shorter than the stem is wide so they read as marks on it
    // rather than as something attached.
    for y in [0.22_f32, 0.32, 0.42] {
        canvas.stroke_poly(&[at(box_, 0.66, y), at(box_, 0.78, y)], STROKE * 0.8, color, false);
    }
}

/// A cloud with the sun behind it. The Mac's "cloud.sun".
///
/// The mark this module exists to get right. Segoe Fluent has no such glyph,
/// so it used to be composed from a cloud and a sun at offsets tuned for the
/// strip - and in a window that centers its glyphs instead of drawing them from
/// a corner, those offsets put the sun on top of the cloud.
///
/// Here the two cannot collide, because the sun is placed where the cloud is
/// not: it sits high and right, the cloud low and left, and the layout is part
/// of the drawing rather than a pair of offsets that happen to work.
pub fn weather(canvas: &Canvas, box_: Rect, color: Color) {
    let sun_x = box_.x + 0.74 * box_.w;
    let sun_y = box_.y + 0.22 * box_.h;
    let sun_r = box_.w * 0.10;

    // Rays first, so the disc laps over their inner ends and each one reads as
    // coming out from behind it.
    //
    // Five, not eight, and none pointing down-left: that is where the cloud is,
    // and a ray disappearing under an outlined shape reads as a mistake rather
    // than as depth. A filled mark could overlap them properly; an outlined one
    // has to be laid out so it never needs to.
    for (dx, dy) in [(0.0_f32, -1.0_f32), (0.71, -0.71), (1.0, 0.0), (0.71, 0.71), (-0.71, -0.71)] {
        let inner = sun_r * 1.5;
        let outer = sun_r * 2.0;
        canvas.stroke_poly(
            &[
                (sun_x + dx * inner, sun_y + dy * inner),
                (sun_x + dx * outer, sun_y + dy * outer),
            ],
            STROKE * 0.85,
            color,
            false,
        );
    }
    canvas.stroke_circle_width(sun_x, sun_y, sun_r, STROKE * 0.9, color);

    // The cloud: three lobes and a close. GDI+ runs a line from the end of one
    // arc to the start of the next and, on closing, from the last point back to
    // the first - which is what draws the flat underside without listing it.
    //
    // Angles are GDI+'s: zero along +x, positive sweeping clockwise on screen
    // because y runs down. So 180 through 360 is the top of a circle.
    let left = sub(box_, 0.04, 0.50, 0.32, 0.32);
    let middle = sub(box_, 0.20, 0.44, 0.40, 0.40);
    let right = sub(box_, 0.42, 0.54, 0.28, 0.28);
    canvas.stroke_arcs(
        &[(left, 180.0, 145.0), (middle, 195.0, 150.0), (right, 250.0, 110.0)],
        STROKE,
        color,
        true,
    );
}

/// Several readings gathered into one item: a frame with three tiles in it.
/// The Mac's "rectangle.3.group".
pub fn stack(canvas: &Canvas, box_: Rect, color: Color) {
    canvas.stroke_round_icon(sub(box_, 0.10, 0.18, 0.80, 0.64), box_.w * 0.09, STROKE, color);
    // One across the top, two side by side beneath, which is what tells this
    // apart from a plain framed rectangle at this size.
    canvas.fill_poly(
        &[
            at(box_, 0.24, 0.32),
            at(box_, 0.76, 0.32),
            at(box_, 0.76, 0.45),
            at(box_, 0.24, 0.45),
        ],
        color,
    );
    for x in [0.24_f32, 0.53] {
        canvas.fill_poly(
            &[
                at(box_, x, 0.55),
                at(box_, x + 0.23, 0.55),
                at(box_, x + 0.23, 0.68),
                at(box_, x, 0.68),
            ],
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_scales_with_its_box_rather_than_being_two_assets() {
        // The same definition serves the 16-point mark in a list row and the
        // 28-point one in a pane header. If this stopped holding, a header
        // would need its own drawing and the two would drift.
        let small = Rect::new(0.0, 0.0, 16.0, 16.0);
        let large = Rect::new(0.0, 0.0, 32.0, 32.0);
        assert_eq!(stroke(large), stroke(small) * 2.0);
        assert_eq!(at(large, 0.5, 0.5), (16.0, 16.0));
        assert_eq!(at(small, 0.5, 0.5), (8.0, 8.0));
    }

    #[test]
    fn a_mark_is_placed_relative_to_its_box_not_the_origin() {
        // Marks are drawn into a row that is somewhere down a scrolled pane,
        // so anything anchored at zero would draw at the top of the window.
        let box_ = Rect::new(100.0, 40.0, 20.0, 20.0);
        assert_eq!(at(box_, 0.0, 0.0), (100.0, 40.0));
        assert_eq!(at(box_, 1.0, 1.0), (120.0, 60.0));
        let inner = sub(box_, 0.25, 0.25, 0.5, 0.5);
        assert_eq!((inner.x, inner.y, inner.w, inner.h), (105.0, 45.0, 10.0, 10.0));
    }

    #[test]
    fn the_sun_and_the_cloud_do_not_overlap() {
        // The bug this module was written for. The composed-glyph weather mark
        // drew the sun on top of the cloud, because its offsets were tuned for
        // the strip, which draws a glyph from its corner, and the settings
        // window centers glyphs in a box instead.
        //
        // Here the sun's disc is high and right and the cloud's lobes are low
        // and left, so they cannot collide however the mark is scaled. Checked
        // rather than assumed, because a later nudge to either would put it
        // straight back.
        let box_ = Rect::new(0.0, 0.0, 100.0, 100.0);

        // The sun's disc, with its rays, as a box.
        let sun_r = box_.w * 0.10;
        let reach = sun_r * 2.0;
        let sun_bottom = 22.0 + reach;
        let sun_left = 74.0 - reach;

        // The topmost point of the cloud is the top of its middle lobe.
        let middle = sub(box_, 0.20, 0.44, 0.40, 0.40);
        let cloud_top = middle.y;
        // And its rightmost is the right edge of the right lobe.
        let right = sub(box_, 0.42, 0.54, 0.28, 0.28);
        let cloud_right = right.right();

        // They may share neither a row nor a column of the box: the sun ends
        // above the cloud begins, or starts right of where the cloud ends.
        assert!(
            sun_bottom < cloud_top || sun_left > cloud_right,
            "sun (bottom {sun_bottom}, left {sun_left}) overlaps cloud (top {cloud_top}, right {cloud_right})"
        );
    }

    #[test]
    fn every_module_has_a_mark() {
        // A module added to ModuleId::ALL without one here would draw nothing
        // at all, which reads as a layout bug rather than a missing icon.
        // `module` matches exhaustively, so this is really a check that the
        // match has no catch-all arm quietly swallowing the new one.
        for id in ModuleId::ALL {
            let named = matches!(
                id,
                ModuleId::Cpu
                    | ModuleId::Gpu
                    | ModuleId::Memory
                    | ModuleId::Disks
                    | ModuleId::Network
                    | ModuleId::Sensors
                    | ModuleId::Weather
            );
            assert!(named, "{id:?} has no mark");
        }
    }
}
