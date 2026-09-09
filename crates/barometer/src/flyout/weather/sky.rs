// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The colors that are the weather's own, ported from WeatherSky and
// TemperatureScale in WeatherDropdownView.swift.
//
// The panel's neutrals are Windows' and live in ui.rs; the sky behind the
// current conditions and the scale a warm day is colored on come from the
// Mac app unchanged, because they are Barometer's (docs/ui-design.md,
// section 0) and a user with both apps should see one weather.

use barometer_core::weather::badge::Condition;
use barometer_core::weather::models::TemperatureUnit;

use crate::flyout::ui::Palette;
use crate::settings_ui::theme::{Color, WHITE};

/// Rain, wherever a chance of it is printed or drawn.
pub const RAIN: Color = Color(0x60A5FA);
pub const RAIN_DEEP: Color = Color(0x2563EB);

/// The six families the Swift sorts conditions into for the sky.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Category {
    Clear,
    Cloudy,
    Rain,
    Snow,
    Storm,
    Fog,
}

/// Which family a condition belongs to.
///
/// The Swift works this out from the *daytime* symbol's name - "bolt" is a
/// storm, "sun.max" is clear, anything else with a cloud is cloudy - so a
/// clear night is Clear and a partly cloudy day is Cloudy. Written here as
/// the match that string search amounts to.
pub fn category(condition: Condition) -> Category {
    match condition {
        Condition::ClearDay | Condition::ClearNight => Category::Clear,
        Condition::PartlyCloudy
        | Condition::PartlyCloudyNight
        | Condition::Cloudy
        | Condition::Unknown => Category::Cloudy,
        Condition::Drizzle | Condition::Rain | Condition::HeavyRain => Category::Rain,
        Condition::Sleet | Condition::Snow => Category::Snow,
        Condition::Thunder | Condition::Thunderstorm => Category::Storm,
        Condition::Fog => Category::Fog,
    }
}

/// The two ends of the sky gradient behind the current conditions.
///
/// Night wins over everything: a rainy night is the same deep indigo as a
/// clear one, which is what the Swift does and what the sky does.
pub fn sky(condition: Condition, is_day: bool) -> (Color, Color) {
    if !is_day {
        return (Color(0x1E1B4B), Color(0x0F172A));
    }
    match category(condition) {
        Category::Clear => (Color(0x38BDF8), Color(0x2563EB)),
        Category::Cloudy => (Color(0x64748B), Color(0x334155)),
        Category::Rain => (Color(0x3B82F6), Color(0x1E3A8A)),
        Category::Snow => (Color(0x93C5FD), Color(0x475569)),
        Category::Storm => (Color(0x4C1D95), Color(0x1E1B4B)),
        Category::Fog => (Color(0x94A3B8), Color(0x475569)),
    }
}

/// The ink for words on the sky card, which is always dark enough for white
/// however the theme is set.
pub fn on_sky() -> Color {
    WHITE
}

/// The same, for words that step back: white at 80%, resolved over the
/// middle of the gradient it will sit on.
pub fn on_sky_dim(sky: (Color, Color)) -> Color {
    WHITE.over(sky.0.over(sky.1, 0.5), 0.80)
}

/// The moon's lit and unlit faces on a card.
pub fn moon_faces(palette: &Palette) -> (Color, Color) {
    let lit = if palette.theme.light { Color(0x475569) } else { Color(0xF1F5F9) };
    (lit, lit.over(palette.card, 0.22))
}

/// What the sun's arc is drawn in before the sun has reached it.
pub fn arc(palette: &Palette) -> Color {
    palette.theme.text_secondary.over(palette.card, 0.45)
}

/// The color of a temperature, so a bar says how warm a day is rather than
/// where it sits among the ten.
///
/// The scale runs from violet at deep cold through blue, cyan, green, yellow
/// and orange to red at heat, with the stops at the Celsius values people
/// feel the difference at. A bar samples it at its own low and high and at
/// every stop between, so a 20 to 27° day is green into yellow and a 24 to
/// 34° day runs yellow into red, in either unit.
pub struct Scale;

/// A stop on the scale: a temperature and the color at it.
struct Stop {
    celsius: f64,
    rgb: (f64, f64, f64),
}

const STOPS: [Stop; 10] = [
    Stop { celsius: -20.0, rgb: (0.43, 0.16, 0.85) },
    Stop { celsius: -10.0, rgb: (0.23, 0.51, 0.96) },
    Stop { celsius: 0.0, rgb: (0.22, 0.74, 0.97) },
    Stop { celsius: 8.0, rgb: (0.13, 0.83, 0.93) },
    Stop { celsius: 14.0, rgb: (0.29, 0.87, 0.50) },
    Stop { celsius: 20.0, rgb: (0.64, 0.86, 0.29) },
    Stop { celsius: 25.0, rgb: (0.98, 0.80, 0.08) },
    Stop { celsius: 30.0, rgb: (0.98, 0.57, 0.24) },
    Stop { celsius: 35.0, rgb: (0.94, 0.27, 0.27) },
    Stop { celsius: 42.0, rgb: (0.73, 0.11, 0.11) },
];

