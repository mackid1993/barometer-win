// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The Sensors flyout, ported from SensorsDropdownView.swift.
//
// The hottest reading in the header, then one card per kind - temperatures,
// fans, power, voltage, current - each row carrying a sparkline of the last
// minute and its reading. Temperatures run hottest first; every other kind
// keeps the order the source reported it in, as on the Mac.
//
// Two things the Mac shows are not here, on purpose. Session energy, the
// joules accumulated since the app opened, comes from a counter Windows has
// no counterpart for, and the Mac hides that card when it is empty, so its
// absence is the Mac's own empty state rather than a placeholder. And the
// Mac's duplicate hiding, which folds one probe reported by two interfaces
// into one row, is turned around: LibreHardwareMonitor reports each probe
// once, and a name that repeats - "Temperature" on every NVMe drive - is a
// different probe on different hardware, so the repeat is qualified with the
// hardware rather than dropped.
//
// Nothing here draws. `build` lays elements out in DIPs through the shared
// Builder and the chrome paints them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use barometer_core::sensors::{hottest, Sensor, SensorError, SensorKind};
use barometer_core::weather::models::TemperatureUnit;
use barometer_core::ModuleId;

use super::cpu::latest;
use super::ui::{Accent, Builder, Graph, Ink, Kind, Measure, Style, ROW_H};
use super::{Content, Context, Page};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::model::module_glyph;
use crate::settings_ui::theme::{module_color, Color};

/// How many samples each row's sparkline keeps: the Mac's
/// `history.recent(60)`.
pub const HISTORY: usize = 60;

/// Fractional digits on a reading: SensorSettings.swift's default, and what
/// a panel shows until the store's own `decimal_places` is applied to it.
pub const DEFAULT_DECIMAL_PLACES: usize = 1;

/// The sparkline beside a reading, from SensorReadingRow's frame.
const SPARK_W: f32 = 64.0;
const SPARK_H: f32 = 16.0;

/// The reading column never narrows past this, so rows in one card keep
/// their sparklines in a line however the numbers vary.
const VALUE_MIN_W: f32 = 64.0;

/// What stands in for a reading that is not there.
const DASH: &str = "\u{2014}";

/// One of the Mac's five kinds: its card's title and the color the card is
/// tinted, which are Apple's system colors so the two apps' cards match.
struct Section {
    kind: SensorKind,
    title: &'static str,
    color: Color,
}

/// In the Mac's order.
static SECTIONS: [Section; 5] = [
    Section { kind: SensorKind::Temperature, title: "Temperatures", color: Color(0xFF9500) },
    Section { kind: SensorKind::Fan, title: "Fans", color: Color(0x32ADE6) },
    Section { kind: SensorKind::Power, title: "Power", color: Color(0xFFCC00) },
    Section { kind: SensorKind::Voltage, title: "Voltage", color: Color(0xAF52DE) },
    Section { kind: SensorKind::Current, title: "Current", color: Color(0x00C7BE) },
];

/// "2 temperatures", "1 fan", "1 power reading": the count in words that
/// agree with it. Power, voltage and current are what is measured, not the
/// things counted, so those count readings.
fn count_words(count: usize, kind: &SensorKind) -> String {
    let one = count == 1;
    let noun = match kind {
        SensorKind::Temperature => if one { "temperature" } else { "temperatures" },
        SensorKind::Fan => if one { "fan" } else { "fans" },
        SensorKind::Power => if one { "power reading" } else { "power readings" },
        SensorKind::Voltage => if one { "voltage reading" } else { "voltage readings" },
        SensorKind::Current => if one { "current reading" } else { "current readings" },
        _ => if one { "reading" } else { "readings" },
    };
    format!("{count} {noun}")
}

/// A minute of history for every reading, by sensor id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// Oldest first, never longer than HISTORY.
    lines: HashMap<String, Vec<f32>>,
}

