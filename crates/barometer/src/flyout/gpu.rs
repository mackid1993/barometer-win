// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The graphics flyout, ported from GPUDropdownView.swift.
//
// What the Mac shows, in its order: a hero header with the adapter's name
// and its headline figure; a History card with the range picker and a taller
// graph than the CPU's; a Utilization card of three labeled bars; a Memory
// card with a capsule and two tiles; and a Hardware card of three tiles for
// frequency, power and temperature.
//
// Three departures. The Swift's Device, Renderer and Tiler rows are the
// parts of an Apple GPU; Windows publishes utilization per engine type - 3D,
// Copy, Video Decode and so on - and modules/gpu.rs takes the busiest as the
// headline, which is what Task Manager does, so the Utilization card is one
// row per engine type, busiest first, and the headline is the first of
// them. "Allocated" becomes "Dedicated", Task Manager's word for the memory
// on the adapter. And frequency, power and temperature are not Windows'
// figures to give: modules/gpu.rs says they arrive through the sensor helper,
// so the tiles show a dash until the strip's thread finds them among the
// sensors module's readings, rather than a guess.

use std::sync::{Arc, Mutex};

use barometer_core::format;
use barometer_core::sensors::{self, Sensor, SensorKind};
use barometer_core::weather::models::TemperatureUnit;
use barometer_core::ModuleId;

use super::cpu::{history_graph, latest, percent1, range_picker, share_bar, HistoryRange, Sample, BAR_H, DASH};
use super::ui::{Accent, Builder, Id, Ink, Measure, Tile, TileIcon};
use super::{Content, Context, Page, Response};
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::model::module_glyph;
use crate::settings_ui::theme;

/// One engine type's share of the interval, summed over every process
/// using it, as modules/gpu.rs sums them.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineLoad {
    /// The type as the counter names it: "3D", "Copy", "VideoDecode".
    pub name: String,
    /// Zero to one.
    pub load: f32,
}

/// Everything the panel says, from GPUSample.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuSnapshot {
    /// Whether Windows publishes GPU counters here at all. False is a
    /// machine with no adapter or a stripped install, and the panel says
    /// so instead of waiting forever.
    pub supported: bool,
    pub name: Option<String>,
    /// The busiest engine type's share, which is the strip's figure.
    pub load: Option<f32>,
    pub engines: Vec<EngineLoad>,
    /// Dedicated adapter memory in use, and how much there is.
    pub memory_used: Option<u64>,
    pub memory_total: Option<u64>,
    /// From the sensors module's readings for the adapter: its Clock, Power
    /// and Temperature sensors, when a sensor source is running.
    pub frequency_mhz: Option<f64>,
    pub power_watts: Option<f64>,
    pub temperature_c: Option<f64>,
    /// settings.sensors.temperature: the hardware unit, not the weather's,
    /// as the Swift reads the sensor unit for the same tile.
    pub unit: TemperatureUnit,
    /// The headline share at each sample of the last day, oldest first,
    /// kept with cpu::remember. The picker chooses how much of it the graph
    /// shows.
    pub history: Vec<Sample>,
}

impl Default for GpuSnapshot {
    /// A snapshot nobody has filled in is one that is still waiting, not
    /// one from a machine without a GPU.
    fn default() -> GpuSnapshot {
        GpuSnapshot {
            supported: true,
            name: None,
            load: None,
            engines: Vec::new(),
            memory_used: None,
            memory_total: None,
            frequency_mhz: None,
            power_watts: None,
            temperature_c: None,
            unit: TemperatureUnit::Celsius,
            history: Vec::new(),
        }
    }
}

impl GpuSnapshot {
    /// The engines with the busiest first, which is the order the eye
    /// looks for and the order the headline was chosen in.
    /// Takes the adapter's clock, power and temperature from the sensor
    /// source's readings.
    ///
    /// The source names hardware the way the driver does, which is the
    /// name the kernel gave the adapter, so the match is by name; when the
    /// source spells it differently and there is only one GPU among its
    /// readings, that one is taken. Among a GPU's several sensors of a kind
    /// the core's is the headline - "GPU Core" clock and temperature, the
    /// package's power - as Task Manager and the Swift both show.
    pub fn adopt_sensors(&mut self, sensors: &[Sensor]) {
        let adapter = self.name.as_deref();
        let pick = |kind, preferred: &[&str]| sensors::gpu_reading(sensors, adapter, kind, preferred);
        self.frequency_mhz = pick(SensorKind::Clock, &sensors::GPU_CLOCK);
        self.power_watts = pick(SensorKind::Power, &sensors::GPU_POWER);
        self.temperature_c = pick(SensorKind::Temperature, &sensors::GPU_TEMPERATURE);
    }

