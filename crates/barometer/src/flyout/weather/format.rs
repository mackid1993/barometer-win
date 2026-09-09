// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The words and numbers the weather flyout shows, ported from WeatherValue in
// WeatherDropdownView.swift, WeatherDetailUnit in WeatherDetailSections.swift
// and WMOCode.description in WeatherModels.swift.
//
// Every function takes an Option and answers an em dash for None, which is
// the Swift's convention too: a reading the provider did not send is shown as
// absent rather than as zero, and a dash in a tile is the honest shape of
// "nobody knows".

use barometer_core::weather::models::{PrecipitationUnit, TemperatureUnit, WeatherUnits};

/// What stands in for a reading that is not there.
pub const DASH: &str = "\u{2014}";

/// Rounding that never prints "-0" - see `barometer_core::format::whole`.
///
/// It lives in core now because the taskbar rounds the same numbers, and a
/// freezing night showed "-0" on the strip beside "0" in the panel it opened.
use barometer_core::format::whole;

/// "72°F", in the unit the user chose.
pub fn temperature(value: Option<f64>, units: WeatherUnits) -> String {
    match value {
        Some(value) => format!("{}{}", whole(value), units.temperature.symbol()),
        None => DASH.to_string(),
    }
}

/// "72°", with the unit letter left off where the reader already knows it.
pub fn degree(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{}\u{00B0}", whole(value)),
        None => DASH.to_string(),
    }
}

pub fn percent(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{}%", whole(value)),
        None => DASH.to_string(),
    }
}

/// One decimal, for indexes such as UV.
pub fn number(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.1}"),
        None => DASH.to_string(),
    }
}

/// "0.12 in": two decimals, because a tenth of a millimeter is a reading and
/// a whole one is not.
pub fn precipitation(value: Option<f64>, units: WeatherUnits) -> String {
    match value {
        Some(value) => format!("{value:.2} {}", units.precipitation.symbol()),
        None => DASH.to_string(),
    }
}

/// Snowfall arrives in centimeters or inches, never millimeters, so it has
/// its own unit word.
pub fn snowfall(value: Option<f64>, units: WeatherUnits) -> String {
    let unit = if units.precipitation == PrecipitationUnit::Inches { "in" } else { "cm" };
    match value {
        Some(value) => format!("{value:.1} {unit}"),
        None => DASH.to_string(),
    }
}

/// Pressure in the chosen unit, converted from the provider's hectopascals.
///
/// Two decimals for inches of mercury and none for the others, as in the
/// Swift: a hundredth of an inch is what a barometer's needle moves in an
/// afternoon, and a hundredth of a hectopascal is noise.
pub fn pressure(hectopascals: Option<f64>, units: WeatherUnits) -> String {
    use barometer_core::weather::models::PressureUnit;
    let Some(hpa) = hectopascals else {
        return DASH.to_string();
    };
    let value = units.pressure.from_hectopascals(hpa);
    match units.pressure {
        PressureUnit::InchesOfMercury => format!("{value:.2} {}", units.pressure.symbol()),
        _ => format!("{} {}", whole(value), units.pressure.symbol()),
    }
}

/// "SW 12 mph": where it blows from, then how hard.
pub fn wind(speed: Option<f64>, direction: Option<f64>, units: WeatherUnits) -> String {
    let Some(speed) = speed else {
        return DASH.to_string();
    };
    let point = direction.map(compass).unwrap_or("");
    format!("{point} {} {}", whole(speed), units.wind_speed.symbol()).trim().to_string()
}

/// A bearing as one of eight compass points.
///
/// Wind direction is where the wind comes *from*, which is what the provider
/// reports and what "SW 12 mph" says.
pub fn compass(degrees: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let normalized = degrees.rem_euclid(360.0);
    POINTS[((normalized / 45.0).round() as usize) % POINTS.len()]
}

