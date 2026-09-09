// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Temperatures, fans and voltages.
//
// Windows exposes none of these. They come from a sensor source the user
// installs, which Barometer detects and reads but does not ship. See AGENTS.md
// for why, and for the credit that is owed.
//
// The provider is a trait so a second source can be added without the modules
// above caring which one answered.

pub mod health;
pub mod helper;
pub mod lhm;
pub mod library;

use std::fmt;

/// What a reading measures.
///
/// Mirrors LibreHardwareMonitor's SensorType. `Other` carries the original
/// spelling rather than discarding it, so a sensor kind added upstream still
/// shows up in the detail panel instead of vanishing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SensorKind {
    Temperature,
    Fan,
    Voltage,
    Current,
    Power,
    Clock,
    Load,
    Level,
    Factor,
    Data,
    SmallData,
    Throughput,
    Control,
    Energy,
    Humidity,
    Noise,
    Flow,
    Other(String),
}

impl SensorKind {
    /// The kind as a word, for a list where the same name means several
    /// things: "CPU Package" is a temperature, a power figure and a load.
    pub fn name(&self) -> &str {
        match self {
            SensorKind::Temperature => "Temperature",
            SensorKind::Fan => "Fan",
            SensorKind::Voltage => "Voltage",
            SensorKind::Current => "Current",
            SensorKind::Power => "Power",
            SensorKind::Clock => "Clock",
            SensorKind::Load => "Load",
            SensorKind::Level => "Level",
            SensorKind::Factor => "Factor",
            SensorKind::Data => "Data",
            SensorKind::SmallData => "Data",
            SensorKind::Throughput => "Throughput",
            SensorKind::Control => "Control",
            SensorKind::Energy => "Energy",
            SensorKind::Humidity => "Humidity",
            SensorKind::Noise => "Noise",
            SensorKind::Flow => "Flow",
            SensorKind::Other(raw) => raw,
        }
    }

    pub fn parse(raw: &str) -> SensorKind {
        match raw {
            "Temperature" => SensorKind::Temperature,
            "Fan" => SensorKind::Fan,
            "Voltage" => SensorKind::Voltage,
            "Current" => SensorKind::Current,
            "Power" => SensorKind::Power,
            "Clock" => SensorKind::Clock,
            "Load" => SensorKind::Load,
            "Level" => SensorKind::Level,
            "Factor" => SensorKind::Factor,
            "Data" => SensorKind::Data,
            "SmallData" => SensorKind::SmallData,
            "Throughput" => SensorKind::Throughput,
            "Control" => SensorKind::Control,
            "Energy" => SensorKind::Energy,
            "Humidity" => SensorKind::Humidity,
            "Noise" => SensorKind::Noise,
            "Flow" => SensorKind::Flow,
            other => SensorKind::Other(other.to_string()),
        }
    }

    /// The unit the raw value is always in, per the source's own contract.
    pub fn unit(&self) -> &str {
        match self {
            SensorKind::Temperature => "\u{00B0}C",
            SensorKind::Fan => "RPM",
            SensorKind::Voltage => "V",
            SensorKind::Current => "A",
            SensorKind::Power => "W",
            SensorKind::Clock => "MHz",
            SensorKind::Load | SensorKind::Level | SensorKind::Humidity => "%",
            SensorKind::Data => "GB",
            SensorKind::SmallData => "MB",
            SensorKind::Throughput => "B/s",
            SensorKind::Control => "%",
            SensorKind::Energy => "mWh",
            SensorKind::Noise => "dBA",
            SensorKind::Flow => "L/h",
            SensorKind::Factor | SensorKind::Other(_) => "",
        }
    }
}

/// One reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Sensor {
    /// The source's stable identifier, e.g. `/amdcpu/0/temperature/2`. Used to
    /// remember which sensor the user pinned, so it survives a reboot even
    /// though display names repeat across devices.
    pub id: String,
    /// Display name, e.g. "CPU Package".
    pub name: String,
    /// The device it belongs to, e.g. "AMD Ryzen 9 7950X".
    pub hardware: String,
    pub kind: SensorKind,
    /// The raw value in the kind's canonical unit.
    ///
    /// None when the source reported NaN, which it does for a sensor it knows
    /// about but cannot currently read. That is deliberately not zero: a fan
    /// that cannot be read is not a stopped fan.
    pub value: Option<f64>,
}

