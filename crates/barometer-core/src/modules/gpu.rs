// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

use std::collections::HashMap;

use windows_sys::Win32::Foundation::LUID;

use crate::format;
use crate::module::{Module, ModuleId, Readout};
use crate::pdh::Counter;
use crate::settings::GpuChoice;

/// Graphics utilization.
///
/// Windows publishes GPU work per *engine*, per process: one instance for
/// every process using every engine of every adapter. There is no counter that
/// simply says "the GPU is 40% busy".
///
/// Task Manager's headline figure is the busiest engine type, not the sum of
/// all of them, and this matches that. Summing every engine would report 200%
/// for a machine doing 3D and video decode at once, and averaging would report
/// a idle-looking number for a card pinned at 100% on compute.
///
/// Temperature, power and clock are not here. Windows does not publish them
/// and they arrive through the Sensors module instead.
pub struct GpuModule {
    engine: Option<Counter>,
    /// Dedicated adapter memory in use, per adapter; absent where the
    /// engine counters are.
    memory: Option<Counter>,
    fraction: f32,
    reading: bool,
    /// Every engine type's share at the last sample, busiest first, for
    /// the panel's utilization rows.
    engines: Vec<(String, f32)>,
    /// Dedicated memory in use on the active adapter, in bytes.
    memory_used: Option<u64>,
    /// Every adapter the kernel knows, keyed stably; refreshed whenever the
    /// settings are applied, which is when a picker might be looking.
    adapters: Vec<Adapter>,
    choice: GpuChoice,
}

/// One graphics adapter, as the kernel's display driver model lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// The registry name plus an ordinal among adapters with that name:
    /// `NVIDIA GeForce RTX 4090#0`. A LUID would be simpler, and is what the
    /// counters carry, but it is reassigned at every boot; the name is what
    /// holds still, and the ordinal separates two of the same card.
    pub key: String,
    pub name: String,
    /// The LUID as (high, low), which is how the counters spell it.
    pub luid: (i32, u32),
    /// Dedicated video memory in bytes; zero when the kernel would not say,
    /// which is what an integrated adapter with no memory of its own says.
    pub memory_total: u64,
}

impl Adapter {
    /// Whether a GPU Engine counter instance belongs to this adapter.
    fn owns(&self, instance: &str) -> bool {
        instance_luid(instance) == Some(self.luid)
    }
}

/// The LUID in a counter instance name, as (high, low).
///
/// Instances look like `pid_1234_luid_0x00000000_0x0000ABCD_phys_0_...`, the
/// two hex words being the LUID's high and low parts, which is how the
/// counters say which adapter a process's engine time was on.
fn instance_luid(instance: &str) -> Option<(i32, u32)> {
    let rest = instance.split_once("luid_0x")?.1;
    let (high, rest) = rest.split_once("_0x")?;
    let low = rest.split('_').next()?;
    let high = u32::from_str_radix(high, 16).ok()? as i32;
    let low = u32::from_str_radix(low, 16).ok()?;
    Some((high, low))
}

