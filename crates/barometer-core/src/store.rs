// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Settings, between runs.
//
// Ported from Sources/MenuBarStatsCore/Settings/{AppSettings,SettingsStore}.swift.
// macOS keeps the whole document in UserDefaults under one key; Windows has no
// defaults domain, so it is a file - %APPDATA%\Barometer\settings.json - and
// this module is the plumbing macOS gets for free.
//
// Two divergences from the Swift, both forced by the port:
//
//   - `modules` is an ordered array here, not a dictionary keyed by module. On
//     macOS the menu bar holds the order, because each module is its own
//     draggable item; there is one strip on Windows, so the order is ours to
//     keep and the array *is* the strip order.
//   - There is no schemaVersion. macOS bumps one and migrates inside
//     `init(from decoder:)`. Forward compatibility here is carried by
//     preserving keys instead, which is the property that actually matters
//     when two versions of the app share one file, and an unread version
//     number that this code wrote back at its own value would be worse than
//     none at all.
//
// Two rules the rest of this file exists to keep:
//
// A file written by a newer Barometer survives being opened by an older one.
// Loading parses the whole document into a `Value` and keeps it; saving merges
// what this version knows over that original rather than replacing it, so a
// key this version has never heard of goes back out untouched.
//
// A good file is never lost. The new document is encoded, parsed back and
// compared before anything on disk is touched, then written to a temporary
// beside the target and renamed over it, so a crash or a full disk cannot
// leave a half-written file where the settings were. The sibling TrafficMonitor
// checkout carries the rule that came out of exactly this going wrong there: a
// config file was destroyed by a careless read-modify-write round trip, and
// the note that survived it says to assert the result before replacing the
// original and never to edit in place. A file that cannot be parsed is moved
// aside rather than written over - it is the only copy of whatever the user
// had configured.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::module::ModuleId;
use crate::format::RateUnit;
use crate::settings::{FontWeight, StripFont};
use crate::weather::models::{
    Location, PrecipitationUnit, PressureUnit, TemperatureUnit, WeatherSettings, WeatherUnits,
    WindSpeedUnit,
};

/// The gap between columns a fresh install lays out with, in DIPs.
///
/// Compact from the first run, and that is deliberate. On a taskbar the
/// readout competes for room with the task buttons, and the task list gives
/// that room up a whole button at a time: every pixel the readout spends
/// between its columns is a pixel that can tip another button into the
/// overflow menu. Three is enough that two columns of digits still read as
/// two numbers rather than one long one, and no wider. Fourteen, the first
/// figure, was far too much.
pub const DEFAULT_COLUMN_GAP_DIP: f32 = 3.0;

/// What the strip's layout will accept for the gap and the padding, in DIPs.
///
/// Not the range the settings pane offers - it can be narrower - but the guard
/// on a hand-edited file. A negative gap lays the columns out over each other
/// and a large one reserves a taskbar's width of nothing.
pub const MIN_SPACING_DIP: f32 = 0.0;
/// See [`MIN_SPACING_DIP`].
pub const MAX_SPACING_DIP: f32 = 48.0;

/// How often the sensor helper may be asked, in seconds.
///
/// A read walks every device LibreHardwareMonitor knows about and takes
/// hundreds of milliseconds, so anything below a second is a worker thread
/// that never stops reading hardware; above a minute the readout is a
/// temperature from another era.
pub const MIN_POLL_SECONDS: u32 = 1;
/// See [`MIN_POLL_SECONDS`].
pub const MAX_POLL_SECONDS: u32 = 60;

/// One module's place on the strip.
///
/// Order is the array's order, so moving a module in the settings pane is
/// moving this entry. `enabled` is the module's own item, which is the same
/// switch `ModuleSettings::is_enabled` carries on the macOS side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleEntry {
    pub id: ModuleId,
    pub enabled: bool,
}

/// Choices for the Sensors module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensorSettings {
    /// The source's stable identifier for the reading shown on the strip, e.g.
    /// `/amdcpu/0/temperature/2`. Identifier rather than display name because
    /// names repeat across devices - see `sensors::Sensor::id`.
    pub pinned_sensor_id: Option<String>,
    pub poll_seconds: u32,
    /// What hardware temperatures are shown in.
    ///
    /// Its own setting rather than the weather's, and that is the whole point:
    /// wanting the forecast in Fahrenheit and a die temperature in Celsius is
    /// a perfectly ordinary combination, because they are read against
    /// different reference points. Nobody knows what 85 degrees means for a
    /// processor in the units they use for the weather.
    ///
    /// Celsius by default, which is what every sensor reports natively and
    /// what every other hardware monitor shows.
    pub temperature: TemperatureUnit,
    /// Digits after the point on a temperature, from SensorSettings.
    ///
    /// One by default, as on the Mac: a die temperature that moves half a
    /// degree between ticks reads as still at zero places, and the Windows
    /// build passed a hardcoded 0 to the panel, so the setting the Mac has
    /// had no counterpart here at all. Clamped where it is used, and again
    /// here, so a hand-edited file cannot ask for eight.
    pub decimal_places: u32,
}

impl Default for SensorSettings {
    fn default() -> Self {
        // The interval the sensors module already polls at.
        SensorSettings {
            pinned_sensor_id: None,
            poll_seconds: 2,
            temperature: TemperatureUnit::Celsius,
            decimal_places: 1,
        }
    }
}

