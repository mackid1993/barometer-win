// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Process icons for the panels' process rows: the executable's own icon,
//! as the shell would show it, looked up by process id.
//!
//! Cached twice over. By path, because a hundred Chrome processes share one
//! executable and one icon; and by process id, because resolving the path
//! opens the process and a panel redraws many times a second. The icon
//! handles are kept for the life of the program: an icon is a few kilobytes,
//! the set of programs a person runs is small, and destroying one while a
//! panel could still be drawing it is a use-after-free waiting for its
//! moment.
//!
//! What is *not* kept for the life of the program is the process id map.
//! Windows reuses process ids freely, and a panel left open through a working
//! day would otherwise show a long-dead process's icon beside a freshly
//! started program's name. Those entries go stale after `PID_MEMORY`.
//!
//! The shell lookup itself runs on a thread. `SHGetFileInfoW` reads the
//! executable, so an entry whose program lives on a disconnected share or a
//! spun-down disk takes as long as that disk does - and layout runs inside
//! `WM_PAINT`, which would stall the painting of a foreground window. A row
//! whose icon has not arrived yet simply draws without one, and has it a
//! frame later.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON};

use barometer_core::sys::processes::image_path;

/// How long a process id is believed to still mean the same process.
///
/// Long enough that a burst of redraws asks the kernel once; short enough
/// that a recycled id is a curiosity rather than a wrong icon somebody looks
/// at. Windows hands ids back out within seconds on a busy machine, so this
/// is deliberately not minutes.
const PID_MEMORY: Duration = Duration::from_secs(10);

struct Cache {
    /// What a process id resolved to, and when that was decided.
    by_pid: HashMap<u32, (Instant, Option<String>)>,
    by_path: HashMap<String, Option<isize>>,
    /// Paths a worker is asking the shell about, so a redraw does not start
    /// a second worker on the same file every frame while the first waits.
    asking: HashSet<String>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The small icon of the executable behind a process, as a raw HICON.
///
/// None when the process cannot be opened, its file has no icon, or the
/// shell has not answered about it yet.
pub fn icon_for(pid: u32) -> Option<isize> {
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(|| Cache {
        by_pid: HashMap::new(),
        by_path: HashMap::new(),
        asking: HashSet::new(),
    });

    let now = Instant::now();
    let path = match cache.by_pid.get(&pid) {
        Some((asked, path)) if now.duration_since(*asked) < PID_MEMORY => path.clone(),
        _ => {
            // Anything else that has gone stale goes with it, so the map is
            // bounded by what is running rather than by what ever ran.
            cache.by_pid.retain(|_, (asked, _)| now.duration_since(*asked) < PID_MEMORY);
            let found = image_path(pid);
            cache.by_pid.insert(pid, (now, found.clone()));
            found
        }
    };
    let path = path?;

    if let Some(known) = cache.by_path.get(&path) {
        return *known;
    }
    if cache.asking.insert(path.clone()) {
        std::thread::spawn(move || {
            let loaded = load_small_icon(&path);
            if let Ok(mut guard) = CACHE.lock() {
                if let Some(cache) = guard.as_mut() {
                    cache.asking.remove(&path);
                    // Remembered whether or not there was an icon: a file the
                    // shell has no icon for will not grow one, and asking
                    // again every frame is what this cache is for.
                    cache.by_path.insert(path, loaded);
                }
            }
        });
    }
    None
}

fn load_small_icon(path: &str) -> Option<isize> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a terminated path and a zeroed info struct of the size given.
    unsafe {
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let found = SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        );
        (found != 0 && !info.hIcon.is_null()).then_some(info.hIcon as isize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_that_does_not_exist_has_no_icon_and_the_miss_is_remembered() {
        let absent = u32::MAX - 7;
        assert_eq!(icon_for(absent), None);
        assert_eq!(icon_for(absent), None);
        let guard = CACHE.lock().expect("cache");
        let cache = guard.as_ref().expect("asked at least once");
        assert!(matches!(cache.by_pid.get(&absent), Some((_, None))));
    }

    #[test]
    fn this_process_has_an_icon_or_at_least_a_path() {
        // The test runner is an executable with a path; whether the shell
        // finds an icon in it is the shell's business, and it is asked on a
        // thread, so the first call is expected to come back empty-handed.
        let pid = std::process::id();
        assert!(image_path(pid).is_some());
        let _ = icon_for(pid);
    }

    #[test]
    fn a_process_id_is_looked_up_again_once_it_could_mean_another_process() {
        // Windows hands process ids back out, so an entry that never expired
        // showed a dead process's icon beside a live process's name. The map
        // is also pruned as it goes, rather than growing for the life of the
        // program.
        let pid = u32::MAX - 9;
        assert_eq!(icon_for(pid), None);
        {
            let mut guard = CACHE.lock().expect("cache");
            let cache = guard.as_mut().expect("asked at least once");
            let stale = Instant::now() - PID_MEMORY - Duration::from_secs(1);
            cache.by_pid.insert(pid, (stale, Some("C:/nowhere/gone.exe".into())));
        }
        assert_eq!(icon_for(pid), None);
        let guard = CACHE.lock().expect("cache");
        let cache = guard.as_ref().expect("cache");
        // Asked again rather than trusted, so the path of the process that
        // used to hold this id is gone.
        assert!(matches!(cache.by_pid.get(&pid), Some((_, None))));
    }
}
