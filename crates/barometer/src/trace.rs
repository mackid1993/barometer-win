// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Writing down what the strip did, when it is not being watched.
//
// There was already a trace behind BAROMETER_TRACE, and it was useless for the
// bug it was written for. The strip is a GUI subsystem binary with no console,
// so `eprintln!` goes nowhere unless somebody launched it from a terminal with
// a redirect - and the copy that misbehaves is the one the scheduled task
// starts at sign-in, elevated, with no terminal anywhere near it. The one time
// the trace was needed it could not be read.
//
// So it goes to a file, and it is switched on by the presence of another file
// rather than by an environment variable. A variable would have to be set for
// the task's own environment, which means editing the task; a file in the app's
// local-data folder is something a person can create in Explorer and delete
// afterwards.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Create this in %LOCALAPPDATA%Barometer to switch tracing on. Local rather
/// than roaming, beside the log it switches on, because a trace is about this
/// machine and should not follow the user to another.
const SWITCH: &str = "trace.on";
const LOG: &str = "trace.log";

fn folder() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Barometer"))
}

/// Where the log goes, when tracing is on at all.
///
/// Decided once. The switch is not re-read on every line: this is called from
/// the sampling loop, and a file system check per tick to answer a question
/// that changes once a month is not a trade worth making.
fn destination() -> Option<&'static PathBuf> {
    static WHERE: OnceLock<Option<PathBuf>> = OnceLock::new();
    WHERE
        .get_or_init(|| {
            let folder = folder()?;
            // The environment variable still works, for anybody running the
            // console modes from a terminal where it was already the habit.
            let asked = folder.join(SWITCH).exists()
                || std::env::var_os("BAROMETER_TRACE").is_some();
            let path = asked.then(|| folder.join(LOG))?;
            // What a previous run left is part of the budget, or a log already
            // at the cap would be allowed to grow to twice it before the count
            // this run keeps caught up.
            if let Ok(existing) = std::fs::metadata(&path) {
                WRITTEN.store(existing.len(), Ordering::Relaxed);
            }
            Some(path)
        })
        .as_ref()
}

/// Whether anything is being written, so a caller can skip building a line.
pub fn on() -> bool {
    destination().is_some()
}

/// How large the log is allowed to get before the previous one is thrown away
/// and a fresh one started.
///
/// The strip writes two or three lines a second, so an unrotated trace is
/// something like twenty megabytes a day and does not stop: `trace.on` is a
/// file somebody creates to catch a bug and then forgets about, and the whole
/// reason it is a file rather than an environment variable is that it outlives
/// the session that made it. Two generations of this is a quarter of an hour of
/// history at that rate, which is far more than any of these traces has needed,
/// and it is bounded.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Appends one line, with the time it happened.
///
/// Best effort throughout. A trace that panicked, or that stopped the strip
/// sampling because a disk was full, would be worse than no trace: this is
/// diagnostic machinery and it has no business affecting what it observes.
pub fn line(text: &str) {
    let Some(path) = destination() else { return };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else { return };
    let line = format!("{} {text}\n", stamp());
    let _ = file.write_all(line.as_bytes());
    // Counted rather than measured. `metadata` would be a second call on every
    // line for an answer that only changes at one known moment, and the size at
    // startup is the only one this cannot work out for itself.
    let written = WRITTEN.fetch_add(line.len() as u64, Ordering::Relaxed) + line.len() as u64;
    if written >= MAX_BYTES {
        WRITTEN.store(0, Ordering::Relaxed);
        // Closed first: a rename with the handle still open is refused on
        // Windows, and the next line would then go on extending the file this
        // was meant to retire.
        drop(file);
        // Whatever was kept last time goes. One generation back, not a numbered
        // series: a trace nobody has looked at in two rotations is not going to
        // be looked at.
        let _ = std::fs::remove_file(path.with_extension("log.1"));
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
}

/// Bytes written to the current log by this process, for the cap above.
///
/// Seeded from whatever a previous run left, so a log already at the limit is
/// rotated on the first line rather than doubling.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// Milliseconds since the machine started.
///
/// Not a wall clock, and deliberately: what a trace is read for is how long
/// something took and what happened either side of it, and this is one call
/// with no formatting, no time zone and no dependency.
fn stamp() -> String {
    // SAFETY: takes no arguments and cannot fail.
    let ms = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
    format!("[{:>10}]", ms % 100_000_000)
}

/// Notes that the strip has started, so a log read later says which run it is.
pub fn opened() {
    if !on() {
        return;
    }
    line("----");
    line(&format!(
        "barometer {} starting, tracing to this file",
        env!("CARGO_PKG_VERSION")
    ));
}