impl History {
    /// Takes one snapshot's readings.
    pub fn observe(&mut self, sensors: &[Sensor]) {
        for sensor in sensors {
            // A reading the source could not take leaves no mark on the
            // line: a fan that cannot be read is not a stopped fan.
            let Some(value) = sensor.value else { continue };
            let line = match self.lines.get_mut(sensor.id.as_str()) {
                Some(line) => line,
                // The identifier is only copied for a sensor never seen
                // before, rather than on every tick for every sensor.
                None => self.lines.entry(sensor.id.clone()).or_default(),
            };
            line.push(value as f32);
            if line.len() > HISTORY {
                line.remove(0);
            }
        }
        // A sensor that left the snapshot takes its history with it, so a
        // source that re-enumerates does not grow the map without bound.
        //
        // Against a set built once rather than by scanning the snapshot for
        // every line held. The scan was quadratic in the number of sensors -
        // ten thousand string comparisons a second at a hundred sensors, and
        // several times that on a desktop - to notice a re-enumeration that
        // happens approximately never.
        if self.lines.len() != sensors.len() {
            let present: std::collections::HashSet<&str> =
                sensors.iter().map(|sensor| sensor.id.as_str()).collect();
            self.lines.retain(|id, _| present.contains(id.as_str()));
        }
    }

    pub fn line(&self, id: &str) -> Option<&[f32]> {
        self.lines.get(id).map(Vec::as_slice)
    }
}

/// What the strip knows, published on every tick: the latest readings, a
/// minute of history for every one of them, and how to spell a temperature.
///
/// The strip's thread keeps one of these, feeds it each pass of the sensors
/// module through `observe` so the sparklines accumulate while the panel is
/// closed, and publishes a clone.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorsSnapshot {
    /// From SensorsModule::sensors, or the reason there are none.
    pub sensors: Vec<Sensor>,
    pub error: Option<SensorError>,
    /// Whether any snapshot has arrived yet, which is the difference between
    /// "discovering" and "nothing found".
    pub sampled: bool,
    pub history: History,
    /// settings.sensors.temperature: the sensors' own unit, not the
    /// weather's.
    pub unit: TemperatureUnit,
    pub decimal_places: usize,
}

impl Default for SensorsSnapshot {
    fn default() -> Self {
        SensorsSnapshot {
            sensors: Vec::new(),
            error: None,
            sampled: false,
            history: History::default(),
            unit: TemperatureUnit::Celsius,
            decimal_places: DEFAULT_DECIMAL_PLACES,
        }
    }
}

impl SensorsSnapshot {
    /// Takes the settings the panel honors: the temperature unit and the
    /// precision.
    pub fn configure(&mut self, unit: TemperatureUnit, decimal_places: usize) {
        self.unit = unit;
        // The Mac clamps to two; a third digit on a die temperature is noise.
        self.decimal_places = decimal_places.min(2);
    }

    /// Takes one snapshot, as the sensors module's worker publishes it: the
    /// readings, or the reason there are none.
    pub fn observe(&mut self, sensors: &[Sensor], error: Option<&SensorError>) {
        self.sampled = true;
        self.sensors = sensors.to_vec();
        self.error = error.cloned();
        self.history.observe(sensors);
    }

    /// The header's second line: how many of each kind, or why there are
    /// none.
    fn subtitle(&self) -> String {
        if let Some(error) = &self.error {
            return short_words(error).to_string();
        }
        if !self.sampled {
            return "Discovering sensors".to_string();
        }
        let counts: Vec<String> = SECTIONS
            .iter()
            .filter_map(|section| {
                let count = self.sensors.iter().filter(|sensor| sensor.kind == section.kind).count();
                (count > 0).then(|| count_words(count, &section.kind))
            })
            .collect();
        if counts.is_empty() {
            "No readings yet".to_string()
        } else {
            counts.join("  \u{00B7}  ")
        }
    }

    /// A reading with its unit, from SensorValueFormatter.string.
    ///
    /// Temperatures are spelled as the strip spells them, whole degrees in
    /// the sensors' own unit; fans are whole revolutions whatever the
    /// precision, as on the Mac, because a tenth of an RPM is not a
    /// reading.
    fn reading(&self, sensor: &Sensor) -> String {
        let Some(value) = sensor.value else {
            return DASH.to_string();
        };
        let places = self.decimal_places;
        match sensor.kind {
            SensorKind::Temperature => self.unit.describe(value),
            SensorKind::Fan => format!("{value:.0} RPM"),
            SensorKind::Power => format!("{value:.places$} W"),
            SensorKind::Voltage => format!("{value:.places$} V"),
            SensorKind::Current => format!("{value:.places$} A"),
            ref other => format!("{value:.places$} {}", other.unit()).trim_end().to_string(),
        }
    }

