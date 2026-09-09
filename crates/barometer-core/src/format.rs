// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Number formatting for the strip.
//
// The strip's rule is that an item keeps a constant width while its numbers
// change, so these formatters trade precision for a stable column count: three
// significant figures, never four, and the unit carries the magnitude.

/// Binary units, which is what every tool in this category means by KB/s even
/// where it says so incorrectly. Staying consistent with the neighbors beats
/// being right alone.
const UNITS_PER_SEC: [&str; 5] = ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"];
const UNITS_BYTES: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// Bits per second, which is a different convention rather than a factor of 8.
///
/// Decimal, where the byte units above are binary, and that is not an
/// oversight. A link sold as 100 Mbps carries 100,000,000 bits per second -
/// network hardware, ISPs and speed tests all count it that way - while KB/s
/// everywhere else on Windows means 1024 bytes. Each is right inside its own
/// convention, so each keeps its own base rather than one being bent to match
/// the other.
///
/// Written `Mb/s` rather than `Mbps` to hold the same width as `MB/s`. The
/// strip reserves each column from the widest string its reading can produce,
/// so a unit that changed width when the user changed the setting would shove
/// everything to its right.
const UNITS_BITS_PER_SEC: [&str; 5] = ["b/s", "Kb/s", "Mb/s", "Gb/s", "Tb/s"];

/// The smallest unit a throughput is ever shown in: kilo, never plain bytes.
///
/// A number that sits at `0.00 B/s` and jumps to `847 B/s` when a background
/// sync twitches is noise on a taskbar. The magnitude anybody watches starts at
/// kilobytes, and holding the floor there also stops the unit itself flickering
/// between two widths while the machine is idle.
const SMALLEST_SCALE: usize = 1;

/// Whether throughput is counted in bytes or in bits.
///
/// Both are what somebody means by "how fast is my network": bytes if they are
/// watching a download finish, bits if they are checking it against what they
/// pay for. Neither converts into the other in the reader's head, so it is a
/// setting.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum RateUnit {
    /// KB/s, as Task Manager and every file transfer shows it.
    #[default]
    Bytes,
    /// Kb/s, as the link speed is quoted.
    Bits,
}

impl RateUnit {
    /// The persisted spelling. **Must never change.**
    pub fn raw_value(self) -> &'static str {
        match self {
            RateUnit::Bytes => "bytes",
            RateUnit::Bits => "bits",
        }
    }

    pub fn from_raw(raw: &str) -> Option<RateUnit> {
        match raw {
            "bytes" => Some(RateUnit::Bytes),
            "bits" => Some(RateUnit::Bits),
            _ => None,
        }
    }

    /// The units and the base this convention counts in.
    fn ladder(self) -> (&'static [&'static str; 5], f64) {
        match self {
            RateUnit::Bytes => (&UNITS_PER_SEC, 1024.0),
            RateUnit::Bits => (&UNITS_BITS_PER_SEC, 1000.0),
        }
    }

    /// The widest string `rate_in` can produce, for reserving a column.
    pub fn widest(self) -> &'static str {
        // Four digits, not three. The units are binary and the number is
        // written in decimal, so a rate sits in megabytes right up to 1024 of
        // them and `three_figures` prints "1024" - one character more than
        // the "999" this used to claim, which is exactly the sideways shove
        // of every column to its right that reserving a width exists to
        // prevent. Four digits are also at least as wide as the "9.99" of a
        // low rate in the same unit, a digit being no narrower than a point.
        match self {
            RateUnit::Bytes => "1024 MB/s",
            RateUnit::Bits => "1024 Mb/s",
        }
    }
}

fn scale(value: f64, units: &'static [&'static str; 5]) -> (f64, &'static str) {
    scale_from(value, units, 1024.0, 0)
}

/// Scales into `units`, never below `floor`.
fn scale_from(
    value: f64,
    units: &'static [&'static str; 5],
    base: f64,
    floor: usize,
) -> (f64, &'static str) {
    let mut v = value;
    let mut i = 0;
    while v >= base && i + 1 < units.len() {
        v /= base;
        i += 1;
    }
    // Below the floor the value is divided down anyway, so a rate under a
    // kilobyte reads as `0.43 KB/s` rather than being promoted to `440 B/s`.
    while i < floor && i + 1 < units.len() {
        v /= base;
        i += 1;
    }
    // The array is 'static; indexing it yields a &'static str.
    (v, units[i])
}

/// A whole number, with the "-0" a small negative rounds to written as "0".
///
/// Swift's %.0f prints "-0" for -0.4 and so does Rust's {:.0}. A reading of
/// "-0" looks like a fault rather than like freezing point, so this is the
/// one place the port departs from the Swift's output - in the direction of
/// what the Swift meant. It lives here rather than in the flyout because the
/// taskbar rounds the same numbers: a freezing night showed "-0" on the strip
/// beside "0" in the panel it opened.
pub fn whole(value: f64) -> String {
    let text = format!("{value:.0}");
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

/// Three significant figures, so the text never grows past five characters
/// before the unit: `9.99`, `99.9`, `999`.
fn three_figures(v: f64) -> String {
    if v < 10.0 {
        format!("{v:.2}")
    } else if v < 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    }
}