/// Everything Barometer remembers between runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub font: StripFont,
    /// The strip, in strip order, with each module's own item switched on or off.
    pub modules: Vec<ModuleEntry>,
    pub column_gap_dip: f32,
    /// Whether the network column puts upload above download.
    ///
    /// Download over upload is the macOS app's order and the default here, but
    /// which of two stacked rates you want your eye to land on first is a
    /// genuine preference and not a small one for somebody watching an upload.
    pub network_upload_first: bool,
    /// Whether network throughput is counted in bytes or in bits.
    pub network_unit: RateUnit,
    /// Whether the network panel looks up the public address. Off unless the
    /// user says: it is a request to a service on the internet.
    pub network_shows_public_ip: bool,
    /// The interface to follow, by the name Windows shows, or None to add
    /// every one of them up.
    pub network_interface: Option<String>,
    pub weather: WeatherSettings,
    pub sensors: SensorSettings,
    /// The stacks the user composed.
    pub stacks: crate::stack::StacksSettings,
    /// Modules and stacks in strip order, interleaved. `modules` is kept in
    /// agreement with it, so a reader of `modules` alone still sees the
    /// modules in the right relative order; this is what places a stack
    /// among them.
    pub order: Vec<crate::stack::StripItem>,
    pub gpu: crate::settings::GpuChoice,
    /// Which physical disk the throughput readings are about, as the
    /// counter's own instance name - "1 D: E:".
    ///
    /// None means every disk added together, which is the whole machine's
    /// throughput and what the readings used to be fixed at. That is the
    /// right default and the wrong only choice: a Mac has one built-in SSD
    /// and a PC routinely has three or four, and "how busy is the machine"
    /// is a different question from "is the scratch drive the bottleneck".
    pub disk_device: Option<String>,
    /// Which volume the space readings are about, as its mount - "D:".
    ///
    /// None means the volume Windows itself booted from, which is what the
    /// readings used to be hardwired to as the literal "C:". That is wrong
    /// twice over: a machine can boot from another letter, and somebody
    /// watching their space is usually watching the drive that fills up
    /// rather than the one Windows is on.
    pub disk_volume: Option<String>,
    pub check_for_updates: bool,
    /// A release the user chose to skip. `update::should_offer` hides exactly
    /// this version from the automatic check and no other.
    pub skipped_update: Option<String>,
    /// When the automatic check last ran, in Unix seconds.
    ///
    /// Remembered in the file rather than in memory so that closing Barometer
    /// is not a way of asking again: the settings pane promises one check a
    /// week, and a machine that is restarted daily would otherwise make seven.
    /// None means it has never run, which is treated as due.
    pub last_update_check: Option<u64>,
    /// Where LibreHardwareMonitor was found or put.
    ///
    /// This is a machine fact in a file that roams: %APPDATA% follows the user
    /// to their other PC, where this path may name nothing. The caller checks
    /// the directory rather than trusting it, which it has to do anyway - LHM
    /// is frequently run portable from a zip and gets moved.
    pub library_directory: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            font: StripFont::default(),
            modules: ModuleId::ALL
                .into_iter()
                .map(|id| ModuleEntry { id, enabled: shown_on_a_fresh_install(id) })
                .collect(),
            column_gap_dip: DEFAULT_COLUMN_GAP_DIP,
            network_upload_first: false,
            network_unit: RateUnit::default(),
            network_shows_public_ip: false,
            network_interface: None,
            weather: WeatherSettings::default(),
            sensors: SensorSettings::default(),
            stacks: crate::stack::StacksSettings::default(),
            order: Vec::new(),
            gpu: crate::settings::GpuChoice::default(),
            disk_device: None,
            disk_volume: None,
            check_for_updates: true,
            skipped_update: None,
            last_update_check: None,
            library_directory: None,
        }
    }
}

/// Which modules a fresh install shows.
///
/// CPU and memory, which is `AppSettings.defaultModules` in the Swift: every
/// other module is off there too. It is also the only set that reads something
/// on every machine with nothing installed and nothing configured, since
/// sensors needs a helper and weather needs a location, and both would
/// otherwise spend strip width on "--" before the user had asked for anything.
fn shown_on_a_fresh_install(id: ModuleId) -> bool {
    matches!(id, ModuleId::Cpu | ModuleId::Memory)
}

/// A settings file that could not be used, and what became of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corrupt {
    /// Where the file was moved to, so the user can still get at whatever they
    /// had configured.
    ///
    /// `None` means it could not be moved and is still sitting at the settings
    /// path. The store then refuses to save over it, because it is the only
    /// copy of something the caller has already been told was kept.
    pub saved_as: Option<PathBuf>,
    /// What went wrong, for the message the caller shows.
    pub reason: String,
}

/// What a load produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Load {
    /// Always usable. A missing or unreadable file yields defaults.
    pub settings: Settings,
    /// Set when there was a file and it did not survive being read. The
    /// caller has settings either way and has to decide whether to say so.
    pub corrupt: Option<Corrupt>,
}

/// The settings file, and everything it held when it was last read.
pub struct Store {
    path: PathBuf,
    /// The document exactly as it was parsed.
    ///
    /// Keeping it is the whole reason loading goes through a `Value` rather
    /// than straight into `Settings`: a save merges over this, so keys written
    /// by a version that knows more than this one are still there afterwards.
    original: Map<String, Value>,
    /// Set when a load left a file it could not read and could not move aside.
    refuse_to_overwrite: Option<String>,
}

impl Store {
    /// `%APPDATA%\Barometer\settings.json`.
    ///
    /// Roaming rather than local: the font, the module order and the units are
    /// the user's choices and should follow them between machines.
    pub fn app_data_path() -> io::Result<PathBuf> {
        let base = std::env::var_os("APPDATA").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set for this process")
        })?;
        Ok(Path::new(&base).join("Barometer").join("settings.json"))
    }

    /// The store at the standard location.
    pub fn open() -> io::Result<Store> {
        Ok(Store::at(Store::app_data_path()?))
    }

    /// A store at a path of the caller's choosing.
    pub fn at(path: impl Into<PathBuf>) -> Store {
        Store { path: path.into(), original: Map::new(), refuse_to_overwrite: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the file, forgivingly.
    ///
    /// Never writes and never creates: a first run must not leave a settings
    /// file behind for a user who only ever started the app once.
    pub fn load(&mut self) -> Load {
        self.original = Map::new();
        self.refuse_to_overwrite = None;

        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            // Never having had settings is the normal first run, not a
            // failure, and it is what every fresh install looks like.
            Err(why) if why.kind() == io::ErrorKind::NotFound => {
                return Load { settings: Settings::default(), corrupt: None }
            }
            // Present but unreadable is a different thing entirely, and the
            // one case where carrying on quietly would end with the file
            // being replaced by defaults on the next save.
            Err(why) => {
                let reason = format!("could not be read: {why}");
                self.refuse_to_overwrite = Some(reason.clone());
                return Load {
                    settings: Settings::default(),
                    corrupt: Some(Corrupt { saved_as: None, reason }),
                };
            }
        };

        let document = match Self::document_from(&bytes) {
            Ok(document) => document,
            Err(why) => return self.set_aside(&why),
        };

        let settings = decode(&document);
        if let Value::Object(map) = document {
            self.original = map;
        }
        Load { settings, corrupt: None }
    }

    /// Reads a settings document out of bytes, touching no file.
///
/// What `load` does between reading and decoding, on its own, for a file
/// the user pointed at rather than the one the store owns. An import goes
/// through here because `load` moves a file it cannot read out of the way,
/// which is right for its own file and unforgivable for somebody else's.
pub fn parse(bytes: &[u8]) -> Result<Settings, String> {
    Self::document_from(bytes).map(|document| decode(&document))
}

/// The JSON object in some bytes, or why there is not one.
fn document_from(bytes: &[u8]) -> Result<Value, String> {
    // Notepad, and Windows PowerShell's `Set-Content -Encoding utf8`, put a
    // byte order mark in front of UTF-8. The parser refuses it, and a person
    // who has just edited their settings by hand and been told the file is
    // corrupt has been told wrong.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match serde_json::from_slice::<Value>(bytes) {
        Ok(document @ Value::Object(_)) => Ok(document),
        // Valid JSON of the wrong shape is as unusable as invalid JSON, and a
        // zero-length file - which is what a crash mid-write used to leave
        // before saves became atomic - lands here too.
        Ok(_) => Err("is not a settings document".to_string()),
        Err(why) => Err(format!("is not valid JSON: {why}")),
    }
}

