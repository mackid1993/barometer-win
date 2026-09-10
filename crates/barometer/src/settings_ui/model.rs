// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// What the settings window edits, and the facts it shows alongside.
//
// `store::Settings` is the persisted document and this module does not
// redefine it. What it adds is the two things the strip needs that the store
// does not carry yet - the stacks, and one order that interleaves stacks with
// modules - and the GPU adapter choice. All three are candidates for the
// store, and the report that accompanies this file says so; until they land
// there, `Model` is the whole of what the window hands back.
//
// The one idea this module has to get right is that a module and a stack are
// different things that can both show the same reading (AGENTS.md, "Modules
// and stacks are both real"). A module's own column can be on or off; a stack
// can carry that module's metric whether or not the column is on; and a stack
// that hides its sources takes the column off the strip without touching the
// module's switch. Every caption the composer shows is derived from those
// three facts, here, where it can be tested.

use barometer_core::module::{ModuleId, Readout};
use barometer_core::sensors::Sensor;
use barometer_core::stack::{StackEntry, StackLayout, StackMetric, StackSettings, StacksSettings};
use barometer_core::store::{self, ModuleEntry, Settings};
use barometer_core::weather::models::Location;

// Both persisted now, so both live with the rest of the settings in core;
// re-exported so the window's code keeps its names.
pub use barometer_core::settings::GpuChoice;
pub use barometer_core::stack::StripItem;

/// An adapter the GPU module found, for the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuAdapter {
    pub key: String,
    pub name: String,
}

/// Everything the window edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub settings: Settings,
    pub stacks: StacksSettings,
    /// Modules and stacks in strip order, interleaved.
    ///
    /// `settings.modules` is kept in agreement with this - it is rewritten
    /// after every change - so a reader of the store alone still sees the
    /// modules in the right relative order. This list is what places a stack
    /// among them.
    pub order: Vec<StripItem>,
    pub gpu: GpuChoice,
}

impl Model {
    pub fn new(
        settings: Settings,
        stacks: StacksSettings,
        order: Vec<StripItem>,
        gpu: GpuChoice,
    ) -> Model {
        let mut model = Model { settings, stacks, order, gpu };
        model.reconcile();
        model
    }

    /// A model of the settings, with the stacks, order and adapter choice
    /// the settings carry.
    pub fn from_settings(settings: Settings) -> Model {
        let stacks = settings.stacks.clone();
        let order = settings.order.clone();
        let gpu = settings.gpu.clone();
        Model::new(settings, stacks, order, gpu)
    }

    /// The settings with everything the window edits folded back in, which
    /// is what the app saves and applies.
    pub fn to_settings(&self) -> Settings {
        let mut settings = self.settings.clone();
        settings.stacks = self.stacks.clone();
        settings.order = self.order.clone();
        settings.gpu = self.gpu.clone();
        settings
    }

    /// Repairs the order against what actually exists.
    ///
    /// The order can name a stack that was deleted, list a module twice, or
    /// omit a module added since it was written; each of those is a hand-edit
    /// or an upgrade away. Unknowns and repeats are dropped, and anything
    /// missing is appended in the order its own list holds it, so a module
    /// that was never placed lands where the store had it rather than at
    /// random.
    pub fn reconcile(&mut self) {
        let settings = &self.settings;
        let stacks = &self.stacks;
        let mut order: Vec<StripItem> = Vec::with_capacity(self.order.len());
        for item in self.order.drain(..) {
            let known = match item {
                StripItem::Module(id) => settings.modules.iter().any(|e| e.id == id),
                StripItem::Stack(id) => stacks.stacks.iter().any(|s| s.id == id),
            };
            if known && !order.contains(&item) {
                order.push(item);
            }
        }
        for entry in &settings.modules {
            if !order.contains(&StripItem::Module(entry.id)) {
                order.push(StripItem::Module(entry.id));
            }
        }
        for stack in &stacks.stacks {
            if !order.contains(&StripItem::Stack(stack.id)) {
                order.push(StripItem::Stack(stack.id));
            }
        }
        self.order = order;
        self.sync_modules_order();
    }

