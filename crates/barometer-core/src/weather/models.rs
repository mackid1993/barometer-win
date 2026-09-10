// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Ported from Sources/MenuBarStatsCore/Weather/WeatherModels.swift.
//
// The unit raw values are doing two jobs at once: they are persisted in
// settings *and* several of them are the literal query values Open-Meteo
// expects. Changing one breaks a saved configuration and a request together.

/// A saved weather location with a stable identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub id: String,
    pub name: String,
    pub admin: Option<String>,
    pub country: String,
    /// Coordinates are stored as the string form Open-Meteo returned rather
    /// than as f64, so a round trip through settings cannot shift the last
    /// decimal place and silently move somebody's town.
    pub latitude: String,
    pub longitude: String,
    pub time_zone: String,
}

impl Location {
    /// "Austin, Texas, United States", the form the picker shows.
    pub fn display_name(&self) -> String {
        let mut out = self.name.clone();
        if let Some(admin) = &self.admin {
            if !admin.is_empty() {
                out.push_str(", ");
                out.push_str(admin);
            }
        }
        if !self.country.is_empty() {
            out.push_str(", ");
            out.push_str(&self.country);
        }
        out
    }
}

/// Temperature units supported by Open-Meteo.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TemperatureUnit {
    Celsius,
    #[default]
    Fahrenheit,
}

impl TemperatureUnit {
    pub fn raw_value(self) -> &'static str {
        match self {
            TemperatureUnit::Celsius => "celsius",
            TemperatureUnit::Fahrenheit => "fahrenheit",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "celsius" => Some(TemperatureUnit::Celsius),
            "fahrenheit" => Some(TemperatureUnit::Fahrenheit),
            _ => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            TemperatureUnit::Celsius => "\u{00B0}C",
            TemperatureUnit::Fahrenheit => "\u{00B0}F",
        }
    }

    /// Converts a Celsius reading into this unit.
    ///
    /// Weather never needs this: Open-Meteo is asked to answer in the unit the
    /// user wants, so nothing is converted locally. Hardware sensors have no
    /// such option - every one of them reports Celsius, whatever the reader
    /// prefers - so the conversion has to happen here.
    pub fn from_celsius(self, degrees: f64) -> f64 {
        match self {
            TemperatureUnit::Celsius => degrees,
            TemperatureUnit::Fahrenheit => degrees * 9.0 / 5.0 + 32.0,
        }
    }

    /// A hardware temperature, rounded, with its unit: `61\u{00B0}C`.
    pub fn describe(self, celsius: f64) -> String {
        format!("{}{}", crate::format::whole(self.from_celsius(celsius)), self.symbol())
    }

    /// The widest string `describe` can produce, for reserving a column.
    ///
    /// Both are five characters, so the column does not resize when the
    /// setting changes; a processor that reaches four digits in either unit
    /// has bigger problems than the layout.
    pub fn widest(self) -> &'static str {
        match self {
            TemperatureUnit::Celsius => "100\u{00B0}C",
            TemperatureUnit::Fahrenheit => "212\u{00B0}F",
        }
    }
}

/// The widest *air* temperature the strip can be asked to draw, per unit.
///
/// This is a width, not a forecast. `whole` gives at most three characters for
/// any reading Open-Meteo returns for a place with people in it - -68 to 57 C
/// at Oymyakon and Furnace Creek, -90 to 134 F for the same pair - and the
/// widest three of them is what a column has to hold.
///
/// **Which** three cannot be read off the numbers, and this is the whole
/// reason there are two candidates here rather than one string. Segoe UI's
/// figures are tabular, so "134" and "100" measure the same and both beat
/// "-44" by the difference between a digit and a minus. Segoe UI Semibold's
/// are not: measured on the strip's own DC at 150%, `1` is six pixels, `4` is
/// nine and the rest eight, so there "-44" is the widest reading there is and
/// "100" is two pixels short of it. One literal is exact in one face and
/// short in the other, and short is what makes the column grow the first time
/// the weather turns - the sideways shuffle a reserved width exists to stop.
///
/// So the reservation is the pair, and the renderer measures both. Both are
/// readings the formatter genuinely produces: -44 is an ordinary Arctic
/// winter morning in either unit and 100 an ordinary Texan afternoon.
/// Measured exact - not merely sufficient - in both faces above; a family
/// whose widest digit is some third thing could still be a pixel short, which
/// is what the sweep in `window.rs` is there to catch.
///
/// Separate from `widest`, which is about hardware and reaches three digits
/// in Celsius where the weather never does.
pub fn reserved_air_temperature(unit: TemperatureUnit) -> String {
    let symbol = unit.symbol();
    match unit {
        // Celsius never reaches three digits - the highest air temperature
        // ever recorded is 56.7 - so its widest shape is a minus and two
        // digits and there is nothing to choose between.
        TemperatureUnit::Celsius => format!("-44{symbol}"),
        TemperatureUnit::Fahrenheit => {
            format!("100{symbol}{}-44{symbol}", crate::module::RESERVED_SEPARATOR)
        }
    }
}

