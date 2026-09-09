// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The Open-Meteo client, ported from Sources/MenuBarStatsCore/Weather/.
//
// Open-Meteo needs no account and no key, which is why both apps use it: a
// weather readout that stops working when a free tier is withdrawn is worse
// than no weather readout. Three endpoints are used and they are separate
// hosts, so each one is named where it is called rather than shared.

use serde_json::Value;

use crate::net::{self, HttpError};
use crate::weather::models::{Location, WeatherUnits};

const FORECAST_HOST: &str = "api.open-meteo.com";
const GEOCODING_HOST: &str = "geocoding-api.open-meteo.com";

/// The fields asked for, in the order they are read back.
///
/// Kept as one constant because the request and the parser have to agree, and
/// the way they stop agreeing is somebody adding a field to one of them.
const CURRENT_FIELDS: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,\
is_day,precipitation,weather_code,wind_speed_10m,wind_direction_10m,pressure_msl,\
surface_pressure";

#[derive(Debug)]
pub enum WeatherError {
    /// The request did not get through.
    Transport(String),
    /// The service answered with something unusable.
    Malformed(String),
    /// There is nowhere to ask about.
    NoLocation,
}

impl std::fmt::Display for WeatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeatherError::Transport(why) => write!(f, "{why}"),
            WeatherError::Malformed(why) => write!(f, "unexpected response: {why}"),
            WeatherError::NoLocation => f.write_str("no location set"),
        }
    }
}

impl From<HttpError> for WeatherError {
    fn from(error: HttpError) -> Self {
        WeatherError::Transport(error.to_string())
    }
}

/// One observation, in whatever units were asked for.
///
/// Every field is optional because Open-Meteo omits rather than nulls what a
/// station does not report, and a missing humidity must not cost us the
/// temperature beside it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurrentWeather {
    pub temperature: Option<f64>,
    pub apparent_temperature: Option<f64>,
    pub relative_humidity: Option<f64>,
    pub precipitation: Option<f64>,
    pub wind_speed: Option<f64>,
    pub wind_direction: Option<f64>,
    /// Pressure at sea level, always hectopascals - the only unit Open-Meteo
    /// will not convert, so it is converted here on the way to the display.
    ///
    /// Sea level rather than at the station, because that is what a barometer
    /// reading means and what everything downstream assumes: the panel's
    /// gauge is scaled 960 to 1060, and "1013 is fair weather" is a statement
    /// about sea-level pressure. Reading `surface_pressure` into this field
    /// put Denver at about 830 and pinned the gauge empty, and the hourly
    /// figure the panel falls back to really is `pressure_msl`, so the tile
    /// silently swapped between two quantities 180 hPa apart. The Mac asks
    /// for both, and so does this now.
    pub pressure_hectopascals: Option<f64>,
    /// Pressure where the station actually stands, which is the same reading
    /// uncorrected for altitude. Kept because Open-Meteo returns it and the
    /// Mac keeps it too; nothing on the strip uses it yet.
    pub surface_pressure_hectopascals: Option<f64>,
    /// WMO code, which the badge renderer turns into a mark.
    pub weather_code: Option<u8>,
    /// Whether it is daytime there, which decides sun against moon.
    pub is_day: bool,
}

/// Reads the current conditions for a location.
pub fn fetch(location: &Location, units: WeatherUnits) -> Result<CurrentWeather, WeatherError> {
    let query = format!(
        "/v1/forecast?latitude={}&longitude={}&current={}\
&temperature_unit={}&wind_speed_unit={}&precipitation_unit={}&timezone=auto",
        net::encode(&location.latitude),
        net::encode(&location.longitude),
        CURRENT_FIELDS,
        units.temperature.raw_value(),
        units.wind_speed.raw_value(),
        units.precipitation.raw_value(),
    );
    let body = net::get(FORECAST_HOST, &query)?;
    parse_current(&body)
}

/// Pulls the observation out of a forecast response.
///
/// Separate from the request so it can be tested against a recorded body. This
/// is the part that breaks when a provider changes shape, and it is the part
/// that is impossible to exercise if it only exists inside a network call.
pub fn parse_current(body: &str) -> Result<CurrentWeather, WeatherError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| WeatherError::Malformed(e.to_string()))?;
    let current = root
        .get("current")
        .ok_or_else(|| WeatherError::Malformed("no current block".into()))?;

    let number = |key: &str| current.get(key).and_then(Value::as_f64);
    Ok(CurrentWeather {
        temperature: number("temperature_2m"),
        apparent_temperature: number("apparent_temperature"),
        relative_humidity: number("relative_humidity_2m"),
        precipitation: number("precipitation"),
        wind_speed: number("wind_speed_10m"),
        wind_direction: number("wind_direction_10m"),
        pressure_hectopascals: number("pressure_msl"),
        surface_pressure_hectopascals: number("surface_pressure"),
        weather_code: number("weather_code").map(|c| c as u8),
        // Absent means day. A readout that defaults to night shows a moon over
        // a sunny afternoon, which reads as broken; the other way round it
        // reads as merely unlucky.
        is_day: number("is_day").map(|d| d != 0.0).unwrap_or(true),
    })
}

