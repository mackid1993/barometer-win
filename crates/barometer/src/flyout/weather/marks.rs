// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The small pictures the panel draws for itself: the moon at its phase, the
// sun on its arc, a wind arrow, a pressure gauge.
//
// The Swift leans on SF Symbols for these - moonphase.waxing.gibbous,
// sunrise.fill, gauge.with.dots.needle.50percent - and Segoe Fluent Icons has
// none of them (glyph.rs records the sweep that established what it does
// have). So they are drawn, in the same spirit as the weather marks in
// badge.rs: geometry here in DIPs with no drawing API in sight, so that a
// gibbous moon really is gibbous and the tests can say so, and pixels in
// paint.rs.

use barometer_core::weather::detail::{LocalTime, MoonPhase};

use crate::settings_ui::geometry::Rect;

/// How to paint a moon at a phase.
///
/// The disc is painted unlit, then the lit half, then an ellipse down the
/// middle whose width is the terminator's and whose color decides whether it
/// eats into the lit half (a crescent) or adds to it (a gibbous moon).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MoonDisc {
    /// Whether the lit limb is on the right, as a waxing moon's is when seen
    /// from the northern hemisphere.
    pub lit_right: bool,
    /// The terminator ellipse's half-width as a fraction of the radius.
    pub terminator: f32,
    /// Whether the ellipse is lit (more than half the disc is) or dark.
    pub ellipse_lit: bool,
}

/// The moon's face for a phase and how much of it is lit.
pub fn moon(phase: MoonPhase, illumination: f64) -> MoonDisc {
    let lit = illumination.clamp(0.0, 1.0) as f32;
    MoonDisc {
        lit_right: matches!(
            phase,
            MoonPhase::NewMoon
                | MoonPhase::WaxingCrescent
                | MoonPhase::FirstQuarter
                | MoonPhase::WaxingGibbous
                | MoonPhase::FullMoon
        ),
        terminator: (2.0 * lit - 1.0).abs(),
        ellipse_lit: lit >= 0.5,
    }
}

/// How far through the day the sun is: zero at sunrise, one at sunset,
/// outside that range before and after.
///
/// None when either event is missing or they are the wrong way round, which
/// is the polar case daylight_duration() guards against too.
pub fn day_progress(sunrise: Option<LocalTime>, sunset: Option<LocalTime>, now: LocalTime) -> Option<f32> {
    let (rise, set) = (sunrise?.wall_clock_seconds(), sunset?.wall_clock_seconds());
    if set <= rise {
        return None;
    }
    Some((now.wall_clock_seconds() - rise) as f32 / (set - rise) as f32)
}

/// The sun's arc across a plate, and where the sun is on it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SunPath {
    /// The ellipse whose upper half is the arc, in the plate's coordinates.
    pub arc: Rect,
    /// The horizon's y.
    pub horizon: f32,
    /// Where the sun is drawn.
    pub sun: (f32, f32),
    /// How much of the arc has been traveled, clamped to the arc.
    pub progress: f32,
    /// Whether the sun is above the horizon at all.
    pub up: bool,
}

/// Room left under the horizon for the sunrise and sunset times.
pub const SUN_PATH_FOOTER: f32 = 30.0;

/// Lays the arc out in a plate.
///
/// Before sunrise and after sunset the sun sits on the horizon at the end it
/// is nearest, and `up` says it is not to be lit; a sun drawn below the
/// horizon line would be under the times printed there.
pub fn sun_path(plate: Rect, progress: Option<f32>) -> SunPath {
    let inset = 18.0;
    let horizon = plate.h - SUN_PATH_FOOTER;
    let top = 10.0;
    let arc = Rect::new(inset, top, plate.w - 2.0 * inset, 2.0 * (horizon - top));
    let (a, b) = (arc.w / 2.0, arc.h / 2.0);
    let (cx, cy) = (arc.x + a, arc.y + b);
    let raw = progress.unwrap_or(-1.0);
    let clamped = raw.clamp(0.0, 1.0);
    let angle = std::f32::consts::PI * (1.0 - clamped);
    SunPath {
        arc,
        horizon,
        sun: (cx + a * angle.cos(), cy - b * angle.sin()),
        progress: clamped,
        up: progress.is_some_and(|t| (0.0..=1.0).contains(&t)),
    }
}

