// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

use crate::format;
use crate::module::{Module, ModuleId, Readout};
use crate::sys::{self, Memory};

/// Physical memory in use.
///
/// "Used" here is total minus available, which is what Task Manager's
/// percentage reports. It is not the same as the Mac's memory pressure, and
/// the detail panel will want to say so once it exists.
#[derive(Default)]
pub struct MemoryModule {
    memory: Option<Memory>,
    /// Commit charge and its limit, in bytes, for the pressure reading.
    commit: Option<(u64, u64)>,
    /// Page file in use and its size, in bytes, for the swap reading.
    page_file: Option<(u64, u64)>,
    samples: u32,
}

/// How many ticks apart the page file is asked for - see `sample`.
const PAGE_FILE_EVERY: u32 = 15;

impl MemoryModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes used and bytes installed, for the detail panel.
    pub fn used_and_total(&self) -> Option<(u64, u64)> {
        self.memory.map(|m| (m.used(), m.total))
    }
}

impl Module for MemoryModule {
    fn id(&self) -> ModuleId {
        ModuleId::Memory
    }

    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        _temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        use crate::stack::StackMetric::*;
        let (used, total) = self.used_and_total()?;
        match metric {
            MemoryUsedPercent if total > 0 => Some(format::percent(used as f32 / total as f32)),
            MemoryUsedBytes => Some(format::bytes(used)),
            MemoryFreeBytes => Some(format::bytes(total.saturating_sub(used))),
            // The Mac's pressure is the kernel's own level. Windows has no
            // such figure; commit charge against its limit is what the
            // memory panel grades Normal, High and Critical by, and the
            // same number is what the stack shows.
            MemoryPressure => self
                .commit
                .filter(|(_, limit)| *limit > 0)
                .map(|(committed, limit)| format::percent(committed as f32 / limit as f32)),
            MemorySwap => self.page_file.map(|(used, _)| format::bytes(used)),
            _ => None,
        }
    }

    fn memory(&self) -> Option<(u64, u64)> {
        self.used_and_total()
    }

    fn sample(&mut self) {
        self.memory = sys::memory();
        self.commit = sys::processes::summary().map(|s| (s.committed, s.commit_limit));
        // Not every tick. The page file is a single figure that moves in
        // megabytes a minute, and asking for it means an
        // NtQuerySystemInformation whose buffer starts at a quarter of a
        // megabyte - a 256 KB allocate and free every second, for a number
        // only an optional stack reading and two rows of a panel ever show.
        if self.samples % PAGE_FILE_EVERY == 0 {
            self.page_file = sys::processes::page_file();
        }
        self.samples = self.samples.wrapping_add(1);
    }

    fn page_file(&self) -> Option<(u64, u64)> {
        self.page_file
    }

    fn detail(&self) -> Vec<String> {
        // The percentage is what fits on the strip; the number of gigabytes is
        // what somebody actually wants when they stop to look.
        let Some((used, total)) = self.used_and_total() else {
            return vec![self.id().title().to_string(), "Not available".to_string()];
        };
        vec![
            self.id().title().to_string(),
            format!(
                "{} of {} used",
                format::bytes(used),
                format::bytes(total)
            ),
            format!("{} free", format::bytes(total.saturating_sub(used))),
        ]
    }

    fn readout(&self) -> Readout {
        match self.memory {
            Some(memory) => Readout::one(format::percent(memory.used_fraction())).reserving("100%"),
            None => Readout::unavailable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::StackMetric;
    use crate::weather::models::TemperatureUnit;

    #[test]
    fn a_stack_reads_the_used_share_the_used_bytes_and_the_free_bytes() {
        let module = MemoryModule {
            samples: 0,
            memory: Some(Memory { total: 32 << 30, available: 8 << 30 }),
            commit: Some((30 << 30, 40 << 30)),
            page_file: Some((3 << 30, 8 << 30)),
        };
        let read = |m| module.stack_value(&m, TemperatureUnit::Celsius);
        assert_eq!(read(StackMetric::MemoryUsedPercent).as_deref(), Some("75%"));
        assert_eq!(read(StackMetric::MemoryUsedBytes).as_deref(), Some(format::bytes(24 << 30).as_str()));
        assert_eq!(read(StackMetric::MemoryFreeBytes).as_deref(), Some(format::bytes(8 << 30).as_str()));
        // Pressure is the commit charge against its limit; swap is the page
        // file in use.
        assert_eq!(read(StackMetric::MemoryPressure).as_deref(), Some("75%"));
        assert_eq!(read(StackMetric::MemorySwap).as_deref(), Some(format::bytes(3 << 30).as_str()));
        // Before the first sample there is nothing to say for either.
        let fresh = MemoryModule::default();
        assert_eq!(fresh.stack_value(&StackMetric::MemorySwap, TemperatureUnit::Celsius), None);
    }

    #[test]
    fn a_stack_gets_nothing_before_the_first_sample() {
        let module = MemoryModule::default();
        assert_eq!(module.stack_value(&StackMetric::MemoryUsedPercent, TemperatureUnit::Celsius), None);
    }
}
