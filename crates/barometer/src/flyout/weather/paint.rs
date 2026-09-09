// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The pictures the weather flyout draws for itself, as Painters the chrome
// calls: the sky behind the current conditions, a condition's mark in
// color, the hourly chart, the moon, the sun on its arc, a wind arrow, a
// pressure gauge and a day's temperature range.
//
// Geometry comes from chart.rs and marks.rs already worked out in DIPs; this
// file only decides what color each piece is and hands it to the surface.

use barometer_core::weather::badge::Condition;
use barometer_core::weather::glyph;

use super::chart::{self, Geometry};
use super::marks::{MoonDisc, SunPath};
use super::sky;
use crate::flyout::paint::Surface;
use crate::flyout::ui::{Painter, Style, CARD_RADIUS};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::theme::{Color, BLACK, WHITE};

/// The sun's color wherever it is drawn as a thing rather than a glyph.
const AMBER: Color = Color(0xFBBF24);

/// The sky-tinted card behind the current conditions, from
/// CurrentWeatherCard: a diagonal gradient, a sheen from white at the top
/// to a shade at the bottom, a hairline of white, and a shadow.
#[derive(Debug)]
pub struct SkyCard {
    pub sky: (Color, Color),
}

impl Painter for SkyCard {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        surface.soft_shadow(rect, CARD_RADIUS);
        surface.fill_round_gradient(rect, CARD_RADIUS, (self.sky.0, 255), (self.sky.1, 255), 45.0);
        surface.fill_round_stops(
            rect,
            CARD_RADIUS,
            &[(0.0, WHITE, 46), (0.5, WHITE, 0), (1.0, BLACK, 31)],
            true,
        );
        surface.stroke_round_alpha(rect, CARD_RADIUS, WHITE, 56, 0.75);
    }
}

/// A condition's mark in white with a shadow, for the sky card.
#[derive(Debug)]
pub struct SkyMark {
    pub condition: Condition,
    pub size: f32,
}

/// The square a mark of `size` is drawn in, centered in the rectangle it
/// was laid out in. Both painters draw the fitted composition inside it, so
/// a mark with a sun peeking over its cloud is no taller than a plain one.
fn mark_cell(rect: Rect, size: f32) -> Rect {
    Rect::new(rect.x + (rect.w - size) / 2.0, rect.y + (rect.h - size) / 2.0, size, size)
}

/// Where one placed glyph lands in a cell: its top-left and its size in DIPs.
fn placed_in(cell: Rect, placed: &glyph::Placed) -> Rect {
    let size = cell.w * placed.size;
    Rect::new(cell.x + cell.w * placed.x, cell.y + cell.w * placed.y, size, size)
}

impl Painter for SkyMark {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let mark = glyph::composed(self.condition).fitted();
        let cell = mark_cell(rect, self.size);
        let base = placed_in(cell, &mark.base);
        surface.text_alpha(base, &mark.base.glyph.to_string(), Style::Icon(base.w), WHITE, 255, Align::Center, Some((3.0, 64)));
        if let Some(accent) = mark.accent {
            let small = placed_in(cell, &accent);
            surface.text_alpha(small, &accent.glyph.to_string(), Style::Icon(small.w), WHITE, 255, Align::Center, Some((2.0, 64)));
        }
    }
}

/// The headline temperature on the sky card, with its shadow.
#[derive(Debug)]
pub struct SkyValue {
    pub text: String,
    pub size: f32,
}

impl Painter for SkyValue {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        surface.text_alpha(rect, &self.text, Style::Display(self.size), WHITE, 255, Align::Right, Some((2.0, 64)));
    }
}

/// A condition's mark in its own colors on a card.
#[derive(Debug)]
pub struct Mark {
    pub condition: Condition,
    pub size: f32,
}

impl Painter for Mark {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let (base_ink, accent_ink) = sky::mark_inks(self.condition, surface.palette.theme.light);
        let mark = glyph::composed(self.condition).fitted();
        let cell = mark_cell(rect, self.size);
        let base = placed_in(cell, &mark.base);
        surface.glyph_at(base.x, base.y, mark.base.glyph, base.w, base_ink);
        if let Some(accent) = mark.accent {
            let small = placed_in(cell, &accent);
            surface.glyph_at(small.x, small.y, accent.glyph, small.w, accent_ink);
        }
    }
}

/// The hourly chart: rain bars, the temperature's wash and glowing line,
/// the labels every third hour, and the chosen hour when there is one.
#[derive(Debug)]
pub struct Chart {
    pub geometry: Geometry,
    pub selected: Option<usize>,
}

impl Painter for Chart {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let g = &self.geometry;
        let (ox, oy) = (rect.x, rect.y);
        let shift = |points: &[(f32, f32)]| -> Vec<(f32, f32)> { points.iter().map(|(x, y)| (x + ox, y + oy)).collect() };
        let accent = surface.accent;

        let selected = self.selected.and_then(|index| g.columns.get(index));
        if let Some(column) = selected {
            let band = Rect::new(ox + column.x - g.step / 2.0, oy + 4.0, g.step, rect.h - 8.0);
            surface.fill_round_alpha(band, 4.0, accent.primary, 36);
        }
        for bar in &g.bars {
            surface.fill_round_gradient(bar.offset(ox, oy), 2.0, (sky::RAIN, 230), (sky::RAIN_DEEP, 128), 90.0);
        }
        if g.area.len() >= 3 {
            surface.polygon_vertical(
                &shift(&g.area),
                (accent.secondary, 71),
                (accent.primary, 5),
                oy + g.area_top,
                oy + g.area_bottom,
            );
        }
        if g.line.len() >= 2 {
            let line = shift(&g.line);
            // Two wider passes at low alpha stand in for the Mac's blur.
            surface.polyline(&line, 5.0, accent.secondary, 40);
            surface.polyline(&line, 3.5, accent.secondary, 80);
            surface.polyline_gradient(&line, 2.0, accent.primary, accent.secondary, 255);
        }
        if let Some((x, y)) = selected.and_then(|column| column.dot) {
            surface.ellipse(x + ox, y + oy, 5.0, 5.0, accent.primary, 90);
            surface.ellipse(x + ox, y + oy, 3.0, 3.0, accent.primary, 255);
            surface.ring(x + ox, y + oy, 3.0, 3.0, 1.0, WHITE, 255);
        }