impl fmt::Display for Sensor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value {
            Some(v) => write!(f, "{} {} {:.1}{}", self.hardware, self.name, v, self.kind.unit()),
            None => write!(f, "{} {} unavailable", self.hardware, self.name),
        }
    }
}

/// Why a read did not produce sensors.
///
/// These map to states the Sensors settings pane shows, so they are separate
/// cases rather than one error string: "not installed" and "installed but not
/// running" need different things from the user.
#[derive(Clone, Debug, PartialEq)]
pub enum SensorError {
    /// No source is configured or discoverable.
    NotConfigured,
    /// A source is configured but nothing answered on its endpoint.
    NotRunning,
    /// Something answered but refused us.
    Unauthorized,
    /// Something answered and it was not what we expected.
    Malformed(String),
    /// The transport itself failed.
    Transport(String),
}

impl fmt::Display for SensorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SensorError::NotConfigured => f.write_str("no sensor source configured"),
            SensorError::NotRunning => f.write_str("sensor source is not running"),
            SensorError::Unauthorized => f.write_str("sensor source refused the request"),
            SensorError::Malformed(why) => write!(f, "unexpected response: {why}"),
            SensorError::Transport(why) => write!(f, "could not reach the sensor source: {why}"),
        }
    }
}

/// A source of sensor readings.
pub trait SensorProvider: Send {
    /// Name shown in the settings pane, e.g. "LibreHardwareMonitor".
    fn name(&self) -> &str;

    /// Reads every sensor the source currently exposes.
    fn read(&mut self) -> Result<Vec<Sensor>, SensorError>;
}

/// The hottest processor temperature, which is what the strip shows.
///
/// Matched on the id rather than the display name. LibreHardwareMonitor builds
/// ids from its own hardware type - `/amdcpu/0/temperature/2`,
/// `/intelcpu/0/temperature/0` - which is stable across locales and across
/// every marketing name a chip has ever been sold under. A name match would
/// have to guess at "CPU Package", "Core (Tctl/Tdie)", "Package id 0" and
/// whatever the next generation calls it.
///
/// Falls back to the hottest sensor anywhere, because a machine that reports
/// no processor temperature at all still reports something worth showing, and
/// a blank readout looks like the module is broken.
/// One reading of one kind belonging to a graphics adapter.
///
/// Windows publishes no clock, power or temperature for a GPU - only the
/// engine counters - so those come from the sensor source, which names the
/// adapter the way its driver does. The name is matched first; when the
/// source spells it differently and there is only one GPU among its
/// readings, that one is taken, because a machine with one graphics adapter
/// has no ambiguity to resolve.
///
/// Among a GPU's several readings of a kind, `preferred` names the ones
/// worth having in order - the core's clock and temperature, the package's
/// power - which is what Task Manager and the Mac's panel both show.
///
/// Lives here rather than in the panel because the panel's tiles and a
/// stack's readings are the same question asked twice, and they were
/// answering it differently.
pub fn gpu_reading(
    sensors: &[Sensor],
    adapter: Option<&str>,
    kind: SensorKind,
    preferred: &[&str],
) -> Option<f64> {
    let wanted = adapter.map(|name| name.trim().to_ascii_lowercase());
    let mut mine: Vec<&Sensor> = sensors
        .iter()
        .filter(|s| wanted.as_deref().is_some_and(|w| s.hardware.trim().eq_ignore_ascii_case(w)))
        .collect();
    if mine.is_empty() {
        let gpus: Vec<&Sensor> = sensors.iter().filter(|s| s.id.starts_with("/gpu")).collect();
        let mut hardware: Vec<&str> = gpus.iter().map(|s| s.hardware.as_str()).collect();
        hardware.sort_unstable();
        hardware.dedup();
        if hardware.len() == 1 {
            mine = gpus;
        }
    }
    let of_kind: Vec<&&Sensor> = mine.iter().filter(|s| s.kind == kind && s.value.is_some()).collect();
    preferred
        .iter()
        .find_map(|p| of_kind.iter().find(|s| s.name.to_ascii_lowercase().contains(p)))
        .or_else(|| of_kind.first())
        .and_then(|s| s.value)
}

