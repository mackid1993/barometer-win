// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Stacks: several readings inside one item.
//
// Ported from Sources/MenuBarStatsCore/Modules/Stacks/ in the macOS app.
// A stack composes *readings* rather than whole module presentations, so one
// item can mix CPU, memory, network and power. That is what makes the strip
// affordable: seven modules each carrying their own label cost far more width
// than one stack showing the six numbers the user actually wanted.

use crate::module::ModuleId;
use crate::sensors::{Sensor, SensorKind};
use crate::weather::models::TemperatureUnit;

/// One thing on the strip, in the composer's one list.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum StripItem {
    Module(ModuleId),
    Stack(u32),
}

impl StripItem {
    /// The persisted form: `module:cpu`, `stack:3`.
    pub fn raw_value(self) -> String {
        match self {
            StripItem::Module(id) => format!("module:{}", id.key()),
            StripItem::Stack(id) => format!("stack:{id}"),
        }
    }

    pub fn from_raw(raw: &str) -> Option<StripItem> {
        if let Some(key) = raw.strip_prefix("module:") {
            return ModuleId::ALL.into_iter().find(|id| id.key() == key).map(StripItem::Module);
        }
        raw.strip_prefix("stack:")?.parse().ok().map(StripItem::Stack)
    }
}

/// One reading a stack can show.
///
/// The raw strings are persisted and **must never change**. Adding is safe;
/// renaming silently empties somebody's stack.
///
/// Not `Copy`, because of `Sensor`: a machine has a hundred and more
/// sensors and a stack can show any of them, so the metric has to carry
/// which one, and the only stable name for a sensor is the source's own
/// identifier - `/amdcpu/0/temperature/2` - which is a string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StackMetric {
    CpuTotal,
    CpuUser,
    CpuSystem,
    CpuIdle,
    CpuLoad,
    GpuUtilization,
    GpuPower,
    GpuTemperature,
    MemoryUsedPercent,
    MemoryUsedBytes,
    MemoryFreeBytes,
    MemoryPressure,
    MemorySwap,
    DiskRead,
    DiskWrite,
    DiskUsedPercent,
    DiskFreeBytes,
    NetworkDownload,
    NetworkUpload,
    SensorsHottest,
    SensorsFan,
    WeatherTemperature,
    /// One particular sensor, by the source's identifier. Every sensor the
    /// source reports is available this way: each core's temperature, each
    /// GPU's, every fan and voltage, not only the hottest and the first fan.
    Sensor(String),
}

/// One reading in a stack, with what the strip calls it.
///
/// The label is the user's, and independent of the sensor's own name and of
/// any mapping: a row can say `foo` if that is what the user wants it to
/// say. None means the metric's usual label.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StackEntry {
    pub metric: StackMetric,
    pub label: Option<String>,
}

impl StackEntry {
    pub fn new(metric: StackMetric) -> StackEntry {
        StackEntry { metric, label: None }
    }

    /// What the strip writes beside the value: the user's label if they set
    /// one, else the metric's own, which for a sensor is the sensor's name.
    pub fn caption(&self, sensors: &[Sensor]) -> String {
        match &self.label {
            // Without a trailing colon, whatever was typed: the renderer
            // adds the colon itself, and people typed one before it did.
            Some(label) if !label.trim().is_empty() => {
                label.trim().trim_end_matches(':').trim_end().to_string()
            }
            _ => self.metric.label_in(sensors),
        }
    }

    /// The label the strip falls back to, for showing as a placeholder.
    pub fn default_caption(&self, sensors: &[Sensor]) -> String {
        self.metric.label_in(sensors)
    }
}

/// Which units reserved widths are computed against.
///
/// The macOS `AppSettings`' share of this, built from the settings store by
/// the renderer. It is separate from the metric because the widest string a
/// reading can show depends on the unit chosen for it, and the renderer sizes
/// from that.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UnitPrefs {
    /// Whether *hardware* temperatures are shown in Fahrenheit -
    /// `SensorSettings::temperature`, not the weather's. The two are
    /// deliberately separate settings, and reserving a column from the wrong
    /// one gives the strip a width for `125\u{00B0}C` while it draws
    /// `257\u{00B0}F`. The weather's own column reserves `-99\u{00B0}`, which
    /// is the same width in either scale, so this is the only temperature
    /// unit a reservation depends on.
    pub hardware_fahrenheit: bool,
    /// Which convention throughput is shown in, so a rate reserves the width
    /// its own formatter produces.
    pub rate: crate::format::RateUnit,
}

impl Default for UnitPrefs {
    fn default() -> Self {
        // Celsius, which is `SensorSettings::temperature`'s default and what
        // every sensor reports natively.
        UnitPrefs { hardware_fahrenheit: false, rate: crate::format::RateUnit::Bytes }
    }
}