/// Wind-speed units supported by Open-Meteo.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum WindSpeedUnit {
    KilometersPerHour,
    #[default]
    MilesPerHour,
    MetersPerSecond,
    Knots,
}

impl WindSpeedUnit {
    /// Also the query value sent to Open-Meteo.
    pub fn raw_value(self) -> &'static str {
        match self {
            WindSpeedUnit::KilometersPerHour => "kmh",
            WindSpeedUnit::MilesPerHour => "mph",
            WindSpeedUnit::MetersPerSecond => "ms",
            WindSpeedUnit::Knots => "kn",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "kmh" => Some(WindSpeedUnit::KilometersPerHour),
            "mph" => Some(WindSpeedUnit::MilesPerHour),
            "ms" => Some(WindSpeedUnit::MetersPerSecond),
            "kn" => Some(WindSpeedUnit::Knots),
            _ => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            WindSpeedUnit::KilometersPerHour => "km/h",
            WindSpeedUnit::MilesPerHour => "mph",
            WindSpeedUnit::MetersPerSecond => "m/s",
            WindSpeedUnit::Knots => "kn",
        }
    }
}

/// Pressure display units, converted locally from Open-Meteo's hPa values.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PressureUnit {
    Hectopascals,
    #[default]
    InchesOfMercury,
    MillimetersOfMercury,
}

impl PressureUnit {
    pub fn raw_value(self) -> &'static str {
        match self {
            PressureUnit::Hectopascals => "hectopascals",
            PressureUnit::InchesOfMercury => "inchesOfMercury",
            PressureUnit::MillimetersOfMercury => "millimetersOfMercury",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "hectopascals" => Some(PressureUnit::Hectopascals),
            "inchesOfMercury" => Some(PressureUnit::InchesOfMercury),
            "millimetersOfMercury" => Some(PressureUnit::MillimetersOfMercury),
            _ => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            PressureUnit::Hectopascals => "hPa",
            PressureUnit::InchesOfMercury => "inHg",
            PressureUnit::MillimetersOfMercury => "mmHg",
        }
    }

    /// Open-Meteo reports pressure in hectopascals and this is the only unit
    /// converted on our side rather than by the API, which is why it has a
    /// factor and the others do not.
    pub fn from_hectopascals(self, hpa: f64) -> f64 {
        match self {
            PressureUnit::Hectopascals => hpa,
            PressureUnit::InchesOfMercury => hpa * 0.029_529_98,
            PressureUnit::MillimetersOfMercury => hpa * 0.750_061_7,
        }
    }
}

/// Precipitation units supported by Open-Meteo.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PrecipitationUnit {
    Millimeters,
    #[default]
    Inches,
}

impl PrecipitationUnit {
    /// Also the query value sent to Open-Meteo.
    pub fn raw_value(self) -> &'static str {
        match self {
            PrecipitationUnit::Millimeters => "mm",
            PrecipitationUnit::Inches => "inch",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "mm" => Some(PrecipitationUnit::Millimeters),
            "inch" => Some(PrecipitationUnit::Inches),
            _ => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            PrecipitationUnit::Millimeters => "mm",
            PrecipitationUnit::Inches => "in",
        }
    }
}

/// Independent unit choices used for requests and presentation.
///
/// Independent is the point: somebody can want Fahrenheit with km/h, and the
/// macOS app lets them, so this is four separate choices rather than one
/// imperial/metric switch.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WeatherUnits {
    pub temperature: TemperatureUnit,
    pub wind_speed: WindSpeedUnit,
    pub pressure: PressureUnit,
    pub precipitation: PrecipitationUnit,
}

impl WeatherUnits {
    /// U.S. customary defaults.
    pub const IMPERIAL: WeatherUnits = WeatherUnits {
        temperature: TemperatureUnit::Fahrenheit,
        wind_speed: WindSpeedUnit::MilesPerHour,
        pressure: PressureUnit::InchesOfMercury,
        precipitation: PrecipitationUnit::Inches,
    };

    /// Common metric defaults.
    pub const METRIC: WeatherUnits = WeatherUnits {
        temperature: TemperatureUnit::Celsius,
        wind_speed: WindSpeedUnit::KilometersPerHour,
        pressure: PressureUnit::Hectopascals,
        precipitation: PrecipitationUnit::Millimeters,
    };
}

impl Default for WeatherUnits {
    fn default() -> Self {
        WeatherUnits::IMPERIAL
    }
}

/// Persisted choices for the Weather module.
///
/// `detail_sections` is not ported yet; see WeatherDetailSettings.swift for
/// which sections of the day forecast can be shown.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherSettings {
    /// Saved locations, in the order the user arranged them.
    pub locations: Vec<Location>,
    /// Identity of the location shown on the strip.
    pub primary_location_id: Option<String>,
    /// Whether to resolve the machine's current location.
    ///
    /// Defaults on here, where the Swift defaults it off. That is deliberate
    /// and is the one place the two apps differ on purpose: macOS resolves
    /// this through CoreLocation, which raises a consent prompt, so defaulting
    /// it on would mean prompting somebody who has not asked for weather yet.
    /// Windows resolves it from the IP address instead - no prompt, no
    /// precision worth worrying about - and defaulting it off would mean a
    /// fresh install shows an empty weather column that looks broken.
    pub uses_current_location: bool,
    pub units: WeatherUnits,
    /// Refresh interval, constrained by the UI to 5 through 60 minutes.
    pub refresh_interval_minutes: u32,
    /// Whether weather icons stay in color when the rest of the strip is monochrome.
    pub uses_color_icons: bool,
}