    /// The engines worth a row, busiest first.
    ///
    /// A driver publishes a dozen engine types and most sit at zero all
    /// day - security, overlay, a numbered one nobody can name. The rows
    /// are the ones doing something, plus the ones Task Manager always
    /// shows so a quiet card still reads as a card, and never more than
    /// six.
    fn engines_by_load(&self) -> Vec<&EngineLoad> {
        const ALWAYS: [&str; 4] = ["3D", "Copy", "VideoDecode", "VideoEncode"];
        const MOST: usize = 6;
        let mut engines: Vec<&EngineLoad> = self
            .engines
            .iter()
            .filter(|e| e.load > 0.0 || ALWAYS.iter().any(|a| a.eq_ignore_ascii_case(&e.name)))
            .collect();
        engines.sort_by(|a, b| b.load.total_cmp(&a.load).then_with(|| a.name.cmp(&b.name)));
        engines.truncate(MOST);
        engines
    }
}

/// The graphics flyout's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuFlyout {
    pub snapshot: GpuSnapshot,
    pub range: HistoryRange,
}

impl GpuFlyout {
    pub fn new(snapshot: GpuSnapshot) -> GpuFlyout {
        GpuFlyout { snapshot, range: HistoryRange::default() }
    }

    pub fn title(&self) -> &'static str {
        "GPU"
    }

    /// How tall the content is at a width, in DIPs: what the builder
    /// walked, the gap under the last card included, as Page::finish
    /// counts it.
    pub fn height(&self, width: f32, measure: Measure<'_>) -> f32 {
        let mut b = Builder::new(0.0, 0.0, width, measure);
        self.build(&mut b);
        b.y()
    }

    /// A press on one of the panel's controls: true when the panel changed
    /// and wants laying out again.
    pub fn activate(&mut self, id: Id) -> bool {
        match HistoryRange::from_id(id) {
            Some(range) if range != self.range => {
                self.range = range;
                true
            }
            _ => false,
        }
    }

    /// Lays the whole panel out, in the Swift's order. Every card stands
    /// whether or not there is a sample, as the Swift's do, with dashes
    /// where the figures would be.
    pub fn build(&self, b: &mut Builder<'_>) {
        let s = &self.snapshot;
        let accent = Accent::signature(ModuleId::Gpu);
        let value = s.load.map(percent1);
        b.hero_header(
            (theme::module_color(ModuleId::Gpu), module_glyph(ModuleId::Gpu)),
            self.title(),
            Some(self.subtitle()),
            Some(value.as_deref().unwrap_or(DASH)),
            accent,
        );

        let card = b.card_begin(Some(accent.primary));
        let trailing = b.section_label("History");
        range_picker(b, trailing, self.range, accent);
        b.graph(96.0, history_graph(&s.history, self.range.seconds(), accent));
        b.card_end(card);

        let card = b.card_begin(None);
        b.section_label("Utilization");
        let engines = s.engines_by_load();
        if engines.is_empty() {
            utilization_row(b, "Busiest engine", s.load, accent);
        } else {
            for engine in engines {
                utilization_row(b, &engine_title(&engine.name), Some(engine.load), accent);
            }
        }
        b.card_end(card);

        let card = b.card_begin(None);
        b.section_label("Memory");
        match (s.memory_used, s.memory_total) {
            (Some(used), Some(total)) if total > 0 => {
                b.capsule(used.min(total) as f32 / total as f32, accent, BAR_H);
                b.gap(8.0);
                b.stat_tiles(vec![
                    Tile {
                        icon: TileIcon::None,
                        label: "In use".to_string(),
                        value: format::bytes(used),
                        tint: Ink::Custom(accent.primary),
                    },
                    Tile {
                        icon: TileIcon::None,
                        label: "Dedicated".to_string(),
                        value: format::bytes(total),
                        tint: Ink::Custom(accent.secondary),
                    },
                ]);
            }
            // An adapter with no memory of its own - integrated graphics -
            // still has a figure for what it is using of the system's.
            (Some(used), _) => b.metric_row(None, "In use", &format::bytes(used), Ink::Secondary),
            _ => b.caption("Unavailable"),
        }
        b.card_end(card);

        // The Swift's three tiles sit in one row; the vocabulary's grid is
        // two across, so the third wraps, as the Wi-Fi tiles do beside it.
        let card = b.card_begin(None);
        b.section_label("Hardware");
        b.stat_tiles(vec![
            tile("Frequency", frequency(s.frequency_mhz)),
            tile("Power", power(s.power_watts)),
            tile("Temperature", temperature(s.temperature_c, s.unit)),
        ]);
        b.card_end(card);
    }

    /// The adapter's name under the title, or why there is not one.
    fn subtitle(&self) -> &str {
        let s = &self.snapshot;
        if !s.supported {
            return "No GPU counters on this machine";
        }
        s.name.as_deref().unwrap_or("Waiting for the first sample")
    }
}