/// Every adapter, from the kernel, with its name from the driver's registry
/// entry - the same string Device Manager and Task Manager show.
///
/// Through D3DKMT rather than DXGI, which needs COM and a factory for what is
/// two calls here; and the kernel's list is the one the counters are named
/// against. The declarations are written out because this windows-sys build
/// carries neither.
pub fn enumerate_adapters() -> Vec<Adapter> {
    #[repr(C)]
    struct AdapterInfo {
        handle: u32,
        luid: LUID,
        sources: u32,
        precise_present_regions_preferred: i32,
    }
    #[repr(C)]
    struct EnumAdapters2 {
        count: u32,
        adapters: *mut AdapterInfo,
    }
    #[repr(C)]
    struct QueryAdapterInfo {
        handle: u32,
        kind: u32,
        data: *mut std::ffi::c_void,
        size: u32,
    }
    #[repr(C)]
    struct RegistryInfo {
        adapter: [u16; 260],
        bios: [u16; 260],
        dac: [u16; 260],
        chip: [u16; 260],
    }
    #[repr(C)]
    struct SegmentSizeInfo {
        dedicated_video: u64,
        dedicated_system: u64,
        shared_system: u64,
    }
    #[repr(C)]
    struct CloseAdapter {
        handle: u32,
    }
    const KMTQAITYPE_GETSEGMENTSIZE: u32 = 3;
    const KMTQAITYPE_ADAPTERREGISTRYINFO: u32 = 8;
    #[link(name = "gdi32")]
    extern "system" {
        fn D3DKMTEnumAdapters2(request: *mut EnumAdapters2) -> i32;
        fn D3DKMTQueryAdapterInfo(request: *mut QueryAdapterInfo) -> i32;
        fn D3DKMTCloseAdapter(request: *mut CloseAdapter) -> i32;
    }

    // SAFETY: documented kernel-mode thunks taking structures laid out as
    // d3dkmthk.h declares them. The first call with a null buffer asks how
    // many adapters there are; the second fills a buffer of that size, and
    // every handle it hands out is closed before returning.
    unsafe {
        let mut request = EnumAdapters2 { count: 0, adapters: std::ptr::null_mut() };
        if D3DKMTEnumAdapters2(&mut request) < 0 || request.count == 0 {
            return Vec::new();
        }
        let mut infos: Vec<AdapterInfo> = (0..request.count)
            .map(|_| AdapterInfo {
                handle: 0,
                luid: LUID { LowPart: 0, HighPart: 0 },
                sources: 0,
                precise_present_regions_preferred: 0,
            })
            .collect();
        request.adapters = infos.as_mut_ptr();
        if D3DKMTEnumAdapters2(&mut request) < 0 {
            return Vec::new();
        }
        infos.truncate(request.count as usize);

        let mut found: Vec<Adapter> = Vec::new();
        for info in &infos {
            let mut registry = RegistryInfo { adapter: [0; 260], bios: [0; 260], dac: [0; 260], chip: [0; 260] };
            let mut query = QueryAdapterInfo {
                handle: info.handle,
                kind: KMTQAITYPE_ADAPTERREGISTRYINFO,
                data: (&mut registry as *mut RegistryInfo).cast(),
                size: std::mem::size_of::<RegistryInfo>() as u32,
            };
            let name = if D3DKMTQueryAdapterInfo(&mut query) >= 0 {
                let end = registry.adapter.iter().position(|c| *c == 0).unwrap_or(260);
                String::from_utf16_lossy(&registry.adapter[..end]).trim().to_string()
            } else {
                String::new()
            };
            // How much memory is the card's own. Task Manager's "Dedicated
            // GPU memory" is this figure.
            let mut segments = SegmentSizeInfo { dedicated_video: 0, dedicated_system: 0, shared_system: 0 };
            let mut query = QueryAdapterInfo {
                handle: info.handle,
                kind: KMTQAITYPE_GETSEGMENTSIZE,
                data: (&mut segments as *mut SegmentSizeInfo).cast(),
                size: std::mem::size_of::<SegmentSizeInfo>() as u32,
            };
            let memory_total = if D3DKMTQueryAdapterInfo(&mut query) >= 0 { segments.dedicated_video } else { 0 };
            let mut close = CloseAdapter { handle: info.handle };
            D3DKMTCloseAdapter(&mut close);
            // The software adapter is not a choice that reads anything. It is
            // recognized by name and not by having no display outputs: on a
            // laptop with hybrid graphics the discrete GPU has no outputs
            // either - every display hangs off the integrated one - and it
            // is the adapter the user most wants to pick.
            if name.is_empty() || name == "Microsoft Basic Render Driver" {
                continue;
            }
            let ordinal = found.iter().filter(|a| a.name == name).count();
            let luid = (info.luid.HighPart, info.luid.LowPart);
            found.push(Adapter { key: format!("{name}#{ordinal}"), name, luid, memory_total });
        }
        found
    }
}

impl Default for GpuModule {
    fn default() -> Self {
        GpuModule::new()
    }
}

impl GpuModule {
    pub fn new() -> Self {
        // Absent on a machine with no GPU counters, which is old Windows or a
        // stripped install. The module then reports unavailable rather than
        // pretending to a zero.
        GpuModule {
            engine: Counter::open(r"\GPU Engine(*)\Utilization Percentage"),
            memory: Counter::open(r"\GPU Adapter Memory(*)\Dedicated Usage"),
            fraction: 0.0,
            reading: false,
            engines: Vec::new(),
            memory_used: None,
            adapters: enumerate_adapters(),
            choice: GpuChoice::Automatic,
        }
    }