/// Finds places matching what the user typed.
pub fn search(query: &str) -> Result<Vec<Location>, WeatherError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let path = format!(
        "/v1/search?name={}&count=10&language=en&format=json",
        net::encode(trimmed)
    );
    let body = net::get(GEOCODING_HOST, &path)?;
    parse_search(&body)
}

/// Turns a geocoding response into locations.
pub fn parse_search(body: &str) -> Result<Vec<Location>, WeatherError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| WeatherError::Malformed(e.to_string()))?;
    // No matches is an absent "results" key, not an empty array, and that is
    // an ordinary answer rather than an error.
    let Some(results) = root.get("results").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut found = Vec::with_capacity(results.len());
    for entry in results {
        let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
        let (Some(name), Some(latitude), Some(longitude)) = (
            text("name"),
            entry.get("latitude").and_then(Value::as_f64),
            entry.get("longitude").and_then(Value::as_f64),
        ) else {
            // A result without a name or a position cannot be shown or asked
            // about, so it is skipped rather than failing the whole search.
            continue;
        };
        found.push(Location {
            // Open-Meteo's own id, so the same place saved twice is the same
            // place. Names repeat: there are more than thirty Springfields.
            id: entry
                .get("id")
                .and_then(Value::as_u64)
                .map(|id| id.to_string())
                .unwrap_or_else(|| format!("{latitude},{longitude}")),
            name,
            admin: text("admin1"),
            country: text("country").unwrap_or_default(),
            // Stored as text so the coordinates that go back to the service
            // are exactly the ones it gave us, with no round trip through a
            // float and back to a string that shortens them.
            latitude: latitude.to_string(),
            longitude: longitude.to_string(),
            time_zone: text("timezone").unwrap_or_default(),
        });
    }
    Ok(found)
}

/// Guesses where the user is, from their IP address.
///
/// Only ever a starting point: it is offered as the first entry when the
/// location list is empty, and the user can replace it. It is deliberately not
/// the Windows location API, which raises a consent prompt for something the
/// user did not ask for on first run.
///
/// Two services, because the first one has already been seen to start refusing
/// anonymous requests with a 403. The user agent is not optional for the same
/// reason.
pub fn locate_by_ip() -> Result<Location, WeatherError> {
    match net::get("ipapi.co", "/json/").ok().and_then(|body| parse_ipapi(&body).ok()) {
        Some(location) => Ok(location),
        None => {
            let body = net::get("ipwho.is", "/")?;
            parse_ipwho(&body)
        }
    }
}

/// ipapi.co's shape.
pub fn parse_ipapi(body: &str) -> Result<Location, WeatherError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| WeatherError::Malformed(e.to_string()))?;
    location_from(&root, "city", "region", "country_name", "latitude", "longitude", "timezone")
}

/// ipwho.is's shape, which differs only in what the fields are called.
pub fn parse_ipwho(body: &str) -> Result<Location, WeatherError> {
    let root: Value =
        serde_json::from_str(body).map_err(|e| WeatherError::Malformed(e.to_string()))?;
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(WeatherError::Malformed("lookup refused".into()));
    }
    let zone = root
        .get("timezone")
        .and_then(|z| z.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut location =
        location_from(&root, "city", "region", "country", "latitude", "longitude", "__none__")?;
    location.time_zone = zone;
    Ok(location)
}

