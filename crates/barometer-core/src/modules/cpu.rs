// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

use std::time::Instant;

use crate::format;
use crate::module::{Module, ModuleId, Readout};
use crate::pdh::Counter;
use crate::sys::{self, CpuTimes};

/// Total processor load, as a percentage.
///
/// Derived from tick deltas rather than from a rate, so it needs no clock of
/// its own: the counters advance by wall time times the core count, and the
/// ratio of busy to total is the load over exactly the interval between two
/// samples, however irregular that interval turned out to be.
#[derive(Default)]
pub struct CpuModule {
    previous: Option<CpuTimes>,
    fraction: f32,
    /// The busy fraction split the way the Mac's readings split it: user
    /// time and kernel time, each of the whole.
    user: f32,
    system: f32,
    reading: bool,
    /// Threads ready to run and waiting for a processor, which is the same
    /// quantity Unix averages into a load average.
    queue: Option<Counter>,
    /// When the queue was last read, so the decay is over the interval that
    /// actually elapsed rather than the one the loop intends.
    queue_at: Option<Instant>,
    /// The three averages of it, one, five and fifteen minutes.
    load: Option<LoadAverage>,
}

/// The Mac's `getloadavg`, which Windows does not have, computed the way
/// the Unix kernel computes it.
///
/// A load average is not a utilization figure: it is how many threads are
/// runnable, averaged with an exponential decay, so a machine with four
/// busy threads reads 4 whatever percentage that is of its cores.
/// `\System\Processor Queue Length` is that same count on Windows - threads
/// in the ready queue - so the port is the same decay over the same
/// quantity, and it is a real figure rather than the busy fraction wearing
/// a load average's name.
///
/// Windows samples the queue instantaneously where Unix accumulates it, so
/// a burst between two samples is missed. The average is still the right
/// shape and the right order of magnitude, which is what the number is for.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

impl LoadAverage {
    /// The three, as the Mac's row writes them: `0.42  \u{00B7}  0.31  \u{00B7}  0.28`.
    pub fn describe(&self) -> String {
        format!("{:.2}  \u{00B7}  {:.2}  \u{00B7}  {:.2}", self.one, self.five, self.fifteen)
    }

    /// Folds one sample of the ready queue in, `elapsed` seconds after the
    /// last. Each average decays by `exp(-elapsed / window)`, which is the
    /// kernel's CALC_LOAD with its fixed five-second tick generalized to
    /// however long this interval actually was.
    fn observe(&mut self, queue: f64, elapsed: f64) {
        let fold = |average: f64, window: f64| {
            let decay = (-elapsed / window).exp();
            average * decay + queue * (1.0 - decay)
        };
        self.one = fold(self.one, 60.0);
        self.five = fold(self.five, 300.0);
        self.fifteen = fold(self.fifteen, 900.0);
    }
}

impl CpuModule {
    pub fn new() -> Self {
        CpuModule {
            // Absent where the performance counters are not there, which is
            // the same machine the GPU counters are missing from. The
            // reading is then unavailable rather than zero.
            queue: Counter::open(r"\System\Processor Queue Length"),
            ..Default::default()
        }
    }

    /// The load averages, once there has been more than one sample.
    pub fn load_average(&self) -> Option<LoadAverage> {
        self.load
    }

    /// Folds one reading of the ready queue into the three averages.
    ///
    /// The first reading seeds them rather than decaying from zero: a
    /// machine that is busy when the app starts should not spend a minute
    /// climbing from nothing to the truth.
    fn sample_load(&mut self) {
        let Some(queue) = self.queue.as_mut().and_then(|counter| counter.read()) else { return };
        let Some(queue) = queue.first().map(|instance| instance.value.max(0.0)) else { return };
        let now = Instant::now();
        match (&mut self.load, self.queue_at) {
            (Some(load), Some(at)) => load.observe(queue, now.duration_since(at).as_secs_f64()),
            _ => self.load = Some(LoadAverage { one: queue, five: queue, fifteen: queue }),
        }
        self.queue_at = Some(now);
    }
}