impl Default for WeatherSettings {
    fn default() -> Self {
        WeatherSettings {
            locations: Vec::new(),
            primary_location_id: None,
            uses_current_location: true,
            units: WeatherUnits::IMPERIAL,
            refresh_interval_minutes: 15,
            uses_color_icons: true,
        }
    }
}

/// The interval bounds the settings UI enforces.
pub const REFRESH_INTERVAL_MINUTES: std::ops::RangeInclusive<u32> = 5..=60;

impl WeatherSettings {
    /// The selected location, falling back to the first saved one.
    pub fn primary_location(&self) -> Option<&Location> {
        match &self.primary_location_id {
            Some(id) => self
                .locations
                .iter()
                .find(|l| &l.id == id)
                .or_else(|| self.locations.first()),
            None => self.locations.first(),
        }
    }

    /// Clamps the refresh interval to what the UI allows.
    pub fn clamped_refresh_minutes(&self) -> u32 {
        self.refresh_interval_minutes
            .clamp(*REFRESH_INTERVAL_MINUTES.start(), *REFRESH_INTERVAL_MINUTES.end())
    }
}

#[cfg(test)]
mod temperature_tests {
    use super::*;

    #[test]
    fn hardware_readings_convert_because_the_hardware_only_speaks_celsius() {
        // Weather asks Open-Meteo for the unit it wants and converts nothing.
        // Sensors have no such option, so this is the only temperature
        // conversion in the program and it has to be right.
        assert_eq!(TemperatureUnit::Celsius.describe(61.0), "61\u{00B0}C");
        assert_eq!(TemperatureUnit::Fahrenheit.describe(61.0), "142\u{00B0}F");
        assert_eq!(TemperatureUnit::Fahrenheit.describe(0.0), "32\u{00B0}F");
        assert_eq!(TemperatureUnit::Fahrenheit.describe(100.0), "212\u{00B0}F");
    }

    #[test]
    fn no_air_temperature_anybody_lives_in_is_wider_than_the_width_reserved_for_one() {
        // Every reading Open-Meteo can return for an inhabited place, against
        // the string the weather column holds room for. The bounds are the
        // records: 56.7 C / 134 F at Furnace Creek, and -68 C / -90 F at
        // Oymyakon, which is the coldest permanently inhabited place there is.
        // Every candidate has to be a string the formatter could produce, or
        // the column is reserving room for a reading that cannot happen.
        let reserved = |unit: TemperatureUnit| {
            reserved_air_temperature(unit)
                .split(crate::module::RESERVED_SEPARATOR)
                .map(|candidate| candidate.chars().count())
                .max()
                .expect("a reservation has at least one candidate")
        };
        for tenths in -680..=567 {
            let celsius = f64::from(tenths) / 10.0;
            let text = format!("{}{}", crate::format::whole(celsius), TemperatureUnit::Celsius.symbol());
            assert!(
                text.chars().count() <= reserved(TemperatureUnit::Celsius),
                "{text} against {}",
                reserved_air_temperature(TemperatureUnit::Celsius)
            );
        }
        for tenths in -900..=1340 {
            let fahrenheit = f64::from(tenths) / 10.0;
            let text =
                format!("{}{}", crate::format::whole(fahrenheit), TemperatureUnit::Fahrenheit.symbol());
            assert!(
                text.chars().count() <= reserved(TemperatureUnit::Fahrenheit),
                "{text} against {}",
                reserved_air_temperature(TemperatureUnit::Fahrenheit)
            );
        }
        // Three characters and not a fourth: covering Fahrenheit below -99
        // would cost every strip that shows the weather a whole character
        // forever, for the interior of Antarctica.
        assert_eq!(reserved(TemperatureUnit::Fahrenheit), "100\u{00B0}F".chars().count());
        assert_eq!(reserved(TemperatureUnit::Celsius), "-99\u{00B0}C".chars().count());
        // Which three characters is a pixel question, not a character one -
        // see the note on `reserved_air_temperature` and the measured test in
        // `window.rs`, which is what proves these are the widest.
        assert_eq!(reserved_air_temperature(TemperatureUnit::Celsius), "-44\u{00B0}C");
        assert_eq!(
            reserved_air_temperature(TemperatureUnit::Fahrenheit),
            "100\u{00B0}F\n-44\u{00B0}F"
        );
    }

    #[test]
    fn both_units_reserve_the_same_width() {
        // The strip sizes a column from the widest string its reading can
        // produce, so changing the unit must not shove the columns beside it.
        assert_eq!(
            TemperatureUnit::Celsius.widest().chars().count(),
            TemperatureUnit::Fahrenheit.widest().chars().count()
        );
    }
}
