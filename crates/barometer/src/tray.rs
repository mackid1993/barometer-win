// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The placeholder tray icons, and the window that owns them.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateIconIndirect, CreateWindowExW, DefWindowProcW,
    RegisterClassExW, RegisterWindowMessageW, HICON, ICONINFO, MSGFLT_ALLOW, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

/// Window class for the owner window.
const OWNER_CLASS: &str = "BarometerTrayReserve";

/// The tooltip every placeholder carries.
///
/// The shell records it as `InitialTooltip` and shows it wherever it names a
/// tray icon. Leftovers from a previous run are found by `IconGuid` rather
/// than by this: a GUID is an identity this program minted and can prove, a
/// tooltip is only a string anyone may write.
pub const RESERVE_TIP: &str = "Barometer reserved";

pub fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// A fully transparent 32x32 icon.
///
/// The AND mask has to be filled with ones, meaning fully transparent. Passing
/// a null mask leaves it uninitialized, and wherever its bits land on zero the
/// shell draws solid black - which shows up as a row of black squares sitting
/// in the notification area.
pub fn blank_icon() -> HICON {
    const SIZE: i32 = 32;

    // SAFETY: a screen DC, released on every path out.
    let screen = unsafe { GetDC(std::ptr::null_mut()) };
    if screen.is_null() {
        return std::ptr::null_mut();
    }

    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = SIZE;
    // Negative height for a top-down bitmap, so the pixels sit in the order
    // everything else assumes.
    info.bmiHeader.biHeight = -SIZE;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB as u32;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: info describes the section; bits receives the pixel pointer.
    let color: HBITMAP = unsafe {
        CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0)
    };

    let mut icon: HICON = std::ptr::null_mut();
    if !color.is_null() {
        if !bits.is_null() {
            // Alpha zero everywhere: nothing to see, but a full slot wide.
            // SAFETY: the section is SIZE * SIZE pixels of four bytes.
            unsafe { std::ptr::write_bytes(bits.cast::<u8>(), 0, (SIZE * SIZE * 4) as usize) };
        }

        // Ones, not zeroes, and never null. See the note above.
        let mask_bits = vec![0xFFu8; (SIZE * SIZE / 8) as usize];
        // SAFETY: a 1bpp monochrome bitmap of exactly that many bytes.
        let mask: HBITMAP = unsafe { CreateBitmap(SIZE, SIZE, 1, 1, mask_bits.as_ptr().cast()) };

        if !mask.is_null() {
            let mut icon_info: ICONINFO = unsafe { std::mem::zeroed() };
            icon_info.fIcon = TRUE;
            icon_info.hbmMask = mask;
            icon_info.hbmColor = color;
            // SAFETY: both bitmaps are live and CreateIconIndirect copies them.
            icon = unsafe { CreateIconIndirect(&icon_info) };
            // SAFETY: the copies were taken, so these are ours to release.
            unsafe { DeleteObject(mask as _) };
        }
        // SAFETY: as above.
        unsafe { DeleteObject(color as _) };
    }

    // SAFETY: the DC came from GetDC with a null window.
    unsafe { ReleaseDC(std::ptr::null_mut(), screen) };
    icon
}

/// Set when Explorer announces it has restarted. Read and cleared by the tick.
static SHELL_RESTARTED: AtomicBool = AtomicBool::new(false);

/// The id of the `TaskbarCreated` broadcast, resolved once.
///
/// It is a registered message, so its number is assigned at run time and is the
/// same for every process in the session. Zero means the registration failed,
/// which no real message id ever is, so comparing against it is safe.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// True if Explorer restarted since this was last asked, clearing the flag.
pub fn take_shell_restarted() -> bool {
    SHELL_RESTARTED.swap(false, Ordering::AcqRel)
}

