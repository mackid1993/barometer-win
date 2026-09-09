// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// PawnIO: whether it is there, whether we may use it, and what to say when the
// answer is no.
//
// Most of a computer's hardware sensors are behind instructions and ports
// that user mode may not touch: the processor's own temperatures live in
// model-specific registers, and the motherboard's fans, voltages and board
// temperatures live behind the SuperIO chip, the embedded controller or the
// SMBus. PawnIO is the signed kernel driver that reads all of those on the
// sensor stack's behalf - LibreHardwareMonitor carries a module for each
// (IntelMSR, AMDFamily17, RyzenSMU, LpcIO, LpcACPIEC, IsaBridgeEC, SmbusI801
// and the rest) and runs them inside it.
//
// So this is not only about the processor, which is what an earlier version
// of every string in this file said. What arrives without PawnIO is what has
// a path of its own: the graphics card, which vendor libraries report, and
// the drives, which answer SMART and NVMe queries directly.
//
// Barometer ships no driver and installs no driver - see AGENTS.md,
// "Antivirus and code signing" - so PawnIO is something a person installs
// themselves, and all this module does is notice what they have and say so
// once.
//
// The important thing here is that this is *two* questions with two different
// remedies, and collapsing them into one "temperatures do not work" is the
// failure this module exists to prevent:
//
//   1. Is PawnIO installed?      A missing driver. Offer the download.
//   2. May this process open it? Installed, but we are not an administrator.
//                                Offering the download to somebody who has
//                                already installed it is worse than silence.
//
// Measured on the author's machine, unelevated, with PawnIO 2.2.0 installed
// and its driver in state RUNNING:
//
//   CreateFileW(r"\\?\GLOBALROOT\Device\PawnIO")  -> ERROR_ACCESS_DENIED (5)
//   CreateFileW(a name that does not exist)       -> ERROR_FILE_NOT_FOUND (2)
//   PawnIOLib!pawnio_open                         -> E_ACCESSDENIED
//
// Denied is not absent, and those two error codes are the whole detector.
// PawnIO's INF gives its device the SDDL `D:P(A;;GA;;;SY)(A;;GA;;;BA)` and
// creates it with FILE_DEVICE_SECURE_OPEN, so the only accounts that may open
// it are SYSTEM and BUILTIN\Administrators. That is why access-denied is read
// as "needs administrator rights" rather than as some unexplained failure: it
// is the documented ACL doing exactly what it was written to do.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    ERROR_SUCCESS, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_YESNO, SW_SHOWNORMAL,
};

use crate::update::Version;

/// PawnIO's own site, which is where its installer is published.
///
/// A compile-time constant, and it has to stay one: this string is handed
/// straight to the shell, and AGENTS.md's rule for the updater - that nothing
/// which arrived over a network ever reaches something that executes - applies
/// with more force to a call that will open whatever it is given.
const DOWNLOAD_PAGE: &str = "https://pawnio.eu/";

/// The device object, by the name LibreHardwareMonitor opens.
///
/// `\\.\PawnIO` also resolves today, through a `\DosDevices\PawnIO` symlink
/// the driver still creates - but that symlink is named `_DEPRECATED` in
/// PawnIO's own source and is liable to go away. The GLOBALROOT form addresses
/// the real object, and it is the one the sensor stack itself uses, so a
/// disagreement between what Barometer probes and what actually reads the
/// hardware is not possible.
const DEVICE_PATH: &str = r"\\?\GLOBALROOT\Device\PawnIO";

/// Where PawnIO's installer records itself.
///
/// This is the key LibreHardwareMonitor reads, and reading the same one is the
/// point rather than a coincidence: LHM is what will or will not get the
/// readings, so Barometer's idea of "installed" should be true exactly when
/// LHM's is. A cleverer check that disagreed with it would be wrong however
/// well it worked.
///
/// No WOW6432Node fallback. LHM needs one because it can run as a 32-bit
/// process and get a redirected view; Barometer is x64-only, so the native
/// view is the only one it can see and the only one the installer writes.
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";

/// Barometer's own settings root.
///
/// The registry is a placeholder. These two flags are the only thing Barometer
/// keeps here, and they belong in the settings file as soon as there is one -
/// which is why every mention of the location is in this one block rather than
/// spread through the module.
const SETTINGS_KEY: &str = r"Software\Barometer";
const ASKED_ABOUT_INSTALL: &str = "PawnIoInstallPrompted";
const ASKED_ABOUT_ELEVATION: &str = "PawnIoElevationPrompted";

/// What was found, in the terms that decide what to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The device opened. Ring-0 readings will work.
    Ready,
    /// Installed and running, but closed to this process. Administrator
    /// rights, not an installation, is what is missing.
    NeedsElevation,
    /// Nothing recorded it as installed, and its device is not there.
    NotInstalled,
    /// The check did not answer. Nothing is claimed and nothing is said - a
    /// probe that failed is not evidence that a driver is missing.
    Unknown,
}

