// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::format;
use crate::module::{Module, ModuleId, Readout};
use crate::pdh::Counter;

/// The volume Windows booted from, for when the user has not chosen one.
///
/// Asked of the environment rather than written down as "C:": that is the
/// usual answer and not the only one, and a machine that boots from another
/// letter had its space readings quietly describing a drive it does not have.
fn boot_volume() -> String {
    std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string())
}

/// The instance PDH uses for the sum across every physical disk.
const TOTAL_INSTANCE: &str = "_Total";

/// Disk throughput, read over write.
///
/// The macOS module reports rates and operations per second per physical
/// device, and mounted-volume capacity alongside. This is the strip's half of
/// that: the machine's total read and write rate. Per-device rows and volume
/// capacity belong in the detail panel and are read separately, because
/// enumerating volumes touches every mount and there is no reason to do that
/// once a second for a readout that shows two numbers.
///
/// Rates come from performance counters rather than from a cumulative counter
/// we difference ourselves: PDH already computes the rate over its own
/// interval, and its interval is the honest one.
pub struct DiskModule {
    read_bytes: Option<Counter>,
    write_bytes: Option<Counter>,
    read_ops: Option<Counter>,
    write_ops: Option<Counter>,
    read: f64,
    write: f64,
    reading: bool,
    /// Every physical disk as the counters last saw it, for the panel.
    devices: Vec<DiskDevice>,
    /// The system volume's used and total bytes, for the stack readings
    /// that are about space rather than throughput. Re-read every so
    /// often: free space moves slowly and the call is not free.
    system_volume: Option<(u64, u64)>,
    /// Where the scan leaves its answer, and whether one is already out.
    ///
    /// On a thread, because `volumes()` asks every drive letter for its size
    /// and deliberately keeps network drives - and `sample()` runs on the
    /// window's message thread. A sleeping NAS or a dropped VPN made
    /// `GetDiskFreeSpaceExW` sit for the SMB timeout, and for that whole time
    /// the readout did not paint, did not follow the tray and did not answer
    /// a click. The flag keeps one slow scan from starting another behind it.
    scanned: Arc<Mutex<Option<Vec<crate::volumes::Volume>>>>,
    /// Every volume the last scan saw, for the settings pane's picker.
    known_volumes: Vec<crate::volumes::Volume>,
    /// Which volume the space readings are about; None is the boot volume.
    volume: Option<String>,
    /// Which disk the throughput readings are about, by counter instance;
    /// None is every disk together.
    device: Option<String>,
    scanning: Arc<AtomicBool>,
    samples: u32,
    /// What each disk is called, by its counter instance, asked once when a
    /// disk is first seen: the answer does not change and the question
    /// opens a device.
    ///
    /// Shared with a thread because opening `PhysicalDriveN` and sending it
    /// an ioctl is not instant - a spun-down USB disk takes seconds to answer
    /// - and asking on the message thread stopped the readout for exactly
    /// that long the first time such a disk appeared. A name nobody has yet
    /// is simply absent for a tick or two.
    models: Arc<Mutex<HashMap<String, Option<String>>>>,
}

/// One physical disk, as the performance counters name it.
///
/// The counters call a disk "0 C:" or "1 D: E:" - its number and the
/// letters mounted on it - which is a name a person can place. The model
/// string needs a device ioctl and is not read yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskDevice {
    /// The counter instance, "0 C:".
    pub id: String,
    /// "Disk 0 (C:)".
    pub name: String,
    /// The drive's own name for itself, "Samsung SSD 990 PRO 2TB", where
    /// it answers.
    pub model: Option<String>,
    /// Bytes per second.
    pub read: f64,
    pub write: f64,
    /// Operations per second.
    pub read_ops: f64,
    pub write_ops: f64,
}

/// The counter's number for a disk, "0 C:" being disk 0, which is also the
/// number in its device path.
fn device_number(instance: &str) -> Option<u32> {
    instance.split(' ').next()?.parse().ok()
}

/// "0 C:" as "Disk 0 (C:)"; an instance with no letters is "Disk 0".
fn device_name(instance: &str) -> String {
    let mut parts = instance.splitn(2, ' ');
    let number = parts.next().unwrap_or(instance);
    match parts.next().map(str::trim).filter(|m| !m.is_empty()) {
        Some(mounts) => format!("Disk {number} ({mounts})"),
        None => format!("Disk {number}"),
    }
}

impl Default for DiskModule {
    fn default() -> Self {
        DiskModule::new()
    }
}