    /// Rewrites `settings.modules` to follow `order`.
    fn sync_modules_order(&mut self) {
        let mut entries: Vec<ModuleEntry> = Vec::with_capacity(self.settings.modules.len());
        for item in &self.order {
            if let StripItem::Module(id) = item {
                if let Some(entry) = self.settings.modules.iter().find(|e| e.id == *id) {
                    entries.push(entry.clone());
                }
            }
        }
        self.settings.modules = entries;
    }

    pub fn module_entry(&self, id: ModuleId) -> Option<&ModuleEntry> {
        self.settings.modules.iter().find(|e| e.id == id)
    }

    pub fn is_module_enabled(&self, id: ModuleId) -> bool {
        self.module_entry(id).is_some_and(|e| e.enabled)
    }

    pub fn stack(&self, id: u32) -> Option<&StackSettings> {
        self.stacks.stacks.iter().find(|s| s.id == id)
    }

    pub fn stack_mut(&mut self, id: u32) -> Option<&mut StackSettings> {
        self.stacks.stacks.iter_mut().find(|s| s.id == id)
    }

    /// Every stack carrying a reading from this module, on or off.
    pub fn stacks_using(&self, module: ModuleId) -> Vec<&StackSettings> {
        self.stacks
            .stacks
            .iter()
            .filter(|s| s.metrics.iter().any(|e| e.metric.module() == module))
            .collect()
    }

    /// The stack that is currently taking this module's own column off the
    /// strip, if one is.
    pub fn hidden_by(&self, module: ModuleId) -> Option<&StackSettings> {
        self.stacks
            .stacks
            .iter()
            .find(|s| s.is_enabled && s.hides_source_items && s.metrics.iter().any(|e| e.metric.module() == module))
    }

    /// Whether an item draws on the strip right now.
    ///
    /// A module needs its switch on and no stack hiding it; a stack needs its
    /// switch on and something to show. The composer lists everything either
    /// way - this decides what the preview and the count include.
    pub fn is_shown(&self, item: StripItem) -> bool {
        match item {
            StripItem::Module(id) => self.is_module_enabled(id) && self.hidden_by(id).is_none(),
            StripItem::Stack(id) => {
                self.stack(id).is_some_and(|s| s.is_enabled && !s.metrics.is_empty())
            }
        }
    }

    /// How many items the strip draws, which is what the Strip pane's summary
    /// line counts.
    pub fn shown_count(&self) -> usize {
        self.order.iter().filter(|item| self.is_shown(**item)).count()
    }

    pub fn is_item_enabled(&self, item: StripItem) -> bool {
        match item {
            StripItem::Module(id) => self.is_module_enabled(id),
            StripItem::Stack(id) => self.stack(id).is_some_and(|s| s.is_enabled),
        }
    }

    pub fn set_item_enabled(&mut self, item: StripItem, on: bool) {
        match item {
            StripItem::Module(id) => {
                if let Some(entry) = self.settings.modules.iter_mut().find(|e| e.id == id) {
                    entry.enabled = on;
                }
            }
            StripItem::Stack(id) => {
                if let Some(stack) = self.stack_mut(id) {
                    stack.is_enabled = on;
                }
            }
        }
    }

    pub fn position(&self, item: StripItem) -> Option<usize> {
        self.order.iter().position(|i| *i == item)
    }

    /// Moves the item at `from` so that it sits at index `to`.
    pub fn move_item(&mut self, from: usize, to: usize) {
        move_in(&mut self.order, from, to);
        self.sync_modules_order();
    }

    /// Creates a stack, placed at the end of the strip, and returns its id.
    ///
    /// Seeded with one reading when the stack is being made *from* a module -
    /// "put the GPU in a stack" should produce a stack with the GPU in it, not
    /// an empty one the person then has to fill from a list.
    pub fn add_stack(&mut self, seed: Option<StackMetric>) -> u32 {
        let id = self.stacks.add();
        if let (Some(metric), Some(stack)) = (seed, self.stack_mut(id)) {
            stack.metrics.push(StackEntry::new(metric));
        }
        self.order.push(StripItem::Stack(id));
        id
    }

    pub fn remove_stack(&mut self, id: u32) {
        self.stacks.remove(id);
        self.order.retain(|item| *item != StripItem::Stack(id));
    }

    /// Adds a reading to a stack. A reading already there is left alone
    /// rather than shown twice.
    pub fn add_metric(&mut self, id: u32, metric: StackMetric) -> bool {
        let Some(stack) = self.stack_mut(id) else { return false };
        if stack.has(&metric) {
            return false;
        }
        stack.metrics.push(StackEntry::new(metric));
        true
    }

