//! What a module is, and what it hands to the taskbar.
//!
//! Barometer on macOS gives every module its own menu bar item. Windows has no
//! equivalent — the taskbar exposes no multi-item API — so here every enabled
//! module is laid out inside one strip. That makes the readout contract
//! narrower than the Mac's: a module produces a label and one or two short
//! lines, and the strip decides where they sit.

use std::fmt;

/// The modules Barometer knows about, in the order they appear by default.
///
/// The discriminants are stable: they are written to the settings file, so
/// inserting a variant in the middle would silently reorder a user's strip.
/// Append only.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModuleId {
    Cpu,
    Gpu,
    Memory,
    Disks,
    Network,
    Sensors,
    Weather,
}

impl ModuleId {
    /// Every module, in the order the macOS app lists them, which is the order
    /// the settings picker and `StackMetric::by_module` both follow.
    ///
    /// The macOS enum also has `time`, `combined`, `focus` and `nowPlaying`.
    /// Time and Battery are both dropped for the same reason: Windows already
    /// shows a clock and a battery in the shell, each with a flyout carrying
    /// the detail, so a second one spends strip width duplicating what the
    /// user already has. macOS needed both because its own are poor, which is
    /// a reason that does not cross over. Battery *sensors* are unaffected -
    /// they still arrive through the Sensors module.
    /// Combined is subsumed because the Windows strip is always one item, and
    /// focus and nowPlaying are macOS facilities with no Windows counterpart.
    pub const ALL: [ModuleId; 7] = [
        ModuleId::Cpu,
        ModuleId::Gpu,
        ModuleId::Memory,
        ModuleId::Disks,
        ModuleId::Network,
        ModuleId::Sensors,
        ModuleId::Weather,
    ];

    /// The short uppercase tag shown above a value in two-row form.
    pub fn label(self) -> &'static str {
        match self {
            ModuleId::Network => "NET",
            ModuleId::Cpu => "CPU",
            ModuleId::Memory => "MEM",
            ModuleId::Gpu => "GPU",
            ModuleId::Disks => "DSK",
            ModuleId::Sensors => "TEMP",
            ModuleId::Weather => "WX",
        }
    }

    /// The module's name in full, for tooltips and the settings list.
    ///
    /// Spelled out where the strip's label is abbreviated: "DSK" has to fit in
    /// a column, "Disks" does not.
    pub fn title(self) -> &'static str {
        match self {
            ModuleId::Network => "Network",
            ModuleId::Cpu => "Processor",
            ModuleId::Memory => "Memory",
            ModuleId::Gpu => "Graphics",
            ModuleId::Disks => "Disks",
            ModuleId::Sensors => "Temperature",
            ModuleId::Weather => "Weather",
        }
    }

    /// The key this module is stored under in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            ModuleId::Network => "network",
            ModuleId::Cpu => "cpu",
            ModuleId::Memory => "memory",
            ModuleId::Gpu => "gpu",
            ModuleId::Disks => "disks",
            ModuleId::Sensors => "sensors",
            ModuleId::Weather => "weather",
        }
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// One module's current reading, formatted for the strip.
///
/// `primary` and `secondary` are already-formatted text rather than numbers
/// because the formatting rules are per-module: bytes-per-second picks its own
/// unit, a percentage never does. The strip only lays them out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readout {
    /// The widest text this module can ever show, if it is worth reserving.
    ///
    /// The strip sizes each column to the wider of this and what is currently
    /// displayed, which is what stops the readout shuffling. Without it every
    /// column is exactly as wide as its present value, so "9 KB/s" becoming
    /// "512 KB/s" moves every column to its right, several times a second,
    /// and the whole strip crawls. The macOS app calls the same idea a
    /// reserved string.
    ///
    /// None means "size me to my content", which is right for anything whose
    /// width genuinely does not vary.
    pub reserved: Option<String>,

    /// The weather mark, when there is one.
    ///
    /// Only the Weather module sets this, and it is here rather than reached
    /// for by downcasting the module: the strip has to know that this column
    /// is drawn as digits with a mark around them instead of a label over a
    /// value, and that is a fact about the readout, not about the module that
    /// produced it.
    pub badge: Option<crate::weather::badge::Condition>,
    /// First line, always present. Empty means "no data yet".
    pub primary: String,
    /// Second line, for modules that show two rows (network up over down).
    pub secondary: Option<String>,
    /// True when the underlying source could not be read. The strip renders
    /// these dimmed rather than showing a stale number, which is the rule the
    /// Mac app follows: readings that stop arriving show as unavailable
    /// instead of as old data.
    pub unavailable: bool,
}