/// Which prompts a person has already seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dismissed {
    pub install: bool,
    pub elevation: bool,
}

impl Dismissed {
    /// What the registry remembers.
    fn stored() -> Dismissed {
        Dismissed { install: flag(ASKED_ABOUT_INSTALL), elevation: flag(ASKED_ABOUT_ELEVATION) }
    }
}

/// What, if anything, to tell the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advice {
    Nothing,
    InstallPawnIo,
    RunAsAdministrator,
}

/// Turns a probe result into a status.
///
/// Split out from the probe itself so the mapping can be tested against the
/// error codes that were actually measured, rather than against a guess about
/// which ones Windows returns.
fn classify(opened: bool, error: u32, installed: bool) -> Status {
    if opened {
        return Status::Ready;
    }
    match error {
        // The ACL refused us. The object exists, so something installed it.
        ERROR_ACCESS_DENIED => Status::NeedsElevation,
        // No such object. Absent only if nothing else says otherwise; with an
        // install on record this is a driver that is not loaded right now - a
        // demand-start driver between boots, or a half-finished install - and
        // neither "go and install it" nor "run as administrator" is true, so
        // the honest answer is that we do not know.
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => {
            if installed {
                Status::Unknown
            } else {
                Status::NotInstalled
            }
        }
        _ => Status::Unknown,
    }
}

/// Decides what to say. No registry, no dialogs, no machine.
///
/// Every rule about when a person gets interrupted lives here, which is what
/// makes those rules testable at all: the effectful half below only carries
/// out whatever this returns.
pub fn advice(sensors_enabled: bool, status: Status, dismissed: Dismissed) -> Advice {
    // Nothing about a temperature driver is worth a dialog to somebody who is
    // not asking for temperatures.
    if !sensors_enabled {
        return Advice::Nothing;
    }
    match status {
        Status::NotInstalled if !dismissed.install => Advice::InstallPawnIo,
        Status::NeedsElevation if !dismissed.elevation => Advice::RunAsAdministrator,
        Status::Ready | Status::Unknown | Status::NotInstalled | Status::NeedsElevation => {
            Advice::Nothing
        }
    }
}

/// Looks at the machine.
pub fn status() -> Status {
    let (opened, error) = probe_device();
    classify(opened, error, installed_version().is_some())
}

/// The version PawnIO's installer recorded, if it recorded one.
///
/// Presence of a parseable version *is* the installed test, which is how LHM
/// decides too: a key with no readable `DisplayVersion` is a leftover rather
/// than an installation. Reusing the updater's version type rather than
/// writing a second one - a three-field dotted number is the same problem
/// whether it came from a release tag or from an uninstall entry, and the
/// fourth field Windows adds is dropped by both.
fn installed_version() -> Option<Version> {
    Version::parse(&string_value(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, "DisplayVersion")?)
}

/// Asks the object manager whether PawnIO's device is there.
///
/// Opened for **no access at all**, and no IOCTL is ever sent. AGENTS.md's
/// rule is that Barometer never talks to this driver, and that stays true: the
/// object manager resolves a name and applies an ACL, and the driver is asked
/// to do nothing. Zero access was checked to still separate access-denied from
/// no-such-device, which is the only reason the probe can be this weak.
fn probe_device() -> (bool, u32) {
    // SAFETY: a NUL-terminated device path, and the handle is closed on the
    // one path that produces one.
    unsafe {
        let handle = CreateFileW(
            wide(DEVICE_PATH).as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return (false, GetLastError());
        }
        CloseHandle(handle);
        (true, 0)
    }
}

/// Says whatever needs saying, at most once per machine per prompt.
///
/// The one call the rest of the program makes. Modal, so it belongs on the
/// path that draws the strip and not in a diagnostic mode where there may be
/// nobody watching a screen.
pub fn prompt_if_needed(sensors_enabled: bool) {
    match advice(sensors_enabled, status(), Dismissed::stored()) {
        Advice::Nothing => {}
        Advice::InstallPawnIo => {
            // Recorded *before* the dialog, not after. A dialog can be left
            // standing for an hour and the process killed underneath it, and
            // "asked once" has to survive that: the promise is that Barometer
            // interrupts once, not that it interrupts until somebody answers.
            remember(ASKED_ABOUT_INSTALL);
            if ask(INSTALL_MESSAGE, MB_YESNO) == IDYES {
                open_download_page();
            }
        }
        Advice::RunAsAdministrator => {
            remember(ASKED_ABOUT_ELEVATION);
            ask(ELEVATION_MESSAGE, MB_OK);
        }
    }
}

