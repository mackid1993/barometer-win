// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The detailed forecast, ported from WeatherDetailMetrics.swift,
// WeatherDetailResponse.swift and WeatherDayDetails.swift.
//
// client.rs answers "what is it doing out there now", which is all the strip
// needs. This answers "what is it going to do": ten days of summaries, the
// hours inside them, and the long tail of optional metrics Open-Meteo will
// report if asked - soil temperatures, radiation, wet bulb, moon phase.
//
// Two things are worth knowing before reading it.
//
// Times never become absolute instants. Open-Meteo returns local wall-clock
// strings for the location's own zone ("2026-09-07T14:00"), and every question
// the panel asks - which hours belong to this day, when is sunset - is a
// question about those wall clocks. Parsing them into fields and comparing the
// fields answers all of it with no time zone database at all, which is why
// this file needs neither chrono nor an IANA table. The single absolute
// instant that is genuinely needed, for the moon, is built from the response's
// own utc_offset_seconds.
//
// Nothing is required. Models and stations omit whole variables, and the rule
// parse_current already follows holds here too: a missing field costs that
// field and nothing else.

use serde_json::Value;

use crate::net;
use crate::weather::client::WeatherError;
use crate::weather::models::{Location, WeatherUnits};

/// The same host client.rs uses, which keeps its own copy private.
const FORECAST_HOST: &str = "api.open-meteo.com";

/// Days of forecast asked for, as in OpenMeteoClient.forecast(for:units:).
const FORECAST_DAYS: u32 = 10;

/// Exact, and not the survey foot: the provider means international feet.
const FEET_TO_METERS: f64 = 0.3048;

/// Length of the mean synodic month, in seconds.
const SYNODIC_MONTH_SECONDS: f64 = 29.530_588_853 * 86_400.0;

/// A new moon that actually happened: 2000-01-06 18:14 UTC.
///
/// The same constant MoonPhase.cycleFraction(for:) uses, so both apps estimate
/// the same phase for the same instant when the provider sends no lunar data.
const REFERENCE_NEW_MOON: f64 = 947_182_440.0;

/// The hourly fields the panel reads directly, before the metric catalog.
const HOURLY_BASE: [&str; 13] = [
    "temperature_2m",
    "apparent_temperature",
    "precipitation_probability",
    "precipitation",
    "weather_code",
    "wind_speed_10m",
    "wind_direction_10m",
    "uv_index",
    "is_day",
    "relative_humidity_2m",
    "dew_point_2m",
    "visibility",
    "cloud_cover",
];

/// The daily fields the panel reads directly, before the metric catalog.
const DAILY_BASE: [&str; 12] = [
    "weather_code",
    "temperature_2m_max",
    "temperature_2m_min",
    "apparent_temperature_max",
    "apparent_temperature_min",
    "sunrise",
    "sunset",
    "uv_index_max",
    "precipitation_sum",
    "precipitation_probability_max",
    "wind_speed_10m_max",
    "wind_gusts_10m_max",
];

// ---------------------------------------------------------------------------
// Local wall-clock time
// ---------------------------------------------------------------------------

/// One calendar date in the location's own zone.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl LocalDate {
    /// Reads "2026-09-07", refusing anything that is not a plausible date.
    pub fn parse(value: &str) -> Option<LocalDate> {
        let bytes = value.as_bytes();
        if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let date = LocalDate {
            year: value.get(0..4)?.parse().ok()?,
            month: value.get(5..7)?.parse().ok()?,
            day: value.get(8..10)?.parse().ok()?,
        };
        // Nothing downstream can do anything sensible with month 13, and the
        // arithmetic below would answer confidently rather than refuse.
        ((1..=12).contains(&date.month) && (1..=31).contains(&date.day)).then_some(date)
    }

    /// Days since 1970-01-01, negative before it.
    ///
    /// Howard Hinnant's days_from_civil, exact for every proleptic Gregorian
    /// date. It is what makes a weekday and a date difference possible here
    /// without a calendar library.
    pub fn days_from_epoch(self) -> i64 {
        let year = i64::from(if self.month <= 2 { self.year - 1 } else { self.year });
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let shifted_month = i64::from((self.month + 9) % 12);
        let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// Day of the week, 0 for Sunday through 6 for Saturday.
    ///
    /// Not in the Swift, which asks Foundation. A ten-day list has to be
    /// labeled with weekday names and nothing else in this port knows how to
    /// work one out, so it sits beside the date arithmetic that already exists
    /// rather than being reinvented in the panel.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday, which is index 4 counting from Sunday.
        (self.days_from_epoch() + 4).rem_euclid(7) as u32
    }
}

/// One wall-clock instant in the location's own zone.
///
/// Deliberately not a moment in absolute time. The provider's local strings
/// are what the panel displays and what the day grouping compares, and turning
/// them into instants would need zone rules this port has no source for.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalTime {
    pub date: LocalDate,
    pub hour: u32,
    pub minute: u32,
}

impl LocalTime {
    /// Reads "2026-09-07T14:00", ignoring any seconds the provider adds.
    pub fn parse(value: &str) -> Option<LocalTime> {
        let (date, clock) = value.split_once('T').or_else(|| value.split_once(' '))?;
        let (hour, minute) = clock.get(0..5)?.split_once(':')?;
        let time = LocalTime {
            date: LocalDate::parse(date)?,
            hour: hour.parse().ok()?,
            minute: minute.parse().ok()?,
        };
        (time.hour < 24 && time.minute < 60).then_some(time)
    }

    /// Seconds since the epoch as if the wall clock were UTC.
    ///
    /// Meaningful only as a difference between two of these: subtracting one
    /// from another gives elapsed wall-clock time, which is what a sunrise to
    /// sunset span is.
    pub fn wall_clock_seconds(self) -> i64 {
        self.date.days_from_epoch() * 86_400
            + i64::from(self.hour) * 3_600
            + i64::from(self.minute) * 60
    }

    /// The real instant this wall clock names, given the zone's offset.
    pub fn unix_seconds(self, utc_offset_seconds: i64) -> i64 {
        self.wall_clock_seconds() - utc_offset_seconds
    }
}

// ---------------------------------------------------------------------------
// Moon
// ---------------------------------------------------------------------------