impl Scale {
    /// A temperature in the scale's own unit.
    pub fn celsius(value: f64, unit: TemperatureUnit) -> f64 {
        match unit {
            TemperatureUnit::Fahrenheit => (value - 32.0) * 5.0 / 9.0,
            TemperatureUnit::Celsius => value,
        }
    }

    /// The scale's color at a temperature, interpolated between the nearest
    /// stops and held at the ends.
    pub fn color(celsius: f64) -> Color {
        let (r, g, b) = Scale::components(celsius);
        let channel = |value: f64| (value * 255.0).round().clamp(0.0, 255.0) as u8;
        Color::rgb(channel(r), channel(g), channel(b))
    }

    fn components(celsius: f64) -> (f64, f64, f64) {
        let first = &STOPS[0];
        let last = &STOPS[STOPS.len() - 1];
        if celsius <= first.celsius {
            return first.rgb;
        }
        if celsius >= last.celsius {
            return last.rgb;
        }
        for pair in STOPS.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if celsius <= b.celsius {
                let t = (celsius - a.celsius) / (b.celsius - a.celsius);
                let mix = |from: f64, to: f64| from + (to - from) * t;
                return (mix(a.rgb.0, b.rgb.0), mix(a.rgb.1, b.rgb.1), mix(a.rgb.2, b.rgb.2));
            }
        }
        last.rgb
    }

    /// A left-to-right gradient sampled at `from`, at every scale stop
    /// between, and at `to`: positions from zero to one with their colors.
    pub fn gradient(from_celsius: f64, to_celsius: f64) -> Vec<(f32, Color)> {
        let lower = from_celsius.min(to_celsius);
        let upper = from_celsius.max(to_celsius);
        // A day whose low and high are the same temperature still wants a
        // gradient of nonzero span rather than a division by zero.
        let span = (upper - lower).max(0.5);
        let mut stops = vec![(0.0, Scale::color(lower))];
        for stop in STOPS.iter().filter(|stop| stop.celsius > lower && stop.celsius < upper) {
            stops.push((((stop.celsius - lower) / span) as f32, Scale::color(stop.celsius)));
        }
        stops.push((1.0, Scale::color(upper)));
        stops
    }
}

/// Rain as words or a glyph on a card: the sky blue on a dark card, and its
/// darker twin on a light one, where the sky blue reads at barely 2:1.
///
/// The gradients and bars keep `RAIN` in both appearances - they are washes,
/// not words - which is why this is a function of the theme and that is not.
pub fn rain_ink(light: bool) -> Color {
    if light {
        RAIN_DEEP
    } else {
        RAIN
    }
}

/// The sun as a glyph on a card, with the same darker twin for a light card.
pub fn sun_ink(light: bool) -> Color {
    if light {
        Color(0xD97706)
    } else {
        Color(0xFBBF24)
    }
}

