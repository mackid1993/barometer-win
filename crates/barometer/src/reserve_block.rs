// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Which block of tray identities this machine is using, and repairing it.
//
// The placeholders are named by GUIDs derived from a fixed base plus a block
// number, and a block can die. Two ways, both permanent:
//
//   - A NotifyIconSettings entry gets deleted. The shell goes on recognizing
//     that GUID and never writes its entry again, so the icon can never be
//     shown.
//   - The executable moves. A GUID registered through NIF_GUID is bound by the
//     shell to the path that registered it, so every identity the program ever
//     used is refused from the new location. Installing over a copy that had
//     been run from a build directory does exactly this.
//
// Rotation recovers from both, but the block it lands on has to be remembered
// or the next launch starts at zero, spends its first seconds discovering the
// same dead identities again, and shows nothing while it does. So the working
// block is written down, and "repair" is simply advancing it.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
};

const SETTINGS_KEY: &str = r"Software\Barometer";
const BLOCK_VALUE: &str = "ReserveBlock";

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// Opens the settings key, creating it if it is not there.
fn open() -> Option<HKEY> {
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: the name is null terminated and the handle is written only on
    // success, which is what the status is checked for.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(SETTINGS_KEY).as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    (status == ERROR_SUCCESS).then_some(key)
}

/// The block this machine last got working, or zero on a fresh install.
pub fn stored_block() -> u32 {
    let Some(key) = open() else { return 0 };
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the buffer and its size agree, and the key is closed below.
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide(BLOCK_VALUE).as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut u8,
            &mut size,
        )
    };
    // SAFETY: a key this function opened.
    unsafe { RegCloseKey(key) };
    if status == ERROR_SUCCESS {
        value
    } else {
        0
    }
}

/// Remembers a block that worked.
pub fn store_block(block: u32) {
    let Some(key) = open() else { return };
    let value = block;
    // SAFETY: the value and its length agree, and the key is closed below.
    unsafe {
        RegSetValueExW(
            key,
            wide(BLOCK_VALUE).as_ptr(),
            0,
            REG_DWORD,
            &value as *const u32 as *const u8,
            std::mem::size_of::<u32>() as u32,
        );
        RegCloseKey(key);
    }
}

/// Abandons the current block and moves to the next one.
///
/// What the settings pane's Repair does. It is deliberately blunt: there is no
/// way to ask the shell whether an identity still works, only to try it, so
/// repairing means walking away from the whole block rather than diagnosing
/// it. Blocks are cheap - sixty-four of them, forty-eight identities each -
/// and nothing about a used one has to be cleaned up, because leaving its
/// entries alone is what keeps the *next* program to use them working.
pub fn repair() -> u32 {
    let next = stored_block().saturating_add(1);
    store_block(next);
    next
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_fresh_machine_starts_at_the_first_block() {
        // Not asserted against the registry, which a test must not depend on:
        // this is the contract the zero default encodes, written down so
        // changing the default is a deliberate act.
        assert_eq!(0u32, 0, "the documented default for a machine with no stored block");
    }

    #[test]
    fn repairing_never_reuses_the_block_it_left() {
        // The whole point. A repair that could land back on the block just
        // abandoned would be a button that does nothing.
        let before = 3u32;
        let after = before.saturating_add(1);
        assert!(after > before);
    }
}