/// Eight-phase lunar cycle, from the provider or from the mean synodic cycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MoonPhase {
    NewMoon,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    FullMoon,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl MoonPhase {
    /// In cycle order, which is the order the eighth indexes into.
    pub const ALL: [MoonPhase; 8] = [
        MoonPhase::NewMoon,
        MoonPhase::WaxingCrescent,
        MoonPhase::FirstQuarter,
        MoonPhase::WaxingGibbous,
        MoonPhase::FullMoon,
        MoonPhase::WaningGibbous,
        MoonPhase::LastQuarter,
        MoonPhase::WaningCrescent,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MoonPhase::NewMoon => "New Moon",
            MoonPhase::WaxingCrescent => "Waxing Crescent",
            MoonPhase::FirstQuarter => "First Quarter",
            MoonPhase::WaxingGibbous => "Waxing Gibbous",
            MoonPhase::FullMoon => "Full Moon",
            MoonPhase::WaningGibbous => "Waning Gibbous",
            MoonPhase::LastQuarter => "Last Quarter",
            MoonPhase::WaningCrescent => "Waning Crescent",
        }
    }

    /// How far through the cycle an instant is, from zero to just under one.
    ///
    /// Swift normalizes with two truncating remainders because it has no
    /// Euclidean one. rem_euclid is the same normalization in a single step,
    /// and it is what keeps dates before the reference new moon on the cycle
    /// instead of on a negative mirror of it.
    pub fn cycle_fraction(unix_seconds: f64) -> f64 {
        ((unix_seconds - REFERENCE_NEW_MOON) / SYNODIC_MONTH_SECONDS).rem_euclid(1.0)
    }

    /// The nearest named eighth of a cycle fraction.
    pub fn from_cycle_fraction(fraction: f64) -> MoonPhase {
        let index = ((fraction * 8.0 + 0.5).floor() as i64).rem_euclid(8) as usize;
        MoonPhase::ALL[index]
    }

    /// The estimated phase at an instant, for when the provider sends none.
    pub fn at(unix_seconds: f64) -> MoonPhase {
        MoonPhase::from_cycle_fraction(MoonPhase::cycle_fraction(unix_seconds))
    }

    /// Fraction of the disc lit, from a cycle fraction.
    pub fn illumination(fraction: f64) -> f64 {
        (1.0 - (2.0 * std::f64::consts::PI * fraction).cos()) / 2.0
    }

    /// Estimated illuminated fraction at an instant, from the same mean cycle.
    pub fn illumination_at(unix_seconds: f64) -> f64 {
        MoonPhase::illumination(MoonPhase::cycle_fraction(unix_seconds))
    }
}

// ---------------------------------------------------------------------------
// The metric catalog
// ---------------------------------------------------------------------------

/// What the two metric catalogs have in common.
///
/// The Swift passes the set of length fields in at each call site; here each
/// metric answers for itself, because both call sites always passed the same
/// set for a given catalog and a fact about a metric belongs on the metric.
pub trait WeatherMetric: Copy + PartialEq + Sized + 'static {
    /// Every metric requested, in the order the request lists them.
    const ALL: &'static [Self];

    /// Open-Meteo's own parameter name. This is wire format: it goes out in
    /// the query and comes back as the key, so it is not free to be tidied.
    fn raw_value(self) -> &'static str;

    /// Whether this metric is a length the provider may report in feet.
    fn is_length(self) -> bool;
}

/// Extra surface-weather fields requested for each forecast hour.
///
/// The variant names are American and the raw values are British, exactly as
/// in the Swift: the provider spells it "vapour" and the raw value is the wire
/// format, so it is not ours to correct.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HourlyMetric {
    Rain,
    Showers,
    Snowfall,
    SnowDepth,
    SeaLevelPressure,
    SurfacePressure,
    WindGusts,
    CloudCoverLow,
    CloudCoverMid,
    CloudCoverHigh,
    WetBulbTemperature,
    SunshineDuration,
    UVIndexClearSky,
    FreezingLevelHeight,
    Cape,
    VaporPressureDeficit,
    Evapotranspiration,
    ReferenceEvapotranspiration,
    ShortwaveRadiation,
    DirectRadiation,
    DiffuseRadiation,
    DirectNormalIrradiance,
    SoilTemperature0cm,
    SoilTemperature6cm,
    SoilTemperature18cm,
    SoilTemperature54cm,
    SoilMoisture0To1cm,
    SoilMoisture1To3cm,
    SoilMoisture3To9cm,
    SoilMoisture9To27cm,
    SoilMoisture27To81cm,
}

impl WeatherMetric for HourlyMetric {
    const ALL: &'static [HourlyMetric] = &[
        HourlyMetric::Rain,
        HourlyMetric::Showers,
        HourlyMetric::Snowfall,
        HourlyMetric::SnowDepth,
        HourlyMetric::SeaLevelPressure,
        HourlyMetric::SurfacePressure,
        HourlyMetric::WindGusts,
        HourlyMetric::CloudCoverLow,
        HourlyMetric::CloudCoverMid,
        HourlyMetric::CloudCoverHigh,
        HourlyMetric::WetBulbTemperature,
        HourlyMetric::SunshineDuration,
        HourlyMetric::UVIndexClearSky,
        HourlyMetric::FreezingLevelHeight,
        HourlyMetric::Cape,
        HourlyMetric::VaporPressureDeficit,
        HourlyMetric::Evapotranspiration,
        HourlyMetric::ReferenceEvapotranspiration,
        HourlyMetric::ShortwaveRadiation,
        HourlyMetric::DirectRadiation,
        HourlyMetric::DiffuseRadiation,
        HourlyMetric::DirectNormalIrradiance,
        HourlyMetric::SoilTemperature0cm,
        HourlyMetric::SoilTemperature6cm,
        HourlyMetric::SoilTemperature18cm,
        HourlyMetric::SoilTemperature54cm,
        HourlyMetric::SoilMoisture0To1cm,
        HourlyMetric::SoilMoisture1To3cm,
        HourlyMetric::SoilMoisture3To9cm,
        HourlyMetric::SoilMoisture9To27cm,
        HourlyMetric::SoilMoisture27To81cm,
    ];

    fn raw_value(self) -> &'static str {
        match self {
            HourlyMetric::Rain => "rain",
            HourlyMetric::Showers => "showers",
            HourlyMetric::Snowfall => "snowfall",
            HourlyMetric::SnowDepth => "snow_depth",
            HourlyMetric::SeaLevelPressure => "pressure_msl",
            HourlyMetric::SurfacePressure => "surface_pressure",
            HourlyMetric::WindGusts => "wind_gusts_10m",
            HourlyMetric::CloudCoverLow => "cloud_cover_low",
            HourlyMetric::CloudCoverMid => "cloud_cover_mid",
            HourlyMetric::CloudCoverHigh => "cloud_cover_high",
            HourlyMetric::WetBulbTemperature => "wet_bulb_temperature_2m",
            HourlyMetric::SunshineDuration => "sunshine_duration",
            HourlyMetric::UVIndexClearSky => "uv_index_clear_sky",
            HourlyMetric::FreezingLevelHeight => "freezing_level_height",
            HourlyMetric::Cape => "cape",
            HourlyMetric::VaporPressureDeficit => "vapour_pressure_deficit",
            HourlyMetric::Evapotranspiration => "evapotranspiration",
            HourlyMetric::ReferenceEvapotranspiration => "et0_fao_evapotranspiration",
            HourlyMetric::ShortwaveRadiation => "shortwave_radiation",
            HourlyMetric::DirectRadiation => "direct_radiation",
            HourlyMetric::DiffuseRadiation => "diffuse_radiation",
            HourlyMetric::DirectNormalIrradiance => "direct_normal_irradiance",
            HourlyMetric::SoilTemperature0cm => "soil_temperature_0cm",
            HourlyMetric::SoilTemperature6cm => "soil_temperature_6cm",
            HourlyMetric::SoilTemperature18cm => "soil_temperature_18cm",
            HourlyMetric::SoilTemperature54cm => "soil_temperature_54cm",
            HourlyMetric::SoilMoisture0To1cm => "soil_moisture_0_to_1cm",
            HourlyMetric::SoilMoisture1To3cm => "soil_moisture_1_to_3cm",
            HourlyMetric::SoilMoisture3To9cm => "soil_moisture_3_to_9cm",
            HourlyMetric::SoilMoisture9To27cm => "soil_moisture_9_to_27cm",
            HourlyMetric::SoilMoisture27To81cm => "soil_moisture_27_to_81cm",
        }
    }

    fn is_length(self) -> bool {
        matches!(self, HourlyMetric::SnowDepth | HourlyMetric::FreezingLevelHeight)
    }
}

