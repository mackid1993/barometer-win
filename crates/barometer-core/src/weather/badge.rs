// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The weather mark, ported from
// Sources/MenuBarStatsUI/Rendering/WeatherBadgeRenderer.swift.
//
// There is no icon and no container. The temperature is drawn as plain text at
// full size, and the condition lives in the bands above and below it that the
// strip otherwise leaves empty: a cloud cap rests on the digits, rain or snow
// or a bolt hangs underneath, a sunburst fans around them, and a clear night
// tucks a crescent over the degree sign. The column costs no more width than
// the number would on its own.
//
// The fourteen conditions have to differ in **shape**, never only in color.
// The wet family is told apart by how many strokes hang under the cloud and
// how far they reach; sleet puts a flake between two strokes; a storm with
// rain flanks the bolt with two strokes; and the sun or moon beside a partial
// cloud says whether it is day. Anyone drawing these monochrome, or on a
// taskbar tinted an unhelpful color, still gets the forecast.
//
// This module produces geometry, not pixels. Everything is expressed against
// the digits' cap box in the same units the Swift used, which keeps the
// numbers here identical to the ones there and lets the whole set be tested
// without a drawing surface anywhere near it.
//
// Nothing draws that geometry today: at fourteen device pixels an icon font
// read better and won - see glyph.rs - and the GDI renderer that consumed
// these shapes is gone. `shapes`, `Shape` and `Palette` are kept for their
// tests and for the day the marks are drawn as vectors again; `Condition`,
// from this file, is what everything uses.

/// The conditions the marks cover, one per system symbol the Mac app names.
///
/// The order is the order the settings preview shows them in: clear, then the
/// cloud family from dry to wet, then the storms, then the fallback.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    ClearDay,
    ClearNight,
    PartlyCloudy,
    PartlyCloudyNight,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    HeavyRain,
    Sleet,
    Snow,
    Thunder,
    Thunderstorm,
    Unknown,
}

impl Condition {
    pub const ALL: [Condition; 14] = [
        Condition::ClearDay,
        Condition::ClearNight,
        Condition::PartlyCloudy,
        Condition::PartlyCloudyNight,
        Condition::Cloudy,
        Condition::Fog,
        Condition::Drizzle,
        Condition::Rain,
        Condition::HeavyRain,
        Condition::Sleet,
        Condition::Snow,
        Condition::Thunder,
        Condition::Thunderstorm,
        Condition::Unknown,
    ];

    /// The condition for a WMO code and whether it is daytime there.
    ///
    /// The ranges are Open-Meteo's published table and are ported exactly. A
    /// code outside them is Unknown rather than a guess: a wrong mark is worse
    /// than an honest one, because it is believed.
    pub fn for_code(code: u8, is_day: bool) -> Condition {
        match code {
            0 => {
                if is_day {
                    Condition::ClearDay
                } else {
                    Condition::ClearNight
                }
            }
            1 | 2 => {
                if is_day {
                    Condition::PartlyCloudy
                } else {
                    Condition::PartlyCloudyNight
                }
            }
            3 => Condition::Cloudy,
            45 | 48 => Condition::Fog,
            51..=57 => Condition::Drizzle,
            61 | 63 => Condition::Rain,
            65 | 80..=82 => Condition::HeavyRain,
            66 | 67 => Condition::Sleet,
            71..=77 | 85 | 86 => Condition::Snow,
            95 => Condition::Thunder,
            96 | 99 => Condition::Thunderstorm,
            _ => Condition::Unknown,
        }
    }

    /// Whether this condition wears the cloud cap over the digits.
    fn has_cloud(self) -> bool {
        !matches!(
            self,
            Condition::ClearDay | Condition::ClearNight | Condition::Unknown
        )
    }

    /// Whether the cap leaves room at its trailing end for a sun or a moon.
    fn partial(self) -> bool {
        matches!(
            self,
            Condition::PartlyCloudy | Condition::PartlyCloudyNight
        )
    }
}