impl Readout {
    pub fn one(primary: impl Into<String>) -> Self {
        Readout { primary: primary.into(), ..Default::default() }
    }

    pub fn two(primary: impl Into<String>, secondary: impl Into<String>) -> Self {
        Readout {
            primary: primary.into(),
            secondary: Some(secondary.into()),
            ..Default::default()
        }
    }

    pub fn unavailable() -> Self {
        Readout { primary: "--".into(), unavailable: true, ..Default::default() }
    }

    /// Sizes this readout's column for `text` rather than for its content.
    pub fn reserving(mut self, text: impl Into<String>) -> Self {
        self.reserved = Some(text.into());
        self
    }

    /// Marks this readout as the weather, drawn with `condition` around it.
    pub fn with_badge(mut self, condition: crate::weather::badge::Condition) -> Self {
        self.badge = Some(condition);
        self
    }
}

/// A source of one reading, sampled on a timer.
///
/// Implementations are polled from a single sampling thread, so they need no
/// interior locking; the strip reads a snapshot the thread publishes.
pub trait Module: Send {
    fn id(&self) -> ModuleId;

    /// Takes whatever the user has chosen that this module acts on.
    ///
    /// A default that does nothing, because most modules have no preference to
    /// honor: what is shown and in what order is decided by the strip, not
    /// here. Passing the whole settings rather than one argument per module
    /// keeps the trait from growing a method every time somebody wants a new
    /// choice, and keeps each module the only place that knows which parts of
    /// the settings are its business.
    fn configure(&mut self, settings: &crate::store::Settings) {
        let _ = settings;
    }

    /// Take one reading. Called at the sampling interval.
    ///
    /// This runs on the sampling thread and must not block on the network or
    /// on a dialog: a module that needs to go out to the internet (Weather)
    /// owns its own worker and returns whatever it last received.
    fn sample(&mut self);

    /// The current reading, formatted.
    fn readout(&self) -> Readout;

    /// Every sensor this module can see, for a settings pane that lets the
    /// user pick one.
    ///
    /// Empty for every module but Sensors, and defaulted here rather than
    /// downcast at the call site: the modules are held as trait objects, and
    /// asking "is this the sensors one" from outside would mean carrying `Any`
    /// through the whole list to answer a question one module can answer for
    /// itself.
    fn sensors(&self) -> Vec<crate::sensors::Sensor> {
        Vec::new()
    }

    /// The module's headline as a fraction of one - processor busy, GPU
    /// busy - for a panel drawing a graph of it. None where the reading is
    /// not a fraction or is not available.
    fn fraction(&self) -> Option<f32> {
        None
    }

    /// Two rates in bytes per second - read and write, down and up - for the
    /// disk and network panels. None for every other module.
    fn rates(&self) -> Option<(f64, f64)> {
        None
    }

    /// Used and total bytes, for the memory panel.
    fn memory(&self) -> Option<(u64, u64)> {
        None
    }

    /// Asks the module to read again now rather than at its next interval,
    /// for a panel's Refresh. Nothing to do for a module that samples every
    /// tick anyway.
    fn refresh(&mut self) {}

    /// Page file in use and its size, in bytes, for the memory panel's row
    /// where the Mac shows swap.
    fn page_file(&self) -> Option<(u64, u64)> {
        None
    }

