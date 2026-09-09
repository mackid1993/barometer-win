// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Starting with Windows, as a logon task.
//!
//! A scheduled task rather than the `Run` key, which is the obvious choice
//! and the wrong one here: Barometer's manifest asks for administrator - it
//! has to, to reach the sensor driver - and Windows never elevates a `Run`
//! entry. An entry there would sit in Task Manager's Startup tab looking
//! correct and simply never start the program, which is worse than having no
//! switch at all.
//!
//! A logon task registered with the highest privileges does start it, and is
//! the mechanism every other elevated tray program uses for this. Creating
//! one needs administrator, which is exactly what this process already has.
//!
//! `schtasks.exe` rather than the Task Scheduler COM interfaces: the whole of
//! what is needed here is create, delete and ask, the tool has done those
//! three things unchanged since Windows XP, and the alternative is a large
//! amount of COM for no more capability. It is run by absolute path, so what
//! runs is the system's copy and not something earlier on the PATH.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// The task's name, which is what the user sees in Task Scheduler.
const TASK: &str = "Barometer";

/// Where `schtasks.exe` is, whatever the PATH says.
fn schtasks() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root).join(r"System32\schtasks.exe")
}

/// Runs the tool and says whether it succeeded.
///
/// No window: this is called from the settings window's own thread and a
/// console flashing up in the middle of the screen is not what pressing a
/// switch should look like.
fn run(arguments: &[&str]) -> bool {
    Command::new(schtasks())
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Whether Barometer is registered to start at logon.
pub fn is_on() -> bool {
    run(&["/Query", "/TN", TASK])
}

/// Registers the logon task, or removes it.
///
/// Returns whether the change took. The caller reads `is_on` back afterwards
/// rather than trusting this, so a machine that refuses - a policy, a group
/// that cannot register tasks - leaves the switch showing what is actually
/// so instead of what was pressed.
pub fn set(on: bool) -> bool {
    if !on {
        // Already absent is the state that was asked for, not a failure.
        return run(&["/Delete", "/TN", TASK, "/F"]) || !is_on();
    }
    let Some(command) = command() else { return false };
    // /RL HIGHEST is the whole point: without it the task starts Barometer
    // without the rights its manifest asks for and it exits again.
    // /F replaces an existing task, so switching this off and on again, or
    // moving the program, does not leave the old one behind.
    run(&["/Create", "/TN", TASK, "/TR", &command, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F"])
}

/// What the task should run, quoted.
///
/// Quoted because the installer puts Barometer under Program Files and an
/// unquoted path with a space in it is the oldest way there is to have
/// Windows start the wrong program.
fn command() -> Option<String> {
    let path = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_quoted_so_a_path_with_spaces_starts_the_right_program() {
        let command = command().expect("this test runs from an executable");
        assert!(command.starts_with('"'), "{command}");
        assert!(command.ends_with('"'), "{command}");
        assert!(command.contains(".exe"), "{command}");
    }

    #[test]
    fn the_tool_is_taken_from_the_system_and_not_from_the_path() {
        let path = schtasks();
        assert!(path.is_absolute(), "{}", path.display());
        assert!(path.ends_with("schtasks.exe"), "{}", path.display());
    }

    #[test]
    fn asking_and_unasking_leaves_the_machine_as_it_was_found() {
        // The real scheduler, because the point of this module is that it is
        // the real scheduler; it is put back whichever way it started.
        // Registering a task needs administrator, and the test runner may not
        // have it, so a refusal ends the test rather than failing it - there
        // is nothing to check and nothing was changed.
        let before = is_on();
        if !set(!before) {
            return;
        }
        assert_eq!(is_on(), !before);
        assert!(set(before));
        assert_eq!(is_on(), before);
    }
}