/// The inks a condition's mark is drawn in, base glyph then accent.
///
/// SF Symbols render the Mac's marks in several colors at once; Segoe Fluent
/// Icons are one color per glyph, so the composition in glyph.rs is what
/// carries the color here: a gray cloud with an amber sun behind it, a blue
/// raining cloud with a pale flake under it. Each ink has a darker twin for a
/// light card, where amber and sky blue would wash out.
pub fn mark_inks(condition: Condition, light: bool) -> (Color, Color) {
    let pick = |dark: u32, on_light: u32| Color(if light { on_light } else { dark });
    let sun = sun_ink(light);
    let moon = pick(0xC7D2FE, 0x6366F1);
    let cloud = pick(0xCBD5E1, 0x64748B);
    let mist = pick(0xA1A1AA, 0x71717A);
    let rain = rain_ink(light);
    let rain_deep = pick(0x3B82F6, 0x1D4ED8);
    let flake = pick(0xBAE6FD, 0x0284C7);
    let bolt = pick(0xFBBF24, 0xD97706);
    let query = pick(0x9D9D9D, 0x8E8E8E);
    match condition {
        Condition::ClearDay => (sun, sun),
        Condition::ClearNight => (moon, moon),
        Condition::PartlyCloudy => (cloud, sun),
        Condition::PartlyCloudyNight => (cloud, moon),
        Condition::Cloudy => (cloud, cloud),
        Condition::Fog => (mist, mist),
        Condition::Drizzle => (rain, rain),
        Condition::Rain => (rain, rain),
        Condition::HeavyRain => (rain, rain_deep),
        Condition::Sleet => (rain, flake),
        Condition::Snow => (cloud, flake),
        Condition::Thunder => (bolt, bolt),
        Condition::Thunderstorm => (rain, bolt),
        Condition::Unknown => (query, query),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

    #[test]
    fn every_condition_falls_into_the_family_the_swift_puts_it_in() {
        // The Swift decides from the daytime symbol name, which puts a clear
        // night with the clear skies and the unknown mark with the clouds.
        assert_eq!(category(Condition::ClearNight), Category::Clear);
        assert_eq!(category(Condition::PartlyCloudy), Category::Cloudy);
        assert_eq!(category(Condition::Unknown), Category::Cloudy);
        assert_eq!(category(Condition::Drizzle), Category::Rain);
        assert_eq!(category(Condition::Sleet), Category::Snow);
        assert_eq!(category(Condition::Thunderstorm), Category::Storm);
        assert_eq!(category(Condition::Fog), Category::Fog);
    }

    #[test]
    fn night_is_indigo_whatever_the_weather_is_doing() {
        let clear = sky(Condition::ClearNight, false);
        let raining = sky(Condition::HeavyRain, false);
        assert_eq!(clear, raining);
        assert_eq!(clear.0, Color(0x1E1B4B));
        assert_ne!(sky(Condition::HeavyRain, true), raining);
        assert_eq!(sky(Condition::ClearDay, true), (Color(0x38BDF8), Color(0x2563EB)));
    }

    #[test]
    fn words_on_the_sky_read_on_every_gradient_it_can_be() {
        for condition in Condition::ALL {
            for is_day in [true, false] {
                let (top, bottom) = sky(condition, is_day);
                let middle = top.over(bottom, 0.5);
                assert!(on_sky().contrast(middle) >= 3.0, "{condition:?} day={is_day}");
                assert!(on_sky_dim((top, bottom)).contrast(middle) >= 2.5, "{condition:?} day={is_day}");
            }
        }
    }

    #[test]
    fn the_scale_hits_its_stops_exactly_and_holds_at_the_ends() {
        assert_eq!(Scale::color(0.0), Color::rgb(56, 189, 247));
        assert_eq!(Scale::color(-40.0), Scale::color(-20.0));
        assert_eq!(Scale::color(60.0), Scale::color(42.0));
        // Halfway between 20 (green) and 25 (yellow) is between them.
        let between = Scale::color(22.5);
        let green = Scale::color(20.0);
        let yellow = Scale::color(25.0);
        assert!(between.r() > green.r() && between.r() < yellow.r());
    }

    #[test]
    fn the_scale_reads_either_unit() {
        assert!((Scale::celsius(212.0, TemperatureUnit::Fahrenheit) - 100.0).abs() < 1e-9);
        assert_eq!(Scale::celsius(21.0, TemperatureUnit::Celsius), 21.0);
        assert_eq!(Scale::color(Scale::celsius(32.0, TemperatureUnit::Fahrenheit)), Scale::color(0.0));
    }

    #[test]
    fn a_range_samples_every_stop_it_crosses_and_only_those() {
        // 24 to 34° crosses the 25 and 30 stops: four samples in order.
        let stops = Scale::gradient(24.0, 34.0);
        assert_eq!(stops.len(), 4);
        assert_eq!(stops[0].0, 0.0);
        assert_eq!(stops[3].0, 1.0);
        assert!(stops[1].0 < stops[2].0);
        assert_eq!(stops[1].1, Scale::color(25.0));
        // Reversed ends give the same gradient.
        assert_eq!(Scale::gradient(34.0, 24.0), stops);
        // A flat day is still a gradient of two ends.
        assert_eq!(Scale::gradient(20.0, 20.0).len(), 2);
    }

    #[test]
    fn mark_inks_differ_between_a_dark_and_a_light_card() {
        for condition in Condition::ALL {
            let dark = mark_inks(condition, false);
            let light = mark_inks(condition, true);
            assert_ne!(dark, light, "{condition:?} has the same inks on both cards");
        }
        // The compositions carry the color: a sun behind a cloud is not the
        // same ink twice.
        let (cloud, sun) = mark_inks(Condition::PartlyCloudy, false);
        assert_ne!(cloud, sun);
    }

    #[test]
    fn rain_and_sun_as_words_or_marks_read_against_the_card_in_both_appearances() {
        // A chance of rain is printed in the rain's color; on a light card the
        // sky blue the dark card uses fell to 2.4:1, under even a mark's 3:1.
        for light in [false, true] {
            let palette = Palette::resolve(Theme::resolve(light, DEFAULT_ACCENT));
            assert!(rain_ink(light).contrast(palette.card) >= 4.5, "light={light}: {:?}", rain_ink(light));
            assert!(sun_ink(light).contrast(palette.card) >= 3.0, "light={light}: {:?}", sun_ink(light));
        }
        assert_ne!(rain_ink(true), rain_ink(false));
    }

    #[test]
    fn the_moon_reads_against_the_card_in_both_appearances() {
        for light in [false, true] {
            let palette = Palette::resolve(Theme::resolve(light, DEFAULT_ACCENT));
            let (lit, dark) = moon_faces(&palette);
            assert!(lit.contrast(palette.card) >= 3.0, "light={light}");
            assert!(lit.contrast(dark) >= 2.0, "light={light}");
        }
    }
}