    /// The last minute of a reading scaled to its own range, once there
    /// are two points to draw a line between.
    fn sparkline(&self, id: &str) -> Option<Vec<f32>> {
        self.history.line(id).filter(|line| line.len() >= 2).map(normalized)
    }
}

/// The Sensors flyout's layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SensorsFlyout {
    pub snapshot: SensorsSnapshot,
    /// One sensor to open on: its hardware's readings only, with its row
    /// marked. A stack's tab for a sensor reading sets it; the module's own
    /// panel never does.
    pub focus: Option<String>,
}

impl SensorsFlyout {
    pub fn new(snapshot: SensorsSnapshot) -> SensorsFlyout {
        SensorsFlyout { snapshot, focus: None }
    }

    /// The focused sensor, while it is among the readings.
    fn focused(&self) -> Option<&Sensor> {
        let id = self.focus.as_deref()?;
        self.snapshot.sensors.iter().find(|sensor| sensor.id == id)
    }

    pub fn title(&self) -> &str {
        "Sensors"
    }

    /// How tall the content is at a width, in DIPs.
    pub fn height(&self, width: f32, measure: Measure) -> f32 {
        let mut b = Builder::new(0.0, 0.0, width, measure);
        self.build(&mut b);
        b.y()
    }

    /// Lays the panel out, top to bottom, in the Mac's order.
    pub fn build(&self, b: &mut Builder) {
        let s = &self.snapshot;
        let accent = Accent::signature(ModuleId::Sensors);
        let focused = self.focused();
        // Opened on one sensor, the panel is that sensor's hardware: the
        // header names the device and carries the sensor's own reading, and
        // the cards hold only that device's readings, the sensor's row
        // marked. That is what a stack's tab for "GPU Core" means - the
        // GPU's sensors, not every sensor in the machine with one row
        // somewhere in it.
        let subtitle = match focused {
            Some(sensor) => sensor.hardware.clone(),
            None => s.subtitle(),
        };
        // The hottest reading anywhere, which is the panel's figure where the
        // strip shows the hottest processor sensor.
        let value = match focused {
            Some(sensor) => s.reading(sensor),
            None => hottest(&s.sensors)
                .and_then(|sensor| sensor.value)
                .map(|celsius| s.unit.describe(celsius))
                .unwrap_or_else(|| DASH.to_string()),
        };
        let title = match focused {
            Some(sensor) => sensor.name.clone(),
            None => self.title().to_string(),
        };
        b.hero_header(
            (module_color(ModuleId::Sensors), module_glyph(ModuleId::Sensors)),
            &title,
            Some(&subtitle),
            Some(&value),
            accent,
        );

        let hardware = focused.map(|sensor| sensor.hardware.clone());
        let mut shown = 0;
        for section in SECTIONS.iter() {
            let group: Vec<&Sensor> = ordered(&s.sensors, &section.kind)
                .into_iter()
                .filter(|sensor| hardware.as_ref().is_none_or(|h| &sensor.hardware == h))
                .collect();
            if group.is_empty() {
                continue;
            }
            shown += group.len();
            let card = b.card_begin(Some(section.color));
            let area = b.section_label(section.title);
            b.chip_at(area, &group.len().to_string(), section.color, None);
            for sensor in &group {
                let selected = focused.is_some_and(|f| f.id == sensor.id);
                sensor_row(
                    b,
                    &display_name(sensor, &group),
                    &s.reading(sensor),
                    s.sparkline(&sensor.id),
                    section.color,
                    selected,
                );
            }
            b.card_end(card);
        }

        if shown == 0 {
            let card = b.card_begin(None);
            empty_state(b, "No sensor readings yet", s.error.as_ref().map(calm_words).as_deref());
            b.card_end(card);
        }
    }
}

/// The Sensors content: the panel's end of the feed, and the layout.
pub struct SensorsContent {
    slot: Arc<Mutex<SensorsSnapshot>>,
    flyout: SensorsFlyout,
}

impl SensorsContent {
    /// Content reading from a feed's slot, as `Flyout::feed` hands it out.
    pub fn new(slot: Arc<Mutex<SensorsSnapshot>>) -> SensorsContent {
        SensorsContent { slot, flyout: SensorsFlyout::default() }
    }
}

impl Content for SensorsContent {
    fn module(&self) -> ModuleId {
        ModuleId::Sensors
    }