/// Extra daily summaries, in units matching the forecast's unit selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DailyMetric {
    RainSum,
    ShowersSum,
    SnowfallSum,
    PrecipitationHours,
    DaylightDuration,
    SunshineDuration,
    WindDirectionDominant,
    ShortwaveRadiationSum,
    ReferenceEvapotranspiration,
    UVIndexClearSkyMax,
    TemperatureMean,
    ApparentTemperatureMean,
    HumidityMean,
    HumidityMin,
    HumidityMax,
    DewPointMean,
    CloudCoverMean,
    PressureMean,
    SurfacePressureMean,
    VisibilityMean,
    VisibilityMin,
    CapeMax,
    WetBulbTemperatureMean,
    VaporPressureDeficitMax,
    MoonPhase,
}

impl WeatherMetric for DailyMetric {
    const ALL: &'static [DailyMetric] = &[
        DailyMetric::RainSum,
        DailyMetric::ShowersSum,
        DailyMetric::SnowfallSum,
        DailyMetric::PrecipitationHours,
        DailyMetric::DaylightDuration,
        DailyMetric::SunshineDuration,
        DailyMetric::WindDirectionDominant,
        DailyMetric::ShortwaveRadiationSum,
        DailyMetric::ReferenceEvapotranspiration,
        DailyMetric::UVIndexClearSkyMax,
        DailyMetric::TemperatureMean,
        DailyMetric::ApparentTemperatureMean,
        DailyMetric::HumidityMean,
        DailyMetric::HumidityMin,
        DailyMetric::HumidityMax,
        DailyMetric::DewPointMean,
        DailyMetric::CloudCoverMean,
        DailyMetric::PressureMean,
        DailyMetric::SurfacePressureMean,
        DailyMetric::VisibilityMean,
        DailyMetric::VisibilityMin,
        DailyMetric::CapeMax,
        DailyMetric::WetBulbTemperatureMean,
        DailyMetric::VaporPressureDeficitMax,
        DailyMetric::MoonPhase,
    ];

    fn raw_value(self) -> &'static str {
        match self {
            DailyMetric::RainSum => "rain_sum",
            DailyMetric::ShowersSum => "showers_sum",
            DailyMetric::SnowfallSum => "snowfall_sum",
            DailyMetric::PrecipitationHours => "precipitation_hours",
            DailyMetric::DaylightDuration => "daylight_duration",
            DailyMetric::SunshineDuration => "sunshine_duration",
            DailyMetric::WindDirectionDominant => "wind_direction_10m_dominant",
            DailyMetric::ShortwaveRadiationSum => "shortwave_radiation_sum",
            DailyMetric::ReferenceEvapotranspiration => "et0_fao_evapotranspiration",
            DailyMetric::UVIndexClearSkyMax => "uv_index_clear_sky_max",
            DailyMetric::TemperatureMean => "temperature_2m_mean",
            DailyMetric::ApparentTemperatureMean => "apparent_temperature_mean",
            DailyMetric::HumidityMean => "relative_humidity_2m_mean",
            DailyMetric::HumidityMin => "relative_humidity_2m_min",
            DailyMetric::HumidityMax => "relative_humidity_2m_max",
            DailyMetric::DewPointMean => "dew_point_2m_mean",
            DailyMetric::CloudCoverMean => "cloud_cover_mean",
            DailyMetric::PressureMean => "pressure_msl_mean",
            DailyMetric::SurfacePressureMean => "surface_pressure_mean",
            DailyMetric::VisibilityMean => "visibility_mean",
            DailyMetric::VisibilityMin => "visibility_min",
            DailyMetric::CapeMax => "cape_max",
            DailyMetric::WetBulbTemperatureMean => "wet_bulb_temperature_2m_mean",
            DailyMetric::VaporPressureDeficitMax => "vapour_pressure_deficit_max",
            DailyMetric::MoonPhase => "moon_phase",
        }
    }

    fn is_length(self) -> bool {
        matches!(self, DailyMetric::VisibilityMean | DailyMetric::VisibilityMin)
    }
}

/// The metrics one forecast point actually came back with.
///
/// Absent means the provider sent nothing usable: no key, a null, an array too
/// short to reach, or a value that is not finite. No zero stands in for any of
/// those, which is the whole point of the type.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricValues<M: WeatherMetric> {
    values: Vec<(M, f64)>,
}

impl<M: WeatherMetric> Default for MetricValues<M> {
    fn default() -> Self {
        MetricValues { values: Vec::new() }
    }
}

impl<M: WeatherMetric> MetricValues<M> {
    /// The value in the provider's units, or None if it did not send one.
    pub fn get(&self, metric: M) -> Option<f64> {
        self.values.iter().find(|(held, _)| *held == metric).map(|(_, value)| *value)
    }

    /// Whether the provider sent nothing at all for this point.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Everything present, in catalog order.
    pub fn iter(&self) -> impl Iterator<Item = (M, f64)> + '_ {
        self.values.iter().copied()
    }
}

/// Optional hourly details.
pub type HourlyDetails = MetricValues<HourlyMetric>;

/// Optional daily details.
pub type DailyDetails = MetricValues<DailyMetric>;

// ---------------------------------------------------------------------------
// Forecast points
// ---------------------------------------------------------------------------

/// One hourly forecast point.
#[derive(Clone, Debug, PartialEq)]
pub struct HourlyPoint {
    pub time: LocalTime,
    pub temperature: Option<f64>,
    pub apparent_temperature: Option<f64>,
    pub precipitation_probability: Option<f64>,
    pub precipitation: Option<f64>,
    pub weather_code: Option<u8>,
    pub wind_speed: Option<f64>,
    pub wind_direction: Option<f64>,
    pub uv_index: Option<f64>,
    pub is_day: Option<bool>,
    pub humidity: Option<f64>,
    pub dew_point: Option<f64>,
    /// Meters, normalized here when the response says it came in feet.
    pub visibility: Option<f64>,
    pub cloud_cover: Option<f64>,
    pub details: HourlyDetails,
}