impl StackMetric {
    /// Every metric, in the order the settings picker offers them.
    pub const ALL: [StackMetric; 22] = [
        StackMetric::CpuTotal,
        StackMetric::CpuUser,
        StackMetric::CpuSystem,
        StackMetric::CpuIdle,
        StackMetric::CpuLoad,
        StackMetric::GpuUtilization,
        StackMetric::GpuPower,
        StackMetric::GpuTemperature,
        StackMetric::MemoryUsedPercent,
        StackMetric::MemoryUsedBytes,
        StackMetric::MemoryFreeBytes,
        StackMetric::MemoryPressure,
        StackMetric::MemorySwap,
        StackMetric::DiskRead,
        StackMetric::DiskWrite,
        StackMetric::DiskUsedPercent,
        StackMetric::DiskFreeBytes,
        StackMetric::NetworkDownload,
        StackMetric::NetworkUpload,
        StackMetric::SensorsHottest,
        StackMetric::SensorsFan,
        StackMetric::WeatherTemperature,
    ];
}

impl StackMetric {
    /// The persisted identifier. Never change one of these.
    pub fn raw_value(&self) -> String {
        use StackMetric::*;
        let fixed = match self {
            Sensor(id) => return format!("sensor:{id}"),
            CpuTotal => "cpu.total",
            CpuUser => "cpu.user",
            CpuSystem => "cpu.system",
            CpuIdle => "cpu.idle",
            CpuLoad => "cpu.load",
            GpuUtilization => "gpu.utilization",
            GpuPower => "gpu.power",
            GpuTemperature => "gpu.temperature",
            MemoryUsedPercent => "memory.usedPercent",
            MemoryUsedBytes => "memory.usedBytes",
            MemoryFreeBytes => "memory.freeBytes",
            MemoryPressure => "memory.pressure",
            MemorySwap => "memory.swap",
            DiskRead => "disks.read",
            DiskWrite => "disks.write",
            DiskUsedPercent => "disks.usedPercent",
            DiskFreeBytes => "disks.freeBytes",
            NetworkDownload => "network.download",
            NetworkUpload => "network.upload",
            SensorsHottest => "sensors.hottest",
            SensorsFan => "sensors.fan",
            WeatherTemperature => "weather.temperature",
        };
        fixed.to_string()
    }

    /// Parses a persisted identifier. An unknown value is dropped rather than
    /// failing the whole record, so a stack written by a newer build still
    /// loads here with the metrics this build understands.
    pub fn from_raw(raw: &str) -> Option<StackMetric> {
        if let Some(id) = raw.strip_prefix("sensor:") {
            return (!id.is_empty()).then(|| StackMetric::Sensor(id.to_string()));
        }
        StackMetric::ALL.iter().find(|m| m.raw_value() == raw).cloned()
    }

    /// The module that samples this reading.
    ///
    /// A stack keeps its source module's scheduler running and takes its color
    /// from that module, so every metric names exactly one owner.
    pub fn module(&self) -> ModuleId {
        use StackMetric::*;
        match self {
            CpuTotal | CpuUser | CpuSystem | CpuIdle | CpuLoad => ModuleId::Cpu,
            GpuUtilization | GpuPower | GpuTemperature => ModuleId::Gpu,
            MemoryUsedPercent | MemoryUsedBytes | MemoryFreeBytes | MemoryPressure
            | MemorySwap => ModuleId::Memory,
            DiskRead | DiskWrite | DiskUsedPercent | DiskFreeBytes => ModuleId::Disks,
            NetworkDownload | NetworkUpload => ModuleId::Network,
            SensorsHottest | SensorsFan | Sensor(_) => ModuleId::Sensors,
            WeatherTemperature => ModuleId::Weather,
        }
    }

    /// The sensor this metric names, looked up in what the source reports.
    pub fn sensor<'a>(&self, sensors: &'a [Sensor]) -> Option<&'a Sensor> {
        match self {
            StackMetric::Sensor(id) => sensors.iter().find(|s| &s.id == id),
            _ => None,
        }
    }

    /// Short label drawn above or beside the value, given the sensors the
    /// source reports, so a sensor metric can be called by the sensor's name.
    pub fn label_in(&self, sensors: &[Sensor]) -> String {
        match self {
            StackMetric::Sensor(id) => match self.sensor(sensors) {
                Some(sensor) => sensor.name.clone(),
                None => sensor_fallback_label(id),
            },
            other => other.label().to_string(),
        }
    }

    /// Short label drawn above or beside the value.
    ///
    /// A sensor's label wants the sensor's name, which only `label_in` has;
    /// this gives a stand-in derived from the identifier.
    pub fn label(&self) -> String {
        use StackMetric::*;
        let fixed = match self {
            Sensor(id) => return sensor_fallback_label(id),
            CpuTotal => "CPU",
            CpuUser => "USR",
            CpuSystem => "SYS",
            CpuIdle => "IDLE",
            CpuLoad => "LOAD",
            GpuUtilization => "GPU",
            GpuPower => "GPUW",
            GpuTemperature => "GPU\u{00B0}",
            MemoryUsedPercent => "MEM",
            MemoryUsedBytes => "USED",
            MemoryFreeBytes => "FREE",
            MemoryPressure => "PRES",
            MemorySwap => "SWAP",
            DiskRead => "READ",
            DiskWrite => "WRIT",
            DiskUsedPercent => "DISK",
            DiskFreeBytes => "DFRE",
            NetworkDownload => "DOWN",
            NetworkUpload => "UP",
            SensorsHottest => "TEMP",
            SensorsFan => "FAN",
            WeatherTemperature => "OUT",
        };
        fixed.to_string()
    }
}