    /// One of this module's readings, formatted for a stack: "45%", "12.3
    /// GB", "1.2 MB/s". None for a reading the module cannot produce, which
    /// the stack shows as a dash rather than a number it does not have.
    ///
    /// This is the macOS app's per-metric store, folded into the module:
    /// there is one sample and many readings of it, and the stack asks for
    /// the reading it wants.
    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        let _ = (metric, temperature);
        None
    }

    /// Every interface the counters offer, by name, with whether it is a
    /// tunnel - which the picker says out loud, as the Swift's does.
    fn net_interfaces(&self) -> Vec<(String, bool)> {
        Vec::new()
    }

    /// The one-, five- and fifteen-minute load averages, for the CPU panel
    /// and the stack's load reading.
    fn load_average(&self) -> Option<crate::modules::LoadAverage> {
        None
    }

    /// The processor's busy share split into user and kernel time, each of
    /// the whole, for the CPU panel.
    fn cpu_split(&self) -> Option<(f32, f32)> {
        None
    }

    /// Each engine type's share of the adapter, busiest first, for the GPU
    /// panel: ("3D", 0.63), ("VideoDecode", 0.2).
    fn gpu_engines(&self) -> Vec<(String, f32)> {
        Vec::new()
    }

    /// Dedicated adapter memory in use and how much there is, in bytes; the
    /// total is zero where the kernel would not say.
    fn gpu_memory(&self) -> Option<(u64, u64)> {
        None
    }

    /// Each physical disk with its rates, for the disks panel.
    fn disk_devices(&self) -> Vec<crate::modules::DiskDevice> {
        Vec::new()
    }

    /// The volumes with a drive letter, for a settings pane that lets the
    /// user pick which one the space readings are about.
    ///
    /// Whatever the disks module's own scan last found, so asking costs
    /// nothing: that scan already runs on a thread of its own because a
    /// sleeping network drive takes the SMB timeout to answer.
    fn volumes(&self) -> Vec<crate::volumes::Volume> {
        Vec::new()
    }

    /// Why the sensor source has nothing, if it has nothing.
    fn sensor_error(&self) -> Option<crate::sensors::SensorError> {
        None
    }

    /// Where the weather worker decided the machine is, when nothing was
    /// saved, and why it could not decide if it could not.
    ///
    /// The settings pane's "Use current location" row is written from these,
    /// and had nothing to write from: the fields existed on the snapshot and
    /// nothing ever filled them, so the row said "Finding your location..."
    /// for as long as the window stayed open, whether the lookup had already
    /// succeeded or already failed. Defaulted, like the rest of these, so
    /// only the module that has an answer has to say anything.
    fn located(&self) -> Option<crate::weather::models::Location> {
        None
    }

    /// Why there is no weather, if there is none.
    fn weather_error(&self) -> Option<String> {
        None
    }

    /// The graphics adapters the GPU module can read, as (key, name), for a
    /// settings pane that lets the user pick one. Empty for every other
    /// module, and defaulted here for the same reason `sensors` is.
    fn gpu_adapters(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// The weather module's whole observation, for its detail panel.
    ///
    /// None for every module but Weather, and defaulted here for the same
    /// reason `sensors` is: the modules are trait objects, and the panel
    /// needs the location, the units and the reason for a missing reading,
    /// none of which fit in a `Readout`.
    fn weather(&self) -> Option<crate::modules::Observation> {
        None
    }

    /// What hovering this module says, as lines.
    ///
    /// The strip shows a number; this is everything else the module knows and
    /// has nowhere to put - which interface a rate came from, how much memory
    /// a percentage is, which sensor a temperature was read off. The default
    /// is the module's own name and its reading, which is worth having on its
    /// own: on a strip of bare numbers, a tooltip that says which is which is
    /// the difference between a readout and a puzzle.
    fn detail(&self) -> Vec<String> {
        let readout = self.readout();
        let mut lines = vec![self.id().title().to_string()];
        if readout.unavailable {
            lines.push("Not available".to_string());
            return lines;
        }
        lines.push(readout.primary);
        if let Some(second) = readout.secondary {
            lines.push(second);
        }
        lines
    }
}
