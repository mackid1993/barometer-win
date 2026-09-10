// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Getting the sensor helper to let go of the library.
//
// The helper loads LibreHardwareMonitorLib.dll and holds it open for as long
// as it runs, which is as long as Barometer runs. That is what you want while
// readings are arriving and exactly what you do not want while somebody is
// installing, reinstalling or removing the library underneath it: Windows will
// not delete a file that is mapped into a live process.
//
// So the install code asks first. `hold()` tells the sensor worker to close
// the helper and waits until it has, and the hold being dropped is what lets
// the worker open a fresh one - against whatever is on disk by then. Nobody
// presses anything and nothing has to be restarted; installing the library is
// what makes the temperatures appear, which is what a person pressing Install
// already believed they were doing.
//
// One static, because there is one helper. Passing a handle from the strip's
// thread to the settings window's would mean the settings window could only
// remove the library while the strip was running, and the settings window can
// be opened with `--settings` on its own.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long a hold waits for the helper to close before going ahead regardless.
///
/// The ordinary case is milliseconds: the worker is asleep between reads, the
/// condvar wakes it at once, and closing the helper is a write and a wait.
/// The cap is for a helper wedged inside a read that will never return, where
/// waiting longer only trades a frozen settings window for a delete that was
/// going to fail either way. Failing honestly after three seconds beats
/// hanging.
const PATIENCE: Duration = Duration::from_secs(3);

struct State {
    /// How many callers want the library to themselves.
    ///
    /// A count rather than a flag so that two overlapping holds - an install
    /// worker and a removal, say - cannot have the first one to finish let the
    /// helper back in while the second is still writing.
    holders: u32,
    /// True while the helper has the library open.
    loaded: bool,
}

struct Library {
    state: Mutex<State>,
    changed: Condvar,
}

static LIBRARY: Library = Library {
    state: Mutex::new(State { holders: 0, loaded: false }),
    changed: Condvar::new(),
};

/// The lock, with a poisoned mutex treated as an ordinary one.
///
/// Poisoning would mean a thread panicked mid-update, and the two fields here
/// are a counter and a bool that every path re-derives rather than accumulates.
/// Refusing to install the library because a worker thread once panicked would
/// be a worse outcome than carrying on with the counts we have.
fn lock() -> MutexGuard<'static, State> {
    LIBRARY.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Exclusive use of the library files, for as long as this is alive.
///
/// The helper is closed before this returns and reopened when it is dropped,
/// so the whole of an install or a removal goes between the two. Readings stop
/// for that window; there is no way to both replace a file and keep reading it.
#[must_use = "dropping the hold immediately is the same as not taking one"]
pub struct Hold(());

/// Closes the sensor helper and keeps it closed.
///
/// Returns once the helper has let the library go, or after `PATIENCE` if it
/// will not. Safe to call when nothing is running - there is then nothing to
/// wait for and it returns at once - which is what happens under `--settings`,
/// where the settings window is the whole program.
pub fn hold() -> Hold {
    let mut state = lock();
    state.holders += 1;
    // Wakes the worker out of its sleep rather than leaving it to notice on
    // its next pass, which is the difference between a hold that costs
    // milliseconds and one that costs the polling interval.
    LIBRARY.changed.notify_all();

    let deadline = Instant::now() + PATIENCE;
    while state.loaded {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let (next, _) = LIBRARY
            .changed
            .wait_timeout(state, deadline - now)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state = next;
    }
    Hold(())
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut state = lock();
        state.holders = state.holders.saturating_sub(1);
        drop(state);
        LIBRARY.changed.notify_all();
    }
}

// ---- the worker's side -----------------------------------------------------

/// Whether somebody is waiting for the library.
pub(crate) fn wanted() -> bool {
    lock().holders > 0
}

/// Claims the library for the helper, refusing if somebody is waiting for it.
///
/// The check and the claim are one operation on purpose. Testing `wanted()`
/// and then spawning would leave a window in which a hold saw an idle helper,
/// went ahead and started deleting, and the worker opened the library out from
/// under it.
pub(crate) fn opening() -> bool {
    let mut state = lock();
    if state.holders > 0 {
        return false;
    }
    state.loaded = true;
    true
}

/// The helper has closed, however that came about.
///
/// Idempotent, because the worker calls it on every path that ends without a
/// provider - a spawn that failed, a read that ended the stream, a library
/// that moved - rather than tracking which of them had claimed it.
pub(crate) fn closed() {
    let mut state = lock();
    state.loaded = false;
    drop(state);
    LIBRARY.changed.notify_all();
}