/// Which of a mark's three shape colors a shape takes.
///
/// Named by role rather than by color so monochrome is a matter of resolving
/// them all to one value, and so the rain that shares a mark with a bolt or a
/// flake keeps its own color without the caller tracking why.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Ink {
    /// The cloud cap.
    Cloud,
    /// The condition detail: rays, crescent, flakes, bolt, fog, question mark.
    Mark,
    /// Rain that hangs beside something else.
    Rain,
}

/// One drawing instruction, in the box's coordinate space.
///
/// Y increases **downwards**, as it does in every Windows drawing API. The
/// Swift is written the other way up, so every constant that was `+` above the
/// box there is `-` here, and vice versa; that flip is the only edit made to
/// the numbers.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Shape {
    /// A rounded bar, filled.
    RoundedBar { x: f32, y: f32, width: f32, height: f32, radius: f32, ink: Ink },
    /// A filled circle.
    Disc { x: f32, y: f32, radius: f32, ink: Ink },
    /// A circle cut *out* of everything drawn so far, which is how a crescent
    /// is made: a disc with a disc taken out of it. Order matters, so this is
    /// a shape in the list rather than a flag on the disc before it.
    Bite { x: f32, y: f32, radius: f32 },
    /// A round-capped stroke.
    Stroke { x1: f32, y1: f32, x2: f32, y2: f32, width: f32, ink: Ink },
    /// A filled polygon, for the bolt.
    Polygon { points: [(f32, f32); 6], ink: Ink },
    /// A round-capped arc, for the question mark's hook. Angles in degrees,
    /// clockwise from three o'clock in screen coordinates.
    Arc { x: f32, y: f32, radius: f32, start: f32, sweep: f32, width: f32, ink: Ink },
}

/// The box a mark is measured from: the digits' cap height, centered in the
/// strip, with the padding either side that leaves room for rays and crescent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CapBox {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl CapBox {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    fn mid_x(&self) -> f32 {
        (self.left + self.right) / 2.0
    }

    fn mid_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }

    /// A point at a fraction of the way across.
    fn across(&self, fraction: f32) -> f32 {
        self.left + self.width() * fraction
    }
}

/// Horizontal room either side of the digits, for rays and the crescent.
pub const SIDE_PADDING: f32 = 2.0;

/// The shapes that make one condition's mark, in drawing order.
///
/// Order is load-bearing twice over: the rain that shares a mark with a bolt
/// or a flake is laid down first so the mark sits on top of it, and a Bite
/// cuts everything already drawn, so the disc it eats has to come before it.
pub fn shapes(condition: Condition, cap: CapBox) -> Vec<Shape> {
    let mut out = Vec::new();

    if condition.has_cloud() {
        cloud_cap(&mut out, cap, if condition.partial() { 6.5 } else { 0.0 });
    }

    // The rain that shares a mark with something else goes down first.
    match condition {
        Condition::Sleet => drops(&mut out, cap, &[0.2, 0.8], 1.6, 4.8, Ink::Rain),
        Condition::Thunderstorm => drops(&mut out, cap, &[0.12, 0.88], 1.6, 4.8, Ink::Rain),
        _ => {}
    }

    match condition {
        Condition::ClearDay => rays(&mut out, cap),
        Condition::ClearNight => moon(&mut out, cap),
        Condition::PartlyCloudy => sun_disc(&mut out, cap),
        Condition::PartlyCloudyNight => cloud_moon(&mut out, cap),
        Condition::Cloudy => {}
        Condition::Fog => fog_lines(&mut out, cap),
        Condition::Drizzle => drizzle_ticks(&mut out, cap),
        Condition::Rain => drops(&mut out, cap, &[0.24, 0.5, 0.76], 1.6, 4.8, Ink::Mark),
        Condition::HeavyRain => drops(&mut out, cap, &[0.1, 0.37, 0.63, 0.9], 1.2, 5.4, Ink::Mark),
        Condition::Sleet => flakes(&mut out, cap, &[0.5]),
        Condition::Snow => flakes(&mut out, cap, &[0.24, 0.5, 0.76]),
        Condition::Thunder | Condition::Thunderstorm => bolt(&mut out, cap),
        Condition::Unknown => question_mark(&mut out, cap),
    }

    out
}

