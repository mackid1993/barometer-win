// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// What the sensor helper is currently doing, for anybody who needs to say so.
//
// The strip owns the Sensors module and the settings window does not, which is
// right - one thread samples hardware and the other draws a dialog, and giving
// the dialog a handle on the sampler would mean the two had to agree about
// locking. But the pane whose whole job is explaining the sensor source then
// has nothing to explain it from, and it said "Waiting for the sensor helper"
// forever, on a machine where temperatures were arriving the entire time.
//
// So the worker publishes and anybody reads. One value, last writer wins,
// because there is one helper and the only question ever asked of it is what
// it is doing right now.

use std::sync::{Mutex, MutexGuard};

use super::SensorError;

/// What the sensor source is doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Health {
    /// Nothing has reported yet.
    ///
    /// Only true for the first second or so of a run, and in the settings
    /// window opened with `--settings`, where there is no strip behind it and
    /// so no worker to report anything. It must never be what a pane shows on
    /// a working machine.
    #[default]
    Unknown,
    /// Readings are arriving.
    Providing {
        /// The provider's name, for the pane to say which source answered.
        ///
        /// Owned rather than borrowed: the name comes off the provider, and it
        /// is wanted after the provider has been dropped.
        source: String,
        /// How many sensors the last read returned, and how many of those were
        /// temperatures. A source that answers with nothing is running but
        /// useless, and that is worth telling apart from one that is not
        /// running at all.
        sensors: usize,
        temperatures: usize,
    },
    /// No source is installed.
    NotConfigured,
    /// A source is installed but the helper would not start or has stopped.
    NotRunning,
    /// The helper answered with something unusable.
    Failed(String),
}

impl Health {
    /// Whether readings are currently reaching the strip.
    pub fn is_providing(&self) -> bool {
        matches!(self, Health::Providing { .. })
    }

    /// How a failed read maps onto this.
    pub(crate) fn from_error(why: &SensorError) -> Health {
        match why {
            SensorError::NotConfigured => Health::NotConfigured,
            SensorError::NotRunning => Health::NotRunning,
            // Both mean the helper is there and something is wrong with what
            // it said, which is a different sentence from "it is not running"
            // and a different thing for the user to do about it.
            SensorError::Unauthorized | SensorError::Malformed(_) | SensorError::Transport(_) => {
                Health::Failed(why.to_string())
            }
        }
    }
}

static HEALTH: Mutex<Health> = Mutex::new(Health::Unknown);

/// A poisoned mutex treated as an ordinary one.
///
/// The value is replaced wholesale on every write and never accumulated, so a
/// thread that panicked mid-update leaves nothing inconsistent behind. Refusing
/// to report the sensor state because a worker once panicked would be strictly
/// worse than reporting the state we have.
fn lock() -> MutexGuard<'static, Health> {
    HEALTH.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Says what the sensor source is doing. Called by the sensor worker.
pub fn publish(health: Health) {
    *lock() = health;
}

/// What the sensor source is doing.
pub fn health() -> Health {
    lock().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_source_is_reported_as_reading() {
        // The bug this exists to fix: the pane said "waiting for the sensor
        // helper" on a machine that had been reporting temperatures for
        // minutes, because nothing ever told it otherwise.
        publish(Health::Providing {
            source: "LibreHardwareMonitor".into(),
            sensors: 87,
            temperatures: 43,
        });
        assert!(health().is_providing());
        publish(Health::Unknown);
    }

    #[test]
    fn the_errors_a_pane_can_act_on_stay_apart() {
        // Each of these asks something different of the user - install a
        // source, wait for one to start, or read what went wrong - so folding
        // them together would lose the only thing the pane is for.
        assert_eq!(Health::from_error(&SensorError::NotConfigured), Health::NotConfigured);
        assert_eq!(Health::from_error(&SensorError::NotRunning), Health::NotRunning);
        assert!(matches!(
            Health::from_error(&SensorError::Malformed("bad json".into())),
            Health::Failed(_)
        ));
        assert!(matches!(
            Health::from_error(&SensorError::Transport("pipe closed".into())),
            Health::Failed(_)
        ));
    }
}