/// Moves an unusable file out of the way and returns defaults.
    ///
    /// Moved, never deleted and never written over: it holds whatever the user
    /// had configured, and a corrupt file is often one bad byte away from
    /// being readable by hand.
    fn set_aside(&mut self, reason: &str) -> Load {
        let destination = aside(&self.path);
        let corrupt = match fs::rename(&self.path, &destination) {
            Ok(()) => Corrupt {
                saved_as: Some(destination),
                reason: format!("the settings file {reason}"),
            },
            Err(why) => {
                let reason = format!("the settings file {reason}, and could not be moved: {why}");
                self.refuse_to_overwrite = Some(reason.clone());
                Corrupt { saved_as: None, reason }
            }
        };
        Load { settings: Settings::default(), corrupt: Some(corrupt) }
    }

    /// Writes the file, strictly.
    ///
    /// Out-of-range numbers are clamped on the way out as well as on the way
    /// in, so a value the settings pane never offered cannot be persisted by a
    /// caller that built a `Settings` by hand.
    pub fn save(&mut self, settings: &Settings) -> io::Result<()> {
        if let Some(reason) = &self.refuse_to_overwrite {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("refusing to write over the file at {}: {reason}", self.path.display()),
            ));
        }

        let document = merged(Some(&Value::Object(self.original.clone())), encode(settings));

        let mut bytes = serde_json::to_vec_pretty(&document)
            .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why))?;
        // People do open this file. A trailing newline costs a byte and makes
        // it behave in an editor and on a terminal.
        bytes.push(b'\n');

        // Assert the parse before anything on disk is touched. This is the
        // TrafficMonitor rule: what gets written is checked against what was
        // meant while the original is still the only thing on disk, so a bug
        // in the encoding above costs a failed save rather than the settings.
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(reread) if reread == document => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the encoded settings did not read back as themselves; the file was left alone",
                ))
            }
        }

        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory)?;
        }

        // The temporary sits beside the target, not in %TEMP%: a rename is
        // only atomic within one volume, and the system temp directory is
        // frequently on another. The process id is in the name so two
        // Barometers saving at once cannot write over each other's half-built
        // file and rename the result into place.
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "settings.json".to_string());
        let temporary = self.path.with_file_name(format!("{name}.{}.tmp", std::process::id()));

        // A temporary left behind is a complete copy of the user's settings
        // under a name nothing will ever read again, so both failure paths
        // clear it.
        // Written and flushed to the disk before the rename, rather than
        // with `fs::write`, which returns as soon as the bytes are in the
        // cache. The rename is what makes the new file the settings, and a
        // power loss between a cached write and a committed rename leaves
        // settings.json existing and empty - which this store then reports as
        // corrupt and moves aside, having lost everything for a crash it was
        // built to survive.
        let flushed = (|| -> io::Result<()> {
            let file = fs::File::create(&temporary)?;
            {
                let mut writer = io::BufWriter::new(&file);
                io::Write::write_all(&mut writer, &bytes)?;
                io::Write::flush(&mut writer)?;
            }
            file.sync_all()
        })();
        if let Err(why) = flushed {
            let _ = fs::remove_file(&temporary);
            return Err(why);
        }
        // std's rename is MoveFileExW with MOVEFILE_REPLACE_EXISTING, so the
        // old file is replaced in one step and there is no moment where the
        // settings path holds nothing.
        if let Err(why) = fs::rename(&temporary, &self.path) {
            let _ = fs::remove_file(&temporary);
            return Err(why);
        }

        if let Value::Object(map) = document {
            self.original = map;
        }
        Ok(())
    }
}

/// Where an unusable settings file is put so the user can still get at it.
///
/// Stamped with the time rather than given a fixed `.corrupt` suffix: a second
/// bad start must not write over the first file moved aside, which may be the
/// one that still has the user's configuration in it.
fn aside(path: &Path) -> PathBuf {
    let seconds =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs()).unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_string());
    let mut candidate = path.with_file_name(format!("{name}.corrupt-{seconds}"));
    // A clock with one-second resolution is not a unique name by itself.
    let mut nth = 1;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.corrupt-{seconds}-{nth}"));
        nth += 1;
    }
    candidate
}