/// A cloud resting on the digits: a low rounded bar with three puffs, the
/// width of the number.
fn cloud_cap(out: &mut Vec<Shape>, cap: CapBox, trailing_room: f32) {
    let height = 1.8;
    let top = cap.top - 1.2 - height;
    let width = cap.width() - trailing_room;
    out.push(Shape::RoundedBar {
        x: cap.left,
        y: top,
        width,
        height,
        radius: 0.9,
        ink: Ink::Cloud,
    });
    for (fraction, radius) in [(0.22f32, 2.2f32), (0.50, 2.9), (0.78, 2.3)] {
        out.push(Shape::Disc {
            x: cap.left + width * fraction,
            // The puffs sit proud of the bar's upper edge by nearly half their
            // radius, which is what gives the cloud its bumps rather than a
            // row of circles balanced on a line.
            y: top + radius * 0.55,
            radius,
            ink: Ink::Cloud,
        });
    }
}

/// Short rays fanning off the top and bottom of the number.
fn rays(out: &mut Vec<Shape>, cap: CapBox) {
    let (cx, cy) = (cap.mid_x(), cap.mid_y());
    for fraction in [0.06f32, 0.28, 0.5, 0.72, 0.94] {
        for edge_y in [cap.top - 1.0, cap.bottom + 1.0] {
            let x = cap.across(fraction);
            let (dx, dy) = (x - cx, edge_y - cy);
            let length = (dx * dx + dy * dy).sqrt();
            // A ray at the exact center has no direction to point in. It
            // cannot happen with these fractions, but dividing by it would be
            // a NaN that silently swallows the whole mark.
            if length <= f32::EPSILON {
                continue;
            }
            let (ux, uy) = (dx / length, dy / length);
            out.push(Shape::Stroke {
                x1: x + ux * 0.8,
                y1: edge_y + uy * 0.8,
                x2: x + ux * 3.0,
                y2: edge_y + uy * 3.0,
                width: 1.5,
                ink: Ink::Mark,
            });
        }
    }
}

/// A crescent tucked over the degree sign.
fn moon(out: &mut Vec<Shape>, cap: CapBox) {
    let r = 3.2;
    let (cx, cy) = (cap.right - r + 0.6, cap.top - r - 0.4);
    out.push(Shape::Disc { x: cx, y: cy, radius: r, ink: Ink::Mark });
    // Bitten from the lower right, so what is left leans back over the digits.
    out.push(Shape::Bite { x: cx + 2.0, y: cy + 1.2, radius: r });
}

/// A sun disc peeking out beside the cloud cap.
fn sun_disc(out: &mut Vec<Shape>, cap: CapBox) {
    let r = 2.5;
    let (cx, cy) = (cap.right - r - 0.2, cap.top - 2.9);
    out.push(Shape::Disc { x: cx, y: cy, radius: r, ink: Ink::Mark });
    for step in 0..6 {
        let angle = (step as f32) * 60.0 * std::f32::consts::PI / 180.0;
        let (c, s) = (angle.cos(), angle.sin());
        out.push(Shape::Stroke {
            x1: cx + c * (r + 0.7),
            y1: cy + s * (r + 0.7),
            x2: cx + c * (r + 1.9),
            y2: cy + s * (r + 1.9),
            width: 1.2,
            ink: Ink::Mark,
        });
    }
}

/// A crescent where the day mark puts its sun, so the pair read as one mark
/// with a different light in it.
fn cloud_moon(out: &mut Vec<Shape>, cap: CapBox) {
    let r = 2.8;
    let (cx, cy) = (cap.right - r - 0.2, cap.top - 2.9);
    out.push(Shape::Disc { x: cx, y: cy, radius: r, ink: Ink::Mark });
    // The bite comes from below and to the right, so the crescent leans back
    // toward the cloud rather than away from it, and stays inside the trailing
    // room the cap reserved.
    out.push(Shape::Bite { x: cx + 1.7, y: cy + 1.5, radius: r });
}