/// An arrow of radius `r` pointing where the wind is going.
///
/// The provider reports the bearing the wind blows *from*, and the words
/// beside the arrow say that ("SW 12 mph"); the arrow itself points the way
/// the air is moving, which is how every weather map draws it and the only
/// direction an arrow can honestly point.
pub fn compass_arrow(cx: f32, cy: f32, r: f32, bearing_from: f64) -> [(f32, f32); 4] {
    let heading = (bearing_from + 180.0).to_radians() as f32;
    let (sin, cos) = heading.sin_cos();
    // Bearings turn clockwise from north, and screen y runs downward, so a
    // point (x, y) drawn with north up rotates as it would on a chart.
    let place = |x: f32, y: f32| (cx + x * cos - y * sin, cy + x * sin + y * cos);
    [
        place(0.0, -r),
        place(0.55 * r, 0.75 * r),
        place(0.0, 0.35 * r),
        place(-0.55 * r, 0.75 * r),
    ]
}

/// A gauge's dial: the arc it sweeps and where the needle points.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Gauge {
    /// Degrees, in the screen's clockwise sense, where the arc begins.
    pub start: f32,
    /// Degrees the arc sweeps.
    pub sweep: f32,
    /// The needle's tip.
    pub needle: (f32, f32),
}

/// A gauge of radius `r` reading `fraction` of its range.
///
/// Two hundred and seventy degrees from the lower left round to the lower
/// right, which is a speedometer's dial and reads as a gauge at a glance
/// even at fourteen DIPs.
pub fn gauge(cx: f32, cy: f32, r: f32, fraction: f32) -> Gauge {
    let start = 135.0;
    let sweep = 270.0;
    let angle = (start + sweep * fraction.clamp(0.0, 1.0)).to_radians();
    Gauge { start, sweep, needle: (cx + 0.78 * r * angle.cos(), cy + 0.78 * r * angle.sin()) }
}