    /// What the strip calls a reading, as the user typed it. Empty means
    /// the reading's own label.
    pub fn set_metric_label(&mut self, id: u32, index: usize, label: &str) {
        if let Some(entry) = self.stack_mut(id).and_then(|s| s.metrics.get_mut(index)) {
            let label = label.trim();
            entry.label = (!label.is_empty()).then(|| label.to_string());
        }
    }

    pub fn remove_metric(&mut self, id: u32, index: usize) {
        if let Some(stack) = self.stack_mut(id) {
            if index < stack.metrics.len() {
                stack.metrics.remove(index);
            }
        }
    }

    pub fn move_metric(&mut self, id: u32, from: usize, to: usize) {
        if let Some(stack) = self.stack_mut(id) {
            move_in(&mut stack.metrics, from, to);
        }
    }

    pub fn set_stack_name(&mut self, id: u32, name: &str) {
        if let Some(stack) = self.stack_mut(id) {
            stack.name = name.trim().to_string();
        }
    }

    pub fn set_stack_layout(&mut self, id: u32, layout: StackLayout) {
        if let Some(stack) = self.stack_mut(id) {
            stack.layout = layout;
        }
    }

    pub fn set_stack_hides_sources(&mut self, id: u32, hides: bool) {
        if let Some(stack) = self.stack_mut(id) {
            stack.hides_source_items = hides;
        }
    }

    /// The readings not yet in a stack, in picker order, for "Add a reading".
    pub fn metrics_not_in(&self, id: u32) -> Vec<StackMetric> {
        let present = self.stack(id);
        StackMetric::ALL
            .iter()
            .filter(|m| !present.is_some_and(|s| s.has(m)))
            .cloned()
            .collect()
    }

    /// Everything "Add a reading" offers, each with the name it is listed
    /// under: the fixed readings first, then every sensor the source
    /// reports that is not already in the stack - all of them, each core's
    /// temperature and each adapter's, listed under its hardware.
    pub fn metric_candidates(&self, id: u32, sensors: &[Sensor]) -> Vec<(StackMetric, String)> {
        let present = self.stack(id);
        let mut candidates: Vec<(StackMetric, String)> = self
            .metrics_not_in(id)
            .into_iter()
            .map(|m| {
                let name = format!("{} \u{00B7} {}", module_name(m.module()), m.display_name());
                (m, name)
            })
            .collect();
        // By hardware, then by kind, then by name, so every temperature on a
        // device sits together and the identical names do not interleave.
        let mut found: Vec<&Sensor> = sensors.iter().collect();
        found.sort_by(|a, b| {
            a.hardware
                .cmp(&b.hardware)
                .then_with(|| a.kind.name().cmp(b.kind.name()))
                .then_with(|| a.name.cmp(&b.name))
        });
        for sensor in found {
            let metric = StackMetric::Sensor(sensor.id.clone());
            if present.is_some_and(|s| s.has(&metric)) {
                continue;
            }
            candidates.push((metric.clone(), metric.display_name_in(sensors)));
        }
        candidates
    }
}

/// Moves `items[from]` so that it lands at index `to`, clamping both.
pub fn move_in<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if items.is_empty() || from >= items.len() {
        return;
    }
    let to = to.min(items.len() - 1);
    if from == to {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

/// What the composer calls a stack.
///
/// The persisted name when there is one. `StackSettings::display_name` says
/// "Untitled" otherwise, and it is right not to invent a number; but a list of
/// three "Untitled" rows tells nobody which is which, so an unnamed stack is
/// titled by what it shows - "CPU + GPU" - and a stack with nothing in it yet
/// by what it is.
pub fn stack_title(stack: &StackSettings) -> String {
    if !stack.name.is_empty() {
        return stack.name.clone();
    }
    if stack.metrics.is_empty() {
        return "New stack".to_string();
    }
    let labels: Vec<String> = stack.metrics.iter().map(|e| e.caption(&[])).collect();
    labels.join(" + ")
}

/// The module's name as the composer and inspector show it.
pub fn module_name(id: ModuleId) -> &'static str {
    match id {
        ModuleId::Cpu => "CPU",
        ModuleId::Gpu => "GPU",
        ModuleId::Memory => "Memory",
        ModuleId::Disks => "Disks",
        ModuleId::Network => "Network",
        ModuleId::Sensors => "Sensors",
        ModuleId::Weather => "Weather",
    }
}