    /// The adapter the settings chose, if it is present on this machine.
    fn chosen(&self) -> Option<&Adapter> {
        match &self.choice {
            GpuChoice::Automatic => None,
            GpuChoice::Adapter { key, .. } => self.adapters.iter().find(|a| &a.key == key),
        }
    }

    /// The adapter the readout and the panel are about: the chosen one,
    /// or, when the choice is automatic, the one with the most memory of
    /// its own - the discrete card on a laptop that also has integrated
    /// graphics, which is what "the GPU" means to the person who bought it.
    /// Ties go to the first the kernel lists.
    pub fn active(&self) -> Option<&Adapter> {
        self.chosen().or_else(|| self.adapters.iter().max_by_key(|a| (a.memory_total, std::cmp::Reverse(a.luid))))
    }

}

/// Pulls the engine type out of an instance name.
///
/// They look like
/// `pid_1234_luid_0x00000000_0x0000ABCD_phys_0_eng_0_engtype_3D`,
/// and the part after the last `engtype_` is the classification we group on.
fn engine_type(instance: &str) -> Option<&str> {
    // An engine the counters give no name - some drivers publish one whose
    // instance ends at "engtype_" - is not a row anyone can read.
    instance.rsplit_once("engtype_").map(|(_, kind)| kind).filter(|kind| !kind.is_empty())
}

