// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The air at a location, from Open-Meteo's air-quality service: the US AQI
// and the pollutants behind it, as the macOS app's OpenMeteoClient.airQuality
// asks for them. A separate service from the forecast, with its own host,
// which is why it is a separate request.

use serde_json::Value;

use super::client::WeatherError;
use super::models::Location;
use crate::net;

const AIR_HOST: &str = "air-quality-api.open-meteo.com";

/// What the air is like right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AirQuality {
    /// The United States index, 0 upward; past 300 is hazardous.
    pub us_aqi: Option<i64>,
    /// Micrograms per cubic meter.
    pub pm2_5: Option<f64>,
    pub pm10: Option<f64>,
    pub ozone: Option<f64>,
}

impl AirQuality {
    /// Whether there is anything here to show.
    pub fn is_empty(&self) -> bool {
        self.us_aqi.is_none() && self.pm2_5.is_none() && self.pm10.is_none() && self.ozone.is_none()
    }
}

/// Reads the current air quality for a location.
pub fn fetch_air(location: &Location) -> Result<AirQuality, WeatherError> {
    let query = format!(
        "/v1/air-quality?latitude={}&longitude={}&timezone=auto&current=us_aqi,pm2_5,pm10,ozone",
        net::encode(&location.latitude),
        net::encode(&location.longitude),
    );
    let body = net::get(AIR_HOST, &query)?;
    parse_air(&body)
}

/// Pulls the reading out of an air-quality response.
///
/// Separate from the request so it can be tested against a recorded body,
/// as client.rs and detail.rs are.
pub fn parse_air(body: &str) -> Result<AirQuality, WeatherError> {
    let root: Value = serde_json::from_str(body).map_err(|e| WeatherError::Malformed(e.to_string()))?;
    let current = root.get("current").ok_or_else(|| WeatherError::Malformed("no current block".into()))?;
    let number = |key: &str| current.get(key).and_then(Value::as_f64);
    Ok(AirQuality {
        // The service sends the index as a number that may carry a
        // fraction; the index is whole by definition.
        us_aqi: number("us_aqi").map(|v| v.round() as i64),
        pm2_5: number("pm2_5"),
        pm10: number("pm10"),
        ozone: number("ozone"),
    })
}

/// The EPA's word for an index, as the Mac's WeatherValue.aqiDescription.
pub fn describe(us_aqi: i64) -> &'static str {
    match us_aqi {
        ..=50 => "Good",
        ..=100 => "Moderate",
        ..=150 => "Unhealthy for sensitive groups",
        ..=200 => "Unhealthy",
        ..=300 => "Very unhealthy",
        _ => "Hazardous",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_answer_reads_as_the_index_and_its_pollutants() {
        let body = r#"{"latitude":40.7,"longitude":-74.0,"timezone":"America/New_York",
            "current_units":{"us_aqi":"USAQI","pm2_5":"μg/m³"},
            "current":{"time":"2026-09-08T14:00","interval":3600,"us_aqi":42,"pm2_5":8.4,"pm10":15.1,"ozone":61.0}}"#;
        let air = parse_air(body).expect("parses");
        assert_eq!(air, AirQuality { us_aqi: Some(42), pm2_5: Some(8.4), pm10: Some(15.1), ozone: Some(61.0) });
        assert!(!air.is_empty());
    }

    #[test]
    fn a_reading_the_service_does_not_have_is_none_and_a_body_without_current_is_malformed() {
        let air = parse_air(r#"{"current":{"time":"2026-09-08T14:00","us_aqi":null}}"#).expect("parses");
        assert!(air.is_empty());
        assert!(matches!(parse_air(r#"{"latitude":1}"#), Err(WeatherError::Malformed(_))));
        assert!(matches!(parse_air("not json"), Err(WeatherError::Malformed(_))));
    }

    #[test]
    fn the_bands_break_where_the_epa_breaks_them() {
        assert_eq!(describe(0), "Good");
        assert_eq!(describe(50), "Good");
        assert_eq!(describe(51), "Moderate");
        assert_eq!(describe(150), "Unhealthy for sensitive groups");
        assert_eq!(describe(200), "Unhealthy");
        assert_eq!(describe(300), "Very unhealthy");
        assert_eq!(describe(301), "Hazardous");
    }
}
