// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Moving the settings in and out by hand.
//
// Export writes the same document the store keeps, wherever the user points;
// import reads one back through the same parser. So a file that survives an
// import is a file the app would have loaded at startup, and a file exported
// today is one a later version still reads, because the store already
// promises that of its own file.

use std::path::PathBuf;

use barometer_core::store::{Settings, Store};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

/// The name the save dialog proposes. Named for the app rather than
/// `settings.json`, because it is going to sit in a folder full of other
/// programs' files.
const SUGGESTED_NAME: &str = "barometer-settings.json";

/// Writes the settings to a file the user picks.
///
/// Returns the line the pane shows afterwards, or None when the dialog was
/// canceled - a cancel is not news, and a caption saying "canceled" would
/// outlive the moment it described.
pub fn export(owner: HWND, settings: &Settings) -> Option<String> {
    let path = pick(owner, Direction::Save)?;
    // A store at the chosen path, never the app's own: `save` merges over
    // whatever the store last read, and this one has read nothing, so the
    // file is written whole.
    Some(match Store::at(&path).save(settings) {
        Ok(()) => format!("Exported to {}.", path.display()),
        Err(why) => format!("Couldn't write {}: {why}", path.display()),
    })
}

/// Reads settings from a file the user picks.
///
/// Through `Store::parse` rather than `Store::load`: `load` moves a file it
/// cannot read out of the way, which is right for the file it owns and
/// unforgivable for one the user merely pointed at. None when canceled.
pub fn import(owner: HWND) -> Option<Result<Settings, String>> {
    let path = pick(owner, Direction::Open)?;
    Some(match std::fs::read(&path) {
        Ok(bytes) => Store::parse(&bytes)
            .map_err(|why| format!("{} {why}.", path.display())),
        Err(why) => Err(format!("Couldn't read {}: {why}.", path.display())),
    })
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Direction {
    Open,
    Save,
}

/// The common file dialog, as a path or nothing.
///
/// The classic `comdlg32` dialog rather than `IFileDialog`: it is one call
/// with a struct, needs no COM, and on Windows 11 it opens the same modern
/// picker the COM interface does.
fn pick(owner: HWND, direction: Direction) -> Option<PathBuf> {
    // Pairs of display name and pattern, each NUL-terminated, the whole list
    // terminated by one more NUL. The struct wants exactly this shape.
    let filter: Vec<u16> = "Barometer settings (*.json)\0*.json\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title: Vec<u16> = match direction {
        Direction::Open => "Import settings\0",
        Direction::Save => "Export settings\0",
    }
    .encode_utf16()
    .collect();
    let extension: Vec<u16> = "json\0".encode_utf16().collect();

    // The buffer is both the suggested name in and the chosen path out.
    let mut file = vec![0u16; 32 * 1024];
    if direction == Direction::Save {
        for (slot, unit) in file.iter_mut().zip(SUGGESTED_NAME.encode_utf16()) {
            *slot = unit;
        }
    }

    // SAFETY: every pointer refers to a buffer that outlives the call, and
    // the struct is zeroed before the fields Windows reads are set.
    unsafe {
        let mut request: OPENFILENAMEW = std::mem::zeroed();
        request.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        request.hwndOwner = owner;
        request.lpstrFilter = filter.as_ptr();
        request.nFilterIndex = 1;
        request.lpstrFile = file.as_mut_ptr();
        request.nMaxFile = file.len() as u32;
        request.lpstrTitle = title.as_ptr();
        request.lpstrDefExt = extension.as_ptr();
        // NOCHANGEDIR because the dialog otherwise moves the process's
        // working directory to wherever the user browsed, and nothing else
        // in the app expects that to change under it.
        request.Flags = OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | match direction {
                Direction::Open => OFN_FILEMUSTEXIST,
                Direction::Save => OFN_OVERWRITEPROMPT,
            };
        let chosen = match direction {
            Direction::Open => GetOpenFileNameW(&mut request),
            Direction::Save => GetSaveFileNameW(&mut request),
        };
        if chosen == 0 {
            return None;
        }
    }

    let length = file.iter().position(|unit| *unit == 0).unwrap_or(0);
    if length == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&file[..length])))
}