unsafe extern "system" fn owner_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let created = TASKBAR_CREATED.load(Ordering::Acquire);
    if created != 0 && message == created {
        // Set a flag and nothing else. This message arrives from inside the
        // shell's own re-initialization; calling back into it here - adding an
        // icon, asking for a rect - lands in a shell that is half built and
        // either fails silently or wedges. The rebuild happens on the next
        // tick, when the shell has finished and is answering normally.
        SHELL_RESTARTED.store(true, Ordering::Release);
        return 0;
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

/// Creates the window that owns the placeholders.
///
/// A real, never-shown, top-level window. It must **not** be message-only: the
/// shell rejects `Shell_NotifyIcon` from an `HWND_MESSAGE` owner and every add
/// fails, with nothing to say why. `WS_EX_TOOLWINDOW` keeps it out of the
/// taskbar and Alt-Tab; `WS_EX_NOACTIVATE` keeps it from ever taking focus.
pub fn create_owner_window() -> HWND {
    let class = wide(OWNER_CLASS);
    // SAFETY: a zeroed class filled in before registration. Registering twice
    // is harmless; the second call fails and the first registration stands.
    unsafe {
        let mut info: WNDCLASSEXW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        info.lpfnWndProc = Some(owner_proc);
        info.lpszClassName = class.as_ptr();
        RegisterClassExW(&info);

        let created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        TASKBAR_CREATED.store(created, Ordering::Release);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            wide("").as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );

        // Broadcasts do not cross an integrity boundary on their own. Run
        // elevated - which people do, and which the installer must not require
        // but cannot prevent - and Explorer's `TaskbarCreated` is dropped by
        // UIPI before it is ever seen, so the reservation is never rebuilt and
        // the readout is gone until the app is restarted. Letting exactly this
        // one message through costs nothing: it carries no payload.
        if !hwnd.is_null() && created != 0 {
            ChangeWindowMessageFilterEx(hwnd, created, MSGFLT_ALLOW, std::ptr::null_mut());
        }
        hwnd
    }
}

use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_GUID, NIF_ICON, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};

use crate::registry;
use crate::spacer::Guid;

fn icon_data(owner: HWND, guid: Guid, with_icon: Option<HICON>) -> NOTIFYICONDATAW {
    // SAFETY: plain data, filled in immediately below.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = owner;
    // SAFETY: our Guid is laid out exactly as Windows' GUID, which is the
    // whole reason it is declared repr(C) with those four fields.
    data.guidItem = unsafe { std::mem::transmute(guid) };
    match with_icon {
        Some(icon) => {
            // A tooltip, deliberately, and it is the one thing the
            // obstruction query cannot work without. UI Automation
            // exposes a tray icon's tip as its button Name, and that name
            // is how the sweep separates our placeholders from a real
            // icon someone dragged in among them. Without it every
            // placeholder reads as foreign.
            //
            // This was removed once, because a registered tip is one the
            // user can hover into through the readout, and the shell
            // remembers the first tip it is given as InitialTooltip so
            // clearing it later does not undo that. Identifying the
            // placeholders by rectangle instead was tried and is what
            // made the readout vanish outright: the shell's rectangles
            // and UI Automation's disagree exactly when the tray is
            // re-laying out, so every placeholder looked foreign at the
            // moment it mattered. The tip stays.
            let tip = wide(RESERVE_TIP);
            let count = tip.len().min(data.szTip.len());
            data.szTip[..count].copy_from_slice(&tip[..count]);
            data.uFlags = NIF_ICON | NIF_GUID | NIF_TIP;
            data.hIcon = icon;
        }
        None => data.uFlags = NIF_GUID,
    }
    data
}

/// Registers one placeholder.
///
/// A GUID can still be held by an instance that did not exit cleanly, and the
/// first add is then refused. Dropping it and adding again recovers, rather
/// than leaving that slot unusable for the life of the session.
pub fn add_icon(owner: HWND, icon: HICON, guid: Guid) -> bool {
    let data = icon_data(owner, guid, Some(icon));
    // SAFETY: data is fully described by its cbSize and uFlags.
    let added = unsafe {
        if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
            true
        } else {
            Shell_NotifyIconW(NIM_DELETE, &data);
            Shell_NotifyIconW(NIM_ADD, &data) != 0
        }
    };
    added
}

