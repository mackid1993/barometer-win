// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

use std::time::Instant;

use crate::format::{self, RateUnit};
use crate::module::{Module, ModuleId, Readout};
use crate::sys::{self, NetCounters};

/// Throughput in and out, in bytes per second.
///
/// The interface counters are cumulative since boot, so a rate is a delta over
/// a measured interval. The interval is measured rather than assumed: the
/// sampling thread can be late, and dividing by the interval we asked for
/// rather than the one we got turns a delayed tick into a spike.
pub struct NetworkModule {
    previous: Option<(NetCounters, Instant)>,
    down: f64,
    up: f64,
    reading: bool,
    upload_first: bool,
    unit: RateUnit,
    /// The interface to follow, or None for every one of them summed.
    interface: Option<String>,
}

impl Default for NetworkModule {
    fn default() -> Self {
        NetworkModule {
            previous: None,
            down: 0.0,
            up: 0.0,
            reading: false,
            // Off, so the readout leads with the download - the Mac app's
            // order. See `readout` for why it is a preference at all.
            upload_first: false,
            unit: RateUnit::default(),
            interface: None,
        }
    }
}

impl NetworkModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current rates in bytes per second, down and up, for the detail panel
    /// and for the history graph.
    pub fn rates(&self) -> Option<(f64, f64)> {
        self.reading.then_some((self.down, self.up))
    }
}

/// A counter that has gone backwards means the counter reset, not that
/// negative traffic occurred. An adapter being disabled, a dock being
/// unplugged, or a driver reloading all restart the count from zero, and the
/// honest reading for that interval is "unknown", which we render as no
/// traffic rather than as an enormous spike.
fn delta(now: u64, before: u64) -> Option<u64> {
    now.checked_sub(before)
}

impl Module for NetworkModule {
    fn id(&self) -> ModuleId {
        ModuleId::Network
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
            NetworkDownload => Some(format::rate_in(self.down, self.unit)),
            NetworkUpload => Some(format::rate_in(self.up, self.unit)),
            _ => None,
        }
    }

    fn rates(&self) -> Option<(f64, f64)> {
        NetworkModule::rates(self)
    }

    fn configure(&mut self, settings: &crate::store::Settings) {
        self.upload_first = settings.network_upload_first;
        self.unit = settings.network_unit;
        if self.interface != settings.network_interface {
            // The counters of two interfaces are not comparable, so the
            // baseline goes with the choice; keeping it would show one
            // enormous rate for the interval the choice was made in.
            self.interface = settings.network_interface.clone();
            self.previous = None;
            self.reading = false;
        }
    }

    fn net_interfaces(&self) -> Vec<(String, bool)> {
        sys::net_interfaces().into_iter().map(|found| (found.name, found.is_tunnel)).collect()
    }

    fn sample(&mut self) {
        let taken_at = Instant::now();
        let read = match &self.interface {
            Some(name) => sys::net_counters_of(name),
            None => sys::net_counters(),
        };
        let Some(counters) = read else {
            self.reading = false;
            self.previous = None;
            return;
        };

        if let Some((previous, previous_at)) = self.previous {
            let elapsed = taken_at.duration_since(previous_at).as_secs_f64();
            // Guard the divisor rather than the numerator: a zero or absurdly
            // small interval is the one input that turns a legitimate delta
            // into an infinity.
            if elapsed > 0.001 {
                match (
                    delta(counters.received, previous.received),
                    delta(counters.sent, previous.sent),
                ) {
                    (Some(rx), Some(tx)) => {
                        self.down = rx as f64 / elapsed;
                        self.up = tx as f64 / elapsed;
                        self.reading = true;
                    }
                    _ => {
                        // Counters restarted. Drop this interval and rebase on
                        // the new values below.
                        self.down = 0.0;
                        self.up = 0.0;
                        self.reading = true;
                    }
                }
            }
        }

        self.previous = Some((counters, taken_at));
    }

    fn readout(&self) -> Readout {
        if !self.reading {
            return Readout::unavailable();
        }
        // Download over upload by default, matching the order the Mac app
        // uses, and swappable because which of two stacked rates the eye should
        // land on first is a real preference - somebody watching an upload
        // wants it on top.
        //
        // The arrows are U+2193 and U+2191, the pair TrafficMonitor uses, and
        // they are written as escapes rather than as literal characters on
        // purpose: a sibling project had exactly these two glyphs turned into
        // mojibake by a text round trip, and an escape cannot suffer that.
        let (first, second) = if self.upload_first {
            (self.up, self.down)
        } else {
            (self.down, self.up)
        };
        let (first_arrow, second_arrow) = if self.upload_first {
            ("\u{2191}", "\u{2193}")
        } else {
            ("\u{2193}", "\u{2191}")
        };
        // Named for their rows rather than their directions: which rate is on
        // top is exactly what `upload_first` decides.
        let (top, bottom) = (
            format!("{first_arrow} {}", format::rate_in(first, self.unit)),
            format!("{second_arrow} {}", format::rate_in(second, self.unit)),
        );
        // Reserved at the widest a rate can be, so the columns to the right of
        // this one hold still while these numbers move. Both conventions are
        // written to the same width, so switching bytes to bits does not shove
        // everything after this column sideways.
        let reserved = format!("\u{2193} {}", self.unit.widest());
        Readout::two(top, bottom).reserving(reserved)
    }
}