impl DiskModule {
    pub fn new() -> Self {
        DiskModule {
            read_bytes: Counter::open(r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
            write_bytes: Counter::open(r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
            read_ops: Counter::open(r"\PhysicalDisk(*)\Disk Reads/sec"),
            write_ops: Counter::open(r"\PhysicalDisk(*)\Disk Writes/sec"),
            read: 0.0,
            write: 0.0,
            reading: false,
            devices: Vec::new(),
            system_volume: None,
            scanned: Arc::new(Mutex::new(None)),
            known_volumes: Vec::new(),
            volume: None,
            device: None,
            scanning: Arc::new(AtomicBool::new(false)),
            samples: 0,
            models: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Current rates in bytes per second, read and write.
    pub fn rates(&self) -> Option<(f64, f64)> {
        self.reading.then_some((self.read, self.write))
    }
}

/// One named instance's value, or None when the counters no longer list it.
fn of_instance(instances: &[crate::pdh::Instance], name: &str) -> Option<f64> {
    instances.iter().find(|i| i.name == name).map(|i| i.value)
}

/// Takes the `_Total` instance, falling back to summing the rest.
///
/// `_Total` is what PDH publishes for the whole machine and is the cheap
/// answer. The fallback exists because the instance is absent on a machine
/// with no physical disks reported, and summing then gives the same figure
/// rather than nothing.
fn total_of(instances: &[crate::pdh::Instance]) -> f64 {
    if let Some(total) = instances.iter().find(|i| i.name == TOTAL_INSTANCE) {
        return total.value;
    }
    instances.iter().filter(|i| i.name != TOTAL_INSTANCE).map(|i| i.value).sum()
}

impl Module for DiskModule {
    fn id(&self) -> ModuleId {
        ModuleId::Disks
    }

    fn rates(&self) -> Option<(f64, f64)> {
        DiskModule::rates(self)
    }

    fn sample(&mut self) {
        let (Some(read), Some(write)) = (self.read_bytes.as_mut(), self.write_bytes.as_mut())
        else {
            self.reading = false;
            return;
        };
        // Both or neither. A tick that updated one rate and not the other
        // would show a read spike against a stale write, which reads as a
        // machine doing something it is not.
        let (Some(read_values), Some(write_values)) = (read.read(), write.read()) else {
            // Priming, or a counter that has stopped answering. A rate that
            // is not being read is not a rate of zero and is not the last one
            // either, and holding the last one on the taskbar showed a
            // stopped counter as a busy disk that never changed.
            self.reading = false;
            return;
        };
        // One disk, or all of them added up. A disk the user chose and that
        // is no longer there - unplugged, or a letter that moved - reads as
        // unavailable rather than silently becoming the whole machine again,
        // which would be the same number under a name that means something
        // else.
        match self.device.as_deref() {
            Some(wanted) => match (of_instance(&read_values, wanted), of_instance(&write_values, wanted)) {
                (Some(read), Some(write)) => {
                    self.read = read.max(0.0);
                    self.write = write.max(0.0);
                    self.reading = true;
                }
                _ => self.reading = false,
            },
            None => {
                self.read = total_of(&read_values).max(0.0);
                self.write = total_of(&write_values).max(0.0);
                self.reading = true;
            }
        }

        // Per disk, for the panel. The operation counters are optional:
        // a machine without them still has its byte rates.
        let read_ops = self.read_ops.as_mut().and_then(|c| c.read()).unwrap_or_default();
        let write_ops = self.write_ops.as_mut().and_then(|c| c.read()).unwrap_or_default();
        let of = |values: &[crate::pdh::Instance], name: &str| -> f64 {
            values.iter().find(|i| i.name == name).map(|i| i.value.max(0.0)).unwrap_or(0.0)
        };
        let known = self.models.lock().ok().map(|m| m.clone()).unwrap_or_default();
        // Whichever disks nobody has asked about yet, asked once, together.
        let unasked: Vec<String> = read_values
            .iter()
            .map(|i| i.name.clone())
            .filter(|name| name != TOTAL_INSTANCE && !known.contains_key(name))
            .collect();
        if !unasked.is_empty() {
            // Claimed before the thread starts, so a disk that takes seconds
            // to answer is not asked again on every tick in between. The
            // placeholder reads as "no model", which is also the answer for
            // a disk that never gives one.
            if let Ok(mut slot) = self.models.lock() {
                for name in &unasked {
                    slot.insert(name.clone(), None);
                }
            }
            let models = Arc::clone(&self.models);
            std::thread::spawn(move || {
                for name in unasked {
                    let model = device_number(&name).and_then(crate::sys::storage::disk_model);
                    if let Ok(mut slot) = models.lock() {
                        slot.insert(name, model);
                    }
                }
            });
        }
        let mut devices: Vec<DiskDevice> = read_values
            .iter()
            .filter(|i| i.name != TOTAL_INSTANCE)
            .map(|i| DiskDevice {
                id: i.name.clone(),
                name: device_name(&i.name),
                model: known.get(&i.name).cloned().flatten(),
                read: i.value.max(0.0),
                write: of(&write_values, &i.name),
                read_ops: of(&read_ops, &i.name),
                write_ops: of(&write_ops, &i.name),
            })
            .collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        self.devices = devices;

        // Whatever the last scan found, which is at worst half a minute old
        // and describes something that moves in megabytes an hour.
        if let Ok(found) = self.scanned.lock() {
            if let Some(volumes) = found.as_ref() {
                self.known_volumes = volumes.clone();
            }
        }
        let wanted = self.volume.clone().unwrap_or_else(boot_volume);
        self.system_volume = self
            .known_volumes
            .iter()
            .find(|v| v.mount.eq_ignore_ascii_case(&wanted))
            .map(|v| (v.used(), v.total));
        if self.samples % 30 == 0 && !self.scanning.swap(true, Ordering::AcqRel) {
            let scanned = Arc::clone(&self.scanned);
            let scanning = Arc::clone(&self.scanning);
            std::thread::spawn(move || {
                let found = crate::volumes::volumes();
                if let Ok(mut slot) = scanned.lock() {
                    *slot = Some(found);
                }
                scanning.store(false, Ordering::Release);
            });
        }
        self.samples = self.samples.wrapping_add(1);
    }

    fn volumes(&self) -> Vec<crate::volumes::Volume> {
        self.known_volumes.clone()
    }

    fn configure(&mut self, settings: &crate::store::Settings) {
        self.volume = settings.disk_volume.clone();
        if self.device != settings.disk_device {
            // The rate belongs to whichever disk it was read from, so the
            // old one is not a reading of the new one.
            self.device = settings.disk_device.clone();
            self.reading = false;
        }
    }

    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        _temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        use crate::stack::StackMetric::*;
        match metric {
            DiskRead => self.reading.then(|| format::rate(self.read)),
            DiskWrite => self.reading.then(|| format::rate(self.write)),
            DiskUsedPercent => self
                .system_volume
                .filter(|(_, total)| *total > 0)
                .map(|(used, total)| format::percent(used as f32 / total as f32)),
            DiskFreeBytes => self.system_volume.map(|(used, total)| format::bytes(total.saturating_sub(used))),
            _ => None,
        }
    }

    fn disk_devices(&self) -> Vec<DiskDevice> {
        self.devices.clone()
    }

    fn readout(&self) -> Readout {
        if !self.reading {
            return Readout::unavailable();
        }
        Readout::two(format::rate(self.read), format::rate(self.write)).reserving("000 MB/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdh::Instance;

    fn instance(name: &str, value: f64) -> Instance {
        Instance { name: name.to_string(), value }
    }

    #[test]
    fn a_counter_instance_carries_the_disks_number() {
        assert_eq!(device_number("0 C:"), Some(0));
        assert_eq!(device_number("12"), Some(12));
        assert_eq!(device_number("_Total"), None);
    }

    #[test]
    fn a_counter_instance_names_its_disk_and_its_letters() {
        assert_eq!(device_name("0 C:"), "Disk 0 (C:)");
        assert_eq!(device_name("1 D: E:"), "Disk 1 (D: E:)");
        assert_eq!(device_name("2"), "Disk 2");
    }

    #[test]
    fn the_total_instance_is_preferred() {
        let values = vec![
            instance("0 C:", 100.0),
            instance("1 D:", 50.0),
            instance(TOTAL_INSTANCE, 150.0),
        ];
        assert_eq!(total_of(&values), 150.0);
    }

    #[test]
    fn without_a_total_the_disks_are_summed() {
        let values = vec![instance("0 C:", 100.0), instance("1 D:", 50.0)];
        assert_eq!(total_of(&values), 150.0);
    }

    #[test]
    fn the_total_is_never_double_counted_in_the_fallback() {
        // The fallback must exclude _Total, or a machine that reports it plus
        // its disks would read twice the real rate.
        let values = vec![instance("0 C:", 100.0)];
        assert_eq!(total_of(&values), 100.0);
    }

    #[test]
    fn no_instances_is_zero_rather_than_a_panic() {
        assert_eq!(total_of(&[]), 0.0);
    }
}