/// One line under the inspector's title.
pub fn module_blurb(id: ModuleId) -> &'static str {
    match id {
        ModuleId::Cpu => "Processor utilization across every core.",
        ModuleId::Gpu => "Graphics utilization, as Task Manager reports it.",
        ModuleId::Memory => "Physical memory in use.",
        ModuleId::Disks => "Read and write throughput across every disk.",
        ModuleId::Network => "Download and upload rates, across every active interface.",
        ModuleId::Sensors => "Temperatures from LibreHardwareMonitor.",
        ModuleId::Weather => "Conditions and temperature from Open-Meteo.",
    }
}

/// What the Sensors module's column is showing.
///
/// The module shows the pinned reading when there is one and the source
/// still reports it, and the hottest processor temperature otherwise - the
/// same fallback `SensorsModule` makes, so the caption never promises a
/// reading the strip is not drawing. The macOS app lists the chosen sensor
/// by name in the same place, which is what `Pinned` carries.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SensorShown<'a> {
    Hottest,
    Pinned(&'a str),
    /// Pinned to a sensor the source is not reporting, so the strip has
    /// fallen back to the hottest.
    PinnedMissing,
}

/// How a module's own column is drawn, for the list caption.
pub fn module_style(id: ModuleId, upload_first: bool, sensor: SensorShown) -> String {
    match id {
        ModuleId::Cpu | ModuleId::Gpu | ModuleId::Memory => "Label over value".to_string(),
        ModuleId::Disks => "Read over write".to_string(),
        ModuleId::Network if upload_first => "Upload over download".to_string(),
        ModuleId::Network => "Download over upload".to_string(),
        ModuleId::Sensors => match sensor {
            SensorShown::Hottest => "Hottest processor temperature".to_string(),
            SensorShown::Pinned(name) => format!("{name}, pinned"),
            SensorShown::PinnedMissing => {
                "Hottest processor temperature, while the pinned sensor is not reporting".to_string()
            }
        },
        ModuleId::Weather => "Condition mark and temperature".to_string(),
    }
}

/// The glyph a module is marked with.
///
/// Segoe Fluent Icons, which is on every Windows 11 machine, so nothing is
/// shipped and the marks match the rest of the system. Each is the glyph
/// that means what the macOS app's SF Symbol means - `symbolName` in
/// SettingsWindowController.swift - so somebody with both apps sees the
/// same things in both lists. Letters were here first, C, G, M, D, N, T, W,
/// and a row of initials reads as a legend rather than as a list of things.
///
/// Every codepoint below was picked by rendering the font and looking, not
/// from a name table: the font has no glyph names, so there is nothing to
/// search, and the range has gaps that draw as boxes.
pub fn module_glyph(id: ModuleId) -> char {
    match id {
        // A processor package with pins down both sides; the Mac's "cpu".
        ModuleId::Cpu => '\u{E950}',
        // Three planes stacked in perspective, which is the Mac's
        // "square.stack.3d.up". It replaced a monitor at E7F4: a monitor is
        // the display, and the module measures the card behind it.
        ModuleId::Gpu => '\u{E81E}',
        // A memory module. The Mac draws a chip; the font's only chip is the
        // processor above, so this is the nearest that is not the CPU again.
        ModuleId::Memory => '\u{E964}',
        // A drive; the Mac's "internaldrive".
        ModuleId::Disks => '\u{EDA2}',
        // A globe, as the Mac's "network" is. It replaced connected nodes at
        // F0B9, which read as a diagram rather than as the outside world.
        ModuleId::Network => '\u{E774}',
        // A thermometer; the Mac's "thermometer.medium".
        ModuleId::Sensors => '\u{E9CA}',
        // The cloud alone. The Mac's "cloud.sun" has a sun behind it, which
        // this font does not draw; the settings window composes the two the
        // way the strip does, and this is the base of that pair.
        ModuleId::Weather => '\u{E753}',
    }
}

/// The mark on a stack: tiles grouped together, the Mac's
/// "rectangle.3.group". Not the layered planes at F156, which are now what
/// the GPU is drawn with and would have made a stack look like a second GPU.
pub const STACK_GLYPH: char = '\u{E8A9}';