/// What `gpu_reading` prefers for each kind, so the panel and a stack ask
/// for the same reading.
pub const GPU_CLOCK: [&str; 1] = ["core"];
pub const GPU_POWER: [&str; 3] = ["package", "gpu power", "board"];
pub const GPU_TEMPERATURE: [&str; 1] = ["core"];

pub fn hottest_cpu(sensors: &[Sensor]) -> Option<&Sensor> {
    let from_cpu = sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .filter(|s| s.value.is_some())
        .filter(|s| is_processor(&s.id))
        .max_by(|a, b| {
            a.value.unwrap().partial_cmp(&b.value.unwrap()).unwrap_or(std::cmp::Ordering::Equal)
        });
    from_cpu.or_else(|| hottest(sensors))
}

/// Whether a sensor id belongs to a processor.
fn is_processor(id: &str) -> bool {
    let Some(rest) = id.strip_prefix('/') else { return false };
    let device = rest.split('/').next().unwrap_or_default();
    // Every LibreHardwareMonitor CPU type ends in "cpu": amdcpu, intelcpu,
    // and the generic "cpu" older versions used.
    device.ends_with("cpu")
}

/// The hottest temperature anywhere, which is what the compact readout shows.
///
/// The macOS app shows the hottest processor die sensor in the menu bar and
/// the hottest sensor anywhere in the panel; this is the panel's figure.
pub fn hottest(sensors: &[Sensor]) -> Option<&Sensor> {
    sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .filter(|s| s.value.is_some())
        .max_by(|a, b| {
            a.value.unwrap().partial_cmp(&b.value.unwrap()).unwrap_or(std::cmp::Ordering::Equal)
        })
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn sensor(id: &str, kind: SensorKind, value: Option<f64>) -> Sensor {
        Sensor {
            id: id.into(),
            name: "n".into(),
            hardware: "h".into(),
            kind,
            value,
        }
    }

    #[test]
    fn the_processor_is_preferred_over_a_hotter_graphics_card() {
        let sensors = vec![
            sensor("/amdcpu/0/temperature/2", SensorKind::Temperature, Some(61.0)),
            sensor("/gpu-nvidia/0/temperature/0", SensorKind::Temperature, Some(78.0)),
        ];
        // The strip is a processor readout; a hot GPU must not take its place.
        assert_eq!(hottest_cpu(&sensors).unwrap().value, Some(61.0));
        // The panel figure is still the hottest thing in the machine.
        assert_eq!(hottest(&sensors).unwrap().value, Some(78.0));
    }

    #[test]
    fn without_a_processor_sensor_it_falls_back_rather_than_showing_nothing() {
        let sensors = vec![sensor("/gpu-amd/0/temperature/0", SensorKind::Temperature, Some(54.0))];
        assert_eq!(hottest_cpu(&sensors).unwrap().value, Some(54.0));
    }

    #[test]
    fn a_processor_sensor_that_cannot_be_read_is_skipped_not_treated_as_zero() {
        let sensors = vec![
            sensor("/intelcpu/0/temperature/0", SensorKind::Temperature, None),
            sensor("/intelcpu/0/temperature/1", SensorKind::Temperature, Some(49.0)),
        ];
        assert_eq!(hottest_cpu(&sensors).unwrap().value, Some(49.0));
    }

    #[test]
    fn only_temperatures_are_considered() {
        let sensors = vec![
            sensor("/amdcpu/0/load/0", SensorKind::Load, Some(99.0)),
            sensor("/amdcpu/0/temperature/0", SensorKind::Temperature, Some(45.0)),
        ];
        assert_eq!(hottest_cpu(&sensors).unwrap().value, Some(45.0));
    }

    #[test]
    fn processor_ids_are_recognized_across_the_vendors_and_the_old_generic() {
        assert!(is_processor("/amdcpu/0/temperature/2"));
        assert!(is_processor("/intelcpu/0/temperature/0"));
        assert!(is_processor("/cpu/0/temperature/0"));
        assert!(!is_processor("/gpu-nvidia/0/temperature/0"));
        assert!(!is_processor("/nvme/0/temperature/0"));
        assert!(!is_processor("no-leading-slash/0"));
    }
}