/// One daily forecast point.
///
/// One divergence from DailyWeatherDetails: the lunar events sit here rather
/// than inside the details. They are per-day facts of the same kind as sunrise
/// and sunset, which are already fields, and in Swift they live in the details
/// only because that struct is the optional block added later for cache
/// compatibility - a concern this port does not have, since nothing here
/// persists a forecast.
#[derive(Clone, Debug, PartialEq)]
pub struct DailyPoint {
    pub date: LocalDate,
    pub weather_code: Option<u8>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub apparent_high: Option<f64>,
    pub apparent_low: Option<f64>,
    pub sunrise: Option<LocalTime>,
    pub sunset: Option<LocalTime>,
    pub uv_index_max: Option<f64>,
    pub precipitation: Option<f64>,
    pub precipitation_probability: Option<f64>,
    pub wind_speed_max: Option<f64>,
    pub wind_gusts_max: Option<f64>,
    pub details: DailyDetails,
    /// Moonrise in the location's own zone, absent when there was none.
    pub moonrise: Option<LocalTime>,
    /// Moonset in the location's own zone, absent when there was none.
    pub moonset: Option<LocalTime>,
    /// Whether lunar events were asked for at all.
    ///
    /// Tells a night with genuinely no moonrise apart from a response that
    /// never carried the field, which the panel has to word differently.
    pub moon_events_available: bool,
}

/// A parsed multi-day forecast.
///
/// Narrower than Swift's Forecast, which also carries the location, the units,
/// the current conditions and a fetch timestamp. The first two belong to the
/// caller that asked, the current conditions are client.rs's business, and
/// nothing here is cached, so the timestamp would have no reader.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailForecast {
    /// IANA identifier the provider resolved for the coordinates.
    pub time_zone: String,
    /// The offset the local times above are expressed in.
    pub utc_offset_seconds: i64,
    pub hourly: Vec<HourlyPoint>,
    pub daily: Vec<DailyPoint>,
}

impl DetailForecast {
    /// One day of the forecast, with the hours that fall inside it.
    pub fn day(&self, index: usize) -> Option<DayDetails> {
        let day = self.daily.get(index)?;
        Some(DayDetails::new(day, &self.hourly, self.utc_offset_seconds))
    }
}

/// One forecast day, grouped by the location's calendar rather than ours.
#[derive(Clone, Debug)]
pub struct DayDetails {
    pub day: DailyPoint,
    /// Chronologically ordered hours inside the selected local day.
    pub hourly: Vec<HourlyPoint>,
    pub moon_phase: MoonPhase,
    /// Approximate fraction illuminated, from zero to one.
    pub moon_illumination: f64,
    /// Whether the mean-cycle fallback was needed instead of provider data.
    pub uses_estimated_moon_phase: bool,
}

impl DayDetails {
    /// Groups by the location's own calendar day, and dates the moon at local noon.
    pub fn new(day: &DailyPoint, hourly: &[HourlyPoint], utc_offset_seconds: i64) -> DayDetails {
        // Swift asks a Calendar for the day's half-open interval, because a
        // day is not always 86400 seconds long: "start plus a day" loses an
        // hour every autumn and invents one every spring. Comparing the local
        // date is the same answer without the calendar - a wall clock reading
        // 2026-11-01 is inside that day whether the day ran 23, 24 or 25
        // hours, and a repeated hour after the clocks go back is counted
        // twice because it genuinely happened twice.
        let mut hours: Vec<HourlyPoint> =
            hourly.iter().filter(|point| point.time.date == day.date).cloned().collect();
        hours.sort_by_key(|point| point.time);

        match day.details.get(DailyMetric::MoonPhase) {
            Some(cycle) if cycle.is_finite() && (0.0..=1.0).contains(&cycle) => DayDetails {
                day: day.clone(),
                hourly: hours,
                moon_phase: MoonPhase::from_cycle_fraction(cycle),
                moon_illumination: MoonPhase::illumination(cycle),
                uses_estimated_moon_phase: false,
            },
            _ => {
                // The provider does not always carry lunar data, so the app
                // estimates from the mean synodic cycle - and says that it is
                // estimating rather than quietly showing the worse number.
                // Local noon, so the phase belongs to the day being looked at
                // rather than to whatever hour the machine is at.
                let noon = LocalTime { date: day.date, hour: 12, minute: 0 };
                let instant = noon.unix_seconds(utc_offset_seconds) as f64;
                DayDetails {
                    day: day.clone(),
                    hourly: hours,
                    moon_phase: MoonPhase::at(instant),
                    moon_illumination: MoonPhase::illumination_at(instant),
                    uses_estimated_moon_phase: true,
                }
            }
        }
    }

    /// Seconds of daylight, or None when nothing available says.
    ///
    /// The range guard is the polar one: inside the Arctic circle a summer day
    /// has no sunrise and no sunset, the provider sends null for both, and
    /// reading that as zero would report total darkness in the land of the
    /// midnight sun. Unknown is the honest answer.
    pub fn daylight_duration(&self) -> Option<f64> {
        if let Some(duration) = self.day.details.get(DailyMetric::DaylightDuration) {
            if duration.is_finite() && (0.0..=86_400.0).contains(&duration) {
                return Some(duration);
            }
        }
        let (sunrise, sunset) = (self.day.sunrise?, self.day.sunset?);
        if sunset < sunrise {
            return None;
        }
        // A wall-clock difference, which is all two local strings can support.
        // On the one day a year a clock change falls between sunrise and
        // sunset this is an hour out - but the provider's own
        // daylight_duration above is present on that day as on every other, so
        // this fallback is not what gets used there.
        Some((sunset.wall_clock_seconds() - sunrise.wall_clock_seconds()) as f64)
    }
}

// ---------------------------------------------------------------------------
// Request and response
// ---------------------------------------------------------------------------

/// The hourly parameter: base fields, then the whole metric catalog.
pub fn hourly_fields() -> String {
    let mut fields = HOURLY_BASE.join(",");
    for metric in HourlyMetric::ALL {
        fields.push(',');
        fields.push_str(metric.raw_value());
    }
    fields
}

/// The daily parameter: base fields, the catalog, then the lunar events.
pub fn daily_fields() -> String {
    let mut fields = DAILY_BASE.join(",");
    for metric in DailyMetric::ALL {
        fields.push(',');
        fields.push_str(metric.raw_value());
    }
    fields.push_str(",moonrise,moonset");
    fields
}

/// Fetches the multi-day forecast for a location.
///
/// No current block, unlike the Swift, which asks for all three at once: the
/// strip's reading comes from client.rs, and asking twice would be two answers
/// to one question that can disagree with each other on screen.
pub fn fetch_detail(
    location: &Location,
    units: WeatherUnits,
) -> Result<DetailForecast, WeatherError> {
    let query = format!(
        "/v1/forecast?latitude={}&longitude={}&timezone=auto&forecast_days={}\
&temperature_unit={}&wind_speed_unit={}&precipitation_unit={}&hourly={}&daily={}",
        net::encode(&location.latitude),
        net::encode(&location.longitude),
        FORECAST_DAYS,
        units.temperature.raw_value(),
        units.wind_speed.raw_value(),
        units.precipitation.raw_value(),
        hourly_fields(),
        daily_fields(),
    );
    let body = net::get(FORECAST_HOST, &query)?;
    parse_detail(&body)
}