/// Rain: slanted strokes hanging under the cloud.
///
/// `near` and `far` are how far below the digits a stroke starts and ends. The
/// slant stays the same, so a longer reach is a longer stroke: three at the
/// default reach are rain, four at a longer one are heavy rain, and two make
/// room for something between them.
fn drops(out: &mut Vec<Shape>, cap: CapBox, fractions: &[f32], near: f32, far: f32, ink: Ink) {
    let lean = (far - near) * 0.2;
    for fraction in fractions {
        let x = cap.across(*fraction);
        out.push(Shape::Stroke {
            x1: x + lean,
            y1: cap.bottom + near,
            x2: x - lean,
            y2: cap.bottom + far,
            width: 1.5,
            ink,
        });
    }
}

/// Drizzle: five short ticks in two staggered rows, rain's slant at a third of
/// its length.
fn drizzle_ticks(out: &mut Vec<Shape>, cap: CapBox) {
    for (fraction, top) in [(0.24f32, 1.6f32), (0.5, 1.6), (0.76, 1.6), (0.37, 3.9), (0.63, 3.9)] {
        let x = cap.across(fraction);
        out.push(Shape::Stroke {
            x1: x + 0.25,
            y1: cap.bottom + top,
            x2: x - 0.25,
            y2: cap.bottom + top + 1.2,
            width: 1.5,
            ink: Ink::Mark,
        });
    }
}

/// Snow: six-armed flakes under the cloud.
fn flakes(out: &mut Vec<Shape>, cap: CapBox, fractions: &[f32]) {
    for fraction in fractions {
        let (cx, cy) = (cap.across(*fraction), cap.bottom + 3.4);
        // Three strokes through the center, which is six arms.
        for arm in 0..3 {
            let angle = (arm as f32) * std::f32::consts::PI / 3.0;
            let (c, s) = (angle.cos(), angle.sin());
            out.push(Shape::Stroke {
                x1: cx - c * 1.9,
                y1: cy - s * 1.9,
                x2: cx + c * 1.9,
                y2: cy + s * 1.9,
                width: 1.0,
                ink: Ink::Mark,
            });
        }
    }
}

/// The lightning bolt, as a filled polygon.
fn bolt(out: &mut Vec<Shape>, cap: CapBox) {
    let (cx, cy) = (cap.mid_x(), cap.bottom + 1.0);
    out.push(Shape::Polygon {
        points: [
            (cx + 2.2, cy),
            (cx - 0.8, cy + 2.6),
            (cx + 0.8, cy + 2.6),
            (cx - 2.0, cy + 5.6),
            (cx - 1.0, cy + 3.1),
            (cx - 2.6, cy + 3.1),
        ],
        ink: Ink::Mark,
    });
}

/// Fog: two lines under the digits, the lower one shorter.
fn fog_lines(out: &mut Vec<Shape>, cap: CapBox) {
    for (row, inset) in [(0usize, 1.0f32), (1, 3.5)] {
        let y = cap.bottom + 2.0 + (row as f32) * 2.6;
        out.push(Shape::Stroke {
            x1: cap.left + inset,
            y1: y,
            x2: cap.right - inset,
            y2: y,
            width: 1.5,
            ink: Ink::Mark,
        });
    }
}

/// A question mark where the cloud would sit, for a code the source has not
/// named.
///
/// The hook, a short tail and the dot. The tail stops well above the dot: at
/// this size a stem that reached it would close the gap that makes the mark
/// read as two parts.
fn question_mark(out: &mut Vec<Shape>, cap: CapBox) {
    let r = 1.6;
    let (cx, cy) = (cap.mid_x(), cap.top - 4.7);
    // From nine o'clock, clockwise and over the top, to six o'clock: the hook,
    // then a short tail down. Screen coordinates put the sweep the other way
    // round from the Swift, which is measuring angles up.
    out.push(Shape::Arc {
        x: cx,
        y: cy,
        radius: r,
        start: 180.0,
        sweep: 270.0,
        width: 1.4,
        ink: Ink::Mark,
    });
    out.push(Shape::Stroke {
        x1: cx,
        y1: cy,
        x2: cx,
        y2: cy + r + 0.4,
        width: 1.4,
        ink: Ink::Mark,
    });
    out.push(Shape::Disc { x: cx, y: cap.top - 1.2, radius: 0.8, ink: Ink::Mark });
}