const TITLE: &str = "Barometer - hardware sensors";

const INSTALL_MESSAGE: &str = concat!(
    "Barometer cannot read most of this computer's hardware sensors.\n\n",
    "The processor's temperatures, and the motherboard's fan speeds, ",
    "voltages and board temperatures, are read through instructions and ",
    "ports that Windows does not let ordinary programs use. Reaching them ",
    "needs PawnIO - a small kernel driver, written and signed by namazso, ",
    "that exists to do exactly this - and PawnIO is not installed here.\n\n",
    "This is optional. What has a path of its own arrives either way: ",
    "graphics card temperatures and drive temperatures are unaffected, and ",
    "everything else simply shows as unavailable.\n\n",
    "PawnIO is somebody else's software, with its own installer and its own ",
    "license. Barometer contains no kernel driver, installs none, and will ",
    "not install this one for you. If you want it, you install it yourself.\n\n",
    "Open the PawnIO download page?\n\n",
    "Barometer asks this once."
);

/// Rare, and deliberately still here.
///
/// Barometer's manifest requires administrator, so in an ordinary session this
/// message cannot appear: the process either starts elevated or does not start.
/// The *classification* behind it is what earns its place - a device that
/// answered "denied" must never be reported as a device that is not there -
/// and the message is what happens if that assumption ever stops holding.
/// Saying nothing in that case would leave somebody with blank temperatures
/// and no explanation, which is the failure this whole module is against.
const ELEVATION_MESSAGE: &str = concat!(
    "Barometer cannot read most of this computer's hardware sensors.\n\n",
    "PawnIO is installed here and its driver is running, so there is nothing ",
    "to install. Windows refused Barometer access to it: PawnIO admits only ",
    "the system account and administrators, and this copy of Barometer is not ",
    "running as either.\n\n",
    "Barometer asks Windows for administrator rights when it starts, so this ",
    "is not supposed to happen. Closing it and starting it again from its ",
    "Start menu shortcut, allowing the prompt Windows shows, is the thing to ",
    "try.\n\n",
    "What has a path of its own - graphics card and drive temperatures - ",
    "works exactly as it does now.\n\n",
    "Barometer says this once."
);

/// Puts the message on the screen and returns which button was pressed.
fn ask(message: &str, buttons: u32) -> i32 {
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(message).as_ptr(),
            wide(TITLE).as_ptr(),
            buttons | MB_ICONINFORMATION | MB_SETFOREGROUND,
        )
    }
}

/// Opens PawnIO's page in whatever the user's browser is.
fn open_download_page() {
    // SAFETY: NUL-terminated constants; the returned pseudo-handle is not a
    // resource and is deliberately dropped.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(DOWNLOAD_PAGE).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL as i32,
        );
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// An open key that closes itself.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: only built from a successful open or create.
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(root: HKEY, path: &str, access: u32) -> Option<Key> {
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: a NUL-terminated path; key is written on success.
    let status = unsafe { RegOpenKeyExW(root, wide(path).as_ptr(), 0, access, &mut key) };
    (status == ERROR_SUCCESS).then_some(Key(key))
}

/// Reads one string value.
fn string_value(root: HKEY, path: &str, name: &str) -> Option<String> {
    use std::os::windows::ffi::OsStringExt;

    let key = open(root, path, KEY_READ)?;
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let mut kind = 0u32;
    // SAFETY: buffer and size describe the same array, and size is updated to
    // the number of bytes written.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            wide(name).as_ptr(),
            std::ptr::null(),
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || kind != REG_SZ {
        return None;
    }
    let units = (size as usize / std::mem::size_of::<u16>()).min(buffer.len());
    let text = std::ffi::OsString::from_wide(&buffer[..units]).to_string_lossy().into_owned();
    Some(text.trim_end_matches('\0').to_string())
}

/// Whether a prompt has already been shown.
///
/// A registry that cannot be read reads as "not yet shown", which risks one
/// repeated dialog rather than silently swallowing the only warning a person
/// gets about why a number is missing.
fn flag(name: &str) -> bool {
    let Some(key) = open(HKEY_CURRENT_USER, SETTINGS_KEY, KEY_READ) else {
        return false;
    };
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let mut kind = 0u32;
    // SAFETY: a four byte destination described accurately to the call.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            wide(name).as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::addr_of_mut!(value).cast(),
            &mut size,
        )
    };
    status == ERROR_SUCCESS && kind == REG_DWORD && value != 0
}