/// A label for a sensor whose name is not to hand, from its identifier:
/// `/amdcpu/0/temperature/2` becomes `TEMP2`. Only ever seen when the
/// source is not running, since then there is no name to show instead.
fn sensor_fallback_label(id: &str) -> String {
    let mut parts = id.rsplit('/');
    let index = parts.next().unwrap_or_default();
    let kind = parts.next().unwrap_or_default();
    let short = match kind {
        "temperature" => "TEMP",
        "fan" => "FAN",
        "voltage" => "VOLT",
        "current" => "AMP",
        "power" => "PWR",
        "clock" => "CLK",
        "load" => "LOAD",
        "level" => "LVL",
        "control" => "CTRL",
        "data" | "smalldata" => "DATA",
        "throughput" => "RATE",
        "energy" => "WH",
        "humidity" => "HUM",
        "noise" => "DB",
        "flow" => "FLOW",
        _ => "SENS",
    };
    format!("{short}{index}")
}

impl StackMetric {
    /// Full name shown when choosing metrics in settings, given the sensors
    /// the source reports: "AMD Ryzen 9 7950X · Core #3".
    pub fn display_name_in(&self, sensors: &[Sensor]) -> String {
        match self {
            StackMetric::Sensor(id) => match self.sensor(sensors) {
                // The kind, always: the source names a temperature, a power
                // figure and a load all "CPU Package".
                Some(sensor) => format!(
                    "{} \u{00B7} {} \u{00B7} {}",
                    sensor.hardware,
                    sensor.name,
                    sensor.kind.name()
                ),
                None => describe_sensor_id(id),
            },
            other => other.display_name(),
        }
    }

    /// Full name shown when choosing metrics in settings.
    pub fn display_name(&self) -> String {
        use StackMetric::*;
        let fixed = match self {
            Sensor(id) => return describe_sensor_id(id),
            CpuTotal => "CPU usage",
            CpuUser => "CPU user",
            CpuSystem => "CPU system",
            CpuIdle => "CPU idle",
            CpuLoad => "Load average",
            GpuUtilization => "GPU usage",
            GpuPower => "GPU power",
            GpuTemperature => "GPU temperature",
            MemoryUsedPercent => "Memory used",
            MemoryUsedBytes => "Memory used bytes",
            MemoryFreeBytes => "Memory free bytes",
            MemoryPressure => "Memory pressure",
            MemorySwap => "Swap used",
            DiskRead => "Disk read rate",
            DiskWrite => "Disk write rate",
            DiskUsedPercent => "Disk used",
            DiskFreeBytes => "Disk free",
            NetworkDownload => "Network download",
            NetworkUpload => "Network upload",
            SensorsHottest => "Hottest temperature",
            SensorsFan => "Fan speed",
            WeatherTemperature => "Outside temperature",
        };
        fixed.to_string()
    }

    /// Metrics grouped by owning module, in a stable order for the picker.
    pub fn by_module() -> Vec<(ModuleId, Vec<StackMetric>)> {
        ModuleId::ALL
            .iter()
            .filter_map(|&module| {
                let metrics: Vec<StackMetric> =
                    StackMetric::ALL.iter().filter(|m| m.module() == module).cloned().collect();
                (!metrics.is_empty()).then_some((module, metrics))
            })
            .collect()
    }

    /// The metric a module contributes when an older membership is migrated.
    pub fn primary(module: ModuleId) -> Option<StackMetric> {
        match module {
            ModuleId::Cpu => Some(StackMetric::CpuTotal),
            ModuleId::Gpu => Some(StackMetric::GpuUtilization),
            ModuleId::Memory => Some(StackMetric::MemoryUsedPercent),
            ModuleId::Disks => Some(StackMetric::DiskRead),
            ModuleId::Network => Some(StackMetric::NetworkDownload),
            ModuleId::Sensors => Some(StackMetric::SensorsHottest),
            ModuleId::Weather => Some(StackMetric::WeatherTemperature),
        }
    }