/// A throughput in the user's chosen convention, as `1.21 MB/s` or `9.68 Mb/s`.
///
/// Always kilo or above - see `SMALLEST_SCALE`.
pub fn rate_in(bytes_per_sec: f64, unit: RateUnit) -> String {
    let v = if bytes_per_sec.is_finite() && bytes_per_sec > 0.0 { bytes_per_sec } else { 0.0 };
    // Eight bits to the byte, applied before scaling: the counters underneath
    // are bytes whichever way it is shown.
    let v = match unit {
        RateUnit::Bytes => v,
        RateUnit::Bits => v * 8.0,
    };
    let (units, base) = unit.ladder();
    let (scaled, suffix) = scale_from(v, units, base, SMALLEST_SCALE);
    format!("{} {}", three_figures(scaled), suffix)
}

/// A throughput in bytes, as `1.21 MB/s`.
pub fn rate(bytes_per_sec: f64) -> String {
    rate_in(bytes_per_sec, RateUnit::Bytes)
}

/// A quantity of bytes, as `12.4 GB`.
pub fn bytes(count: u64) -> String {
    let (scaled, unit) = scale(count as f64, &UNITS_BYTES);
    format!("{} {}", three_figures(scaled), unit)
}

/// A percentage as whole digits, `0%` through `100%`.
///
/// Fractions are pointless at this size and they make the column jitter, which
/// is the one thing the strip is not allowed to do.
pub fn percent(fraction: f32) -> String {
    let pct = (fraction.clamp(0.0, 1.0) * 100.0).round() as u32;
    format!("{pct}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freezing_reading_rounds_to_zero_and_never_to_minus_zero() {
        assert_eq!(whole(-0.4), "0");
        assert_eq!(whole(-0.0), "0");
        assert_eq!(whole(0.4), "0");
        // Anything that really is below zero still says so.
        assert_eq!(whole(-0.6), "-1");
        assert_eq!(whole(-12.0), "-12");
        assert_eq!(whole(21.6), "22");
    }

    #[test]
    fn no_rate_is_wider_than_the_width_reserved_for_one() {
        // The units are binary and the number decimal, so a rate stays in
        // megabytes to 1024 of them: "1023 MB/s" and "9.99 MB/s" are both a
        // character longer than the "999 MB/s" the reservation used to be
        // taken from, and both shoved every column to their right sideways.
        let mega = 1024.0 * 1024.0;
        for unit in [RateUnit::Bytes, RateUnit::Bits] {
            for bytes_per_sec in [0.0, 1.0, 1500.0, 9.99 * mega, 999.0 * mega, 1023.9 * mega, 1e12] {
                let text = rate_in(bytes_per_sec, unit);
                assert!(text.len() <= unit.widest().len(), "{text} against {}", unit.widest());
            }
        }
        // And the numbers themselves are unchanged.
        assert_eq!(rate(1023.0 * mega), "1023 MB/s");
        assert_eq!(rate(999.0 * mega), "999 MB/s");
    }

    #[test]
    fn rates_stay_narrow() {
        assert_eq!(rate(1024.0), "1.00 KB/s");
        assert_eq!(rate(1024.0 * 1536.0), "1.50 MB/s");
    }

    #[test]
    fn a_negative_or_nan_rate_reads_as_zero() {
        // Counter wraps and adapter resets can both produce a negative delta.
        assert_eq!(rate(-5.0), "0.00 KB/s");
        assert_eq!(rate(f64::NAN), "0.00 KB/s");
    }

    #[test]
    fn percentages_clamp() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.5), "50%");
        assert_eq!(percent(1.5), "100%");
    }

    #[test]
    fn a_rate_never_drops_below_kilobytes() {
        // Bytes per second is too small a unit for a taskbar: it reads as
        // noise, and it makes the unit itself flicker between two widths while
        // the machine is idle.
        assert_eq!(rate(0.0), "0.00 KB/s");
        assert_eq!(rate(999.0), "0.98 KB/s");
        assert_eq!(rate_in(0.0, RateUnit::Bits), "0.00 Kb/s");
        assert_eq!(rate_in(50.0, RateUnit::Bits), "0.40 Kb/s");
    }

    #[test]
    fn bits_are_eight_times_bytes_and_counted_in_thousands() {
        // A mebibyte a second is 8.39 megabits a second, not 8.00: decimal for
        // bits, binary for bytes, each convention keeping its own base. This is
        // deliberately not the byte figure times eight.
        assert_eq!(rate(1024.0 * 1024.0), "1.00 MB/s");
        assert_eq!(rate_in(1024.0 * 1024.0, RateUnit::Bits), "8.39 Mb/s");
    }

    #[test]
    fn both_conventions_reserve_the_same_width() {
        // The strip sizes each column from the widest string its reading can
        // produce, so a unit that changed width when the setting changed would
        // shove every column to its right.
        assert_eq!(RateUnit::Bytes.widest().len(), RateUnit::Bits.widest().len());
    }

    #[test]
    fn rate_units_round_trip() {
        for unit in [RateUnit::Bytes, RateUnit::Bits] {
            assert_eq!(RateUnit::from_raw(unit.raw_value()), Some(unit));
        }
        assert_eq!(RateUnit::from_raw("kilobits"), None);
    }

}
