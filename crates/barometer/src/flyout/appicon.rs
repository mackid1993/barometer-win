// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Process icons for the panels' process rows: the executable's own icon,
//! as the shell would show it, looked up by process id.
//!
//! Cached twice over. By path, because a hundred Chrome processes share one
//! executable and one icon; and by process id, because resolving the path
//! opens the process and a panel redraws many times a second.
//!
//! The icon handles used to be kept for the life of the program, on the
//! reasoning that the set of programs a person runs is small and that
//! destroying one while a panel could still be drawing it is a use-after-free
//! waiting for its moment. The second half of that is true and is why
//! eviction happens where it does; the first half is not. Installers,
//! updaters and per-version application folders come and go all day, each
//! with a path of its own, and these are USER handles against a quota of ten
//! thousand. `MAX_ICONS` is the ceiling, and the coldest goes first.
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
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

use barometer_core::sys::processes::image_path;

/// How long a process id is believed to still mean the same process.
///
/// Long enough that a burst of redraws asks the kernel once; short enough
/// that a recycled id is a curiosity rather than a wrong icon somebody looks
/// at. Windows hands ids back out within seconds on a busy machine, so this
/// is deliberately not minutes.
const PID_MEMORY: Duration = Duration::from_secs(10);

/// How many executables' icons are kept.
///
/// A process list is twelve rows, and the panels between them show three,
/// so this is never reached by what is on screen - which is what makes
/// evicting the coldest safe as well as cheap.
const MAX_ICONS: usize = 64;

struct Cache {
    /// What a process id resolved to, and when that was decided.
    by_pid: HashMap<u32, (Instant, Option<String>)>,
    /// The icon for an executable, and the pass it was last asked for on.
    by_path: HashMap<String, (u64, Option<isize>)>,
    /// Bumped once per layout. Ages are counted in passes rather than in
    /// calls because every row of one list is equally recent.
    pass: u64,
    /// Paths a worker is asking the shell about, so a redraw does not start
    /// a second worker on the same file every frame while the first waits.
    asking: HashSet<String>,
}

impl Cache {
    fn new() -> Cache {
        Cache { by_pid: HashMap::new(), by_path: HashMap::new(), pass: 0, asking: HashSet::new() }
    }

    /// Removes entries past `ceiling`, coldest first, and hands back the
    /// icons they held for the caller to destroy.
    ///
    /// Separated from the destroying so that the order can be tested
    /// without a window manager: what goes and what stays is arithmetic,
    /// and `DestroyIcon` on an invented number is not something a test
    /// should ever do.
    fn evict_beyond(&mut self, ceiling: usize) -> Vec<isize> {
        let mut given_back = Vec::new();
        while self.by_path.len() > ceiling {
            let Some(coldest) = self
                .by_path
                .iter()
                .min_by_key(|(path, (pass, _))| (*pass, (*path).clone()))
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            if let Some((_, Some(icon))) = self.by_path.remove(&coldest) {
                given_back.push(icon);
            }
        }
        given_back
    }
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The small icon of the executable behind a process, as a raw HICON.
///
/// None when the process cannot be opened, its file has no icon, or the
/// shell has not answered about it yet.
pub fn icon_for(pid: u32) -> Option<isize> {
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(Cache::new);

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

    let pass = cache.pass;
    if let Some(known) = cache.by_path.get_mut(&path) {
        known.0 = pass;
        return known.1;
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
                    let pass = cache.pass;
                    cache.by_path.insert(path, (pass, loaded));
                }
            }
        });
    }
    None
}

/// Starts a layout and gives back the icons nothing has drawn in a while.
///
/// Called from `Panel::ensure_layout` and nowhere else, and that is what
/// makes it safe. A row's handle is put into the laid-out element and drawn
/// from there on every repaint until the page is built again, so an icon
/// must never be destroyed while a layout stands. The moment before a new
/// one is built, the old one is about to be thrown away and the new one has
/// asked for nothing yet.
pub fn begin_pass() {
    let Ok(mut guard) = CACHE.lock() else { return };
    let Some(cache) = guard.as_mut() else { return };
    cache.pass = cache.pass.wrapping_add(1);
    for icon in cache.evict_beyond(MAX_ICONS) {
        // SAFETY: an icon this cache loaded, destroyed once, and in no
        // layout any more - see above.
        unsafe { DestroyIcon(icon as HICON) };
    }
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

    /// A cache of invented handles, so the eviction order can be checked
    /// without asking the shell for a real icon or destroying one.
    fn fake(entries: &[(&str, u64, Option<isize>)]) -> Cache {
        let mut cache = Cache::new();
        for (path, pass, icon) in entries {
            cache.by_path.insert((*path).to_string(), (*pass, *icon));
        }
        cache
    }

    #[test]
    fn the_coldest_icons_are_the_ones_given_back_and_a_handle_comes_back_with_each() {
        let mut cache = fake(&[
            ("chrome.exe", 9, Some(0x11)),
            ("explorer.exe", 7, Some(0x22)),
            ("some-installer.exe", 2, Some(0x33)),
            ("an-updater.exe", 1, Some(0x44)),
        ]);

        // Under the ceiling nothing goes, however old it is: what is on screen
        // is what matters, and four rows is not a leak.
        assert!(cache.evict_beyond(4).is_empty());
        assert_eq!(cache.by_path.len(), 4);

        // Over it, oldest pass first, and the handle comes back so the caller
        // can destroy it - which is the whole point of separating the two.
        assert_eq!(cache.evict_beyond(2), vec![0x44, 0x33]);
        assert_eq!(cache.by_path.len(), 2);
        assert!(cache.by_path.contains_key("chrome.exe"));
        assert!(cache.by_path.contains_key("explorer.exe"));

        // An executable the shell had no icon for is still an entry to give
        // back, and there is simply no handle to destroy with it.
        let mut nothing = fake(&[("a.exe", 1, None), ("b.exe", 2, Some(0x55))]);
        assert!(nothing.evict_beyond(1).is_empty(), "a missing icon leaves nothing to destroy");
        assert_eq!(nothing.by_path.len(), 1);
        assert!(nothing.by_path.contains_key("b.exe"));
    }

    #[test]
    fn asking_for_an_icon_makes_it_the_newest_so_a_row_on_screen_is_never_the_one_evicted() {
        let mut cache = fake(&[("old.exe", 1, Some(0x11)), ("new.exe", 5, Some(0x22))]);
        // What `icon_for` does on a hit: the entry takes the current pass.
        cache.pass = 9;
        let pass = cache.pass;
        cache.by_path.get_mut("old.exe").expect("present").0 = pass;
        assert_eq!(cache.evict_beyond(1), vec![0x22], "the one nobody asked for goes");
        assert!(cache.by_path.contains_key("old.exe"));
    }

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
