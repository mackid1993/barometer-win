// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Weather, ported from Sources/MenuBarStatsCore/Weather/.
//
// Data comes from Open-Meteo, which is free, needs no account and no key, and
// is the same source the macOS app uses - so the two apps agree about the
// weather rather than merely both having some.

pub mod air;
pub mod badge;
pub mod client;
pub mod detail;
pub mod glyph;
pub mod models;

pub use client::{fetch, search, CurrentWeather, WeatherError};
pub use models::{
    Location, PrecipitationUnit, PressureUnit, TemperatureUnit, WeatherSettings, WeatherUnits,
    WindSpeedUnit, REFRESH_INTERVAL_MINUTES,
};

#[cfg(test)]
mod tests {
    use super::models::*;

    fn austin() -> Location {
        Location {
            id: "austin-tx".into(),
            name: "Austin".into(),
            admin: Some("Texas".into()),
            country: "United States".into(),
            latitude: "30.2672".into(),
            longitude: "-97.7431".into(),
            time_zone: "America/Chicago".into(),
        }
    }

    #[test]
    fn unit_raw_values_round_trip() {
        // These are persisted and several are also Open-Meteo query values, so
        // a change breaks a saved configuration and a request at once.
        for u in [TemperatureUnit::Celsius, TemperatureUnit::Fahrenheit] {
            assert_eq!(TemperatureUnit::from_raw(u.raw_value()), Some(u));
        }
        for u in [
            WindSpeedUnit::KilometersPerHour,
            WindSpeedUnit::MilesPerHour,
            WindSpeedUnit::MetersPerSecond,
            WindSpeedUnit::Knots,
        ] {
            assert_eq!(WindSpeedUnit::from_raw(u.raw_value()), Some(u));
        }
        for u in [PrecipitationUnit::Millimeters, PrecipitationUnit::Inches] {
            assert_eq!(PrecipitationUnit::from_raw(u.raw_value()), Some(u));
        }
        for u in [
            PressureUnit::Hectopascals,
            PressureUnit::InchesOfMercury,
            PressureUnit::MillimetersOfMercury,
        ] {
            assert_eq!(PressureUnit::from_raw(u.raw_value()), Some(u));
        }
    }

    #[test]
    fn wind_query_values_are_what_open_meteo_expects() {
        assert_eq!(WindSpeedUnit::MilesPerHour.raw_value(), "mph");
        assert_eq!(WindSpeedUnit::KilometersPerHour.raw_value(), "kmh");
        assert_eq!(PrecipitationUnit::Inches.raw_value(), "inch");
    }

    #[test]
    fn pressure_is_the_only_unit_converted_locally() {
        // Open-Meteo reports hPa; the others it converts server side.
        let hpa = 1013.25;
        assert_eq!(PressureUnit::Hectopascals.from_hectopascals(hpa), hpa);
        let inhg = PressureUnit::InchesOfMercury.from_hectopascals(hpa);
        assert!((inhg - 29.92).abs() < 0.01, "standard pressure should be ~29.92 inHg, got {inhg}");
        let mmhg = PressureUnit::MillimetersOfMercury.from_hectopascals(hpa);
        assert!((mmhg - 760.0).abs() < 0.5, "standard pressure should be ~760 mmHg, got {mmhg}");
    }

    #[test]
    fn units_are_independent_not_one_switch() {
        // Fahrenheit with km/h is a configuration somebody actually wants.
        let mixed = WeatherUnits {
            temperature: TemperatureUnit::Fahrenheit,
            wind_speed: WindSpeedUnit::KilometersPerHour,
            ..WeatherUnits::IMPERIAL
        };
        assert_eq!(mixed.temperature.symbol(), "\u{00B0}F");
        assert_eq!(mixed.wind_speed.symbol(), "km/h");
    }

    #[test]
    fn the_primary_location_falls_back_to_the_first_saved() {
        let mut settings = WeatherSettings { locations: vec![austin()], ..Default::default() };
        assert_eq!(settings.primary_location().map(|l| l.name.as_str()), Some("Austin"));
        // An id that no longer matches anything must not blank the readout.
        settings.primary_location_id = Some("deleted".into());
        assert_eq!(settings.primary_location().map(|l| l.name.as_str()), Some("Austin"));
    }

    #[test]
    fn no_locations_means_no_primary() {
        assert!(WeatherSettings::default().primary_location().is_none());
    }

    #[test]
    fn the_refresh_interval_is_clamped_to_what_the_ui_allows() {
        let mut settings = WeatherSettings::default();
        assert_eq!(settings.clamped_refresh_minutes(), 15);
        settings.refresh_interval_minutes = 1;
        assert_eq!(settings.clamped_refresh_minutes(), 5);
        settings.refresh_interval_minutes = 600;
        assert_eq!(settings.clamped_refresh_minutes(), 60);
    }

    #[test]
    fn a_location_reads_as_city_region_country() {
        assert_eq!(austin().display_name(), "Austin, Texas, United States");
    }
}