/// The graphics content: the panel's end of the feed, and the layout.
pub struct GpuContent {
    slot: Arc<Mutex<GpuSnapshot>>,
    flyout: GpuFlyout,
}

impl GpuContent {
    /// Content reading from a feed's slot, as `Flyout::feed` hands it out.
    pub fn new(slot: Arc<Mutex<GpuSnapshot>>) -> GpuContent {
        GpuContent { slot, flyout: GpuFlyout::new(GpuSnapshot::default()) }
    }
}

impl Content for GpuContent {
    fn module(&self) -> ModuleId {
        ModuleId::Gpu
    }

    fn build(&mut self, cx: &Context) -> Page {
        if let Some(snapshot) = latest(&self.slot) {
            self.flyout.snapshot = snapshot;
        }
        let mut b = cx.builder();
        self.flyout.build(&mut b);
        Page::finish(b)
    }

    fn activate(&mut self, id: Id) -> Response {
        if self.flyout.activate(id) {
            Response::Relayout
        } else {
            Response::None
        }
    }

    fn tick(&mut self) -> bool {
        self.slot.lock().map(|slot| *slot != self.flyout.snapshot).unwrap_or(false)
    }
}

// MARK: - Utilization

/// Under a utilization row's bar, before the next row.
const UTILIZATION_GAP: f32 = 8.0;

/// A label, a figure and a bar, from UtilizationRow: the vocabulary's
/// metric row with the share drawn under it on the same insets.
fn utilization_row(b: &mut Builder<'_>, label: &str, load: Option<f32>, accent: Accent) {
    let figure = load.map(percent1).unwrap_or_else(|| DASH.to_string());
    b.metric_row(None, label, &figure, Ink::Secondary);
    let bar = Rect::new(b.inner_x() + 6.0, b.y(), b.inner_w() - 12.0, BAR_H);
    share_bar(b, bar, load.unwrap_or(0.0), accent, true);
    b.advance(BAR_H + UTILIZATION_GAP);
}

fn tile(label: &str, value: String) -> Tile {
    Tile { icon: TileIcon::None, label: label.to_string(), value, tint: Ink::Secondary }
}

/// The engine type as a label: the counter's "VideoDecode" is "Video
/// Decode" to a person, and "3D" is left alone.
pub fn engine_title(name: &str) -> String {
    let mut title = String::with_capacity(name.len() + 2);
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_uppercase() && previous_lower {
            title.push(' ');
        }
        title.push(c);
        previous_lower = c.is_lowercase();
    }
    title
}

// MARK: - Hardware

/// "1450 MHz", the Swift's "%.0f MHz".
pub fn frequency(mhz: Option<f64>) -> String {
    mhz.map(|value| format!("{value:.0} MHz")).unwrap_or_else(|| DASH.to_string())
}

/// "23.50 W", the Swift's "%.2f W".
pub fn power(watts: Option<f64>) -> String {
    watts.map(|value| format!("{value:.2} W")).unwrap_or_else(|| DASH.to_string())
}