/// Lays `new` over `old`, keeping everything of `old` that `new` does not name.
///
/// Recursive rather than top-level only, because a version that adds a setting
/// is at least as likely to add it inside `font` or `weather` as beside them.
/// Arrays of objects are matched by `id` - which is what both `modules` and
/// `weather.locations` are keyed by - so a future per-module setting written
/// against an entry survives this version rewriting that entry's order and
/// enabled flag. An element `new` does not list is gone, which is how a module
/// dropped or a location deleted actually leaves.
fn merged(old: Option<&Value>, new: Value) -> Value {
    match new {
        Value::Object(fresh) => {
            let mut out = match old {
                Some(Value::Object(existing)) => existing.clone(),
                _ => Map::new(),
            };
            for (key, value) in fresh {
                let previous = out.remove(&key);
                out.insert(key, merged(previous.as_ref(), value));
            }
            Value::Object(out)
        }
        Value::Array(fresh) => {
            let existing: &[Value] = match old {
                Some(Value::Array(items)) => items,
                _ => &[],
            };
            Value::Array(
                fresh
                    .into_iter()
                    .map(|item| {
                        let previous = identity(&item).and_then(|id| {
                            existing.iter().find(|other| identity(other) == Some(id))
                        });
                        merged(previous, item)
                    })
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

/// What an array element calls itself, for matching it against its old self.
///
/// Whatever kind of scalar the id is written as: `modules` and
/// `weather.locations` name themselves with a string, `stacks` with a number.
/// Requiring a string here meant no stack ever matched its predecessor, so
/// every stack was rebuilt from nothing and a newer version's per-stack
/// setting was erased by the first save an older build made - which is the
/// one loss this merge exists to prevent.
fn identity(value: &Value) -> Option<&Value> {
    value.get("id").filter(|id| !id.is_object() && !id.is_array())
}

/// A string field, with empty read as absent.
///
/// Empty can only come from a hand-edit or from a text field the user cleared,
/// and everywhere else here "not set" is spelled `None`. Treating the two the
/// same is what keeps a save-then-load round trip total.
fn text(parent: Option<&Value>, key: &str) -> Option<String> {
    parent?.get(key)?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn flag(parent: Option<&Value>, key: &str) -> Option<bool> {
    parent?.get(key)?.as_bool()
}

fn number(parent: Option<&Value>, key: &str) -> Option<f64> {
    parent?.get(key)?.as_f64()
}

/// `None` and the empty string are both written as null.
fn optional(value: &Option<String>) -> Value {
    match value {
        Some(text) if !text.is_empty() => Value::String(text.clone()),
        _ => Value::Null,
    }
}

fn spacing(dip: f32) -> f32 {
    dip.clamp(MIN_SPACING_DIP, MAX_SPACING_DIP)
}

fn poll(seconds: u32) -> u32 {
    seconds.clamp(MIN_POLL_SECONDS, MAX_POLL_SECONDS)
}

/// The document this version writes.
///
/// Keys are camelCase, and every enum goes out through its `raw_value()`. Both
/// are the macOS document's own conventions - `inchesOfMercury` is already
/// spelled that way in `WeatherModels.swift` - and the raw values exist
/// precisely so that persistence never depends on the order Rust happens to
/// declare a variant in.
fn encode(settings: &Settings) -> Value {
    let weather = &settings.weather;
    json!({
        "font": {
            "family": settings.font.family,
            "headingFamily": optional(&settings.font.heading_family),
            "weight": settings.font.weight.raw_value(),
            "headingWeight": settings.font.heading_weight.raw_value(),
        },
        "modules": settings.modules.iter().map(|entry| json!({
            "id": entry.id.key(),
            "enabled": entry.enabled,
        })).collect::<Vec<Value>>(),
        "columnGapDip": spacing(settings.column_gap_dip),
        "networkUploadFirst": settings.network_upload_first,
        "networkUnit": settings.network_unit.raw_value(),
        "networkShowsPublicIP": settings.network_shows_public_ip,
        "networkInterface": settings.network_interface.clone().map(Value::from).unwrap_or(Value::Null),
        "weather": {
            "locations": weather.locations.iter().map(|place| json!({
                "id": place.id,
                "name": place.name,
                "admin": place.admin,
                "country": place.country,
                "latitude": place.latitude,
                "longitude": place.longitude,
                "timeZone": place.time_zone,
            })).collect::<Vec<Value>>(),
            "primaryLocationId": optional(&weather.primary_location_id),
            "usesCurrentLocation": weather.uses_current_location,
            "units": {
                "temperature": weather.units.temperature.raw_value(),
                "windSpeed": weather.units.wind_speed.raw_value(),
                "pressure": weather.units.pressure.raw_value(),
                "precipitation": weather.units.precipitation.raw_value(),
            },
            "refreshIntervalMinutes": weather.clamped_refresh_minutes(),
            "usesColorIcons": weather.uses_color_icons,
        },
        "sensors": {
            "pinnedSensorId": optional(&settings.sensors.pinned_sensor_id),
            "pollSeconds": poll(settings.sensors.poll_seconds),
            "temperature": settings.sensors.temperature.raw_value(),
            "decimalPlaces": settings.sensors.decimal_places.min(2),
        },
        "stacks": settings.stacks.stacks.iter().map(|stack| json!({
            "id": stack.id,
            "enabled": stack.is_enabled,
            "name": stack.name,
            "layout": stack.layout.raw_value(),
            "hidesSourceItems": stack.hides_source_items,
            // Each reading is an object so the label can ride beside it;
            // a bare string is still read, for a file from before labels.
            "metrics": stack.metrics_for_file().into_iter().map(|entry| match entry {
                Ok(entry) => json!({
                    "metric": entry.metric.raw_value(),
                    "label": optional(&entry.label),
                }),
                // A reading a newer Barometer wrote and this one cannot read.
                // Written back as it was found rather than dropped.
                Err(raw) => json!({ "metric": raw }),
            }).collect::<Vec<Value>>(),
        })).collect::<Vec<Value>>(),
        "nextStackId": settings.stacks.next_id(),
        "order": settings.order.iter().map(|item| Value::String(item.raw_value())).collect::<Vec<Value>>(),
        "gpuAdapter": match &settings.gpu {
            crate::settings::GpuChoice::Automatic => Value::Null,
            crate::settings::GpuChoice::Adapter { key, name } => json!({ "key": key, "name": name }),
        },
        "diskDevice": optional(&settings.disk_device),
        "diskVolume": optional(&settings.disk_volume),
        "checkForUpdates": settings.check_for_updates,
        "skippedUpdate": optional(&settings.skipped_update),
        "lastUpdateCheck": settings.last_update_check.map(Value::from).unwrap_or(Value::Null),
        "libraryDirectory": optional(&settings.library_directory),
    })
}

/// Reads what this version knows, and defaults for everything else.
///
/// Nothing in here can fail. A field that is missing, of the wrong type or out
/// of range costs that field and nothing more, because the alternative -
/// refusing the document - throws away every other setting the user had over
/// one bad line.
fn decode(document: &Value) -> Settings {
    let root = Some(document);
    let defaults = Settings::default();
    let font = document.get("font");

    Settings {
        font: StripFont {
            family: text(font, "family").unwrap_or(defaults.font.family),
            heading_family: text(font, "headingFamily"),
            weight: text(font, "weight")
                .and_then(|raw| FontWeight::from_raw(&raw))
                .unwrap_or(defaults.font.weight),
            heading_weight: text(font, "headingWeight")
                .and_then(|raw| FontWeight::from_raw(&raw))
                .unwrap_or(defaults.font.heading_weight),
        },
        modules: decode_modules(document.get("modules")),
        column_gap_dip: number(root, "columnGapDip")
            .map(|dip| spacing(dip as f32))
            .unwrap_or(defaults.column_gap_dip),
        network_upload_first: flag(root, "networkUploadFirst")
            .unwrap_or(defaults.network_upload_first),
        network_unit: text(root, "networkUnit")
            .and_then(|raw| RateUnit::from_raw(&raw))
            .unwrap_or(defaults.network_unit),
        network_shows_public_ip: flag(root, "networkShowsPublicIP")
            .unwrap_or(defaults.network_shows_public_ip),
        // An empty name is nobody's interface and means the same as absent.
        network_interface: text(root, "networkInterface").filter(|name| !name.trim().is_empty()),
        weather: decode_weather(document.get("weather")),
        sensors: decode_sensors(document.get("sensors")),
        stacks: decode_stacks(document.get("stacks"), number(root, "nextStackId")),
        order: decode_order(document.get("order")),
        gpu: decode_gpu(document.get("gpuAdapter")),
        disk_device: text(root, "diskDevice"),
        disk_volume: text(root, "diskVolume"),
        check_for_updates: flag(root, "checkForUpdates").unwrap_or(defaults.check_for_updates),
        skipped_update: text(root, "skippedUpdate"),
        last_update_check: number(root, "lastUpdateCheck")
            .filter(|seconds| *seconds >= 0.0)
            .map(|seconds| seconds as u64),
        library_directory: text(root, "libraryDirectory"),
    }
}

/// The stacks, forgivingly: a stack with no usable id is dropped, an unknown
/// metric is dropped from its stack, and a reading written as a bare string
/// - the form before labels - reads as that metric with no label.
fn decode_stacks(value: Option<&Value>, next_id: Option<f64>) -> crate::stack::StacksSettings {
    use crate::stack::{StackEntry, StackLayout, StackMetric, StackSettings, StacksSettings};
    let stacks: Vec<StackSettings> = value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = number(Some(item), "id")? as u32;
                    if id == 0 {
                        return None;
                    }
                    let written = item.get("metrics").and_then(Value::as_array);
                    let mut metrics: Vec<StackEntry> = Vec::new();
                    let mut unknown_metrics: Vec<(usize, String)> = Vec::new();
                    for (at, entry) in written.into_iter().flatten().enumerate() {
                        let raw = match entry {
                            Value::String(raw) => Some(raw.clone()),
                            Value::Object(_) => text(Some(entry), "metric"),
                            _ => None,
                        };
                        let Some(raw) = raw else { continue };
                        match StackMetric::from_raw(&raw) {
                            Some(metric) => {
                                let label = text(Some(entry), "label").filter(|l| !l.trim().is_empty());
                                metrics.push(StackEntry { metric, label });
                            }
                            // Kept with its place so saving writes it back
                            // where it was, instead of losing a reading this
                            // build happens not to know.
                            None => unknown_metrics.push((at, raw)),
                        }
                    }
                    Some(StackSettings {
                        id,
                        is_enabled: flag(Some(item), "enabled").unwrap_or(true),
                        name: text(Some(item), "name").unwrap_or_default(),
                        layout: text(Some(item), "layout")
                            .and_then(|raw| StackLayout::from_raw(&raw))
                            .unwrap_or_default(),
                        metrics,
                        unknown_metrics,
                        hides_source_items: flag(Some(item), "hidesSourceItems").unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    StacksSettings::from_parts(stacks, next_id.map(|n| n.max(1.0) as u32).unwrap_or(1))
}

/// The strip order. Anything unrecognized is dropped; the model repairs the
/// rest against what exists.
fn decode_order(value: Option<&Value>) -> Vec<crate::stack::StripItem> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(crate::stack::StripItem::from_raw)
                .collect()
        })
        .unwrap_or_default()
}

fn decode_gpu(value: Option<&Value>) -> crate::settings::GpuChoice {
    match (text(value, "key"), text(value, "name")) {
        (Some(key), name) if !key.is_empty() => {
            crate::settings::GpuChoice::Adapter { key, name: name.unwrap_or_default() }
        }
        _ => crate::settings::GpuChoice::Automatic,
    }
}

/// Matched on `key()` rather than through a second table, so a module added to
/// `ModuleId::ALL` cannot be forgotten here.
fn module_from_key(key: &str) -> Option<ModuleId> {
    ModuleId::ALL.into_iter().find(|id| id.key() == key)
}

fn decode_modules(value: Option<&Value>) -> Vec<ModuleEntry> {
    let Some(Value::Array(items)) = value else {
        // No list at all is a first run, or a file written before the strip
        // was persisted. Either way it wants the default strip; an empty list
        // here would mean an app that starts up showing nothing.
        return Settings::default().modules;
    };

    let mut entries: Vec<ModuleEntry> = Vec::with_capacity(ModuleId::ALL.len());
    for item in items {
        // A key this version does not know is dropped rather than kept as a
        // hole: it is a module removed since the file was written, or a typo
        // in a hand-edit, and neither has anything to draw.
        let Some(id) = item.get("id").and_then(Value::as_str).and_then(module_from_key) else {
            continue;
        };
        // A module listed twice would be laid out twice, sampled twice and
        // shown twice in the settings pane. The first place wins, because that
        // is the one the user dragged it to.
        if entries.iter().any(|entry| entry.id == id) {
            continue;
        }
        entries.push(ModuleEntry {
            id,
            enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(false),
        });
    }

    // A module that exists now but was not in the file is appended rather than
    // left out. The list is what the settings pane shows, so a module missing
    // from it is one the user has no way to switch on - invisible, with no
    // control anywhere. Appended switched off, though: this is an upgrade, and
    // a module that turns itself on during one has appeared on the strip
    // uninvited.
    for id in ModuleId::ALL {
        if !entries.iter().any(|entry| entry.id == id) {
            entries.push(ModuleEntry { id, enabled: false });
        }
    }
    entries
}

fn decode_weather(value: Option<&Value>) -> WeatherSettings {
    let defaults = WeatherSettings::default();
    let units = value.and_then(|v| v.get("units"));

    let mut settings = WeatherSettings {
        locations: decode_locations(value.and_then(|v| v.get("locations"))),
        primary_location_id: text(value, "primaryLocationId"),
        uses_current_location: flag(value, "usesCurrentLocation")
            .unwrap_or(defaults.uses_current_location),
        units: WeatherUnits {
            temperature: text(units, "temperature")
                .and_then(|raw| TemperatureUnit::from_raw(&raw))
                .unwrap_or(defaults.units.temperature),
            wind_speed: text(units, "windSpeed")
                .and_then(|raw| WindSpeedUnit::from_raw(&raw))
                .unwrap_or(defaults.units.wind_speed),
            pressure: text(units, "pressure")
                .and_then(|raw| PressureUnit::from_raw(&raw))
                .unwrap_or(defaults.units.pressure),
            precipitation: text(units, "precipitation")
                .and_then(|raw| PrecipitationUnit::from_raw(&raw))
                .unwrap_or(defaults.units.precipitation),
        },
        // Saturating rather than wrapping: a negative or absurd interval in
        // the file becomes 0 or u32::MAX here and is pulled into range below.
        refresh_interval_minutes: number(value, "refreshIntervalMinutes")
            .map(|minutes| minutes as u32)
            .unwrap_or(defaults.refresh_interval_minutes),
        uses_color_icons: flag(value, "usesColorIcons").unwrap_or(defaults.uses_color_icons),
    };

    settings.refresh_interval_minutes = settings.clamped_refresh_minutes();
    settings
}

fn decode_locations(value: Option<&Value>) -> Vec<Location> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let item = Some(item);
            // Without an identity or a position a location can neither be
            // requested from Open-Meteo nor matched to the primary selection,
            // so it is dropped rather than kept as a row that never loads.
            Some(Location {
                id: text(item, "id")?,
                name: text(item, "name").unwrap_or_default(),
                admin: text(item, "admin"),
                country: text(item, "country").unwrap_or_default(),
                latitude: coordinate(item, "latitude")?,
                longitude: coordinate(item, "longitude")?,
                time_zone: text(item, "timeZone").unwrap_or_default(),
            })
        })
        .collect()
}

/// Coordinates are strings here - `Location` keeps them as the source spelled
/// them so a round trip cannot shift a decimal place - but a number is
/// accepted too, for the same reason `weather::client` accepts both: a
/// hand-edited file should not silently lose somebody's town.
fn coordinate(parent: Option<&Value>, key: &str) -> Option<String> {
    let value = parent?.get(key)?;
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| value.as_f64().map(|n| n.to_string()))
}

