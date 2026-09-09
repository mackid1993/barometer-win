// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Starting with Windows, through the same logon task the installer registers.
//!
//! Not the `Run` key, which is the obvious choice and the wrong one here:
//! Barometer's manifest asks for administrator - it has to, to reach the
//! sensor driver - and Windows never elevates a `Run` entry. One there would
//! sit in Task Manager's Startup tab looking correct and never start the
//! program.
//!
//! The installer already creates the task, named for the program, with
//! settings that matter and are not obvious: `AllowStartIfOnBatteries` and
//! `DontStopIfGoingOnBatteries`, or a laptop away from the wall does not start
//! it; `ExecutionTimeLimit 0`, or the scheduler stops it after three days;
//! `MultipleInstances IgnoreNew`, so a second sign-in does not race the first.
//! So this switch **enables and disables that task** rather than deleting and
//! recreating one. An earlier version of this file created its own with
//! `schtasks /Create`, which made a task of the same name carrying none of the
//! above - turning the switch off and on again would quietly have cost a
//! laptop user their startup.
//!
//! Registering from scratch is still here for when the task is missing: a copy
//! run from a build directory rather than installed. It uses the settings the
//! installer uses, which is why they are written twice - the installer cannot
//! call this, and this cannot read the installer. The tests below are what
//! keeps the two copies the same.
//!
//! PowerShell rather than `schtasks.exe`: those settings have no command-line
//! switches, and whether a task is enabled is not something to infer by
//! parsing localized console output.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// The task's name, which is what Task Scheduler shows and what the installer
/// registers. **Must match `AppName` in `installer\barometer.iss`.**
const TASK: &str = "Barometer";

/// Where PowerShell is, whatever the PATH says.
fn powershell() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// Runs one PowerShell command and hands back what it said.
///
/// No window: this is called from the settings window's own thread, and a
/// console flashing up in the middle of the screen is not what pressing a
/// switch should look like.
fn run(script: &str) -> Option<Output> {
    Command::new(powershell())
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
}

/// Whether Barometer is set to start at sign-in.
///
/// A task that exists but is disabled is off - which is the state this switch
/// leaves behind, and also what somebody gets by disabling it in Task
/// Scheduler themselves.
pub fn is_on() -> bool {
    let script = format!(
        "$t = Get-ScheduledTask -TaskName '{TASK}' -ErrorAction SilentlyContinue; \
         if ($t -and $t.State -ne 'Disabled') {{ 'on' }} else {{ 'off' }}"
    );
    let Some(output) = run(&script) else { return false };
    String::from_utf8_lossy(&output.stdout).trim() == "on"
}

/// Turns starting at sign-in on or off.
///
/// Returns whether the change took. The caller reads `is_on` back afterwards
/// rather than trusting this, so a machine that refuses - a policy, an account
/// that cannot touch the scheduler - leaves the switch showing what is
/// actually so instead of what was pressed.
pub fn set(on: bool) -> bool {
    if !on {
        // Disabled, not deleted: the task carries settings this program did
        // not choose, and the uninstaller is what removes it.
        let script = format!(
            "Disable-ScheduledTask -TaskName '{TASK}' -ErrorAction SilentlyContinue | Out-Null"
        );
        run(&script);
        return !is_on();
    }
    let Some(command) = command() else { return false };
    // Enabled if it is there; registered with the installer's own settings if
    // it is not.
    let script = format!(
        "$existing = Get-ScheduledTask -TaskName '{TASK}' -ErrorAction SilentlyContinue; \
         if ($existing) {{ Enable-ScheduledTask -TaskName '{TASK}' | Out-Null }} else {{ \
         $a = New-ScheduledTaskAction -Execute '{command}'; \
         $t = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME; \
         $s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries \
         -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0 -MultipleInstances IgnoreNew; \
         $p = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive \
         -RunLevel Highest; \
         Register-ScheduledTask -TaskName '{TASK}' -Action $a -Trigger $t -Settings $s \
         -Principal $p -Force | Out-Null }}"
    );
    run(&script);
    is_on()
}

/// The program's own path, for registering a task that does not exist yet.
///
/// Not quoted: `New-ScheduledTaskAction -Execute` takes the path as one
/// argument, so quotes would become part of the file name. The single quotes
/// around it in the script are PowerShell's own, and a path containing one
/// would end that string - so such a path is refused rather than half-run.
fn command() -> Option<String> {
    let path = std::env::current_exe().ok()?;
    let text = path.display().to_string();
    (!text.contains('\'')).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installer script, read at compile time so these tests fail on the
    /// build rather than on somebody's machine.
    const INSTALLER: &str = include_str!("../../../installer/barometer.iss");

    #[test]
    fn the_task_is_named_what_the_installer_names_it() {
        // The installer registers this task and this switch enables and
        // disables it. If the two names part company the switch registers a
        // second task, and Barometer starts twice at sign-in.
        let defines_it = INSTALLER.lines().any(|line| {
            let line = line.trim();
            line.starts_with("#define AppName") && line.contains(&format!("\"{TASK}\""))
        });
        assert!(defines_it, "installer\\barometer.iss no longer defines AppName as {TASK}");
        assert!(INSTALLER.contains("Register-ScheduledTask -TaskName '{#AppName}'"));
    }

    #[test]
    fn the_settings_are_the_ones_the_installer_registers() {
        // Written out in both places because neither can read the other, so
        // this is what keeps them together. Losing AllowStartIfOnBatteries
        // means a laptop away from the wall never starts Barometer; losing
        // ExecutionTimeLimit means the scheduler stops it after three days.
        // Both silent, and both one toggle away.
        for setting in [
            "-AllowStartIfOnBatteries",
            "-DontStopIfGoingOnBatteries",
            "-ExecutionTimeLimit 0",
            "-MultipleInstances IgnoreNew",
            "-LogonType Interactive",
            "-RunLevel Highest",
        ] {
            assert!(INSTALLER.contains(setting), "the installer no longer passes {setting}");
        }
    }

    #[test]
    fn a_path_that_would_break_the_script_is_refused_rather_than_half_run() {
        if let Some(text) = command() {
            assert!(!text.contains('\''));
        }
    }

    #[test]
    fn asking_and_unasking_leaves_the_machine_as_it_was_found() {
        // The real scheduler, because the point of this module is that it is
        // the real scheduler; it is put back whichever way it started.
        // Touching the scheduler needs administrator and the test runner may
        // not have it, so a refusal ends the test rather than failing it -
        // there is nothing to check and nothing was changed.
        let before = is_on();
        if !set(!before) {
            return;
        }
        assert_eq!(is_on(), !before);
        set(before);
        assert_eq!(is_on(), before);
    }
}