/// Waits for a hold to end, for up to `how_long`.
///
/// The mirror of `rest`, and the reason the worker does not spin while an
/// install is running: `rest` returns at once when somebody wants the library,
/// which is right between reads and would be a busy loop here. Bounded rather
/// than open-ended so the worker keeps getting back to its stop flag - an
/// install that wedges must not also stop Barometer from exiting.
pub(crate) fn wait_while_held(how_long: Duration) {
    let mut state = lock();
    let deadline = Instant::now() + how_long;
    while state.holders > 0 {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let (next, _) = LIBRARY
            .changed
            .wait_timeout(state, deadline - now)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state = next;
    }
}

/// Bumped by `wake`, so a sleeper can tell a nudge meant for it from the
/// notification a hold or a close sends.
///
/// Without it `wake` did nothing at all: `rest` woke, found no holders and
/// no deadline reached, and went straight back to sleep. Shutdown waited out
/// the whole poll interval it was documented not to, and there was no way to
/// bring a worker resting on the idle cadence back to the user's.
static NUDGE: AtomicU64 = AtomicU64::new(0);

/// Sleeps between reads, waking early if somebody wants the library or has
/// asked for a reading now.
pub(crate) fn rest(how_long: Duration) {
    let since = NUDGE.load(Ordering::Acquire);
    let mut state = lock();
    let deadline = Instant::now() + how_long;
    while state.holders == 0 && NUDGE.load(Ordering::Acquire) == since {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let (next, _) = LIBRARY
            .changed
            .wait_timeout(state, deadline - now)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state = next;
    }
}

/// Wakes anything sleeping in `rest`: at shutdown, so the process does not
/// wait out a poll, and when something that shows a reading has just
/// appeared, so the first one is not an idle interval away.
pub(crate) fn wake() {
    NUDGE.fetch_add(1, Ordering::AcqRel);
    LIBRARY.changed.notify_all();
}

// ---- test support ----------------------------------------------------------
//
// There is one library and one static tracking it, so any test that touches it
// has to have it to itself - including the sensor module's supervision tests,
// which drive this from another file. Hence a lock shared across both rather
// than a private one in each.

/// Held for the length of a test that touches the library state.
#[cfg(test)]
pub(crate) fn sequence() -> std::sync::MutexGuard<'static, ()> {
    static SEQUENCE: Mutex<()> = Mutex::new(());
    let guard = SEQUENCE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Whatever the last test left behind. Each one starts from a closed helper
    // that nobody is waiting on, which is the state a fresh process is in.
    let mut state = lock();
    state.holders = 0;
    state.loaded = false;
    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn a_hold_over_a_closed_helper_returns_at_once() {
        let _sequence = sequence();
        let started = Instant::now();
        let hold = hold();
        assert!(started.elapsed() < PATIENCE, "waited for a helper that was not running");
        drop(hold);
    }

    #[test]
    fn the_helper_cannot_open_the_library_while_a_hold_is_waiting() {
        let _sequence = sequence();
        let hold = hold();
        // This is the race the combined check-and-claim exists to close: a
        // worker deciding to spawn while a removal is deleting the files.
        assert!(!opening(), "the worker opened the library during a hold");
        drop(hold);
        assert!(opening(), "the worker was still locked out after the hold ended");
        closed();
    }

    #[test]
    fn a_hold_waits_for_an_open_helper_and_is_released_when_it_closes() {
        let _sequence = sequence();
        assert!(opening());

        let asked = Arc::new(AtomicBool::new(false));
        let worker = thread::spawn({
            let asked = Arc::clone(&asked);
            move || {
                // Stands in for the sensor worker: sleeps until somebody wants
                // the library, then closes the helper.
                rest(Duration::from_secs(30));
                asked.store(wanted(), Ordering::Release);
                closed();
            }
        });

        let started = Instant::now();
        let hold = hold();
        // It waited for the close rather than sailing past an open helper,
        // and it did not sit out the full sleep to get there.
        assert!(asked.load(Ordering::Acquire), "the worker was not woken by the hold");
        assert!(started.elapsed() < PATIENCE, "the hold waited out its patience instead of being released");
        assert!(!lock().loaded);
        worker.join().unwrap();
        drop(hold);
    }

    #[test]
    fn two_overlapping_holds_both_have_to_end_before_the_helper_returns() {
        let _sequence = sequence();
        let first = hold();
        let second = hold();
        drop(first);
        assert!(!opening(), "the second hold was released by the first one ending");
        drop(second);
        assert!(opening());
        closed();
    }

    #[test]
    fn a_wedged_helper_does_not_hang_the_caller_forever() {
        let _sequence = sequence();
        // Open and never close: a helper stuck inside a read.
        assert!(opening());
        let started = Instant::now();
        let hold = hold();
        let waited = started.elapsed();
        assert!(waited >= PATIENCE, "gave up before giving the helper its time");
        assert!(waited < PATIENCE * 3, "waited far past its patience");
        drop(hold);
        closed();
    }
}