    fn build(&mut self, cx: &Context) -> Page {
        if let Some(snapshot) = latest(&self.slot) {
            self.flyout.snapshot = snapshot;
        }
        let mut b = cx.builder();
        self.flyout.build(&mut b);
        Page::finish(b)
    }

    fn tick(&mut self) -> bool {
        self.slot.lock().map(|slot| *slot != self.flyout.snapshot).unwrap_or(false)
    }

    fn focus_sensor(&mut self, id: Option<&str>) {
        self.flyout.focus = id.map(str::to_string);
    }
}

/// The readings of one kind in the order their card shows them: hottest
/// temperature first, with the ones the source could not read at the end,
/// and every other kind as the source listed it.
pub fn ordered<'a>(sensors: &'a [Sensor], kind: &SensorKind) -> Vec<&'a Sensor> {
    let mut group: Vec<&Sensor> = sensors.iter().filter(|sensor| sensor.kind == *kind).collect();
    if *kind == SensorKind::Temperature {
        group.sort_by(|a, b| match (a.value, b.value) {
            (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }
    group
}

/// The name a row shows: the reading's own, unless another reading in the
/// same card shares it on different hardware, in which case the hardware
/// goes first so an ending cut by the row's width still leaves a name.
pub fn display_name(sensor: &Sensor, group: &[&Sensor]) -> String {
    let repeated = group.iter().any(|other| {
        other.id != sensor.id
            && other.hardware != sensor.hardware
            && other.name.eq_ignore_ascii_case(&sensor.name)
    });
    if repeated {
        format!("{}  \u{00B7}  {}", sensor.hardware, sensor.name)
    } else {
        sensor.name.clone()
    }
}

/// A line scaled to its own lowest and highest point, from the Mac's
/// history(for:). A flat line sits halfway rather than on the floor, so a
/// steady reading does not look like a dead one.
pub fn normalized(raw: &[f32]) -> Vec<f32> {
    let minimum = raw.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = raw.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !(maximum > minimum) {
        return raw.iter().map(|_| 0.5).collect();
    }
    raw.iter().map(|value| (value - minimum) / (maximum - minimum)).collect()
}

/// One reading, from SensorReadingRow: the name, a sparkline, and the value
/// right-aligned in a column that never narrows past VALUE_MIN_W.
fn sensor_row(
    b: &mut Builder,
    name: &str,
    value: &str,
    spark: Option<Vec<f32>>,
    color: Color,
    selected: bool,
) {
    let row = Rect::new(b.inner_x(), b.y(), b.inner_w(), ROW_H);
    if selected {
        b.passive(row, Kind::Row { selected: true });
    }
    let value_w = b.text_width(value, Style::Body).max(VALUE_MIN_W);
    let value_x = row.right() - 6.0 - value_w;
    let spark_x = value_x - 10.0 - SPARK_W;
    if let Some(values) = spark {
        b.passive(
            Rect::new(spark_x, row.y + (ROW_H - SPARK_H) / 2.0, SPARK_W, SPARK_H),
            Kind::Graph(Graph::sparkline(values, color)),
        );
    }
    let name_x = row.x + 6.0;
    b.text(Rect::new(name_x, row.y, (spark_x - 6.0 - name_x).max(24.0), ROW_H), name, Style::Body, Ink::Primary, Align::Left);
    b.text(Rect::new(value_x, row.y, value_w, ROW_H), value, Style::Body, Ink::Primary, Align::Right);
    b.advance(ROW_H);
}

/// The empty state, from ContentUnavailableView: the module's tile, a title,
/// and a sentence when there is one, centered.
///
/// The chrome's `unavailable` wants an icon-font glyph and the module marks
/// are chars for a tile, so the tile stands where the Mac's symbol does.
fn empty_state(b: &mut Builder, title: &str, description: Option<&str>) {
    b.gap(12.0);
    let size = 36.0;
    b.passive(
        Rect::new(b.inner_x() + (b.inner_w() - size) / 2.0, b.y(), size, size),
        Kind::Tile {
            color: module_color(ModuleId::Sensors),
            color2: Accent::signature(ModuleId::Sensors).secondary,
            glyph: module_glyph(ModuleId::Sensors),
        },
    );
    b.advance(size + 8.0);
    b.text(Rect::new(b.inner_x(), b.y(), b.inner_w(), 20.0), title, Style::BodyStrong, Ink::Primary, Align::Center);
    b.advance(20.0);
    if let Some(description) = description {
        b.gap(4.0);
        b.wrapped(description, Style::Caption, Ink::Secondary, Align::Center);
    }
    b.gap(12.0);
}

/// Why there are no readings, in the header's few words.
fn short_words(error: &SensorError) -> &'static str {
    match error {
        SensorError::NotConfigured => "No sensor source",
        SensorError::NotRunning => "Sensor source not running",
        _ => "Sensor source unavailable",
    }
}

/// Why there are no readings, as the empty card explains it.
///
/// Calm on purpose, and in the second person, as docs/ui-layouts.md asks of
/// the Sensors empty state: a fresh install with no source is the ordinary
/// first-run state, not a fault, and nothing here is worded as one.
fn calm_words(error: &SensorError) -> String {
    match error {
        SensorError::NotConfigured => {
            "Windows doesn't provide temperatures, fans or voltages to apps, so Barometer reads them \
             from LibreHardwareMonitor. Install it from Settings and readings show up here on their own."
                .to_string()
        }
        SensorError::NotRunning => "The sensor helper is not running. Readings appear when it starts.".to_string(),
        SensorError::Unauthorized => {
            "The sensor source refused the request. Readings that need administrator rights appear \
             when Barometer has them."
                .to_string()
        }
        other => {
            let words = other.to_string();
            let mut chars = words.chars();
            match chars.next() {
                Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
                None => words,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Element, Palette, PANEL_W};
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

    /// Seven DIPs a character at body size, scaled by the style; wrapped by
    /// width when one is given. The same stand-in ui.rs tests with.
    fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    fn laid_out(snapshot: &SensorsSnapshot) -> Vec<Element> {
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        SensorsFlyout::new(snapshot.clone()).build(&mut b);
        b.elements
    }

    fn texts(elements: &[Element]) -> Vec<&str> {
        elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Text { text, .. } | Kind::Wrapped { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn sensor(id: &str, hardware: &str, name: &str, kind: SensorKind, value: Option<f64>) -> Sensor {
        Sensor { id: id.into(), name: name.into(), hardware: hardware.into(), kind, value }
    }

    fn machine() -> Vec<Sensor> {
        vec![
            sensor("/amdcpu/0/temperature/0", "AMD Ryzen 9", "CPU Package", SensorKind::Temperature, Some(61.0)),
            sensor("/gpu-nvidia/0/temperature/0", "NVIDIA RTX", "GPU Core", SensorKind::Temperature, Some(72.5)),
            sensor("/gpu-nvidia/0/fan/0", "NVIDIA RTX", "GPU Fan", SensorKind::Fan, Some(1234.6)),
            sensor("/amdcpu/0/power/0", "AMD Ryzen 9", "Package", SensorKind::Power, Some(45.26)),
            sensor("/amdcpu/0/load/0", "AMD Ryzen 9", "CPU Total", SensorKind::Load, Some(12.0)),
        ]
    }

    #[test]
    fn opened_on_a_sensor_the_panel_is_that_hardwares_and_marks_its_row() {
        // A stack's GPU tab: the GPU's readings, the sensor named in the
        // header, its row marked - and nothing of the processor's.
        let mut snapshot = SensorsSnapshot::default();
        snapshot.observe(&machine(), None);
        let mut flyout = SensorsFlyout::new(snapshot);
        flyout.focus = Some("/gpu-nvidia/0/temperature/0".into());
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        flyout.build(&mut b);
        let names = texts(&b.elements);
        assert!(names.contains(&"GPU Core"));
        assert!(names.contains(&"NVIDIA RTX"));
        assert!(names.contains(&"GPU Fan"));
        assert!(!names.contains(&"CPU Package"));
        assert!(!names.contains(&"CPU Total"));
        assert!(b.elements.iter().any(|e| matches!(e.kind, Kind::Row { selected: true })));
    }

    #[test]
    fn readings_group_into_the_macs_five_kinds_in_its_order() {
        let mut snapshot = SensorsSnapshot::default();
        snapshot.observe(
            &[
                sensor("/a/current/0", "PSU", "12V Rail", SensorKind::Current, Some(2.5)),
                sensor("/a/fan/0", "Board", "Chassis", SensorKind::Fan, Some(800.0)),
                sensor("/a/temperature/0", "Board", "System", SensorKind::Temperature, Some(40.0)),
                sensor("/a/load/0", "Board", "Load", SensorKind::Load, Some(1.0)),
            ],
            None,
        );
        let elements = laid_out(&snapshot);
        let labels: Vec<&str> =
            texts(&elements).into_iter().filter(|t| ["Temperatures", "Fans", "Power", "Voltage", "Current", "LOAD"].contains(t)).collect();
        assert_eq!(labels, vec!["Temperatures", "Fans", "Current"]);
        // Each card counts its rows in a chip of its own color.
        let chips: Vec<(&str, Color)> = elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Chip { text, color, .. } => Some((text.as_str(), *color)),
                _ => None,
            })
            .collect();
        assert_eq!(chips, vec![("1", Color(0xFF9500)), ("1", Color(0x32ADE6)), ("1", Color(0x00C7BE))]);
        assert!(!texts(&elements).contains(&"Load"));
    }

    #[test]
    fn temperatures_run_hottest_first_with_unreadable_ones_last() {
        let sensors = vec![
            sensor("/a/0", "h", "warm", SensorKind::Temperature, Some(45.0)),
            sensor("/a/1", "h", "blank", SensorKind::Temperature, None),
            sensor("/a/2", "h", "hot", SensorKind::Temperature, Some(80.0)),
            sensor("/a/3", "h", "mild", SensorKind::Temperature, Some(60.0)),
            sensor("/b/0", "h", "second fan", SensorKind::Fan, Some(500.0)),
            sensor("/b/1", "h", "first fan", SensorKind::Fan, Some(900.0)),
        ];
        let names = |kind: SensorKind| -> Vec<String> {
            ordered(&sensors, &kind).iter().map(|s| s.name.clone()).collect()
        };
        assert_eq!(names(SensorKind::Temperature), vec!["hot", "mild", "warm", "blank"]);
        // Fans keep the source's order, not the numeric one.
        assert_eq!(names(SensorKind::Fan), vec!["second fan", "first fan"]);
        assert!(ordered(&sensors, &SensorKind::Voltage).is_empty());
    }

    #[test]
    fn a_name_that_repeats_across_hardware_is_qualified_with_the_hardware() {
        let sensors = vec![
            sensor("/nvme/0/temperature/0", "Samsung SSD 990 PRO", "Temperature", SensorKind::Temperature, Some(41.0)),
            sensor("/nvme/1/temperature/0", "WD Black SN850", "temperature", SensorKind::Temperature, Some(38.0)),
            sensor("/amdcpu/0/temperature/0", "AMD Ryzen 9", "CPU Package", SensorKind::Temperature, Some(61.0)),
        ];
        let group = ordered(&sensors, &SensorKind::Temperature);
        let names: Vec<String> = group.iter().map(|s| display_name(s, &group)).collect();
        assert_eq!(names[0], "CPU Package");
        assert_eq!(names[1], "Samsung SSD 990 PRO  \u{00B7}  Temperature");
        assert_eq!(names[2], "WD Black SN850  \u{00B7}  temperature");
        // The same name on the same hardware is one probe, not a repeat.
        let twin = vec![
            sensor("/a/0", "Board", "System", SensorKind::Temperature, Some(40.0)),
            sensor("/a/1", "Board", "System", SensorKind::Temperature, Some(41.0)),
        ];
        let group = ordered(&twin, &SensorKind::Temperature);
        assert_eq!(display_name(group[0], &group), "System");
    }

    #[test]
    fn a_sparkline_spans_its_own_range_and_a_flat_line_sits_halfway() {
        assert_eq!(normalized(&[40.0, 50.0, 60.0]), vec![0.0, 0.5, 1.0]);
        assert_eq!(normalized(&[50.0, 50.0, 50.0]), vec![0.5, 0.5, 0.5]);
        assert_eq!(normalized(&[50.0]), vec![0.5]);
        assert!(normalized(&[]).is_empty());
    }

    #[test]
    fn history_keeps_a_minute_per_sensor_and_forgets_sensors_that_leave() {
        let mut history = History::default();
        for i in 0..(HISTORY + 5) {
            history.observe(&[sensor("/a/0", "h", "n", SensorKind::Temperature, Some(i as f64))]);
        }
        let line = history.line("/a/0").unwrap();
        assert_eq!(line.len(), HISTORY);
        assert_eq!(line[0], 5.0);
        history.observe(&[sensor("/b/0", "h", "n", SensorKind::Fan, Some(1.0))]);
        assert!(history.line("/a/0").is_none());
        assert_eq!(history.line("/b/0").map(<[f32]>::to_vec), Some(vec![1.0]));
        // A reading that could not be taken leaves the line alone.
        history.observe(&[sensor("/b/0", "h", "n", SensorKind::Fan, None)]);
        assert_eq!(history.line("/b/0").map(<[f32]>::to_vec), Some(vec![1.0]));
    }

    #[test]
    fn a_row_gets_a_sparkline_only_once_there_is_a_line_to_draw() {
        let mut snapshot = SensorsSnapshot::default();
        let mut sensors = machine();
        sensors.push(sensor("/intelcpu/0/temperature/1", "Intel", "Core #1", SensorKind::Temperature, None));
        snapshot.observe(&sensors, None);
        assert_eq!(laid_out(&snapshot).iter().filter(|e| matches!(e.kind, Kind::Graph(_))).count(), 0);
        snapshot.observe(&sensors, None);
        let elements = laid_out(&snapshot);
        // Four readable sensors of the five kinds shown; the load is not a
        // card and the blank core has nothing to plot.
        assert_eq!(elements.iter().filter(|e| matches!(e.kind, Kind::Graph(_))).count(), 4);
        let texts = texts(&elements);
        assert!(texts.contains(&"Core #1"));
        assert!(texts.contains(&DASH));
    }

    #[test]
    fn readings_carry_their_units_and_the_chosen_precision() {
        let mut snapshot = SensorsSnapshot::default();
        let fan = sensor("/a/fan/0", "h", "n", SensorKind::Fan, Some(1234.6));
        let power = sensor("/a/power/0", "h", "n", SensorKind::Power, Some(45.26));
        let volts = sensor("/a/voltage/0", "h", "n", SensorKind::Voltage, Some(1.234));
        let amps = sensor("/a/current/0", "h", "n", SensorKind::Current, Some(2.0));
        let heat = sensor("/a/temperature/0", "h", "n", SensorKind::Temperature, Some(55.0));
        let blank = sensor("/a/temperature/1", "h", "n", SensorKind::Temperature, None);
        assert_eq!(snapshot.reading(&fan), "1235 RPM");
        assert_eq!(snapshot.reading(&power), "45.3 W");
        assert_eq!(snapshot.reading(&volts), "1.2 V");
        assert_eq!(snapshot.reading(&amps), "2.0 A");
        assert_eq!(snapshot.reading(&blank), DASH);
        assert_eq!(snapshot.reading(&heat), "55\u{00B0}C");
        snapshot.configure(TemperatureUnit::Fahrenheit, 2);
        assert_eq!(snapshot.reading(&heat), "131\u{00B0}F", "the sensors' own unit, not the weather's");
        assert_eq!(snapshot.reading(&volts), "1.23 V");
        assert_eq!(snapshot.reading(&fan), "1235 RPM", "fans are whole revolutions at any precision");
        snapshot.configure(TemperatureUnit::Celsius, 9);
        assert_eq!(snapshot.reading(&amps), "2.00 A", "precision is clamped to two");
    }

    #[test]
    fn the_hero_shows_the_hottest_reading_anywhere_and_counts_each_kind() {
        let mut snapshot = SensorsSnapshot::default();
        snapshot.observe(&machine(), None);
        assert_eq!(snapshot.subtitle(), "2 temperatures  \u{00B7}  1 fan  \u{00B7}  1 power reading");
        let elements = laid_out(&snapshot);
        let headline = elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { style: Style::Title, .. }))
            .expect("a headline");
        // The GPU is hotter than the processor, and the panel says so even
        // though the strip would show the processor.
        let gpu = TemperatureUnit::Celsius.describe(72.5);
        assert!(matches!(&headline.kind, Kind::Text { text, .. } if *text == gpu));
    }

    #[test]
    fn before_the_first_snapshot_the_header_says_it_is_discovering() {
        let snapshot = SensorsSnapshot::default();
        assert_eq!(snapshot.subtitle(), "Discovering sensors");
        let elements = laid_out(&snapshot);
        let texts = texts(&elements);
        assert!(texts.contains(&"No sensor readings yet"));
        assert!(texts.contains(&DASH));
        let mut snapshot = snapshot;
        snapshot.observe(&[], None);
        assert_eq!(snapshot.subtitle(), "No readings yet");
    }

    #[test]
    fn without_a_source_the_panel_says_so_calmly() {
        let mut snapshot = SensorsSnapshot::default();
        snapshot.observe(&[], Some(&SensorError::NotConfigured));
        assert_eq!(snapshot.subtitle(), "No sensor source");
        let elements = laid_out(&snapshot);
        let texts = texts(&elements);
        assert!(texts.contains(&"No sensor readings yet"));
        assert!(texts.iter().any(|t| t.starts_with("Windows doesn't provide temperatures")));
        // Nothing is colored as a fault: no caution, no critical.
        assert!(!elements.iter().any(|e| matches!(&e.kind, Kind::Text { ink: Ink::Caution | Ink::Critical, .. })));
        assert!(!elements.iter().any(|e| matches!(e.kind, Kind::Chip { .. })));
        // An error without its own sentence is still a sentence.
        assert_eq!(
            calm_words(&SensorError::Malformed("bad json".into())),
            "Unexpected response: bad json."
        );
        assert_eq!(short_words(&SensorError::Transport("x".into())), "Sensor source unavailable");
    }

    #[test]
    fn each_reading_adds_one_row_of_the_documented_height() {
        let mut one = SensorsSnapshot::default();
        one.observe(&machine()[..1], None);
        let mut two = SensorsSnapshot::default();
        two.observe(&machine()[..2], None);
        let a = SensorsFlyout::new(one).height(356.0, &measure);
        let b = SensorsFlyout::new(two).height(356.0, &measure);
        assert!(((b - a) - ROW_H).abs() < 1e-3, "{a} -> {b}");
    }

    #[test]
    fn the_value_column_never_narrows_past_the_sparkline_it_lines_up_with() {
        let mut snapshot = SensorsSnapshot::default();
        snapshot.observe(&machine(), None);
        snapshot.observe(&machine(), None);
        let elements = laid_out(&snapshot);
        let sparks: Vec<Rect> = elements.iter().filter(|e| matches!(e.kind, Kind::Graph(_))).map(|e| e.rect).collect();
        assert!(sparks.iter().all(|s| s.w == SPARK_W && s.h == SPARK_H));
        // Every reading in a card is short, so every sparkline sits at the
        // same x: the column is held at its minimum, not to each number.
        let temperatures = &sparks[..2];
        assert_eq!(temperatures[0].x, temperatures[1].x);
        assert!((temperatures[0].right() + 10.0 + VALUE_MIN_W + 6.0 - (12.0 + 356.0 - 12.0)).abs() < 1e-3);
    }

    #[test]
    fn the_content_follows_its_feed() {
        let slot = Arc::new(Mutex::new(SensorsSnapshot::default()));
        let mut content = SensorsContent::new(Arc::clone(&slot));
        let palette = Palette::resolve(Theme::resolve(true, DEFAULT_ACCENT));
        let cx = Context {
            width: PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Sensors),
            measure: &measure,
            hover: None,
            now_unix: 0,
        };
        assert_eq!(content.module(), ModuleId::Sensors);
        assert!(!content.tick());
        let page = content.build(&cx);
        assert!(texts(&page.elements).contains(&"Discovering sensors"));
        slot.lock().unwrap().observe(&machine(), None);
        assert!(content.tick());
        let page = content.build(&cx);
        assert!(!content.tick());
        assert!(texts(&page.elements).contains(&"Temperatures"));
        assert!(texts(&page.elements).contains(&"GPU Core"));
    }

    /// Paints the panel for a person to look at; see `flyout::render`.
    #[test]
    #[ignore]
    fn render_the_panel_to_bitmaps() {
        let slot = Arc::new(Mutex::new(SensorsSnapshot::default()));
        for n in 0..30u32 {
            // Readings that move, so the sparklines have a shape to show.
            let drift = ((n as f64) * 0.5).sin() * 4.0;
            let readings: Vec<Sensor> = machine()
                .into_iter()
                .map(|mut sensor| {
                    sensor.value = sensor.value.map(|value| value + drift);
                    sensor
                })
                .collect();
            slot.lock().unwrap().observe(&readings, None);
        }
        let mut content = SensorsContent::new(Arc::clone(&slot));
        content.tick();
        for light in [false, true] {
            crate::flyout::render::to_bitmap(&mut content, "sensors", light, 0);
        }
    }
}
