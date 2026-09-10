// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The mounted volumes, for the disks panel: each drive letter with its
// label, its size and what is left, and whether it can be pulled out.

use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

/// Drive types, from winbase.h; the crate's feature set does not name them.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

/// One mounted volume.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    /// The label, "Windows" or "Data"; empty when the volume has none.
    pub name: String,
    /// The drive letter with its colon, "C:".
    pub mount: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
    pub remote: bool,
}

impl Volume {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

/// Every volume with a drive letter that answers for its size.
///
/// A drive letter that is present but has nothing in it - an empty card
/// reader, a DVD drive with no disc - fails the free-space call and is left
/// out, which is what the user sees in Explorer too. Network drives are
/// kept and flagged: their size is real and they are slow to ask, so the
/// caller decides how often.
pub fn volumes() -> Vec<Volume> {
    let mut found = Vec::new();
    // SAFETY: root paths are terminated; every out-pointer is to a local.
    unsafe {
        let mask = GetLogicalDrives();
        for letter in b'A'..=b'Z' {
            if mask & (1 << (letter - b'A')) == 0 {
                continue;
            }
            let root: Vec<u16> = [letter as u16, b':' as u16, b'\\' as u16, 0].to_vec();
            let kind = GetDriveTypeW(root.as_ptr());
            if !matches!(kind, DRIVE_REMOVABLE | DRIVE_FIXED | DRIVE_REMOTE | DRIVE_CDROM | DRIVE_RAMDISK) {
                continue;
            }
            let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
            if GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, &mut free) == 0 || total == 0 {
                continue;
            }
            let mut label = [0u16; 261];
            let mut serial = 0u32;
            let mut component = 0u32;
            let mut flags = 0u32;
            let mut filesystem = [0u16; 64];
            let named = GetVolumeInformationW(
                root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                &mut serial,
                &mut component,
                &mut flags,
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            ) != 0;
            let name = if named {
                let end = label.iter().position(|c| *c == 0).unwrap_or(label.len());
                String::from_utf16_lossy(&label[..end])
            } else {
                String::new()
            };
            found.push(Volume {
                name,
                mount: format!("{}:", letter as char),
                total,
                available,
                removable: matches!(kind, DRIVE_REMOVABLE | DRIVE_CDROM),
                remote: kind == DRIVE_REMOTE,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_drive_is_found_with_a_size_and_a_mount() {
        let found = volumes();
        let system = found.iter().find(|v| v.mount.eq_ignore_ascii_case("C:")).expect("C: exists");
        assert!(system.total > 0);
        assert!(system.available <= system.total);
        assert!(!system.removable);
    }

    #[test]
    fn used_never_underflows() {
        let volume = Volume { total: 10, available: 12, ..Default::default() };
        assert_eq!(volume.used(), 0);
    }
}