impl Module for CpuModule {
    fn id(&self) -> ModuleId {
        ModuleId::Cpu
    }

    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        _temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        use crate::stack::StackMetric::*;
        if !self.reading {
            return None;
        }
        match metric {
            CpuTotal => Some(format::percent(self.fraction)),
            CpuUser => Some(format::percent(self.user)),
            CpuSystem => Some(format::percent(self.system)),
            CpuIdle => Some(format::percent((1.0 - self.fraction).clamp(0.0, 1.0))),
            // One minute, as the Mac's strip reading is the first of the
            // three.
            CpuLoad => self.load.map(|load| format!("{:.2}", load.one)),
            _ => None,
        }
    }

    fn load_average(&self) -> Option<LoadAverage> {
        self.load
    }

    fn cpu_split(&self) -> Option<(f32, f32)> {
        self.reading.then_some((self.user, self.system))
    }

    fn fraction(&self) -> Option<f32> {
        self.reading.then_some(self.fraction)
    }

    fn sample(&mut self) {
        let Some(now) = sys::cpu_times() else {
            self.reading = false;
            return;
        };
        if let Some(previous) = self.previous {
            let elapsed = now.total().saturating_sub(previous.total());
            let busy = now.busy().saturating_sub(previous.busy());
            // A zero interval happens when two samples land inside the same
            // tick. Keeping the previous figure is better than showing 0%.
            if elapsed > 0 {
                self.fraction = (busy as f64 / elapsed as f64) as f32;
                let user = now.user.saturating_sub(previous.user);
                // Kernel time counts idle time; the system's share is what
                // is left of it once idle is taken out.
                let idle = now.idle.saturating_sub(previous.idle);
                let system = now.kernel.saturating_sub(previous.kernel).saturating_sub(idle);
                self.user = (user as f64 / elapsed as f64) as f32;
                self.system = (system as f64 / elapsed as f64) as f32;
                self.reading = true;
            }
        }
        self.previous = Some(now);
        self.sample_load();
    }

    fn readout(&self) -> Readout {
        if !self.reading {
            // The first sample establishes a baseline and produces no load
            // figure. Showing nothing for one interval beats showing a zero
            // that looks like an idle machine.
            return Readout::unavailable();
        }
        Readout::one(format::percent(self.fraction)).reserving("100%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_average_decays_toward_what_the_queue_is_doing() {
        let mut load = LoadAverage { one: 0.0, five: 0.0, fifteen: 0.0 };
        // A minute of four runnable threads: the one-minute average is
        // 1 - 1/e of the way there, as the Unix kernel's is.
        for _ in 0..60 {
            load.observe(4.0, 1.0);
        }
        assert!((load.one - 4.0 * (1.0 - (-1.0f64).exp())).abs() < 0.01, "{load:?}");
        // The longer windows lag behind it, which is the whole point of
        // having three.
        assert!(load.five < load.one && load.fifteen < load.five);
        // And an idle machine falls back toward nothing.
        for _ in 0..600 {
            load.observe(0.0, 1.0);
        }
        assert!(load.one < 0.01, "{load:?}");
    }

    #[test]
    fn an_irregular_interval_decays_by_as_much_as_it_was_long() {
        let mut once = LoadAverage { one: 0.0, five: 0.0, fifteen: 0.0 };
        once.observe(2.0, 10.0);
        let mut ten = LoadAverage { one: 0.0, five: 0.0, fifteen: 0.0 };
        for _ in 0..10 {
            ten.observe(2.0, 1.0);
        }
        assert!((once.one - ten.one).abs() < 1e-9, "{once:?} {ten:?}");
    }

    #[test]
    fn the_three_are_written_the_way_the_mac_writes_them() {
        let load = LoadAverage { one: 0.4249, five: 0.31, fifteen: 0.2 };
        assert_eq!(load.describe(), "0.42  \u{00B7}  0.31  \u{00B7}  0.20");
    }

    #[test]
    fn the_machine_answers_with_a_load_average_after_two_samples() {
        let mut module = CpuModule::new();
        module.sample();
        let first = module.load_average().expect("the queue counter answers on this machine");
        assert!(first.one >= 0.0);
        module.sample();
        let load = module.load_average().expect("still answering");
        assert_eq!(load.five, load.five, "not NaN");
        assert!(load.one >= 0.0 && load.one < 10_000.0, "{load:?}");
        // And the stack reads the one-minute figure.
        let text = module
            .stack_value(&crate::stack::StackMetric::CpuLoad, crate::weather::models::TemperatureUnit::Celsius);
        assert!(text.is_none() || text.as_deref().is_some_and(|t| t.contains('.')), "{text:?}");
    }
}
