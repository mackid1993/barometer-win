// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Wall-clock time at the forecast's location, and the words for it.
//
// The Swift hands every date to Foundation with the forecast's TimeZone and
// lets it format. This port has no time zone database and, by the rule in
// detail.rs, wants none: the forecast's times are already wall clocks in the
// location's own zone, and the one thing the panel has to work out for itself
// is which of those wall clocks is *now*. The provider says how far its zone
// sits from UTC, and UTC is what the system clock gives, so "now at the
// location" is one addition. Everything the panel prints - which hours are
// still ahead, where the sun is on its arc, how long ago the reading arrived -
// follows from that.
//
// That also settles a question the Swift answers differently: it formats the
// "Updated" time in the machine's zone. Here it is the location's, because
// that is the zone every other time in the panel is in, and for the primary
// location - where the user almost always is - the two are the same clock.

use std::time::{SystemTime, UNIX_EPOCH};

use barometer_core::weather::detail::{LocalDate, LocalTime};

const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS_LONG: [&str; 7] =
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS_SHORT: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The calendar date `days` after 1970-01-01, negative before it.
///
/// Howard Hinnant's civil_from_days: the inverse of the arithmetic
/// LocalDate::days_from_epoch does, and exact for every proleptic Gregorian
/// date, so a round trip through the two is the identity.
pub fn date_from_days(days: i64) -> LocalDate {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    LocalDate { year: year as i32, month: month as u32, day: day as u32 }
}

/// The wall clock at a location whose zone sits `utc_offset_seconds` from UTC.
pub fn local_now(utc_offset_seconds: i64, unix_seconds: i64) -> LocalTime {
    let wall = unix_seconds + utc_offset_seconds;
    let seconds = wall.rem_euclid(86_400);
    LocalTime {
        date: date_from_days(wall.div_euclid(86_400)),
        hour: (seconds / 3_600) as u32,
        minute: (seconds % 3_600 / 60) as u32,
    }
}

/// Seconds since the epoch, from the system clock.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// The same wall clock at the top of its hour, which is the key the hourly
/// series is indexed by.
pub fn floor_hour(time: LocalTime) -> LocalTime {
    LocalTime { minute: 0, ..time }
}

/// "Mon", as the ten-day list labels its rows.
pub fn weekday_short(date: LocalDate) -> &'static str {
    WEEKDAYS_SHORT[date.weekday() as usize]
}

/// "Tuesday, Sep 8", as the day page is titled.
///
/// The Swift asks Foundation for the "EEEE MMM d" template, which in the
/// English locale comes back with the comma; that is the form reproduced.
pub fn date_title(date: LocalDate) -> String {
    format!(
        "{}, {} {}",
        WEEKDAYS_LONG[date.weekday() as usize],
        MONTHS_SHORT[(date.month.clamp(1, 12) - 1) as usize],
        date.day
    )
}

/// Hours as a clock face shows them, with the half of the day they fall in.
fn twelve_hour(hour: u32) -> (u32, &'static str) {
    let half = if hour < 12 { "am" } else { "pm" };
    let clock = hour % 12;
    (if clock == 0 { 12 } else { clock }, half)
}

/// "3pm", the compact label under the chart.
pub fn hour_label(time: LocalTime) -> String {
    let (clock, half) = twelve_hour(time.hour);
    format!("{clock}{half}")
}

/// "3 PM", the headline over one hour's details.
pub fn hour_headline(time: LocalTime) -> String {
    let (clock, half) = twelve_hour(time.hour);
    format!("{clock} {}", half.to_uppercase())
}

/// "3:42 PM", for sunrise, sunset, moonrise and the reading's own time.
pub fn time_short(time: LocalTime) -> String {
    let (clock, half) = twelve_hour(time.hour);
    format!("{clock}:{:02} {}", time.minute, half.to_uppercase())
}

/// A wall clock, or the dash for one the provider did not send.
pub fn time_or_dash(time: Option<LocalTime>) -> String {
    time.map(time_short).unwrap_or_else(|| super::format::DASH.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_through_days_since_the_epoch() {
        for (year, month, day) in [
            (1970, 1, 1),
            (1969, 12, 31),
            (2000, 2, 29),
            (2026, 9, 8),
            (2026, 12, 31),
            (2100, 3, 1),
            (1900, 1, 1),
        ] {
            let date = LocalDate { year, month, day };
            assert_eq!(date_from_days(date.days_from_epoch()), date, "{year}-{month}-{day}");
        }
        assert_eq!(date_from_days(0), LocalDate { year: 1970, month: 1, day: 1 });
        assert_eq!(date_from_days(-1), LocalDate { year: 1969, month: 12, day: 31 });
    }

    #[test]
    fn now_at_a_location_is_utc_plus_its_offset() {
        // 2026-09-08 01:30 UTC is still the evening of the 7th in Chicago.
        let unix = LocalTime::parse("2026-09-08T01:30").unwrap().wall_clock_seconds();
        let chicago = local_now(-5 * 3_600, unix);
        assert_eq!(chicago, LocalTime::parse("2026-09-07T20:30").unwrap());
        // And already the morning of the 8th in Tokyo.
        let tokyo = local_now(9 * 3_600, unix);
        assert_eq!(tokyo, LocalTime::parse("2026-09-08T10:30").unwrap());
        assert_eq!(local_now(0, unix), LocalTime::parse("2026-09-08T01:30").unwrap());
    }

    #[test]
    fn hours_read_as_a_clock_face_would_show_them() {
        let at = |hour| LocalTime { date: LocalDate { year: 2026, month: 9, day: 8 }, hour, minute: 5 };
        assert_eq!(hour_label(at(0)), "12am");
        assert_eq!(hour_label(at(11)), "11am");
        assert_eq!(hour_label(at(12)), "12pm");
        assert_eq!(hour_label(at(15)), "3pm");
        assert_eq!(hour_label(at(23)), "11pm");
        assert_eq!(hour_headline(at(15)), "3 PM");
        assert_eq!(hour_headline(at(0)), "12 AM");
        assert_eq!(time_short(at(15)), "3:05 PM");
        assert_eq!(time_short(at(0)), "12:05 AM");
        assert_eq!(time_or_dash(None), "\u{2014}");
    }

    #[test]
    fn the_day_title_and_row_label_name_the_weekday() {
        let tuesday = LocalDate { year: 2026, month: 9, day: 8 };
        assert_eq!(date_title(tuesday), "Tuesday, Sep 8");
        assert_eq!(weekday_short(tuesday), "Tue");
        let sunday = LocalDate { year: 2026, month: 11, day: 1 };
        assert_eq!(date_title(sunday), "Sunday, Nov 1");
    }

    #[test]
    fn flooring_keeps_the_hour_and_drops_the_minutes() {
        let time = LocalTime::parse("2026-09-08T14:45").unwrap();
        assert_eq!(floor_hour(time), LocalTime::parse("2026-09-08T14:00").unwrap());
    }
}