/// Records that a prompt has been shown.
fn remember(name: &str) {
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: a NUL-terminated subkey under HKCU; key is written on success.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(SETTINGS_KEY).as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return;
    }
    let key = Key(key);
    let value: u32 = 1;
    // SAFETY: a four byte DWORD described accurately to the call.
    unsafe {
        RegSetValueExW(
            key.0,
            wide(name).as_ptr(),
            0,
            REG_DWORD,
            std::ptr::addr_of!(value).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // The codes measured on a machine with PawnIO 2.2.0 installed and its
    // driver running, probed from an unelevated process.
    const DENIED: u32 = 5;
    const NOT_FOUND: u32 = 2;

    #[test]
    fn a_denied_device_means_administrator_rights_not_a_missing_driver() {
        // The whole point of the module. This is what the author's own machine
        // returns, and reading it as "not installed" would tell somebody who
        // has PawnIO to go and install PawnIO.
        assert_eq!(classify(false, DENIED, true), Status::NeedsElevation);
        // True even with no uninstall entry to corroborate it: the device
        // answered, so something put it there.
        assert_eq!(classify(false, DENIED, false), Status::NeedsElevation);
    }

    #[test]
    fn no_device_and_no_installation_on_record_is_the_only_absent_case() {
        assert_eq!(classify(false, NOT_FOUND, false), Status::NotInstalled);
        assert_eq!(classify(false, ERROR_PATH_NOT_FOUND, false), Status::NotInstalled);
    }

    #[test]
    fn an_installation_whose_driver_is_not_loaded_is_not_reported_as_absent() {
        // A demand-start driver that nothing has started yet. Offering the
        // download here would be wrong; so would demanding elevation.
        assert_eq!(classify(false, NOT_FOUND, true), Status::Unknown);
    }

    #[test]
    fn a_check_that_could_not_answer_never_claims_the_driver_is_missing() {
        for error in [1u32, 87, 1231, u32::MAX] {
            assert_eq!(classify(false, error, false), Status::Unknown, "error {error}");
            assert_eq!(classify(false, error, true), Status::Unknown, "error {error}");
        }
        assert_eq!(advice(true, Status::Unknown, Dismissed::default()), Advice::Nothing);
    }

    #[test]
    fn an_open_device_needs_nothing_said_about_it() {
        assert_eq!(classify(true, 0, true), Status::Ready);
        assert_eq!(advice(true, Status::Ready, Dismissed::default()), Advice::Nothing);
    }

    #[test]
    fn a_missing_driver_offers_the_download_once() {
        let fresh = Dismissed::default();
        assert_eq!(advice(true, Status::NotInstalled, fresh), Advice::InstallPawnIo);
        let asked = Dismissed { install: true, elevation: false };
        assert_eq!(advice(true, Status::NotInstalled, asked), Advice::Nothing);
    }

    #[test]
    fn an_installed_driver_we_cannot_open_asks_for_administrator_rights_once() {
        let fresh = Dismissed::default();
        assert_eq!(advice(true, Status::NeedsElevation, fresh), Advice::RunAsAdministrator);
        let asked = Dismissed { install: false, elevation: true };
        assert_eq!(advice(true, Status::NeedsElevation, asked), Advice::Nothing);
    }

    #[test]
    fn dismissing_one_prompt_does_not_dismiss_the_other() {
        // They are different facts with different remedies, and a person who
        // waved away one of them has not been told the other.
        let install_only = Dismissed { install: true, elevation: false };
        assert_eq!(advice(true, Status::NeedsElevation, install_only), Advice::RunAsAdministrator);
        let elevation_only = Dismissed { install: false, elevation: true };
        assert_eq!(advice(true, Status::NotInstalled, elevation_only), Advice::InstallPawnIo);
    }

    #[test]
    fn nobody_is_interrupted_about_sensors_they_have_switched_off() {
        for status in [Status::NotInstalled, Status::NeedsElevation, Status::Ready, Status::Unknown]
        {
            assert_eq!(advice(false, status, Dismissed::default()), Advice::Nothing, "{status:?}");
        }
    }

    #[test]
    fn the_version_windows_records_for_pawnio_is_one_we_can_read() {
        // "2.2.0.0" is what the author's uninstall entry actually holds. The
        // fourth field is Windows' own padding and is not part of the version.
        assert_eq!(Version::parse("2.2.0.0"), Some(Version::new(2, 2, 0)));
        // An entry with nothing readable in it is a leftover, not an install.
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("unknown"), None);
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    /// Reports what the probe sees on the machine running the tests.
    ///
    /// Ignored by default and asserts nothing: the answer depends on whether
    /// PawnIO happens to be installed there, and a test that fails on a
    /// developer's laptop for being a different laptop is noise. Run it with
    /// `cargo test -p barometer -- --ignored --nocapture` when the detection
    /// itself is what is in question, which is the only time it is useful.
    #[test]
    #[ignore]
    fn what_this_machine_looks_like() {
        println!("PawnIO status: {:?}", status());
        println!("advice with sensors on: {:?}", advice(true, status(), Dismissed::default()));
    }
}