/// Removes one placeholder.
pub fn remove_icon(owner: HWND, guid: Guid) -> bool {
    let data = icon_data(owner, guid, None);
    // SAFETY: as above.
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) != 0 }
}

/// Marks a placeholder always-visible, if the shell has written its entry yet.
///
/// Returns false when the entry does not exist, which is the ordinary case
/// immediately after an add: the shell writes it asynchronously, and until it
/// does the icon sits in the overflow flyout reserving nothing. The caller
/// keeps such slots on a retry list rather than treating this as a failure.
pub fn promote(guid: Guid) -> bool {
    let wanted = guid.to_registry_string();
    match registry::find_entry_for_guid(&wanted) {
        Some(entry) => registry::set_promoted(&entry),
        None => false,
    }
}

/// Deletes notification-area entries left behind by a previous run.
///
/// Entries are never deleted on an ordinary exit - that is what kills a batch
/// of identities permanently, because the shell goes on recognizing them and
/// never writes their entries again - so every run leaves its own behind and
/// they would otherwise accumulate in the user's settings forever. Deleting
/// them at the *start* of a later run is the one point at which it is safe,
/// because the shell no longer holds them.
///
/// Ours are recognized by the `IconGuid` the shell recorded against each
/// entry, which is an identity this program minted and can prove.
pub fn purge_orphans() -> usize {
    registry::notify_icon_keys()
        .into_iter()
        .filter(|entry| {
            registry::icon_guid_of(entry).is_some_and(|guid| crate::spacer::is_ours(&guid))
        })
        .filter(|entry| registry::delete_entry(entry))
        .count()
}

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};

/// The notification area's rectangle, on screen.
///
/// Only needed for the fallback placement: past the hold deadline the reserved
/// region is never coming, and the readout is put to the left of the tray the
/// way it was before any of this existed. It is measured from
/// `TrayNotifyWnd` inside `Shell_TrayWnd`.
pub fn notify_rect() -> Option<crate::spacer::Rect> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW};

    // SAFETY: class-name lookups; the rect is written only on success.
    unsafe {
        let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
        if taskbar.is_null() {
            return None;
        }
        let notify = FindWindowExW(
            taskbar,
            std::ptr::null_mut(),
            wide("TrayNotifyWnd").as_ptr(),
            std::ptr::null(),
        );
        if notify.is_null() {
            return None;
        }
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(notify, &mut rect) == 0 {
            return None;
        }
        let found = crate::spacer::Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        (!found.is_empty()).then_some(found)
    }
}

/// Where one placeholder actually is, on screen.
///
/// `Shell_NotifyIconGetRect` answers this directly for an icon identified by
/// GUID, which is markedly simpler than asking UI Automation: no COM, no
/// apartment, no background thread, and no cross-process round trip per
/// element. The original used UI Automation because it also has to see *other*
/// programs' icons, to know when one has been dragged into the reserved
/// region - and this call cannot do that, since it only answers for icons we
/// can name. So this covers the common path and UI Automation stays needed for
/// obstruction detection.
pub fn icon_rect(owner: HWND, guid: Guid) -> Option<crate::spacer::Rect> {
    // SAFETY: plain data; cbSize is set as the API requires.
    let mut id: NOTIFYICONIDENTIFIER = unsafe { std::mem::zeroed() };
    id.cbSize = std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32;
    id.hWnd = owner;
    // SAFETY: our Guid is laid out exactly as Windows' GUID.
    id.guidItem = unsafe { std::mem::transmute(guid) };

    let mut rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: id is fully described; rect is written only on success.
    let result = unsafe { Shell_NotifyIconGetRect(&id, &mut rect) };
    if result != 0 {
        // A hidden icon reports an empty or absent rectangle rather than an
        // error, so the caller cannot distinguish those from failure here.
        return None;
    }
    let found = crate::spacer::Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    (!found.is_empty()).then_some(found)
}
