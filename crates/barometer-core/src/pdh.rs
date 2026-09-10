// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Performance counters.
//
// GPU utilization and per-disk throughput are published by Windows only as
// performance counters; there is no plain API for either. This wraps the parts
// of PDH needed for that and hands back plain Rust values.
//
// Counters are always added with the *English* call. Counter paths are
// localized on a localized Windows - "\Processor" is "\Prozessor" on a German
// install - and a query built from an English literal simply fails there. The
// English variant translates for us, and it is the difference between working
// everywhere and working in one language.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
};

// Deliberately not using PDH_FMT_NOCAP100 (0x8000), which lifts PDH's clamp of
// formatted percentages to 100.
//
// It looks like the careful choice and it is wrong here. Every instance of a
// per-instance percentage counter is genuinely bounded at 100, and uncapped
// transient overshoot multiplies once the instances are summed: with it set,
// GPU utilization on the author's machine read 78% against Windows' own
// 15.3% across 787 engine instances. Without it, 17-19%.
//
// A counter that legitimately exceeds 100 should ask for that itself rather
// than every counter paying for it.

/// PDH's "the buffer you gave me is too small" status.
const PDH_MORE_DATA: u32 = 0x800007D2;

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// One instance of a wildcard counter.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// The instance name PDH expanded the wildcard to.
    pub name: String,
    pub value: f64,
}

/// A query holding one wildcard counter.
///
/// One counter per query rather than many, because the interesting counters
/// here are wildcards whose instance sets change independently - GPU engines
/// come and go with processes - and a query that fails as a unit would take
/// the healthy counters down with the sick one.
pub struct Counter {
    query: isize,
    counter: isize,
    /// PDH computes rates from consecutive samples, so the first collection
    /// establishes a baseline and yields nothing usable.
    primed: bool,
}

impl Counter {
    /// Opens a query for one counter path, which may contain a wildcard.
    pub fn open(path: &str) -> Option<Counter> {
        let mut query: isize = 0;
        // SAFETY: null name and userdata are the documented "real time, no
        // context" form; query is written on success.
        let status = unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) };
        if status != 0 {
            return None;
        }

        let wide_path = wide(path);
        let mut counter: isize = 0;
        // SAFETY: query is open, path is NUL terminated, counter is written.
        let status =
            unsafe { PdhAddEnglishCounterW(query, wide_path.as_ptr(), 0, &mut counter) };
        if status != 0 {
            // SAFETY: closing a query we opened.
            unsafe { PdhCloseQuery(query) };
            return None;
        }

        let mut counter = Counter { query, counter, primed: false };
        // Prime immediately so the first real read has something to subtract
        // from, rather than returning nothing on the first tick.
        counter.collect();
        Some(counter)
    }

    fn collect(&mut self) -> bool {
        // SAFETY: the query is open for the life of this struct.
        let status = unsafe { PdhCollectQueryData(self.query) };
        if status == 0 {
            self.primed = true;
            true
        } else {
            false
        }
    }

    /// Samples every instance of the counter.
    ///
    /// Returns None until two collections have happened, because a rate needs
    /// two points and reporting zero would look like an idle device.
    pub fn read(&mut self) -> Option<Vec<Instance>> {
        let was_primed = self.primed;
        if !self.collect() || !was_primed {
            return None;
        }

        let mut size: u32 = 0;
        let mut count: u32 = 0;
        // SAFETY: the documented two-call pattern. The first asks for the size
        // with a null buffer and is expected to fail with PDH_MORE_DATA.
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                std::ptr::null_mut(),
            )
        };
        if status != PDH_MORE_DATA {
            // A refusal, not an absence. Returning an empty list here made a
            // damaged counter registry indistinguishable from a machine with
            // no disks, so the modules set `reading = true` and showed a
            // confident 0.00 KB/s while the disk was being hammered.
            return None;
        }
        if size == 0 || count == 0 {
            return Some(Vec::new());
        }

        // PDH writes the item array and its instance-name strings into one
        // buffer, so it is sized in bytes and read back as items.
        //
        // Allocated as items rather than as bytes: a `Vec<u8>` is aligned to
        // one, and forming a reference to a `PDH_FMT_COUNTERVALUE_ITEM_W` -
        // which wants eight - at an address that does not suit it is
        // undefined however the address happens to come out in practice. A
        // vector of the item type is aligned by construction, and is rounded
        // up so it is never shorter than the bytes PDH asked for.
        let items_wanted = (size as usize).div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
        let mut buffer: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> =
            Vec::with_capacity(items_wanted.max(count as usize));
        // SAFETY: the items are plain data with no invalid bit patterns, and
        // every one of them is written by PDH below before it is read.
        unsafe { buffer.set_len(items_wanted.max(count as usize)) };
        // SAFETY: buffer is size bytes, which is what PDH just asked for.
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                buffer.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            // As above: a failure to read is not a reading of nothing.
            return None;
        }

        let items = buffer.as_ptr();
        let mut out = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            // SAFETY: PDH reported count items written into this buffer, and
            // each szName points inside it.
            let item = unsafe { &*items.add(index) };
            let name = unsafe { wide_to_string(item.szName) };
            // A per-instance status other than zero means that one instance
            // could not be formatted; skip it rather than reporting a zero.
            if item.FmtValue.CStatus != 0 {
                continue;
            }
            // SAFETY: the format requested was PDH_FMT_DOUBLE, so this is the
            // live union member.
            let value = unsafe { item.FmtValue.Anonymous.doubleValue };
            out.push(Instance { name, value });
        }
        Some(out)
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        // SAFETY: closing a query this struct opened and owns.
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// Reads a NUL-terminated wide string PDH left inside its own buffer.
///
/// # Safety
/// `pointer` must be a valid NUL-terminated wide string.
unsafe fn wide_to_string(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0;
    while *pointer.add(length) != 0 {
        length += 1;
    }
    OsString::from_wide(std::slice::from_raw_parts(pointer, length))
        .to_string_lossy()
        .into_owned()
}
