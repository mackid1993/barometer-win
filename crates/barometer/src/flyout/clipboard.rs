// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The clipboard, for the network flyout's address rows: the Mac's
// CopyableNetworkValue puts the address on the pasteboard when its row is
// clicked, and this is the whole of what that needs on Windows.

use std::cell::Cell;

use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, HWND_MESSAGE, WS_OVERLAPPED};

/// CF_UNICODETEXT, which windows-sys files under OLE with the rest of the
/// clipboard formats; naming the one value here costs less than the feature.
const CF_UNICODETEXT: u32 = 13;

thread_local! {
    /// The window that owns what this program puts on the clipboard.
    ///
    /// `OpenClipboard(NULL)` succeeds, but `EmptyClipboard` then sets the
    /// clipboard owner to NULL and, as Microsoft documents, that makes the
    /// following `SetClipboardData` fail - so clicking an address row emptied
    /// whatever the user had on the clipboard and put nothing in its place.
    /// A message-only window is the smallest thing that can own it: never
    /// shown, never painted, no message loop of its own, and one per thread
    /// because the clipboard is opened by whichever thread is handling the
    /// click.
    static OWNER: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
}

fn owner() -> HWND {
    OWNER.with(|slot| {
        let existing = slot.get();
        if !existing.is_null() {
            return existing;
        }
        let class: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a predefined class name, terminated, and a message-only
        // parent; every other argument is null or zero.
        let created = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        slot.set(created);
        created
    })
}

/// Puts text on the clipboard. False when the clipboard could not be taken,
/// which happens when another program has it open at that instant.
pub fn copy_text(text: &str) -> bool {
    let holder = owner();
    if holder.is_null() {
        // Without an owner the clipboard would be emptied and not refilled,
        // which is worse than not copying at all.
        return false;
    }
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units.len() * 2;
    // SAFETY: the global block is allocated, filled while locked, and
    // either handed to the clipboard, which then owns it, or freed here.
    unsafe {
        let block = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if block.is_null() {
            return false;
        }
        let memory = GlobalLock(block);
        if memory.is_null() {
            GlobalFree(block);
            return false;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr() as *const u8, memory as *mut u8, bytes);
        GlobalUnlock(block);
        if OpenClipboard(holder) == 0 {
            GlobalFree(block);
            return false;
        }
        EmptyClipboard();
        let placed = SetClipboardData(CF_UNICODETEXT, block as _);
        CloseClipboard();
        if placed.is_null() {
            GlobalFree(block);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reaches_the_clipboard_and_not_only_the_emptying_of_it() {
        // The failure this covers was silent: the clipboard was emptied and
        // then refused the new text, so whatever the user had was destroyed
        // and nothing replaced it.
        assert!(!owner().is_null(), "no window to own the clipboard");
        assert!(copy_text("192.168.1.20"));
        // Twice, because the owner window is made once and reused.
        assert!(copy_text("10.0.0.5"));
    }
}