/// Visibility in miles or kilometers, decided by the temperature unit.
///
/// The provider already normalized the reading to meters; which distance
/// unit to show is the Swift's rule, which keys it off Fahrenheit because
/// the settings offer no separate distance choice.
pub fn visibility(meters: Option<f64>, units: WeatherUnits) -> String {
    let Some(meters) = meters else {
        return DASH.to_string();
    };
    if units.temperature == TemperatureUnit::Fahrenheit {
        format!("{:.1} mi", meters / 1_609.344)
    } else {
        format!("{:.1} km", meters / 1_000.0)
    }
}

/// "12 hr 43 min".
pub fn duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{} hr {} min", seconds / 3_600, seconds % 3_600 / 60)
}

/// "12 hr 43 min of daylight", or the honest sentence when nothing says.
pub fn daylight(seconds: Option<f64>) -> String {
    match seconds {
        Some(seconds) => format!("{} of daylight", duration(seconds)),
        None => "Daylight duration unavailable".to_string(),
    }
}

/// "About 62% illuminated".
pub fn illumination(fraction: f64) -> String {
    format!("About {}% illuminated", whole(fraction * 100.0))
}

/// "5 min ago", stepping through hours and days the way the Swift does.
pub fn relative(elapsed_minutes: u64) -> String {
    match elapsed_minutes {
        0 => "just now".to_string(),
        minutes if minutes < 60 => format!("{minutes} min ago"),
        minutes if minutes < 1_440 => format!("{} hr ago", minutes / 60),
        minutes => format!("{} days ago", minutes / 1_440),
    }
}

/// "Updated 3:42 PM · 5 min ago".
pub fn updated(absolute: &str, elapsed_minutes: u64) -> String {
    format!("Updated {absolute} \u{00B7} {}", relative(elapsed_minutes))
}

/// The WMO interpretation code as words, from WMOCode.description.
pub fn description(code: Option<u8>) -> &'static str {
    match code {
        Some(0) => "Clear sky",
        Some(1) => "Mainly clear",
        Some(2) => "Partly cloudy",
        Some(3) => "Overcast",
        Some(45) => "Fog",
        Some(48) => "Depositing rime fog",
        Some(51) => "Light drizzle",
        Some(53) => "Moderate drizzle",
        Some(55) => "Dense drizzle",
        Some(56) => "Light freezing drizzle",
        Some(57) => "Dense freezing drizzle",
        Some(61) => "Light rain",
        Some(63) => "Moderate rain",
        Some(65) => "Heavy rain",
        Some(66) => "Light freezing rain",
        Some(67) => "Heavy freezing rain",
        Some(71) => "Light snow",
        Some(73) => "Moderate snow",
        Some(75) => "Heavy snow",
        Some(77) => "Snow grains",
        Some(80) => "Light rain showers",
        Some(81) => "Moderate rain showers",
        Some(82) => "Violent rain showers",
        Some(85) => "Light snow showers",
        Some(86) => "Heavy snow showers",
        Some(95) => "Thunderstorm",
        Some(96) => "Thunderstorm with light hail",
        Some(99) => "Thunderstorm with heavy hail",
        Some(_) => "Unknown conditions",
        // The Swift shows this for a missing code in the day summary and the
        // hour plate; the Windows panel says the same thing in the same places.
        None => "Conditions unavailable",
    }
}

/// How a metric from the catalog is shown, ported from WeatherDetailUnit.
///
/// The provider fixes some of these units regardless of the user's choices -
/// joules per kilogram, kilopascals, megajoules per square meter - and those
/// stay explicit rather than being made to look like a preference.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unit {
    Temperature,
    Precipitation,
    Snowfall,
    Hours,
    Duration,
    Number,
    Percent,
    Direction,
    Visibility,
    Pressure,
    Cape,
    Kilopascals,
    SolarEnergy,
    Radiation,
    Height,
    SnowDepth,
    SoilMoisture,
    Wind,
}