    /// The widest string this reading can ever show under the given units.
    ///
    /// The renderer and the settings preview both size from this, so a preview
    /// can never be a different width than the item it previews, and a column
    /// never has to grow into its final width over the first few samples. It is
    /// also how the strip stays still: width comes from what a reading *could*
    /// show, never from what it happens to show right now.
    pub fn reserved_value(&self, units: UnitPrefs) -> &'static str {
        use StackMetric::*;
        // The widest string each formatter actually produces, not a
        // hand-written approximation of it: `format::bytes` and
        // `format::rate` both put a space before the unit, and a
        // reservation a character short is a column that resizes the first
        // time the number gets long.
        // Four digits, for the same reason `RateUnit::widest` carries four:
        // the units are binary and the number is written in decimal, so a
        // figure stays in gigabytes right up to 1024 of them.
        let capacity = "1024 GB";
        let degrees = if units.hardware_fahrenheit { "257\u{00B0}F" } else { "125\u{00B0}C" };
        match self {
            // Without the sensor to hand its kind is unknown; a temperature
            // is the widest of the common kinds and the most common.
            Sensor(_) => degrees,
            CpuTotal | CpuUser | CpuSystem | CpuIdle | GpuUtilization | MemoryUsedPercent
            | MemoryPressure | DiskUsedPercent => "100%",
            CpuLoad => "99.99",
            GpuPower => "199.9W",
            GpuTemperature | SensorsHottest => degrees,
            MemoryUsedBytes | MemoryFreeBytes | MemorySwap | DiskFreeBytes => capacity,
            // Disk rates are bytes whatever the network is shown in: the
            // disks module formats with `format::rate`, which is the byte
            // convention, and there is no setting that changes it.
            DiskRead | DiskWrite => crate::format::RateUnit::Bytes.widest(),
            // The macOS placeholder comes from NetworkRateFormatter, which
            // also varies with a decimal-places setting this port does not
            // have. What it varies with here is the bytes-or-bits choice,
            // which `RateUnit::widest` answers.
            NetworkDownload | NetworkUpload => units.rate.widest(),
            SensorsFan => "9999r",
            // The strip draws the weather module's own string, which carries
            // the unit letter - "72\u{00B0}F", not "72\u{00B0}" - so the
            // reservation has to carry it too; without it this was a whole
            // character short and the column grew as soon as a reading
            // arrived. Both candidates, for the reasons on
            // `reserved_air_temperature`; the two symbols measure the same,
            // so one static pair covers either setting.
            WeatherTemperature => "100\u{00B0}C\n-44\u{00B0}C",
        }
    }

    /// The widest string a sensor of this kind can show, for a metric whose
    /// sensor is to hand.
    pub fn reserved_value_in(&self, sensors: &[Sensor], units: UnitPrefs) -> &'static str {
        match self.sensor(sensors) {
            Some(sensor) => reserved_for_kind(&sensor.kind, units),
            None => self.reserved_value(units),
        }
    }
}

/// What a sensor identifier says, for when the source is not reporting it.
///
/// The identifiers are the source's own: `/intelcpu/0/temperature/18` is the
/// eighteenth temperature of the first Intel processor. Only the source
/// knows the sensor's *name* ("CPU Package"), so when it is not running -
/// the settings window opened on its own, the helper stopped, the module
/// switched off, hardware that has since been unplugged - there is nothing
/// to look the name up in.
///
/// The identifier was printed raw in that case, which put
/// "/intelcpu/0/temperature/18" in front of the user where a name belongs.
/// This says what the identifier means instead, and leaves the number on so
/// two temperatures of the same device stay apart.
pub fn describe_sensor_id(id: &str) -> String {
    let parts: Vec<&str> = id.split('/').filter(|part| !part.is_empty()).collect();
    let Some(hardware) = parts.first() else { return id.to_string() };
    let hardware = match *hardware {
        "intelcpu" => "Intel CPU",
        "amdcpu" => "AMD CPU",
        "genericcpu" | "cpu" => "CPU",
        "gpu-nvidia" => "NVIDIA GPU",
        "gpu-amd" | "atigpu" => "AMD GPU",
        "gpu-intel" => "Intel GPU",
        "ram" => "Memory",
        "nvme" => "NVMe drive",
        "hdd" | "ssd" => "Drive",
        "lpc" | "mainboard" => "Mainboard",
        "psu" => "Power supply",
        "battery" => "Battery",
        "network" | "nic" => "Network adapter",
        // Something this build has no word for: its own segment is better
        // than the whole path.
        other => other,
    };
    // The kind is the first segment that names one; what follows it is the
    // sensor's number within that kind.
    let kinds = [
        ("temperature", "Temperature"),
        ("load", "Load"),
        ("power", "Power"),
        ("clock", "Clock"),
        ("voltage", "Voltage"),
        ("current", "Current"),
        ("fan", "Fan"),
        ("control", "Control"),
        ("level", "Level"),
        ("data", "Data"),
        ("throughput", "Throughput"),
        ("factor", "Factor"),
        ("humidity", "Humidity"),
    ];
    let found = parts
        .iter()
        .enumerate()
        .find_map(|(at, part)| kinds.iter().find(|(raw, _)| raw == part).map(|(_, name)| (at, *name)));
    let Some((at, kind)) = found else {
        return hardware.to_string();
    };
    match parts.get(at + 1).and_then(|number| number.parse::<u32>().ok()) {
        Some(number) => format!("{hardware} \u{00B7} {kind} #{number}"),
        None => format!("{hardware} \u{00B7} {kind}"),
    }
}