fn location_from(
    root: &Value,
    city: &str,
    region: &str,
    country: &str,
    latitude: &str,
    longitude: &str,
    zone: &str,
) -> Result<Location, WeatherError> {
    let text = |key: &str| root.get(key).and_then(Value::as_str).map(str::to_string);
    // Coordinates arrive as a number from one service and as a string from the
    // other, so both are accepted rather than one of them being a parse error
    // that silently disables auto-location.
    let coordinate = |key: &str| {
        root.get(key).and_then(|v| {
            v.as_f64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string))
        })
    };
    let (Some(latitude), Some(longitude)) = (coordinate(latitude), coordinate(longitude)) else {
        return Err(WeatherError::Malformed("no coordinates".into()));
    };
    let name = text(city).unwrap_or_else(|| "Current location".to_string());
    Ok(Location {
        id: format!("ip:{latitude},{longitude}"),
        name,
        admin: text(region),
        country: text(country).unwrap_or_default(),
        latitude,
        longitude,
        time_zone: text(zone).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST: &str = r#"{
      "latitude": 30.25, "longitude": -97.75, "timezone": "America/Chicago",
      "current": {
        "time": "2026-09-07T20:00", "interval": 900,
        "temperature_2m": 91.4, "relative_humidity_2m": 44,
        "apparent_temperature": 97.2, "is_day": 1, "precipitation": 0.0,
        "weather_code": 2, "wind_speed_10m": 8.9, "wind_direction_10m": 160,
        "pressure_msl": 1013.4, "surface_pressure": 1004.7
      }
    }"#;

    #[test]
    fn a_forecast_response_is_read_field_for_field() {
        let now = parse_current(FORECAST).unwrap();
        assert_eq!(now.temperature, Some(91.4));
        assert_eq!(now.apparent_temperature, Some(97.2));
        assert_eq!(now.relative_humidity, Some(44.0));
        assert_eq!(now.weather_code, Some(2));
        assert_eq!(now.wind_speed, Some(8.9));
        // Sea level for the reading the app is named after; the station's
        // own figure kept beside it rather than standing in for it.
        assert_eq!(now.pressure_hectopascals, Some(1013.4));
        assert_eq!(now.surface_pressure_hectopascals, Some(1004.7));
        assert!(now.is_day);
    }

    #[test]
    fn a_missing_field_costs_only_that_field() {
        // A station that reports no humidity must not blank the temperature.
        let body = r#"{"current":{"temperature_2m":54.0,"weather_code":61,"is_day":0}}"#;
        let now = parse_current(body).unwrap();
        assert_eq!(now.temperature, Some(54.0));
        assert_eq!(now.relative_humidity, None);
        assert!(!now.is_day);
    }

    #[test]
    fn an_absent_is_day_reads_as_day() {
        // A moon drawn over a sunny afternoon reads as broken; a sun over a
        // clear night reads as merely unlucky.
        let now = parse_current(r#"{"current":{"temperature_2m":1.0}}"#).unwrap();
        assert!(now.is_day);
    }

    #[test]
    fn a_response_with_no_current_block_is_an_error_not_an_empty_reading() {
        assert!(parse_current(r#"{"latitude":30.0}"#).is_err());
        assert!(parse_current("not json at all").is_err());
    }

    const SEARCH: &str = r#"{"results":[
      {"id":4671654,"name":"Austin","latitude":30.26715,"longitude":-97.74306,
       "country":"United States","admin1":"Texas","timezone":"America/Chicago"},
      {"id":4671240,"name":"Austin","latitude":39.49172,"longitude":-85.80802,
       "country":"United States","admin1":"Indiana","timezone":"America/Indiana/Indianapolis"}
    ]}"#;

    #[test]
    fn search_results_keep_what_tells_two_places_of_the_same_name_apart() {
        let found = parse_search(SEARCH).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].display_name(), "Austin, Texas, United States");
        assert_eq!(found[1].display_name(), "Austin, Indiana, United States");
        // Ids differ, so saving both does not collapse them into one entry.
        assert_ne!(found[0].id, found[1].id);
    }

    #[test]
    fn coordinates_survive_as_the_service_gave_them() {
        let found = parse_search(SEARCH).unwrap();
        assert_eq!(found[0].latitude, "30.26715");
        assert_eq!(found[0].longitude, "-97.74306");
    }

    #[test]
    fn no_matches_is_an_empty_list_not_a_failure() {
        // Open-Meteo omits "results" entirely rather than sending an empty one.
        assert!(parse_search(r#"{"generationtime_ms":0.2}"#).unwrap().is_empty());
    }

    #[test]
    fn a_result_missing_a_position_is_skipped_rather_than_failing_the_search() {
        let body = r#"{"results":[{"name":"Nowhere"},
          {"id":1,"name":"Somewhere","latitude":1.0,"longitude":2.0,"country":"X"}]}"#;
        let found = parse_search(body).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Somewhere");
    }

    #[test]
    fn both_geolocation_services_are_understood() {
        let ipapi = r#"{"city":"Austin","region":"Texas","country_name":"United States",
          "latitude":30.2672,"longitude":-97.7431,"timezone":"America/Chicago"}"#;
        let one = parse_ipapi(ipapi).unwrap();
        assert_eq!(one.name, "Austin");
        assert_eq!(one.time_zone, "America/Chicago");

        // ipwho.is nests the zone and sends coordinates as numbers too.
        let ipwho = r#"{"success":true,"city":"Austin","region":"Texas","country":"United States",
          "latitude":30.2672,"longitude":-97.7431,"timezone":{"id":"America/Chicago"}}"#;
        let two = parse_ipwho(ipwho).unwrap();
        assert_eq!(two.name, "Austin");
        assert_eq!(two.time_zone, "America/Chicago");
        assert_eq!(one.latitude, two.latitude);
    }

    #[test]
    fn a_refused_geolocation_is_an_error_not_a_location_at_the_null_island() {
        assert!(parse_ipwho(r#"{"success":false,"message":"rate limited"}"#).is_err());
        assert!(parse_ipapi(r#"{"city":"Austin"}"#).is_err());
    }
}