impl Unit {
    pub fn format(self, value: Option<f64>, units: WeatherUnits) -> String {
        let Some(value) = value.filter(|value| value.is_finite()) else {
            return DASH.to_string();
        };
        match self {
            Unit::Temperature => temperature(Some(value), units),
            Unit::Precipitation => precipitation(Some(value), units),
            Unit::Snowfall => snowfall(Some(value), units),
            Unit::Hours => format!("{value:.1} hr"),
            Unit::Duration => {
                // More than a day of something inside a day is not a number
                // to show, which is the same guard the daylight figure has.
                if (0.0..=86_400.0).contains(&value) {
                    duration(value)
                } else {
                    DASH.to_string()
                }
            }
            Unit::Number => format!("{value:.1}"),
            Unit::Percent => percent(Some(value)),
            Unit::Direction => compass(value).to_string(),
            Unit::Visibility => visibility(Some(value), units),
            Unit::Pressure => pressure(Some(value), units),
            Unit::Cape => format!("{} J/kg", whole(value)),
            Unit::Kilopascals => format!("{value:.2} kPa"),
            Unit::SolarEnergy => format!("{value:.1} MJ/m\u{00B2}"),
            Unit::Radiation => format!("{} W/m\u{00B2}", whole(value)),
            Unit::Height => {
                if units.temperature == TemperatureUnit::Fahrenheit {
                    format!("{} ft", whole(value / 0.3048))
                } else {
                    format!("{} m", whole(value))
                }
            }
            Unit::SnowDepth => {
                // The provider reports depth in meters whatever else was
                // asked for, so both displays convert.
                if units.precipitation == PrecipitationUnit::Inches {
                    format!("{:.1} in", value / 0.0254)
                } else {
                    format!("{:.1} cm", value * 100.0)
                }
            }
            Unit::SoilMoisture => format!("{value:.3} m\u{00B3}/m\u{00B3}"),
            Unit::Wind => wind(Some(value), None, units),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use barometer_core::weather::models::{PressureUnit, WindSpeedUnit};

    #[test]
    fn temperatures_round_to_whole_degrees_in_the_chosen_unit() {
        assert_eq!(temperature(Some(71.6), WeatherUnits::IMPERIAL), "72\u{00B0}F");
        assert_eq!(temperature(Some(21.4), WeatherUnits::METRIC), "21\u{00B0}C");
        assert_eq!(degree(Some(71.6)), "72\u{00B0}");
        assert_eq!(temperature(None, WeatherUnits::IMPERIAL), DASH);
    }

    #[test]
    fn a_small_negative_never_reads_as_minus_zero() {
        // -0.4 rounds to zero and the Swift prints "-0°" for it. Among a
        // column of ten rounded numbers that reads as a fault.
        assert_eq!(degree(Some(-0.4)), "0\u{00B0}");
        assert_eq!(degree(Some(-0.6)), "-1\u{00B0}");
        assert_eq!(percent(Some(-0.2)), "0%");
    }

    #[test]
    fn the_compass_has_eight_points_and_wraps() {
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(44.0), "NE");
        assert_eq!(compass(160.0), "S");
        assert_eq!(compass(225.0), "SW");
        assert_eq!(compass(359.0), "N");
        assert_eq!(compass(-90.0), "W");
        assert_eq!(compass(720.0 + 90.0), "E");
    }

    #[test]
    fn wind_reads_from_then_speed_and_survives_a_missing_bearing() {
        assert_eq!(wind(Some(8.9), Some(160.0), WeatherUnits::IMPERIAL), "S 9 mph");
        assert_eq!(wind(Some(8.9), None, WeatherUnits::IMPERIAL), "9 mph");
        assert_eq!(wind(None, Some(160.0), WeatherUnits::IMPERIAL), DASH);
        let knots = WeatherUnits { wind_speed: WindSpeedUnit::Knots, ..WeatherUnits::METRIC };
        assert_eq!(wind(Some(12.0), Some(0.0), knots), "N 12 kn");
    }

    #[test]
    fn pressure_is_converted_out_of_hectopascals_with_the_units_own_precision() {
        assert_eq!(pressure(Some(1013.25), WeatherUnits::METRIC), "1013 hPa");
        assert_eq!(pressure(Some(1013.25), WeatherUnits::IMPERIAL), "29.92 inHg");
        let mmhg = WeatherUnits { pressure: PressureUnit::MillimetersOfMercury, ..WeatherUnits::METRIC };
        assert_eq!(pressure(Some(1013.25), mmhg), "760 mmHg");
        assert_eq!(pressure(None, WeatherUnits::METRIC), DASH);
    }

    #[test]
    fn visibility_follows_the_temperature_unit() {
        // The provider normalized to meters; 24 km is about 15 miles.
        assert_eq!(visibility(Some(24_140.0), WeatherUnits::IMPERIAL), "15.0 mi");
        assert_eq!(visibility(Some(24_140.0), WeatherUnits::METRIC), "24.1 km");
    }

    #[test]
    fn durations_read_as_hours_and_minutes() {
        assert_eq!(duration(37_620.0), "10 hr 27 min");
        assert_eq!(daylight(Some(45_780.0)), "12 hr 43 min of daylight");
        assert_eq!(daylight(None), "Daylight duration unavailable");
        assert_eq!(Unit::Duration.format(Some(90_000.0), WeatherUnits::METRIC), DASH);
    }

    #[test]
    fn the_updated_line_steps_from_minutes_through_hours_to_days() {
        assert_eq!(updated("3:42 PM", 0), "Updated 3:42 PM \u{00B7} just now");
        assert_eq!(relative(5), "5 min ago");
        assert_eq!(relative(59), "59 min ago");
        assert_eq!(relative(60), "1 hr ago");
        assert_eq!(relative(1_439), "23 hr ago");
        assert_eq!(relative(1_440), "1 days ago");
    }

    #[test]
    fn the_catalog_units_keep_the_providers_fixed_units_explicit() {
        let metric = WeatherUnits::METRIC;
        assert_eq!(Unit::Cape.format(Some(1234.6), metric), "1235 J/kg");
        assert_eq!(Unit::Kilopascals.format(Some(0.123), metric), "0.12 kPa");
        assert_eq!(Unit::SolarEnergy.format(Some(21.46), metric), "21.5 MJ/m\u{00B2}");
        assert_eq!(Unit::Radiation.format(Some(612.4), metric), "612 W/m\u{00B2}");
        assert_eq!(Unit::SoilMoisture.format(Some(0.2345), metric), "0.234 m\u{00B3}/m\u{00B3}");
        assert_eq!(Unit::Hours.format(Some(3.0), metric), "3.0 hr");
        assert_eq!(Unit::Direction.format(Some(270.0), metric), "W");
    }

    #[test]
    fn lengths_the_provider_reports_in_meters_are_shown_in_the_users_units() {
        assert_eq!(Unit::Height.format(Some(3352.8), WeatherUnits::IMPERIAL), "11000 ft");
        assert_eq!(Unit::Height.format(Some(3352.8), WeatherUnits::METRIC), "3353 m");
        assert_eq!(Unit::SnowDepth.format(Some(0.254), WeatherUnits::IMPERIAL), "10.0 in");
        assert_eq!(Unit::SnowDepth.format(Some(0.254), WeatherUnits::METRIC), "25.4 cm");
        assert_eq!(Unit::Snowfall.format(Some(2.54), WeatherUnits::IMPERIAL), "2.5 in");
        assert_eq!(Unit::Snowfall.format(Some(2.54), WeatherUnits::METRIC), "2.5 cm");
    }

    #[test]
    fn a_value_that_is_not_finite_is_shown_as_absent() {
        assert_eq!(Unit::Number.format(Some(f64::NAN), WeatherUnits::METRIC), DASH);
        assert_eq!(Unit::Percent.format(Some(f64::INFINITY), WeatherUnits::METRIC), DASH);
        assert_eq!(Unit::Percent.format(None, WeatherUnits::METRIC), DASH);
    }

    #[test]
    fn every_wmo_code_the_provider_documents_has_words() {
        for code in [0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82, 85, 86, 95, 96, 99] {
            assert_ne!(description(Some(code)), "Unknown conditions", "code {code}");
        }
        assert_eq!(description(Some(42)), "Unknown conditions");
        assert_eq!(description(None), "Conditions unavailable");
        assert_eq!(description(Some(2)), "Partly cloudy");
    }
}