/// Reads a forecast response.
///
/// Separate from the request, as in client.rs, so it can be run against a
/// recorded body. This is the part that breaks when the provider changes
/// shape, and it is impossible to exercise if it only exists inside a network
/// call.
pub fn parse_detail(body: &str) -> Result<DetailForecast, WeatherError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| WeatherError::Malformed(e.to_string()))?;
    let hourly_block = root.get("hourly");
    let daily_block = root.get("daily");
    if hourly_block.is_none() && daily_block.is_none() {
        return Err(WeatherError::Malformed("no forecast blocks".into()));
    }
    Ok(DetailForecast {
        // Swift falls back to GMT when the identifier will not resolve. An
        // absent offset is zero, which is that same fallback by another route.
        time_zone: root.get("timezone").and_then(Value::as_str).unwrap_or_default().to_string(),
        utc_offset_seconds: root.get("utc_offset_seconds").and_then(Value::as_i64).unwrap_or(0),
        hourly: hourly_block
            .map(|block| parse_hourly(block, root.get("hourly_units")))
            .unwrap_or_default(),
        daily: daily_block
            .map(|block| parse_daily(block, root.get("daily_units")))
            .unwrap_or_default(),
    })
}

fn parse_hourly(block: &Value, units: Option<&Value>) -> Vec<HourlyPoint> {
    let times = array(block, "time");
    // Each series is looked up once here rather than once per hour. Swift
    // decodes every one of these as a required key, so a single variable the
    // chosen model does not publish fails the whole forecast; here an absent
    // series is an absent field on each hour and everything else survives,
    // which is the rule parse_current already follows.
    let temperature = array(block, "temperature_2m");
    let apparent = array(block, "apparent_temperature");
    let probability = array(block, "precipitation_probability");
    let precipitation = array(block, "precipitation");
    let codes = array(block, "weather_code");
    let wind_speed = array(block, "wind_speed_10m");
    let wind_direction = array(block, "wind_direction_10m");
    let uv_index = array(block, "uv_index");
    let is_day = array(block, "is_day");
    let humidity = array(block, "relative_humidity_2m");
    let dew_point = array(block, "dew_point_2m");
    let visibility = array(block, "visibility");
    let cloud_cover = array(block, "cloud_cover");
    let visibility_in_feet = unit_is_feet(units, "visibility");
    let metrics = metric_arrays::<HourlyMetric>(block, units);

    let mut points = Vec::with_capacity(times.len());
    for index in 0..times.len() {
        // An hour whose timestamp cannot be read has nowhere to be shown and
        // no day to belong to, so it is dropped and the rest are kept.
        let Some(time) = text(times, index).and_then(LocalTime::parse) else {
            continue;
        };
        points.push(HourlyPoint {
            time,
            temperature: number(temperature, index),
            apparent_temperature: number(apparent, index),
            precipitation_probability: number(probability, index),
            precipitation: number(precipitation, index),
            weather_code: number(codes, index).map(|value| value as u8),
            wind_speed: number(wind_speed, index),
            wind_direction: number(wind_direction, index),
            uv_index: number(uv_index, index),
            is_day: number(is_day, index).map(|flag| flag != 0.0),
            humidity: number(humidity, index),
            dew_point: number(dew_point, index),
            visibility: number(visibility, index)
                .map(|value| if visibility_in_feet { value * FEET_TO_METERS } else { value }),
            cloud_cover: number(cloud_cover, index),
            details: metric_values(&metrics, index),
        });
    }
    points
}

fn parse_daily(block: &Value, units: Option<&Value>) -> Vec<DailyPoint> {
    let times = array(block, "time");
    let codes = array(block, "weather_code");
    let high = array(block, "temperature_2m_max");
    let low = array(block, "temperature_2m_min");
    let apparent_high = array(block, "apparent_temperature_max");
    let apparent_low = array(block, "apparent_temperature_min");
    let sunrise = array(block, "sunrise");
    let sunset = array(block, "sunset");
    let uv_index_max = array(block, "uv_index_max");
    let precipitation = array(block, "precipitation_sum");
    let probability = array(block, "precipitation_probability_max");
    let wind_speed_max = array(block, "wind_speed_10m_max");
    let wind_gusts_max = array(block, "wind_gusts_10m_max");
    let moonrise = block.get("moonrise").and_then(Value::as_array);
    let moonset = block.get("moonset").and_then(Value::as_array);
    // Both arrays present is what tells a day with no moonrise apart from a
    // response that never carried the field.
    let moon_events_available = moonrise.is_some() && moonset.is_some();
    let metrics = metric_arrays::<DailyMetric>(block, units);

    let mut points = Vec::with_capacity(times.len());
    for index in 0..times.len() {
        let Some(date) = text(times, index).and_then(LocalDate::parse) else {
            continue;
        };
        points.push(DailyPoint {
            date,
            weather_code: number(codes, index).map(|value| value as u8),
            high: number(high, index),
            low: number(low, index),
            apparent_high: number(apparent_high, index),
            apparent_low: number(apparent_low, index),
            sunrise: text(sunrise, index).and_then(LocalTime::parse),
            sunset: text(sunset, index).and_then(LocalTime::parse),
            uv_index_max: number(uv_index_max, index),
            precipitation: number(precipitation, index),
            precipitation_probability: number(probability, index),
            wind_speed_max: number(wind_speed_max, index),
            wind_gusts_max: number(wind_gusts_max, index),
            details: metric_values(&metrics, index),
            moonrise: moonrise.and_then(|values| text(values, index)).and_then(LocalTime::parse),
            moonset: moonset.and_then(|values| text(values, index)).and_then(LocalTime::parse),
            moon_events_available,
        });
    }
    points
}

/// The series for every metric the response carried, each with whether it
/// needs converting out of feet.
fn metric_arrays<'a, M: WeatherMetric>(
    block: &'a Value,
    units: Option<&Value>,
) -> Vec<(M, &'a [Value], bool)> {
    M::ALL
        .iter()
        .filter_map(|&metric| {
            let values = block.get(metric.raw_value()).and_then(Value::as_array)?;
            let in_feet = metric.is_length() && unit_is_feet(units, metric.raw_value());
            Some((metric, values.as_slice(), in_feet))
        })
        .collect()
}

/// One forecast point out of those series.
fn metric_values<M: WeatherMetric>(arrays: &[(M, &[Value], bool)], index: usize) -> MetricValues<M> {
    let mut values = Vec::new();
    for &(metric, series, in_feet) in arrays {
        let Some(value) = number(series, index) else {
            continue;
        };
        values.push((metric, if in_feet { value * FEET_TO_METERS } else { value }));
    }
    MetricValues { values }
}

/// Whether the response says this variable arrived in feet.
///
/// Open-Meteo reports lengths in feet when it is asked for imperial units, and
/// the panel works in meters, so the units block is read rather than assumed.
fn unit_is_feet(units: Option<&Value>, key: &str) -> bool {
    units.and_then(|units| units.get(key)).and_then(Value::as_str) == Some("ft")
}

fn array<'a>(block: &'a Value, key: &str) -> &'a [Value] {
    block.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// A finite number at an index, or None for a short array, a null or a NaN.
fn number(values: &[Value], index: usize) -> Option<f64> {
    values.get(index).and_then(Value::as_f64).filter(|value| value.is_finite())
}