/// The widest reading a sensor kind produces, in the strip's formatting.
pub fn reserved_for_kind(kind: &SensorKind, units: UnitPrefs) -> &'static str {
    match kind {
        SensorKind::Temperature => {
            if units.hardware_fahrenheit {
                "257\u{00B0}F"
            } else {
                "125\u{00B0}C"
            }
        }
        SensorKind::Fan => "9999r",
        SensorKind::Voltage => "99.99V",
        SensorKind::Current => "99.9A",
        SensorKind::Power => "999.9W",
        SensorKind::Clock => "9999MHz",
        SensorKind::Load | SensorKind::Level | SensorKind::Control | SensorKind::Humidity => "100%",
        SensorKind::Data => "9999GB",
        SensorKind::SmallData => "9999MB",
        // Rendered through `format::rate`, whose three-figure formatting
        // prints 1024 at the top of a unit, as the disk and network columns
        // reserve for.
        SensorKind::Throughput => "1024 MB/s",
        SensorKind::Energy => "99999mWh",
        SensorKind::Noise => "99dBA",
        SensorKind::Flow => "999.9L/h",
        SensorKind::Factor | SensorKind::Other(_) => "9999.99",
    }
}

/// A sensor's reading as the strip shows it: compact, with its unit.
///
/// Temperatures follow the hardware unit setting the way the Sensors module
/// does; everything else is in the source's own unit, which is the unit the
/// user sees in that source's window and would expect here.
pub fn format_sensor(sensor: &Sensor, temperature: TemperatureUnit) -> String {
    let Some(value) = sensor.value else { return "--".to_string() };
    match &sensor.kind {
        SensorKind::Temperature => temperature.describe(value),
        SensorKind::Fan => format!("{value:.0}r"),
        SensorKind::Voltage => format!("{value:.2}V"),
        SensorKind::Current => format!("{value:.1}A"),
        SensorKind::Power => format!("{value:.1}W"),
        SensorKind::Clock => format!("{value:.0}MHz"),
        SensorKind::Load | SensorKind::Level | SensorKind::Control | SensorKind::Humidity => {
            format!("{value:.0}%")
        }
        SensorKind::Data => format!("{value:.0}GB"),
        SensorKind::SmallData => format!("{value:.0}MB"),
        SensorKind::Throughput => crate::format::rate(value.max(0.0)),
        SensorKind::Energy => format!("{value:.0}mWh"),
        SensorKind::Noise => format!("{value:.0}dBA"),
        SensorKind::Flow => format!("{value:.1}L/h"),
        SensorKind::Factor | SensorKind::Other(_) => format!("{value:.2}"),
    }
}

/// How a stack arranges its metrics inside one item.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum StackLayout {
    /// Two metrics per column, expanding horizontally. Matches the Sensors
    /// compact stack, and is what makes a stack cheaper than separate items:
    /// two readings occupy the height the taskbar was giving away anyway.
    #[default]
    Columns,
    /// Every metric on one line.
    SingleRow,
}

impl StackLayout {
    pub fn raw_value(self) -> &'static str {
        match self {
            StackLayout::Columns => "columns",
            StackLayout::SingleRow => "singleRow",
        }
    }

    pub fn from_raw(raw: &str) -> Option<StackLayout> {
        match raw {
            "columns" => Some(StackLayout::Columns),
            "singleRow" => Some(StackLayout::SingleRow),
            _ => None,
        }
    }
}

/// Persisted configuration for one stack.
#[derive(Clone, Debug, PartialEq)]
pub struct StackSettings {
    /// Stable one-based instance number used by this item's identity.
    pub id: u32,
    pub is_enabled: bool,
    /// User-facing name. Settings only: the item's identity stays permanent,
    /// so renaming a stack never changes what a layout manager sees.
    pub name: String,
    pub layout: StackLayout,
    /// Readings shown left to right, in the order the user chose, each with
    /// the label the user gave it.
    pub metrics: Vec<StackEntry>,
    /// Readings this build does not understand, kept with the place they
    /// held so a newer Barometer's stack survives being opened by an older
    /// one - which is what `store` promises and could not deliver while
    /// these were simply dropped on the way in.
    ///
    /// Raw strings rather than parsed anything: not understanding them is
    /// the whole point. Nothing shows them, so the user cannot delete one by
    /// accident; deleting the stack takes them with it, which is right.
    pub unknown_metrics: Vec<(usize, String)>,
    /// Whether the individual items of the modules in this stack are hidden.
    ///
    /// This is the space saving: put CPU, MEM and GPU in one stack, tick this,
    /// and the three separate readouts stop taking room of their own.
    pub hides_source_items: bool,
}

impl Default for StackSettings {
    fn default() -> Self {
        StackSettings {
            id: 1,
            is_enabled: true,
            name: String::new(),
            layout: StackLayout::Columns,
            metrics: Vec::new(),
            unknown_metrics: Vec::new(),
            hides_source_items: false,
        }
    }
}

impl StackSettings {
    pub fn new(id: u32) -> Self {
        StackSettings { id: id.max(1), ..Default::default() }
    }