/// Where a sea-level pressure sits between the lows storms bring and the
/// highs of settled weather.
///
/// 960 to 1060 hPa: the range a household barometer's dial covers, which is
/// what the tile is imitating.
pub fn pressure_fraction(hectopascals: f64) -> f32 {
    ((hectopascals - 960.0) / 100.0).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moon_is_dark_when_new_full_when_full_and_half_at_the_quarters() {
        let new = moon(MoonPhase::NewMoon, 0.0);
        assert_eq!(new.terminator, 1.0);
        assert!(!new.ellipse_lit);
        let full = moon(MoonPhase::FullMoon, 1.0);
        assert_eq!(full.terminator, 1.0);
        assert!(full.ellipse_lit);
        let first = moon(MoonPhase::FirstQuarter, 0.5);
        assert!(first.terminator.abs() < 1e-6);
        assert!(first.lit_right);
        let last = moon(MoonPhase::LastQuarter, 0.5);
        assert!(!last.lit_right);
    }

    #[test]
    fn a_crescent_eats_into_the_lit_half_and_a_gibbous_moon_spills_past_it() {
        let crescent = moon(MoonPhase::WaxingCrescent, 0.25);
        assert!(!crescent.ellipse_lit);
        assert!((crescent.terminator - 0.5).abs() < 1e-6);
        let gibbous = moon(MoonPhase::WaningGibbous, 0.75);
        assert!(gibbous.ellipse_lit);
        assert!((gibbous.terminator - 0.5).abs() < 1e-6);
        assert!(!gibbous.lit_right);
    }

    fn at(clock: &str) -> LocalTime {
        LocalTime::parse(clock).unwrap()
    }

    #[test]
    fn the_days_progress_runs_from_sunrise_to_sunset() {
        let rise = Some(at("2026-09-08T07:00"));
        let set = Some(at("2026-09-08T19:00"));
        assert_eq!(day_progress(rise, set, at("2026-09-08T07:00")), Some(0.0));
        assert_eq!(day_progress(rise, set, at("2026-09-08T13:00")), Some(0.5));
        assert_eq!(day_progress(rise, set, at("2026-09-08T19:00")), Some(1.0));
        assert!(day_progress(rise, set, at("2026-09-08T05:00")).unwrap() < 0.0);
        assert!(day_progress(rise, set, at("2026-09-08T22:00")).unwrap() > 1.0);
        // Nothing to say without both events, or with them the wrong way round.
        assert_eq!(day_progress(None, set, at("2026-09-08T13:00")), None);
        assert_eq!(day_progress(set, rise, at("2026-09-08T13:00")), None);
    }

    #[test]
    fn the_sun_rises_on_the_left_peaks_in_the_middle_and_sets_on_the_right() {
        let plate = Rect::new(0.0, 0.0, 300.0, 96.0);
        let dawn = sun_path(plate, Some(0.0));
        let noon = sun_path(plate, Some(0.5));
        let dusk = sun_path(plate, Some(1.0));
        assert!(dawn.sun.0 < noon.sun.0 && noon.sun.0 < dusk.sun.0);
        assert!(noon.sun.1 < dawn.sun.1, "noon is higher on the screen than dawn");
        assert!((dawn.sun.1 - dawn.horizon).abs() < 1e-3);
        assert!((dusk.sun.1 - dusk.horizon).abs() < 1e-3);
        assert!(dawn.up && noon.up && dusk.up);
        assert!((noon.sun.0 - 150.0).abs() < 1e-3);
    }

    #[test]
    fn before_dawn_and_after_dusk_the_sun_waits_on_the_horizon_unlit() {
        let plate = Rect::new(0.0, 0.0, 300.0, 96.0);
        let early = sun_path(plate, Some(-0.3));
        assert!(!early.up);
        assert_eq!(early.progress, 0.0);
        let late = sun_path(plate, Some(1.4));
        assert!(!late.up);
        assert_eq!(late.progress, 1.0);
        assert!(!sun_path(plate, None).up);
        // The horizon leaves room under it for the times.
        assert_eq!(early.horizon, 96.0 - SUN_PATH_FOOTER);
    }

    #[test]
    fn the_wind_arrow_points_where_the_air_is_going() {
        // A north wind blows from the north, so the arrow points down the
        // screen, toward the south.
        let north = compass_arrow(0.0, 0.0, 10.0, 0.0);
        assert!(north[0].1 > 9.9, "tip at {:?}", north[0]);
        assert!(north[0].0.abs() < 1e-4);
        // A west wind's arrow points east: to the right.
        let west = compass_arrow(0.0, 0.0, 10.0, 270.0);
        assert!(west[0].0 > 9.9, "tip at {:?}", west[0]);
        assert!(west[0].1.abs() < 1e-4);
    }

    #[test]
    fn the_gauge_needle_sweeps_clockwise_from_the_lower_left() {
        let empty = gauge(0.0, 0.0, 10.0, 0.0);
        let full = gauge(0.0, 0.0, 10.0, 1.0);
        let half = gauge(0.0, 0.0, 10.0, 0.5);
        assert!(empty.needle.0 < 0.0 && empty.needle.1 > 0.0, "lower left: {:?}", empty.needle);
        assert!(full.needle.0 > 0.0 && full.needle.1 > 0.0, "lower right: {:?}", full.needle);
        assert!(half.needle.0.abs() < 1e-4 && half.needle.1 < 0.0, "straight up: {:?}", half.needle);
        assert_eq!(empty.sweep, 270.0);
    }

    #[test]
    fn pressure_reads_against_a_household_barometers_dial() {
        assert_eq!(pressure_fraction(960.0), 0.0);
        assert_eq!(pressure_fraction(1060.0), 1.0);
        assert!((pressure_fraction(1013.25) - 0.5325).abs() < 1e-4);
        assert_eq!(pressure_fraction(900.0), 0.0);
    }
}