impl Module for GpuModule {
    fn id(&self) -> ModuleId {
        ModuleId::Gpu
    }

    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        _temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        match metric {
            crate::stack::StackMetric::GpuUtilization if self.reading => {
                Some(format::percent(self.fraction))
            }
            // Power and temperature come from the sensor source; a sensor
            // reading in the stack is the way to show them.
            _ => None,
        }
    }

    fn fraction(&self) -> Option<f32> {
        self.reading.then_some(self.fraction)
    }

    fn configure(&mut self, settings: &crate::store::Settings) {
        self.choice = settings.gpu.clone();
        // Re-read the list here rather than only at start: a monitor docked
        // or an eGPU attached since is an adapter the picker should offer.
        self.adapters = enumerate_adapters();
    }

    fn gpu_adapters(&self) -> Vec<(String, String)> {
        // The active adapter first: the app's "automatic" is the first of
        // this list, so the list says which one that is.
        let mut adapters: Vec<&Adapter> = self.adapters.iter().collect();
        if let Some(active) = self.active() {
            adapters.sort_by_key(|a| a.key != active.key);
        }
        adapters.iter().map(|a| (a.key.clone(), a.name.clone())).collect()
    }

    fn gpu_engines(&self) -> Vec<(String, f32)> {
        self.engines.clone()
    }

    fn gpu_memory(&self) -> Option<(u64, u64)> {
        let used = self.memory_used?;
        Some((used, self.active().map(|a| a.memory_total).unwrap_or(0)))
    }

    fn sample(&mut self) {
        let Some(counter) = self.engine.as_mut() else {
            self.reading = false;
            return;
        };
        let Some(instances) = counter.read() else {
            // The first collection only establishes a baseline, and a
            // collection that fails afterwards means the counter has stopped
            // answering. Either way there is no reading now: leaving the flag
            // alone held the last percentage on the taskbar forever, shown as
            // though it were live.
            self.reading = false;
            return;
        };

        // Only the active adapter's engines. A machine with an integrated
        // and a discrete GPU otherwise reports whichever happens to be
        // busier, which is rarely the one the user means - and the panel
        // names one adapter, so the figure had better be that adapter's.
        let chosen = self.active().cloned();

        // Sum within an engine type across every process using it, then take
        // the busiest type.
        let mut by_type: HashMap<&str, f64> = HashMap::new();
        for instance in &instances {
            if let Some(adapter) = &chosen {
                if !adapter.owns(&instance.name) {
                    continue;
                }
            }
            if let Some(kind) = engine_type(&instance.name) {
                *by_type.entry(kind).or_insert(0.0) += instance.value;
            }
        }

        let busiest = by_type.values().copied().fold(0.0_f64, f64::max);
        self.fraction = (busiest / 100.0).clamp(0.0, 1.0) as f32;
        self.reading = true;

        let mut engines: Vec<(String, f32)> = by_type
            .into_iter()
            .map(|(kind, share)| (kind.to_string(), (share / 100.0).clamp(0.0, 1.0) as f32))
            .collect();
        engines.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        self.engines = engines;

        // Memory is per adapter, not per process: one instance for each,
        // named by LUID. The active adapter's, or everything when no
        // adapter matches, which is a machine the kernel and the counters
        // disagree about.
        self.memory_used = self.memory.as_mut().and_then(|counter| counter.read()).map(|instances| {
            let active = self.active().map(|a| a.luid);
            let mine: Vec<&crate::pdh::Instance> =
                instances.iter().filter(|i| active.is_some() && instance_luid(&i.name) == active).collect();
            let sum = |items: &[&crate::pdh::Instance]| items.iter().map(|i| i.value.max(0.0)).sum::<f64>() as u64;
            if mine.is_empty() {
                sum(&instances.iter().collect::<Vec<_>>())
            } else {
                sum(&mine)
            }
        });
    }

    fn readout(&self) -> Readout {
        if !self.reading {
            return Readout::unavailable();
        }
        Readout::one(format::percent(self.fraction)).reserving("100%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapters_enumerate_with_unique_keys_and_names() {
        // Zero adapters is a machine with no display driver, which a build
        // agent can be; what must hold is that whatever is found is usable.
        let adapters = enumerate_adapters();
        for adapter in &adapters {
            eprintln!("adapter {} = {:?} luid {:?}", adapter.key, adapter.name, adapter.luid);
            assert!(!adapter.name.is_empty());
            assert!(adapter.key.starts_with(&adapter.name));
        }
        let mut keys: Vec<&str> = adapters.iter().map(|a| a.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), adapters.len());
    }

    #[test]
    fn automatic_means_the_adapter_with_the_most_memory_of_its_own() {
        let mut module = GpuModule::new();
        module.adapters = vec![
            Adapter { key: "Intel#0".into(), name: "Intel".into(), luid: (0, 1), memory_total: 0 },
            Adapter { key: "NVIDIA#0".into(), name: "NVIDIA".into(), luid: (0, 2), memory_total: 8 << 30 },
        ];
        module.choice = GpuChoice::Automatic;
        assert_eq!(module.active().map(|a| a.key.as_str()), Some("NVIDIA#0"));
        assert_eq!(module.gpu_adapters()[0].0, "NVIDIA#0", "the list leads with it");
        // Two with nothing of their own: the first listed.
        module.adapters[1].memory_total = 0;
        assert_eq!(module.active().map(|a| a.key.as_str()), Some("Intel#0"));
        // A choice is a choice.
        module.choice = GpuChoice::Adapter { key: "NVIDIA#0".into(), name: "NVIDIA".into() };
        assert_eq!(module.active().map(|a| a.key.as_str()), Some("NVIDIA#0"));
    }

    #[test]
    fn the_luid_in_an_instance_name_is_read_as_high_and_low_words() {
        assert_eq!(
            instance_luid("pid_1234_luid_0x00000000_0x0000ABCD_phys_0_eng_0_engtype_3D"),
            Some((0, 0xABCD))
        );
        assert_eq!(instance_luid("pid_9_luid_0xFFFFFFFF_0x1_phys_0"), Some((-1, 1)));
        assert_eq!(instance_luid("pid_9_phys_0"), None);
    }

    #[test]
    fn an_adapter_owns_only_the_instances_carrying_its_luid() {
        let adapter = Adapter {
            key: "Card#0".into(),
            name: "Card".into(),
            luid: (0, 0xABCD),
            memory_total: 0,
        };
        assert!(adapter.owns("pid_1_luid_0x00000000_0x0000ABCD_phys_0_eng_0_engtype_3D"));
        assert!(!adapter.owns("pid_1_luid_0x00000000_0x0000ABCE_phys_0_eng_0_engtype_3D"));
    }

    #[test]
    fn the_engine_type_is_the_tail_of_the_instance_name() {
        assert_eq!(
            engine_type("pid_1234_luid_0x00000000_0x0000ABCD_phys_0_eng_0_engtype_3D"),
            Some("3D")
        );
        assert_eq!(
            engine_type("pid_9_luid_0x0_0x1_phys_0_eng_2_engtype_VideoDecode"),
            Some("VideoDecode")
        );
    }

    #[test]
    fn an_instance_without_an_engine_type_is_ignored_rather_than_guessed() {
        assert_eq!(engine_type("something_unexpected"), None);
        // A driver that names an engine nothing at all.
        assert_eq!(engine_type("pid_9_luid_0x0_0x1_phys_0_eng_9_engtype_"), None);
    }
}