fn text(values: &[Value], index: usize) -> Option<&str> {
    values.get(index).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-written response with the awkward cases in it: a null in the
    /// middle of a series, a series shorter than the day asked about, lengths
    /// in feet, one day with a provider moon phase and one without, and an
    /// hour belonging to the day after.
    const FORECAST: &str = r#"{
      "latitude": 40.71, "longitude": -74.01,
      "timezone": "America/New_York", "utc_offset_seconds": -14400,
      "hourly_units": {
        "temperature_2m": "°F", "visibility": "ft",
        "freezing_level_height": "ft", "snow_depth": "m"
      },
      "hourly": {
        "time": ["2026-11-01T00:00","2026-11-01T01:00","2026-11-01T02:00","2026-11-02T00:00"],
        "temperature_2m": [51.0, null, 49.2, 47.5],
        "weather_code": [3, 61, 61, 2],
        "wind_speed_10m": [7.2, 8.1, 6.4, 5.0],
        "is_day": [0, 0, 0, 0],
        "visibility": [78740.0, 78740.0, 39370.0, 78740.0],
        "freezing_level_height": [11000.0],
        "snow_depth": [0.0, 0.0, 0.0, 0.0],
        "vapour_pressure_deficit": [0.12, 0.1, 0.08, 0.07],
        "cloud_cover_low": [80, 90, 95, 40]
      },
      "daily_units": {"visibility_mean": "ft"},
      "daily": {
        "time": ["2026-11-01","2026-11-02"],
        "weather_code": [61, 2],
        "temperature_2m_max": [60.1, 58.0],
        "temperature_2m_min": [44.0, null],
        "sunrise": ["2026-11-01T07:26","2026-11-02T07:27"],
        "sunset": ["2026-11-01T17:53","2026-11-02T17:52"],
        "daylight_duration": [37620.0],
        "visibility_mean": [78740.0, 78740.0],
        "moon_phase": [0.5, null],
        "moonrise": ["2026-11-01T16:12", null],
        "moonset": [null, "2026-11-02T05:31"]
      }
    }"#;

    #[test]
    fn the_metric_catalog_uses_the_providers_own_parameter_names() {
        // These are wire format twice over: they go out in the query and come
        // back as the keys. The provider spells it "vapour", so we do too.
        assert_eq!(HourlyMetric::VaporPressureDeficit.raw_value(), "vapour_pressure_deficit");
        assert_eq!(DailyMetric::VaporPressureDeficitMax.raw_value(), "vapour_pressure_deficit_max");
        assert_eq!(HourlyMetric::SeaLevelPressure.raw_value(), "pressure_msl");
        assert_eq!(
            HourlyMetric::ReferenceEvapotranspiration.raw_value(),
            "et0_fao_evapotranspiration"
        );
        assert_eq!(HourlyMetric::SoilMoisture27To81cm.raw_value(), "soil_moisture_27_to_81cm");
        assert_eq!(DailyMetric::WindDirectionDominant.raw_value(), "wind_direction_10m_dominant");
        assert_eq!(HourlyMetric::ALL.len(), 31);
        assert_eq!(DailyMetric::ALL.len(), 25);
    }

    #[test]
    fn no_two_metrics_ask_for_the_same_parameter() {
        // A duplicate would silently shadow a metric in the response.
        for (position, metric) in HourlyMetric::ALL.iter().enumerate() {
            let twin = HourlyMetric::ALL.iter().position(|m| m.raw_value() == metric.raw_value());
            assert_eq!(twin, Some(position), "{} is listed twice", metric.raw_value());
        }
        for (position, metric) in DailyMetric::ALL.iter().enumerate() {
            let twin = DailyMetric::ALL.iter().position(|m| m.raw_value() == metric.raw_value());
            assert_eq!(twin, Some(position), "{} is listed twice", metric.raw_value());
        }
    }

    #[test]
    fn the_length_metrics_are_the_ones_the_provider_can_report_in_feet() {
        let hourly: Vec<&str> =
            HourlyMetric::ALL.iter().filter(|m| m.is_length()).map(|m| m.raw_value()).collect();
        assert_eq!(hourly, ["snow_depth", "freezing_level_height"]);
        let daily: Vec<&str> =
            DailyMetric::ALL.iter().filter(|m| m.is_length()).map(|m| m.raw_value()).collect();
        assert_eq!(daily, ["visibility_mean", "visibility_min"]);
    }

    #[test]
    fn the_request_asks_for_the_base_fields_the_catalog_and_the_lunar_events() {
        let hourly = hourly_fields();
        assert!(hourly.starts_with("temperature_2m,apparent_temperature,"));
        assert!(hourly.contains(",vapour_pressure_deficit,"));
        assert!(hourly.ends_with(",soil_moisture_27_to_81cm"));
        let daily = daily_fields();
        assert!(daily.starts_with("weather_code,temperature_2m_max,"));
        assert!(daily.contains(",moon_phase,"));
        // The lunar events go last, and they are not metrics: they are times.
        assert!(daily.ends_with(",moonrise,moonset"));
    }

    #[test]
    fn a_recorded_forecast_is_read_field_for_field() {
        let forecast = parse_detail(FORECAST).unwrap();
        assert_eq!(forecast.time_zone, "America/New_York");
        assert_eq!(forecast.utc_offset_seconds, -14_400);
        assert_eq!(forecast.hourly.len(), 4);
        assert_eq!(forecast.daily.len(), 2);

        let first = &forecast.hourly[0];
        assert_eq!(first.time, LocalTime::parse("2026-11-01T00:00").unwrap());
        assert_eq!(first.temperature, Some(51.0));
        assert_eq!(first.weather_code, Some(3));
        assert_eq!(first.wind_speed, Some(7.2));
        assert_eq!(first.is_day, Some(false));
        assert_eq!(first.details.get(HourlyMetric::VaporPressureDeficit), Some(0.12));
        assert_eq!(first.details.get(HourlyMetric::CloudCoverLow), Some(80.0));

        let day = &forecast.daily[0];
        assert_eq!(day.date, LocalDate { year: 2026, month: 11, day: 1 });
        assert_eq!(day.high, Some(60.1));
        assert_eq!(day.low, Some(44.0));
        assert_eq!(day.sunset, LocalTime::parse("2026-11-01T17:53"));
        assert_eq!(day.moonrise, LocalTime::parse("2026-11-01T16:12"));
        assert_eq!(day.moonset, None);
        assert!(day.moon_events_available);
    }

    #[test]
    fn a_null_inside_a_values_array_costs_only_that_hour() {
        let forecast = parse_detail(FORECAST).unwrap();
        assert_eq!(forecast.hourly[1].temperature, None);
        // The hours either side are untouched, and so is the rest of the hour
        // the null is in.
        assert_eq!(forecast.hourly[0].temperature, Some(51.0));
        assert_eq!(forecast.hourly[2].temperature, Some(49.2));
        assert_eq!(forecast.hourly[1].weather_code, Some(61));
        // A null in a daily series behaves the same way.
        assert_eq!(forecast.daily[1].low, None);
        assert_eq!(forecast.daily[1].high, Some(58.0));
    }

    #[test]
    fn an_array_shorter_than_the_hour_asked_for_is_not_an_error() {
        let forecast = parse_detail(FORECAST).unwrap();
        // freezing_level_height carries one value for four hours.
        assert!(forecast.hourly[0].details.get(HourlyMetric::FreezingLevelHeight).is_some());
        assert_eq!(forecast.hourly[1].details.get(HourlyMetric::FreezingLevelHeight), None);
        assert_eq!(forecast.hourly[3].details.get(HourlyMetric::FreezingLevelHeight), None);
        // The metrics beside it still fill every hour.
        assert_eq!(forecast.hourly[3].details.get(HourlyMetric::VaporPressureDeficit), Some(0.07));
    }

    #[test]
    fn a_length_reported_in_feet_is_normalized_to_meters() {
        let forecast = parse_detail(FORECAST).unwrap();
        let visibility = forecast.hourly[0].visibility.unwrap();
        assert!((visibility - 24_000.0).abs() < 1.0, "78740 ft is about 24 km, got {visibility}");
        let freezing = forecast.hourly[0].details.get(HourlyMetric::FreezingLevelHeight).unwrap();
        assert!((freezing - 3_352.8).abs() < 0.01, "11000 ft is 3352.8 m, got {freezing}");
        // snow_depth is a length too, but this response says it came in
        // meters, so it has to be left exactly as it arrived.
        assert_eq!(forecast.hourly[0].details.get(HourlyMetric::SnowDepth), Some(0.0));
        // Daily lengths are normalized from their own units block.
        let mean = forecast.daily[0].details.get(DailyMetric::VisibilityMean).unwrap();
        assert!((mean - 24_000.0).abs() < 1.0, "expected meters, got {mean}");
    }

    #[test]
    fn a_field_the_response_omits_costs_only_that_field() {
        let forecast = parse_detail(FORECAST).unwrap();
        // Nothing in the recorded body carries a dew point or a soil
        // temperature, and everything beside them still arrived.
        assert_eq!(forecast.hourly[0].dew_point, None);
        assert_eq!(forecast.hourly[0].details.get(HourlyMetric::SoilTemperature6cm), None);
        assert_eq!(forecast.hourly[0].temperature, Some(51.0));
    }

    #[test]
    fn a_response_with_no_forecast_blocks_is_an_error_not_an_empty_forecast() {
        assert!(parse_detail(r#"{"timezone":"America/New_York"}"#).is_err());
        assert!(parse_detail("not json at all").is_err());
        // One block missing is not fatal: what did arrive is still usable.
        let hourly_only = parse_detail(r#"{"hourly":{"time":["2026-11-01T00:00"]}}"#).unwrap();
        assert_eq!(hourly_only.hourly.len(), 1);
        assert!(hourly_only.daily.is_empty());
    }

    #[test]
    fn an_unparseable_time_drops_that_hour_and_no_other() {
        let body = r#"{"hourly":{"time":["2026-11-01T00:00","not a time","2026-11-01T02:00"],
          "temperature_2m":[1.0,2.0,3.0]}}"#;
        let forecast = parse_detail(body).unwrap();
        assert_eq!(forecast.hourly.len(), 2);
        // The surviving hours keep their own values: the index into every
        // other series is the position in the response, not in the result.
        assert_eq!(forecast.hourly[0].temperature, Some(1.0));
        assert_eq!(forecast.hourly[1].temperature, Some(3.0));
    }

    #[test]
    fn an_hour_outside_the_locations_own_day_is_not_counted_in_it() {
        let forecast = parse_detail(FORECAST).unwrap();
        let first = forecast.day(0).unwrap();
        assert_eq!(first.hourly.len(), 3);
        assert!(first.hourly.iter().all(|hour| hour.time.date == first.day.date));
        // The midnight hour belongs to the next day, in that day's calendar.
        let second = forecast.day(1).unwrap();
        assert_eq!(second.hourly.len(), 1);
        assert_eq!(second.hourly[0].time.hour, 0);
        assert!(forecast.day(2).is_none());
    }

    #[test]
    fn the_hours_of_a_day_come_back_in_order_however_they_arrived() {
        let body = r#"{"hourly":{"time":["2026-11-01T05:00","2026-11-01T01:00",
          "2026-11-01T03:00"]},"daily":{"time":["2026-11-01"]}}"#;
        let day = parse_detail(body).unwrap().day(0).unwrap();
        let hours: Vec<u32> = day.hourly.iter().map(|hour| hour.time.hour).collect();
        assert_eq!(hours, [1, 3, 5]);
    }

    #[test]
    fn a_day_with_twenty_five_local_hours_keeps_all_of_them() {
        // The clocks go back in America/New_York on 2026-11-01, so 01:00 is
        // lived through twice and the local day is 25 hours long. A day
        // measured as "midnight plus 86400 seconds" would drop the last hour;
        // asking which wall clocks carry that date cannot.
        let body = r#"{"timezone":"America/New_York","utc_offset_seconds":-18000,
          "hourly":{"time":[
            "2026-11-01T00:00","2026-11-01T01:00","2026-11-01T01:00","2026-11-01T02:00",
            "2026-11-01T03:00","2026-11-01T04:00","2026-11-01T05:00","2026-11-01T06:00",
            "2026-11-01T07:00","2026-11-01T08:00","2026-11-01T09:00","2026-11-01T10:00",
            "2026-11-01T11:00","2026-11-01T12:00","2026-11-01T13:00","2026-11-01T14:00",
            "2026-11-01T15:00","2026-11-01T16:00","2026-11-01T17:00","2026-11-01T18:00",
            "2026-11-01T19:00","2026-11-01T20:00","2026-11-01T21:00","2026-11-01T22:00",
            "2026-11-01T23:00","2026-11-02T00:00"]},
          "daily":{"time":["2026-11-01","2026-11-02"]}}"#;
        let forecast = parse_detail(body).unwrap();
        assert_eq!(forecast.day(0).unwrap().hourly.len(), 25);
        assert_eq!(forecast.day(1).unwrap().hourly.len(), 1);
    }

    #[test]
    fn a_spring_forward_day_is_an_hour_short_and_still_whole() {
        // The mirror image: 02:00 never happens on 2026-03-08, and a day
        // grouped this way is simply shorter rather than wrong.
        let body = r#"{"timezone":"America/New_York","utc_offset_seconds":-14400,
          "hourly":{"time":[
            "2026-03-08T00:00","2026-03-08T01:00","2026-03-08T03:00","2026-03-08T04:00",
            "2026-03-09T00:00"]},
          "daily":{"time":["2026-03-08"]}}"#;
        let day = parse_detail(body).unwrap().day(0).unwrap();
        assert_eq!(day.hourly.len(), 4);
        assert!(day.hourly.iter().all(|hour| hour.time.hour != 2));
    }

    #[test]
    fn the_provider_moon_phase_is_used_when_it_sends_one() {
        let day = parse_detail(FORECAST).unwrap().day(0).unwrap();
        assert!(!day.uses_estimated_moon_phase);
        assert_eq!(day.moon_phase, MoonPhase::FullMoon);
        assert!((day.moon_illumination - 1.0).abs() < 1e-9);
    }

    #[test]
    fn missing_lunar_data_falls_back_to_the_estimated_cycle() {
        let forecast = parse_detail(FORECAST).unwrap();
        // The second day's moon_phase is null, so the mean cycle answers and
        // the flag says so - the panel tells the user it is estimating.
        let day = forecast.day(1).unwrap();
        assert!(day.uses_estimated_moon_phase);
        assert!((0.0..=1.0).contains(&day.moon_illumination));
        // Estimated from local noon on that day, which is what fixes the
        // phase to the day being looked at rather than to our own clock.
        let noon = LocalTime { date: day.day.date, hour: 12, minute: 0 };
        let instant = noon.unix_seconds(forecast.utc_offset_seconds) as f64;
        assert_eq!(day.moon_phase, MoonPhase::at(instant));
        assert!((day.moon_illumination - MoonPhase::illumination_at(instant)).abs() < 1e-12);
    }

    #[test]
    fn a_moon_phase_outside_the_cycle_is_not_believed() {
        // A provider value has to be a fraction of a cycle. 47 is not, and an
        // estimate beats a phase indexed off the end of the cycle.
        let body = r#"{"daily":{"time":["2026-11-01"],"moon_phase":[47.0]}}"#;
        assert!(parse_detail(body).unwrap().day(0).unwrap().uses_estimated_moon_phase);
    }

    #[test]
    fn moon_events_are_unavailable_when_the_response_never_carried_them() {
        let body = r#"{"daily":{"time":["2026-11-01"]}}"#;
        let day = &parse_detail(body).unwrap().daily[0];
        assert!(!day.moon_events_available);
        assert_eq!(day.moonrise, None);
        // With the arrays present but null for that day, the events are
        // available and there simply was no moonset - a different statement.
        let forecast = parse_detail(FORECAST).unwrap();
        assert!(forecast.daily[0].moon_events_available);
        assert_eq!(forecast.daily[0].moonset, None);
    }

    #[test]
    fn the_reference_new_moon_reads_as_a_new_moon() {
        assert_eq!(MoonPhase::at(REFERENCE_NEW_MOON), MoonPhase::NewMoon);
        assert!(MoonPhase::illumination_at(REFERENCE_NEW_MOON) < 1e-9);
        let full = REFERENCE_NEW_MOON + SYNODIC_MONTH_SECONDS / 2.0;
        assert_eq!(MoonPhase::at(full), MoonPhase::FullMoon);
        assert!((MoonPhase::illumination_at(full) - 1.0).abs() < 1e-9);
        let quarter = REFERENCE_NEW_MOON + SYNODIC_MONTH_SECONDS / 4.0;
        assert_eq!(MoonPhase::at(quarter), MoonPhase::FirstQuarter);
    }

    #[test]
    fn a_date_before_the_reference_new_moon_still_lands_in_the_cycle() {
        // The day before the reference is nearly a whole cycle along, not a
        // negative fraction of one, which is what the wrap is there for.
        let day_before = REFERENCE_NEW_MOON - 86_400.0;
        assert!((0.0..1.0).contains(&MoonPhase::cycle_fraction(day_before)));
        assert_eq!(MoonPhase::at(day_before), MoonPhase::NewMoon);
        let three_weeks_before = REFERENCE_NEW_MOON - 21.0 * 86_400.0;
        assert_eq!(MoonPhase::at(three_weeks_before), MoonPhase::FirstQuarter);
    }

    #[test]
    fn a_polar_day_without_a_daylight_duration_is_unknown_not_zero() {
        // Tromso in June: the sun does not set, so the provider sends null
        // for both events. Zero hours of daylight would be exactly wrong.
        let body = r#"{"timezone":"Europe/Oslo","utc_offset_seconds":7200,
          "daily":{"time":["2026-06-21"],"sunrise":[null],"sunset":[null]}}"#;
        assert_eq!(parse_detail(body).unwrap().day(0).unwrap().daylight_duration(), None);
    }

    #[test]
    fn an_out_of_range_daylight_duration_is_not_believed_either() {
        let body = r#"{"daily":{"time":["2026-06-21"],"daylight_duration":[90000.0],
          "sunrise":["2026-06-21T04:25"],"sunset":["2026-06-21T20:31"]}}"#;
        let day = parse_detail(body).unwrap().day(0).unwrap();
        // More than a day of daylight inside a day is not a number to show,
        // so the sunrise and sunset it did send answer instead.
        assert_eq!(day.daylight_duration(), Some(57_960.0));
    }

    #[test]
    fn daylight_falls_back_to_sunset_minus_sunrise() {
        let forecast = parse_detail(FORECAST).unwrap();
        // The first day has the provider's own figure.
        assert_eq!(forecast.day(0).unwrap().daylight_duration(), Some(37_620.0));
        // The second is past the end of that series, so the two events answer:
        // 07:27 to 17:52 is ten hours and twenty-five minutes.
        assert_eq!(forecast.day(1).unwrap().daylight_duration(), Some(37_500.0));
    }

    #[test]
    fn a_sunset_before_its_sunrise_is_refused_rather_than_shown_as_negative() {
        let body = r#"{"daily":{"time":["2026-11-01"],
          "sunrise":["2026-11-01T17:53"],"sunset":["2026-11-01T07:26"]}}"#;
        assert_eq!(parse_detail(body).unwrap().day(0).unwrap().daylight_duration(), None);
    }

    #[test]
    fn local_times_parse_as_written_and_sort_chronologically() {
        let time = LocalTime::parse("2026-09-07T14:30").unwrap();
        assert_eq!(time.date, LocalDate { year: 2026, month: 9, day: 7 });
        assert_eq!((time.hour, time.minute), (14, 30));
        // Seconds are ignored rather than refused, in case they ever appear.
        assert_eq!(LocalTime::parse("2026-09-07T14:30:00"), Some(time));
        assert!(LocalTime::parse("2026-09-07T09:00").unwrap() < time);
        assert!(LocalTime::parse("2026-09-06T23:00").unwrap() < time);
        assert!(LocalTime::parse("2026-10-01T00:00").unwrap() > time);
        assert_eq!(LocalTime::parse("2026-09-07"), None);
        assert_eq!(LocalTime::parse("2026-09-07T25:00"), None);
        assert_eq!(LocalDate::parse("2026-13-01"), None);
        assert_eq!(LocalDate::parse("nonsense"), None);
    }

    #[test]
    fn the_weekday_of_a_local_date_is_worked_out_without_a_calendar() {
        // A ten-day list is labeled with these and there is no other source
        // of them in this port.
        assert_eq!(LocalDate { year: 1970, month: 1, day: 1 }.weekday(), 4);
        assert_eq!(LocalDate { year: 2026, month: 9, day: 7 }.weekday(), 1);
        assert_eq!(LocalDate { year: 2026, month: 11, day: 1 }.weekday(), 0);
        // A leap day, and a date before the epoch, which is where this
        // arithmetic is easiest to get wrong.
        assert_eq!(LocalDate { year: 2024, month: 2, day: 29 }.weekday(), 4);
        assert_eq!(LocalDate { year: 1969, month: 7, day: 20 }.weekday(), 0);
    }

    #[test]
    fn a_wall_clock_becomes_an_instant_only_with_the_zones_offset() {
        let noon = LocalTime::parse("2026-11-02T12:00").unwrap();
        // Noon in New York is four hours later than noon in London.
        assert_eq!(noon.unix_seconds(-14_400) - noon.unix_seconds(0), 14_400);
        assert_eq!(noon.unix_seconds(0), noon.wall_clock_seconds());
    }
}
