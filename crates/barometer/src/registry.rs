// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The notification-area settings, which is where a tray icon is told to be
// always-visible.
//
// A newly registered tray icon hides inside the "Show Hidden Icons" flyout and
// occupies no width at all. It reserves nothing until it is marked promoted
// here, which is the whole reason this module exists. It is the same value
// behind Settings > Personalization > Taskbar > Other system tray icons, so it
// is a documented user setting rather than a private hack - but it does mean
// this feature writes to the registry, and there is no way around that.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_SZ,
};

/// Where the shell keeps per-icon notification-area settings.
pub const NOTIFY_ICON_SETTINGS: &str = r"Control Panel\NotifyIconSettings";

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// An open key that closes itself.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: only built from a successful open.
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(path: &str, access: u32) -> Option<Key> {
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: a NUL-terminated path; key is written on success.
    let status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(path).as_ptr(), 0, access, &mut key) };
    (status == ERROR_SUCCESS).then_some(Key(key))
}

/// Every subkey name under the notification-area settings.
///
/// The names are opaque and assigned by the shell, which is why they have to
/// be enumerated rather than derived: the only way to find the entry belonging
/// to an icon is to look at each one's `IconGuid`.
pub fn notify_icon_keys() -> Vec<String> {
    let Some(key) = open(NOTIFY_ICON_SETTINGS, KEY_READ) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buffer = [0u16; 256];
        let mut length = buffer.len() as u32;
        // SAFETY: buffer and length describe the same array; the call writes
        // at most `length` units and updates it to what it wrote.
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                buffer.as_mut_ptr(),
                &mut length,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        names.push(OsString::from_wide(&buffer[..length as usize]).to_string_lossy().into_owned());
        index += 1;
    }
    names
}

/// Reads one entry's `IconGuid`, if it has one.
///
/// This is the ownership check, and it is not optional. "A key that appeared
/// since the snapshot" is not proof the key is ours: the shell writes these
/// asynchronously, so another program registering a tray icon at that moment
/// lands in the same diff. Acting on it would force a stranger's icon
/// always-visible and record its key as ours to delete later, which is
/// vandalizing another application's settings.
pub fn icon_guid_of(entry: &str) -> Option<String> {
    let path = format!(r"{NOTIFY_ICON_SETTINGS}\{entry}");
    let key = open(&path, KEY_READ)?;

    let mut buffer = [0u16; 80];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
    let mut value_type = 0u32;
    // SAFETY: buffer and size describe the same array; size is updated to the
    // bytes written.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            wide("IconGuid").as_ptr(),
            std::ptr::null(),
            &mut value_type,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || value_type != REG_SZ {
        return None;
    }

    let units = (size as usize / std::mem::size_of::<u16>()).min(buffer.len());
    let text = OsString::from_wide(&buffer[..units]).to_string_lossy().into_owned();
    Some(text.trim_end_matches('\0').to_string())
}

/// Marks one entry always-visible.
pub fn set_promoted(entry: &str) -> bool {
    let path = format!(r"{NOTIFY_ICON_SETTINGS}\{entry}");
    let Some(key) = open(&path, KEY_SET_VALUE) else {
        return false;
    };
    let value: u32 = 1;
    // SAFETY: a four byte DWORD described accurately to the call.
    let status = unsafe {
        RegSetValueExW(
            key.0,
            wide("IsPromoted").as_ptr(),
            0,
            REG_DWORD,
            std::ptr::addr_of!(value).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    status == ERROR_SUCCESS
}

/// Finds the entry belonging to a placeholder, by its GUID.
///
/// Matched on the GUID rather than on a set difference, so it works however
/// late the shell gets round to writing the key - which matters, because right
/// after the icon is added the key usually does not exist yet.
pub fn find_entry_for_guid(guid: &str) -> Option<String> {
    notify_icon_keys()
        .into_iter()
        .find(|entry| icon_guid_of(entry).is_some_and(|found| found.eq_ignore_ascii_case(guid)))
}

/// Reads a string value from one entry.
fn string_value(entry: &str, value_name: &str) -> Option<String> {
    let path = format!(r"{NOTIFY_ICON_SETTINGS}\{entry}");
    let key = open(&path, KEY_READ)?;

    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
    let mut value_type = 0u32;
    // SAFETY: buffer and size describe the same array.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            wide(value_name).as_ptr(),
            std::ptr::null(),
            &mut value_type,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || value_type != REG_SZ {
        return None;
    }
    let units = (size as usize / std::mem::size_of::<u16>()).min(buffer.len());
    let text = OsString::from_wide(&buffer[..units]).to_string_lossy().into_owned();
    Some(text.trim_end_matches('\0').to_string())
}

/// The tooltip the shell recorded when an icon was first registered.
///
/// It survives the icon being removed - removing only blanks a key's live
/// values and leaves the entry behind - which is why leftovers were once
/// matched on it. `IconGuid` is what matches them now, so nothing reads this.
pub fn initial_tooltip_of(entry: &str) -> Option<String> {
    string_value(entry, "InitialTooltip")
}

/// Deletes one entry outright.
///
/// Only correct for orphans from a previous run, and never on an ordinary
/// exit. Deleting an entry the shell still remembers in memory is what kills a
/// batch of identities permanently: it keeps recognizing them and never writes
/// their entries again, so they can never be shown.
pub fn delete_entry(entry: &str) -> bool {
    let Some(root) = open(NOTIFY_ICON_SETTINGS, KEY_READ | KEY_SET_VALUE) else {
        return false;
    };
    // SAFETY: a NUL-terminated subkey name under an open key.
    let status = unsafe { RegDeleteKeyW(root.0, wide(entry).as_ptr()) };
    status == ERROR_SUCCESS
}