    /// The name to show.
    ///
    /// Stacks are named by whoever creates them and nothing here invents a
    /// name. A generated one would either follow the permanent instance
    /// number, which only ever climbs and would reach "Stack 47", or follow
    /// the list position, which would silently rename every stack below a
    /// deleted one. This fallback is only for a record older than the naming
    /// prompt.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "Untitled"
        } else {
            &self.name
        }
    }

    /// The readings as a file should carry them: what this build knows, with
    /// what it does not put back where it was.
    ///
    /// `Err` is a raw string to write out untouched; `Ok` is one of ours.
    pub fn metrics_for_file(&self) -> Vec<Result<&StackEntry, &str>> {
        let mut out: Vec<Result<&StackEntry, &str>> = self.metrics.iter().map(Ok).collect();
        // Ascending, so each insertion leaves the later indices right.
        let mut unknown: Vec<&(usize, String)> = self.unknown_metrics.iter().collect();
        unknown.sort_by_key(|(at, _)| *at);
        for (at, raw) in unknown {
            let at = (*at).min(out.len());
            out.insert(at, Err(raw.as_str()));
        }
        out
    }

    /// Whether a reading is already in this stack.
    pub fn has(&self, metric: &StackMetric) -> bool {
        self.metrics.iter().any(|entry| &entry.metric == metric)
    }

    /// Modules whose schedulers this stack needs running.
    pub fn source_modules(&self) -> Vec<ModuleId> {
        let mut modules: Vec<ModuleId> = self.metrics.iter().map(|e| e.metric.module()).collect();
        modules.sort();
        modules.dedup();
        modules
    }
}

/// The stacks the user has created, in display order.
///
/// There is no cap. Every enabled stack is one more item the strip has to find
/// room for, so a great many stacks costs taskbar width rather than
/// correctness.
#[derive(Clone, Debug, PartialEq)]
pub struct StacksSettings {
    pub stacks: Vec<StackSettings>,
    /// Smallest instance number never yet handed out.
    ///
    /// Persisted rather than derived from the stacks in use. Deleting a stack
    /// must not release its identity for reuse: a later stack handed the same
    /// number would inherit the deleted one's saved position.
    next_id: u32,
}

impl Default for StacksSettings {
    fn default() -> Self {
        StacksSettings { stacks: Vec::new(), next_id: 1 }
    }
}