/// "61°C" or "142°F", spelled as the strip spells a hardware temperature.
pub fn temperature(celsius: Option<f64>, unit: TemperatureUnit) -> String {
    celsius.map(|celsius| unit.describe(celsius)).unwrap_or_else(|| DASH.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Element, Kind, Palette, Style, CARD_GAP, CARD_PAD, PANEL_PAD, PANEL_W, ROW_H, TILE_GAP, TILE_H};
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

    /// Seven DIPs a character at body size, scaled by the style, as the
    /// vocabulary's own tests measure.
    fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    const COLUMN_W: f32 = PANEL_W - 2.0 * PANEL_PAD;

    /// A sample every two seconds for `count` samples, ending at `end`.
    fn samples(count: usize, end: i64) -> Vec<Sample> {
        (0..count)
            .map(|n| Sample { at_unix: end - 2 * (count as i64 - 1 - n as i64), value: (n % 10) as f32 / 10.0 })
            .collect()
    }

    fn full() -> GpuSnapshot {
        GpuSnapshot {
            name: Some("NVIDIA GeForce RTX 4080".to_string()),
            load: Some(0.63),
            engines: vec![
                EngineLoad { name: "Copy".to_string(), load: 0.05 },
                EngineLoad { name: "3D".to_string(), load: 0.63 },
                EngineLoad { name: "VideoDecode".to_string(), load: 0.2 },
            ],
            memory_used: Some(6 * 1024 * 1024 * 1024),
            memory_total: Some(16 * 1024 * 1024 * 1024),
            frequency_mhz: Some(2_505.4),
            power_watts: Some(187.25),
            temperature_c: Some(61.4),
            history: samples(30, 1_000_000),
            ..Default::default()
        }
    }

    fn sensor(id: &str, hardware: &str, name: &str, kind: SensorKind, value: f64) -> Sensor {
        Sensor { id: id.into(), name: name.into(), hardware: hardware.into(), kind, value: Some(value) }
    }

    #[test]
    fn the_adapters_sensors_are_taken_by_name_with_the_core_ahead_of_the_rest() {
        let sensors = vec![
            sensor("/gpu-nvidia/0/clock/1", "NVIDIA GeForce RTX 4080", "GPU Memory", SensorKind::Clock, 10_000.0),
            sensor("/gpu-nvidia/0/clock/0", "NVIDIA GeForce RTX 4080", "GPU Core", SensorKind::Clock, 2_505.0),
            sensor("/gpu-nvidia/0/temperature/1", "NVIDIA GeForce RTX 4080", "GPU Hot Spot", SensorKind::Temperature, 75.0),
            sensor("/gpu-nvidia/0/temperature/0", "NVIDIA GeForce RTX 4080", "GPU Core", SensorKind::Temperature, 61.0),
            sensor("/gpu-nvidia/0/power/0", "NVIDIA GeForce RTX 4080", "GPU Package", SensorKind::Power, 187.0),
            sensor("/intelcpu/0/temperature/0", "Intel Core i9", "CPU Package", SensorKind::Temperature, 90.0),
        ];
        let mut s = GpuSnapshot { name: Some("nvidia geforce rtx 4080 ".to_string()), ..Default::default() };
        s.adopt_sensors(&sensors);
        assert_eq!(s.frequency_mhz, Some(2_505.0));
        assert_eq!(s.temperature_c, Some(61.0));
        assert_eq!(s.power_watts, Some(187.0));
    }

    #[test]
    fn a_lone_gpu_among_the_readings_is_taken_when_the_names_disagree() {
        let sensors = vec![
            sensor("/gpu-intel/0/temperature/0", "Intel(R) Arc(TM) Graphics", "GPU Core", SensorKind::Temperature, 48.0),
            sensor("/intelcpu/0/temperature/0", "Intel Core Ultra 7", "CPU Package", SensorKind::Temperature, 70.0),
        ];
        let mut s = GpuSnapshot { name: Some("Intel Arc Graphics".to_string()), ..Default::default() };
        s.adopt_sensors(&sensors);
        assert_eq!(s.temperature_c, Some(48.0));
        assert_eq!(s.frequency_mhz, None);
    }

    #[test]
    fn two_gpus_with_names_the_source_spells_differently_take_nothing_rather_than_the_wrong_one() {
        let sensors = vec![
            sensor("/gpu-intel/0/temperature/0", "Intel Arc", "GPU Core", SensorKind::Temperature, 48.0),
            sensor("/gpu-nvidia/0/temperature/0", "NVIDIA RTX", "GPU Core", SensorKind::Temperature, 61.0),
        ];
        let mut s = GpuSnapshot { name: Some("Some Other Name".to_string()), ..Default::default() };
        s.adopt_sensors(&sensors);
        assert_eq!(s.temperature_c, None);
    }

    fn layout(flyout: &GpuFlyout) -> Vec<Element> {
        let mut b = Builder::new(PANEL_PAD, PANEL_PAD, COLUMN_W, &measure);
        flyout.build(&mut b);
        b.elements
    }

    fn cards(elements: &[Element]) -> Vec<Rect> {
        elements.iter().filter(|e| matches!(e.kind, Kind::Card { .. })).map(|e| e.rect).collect()
    }

    fn texts<'a>(elements: &'a [Element]) -> Vec<&'a str> {
        elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Text { text, .. } | Kind::Wrapped { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The glowing bars inside the utilization card, which is the second.
    fn utilization_bars(elements: &[Element]) -> Vec<Rect> {
        let card = cards(elements)[1];
        elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Capsule { glow: true, .. }) && card.contains(e.rect.x, e.rect.y))
            .map(|e| e.rect)
            .collect()
    }

    #[test]
    fn the_cards_come_in_the_macs_order_and_stack_without_overlapping() {
        let elements = layout(&GpuFlyout::new(full()));
        let cards = cards(&elements);
        assert_eq!(cards.len(), 4, "history, utilization, memory, hardware");
        for pair in cards.windows(2) {
            assert!(pair[1].y >= pair[0].bottom(), "{pair:?}");
        }
        let labels: Vec<&str> = texts(&elements)
            .into_iter()
            .filter(|t| ["HISTORY", "UTILIZATION", "MEMORY", "HARDWARE"].contains(t))
            .collect();
        assert_eq!(labels, ["HISTORY", "UTILIZATION", "MEMORY", "HARDWARE"]);
        // The graph is the Swift's 96, taller than the CPU's, plotted by time.
        let graph = elements.iter().find(|e| matches!(e.kind, Kind::Graph(_))).unwrap();
        assert_eq!(graph.rect.h, 96.0);
        assert!(matches!(&graph.kind, Kind::Graph(g) if g.values.len() == 30 && g.positions.is_some()));
    }

    #[test]
    fn engines_are_listed_busiest_first_and_the_headline_is_the_busiest() {
        let elements = layout(&GpuFlyout::new(full()));
        let texts = texts(&elements);
        let rows: Vec<&str> = texts.iter().copied().filter(|t| ["3D", "Video Decode", "Copy"].contains(t)).collect();
        assert_eq!(rows, ["3D", "Video Decode", "Copy"]);
        assert!(texts.contains(&"63.0%"));
        // Three rows, each a metric row with a bar under it on the row's
        // insets, at the same track height every other panel's bars use.
        let bars = utilization_bars(&elements);
        assert_eq!(bars.len(), 3);
        assert!((bars[1].y - bars[0].y - (ROW_H + BAR_H + UTILIZATION_GAP)).abs() < 1e-3);
        assert_eq!(bars[0].h, BAR_H);
        assert!((bars[0].x - (PANEL_PAD + CARD_PAD + 6.0)).abs() < 1e-3);
        assert!((bars[0].right() - (PANEL_PAD + COLUMN_W - CARD_PAD - 6.0)).abs() < 1e-3);
    }

    #[test]
    fn idle_engines_nobody_can_name_are_left_out_and_the_rows_stop_at_six() {
        let mut flyout = GpuFlyout::new(full());
        flyout.snapshot.engines = ["3D", "Copy", "VideoDecode", "VideoEncode", "Security", "GSC", "OFA_0", "LegacyOverlay", "Compute", "VR"]
            .iter()
            .map(|name| EngineLoad { name: name.to_string(), load: 0.0 })
            .collect();
        flyout.snapshot.engines[8].load = 0.4; // Compute is busy
        let elements = layout(&flyout);
        let rows: Vec<&str> = texts(&elements)
            .into_iter()
            .filter(|t| ["3D", "Copy", "Video Decode", "Video Encode", "Security", "GSC", "OFA_0", "Legacy Overlay", "Compute", "VR"].contains(t))
            .collect();
        assert_eq!(rows, vec!["Compute", "3D", "Copy", "Video Decode", "Video Encode"]);
        // Everything busy: the six busiest.
        for engine in flyout.snapshot.engines.iter_mut() {
            engine.load = 0.1;
        }
        assert_eq!(utilization_bars(&layout(&flyout)).len(), 6);
    }

    #[test]
    fn without_engine_figures_there_is_one_row_for_the_busiest() {
        let mut flyout = GpuFlyout::new(full());
        flyout.snapshot.engines.clear();
        let elements = layout(&flyout);
        assert!(texts(&elements).contains(&"Busiest engine"));
        assert_eq!(utilization_bars(&elements).len(), 1);
        // And with nothing at all, the row dashes and its bar is empty.
        flyout.snapshot.load = None;
        let elements = layout(&flyout);
        let bar = utilization_bars(&elements)[0];
        assert!(elements.iter().any(|e| e.rect == bar && matches!(e.kind, Kind::Capsule { fraction, .. } if fraction == 0.0)));
    }

    #[test]
    fn memory_shows_a_bar_and_two_tiles_when_the_adapter_reports_it_and_says_so_when_not() {
        let elements = layout(&GpuFlyout::new(full()));
        let bars: Vec<&Element> = elements.iter().filter(|e| matches!(e.kind, Kind::Capsule { glow: true, .. })).collect();
        // Three utilization bars and the memory bar.
        assert_eq!(bars.len(), 4);
        assert!(matches!(bars[3].kind, Kind::Capsule { fraction, .. } if (fraction - 0.375).abs() < 1e-6));
        assert_eq!(bars[3].rect.h, BAR_H);
        let words = texts(&elements);
        assert!(words.contains(&"In use") && words.contains(&"Dedicated"));
        assert!(words.contains(&"6.00 GB") && words.contains(&"16.0 GB"));
        assert!(!words.contains(&"Unavailable"));

        // An adapter with no memory of its own still says what it is using,
        // as a row rather than a bar that would have nothing to be of.
        let mut flyout = GpuFlyout::new(full());
        flyout.snapshot.memory_total = None;
        let elements = layout(&flyout);
        let words = texts(&elements);
        assert!(words.contains(&"In use") && words.contains(&"6.00 GB"));
        assert!(!words.contains(&"Dedicated") && !words.contains(&"Unavailable"));
        assert_eq!(elements.iter().filter(|e| matches!(e.kind, Kind::Capsule { glow: true, .. })).count(), 3);
        // A zero total is not something to divide by.
        flyout.snapshot.memory_total = Some(0);
        assert!(!texts(&layout(&flyout)).contains(&"Dedicated"));
        // Nothing at all is said to be so.
        flyout.snapshot.memory_used = None;
        assert!(texts(&layout(&flyout)).contains(&"Unavailable"));
    }

    #[test]
    fn the_hardware_tiles_are_the_vocabularys_two_across_with_the_third_wrapped() {
        let elements = layout(&GpuFlyout::new(full()));
        let hardware = cards(&elements)[3];
        let plates: Vec<Rect> = elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Plate) && e.rect.y > hardware.y)
            .map(|e| e.rect)
            .collect();
        assert_eq!(plates.len(), 3);
        assert!(plates.iter().all(|p| p.h == TILE_H));
        assert_eq!(plates[0].y, plates[1].y);
        assert!((plates[0].x - (hardware.x + CARD_PAD)).abs() < 1e-3);
        assert!((plates[1].right() - (hardware.right() - CARD_PAD)).abs() < 1e-3);
        assert!((plates[1].x - plates[0].right() - TILE_GAP).abs() < 1e-3);
        assert!((plates[2].y - plates[0].bottom() - TILE_GAP).abs() < 1e-3);
        assert_eq!(plates[2].x, plates[0].x);
        let texts = texts(&elements);
        for expected in ["Frequency", "2505 MHz", "Power", "187.25 W", "Temperature", "61\u{00B0}C"] {
            assert!(texts.contains(&expected), "{expected}");
        }
    }

    #[test]
    fn hardware_figures_read_like_the_mac_and_dash_when_absent() {
        assert_eq!(frequency(Some(1_450.4)), "1450 MHz");
        assert_eq!(power(Some(23.5)), "23.50 W");
        assert_eq!(temperature(Some(61.2), TemperatureUnit::Celsius), "61\u{00B0}C");
        assert_eq!(temperature(Some(20.0), TemperatureUnit::Fahrenheit), "68\u{00B0}F");
        assert_eq!(frequency(None), DASH);
        assert_eq!(power(None), DASH);
        assert_eq!(temperature(None, TemperatureUnit::Fahrenheit), DASH);
    }

    #[test]
    fn engine_types_are_spaced_into_words_and_short_ones_left_alone() {
        assert_eq!(engine_title("VideoDecode"), "Video Decode");
        assert_eq!(engine_title("VideoProcessing"), "Video Processing");
        assert_eq!(engine_title("3D"), "3D");
        assert_eq!(engine_title("Copy"), "Copy");
    }

    #[test]
    fn the_header_says_why_when_there_are_no_counters_and_waits_when_there_are() {
        let flyout = GpuFlyout::new(GpuSnapshot { supported: false, ..Default::default() });
        assert_eq!(flyout.subtitle(), "No GPU counters on this machine");
        let flyout = GpuFlyout::new(GpuSnapshot::default());
        assert_eq!(flyout.subtitle(), "Waiting for the first sample");
        assert_eq!(GpuFlyout::new(full()).subtitle(), "NVIDIA GeForce RTX 4080");
        // The cards stand regardless, dashed.
        let elements = layout(&flyout);
        assert_eq!(cards(&elements).len(), 4);
        assert!(texts(&elements).iter().filter(|t| **t == DASH).count() >= 4);
    }

    #[test]
    fn a_new_range_asks_for_a_rebuild_and_the_same_one_does_not() {
        let mut flyout = GpuFlyout::new(full());
        assert!(!flyout.activate(HistoryRange::FiveMinutes.id()));
        assert!(flyout.activate(HistoryRange::TwentyFourHours.id()));
        assert_eq!(flyout.range, HistoryRange::TwentyFourHours);
        assert!(!flyout.activate(Id::Settings));
        let picker = layout(&flyout).into_iter().filter(|e| HistoryRange::from_id(e.id).is_some()).count();
        assert_eq!(picker, 5);
    }

    #[test]
    fn the_height_is_what_the_builder_walked() {
        let flyout = GpuFlyout::new(full());
        let elements = layout(&flyout);
        let last = cards(&elements).last().unwrap().bottom();
        assert!((flyout.height(COLUMN_W, &measure) + PANEL_PAD - CARD_GAP - last).abs() < 1e-3);
        // Losing the memory figures swaps the bar and tiles for one caption.
        let mut without = GpuFlyout::new(full());
        without.snapshot.memory_used = None;
        assert!(without.height(COLUMN_W, &measure) < flyout.height(COLUMN_W, &measure));
    }

    #[test]
    fn the_content_follows_its_feed_and_relays_out_for_a_new_range() {
        let slot = Arc::new(Mutex::new(GpuSnapshot::default()));
        let mut content = GpuContent::new(Arc::clone(&slot));
        let palette = Palette::resolve(Theme::resolve(true, DEFAULT_ACCENT));
        let cx = Context {
            width: PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Gpu),
            measure: &measure,
            hover: None,
            now_unix: 0,
        };
        assert_eq!(content.module(), ModuleId::Gpu);
        assert!(!content.tick());
        *slot.lock().unwrap() = full();
        assert!(content.tick());
        let page = content.build(&cx);
        assert!(!content.tick());
        assert!(texts(&page.elements).contains(&"NVIDIA GeForce RTX 4080"));
        assert!((page.height - (GpuFlyout::new(full()).height(COLUMN_W, &measure) + 2.0 * PANEL_PAD)).abs() < 1e-3);
        assert_eq!(content.activate(HistoryRange::OneMinute.id()), Response::Relayout);
        assert_eq!(content.activate(HistoryRange::OneMinute.id()), Response::None);
    }
}