/// Something about the machine that stops a module drawing even when it is on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModuleFact {
    Fine,
    NeedsSensorSource,
    NeedsLocation,
}

impl ModuleFact {
    pub fn reason(self) -> Option<&'static str> {
        match self {
            ModuleFact::Fine => None,
            ModuleFact::NeedsSensorSource => Some("Needs a sensor source"),
            ModuleFact::NeedsLocation => Some("Needs a location"),
        }
    }
}

/// The caption under a module's name in the composer.
///
/// This is where a module and a stack are told apart in one line. A module
/// hidden by a stack says so and names the stack; a module that is off but
/// still feeding a stack says both, because "Off" alone would suggest the
/// reading is gone from the strip when it is not.
pub fn module_caption(model: &Model, id: ModuleId, fact: ModuleFact, sensor: SensorShown) -> String {
    let in_stacks = model.stacks_using(id);
    let style = module_style(id, model.settings.network_upload_first, sensor);
    if model.is_module_enabled(id) {
        if let Some(stack) = model.hidden_by(id) {
            return format!("Hidden while {} is on", stack_title(stack));
        }
        if let Some(reason) = fact.reason() {
            return reason.to_string();
        }
        match in_stacks.first() {
            Some(stack) => format!("{style} · also in {}", stack_title(stack)),
            None => style,
        }
    } else {
        match in_stacks.first() {
            Some(stack) => format!("Off · in {}", stack_title(stack)),
            None => "Off".to_string(),
        }
    }
}

/// The caption under a stack's name in the composer.
pub fn stack_caption(stack: &StackSettings) -> String {
    if stack.metrics.is_empty() {
        return "No readings yet".to_string();
    }
    let labels: Vec<String> = stack.metrics.iter().map(|e| e.caption(&[])).collect();
    let readings = labels.join(" · ");
    if stack.is_enabled {
        readings
    } else {
        format!("Off · {readings}")
    }
}

/// The modules a stack draws readings from, named, for the hide-sources
/// caption: "CPU and GPU".
pub fn source_names(stack: &StackSettings) -> String {
    let names: Vec<&str> = stack.source_modules().into_iter().map(module_name).collect();
    join_names(&names)
}

/// "A", "A and B", "A, B and C".
pub fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [head @ .., last] => format!("{} and {}", head.join(", "), last),
    }
}

pub fn clamp_gap(dip: f32) -> f32 {
    dip.clamp(store::MIN_SPACING_DIP, store::MAX_SPACING_DIP)
}

pub fn clamp_poll(seconds: u32) -> u32 {
    seconds.clamp(store::MIN_POLL_SECONDS, store::MAX_POLL_SECONDS)
}

/// What the sensor stack is doing, as the running app knows it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SensorSource {
    /// Nothing has been reported yet.
    #[default]
    Unknown,
    Providing {
        name: String,
        readings: usize,
    },
    /// No LibreHardwareMonitor to read from. The normal first-run state.
    NotInstalled,
    /// A library is there but the helper did not answer.
    NotRunning,
    Failed(String),
}

/// What the window shows that is not a setting: live readings for the
/// preview, the machine's sensors for the pin picker, and so on.
///
/// Pushed by the app whenever it has something new; the window never samples
/// hardware itself, because that is the sampling thread's job and two threads
/// asking the same helper would double its cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub readouts: Vec<(ModuleId, Readout)>,
    /// The taskbar's height in DIPs, which the preview draws at and which
    /// decides whether the strip has room for two rows.
    pub taskbar_height_dip: f32,
    pub sensors: Vec<Sensor>,
    pub sensor_source: SensorSource,
    /// Adapters the GPU module found. Empty until it can enumerate them.
    pub gpus: Vec<GpuAdapter>,
    /// Interfaces the network module found: the name Windows shows, and
    /// whether it is a tunnel.
    pub interfaces: Vec<(String, bool)>,
    /// Every stack reading the modules could produce a figure for, keyed
    /// by the metric, formatted for the strip. Filled by the app from the
    /// modules; the preview and the strip both read it.
    pub metric_values: Vec<(StackMetric, String)>,
    /// Where the weather worker decided the machine is, when no location was
    /// saved.
    /// The volumes with a drive letter, for the disk pane's picker. Taken
    /// from the disks module's own threaded scan, so nothing here asks a
    /// sleeping network drive anything.
    pub volumes: Vec<barometer_core::volumes::Volume>,
    /// The physical disks the counters name, for the disk pane's picker.
    pub disks: Vec<barometer_core::modules::DiskDevice>,
    pub located: Option<Location>,
    pub weather_error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            readouts: Vec::new(),
            // The Windows 11 default taskbar.
            taskbar_height_dip: 48.0,
            sensors: Vec::new(),
            sensor_source: SensorSource::Unknown,
            gpus: Vec::new(),
            interfaces: Vec::new(),
            metric_values: Vec::new(),
            volumes: Vec::new(),
            disks: Vec::new(),
            located: None,
            weather_error: None,
        }
    }
}