impl StacksSettings {
    /// Rebuilt from storage. The next identity is taken as stored, or past
    /// every stack present if the stored one would hand out a number in use.
    pub fn from_parts(stacks: Vec<StackSettings>, next_id: u32) -> StacksSettings {
        let past_all = stacks.iter().map(|s| s.id + 1).max().unwrap_or(1);
        StacksSettings { stacks, next_id: next_id.max(past_all).max(1) }
    }

    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Creates a stack with the next never-used identity.
    pub fn add(&mut self) -> u32 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.stacks.push(StackSettings::new(id));
        id
    }

    /// Removes a stack. Its identity is retired, never handed out again.
    pub fn remove(&mut self, id: u32) {
        self.stacks.retain(|s| s.id != id);
    }

    /// Whether any stack carries a reading only the sensor source can give.
    ///
    /// The strip's tick asks this before cloning the machine's whole sensor
    /// list - a hundred and forty-four readings of three strings apiece on
    /// the author's machine - into the snapshot the columns are built from.
    /// Nothing else on the strip reads that list: the Sensors column draws
    /// its module's own readout. With no such reading anywhere the clone was
    /// several hundred allocations a second for nobody.
    ///
    /// The graphics power and temperature count, and that is the part that
    /// is easy to get wrong. Windows' engine counters carry neither, so the
    /// graphics module cannot answer for them and the tick falls back to the
    /// sensor list - see `gpu_from_sensors`. Leaving them out would draw a
    /// dash where a wattage should be.
    ///
    /// Every stack, not only the enabled ones, because the value list a tick
    /// builds is built for all of them. Saying yes too often costs a clone;
    /// saying no wrongly costs a reading.
    pub fn needs_the_sensor_list(&self) -> bool {
        self.stacks.iter().any(|stack| {
            stack.metrics.iter().any(|entry| {
                matches!(
                    entry.metric,
                    StackMetric::Sensor(_)
                        | StackMetric::GpuPower
                        | StackMetric::GpuTemperature
                )
            })
        })
    }

    /// Modules that must keep sampling for the enabled stacks.
    pub fn required_modules(&self) -> Vec<ModuleId> {
        let mut modules: Vec<ModuleId> = self
            .stacks
            .iter()
            .filter(|s| s.is_enabled)
            .flat_map(|s| s.source_modules())
            .collect();
        modules.sort();
        modules.dedup();
        modules
    }

    /// Modules whose own items are hidden because an enabled stack that
    /// contains them asked for it.
    pub fn hidden_source_modules(&self) -> Vec<ModuleId> {
        let mut modules: Vec<ModuleId> = self
            .stacks
            .iter()
            .filter(|s| s.is_enabled && s.hides_source_items)
            .flat_map(|s| s.source_modules())
            .collect();
        modules.sort();
        modules.dedup();
        modules
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_typed_with_a_colon_reads_without_it() {
        let entry = StackEntry { metric: StackMetric::CpuTotal, label: Some("CPU: ".into()) };
        assert_eq!(entry.caption(&[]), "CPU");
        let plain = StackEntry { metric: StackMetric::CpuTotal, label: Some("foo".into()) };
        assert_eq!(plain.caption(&[]), "foo");
    }

    #[test]
    fn every_raw_value_round_trips() {
        // These strings are on disk in people's settings. If this breaks, a
        // stack silently loses the metric that changed.
        for metric in StackMetric::ALL.iter() {
            assert_eq!(StackMetric::from_raw(&metric.raw_value()).as_ref(), Some(metric));
        }
        let sensor = StackMetric::Sensor("/amdcpu/0/temperature/2".into());
        assert_eq!(StackMetric::from_raw(&sensor.raw_value()), Some(sensor));
    }

    #[test]
    fn raw_values_are_unique() {
        let mut seen: Vec<String> = StackMetric::ALL.iter().map(|m| m.raw_value()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two metrics share a persisted identifier");
    }

    #[test]
    fn an_unknown_metric_is_dropped_not_fatal() {
        // A stack written by a newer build must still load here.
        assert_eq!(StackMetric::from_raw("cpu.quantum"), None);
    }

    #[test]
    fn every_module_that_owns_metrics_appears_once_in_the_picker() {
        let grouped = StackMetric::by_module();
        let mut modules: Vec<ModuleId> = grouped.iter().map(|(m, _)| *m).collect();
        let before = modules.len();
        modules.sort();
        modules.dedup();
        assert_eq!(modules.len(), before);
        // Every metric is reachable through exactly one group.
        let total: usize = grouped.iter().map(|(_, ms)| ms.len()).sum();
        assert_eq!(total, StackMetric::ALL.len());
    }

    #[test]
    fn a_deleted_stack_never_gives_its_identity_back() {
        // Reusing it would hand a new stack the deleted one's saved position.
        let mut settings = StacksSettings::default();
        let first = settings.add();
        let second = settings.add();
        settings.remove(first);
        let third = settings.add();
        assert_ne!(third, first);
        assert_ne!(third, second);
        assert!(third > second);
    }

    #[test]
    fn hiding_source_items_reports_the_modules_to_hide() {
        let mut settings = StacksSettings::default();
        let id = settings.add();
        let stack = settings.stacks.iter_mut().find(|s| s.id == id).unwrap();
        stack.metrics =
            vec![StackEntry::new(StackMetric::CpuTotal), StackEntry::new(StackMetric::MemoryUsedPercent)];
        stack.hides_source_items = true;
        assert_eq!(settings.hidden_source_modules(), vec![ModuleId::Cpu, ModuleId::Memory]);
    }

    #[test]
    fn a_disabled_stack_asks_for_nothing() {
        let mut settings = StacksSettings::default();
        let id = settings.add();
        let stack = settings.stacks.iter_mut().find(|s| s.id == id).unwrap();
        stack.metrics = vec![StackEntry::new(StackMetric::CpuTotal)];
        stack.hides_source_items = true;
        stack.is_enabled = false;
        assert!(settings.required_modules().is_empty());
        assert!(settings.hidden_source_modules().is_empty());
    }

    #[test]
    fn reserved_width_does_not_depend_on_the_current_reading() {
        // The whole point: width comes from what a reading could show, so the
        // strip never resizes as numbers change.
        let units = UnitPrefs::default();
        assert_eq!(StackMetric::CpuTotal.reserved_value(units), "100%");
        // Every reservation is a string its own formatter can produce, which
        // is the only way the width can be right.
        // Four digits, not three: binary units written in decimal leave a
        // figure in gigabytes up to 1024 of them, and a reservation taken
        // from "999 GB" is one character short exactly when the number gets
        // long - which is the moment the column must not move.
        assert_eq!(StackMetric::MemoryUsedBytes.reserved_value(units), "1024 GB");
        let nearly_a_terabyte = (1023.6 * (1u64 << 30) as f64) as u64;
        assert_eq!(
            StackMetric::MemoryUsedBytes.reserved_value(units),
            crate::format::bytes(nearly_a_terabyte)
        );
        assert_eq!(StackMetric::DiskRead.reserved_value(units), "1024 MB/s");
        assert_eq!(
            StackMetric::DiskRead.reserved_value(units),
            crate::format::rate(1023.9 * 1024.0 * 1024.0)
        );
        assert_eq!(StackMetric::NetworkDownload.reserved_value(units), "1024 MB/s");
        let bits = UnitPrefs { rate: crate::format::RateUnit::Bits, ..units };
        assert_eq!(StackMetric::NetworkDownload.reserved_value(bits), "1024 Mb/s");
    }

    #[test]
    fn a_hardware_temperature_reserves_the_scale_the_sensors_are_shown_in() {
        let celsius = UnitPrefs::default();
        assert_eq!(StackMetric::SensorsHottest.reserved_value(celsius), "125\u{00B0}C");
        let fahrenheit = UnitPrefs { hardware_fahrenheit: true, ..celsius };
        assert_eq!(StackMetric::SensorsHottest.reserved_value(fahrenheit), "257\u{00B0}F");
        // The weather's column is the same width in either scale, so it does
        // not follow this setting - or the weather's own. It does carry the
        // unit letter, because the string the strip draws carries one; without
        // it this was a whole character short of every reading.
        assert_eq!(
            StackMetric::WeatherTemperature.reserved_value(fahrenheit),
            "100\u{00B0}C\n-44\u{00B0}C"
        );
        assert_eq!(
            StackMetric::WeatherTemperature.reserved_value(celsius),
            StackMetric::WeatherTemperature.reserved_value(fahrenheit)
        );
    }

    #[test]
    fn the_sensor_list_is_wanted_only_by_a_stack_that_carries_a_reading_from_it() {
        let mut stacks = StacksSettings::default();
        assert!(!stacks.needs_the_sensor_list(), "no stacks, nothing to read it");

        let id = stacks.add();
        let stack = stacks.stacks.iter_mut().find(|s| s.id == id).expect("just added");
        stack.metrics = vec![StackEntry::new(StackMetric::CpuTotal)];
        assert!(!stacks.needs_the_sensor_list(), "a processor reading is the processor's");

        // The trap. Windows' engine counters carry neither figure, so the
        // graphics module cannot answer and the tick reads them out of the
        // sensor list instead.
        for metric in [StackMetric::GpuPower, StackMetric::GpuTemperature] {
            let stack = stacks.stacks.iter_mut().find(|s| s.id == id).expect("just added");
            stack.metrics = vec![StackEntry::new(metric)];
            assert!(stacks.needs_the_sensor_list(), "only the sensor source has this one");
        }

        let stack = stacks.stacks.iter_mut().find(|s| s.id == id).expect("just added");
        stack.metrics = vec![StackEntry::new(StackMetric::Sensor("/intelcpu/0/temperature/18".into()))];
        assert!(stacks.needs_the_sensor_list());

        // A disabled stack still counts: the values a tick builds are built
        // for every stack, not only the shown ones.
        let stack = stacks.stacks.iter_mut().find(|s| s.id == id).expect("just added");
        stack.is_enabled = false;
        assert!(stacks.needs_the_sensor_list());

        // And it goes with the stack.
        stacks.remove(id);
        assert!(!stacks.needs_the_sensor_list());
    }

    #[test]
    fn a_sensor_the_source_is_not_reporting_is_named_from_its_identifier() {
        // What the settings showed before was the identifier itself, which
        // is what the user was looking at when they said so.
        assert_eq!(describe_sensor_id("/intelcpu/0/temperature/18"), "Intel CPU \u{00B7} Temperature #18");
        assert_eq!(describe_sensor_id("/gpu-nvidia/0/temperature/0"), "NVIDIA GPU \u{00B7} Temperature #0");
        assert_eq!(describe_sensor_id("/amdcpu/0/load/2"), "AMD CPU \u{00B7} Load #2");
        assert_eq!(describe_sensor_id("/nvme/1/temperature/3"), "NVMe drive \u{00B7} Temperature #3");
        assert_eq!(describe_sensor_id("/lpc/nct6687d/fan/3"), "Mainboard \u{00B7} Fan #3");
        // A device this build has no word for keeps its own segment rather
        // than the whole path.
        assert_eq!(describe_sensor_id("/quantumthing/0/power/1"), "quantumthing \u{00B7} Power #1");
        // And an identifier with no kind in it at least names the device.
        assert_eq!(describe_sensor_id("/intelcpu/0"), "Intel CPU");
    }

    #[test]
    fn a_reading_names_the_sensor_when_there_is_one_and_the_identifier_when_there_is_not() {
        let metric = StackMetric::Sensor("/intelcpu/0/temperature/18".into());
        let sensor = Sensor {
            id: "/intelcpu/0/temperature/18".into(),
            name: "CPU Package".into(),
            hardware: "Intel Core Ultra 9 185H".into(),
            kind: SensorKind::Temperature,
            value: Some(58.0),
        };
        assert_eq!(
            metric.display_name_in(&[sensor]),
            "Intel Core Ultra 9 185H \u{00B7} CPU Package \u{00B7} Temperature"
        );
        assert_eq!(metric.display_name_in(&[]), "Intel CPU \u{00B7} Temperature #18");
    }

    #[test]
    fn an_unnamed_stack_is_untitled_rather_than_numbered() {
        let stack = StackSettings::new(47);
        assert_eq!(stack.display_name(), "Untitled");
    }

    #[test]
    fn an_id_below_one_is_clamped() {
        assert_eq!(StackSettings::new(0).id, 1);
    }
}