fn decode_sensors(value: Option<&Value>) -> SensorSettings {
    let defaults = SensorSettings::default();
    SensorSettings {
        pinned_sensor_id: text(value, "pinnedSensorId"),
        poll_seconds: number(value, "pollSeconds")
            .map(|seconds| poll(seconds as u32))
            .unwrap_or(defaults.poll_seconds),
        temperature: text(value, "temperature")
            .and_then(|raw| TemperatureUnit::from_raw(&raw))
            .unwrap_or(defaults.temperature),
        decimal_places: number(value, "decimalPlaces")
            .map(|places| (places.max(0.0) as u32).min(2))
            .unwrap_or(defaults.decimal_places),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, taken away when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let path =
                std::env::temp_dir().join(format!("barometer-store-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch directory");
            Scratch(path)
        }

        fn settings(&self) -> PathBuf {
            self.0.join("settings.json")
        }

        fn write(&self, body: &str) -> PathBuf {
            let path = self.settings();
            fs::write(&path, body).expect("write fixture");
            path
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("read scratch")
                .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn austin() -> Location {
        Location {
            id: "austin-tx".into(),
            name: "Austin".into(),
            admin: Some("Texas".into()),
            country: "United States".into(),
            latitude: "30.2672".into(),
            longitude: "-97.7431".into(),
            time_zone: "America/Chicago".into(),
        }
    }

    /// Deliberately unlike the defaults in every field.
    fn configured() -> Settings {
        Settings {
            font: StripFont {
                family: "Cascadia Mono".into(),
                // Deliberately a different family from the values, which is
                // the whole point of the setting.
                heading_family: Some("Segoe UI Semibold".into()),
                weight: FontWeight::Semibold,
                heading_weight: FontWeight::Bold,
            },
            modules: vec![
                ModuleEntry { id: ModuleId::Weather, enabled: true },
                ModuleEntry { id: ModuleId::Network, enabled: true },
                ModuleEntry { id: ModuleId::Sensors, enabled: false },
                ModuleEntry { id: ModuleId::Cpu, enabled: true },
                ModuleEntry { id: ModuleId::Gpu, enabled: false },
                ModuleEntry { id: ModuleId::Memory, enabled: false },
                ModuleEntry { id: ModuleId::Disks, enabled: true },
            ],
            column_gap_dip: 20.0,
            network_upload_first: true,
            network_unit: RateUnit::Bits,
            network_shows_public_ip: true,
            network_interface: Some("Ethernet 2".into()),
            weather: WeatherSettings {
                locations: vec![austin()],
                primary_location_id: Some("austin-tx".into()),
                uses_current_location: false,
                units: WeatherUnits::METRIC,
                refresh_interval_minutes: 45,
                uses_color_icons: false,
            },
            sensors: SensorSettings {
                pinned_sensor_id: Some("/amdcpu/0/temperature/2".into()),
                poll_seconds: 5,
                // Fahrenheit here on purpose: the weather above is metric, so
                // the round trip proves the two units are stored separately
                // rather than one following the other.
                temperature: TemperatureUnit::Fahrenheit,
                decimal_places: 2,
            },
            stacks: crate::stack::StacksSettings::from_parts(
                vec![crate::stack::StackSettings {
                    id: 3,
                    is_enabled: true,
                    name: "Rig".into(),
                    layout: crate::stack::StackLayout::SingleRow,
                    metrics: vec![
                        crate::stack::StackEntry {
                            metric: crate::stack::StackMetric::CpuTotal,
                            label: Some("proc".into()),
                        },
                        crate::stack::StackEntry::new(crate::stack::StackMetric::Sensor(
                            "/amdcpu/0/temperature/2".into(),
                        )),
                    ],
                    // A reading a later version wrote, which this one keeps
                    // and writes back where it found it.
                    unknown_metrics: vec![(1, "cpu.quantum".to_string())],
                    hides_source_items: true,
                }],
                4,
            ),
            order: vec![crate::stack::StripItem::Stack(3), crate::stack::StripItem::Module(ModuleId::Cpu)],
            gpu: crate::settings::GpuChoice::Adapter {
                key: "10de:2684#0".into(),
                name: "NVIDIA GeForce RTX 4090".into(),
            },
            disk_device: Some("1 D: E:".into()),
            disk_volume: Some("D:".into()),
            check_for_updates: false,
            skipped_update: Some("1.4.0".into()),
            last_update_check: Some(1_757_000_000),
            library_directory: Some(r"D:\Tools\LibreHardwareMonitor".into()),
        }
    }

    fn document(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).expect("read settings")).expect("parse settings")
    }

    #[test]
    fn a_missing_file_is_a_first_run_and_not_a_failure() {
        let scratch = Scratch::new("missing");
        let load = Store::at(scratch.settings()).load();
        assert_eq!(load.settings, Settings::default());
        assert_eq!(load.corrupt, None);
        // Loading must not create the file: someone who starts the app once
        // and never opens settings should leave nothing behind.
        assert!(scratch.names().is_empty());
    }

    #[test]
    fn everything_this_version_knows_survives_a_round_trip() {
        let scratch = Scratch::new("round-trip");
        let mut store = Store::at(scratch.settings());
        store.save(&configured()).expect("save");

        let load = Store::at(scratch.settings()).load();
        assert_eq!(load.corrupt, None);
        assert_eq!(load.settings, configured());
    }

    #[test]
    fn module_order_and_enabled_state_come_back_exactly() {
        let scratch = Scratch::new("order");
        let mut store = Store::at(scratch.settings());
        store.save(&configured()).expect("save");

        // The array is the strip order, so a reordering that survives as a set
        // but not as a sequence is a bug the round-trip check above would miss
        // if `Settings` ever grew a set-like comparison.
        let modules = Store::at(scratch.settings()).load().settings.modules;
        assert_eq!(modules, configured().modules);
        assert_eq!(modules[0].id, ModuleId::Weather);
        assert_eq!(modules.last().unwrap().id, ModuleId::Disks);
    }

    #[test]
    fn a_file_saved_with_a_byte_order_mark_is_read_and_not_called_corrupt() {
        let scratch = Scratch::new("bom");
        scratch.write("\u{FEFF}{ \"columnGapDip\": 9.5 }");

        let mut store = Store::at(scratch.settings());
        let load = store.load();
        assert!(load.corrupt.is_none(), "{:?}", load.corrupt.map(|c| c.reason));
        assert_eq!(load.settings.column_gap_dip, 9.5);
    }

    #[test]
    fn a_reading_this_version_does_not_know_keeps_its_place_in_the_stack() {
        let scratch = Scratch::new("unknown-metric");
        scratch.write(
            r#"{
              "stacks": [{
                "id": 1,
                "name": "Rig",
                "metrics": [
                  { "metric": "cpu.total" },
                  { "metric": "cpu.quantumFlux", "label": "Flux" },
                  { "metric": "memory.usedPercent" }
                ]
              }]
            }"#,
        );

        let mut store = Store::at(scratch.settings());
        let settings = store.load().settings;
        let stack = &settings.stacks.stacks[0];
        // The two it understands are readings; the one it does not is kept
        // aside with the place it held.
        assert_eq!(stack.metrics.len(), 2);
        assert_eq!(stack.unknown_metrics, vec![(1, "cpu.quantumFlux".to_string())]);

        store.save(&settings).expect("save");
        store.save(&settings).expect("save again");

        let written = document(&scratch.settings());
        let metrics = written["stacks"][0]["metrics"].as_array().expect("metrics");
        let raw: Vec<&str> = metrics.iter().map(|m| m["metric"].as_str().unwrap_or_default()).collect();
        assert_eq!(raw, ["cpu.total", "cpu.quantumFlux", "memory.usedPercent"]);
    }

    #[test]
    fn a_newer_versions_keys_survive_being_opened_by_this_one() {
        let scratch = Scratch::new("unknown-keys");
        scratch.write(
            r#"{
              "modules": [
                { "id": "cpu", "enabled": true, "mode": "perCoreBars" },
                { "id": "memory", "enabled": false }
              ],
              "font": { "family": "Consolas", "italic": true },
              "weather": { "detailSections": ["wind", "pressure"] },
              "panels": [{ "id": "p1", "metrics": ["cpu", "gpu"] }],
              "schemaVersion": 99
            }"#,
        );

        let mut store = Store::at(scratch.settings());
        let settings = store.load().settings;
        store.save(&settings).expect("save");
        // Twice, because the second save must merge over the document this
        // version just wrote rather than over a stale copy of the original.
        store.save(&settings).expect("save again");

        let written = document(&scratch.settings());
        assert_eq!(written["schemaVersion"], 99);
        assert_eq!(written["panels"][0]["metrics"][1], "gpu");
        assert_eq!(written["font"]["italic"], true);
        assert_eq!(written["weather"]["detailSections"][0], "wind");
        // A per-module key belonging to an entry, kept against the entry it
        // was written for even though this version rewrote that entry.
        assert_eq!(written["modules"][0]["mode"], "perCoreBars");
        assert_eq!(written["modules"][0]["id"], "cpu");
    }

    #[test]
    fn a_newer_versions_key_on_a_stack_survives_even_though_stacks_are_numbered() {
        // The other two arrays name themselves with a string and the merge was
        // written for those; a stack's id is a number, so requiring a string
        // meant no stack matched its old self and every key beside the ones
        // this version writes was thrown away.
        let scratch = Scratch::new("unknown-stack-key");
        scratch.write(
            r##"{
              "stacks": [
                { "id": 1, "name": "Left", "metrics": ["cpu.total"], "tint": "#FF9500" },
                { "id": 2, "name": "Right", "metrics": ["memory.usedPercent"] }
              ]
            }"##,
        );

        let mut store = Store::at(scratch.settings());
        let settings = store.load().settings;
        store.save(&settings).expect("save");

        let written = document(&scratch.settings());
        assert_eq!(written["stacks"][0]["id"], 1);
        assert_eq!(written["stacks"][0]["tint"], "#FF9500");
        assert_eq!(written["stacks"][0]["name"], "Left");
    }

    #[test]
    fn a_module_added_since_the_file_was_written_is_offered_switched_off() {
        let scratch = Scratch::new("appended");
        scratch.write(r#"{ "modules": [{ "id": "cpu", "enabled": true }] }"#);

        let modules = Store::at(scratch.settings()).load().settings.modules;
        assert_eq!(modules.len(), ModuleId::ALL.len());
        assert_eq!(modules[0], ModuleEntry { id: ModuleId::Cpu, enabled: true });
        // Present, so the settings pane can list it and the user can turn it
        // on; off, because an upgrade must not put a module on the strip.
        assert!(modules[1..].iter().all(|entry| !entry.enabled));
        assert!(modules.iter().any(|entry| entry.id == ModuleId::Weather));
    }

    #[test]
    fn an_unknown_module_key_is_dropped_and_a_repeat_is_laid_out_once() {
        let scratch = Scratch::new("bad-modules");
        scratch.write(
            r#"{ "modules": [
                 { "id": "battery", "enabled": true },
                 { "id": "network", "enabled": true },
                 { "id": "network", "enabled": false }
               ] }"#,
        );

        let modules = Store::at(scratch.settings()).load().settings.modules;
        assert_eq!(modules[0], ModuleEntry { id: ModuleId::Network, enabled: true });
        assert_eq!(modules.iter().filter(|e| e.id == ModuleId::Network).count(), 1);
        assert_eq!(modules.len(), ModuleId::ALL.len());
    }

    #[test]
    fn a_file_the_store_wrote_reads_back_through_parse_unchanged() {
        let scratch = Scratch::new("parse");
        let mut settings = Settings::default();
        settings.column_gap_dip = 9.5;
        Store::at(scratch.settings()).save(&settings).unwrap();
        let bytes = fs::read(scratch.settings()).unwrap();
        assert_eq!(Store::parse(&bytes).unwrap(), settings);
        // Notepad's byte order mark is forgiven here as it is on load.
        let mut marked = b"\xEF\xBB\xBF".to_vec();
        marked.extend_from_slice(&bytes);
        assert_eq!(Store::parse(&marked).unwrap(), settings);
        // And a non-document is refused, with the file left where it was.
        assert!(Store::parse(b"[1, 2]").is_err());
        assert!(Store::parse(b"{").is_err());
        assert!(scratch.settings().exists());
    }

    #[test]
    fn an_out_of_range_number_is_clamped_rather_than_refused() {
        let scratch = Scratch::new("clamping");
        scratch.write(
            r#"{
              "columnGapDip": -5,
              "weather": { "refreshIntervalMinutes": 0 },
              "sensors": { "pollSeconds": 100000 },
              "checkForUpdates": false
            }"#,
        );

        let settings = Store::at(scratch.settings()).load().settings;
        assert_eq!(settings.column_gap_dip, MIN_SPACING_DIP);
        assert_eq!(settings.weather.refresh_interval_minutes, 5);
        assert_eq!(settings.sensors.poll_seconds, MAX_POLL_SECONDS);
        // One bad number costs that number and nothing else.
        assert!(!settings.check_for_updates);
    }

    #[test]
    fn a_field_of_the_wrong_type_costs_only_that_field() {
        let scratch = Scratch::new("wrong-types");
        scratch.write(
            r#"{
              "font": { "family": 12, "weight": "ultrablack" },
              "checkForUpdates": "yes",
              "skippedUpdate": "2.0.0"
            }"#,
        );

        let settings = Store::at(scratch.settings()).load().settings;
        assert_eq!(settings.font.family, StripFont::default().family);
        assert_eq!(settings.font.weight, FontWeight::Regular);
        assert!(settings.check_for_updates);
        assert_eq!(settings.skipped_update.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_rather_than_eaten() {
        let scratch = Scratch::new("corrupt");
        let body = "{ \"font\": { \"family\": \"Consolas\"  <-- half a file";
        scratch.write(body);

        let mut store = Store::at(scratch.settings());
        let load = store.load();
        assert_eq!(load.settings, Settings::default());
        let corrupt = load.corrupt.expect("the caller has to be told");
        let saved_as = corrupt.saved_as.expect("moved aside");

        // The user's file, byte for byte, under a name that says what happened.
        assert_eq!(fs::read_to_string(&saved_as).expect("read aside"), body);
        assert!(!scratch.settings().exists());
        assert!(saved_as.file_name().unwrap().to_string_lossy().contains(".corrupt-"));

        // And the next save writes a good file without touching the aside one.
        store.save(&Settings::default()).expect("save");
        assert_eq!(fs::read_to_string(&saved_as).expect("read aside"), body);
    }

    #[test]
    fn a_zero_length_file_left_by_a_crash_is_corrupt_not_defaults() {
        let scratch = Scratch::new("truncated");
        scratch.write("");
        let load = Store::at(scratch.settings()).load();
        assert!(load.corrupt.is_some());
        assert_eq!(load.settings, Settings::default());
    }

    #[test]
    fn valid_json_of_the_wrong_shape_is_corrupt() {
        let scratch = Scratch::new("array");
        scratch.write(r#"["cpu", "memory"]"#);
        let load = Store::at(scratch.settings()).load();
        assert!(load.corrupt.expect("told").saved_as.is_some());
    }

    #[test]
    fn a_settings_path_that_cannot_be_read_is_never_written_over() {
        let scratch = Scratch::new("unreadable");
        // A directory where the file should be. Contrived, but it is the one
        // failure that is reproducible everywhere, and it stands in for the
        // permissions and locking cases that are not.
        fs::create_dir(scratch.settings()).expect("directory in the way");

        let mut store = Store::at(scratch.settings());
        let load = store.load();
        let corrupt = load.corrupt.expect("told");
        assert_eq!(corrupt.saved_as, None, "nothing was moved, so nothing may be claimed");

        // The store refuses rather than replacing a file it has just told the
        // caller was left intact.
        let refused = store.save(&Settings::default()).expect_err("must refuse");
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        assert!(scratch.settings().is_dir());
        assert_eq!(scratch.names(), vec!["settings.json"]);
    }

    #[test]
    fn saving_leaves_no_temporary_behind() {
        let scratch = Scratch::new("no-litter");
        let mut store = Store::at(scratch.settings());
        store.save(&configured()).expect("save");
        store.save(&Settings::default()).expect("save again");
        assert_eq!(scratch.names(), vec!["settings.json"]);
    }

    #[test]
    fn a_save_that_cannot_be_written_reports_it_and_leaves_nothing_behind() {
        let scratch = Scratch::new("blocked");
        let blocker = scratch.0.join("blocker");
        fs::write(&blocker, "not a directory").expect("write blocker");

        let mut store = Store::at(blocker.join("settings.json"));
        store.save(&Settings::default()).expect_err("cannot create a directory under a file");
        assert_eq!(fs::read_to_string(&blocker).expect("read blocker"), "not a directory");
        assert_eq!(scratch.names(), vec!["blocker"]);
    }

    #[test]
    fn an_enum_is_persisted_by_its_raw_value_and_never_by_its_discriminant() {
        let scratch = Scratch::new("raw-values");
        let mut store = Store::at(scratch.settings());
        store.save(&configured()).expect("save");

        let written = document(&scratch.settings());
        assert_eq!(written["weather"]["units"]["temperature"], "celsius");
        assert_eq!(written["weather"]["units"]["pressure"], "hectopascals");
        assert_eq!(written["weather"]["units"]["precipitation"], "mm");
        assert_eq!(written["font"]["weight"], "semibold");
        assert_eq!(written["modules"][0]["id"], "weather");
    }

    #[test]
    fn a_value_the_settings_pane_would_never_offer_is_clamped_on_the_way_out() {
        let scratch = Scratch::new("strict-save");
        let mut store = Store::at(scratch.settings());
        store
            .save(&Settings {
                column_gap_dip: -100.0,
                sensors: SensorSettings { poll_seconds: 0, ..Default::default() },
                weather: WeatherSettings {
                    refresh_interval_minutes: 9999,
                    ..Default::default()
                },
                ..Settings::default()
            })
            .expect("save");

        let written = document(&scratch.settings());
        assert_eq!(written["columnGapDip"], 0.0);
        assert_eq!(written["sensors"]["pollSeconds"], MIN_POLL_SECONDS);
        assert_eq!(written["weather"]["refreshIntervalMinutes"], 60);
    }

    #[test]
    fn a_location_without_a_position_is_dropped_and_a_numeric_one_is_kept() {
        let scratch = Scratch::new("locations");
        scratch.write(
            r#"{ "weather": { "locations": [
                 { "id": "nowhere", "name": "Nowhere" },
                 { "id": "hand-edited", "name": "Austin",
                   "latitude": 30.2672, "longitude": -97.7431 }
               ] } }"#,
        );

        let locations = Store::at(scratch.settings()).load().settings.weather.locations;
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].id, "hand-edited");
        assert_eq!(locations[0].latitude, "30.2672");
    }

    #[test]
    fn the_default_strip_is_the_one_the_macos_app_ships() {
        let modules = Settings::default().modules;
        let on: Vec<ModuleId> =
            modules.iter().filter(|e| e.enabled).map(|e| e.id).collect();
        assert_eq!(on, vec![ModuleId::Cpu, ModuleId::Memory]);
        assert_eq!(
            modules.iter().map(|e| e.id).collect::<Vec<_>>(),
            ModuleId::ALL.to_vec(),
            "default order is ModuleId::ALL"
        );
    }

    #[test]
    fn the_file_lives_under_the_roaming_profile() {
        let path = Store::app_data_path().expect("APPDATA");
        assert!(path.ends_with(Path::new("Barometer").join("settings.json")));
    }
}
