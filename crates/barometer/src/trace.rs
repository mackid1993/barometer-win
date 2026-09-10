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
            asked.then(|| folder.join(LOG))
        })
        .as_ref()
}

/// Whether anything is being written, so a caller can skip building a line.
pub fn on() -> bool {
    destination().is_some()
}

/// Appends one line, with the time it happened.
///
/// Best effort throughout. A trace that panicked, or that stopped the strip
/// sampling because a disk was full, would be worse than no trace: this is
/// diagnostic machinery and it has no business affecting what it observes.
pub fn line(text: &str) {
    let Some(path) = destination() else { return };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else { return };
    let _ = writeln!(file, "{} {text}", stamp());
}

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