impl Snapshot {
    pub fn readout(&self, id: ModuleId) -> Option<&Readout> {
        self.readouts.iter().find(|(m, _)| *m == id).map(|(_, r)| r)
    }

    /// Whether a module can draw at all on this machine right now.
    pub fn fact(&self, id: ModuleId, model: &Model) -> ModuleFact {
        match id {
            ModuleId::Sensors => match self.sensor_source {
                SensorSource::Providing { .. } | SensorSource::Unknown => ModuleFact::Fine,
                _ => ModuleFact::NeedsSensorSource,
            },
            ModuleId::Weather => {
                let has_location = model.settings.weather.primary_location().is_some()
                    || (model.settings.weather.uses_current_location && self.located.is_some())
                    || model.settings.weather.uses_current_location && self.weather_error.is_none();
                if has_location {
                    ModuleFact::Fine
                } else {
                    ModuleFact::NeedsLocation
                }
            }
            _ => ModuleFact::Fine,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::from_settings(Settings::default())
    }

    fn stack_with(model: &mut Model, metrics: &[StackMetric], hides: bool) -> u32 {
        let id = model.add_stack(None);
        for metric in metrics {
            model.add_metric(id, metric.clone());
        }
        model.set_stack_hides_sources(id, hides);
        id
    }

    #[test]
    fn a_fresh_model_lists_every_module_in_the_stores_order() {
        let m = model();
        let modules: Vec<ModuleId> = m
            .order
            .iter()
            .filter_map(|i| match i {
                StripItem::Module(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(modules, ModuleId::ALL.to_vec());
    }

    #[test]
    fn reconcile_drops_unknown_stacks_and_repeats_and_appends_what_is_missing() {
        let mut m = model();
        m.order = vec![
            StripItem::Stack(99),
            StripItem::Module(ModuleId::Weather),
            StripItem::Module(ModuleId::Weather),
        ];
        m.reconcile();
        assert_eq!(m.order[0], StripItem::Module(ModuleId::Weather));
        assert_eq!(m.order.len(), ModuleId::ALL.len());
        assert!(!m.order.contains(&StripItem::Stack(99)));
        // The store's module list follows the order.
        assert_eq!(m.settings.modules[0].id, ModuleId::Weather);
    }

    #[test]
    fn moving_an_item_rewrites_the_stores_module_order_to_match() {
        let mut m = model();
        let from = m.position(StripItem::Module(ModuleId::Weather)).unwrap();
        m.move_item(from, 0);
        assert_eq!(m.order[0], StripItem::Module(ModuleId::Weather));
        assert_eq!(m.settings.modules[0].id, ModuleId::Weather);
        assert_eq!(m.settings.modules[1].id, ModuleId::Cpu);
    }

    #[test]
    fn move_in_clamps_and_ignores_a_no_op() {
        let mut items = vec![1, 2, 3];
        move_in(&mut items, 0, 10);
        assert_eq!(items, vec![2, 3, 1]);
        move_in(&mut items, 7, 0);
        assert_eq!(items, vec![2, 3, 1]);
        move_in(&mut items, 1, 1);
        assert_eq!(items, vec![2, 3, 1]);
    }

    #[test]
    fn a_stack_that_hides_its_sources_takes_the_module_off_the_strip_without_switching_it_off() {
        let mut m = model();
        assert!(m.is_shown(StripItem::Module(ModuleId::Cpu)));
        let id = stack_with(&mut m, &[StackMetric::CpuTotal, StackMetric::GpuUtilization], true);
        assert!(!m.is_shown(StripItem::Module(ModuleId::Cpu)));
        // The module's own switch is untouched: this is the stack's doing.
        assert!(m.is_module_enabled(ModuleId::Cpu));
        assert_eq!(m.hidden_by(ModuleId::Cpu).map(|s| s.id), Some(id));
        // Turning the stack off gives the column back.
        m.set_item_enabled(StripItem::Stack(id), false);
        assert!(m.is_shown(StripItem::Module(ModuleId::Cpu)));
        assert!(m.hidden_by(ModuleId::Cpu).is_none());
    }

    #[test]
    fn a_stack_with_nothing_in_it_is_not_drawn_and_not_counted() {
        let mut m = model();
        let id = m.add_stack(None);
        assert!(!m.is_shown(StripItem::Stack(id)));
        let before = m.shown_count();
        m.add_metric(id, StackMetric::MemoryUsedPercent);
        assert!(m.is_shown(StripItem::Stack(id)));
        assert_eq!(m.shown_count(), before + 1);
    }

    #[test]
    fn a_new_stack_lands_at_the_end_of_the_strip_seeded_with_its_module() {
        let mut m = model();
        let id = m.add_stack(Some(StackMetric::GpuUtilization));
        assert_eq!(m.order.last(), Some(&StripItem::Stack(id)));
        assert_eq!(m.stack(id).unwrap().metrics.iter().map(|e| e.metric.clone()).collect::<Vec<_>>(), vec![StackMetric::GpuUtilization]);
        m.remove_stack(id);
        assert!(!m.order.contains(&StripItem::Stack(id)));
        assert!(m.stack(id).is_none());
    }

    #[test]
    fn a_reading_is_never_in_a_stack_twice_and_can_be_reordered() {
        let mut m = model();
        let id = stack_with(&mut m, &[StackMetric::CpuTotal, StackMetric::GpuUtilization], false);
        assert!(!m.add_metric(id, StackMetric::CpuTotal));
        m.move_metric(id, 1, 0);
        assert_eq!(
            m.stack(id).unwrap().metrics.iter().map(|e| e.metric.clone()).collect::<Vec<_>>(),
            vec![StackMetric::GpuUtilization, StackMetric::CpuTotal]
        );
        assert!(!m.metrics_not_in(id).contains(&StackMetric::CpuTotal));
        m.remove_metric(id, 0);
        assert_eq!(m.stack(id).unwrap().metrics.iter().map(|e| e.metric.clone()).collect::<Vec<_>>(), vec![StackMetric::CpuTotal]);
    }

    #[test]
    fn an_unnamed_stack_is_titled_by_what_it_shows() {
        let mut stack = StackSettings::new(3);
        assert_eq!(stack_title(&stack), "New stack");
        stack.metrics = vec![StackMetric::CpuTotal, StackMetric::GpuUtilization].into_iter().map(StackEntry::new).collect();
        assert_eq!(stack_title(&stack), "CPU + GPU");
        stack.name = "Main".into();
        assert_eq!(stack_title(&stack), "Main");
    }

    #[test]
    fn the_sensors_caption_names_the_pinned_sensor_and_says_when_it_has_fallen_back() {
        let mut m = model();
        m.set_item_enabled(StripItem::Module(ModuleId::Sensors), true);
        assert_eq!(
            module_caption(&m, ModuleId::Sensors, ModuleFact::Fine, SensorShown::Hottest),
            "Hottest processor temperature"
        );
        assert_eq!(
            module_caption(&m, ModuleId::Sensors, ModuleFact::Fine, SensorShown::Pinned("CPU Package")),
            "CPU Package, pinned"
        );
        assert!(module_caption(&m, ModuleId::Sensors, ModuleFact::Fine, SensorShown::PinnedMissing)
            .starts_with("Hottest processor temperature, while"));
    }

    #[test]
    fn the_module_caption_says_which_of_the_two_things_is_happening() {
        let mut m = model();
        assert_eq!(module_caption(&m, ModuleId::Cpu, ModuleFact::Fine, SensorShown::Hottest), "Label over value");
        assert_eq!(module_caption(&m, ModuleId::Gpu, ModuleFact::Fine, SensorShown::Hottest), "Off");

        let id = stack_with(&mut m, &[StackMetric::CpuTotal, StackMetric::GpuUtilization], false);
        m.set_stack_name(id, "Main");
        // On, and also inside a stack that leaves its column alone.
        assert_eq!(
            module_caption(&m, ModuleId::Cpu, ModuleFact::Fine, SensorShown::Hottest),
            "Label over value · also in Main"
        );
        // Off on its own, but its reading still reaches the strip through Main.
        assert_eq!(module_caption(&m, ModuleId::Gpu, ModuleFact::Fine, SensorShown::Hottest), "Off · in Main");

        m.set_stack_hides_sources(id, true);
        assert_eq!(module_caption(&m, ModuleId::Cpu, ModuleFact::Fine, SensorShown::Hottest), "Hidden while Main is on");
        // The network caption says which rate is on top, since that is the
        // one thing about the column a person can change.
        m.set_item_enabled(StripItem::Module(ModuleId::Network), true);
        assert_eq!(module_caption(&m, ModuleId::Network, ModuleFact::Fine, SensorShown::Hottest), "Download over upload");
        m.settings.network_upload_first = true;
        assert_eq!(module_caption(&m, ModuleId::Network, ModuleFact::Fine, SensorShown::Hottest), "Upload over download");
        // A machine fact outranks the style but not the switch: a module
        // that is off says so, whatever the machine could not do for it.
        assert_eq!(module_caption(&m, ModuleId::Sensors, ModuleFact::NeedsSensorSource, SensorShown::Hottest), "Off");
        m.set_item_enabled(StripItem::Module(ModuleId::Sensors), true);
        assert_eq!(
            module_caption(&m, ModuleId::Sensors, ModuleFact::NeedsSensorSource, SensorShown::Hottest),
            "Needs a sensor source"
        );
    }

    #[test]
    fn the_stack_caption_lists_its_readings() {
        let mut stack = StackSettings::new(1);
        assert_eq!(stack_caption(&stack), "No readings yet");
        stack.metrics = vec![StackMetric::CpuTotal, StackMetric::NetworkDownload].into_iter().map(StackEntry::new).collect();
        assert_eq!(stack_caption(&stack), "CPU · DOWN");
        stack.is_enabled = false;
        assert_eq!(stack_caption(&stack), "Off · CPU · DOWN");
    }

    #[test]
    fn source_names_read_as_a_sentence() {
        assert_eq!(join_names(&[]), "");
        assert_eq!(join_names(&["CPU"]), "CPU");
        assert_eq!(join_names(&["CPU", "GPU"]), "CPU and GPU");
        assert_eq!(join_names(&["CPU", "GPU", "Memory"]), "CPU, GPU and Memory");
        let mut stack = StackSettings::new(1);
        stack.metrics = vec![StackMetric::GpuTemperature, StackMetric::CpuUser, StackMetric::CpuTotal].into_iter().map(StackEntry::new).collect();
        assert_eq!(source_names(&stack), "CPU and GPU");
    }

    #[test]
    fn clamps_hold_to_the_stores_limits() {
        assert_eq!(clamp_gap(-3.0), store::MIN_SPACING_DIP);
        assert_eq!(clamp_gap(1000.0), store::MAX_SPACING_DIP);
        assert_eq!(clamp_poll(0), store::MIN_POLL_SECONDS);
        assert_eq!(clamp_poll(999), store::MAX_POLL_SECONDS);
    }

    #[test]
    fn the_weather_fact_needs_a_saved_location_or_a_located_machine() {
        let mut m = model();
        m.settings.weather.uses_current_location = false;
        let snapshot = Snapshot::default();
        assert_eq!(snapshot.fact(ModuleId::Weather, &m), ModuleFact::NeedsLocation);
        m.settings.weather.uses_current_location = true;
        assert_eq!(snapshot.fact(ModuleId::Weather, &m), ModuleFact::Fine);
        let failed = Snapshot { weather_error: Some("offline".into()), ..Snapshot::default() };
        assert_eq!(failed.fact(ModuleId::Weather, &m), ModuleFact::NeedsLocation);
    }

    #[test]
    fn the_sensor_fact_is_calm_until_the_app_says_there_is_no_source() {
        let m = model();
        let unknown = Snapshot::default();
        assert_eq!(unknown.fact(ModuleId::Sensors, &m), ModuleFact::Fine);
        let none = Snapshot { sensor_source: SensorSource::NotInstalled, ..Snapshot::default() };
        assert_eq!(none.fact(ModuleId::Sensors, &m), ModuleFact::NeedsSensorSource);
    }
}