        let secondary = surface.palette.theme.text_secondary;
        let primary = surface.palette.theme.text_primary;
        for label in &g.labels {
            let block = |row: (f32, f32)| Rect::new(ox + label.x, oy + row.0, label.width, row.1);
            surface.text(block(chart::HOUR_ROW), &label.hour, Style::Caption, secondary, Align::Center);
            if let Some(condition) = label.condition {
                Mark { condition, size: 18.0 }.paint(surface, block(chart::MARK_ROW), false);
            }
            surface.text(block(chart::TEMPERATURE_ROW), &label.temperature, Style::CaptionStrong, primary, Align::Center);
            if let Some(probability) = &label.probability {
                let row = Rect::new(ox + label.x, rect.bottom() - 6.0 - chart::PROBABILITY_ROW_H, label.width, chart::PROBABILITY_ROW_H);
                surface.text(row, probability, Style::Caption, sky::rain_ink(surface.palette.theme.light), Align::Center);
            }
        }
    }
}

/// The moon at its phase.
#[derive(Debug)]
pub struct Moon {
    pub disc: MoonDisc,
}

impl Painter for Moon {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let (lit, dark) = sky::moon_faces(surface.palette);
        let r = rect.w.min(rect.h) / 2.0;
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        surface.ellipse(cx, cy, r, r, dark, 255);
        // GDI+ angles run clockwise from three o'clock, so the right half
        // is the half-turn starting straight up.
        let start = if self.disc.lit_right { -90.0 } else { 90.0 };
        surface.pie(cx, cy, r, r, start, 180.0, lit, 255);
        let half = r * self.disc.terminator;
        if half > 0.5 {
            surface.ellipse(cx, cy, half, r, if self.disc.ellipse_lit { lit } else { dark }, 255);
        }
        surface.ring(cx, cy, r, r, 1.0, lit, 70);
    }
}

/// The sun's arc across the day, with the sun where it is now.
#[derive(Debug)]
pub struct Sun {
    pub path: SunPath,
}

impl Painter for Sun {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let path = &self.path;
        let arc = path.arc;
        let (cx, cy) = (rect.x + arc.x + arc.w / 2.0, rect.y + arc.y + arc.h / 2.0);
        let (rx, ry) = (arc.w / 2.0, arc.h / 2.0);
        let dim = sky::arc(surface.palette);
        // The whole arc, dashed, from nine o'clock over the top to three.
        surface.arc(cx, cy, rx, ry, 180.0, 180.0, 1.0, dim, 255, true);
        surface.hline(rect.x + 8.0, rect.right() - 8.0, rect.y + path.horizon, surface.palette.divider);
        let (sx, sy) = (rect.x + path.sun.0, rect.y + path.sun.1);
        if path.up {
            surface.arc(cx, cy, rx, ry, 180.0, 180.0 * path.progress, 2.0, AMBER, 255, false);
            surface.ellipse(sx, sy, 10.0, 10.0, AMBER, 50);
            surface.ellipse(sx, sy, 6.0, 6.0, AMBER, 255);
        } else {
            surface.ring(sx, sy, 4.0, 4.0, 1.0, dim, 255);
        }
    }
}

/// An arrow pointing where the wind is going.
#[derive(Debug)]
pub struct Compass {
    pub bearing: f64,
}

impl Painter for Compass {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let r = rect.w.min(rect.h) / 2.0;
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        surface.ring(cx, cy, r, r, 1.0, surface.palette.theme.text_secondary, 90);
        let points = super::marks::compass_arrow(cx, cy, r * 0.8, self.bearing);
        surface.polygon(&points, surface.accent.primary, 255);
    }
}

/// A gauge reading a fraction of its dial.
#[derive(Debug)]
pub struct Gauge {
    pub fraction: f32,
}

impl Painter for Gauge {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        let r = rect.w.min(rect.h) / 2.0;
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let dial = super::marks::gauge(cx, cy, r, self.fraction);
        let theme = &surface.palette.theme;
        surface.arc(cx, cy, r, r, dial.start, dial.sweep, 2.0, theme.text_secondary, 90, false);
        let filled = dial.sweep * self.fraction.clamp(0.0, 1.0);
        if filled > 0.0 {
            surface.arc(cx, cy, r, r, dial.start, filled, 2.0, surface.accent.primary, 255, false);
        }
        surface.line(cx, cy, dial.needle.0, dial.needle.1, 1.5, theme.text_primary, 255);
        surface.ellipse(cx, cy, 1.5, 1.5, theme.text_primary, 255);
    }
}

/// A day's temperature range as a capsule colored by how warm it is.
#[derive(Debug)]
pub struct Range {
    pub stops: Vec<(f32, Color)>,
    pub glow: Color,
}

impl Painter for Range {
    fn paint(&self, surface: &mut Surface, rect: Rect, _hover: bool) {
        surface.fill_round_alpha(rect.inset(-1.5), rect.h / 2.0 + 1.5, self.glow, 55);
        let stops: Vec<(f32, Color, u8)> = self.stops.iter().map(|(position, color)| (*position, *color, 255)).collect();
        surface.fill_round_stops(rect, rect.h / 2.0, &stops, false);
    }
}