/// The four colors one mark is drawn in.
///
/// `digits` is the temperature itself, which takes the condition's color only
/// where that color carries meaning - amber for sun, lavender for night - and
/// stays the module's own color otherwise, so a rainy afternoon does not turn
/// the number blue.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub digits: u32,
    pub cloud: u32,
    pub mark: u32,
    pub rain: u32,
}

impl Palette {
    /// Colors for a condition, or one color for all of it.
    ///
    /// `module` is the strip's own text color, used wherever the condition
    /// has nothing to say about what color the number should be.
    pub fn resolve(
        condition: Option<Condition>,
        dark: bool,
        monochrome: bool,
        module: u32,
    ) -> Palette {
        if monochrome {
            return Palette { digits: module, cloud: module, mark: module, rain: module };
        }
        let cloud = if dark { 0xC9CED6 } else { 0x8D96A3 };
        let rain = if dark { 0x4DA3FF } else { 0x1F6FE0 };
        let amber = if dark { 0xFFC53D } else { 0xA86E00 };
        let night = if dark { 0xC7C4FF } else { 0x5B54C9 };
        let ice = if dark { 0x8FD3FF } else { 0x2E86D6 };
        let storm_cloud = if dark { 0x9AA3B0 } else { 0x5F6873 };
        let bolt = if dark { 0xFFD23F } else { 0xE5A800 };

        let (digits, cloud, mark) = match condition {
            None | Some(Condition::Cloudy) | Some(Condition::Fog) | Some(Condition::Unknown) => {
                (module, cloud, cloud)
            }
            Some(Condition::ClearDay) => (amber, cloud, amber),
            Some(Condition::ClearNight) => (night, cloud, night),
            Some(Condition::PartlyCloudy) => (module, cloud, amber),
            Some(Condition::PartlyCloudyNight) => (module, cloud, night),
            Some(Condition::Drizzle) | Some(Condition::Rain) => (module, cloud, rain),
            Some(Condition::HeavyRain) => (module, storm_cloud, rain),
            Some(Condition::Sleet) | Some(Condition::Snow) => (module, cloud, ice),
            Some(Condition::Thunder) | Some(Condition::Thunderstorm) => (module, storm_cloud, bolt),
        };
        Palette { digits, cloud, mark, rain }
    }

    /// The digits, as a Windows COLORREF.
    ///
    /// COLORREF is 0x00BBGGRR where the palette is 0xRRGGBB, so the ends swap.
    /// Getting this wrong is not a crash, it is an amber sun drawn in blue.
    pub fn digits_as_colorref(&self) -> u32 {
        let (r, g, b) = (self.digits >> 16 & 0xFF, self.digits >> 8 & 0xFF, self.digits & 0xFF);
        (b << 16) | (g << 8) | r
    }

