// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Talking to the sensor helper.
//
// The helper is a separate process because LibreHardwareMonitor is .NET, and
// hosting a runtime inside a monitor that exists to be small would undo the
// point. Ahead-of-time compilation was tried and does not work with it - see
// AGENTS.md - so the runtime lives over there, where it costs nothing while
// sensors are switched off.
//
// One line out, one line back. The parent asks; the helper answers. Nothing
// samples hardware unless somebody wanted a reading.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

use super::{Sensor, SensorError, SensorKind, SensorProvider};

/// Kind codes shared with the helper. Ours rather than LibreHardwareMonitor's
/// enum values, so an upstream renumbering cannot silently relabel readings.
fn kind_from_code(code: i64) -> SensorKind {
    match code {
        1 => SensorKind::Voltage,
        2 => SensorKind::Current,
        3 => SensorKind::Power,
        4 => SensorKind::Clock,
        5 => SensorKind::Temperature,
        6 => SensorKind::Load,
        7 => SensorKind::Other("Frequency".into()),
        8 => SensorKind::Fan,
        9 => SensorKind::Flow,
        10 => SensorKind::Control,
        11 => SensorKind::Level,
        12 => SensorKind::Factor,
        13 => SensorKind::Data,
        14 => SensorKind::SmallData,
        15 => SensorKind::Throughput,
        16 => SensorKind::Other("TimeSpan".into()),
        17 => SensorKind::Energy,
        18 => SensorKind::Noise,
        19 => SensorKind::Humidity,
        _ => SensorKind::Other(String::new()),
    }
}

/// A running sensor helper.
pub struct HelperProvider {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    devices: u64,
}

impl HelperProvider {
    /// Starts the helper and waits for it to announce itself.
    ///
    /// `library` is where Barometer put LibreHardwareMonitor, when Barometer
    /// was the one that fetched it. Passed as an argument rather than through
    /// the environment, for two reasons. A child inherits the environment as
    /// it stood at spawn, so an install finishing has to reach the *next*
    /// helper and setting a variable does nothing for the one already running;
    /// and `set_var` is a process-wide write, which a program with a sensor
    /// worker and a settings window running alongside the strip has no
    /// business making. `None` leaves the helper to its own search, which is
    /// what finds an installation the user made themselves.
    pub fn spawn(
        executable: &Path,
        library: Option<&Path>,
    ) -> Result<HelperProvider, SensorError> {
        if !executable.exists() {
            return Err(SensorError::NotConfigured);
        }

        // CREATE_NO_WINDOW, and it is not cosmetic. The helper is a .NET
        // console program, so spawning it normally gives it a console of its
        // own - a black window that opens on top of whatever the user was
        // doing, every time Barometer starts, and stays there for the life of
        // the program. Barometer's own binary is GUI subsystem and shows
        // nothing; this was the window people were seeing.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let mut command = Command::new(executable);
        if let Some(directory) = library {
            // `--library` is the helper's first-choice source and outranks its
            // own search, which is right: a path we were given is an answer,
            // and a search that then picked a different copy off the machine
            // would be second-guessing it.
            command.arg("--library").arg(directory);
        }

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Diagnostics are the helper's business and must never be mixed
            // into the protocol stream.
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| SensorError::Transport(e.to_string()))?;

        confine_to_job(&child);

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SensorError::Transport("helper stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SensorError::Transport("helper stdout unavailable".into()))?;
        let mut stdout = BufReader::new(stdout);

        let mut greeting = String::new();
        stdout.read_line(&mut greeting).map_err(|e| SensorError::Transport(e.to_string()))?;
        let greeting: Value = serde_json::from_str(greeting.trim())
            .map_err(|e| SensorError::Malformed(format!("greeting: {e}")))?;
        match greeting.get("status").and_then(Value::as_str) {
            Some("ready") => {}
            // Not a malfunction: the helper ran, looked, and there is no
            // library on this machine yet. It is the state every fresh install
            // is in, and the settings pane has a button for it - so it has to
            // arrive as `NotConfigured` and not as "the helper said something
            // strange", which is what the pane would otherwise have to explain.
            Some("no-library") => return Err(SensorError::NotConfigured),
            _ => return Err(SensorError::Malformed("helper did not report ready".into())),
        }
        let devices = greeting.get("devices").and_then(Value::as_u64).unwrap_or(0);

        Ok(HelperProvider { child, stdin, stdout, devices })
    }

    /// Devices found when the helper opened. Zero means it ran but saw no
    /// hardware, which is a different problem from it not running at all.
    pub fn devices(&self) -> u64 {
        self.devices
    }
}

impl Drop for HelperProvider {
    fn drop(&mut self) {
        // Ask, then insist. The job object would take it down regardless, but
        // an orderly close lets the library release its driver handle rather
        // than leaving it to process teardown.
        let _ = self.stdin.write_all(b"quit\n");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl SensorProvider for HelperProvider {
    fn name(&self) -> &str {
        "LibreHardwareMonitor"
    }

    fn read(&mut self) -> Result<Vec<Sensor>, SensorError> {
        self.stdin
            .write_all(b"read\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|_| SensorError::NotRunning)?;

        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| SensorError::Transport(e.to_string()))?;
        if read == 0 {
            // End of stream: the helper exited.
            return Err(SensorError::NotRunning);
        }

        let payload: Value =
            serde_json::from_str(line.trim()).map_err(|e| SensorError::Malformed(e.to_string()))?;
        let entries = payload
            .get("sensors")
            .and_then(Value::as_array)
            .ok_or_else(|| SensorError::Malformed("no sensors array".into()))?;

        Ok(entries
            .iter()
            .map(|entry| Sensor {
                id: entry.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                name: entry.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                hardware: entry
                    .get("hardware")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                kind: kind_from_code(entry.get("kind").and_then(Value::as_i64).unwrap_or(0)),
                // null, not zero: an unreadable fan is not a stopped fan.
                value: entry.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()),
            })
            .collect())
    }
}

/// Puts the child in a job object that kills it when our handle closes.
///
/// Without this, a Barometer that crashes leaves a .NET process running with a
/// driver handle open, and the user has to find it in Task Manager, which they
/// will not. Best effort: failing to confine it is not a reason to refuse the
/// sensors, only a reason not to rely on it for cleanup.
fn confine_to_job(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    // SAFETY: a fresh unnamed job object, configured and then handed the child.
    // The handle is deliberately never closed: closing it kills the child, and
    // its lifetime is meant to be the lifetime of this process.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return;
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of_mut!(limits).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        AssignProcessToJobObject(job, child.as_raw_handle() as _);
    }
}