    /// The color one shape is drawn in.
    pub fn ink(&self, ink: Ink) -> u32 {
        match ink {
            Ink::Cloud => self.cloud,
            Ink::Mark => self.mark,
            Ink::Rain => self.rain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap() -> CapBox {
        // A 24 by 9 cap box, roughly what a temperature occupies at the
        // strip's usual size.
        CapBox { left: 2.0, top: 20.0, right: 26.0, bottom: 29.0 }
    }

    fn kinds(condition: Condition) -> Vec<&'static str> {
        shapes(condition, cap())
            .iter()
            .map(|s| match s {
                Shape::RoundedBar { .. } => "bar",
                Shape::Disc { .. } => "disc",
                Shape::Bite { .. } => "bite",
                Shape::Stroke { .. } => "stroke",
                Shape::Polygon { .. } => "polygon",
                Shape::Arc { .. } => "arc",
            })
            .collect()
    }

    #[test]
    fn every_condition_draws_something() {
        for condition in Condition::ALL {
            assert!(!shapes(condition, cap()).is_empty(), "{condition:?} drew nothing");
        }
    }

    #[test]
    fn no_two_conditions_share_a_shape() {
        // The whole point of the set: they must differ in shape, never only in
        // color, so the marks survive a monochrome strip.
        let mut seen: Vec<(Condition, Vec<Shape>)> = Vec::new();
        for condition in Condition::ALL {
            let drawn = shapes(condition, cap());
            for (other, previous) in &seen {
                assert_ne!(previous, &drawn, "{condition:?} draws the same as {other:?}");
            }
            seen.push((condition, drawn));
        }
    }

    #[test]
    fn only_the_clear_and_unknown_marks_go_without_a_cloud() {
        for condition in Condition::ALL {
            let capped = kinds(condition).first() == Some(&"bar");
            let expected = !matches!(
                condition,
                Condition::ClearDay | Condition::ClearNight | Condition::Unknown
            );
            assert_eq!(capped, expected, "{condition:?}");
        }
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    fn cap() -> CapBox {
        CapBox { left: 2.0, top: 20.0, right: 26.0, bottom: 29.0 }
    }

    #[test]
    fn the_wet_family_is_told_apart_by_how_many_strokes_hang_and_how_far() {
        let strokes = |c: Condition| {
            shapes(c, cap())
                .into_iter()
                .filter_map(|s| match s {
                    Shape::Stroke { y1, y2, .. } => Some((y1, y2)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strokes(Condition::Drizzle).len(), 5);
        assert_eq!(strokes(Condition::Rain).len(), 3);
        assert_eq!(strokes(Condition::HeavyRain).len(), 4);

        let reach = |c: Condition| {
            strokes(c).iter().map(|(a, b)| b - a).fold(0.0f32, f32::max)
        };
        assert!(reach(Condition::HeavyRain) > reach(Condition::Rain));
        assert!(reach(Condition::Rain) > reach(Condition::Drizzle));
    }

    #[test]
    fn rain_that_shares_a_mark_is_laid_down_before_it() {
        // Sleet is two strokes with a flake between them; the flake draws on
        // top, so the strokes come first and in the rain ink.
        let sleet = shapes(Condition::Sleet, cap());
        let rain = sleet.iter().position(|s| matches!(s, Shape::Stroke { ink: Ink::Rain, .. }));
        let mark = sleet.iter().position(|s| matches!(s, Shape::Stroke { ink: Ink::Mark, .. }));
        assert!(rain.is_some() && mark.is_some());
        assert!(rain < mark);

        let storm = shapes(Condition::Thunderstorm, cap());
        let rain = storm.iter().position(|s| matches!(s, Shape::Stroke { ink: Ink::Rain, .. }));
        let bolt = storm.iter().position(|s| matches!(s, Shape::Polygon { .. }));
        assert!(rain < bolt);
    }

    #[test]
    fn a_bite_always_follows_the_disc_it_eats() {
        for condition in [Condition::ClearNight, Condition::PartlyCloudyNight] {
            let drawn = shapes(condition, cap());
            let disc = drawn.iter().position(|s| matches!(s, Shape::Disc { .. }));
            let bite = drawn.iter().position(|s| matches!(s, Shape::Bite { .. }));
            assert!(disc.is_some() && bite.is_some(), "{condition:?}");
            assert!(disc < bite, "{condition:?} bites before there is anything to bite");
        }
    }

    #[test]
    fn the_night_mark_puts_its_crescent_where_the_day_mark_puts_its_sun() {
        let day = shapes(Condition::PartlyCloudy, cap());
        let night = shapes(Condition::PartlyCloudyNight, cap());
        // The cloud puffs are discs too, so the sun and the crescent are the
        // *last* disc in each list rather than the first.
        let light = |shapes: &[Shape]| {
            shapes.iter().rev().find_map(|s| match s {
                Shape::Disc { x, y, .. } => Some((*x, *y)),
                _ => None,
            })
        };
        let (dx, dy) = light(&day).unwrap();
        let (nx, ny) = light(&night).unwrap();
        assert!((dy - ny).abs() < 0.01, "different heights: {dy} against {ny}");
        assert!((dx - nx).abs() < 0.5, "different columns: {dx} against {nx}");
    }

    #[test]
    fn the_cloud_sits_above_the_digits_and_the_weather_hangs_below_them() {
        let b = cap();
        for condition in Condition::ALL {
            for shape in shapes(condition, b) {
                if let Shape::RoundedBar { y, height, .. } = shape {
                    assert!(y + height <= b.top, "{condition:?} cloud is not above the digits");
                }
            }
        }
        for condition in [Condition::Rain, Condition::Snow, Condition::Fog, Condition::Thunder] {
            let below = shapes(condition, b).iter().any(|s| match s {
                Shape::Stroke { y1, .. } => *y1 > b.bottom,
                Shape::Polygon { points, .. } => points.iter().all(|(_, y)| *y >= b.bottom),
                _ => false,
            });
            assert!(below, "{condition:?} does not hang under the digits");
        }
    }

    #[test]
    fn the_wmo_table_is_the_one_open_meteo_publishes() {
        assert_eq!(Condition::for_code(0, true), Condition::ClearDay);
        assert_eq!(Condition::for_code(0, false), Condition::ClearNight);
        assert_eq!(Condition::for_code(1, true), Condition::PartlyCloudy);
        assert_eq!(Condition::for_code(2, false), Condition::PartlyCloudyNight);
        assert_eq!(Condition::for_code(3, true), Condition::Cloudy);
        assert_eq!(Condition::for_code(45, true), Condition::Fog);
        assert_eq!(Condition::for_code(53, true), Condition::Drizzle);
        assert_eq!(Condition::for_code(63, true), Condition::Rain);
        assert_eq!(Condition::for_code(81, true), Condition::HeavyRain);
        assert_eq!(Condition::for_code(66, true), Condition::Sleet);
        assert_eq!(Condition::for_code(86, true), Condition::Snow);
        assert_eq!(Condition::for_code(95, true), Condition::Thunder);
        assert_eq!(Condition::for_code(99, true), Condition::Thunderstorm);
    }

    #[test]
    fn a_code_outside_the_table_is_admitted_rather_than_guessed() {
        // A wrong mark is worse than an honest one, because it is believed.
        for code in [4u8, 30, 60, 70, 90, 100, 255] {
            assert_eq!(Condition::for_code(code, true), Condition::Unknown);
        }
    }

    #[test]
    fn monochrome_resolves_every_ink_to_the_one_color() {
        let mono = Palette::resolve(Some(Condition::Thunderstorm), true, true, 0xF2F2F2);
        assert_eq!(mono.digits, 0xF2F2F2);
        assert_eq!(mono.cloud, 0xF2F2F2);
        assert_eq!(mono.mark, 0xF2F2F2);
        assert_eq!(mono.rain, 0xF2F2F2);
    }

    #[test]
    fn the_number_takes_the_conditions_color_only_where_it_means_something() {
        let module = 0xF2F2F2;
        assert_ne!(Palette::resolve(Some(Condition::ClearDay), true, false, module).digits, module);
        assert_ne!(Palette::resolve(Some(Condition::ClearNight), true, false, module).digits, module);
        assert_eq!(Palette::resolve(Some(Condition::Rain), true, false, module).digits, module);
        assert_eq!(Palette::resolve(Some(Condition::Snow), true, false, module).digits, module);
        assert_eq!(Palette::resolve(None, true, false, module).digits, module);
    }

    #[test]
    fn light_and_dark_never_share_a_color() {
        // The light values are darkened deliberately; a matching pair would
        // mean one of them was left behind in an edit.
        for condition in Condition::ALL {
            let dark = Palette::resolve(Some(condition), true, false, 0xF2F2F2);
            let light = Palette::resolve(Some(condition), false, false, 0x191919);
            assert_ne!(dark.cloud, light.cloud, "{condition:?}");
            assert_ne!(dark.mark, light.mark, "{condition:?}");
        }
    }
}
