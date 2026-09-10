// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Taskbar-button obstruction checks.
//!
//! The query shape is deliberate. Cheaper variants tried locally reintroduced
//! the flash it exists to prevent.
//!
//! UI Automation calls into Explorer and pump messages while they wait, so the
//! query runs on a background MTA thread and never on the one that owns the
//! readout. Every button in the taskbar is enumerated, not just ours: the point
//! is to find another program's icon dragged in among the placeholders, and an
//! icon we cannot see is one we will happily cover.

use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::core::{BSTR, GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    CloseHandle, SysFreeString, SysStringLen, HANDLE, HWND, RECT,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows_sys::Win32::System::Variant::{VARIANT, VT_I4};
use windows_sys::Win32::UI::Accessibility::{
    AutomationElementMode_None, CUIAutomation, SetWinEventHook, TreeScope_Descendants,
    TreeScope_Element, UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId,
    UIA_ControlTypePropertyId, UIA_NamePropertyId, UnhookWinEvent, HWINEVENTHOOK,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetAncestor, GetWindowRect, GetWindowThreadProcessId, PostMessageW,
    CHILDID_SELF, EVENT_OBJECT_CREATE, EVENT_OBJECT_LOCATIONCHANGE, GA_ROOT, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::spacer::{Obstruction, Rect};
use crate::tray::RESERVE_TIP;

/// Fast polling follows a changing tray; the event hook wakes a settled worker.
const QUERY_INTERVAL_FAST: Duration = Duration::from_millis(250);
const QUERY_INTERVAL_SLOW: Duration = Duration::from_millis(5_000);
const STABLE_COUNT_FOR_SLOW: u32 = 8;
/// Taskbar animations can flood the hook. Never run two sweeps closer than this.
const MIN_QUERY_GAP: Duration = Duration::from_millis(200);

static WAKE_EVENT: AtomicUsize = AtomicUsize::new(0);
static HOOKED_TASKBAR: AtomicUsize = AtomicUsize::new(0);

const IID_IUIAUTOMATION: GUID = GUID::from_u128(0x30cbe57d_d9d0_452a_ab13_7ac5ac4825ee);
const TASKBAR_CLASS: [u16; 14] = [
    b'S' as u16,
    b'h' as u16,
    b'e' as u16,
    b'l' as u16,
    b'l' as u16,
    b'_' as u16,
    b'T' as u16,
    b'r' as u16,
    b'a' as u16,
    b'y' as u16,
    b'W' as u16,
    b'n' as u16,
    b'd' as u16,
    0,
];

/// One authoritative result from one taskbar-button sweep.
///
/// `valid` says whether at least one placeholder contributed to `region`.
/// A working enumeration that sees only foreign buttons publishes an invalid
/// snapshot. A total UI Automation blackout publishes nothing, leaving the
/// previous snapshot untouched.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub valid: bool,
    pub found: usize,
    pub region: Rect,
    pub obstructed: bool,
}

#[derive(Copy, Clone, Debug, Default)]
struct Request {
    expected: usize,
    generation: u64,
}

#[derive(Copy, Clone, Debug, Default)]
struct Published {
    generation: u64,
    snapshot: Snapshot,
}

/// The background query and its latest complete answer.
pub struct TaskbarButtons {
    request: Arc<Mutex<Request>>,
    published: Arc<Mutex<Published>>,
    stop: Arc<AtomicBool>,
    wake: usize,
    hook: Mutex<Hook>,
    thread: Option<JoinHandle<()>>,
}

impl TaskbarButtons {
    pub fn new() -> Self {
        let request = Arc::new(Mutex::new(Request::default()));
        let published = Arc::new(Mutex::new(Published::default()));
        let stop = Arc::new(AtomicBool::new(false));
        // SAFETY: unnamed auto-reset event, initially nonsignaled.
        let wake = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) } as usize;
        // The hook is installed by the window-owning thread, which is also the
        // thread on which out-of-context callbacks are delivered.
        let mut hook = Hook::new(wake);
        hook.ensure();

        let worker_request = Arc::clone(&request);
        let worker_published = Arc::clone(&published);
        let worker_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("taskbar-buttons".to_owned())
            .spawn(move || run(worker_request, worker_published, worker_stop, wake))
            .ok();

        Self {
            request,
            published,
            stop,
            wake,
            hook: Mutex::new(hook),
            thread,
        }
    }

    /// Tells the query how many placeholders it should expect to find.
    pub fn set_placeholders(&self, held: usize) {
        if let Ok(mut hook) = self.hook.lock() {
            hook.ensure();
        }
        let generation = {
            let Ok(mut request) = self.request.lock() else {
                return;
            };
            if request.expected == held {
                return;
            }
            request.expected = held;
            request.generation = request.generation.wrapping_add(1);
            request.generation
        };
        self.clear(generation);
    }

    /// Invalidates every answer tied to the old Explorer taskbar.
    pub fn invalidate(&self) {
        if let Ok(mut hook) = self.hook.lock() {
            hook.ensure();
        }
        let generation = {
            let Ok(mut request) = self.request.lock() else {
                return;
            };
            request.generation = request.generation.wrapping_add(1);
            request.generation
        };
        self.clear(generation);
    }

    fn clear(&self, generation: u64) {
        if let Ok(mut published) = self.published.lock() {
            *published = Published {
                generation,
                snapshot: Snapshot::default(),
            };
        }
        if self.wake != 0 {
            // SAFETY: the event belongs to this worker and remains open until
            // after the worker has joined.
            unsafe { SetEvent(self.wake as HANDLE) };
        }
    }

    /// The last nonblackout answer for the current placeholder generation.
    pub fn snapshot(&self) -> Snapshot {
        let generation = self
            .request
            .lock()
            .map(|value| value.generation)
            .unwrap_or(0);
        self.published
            .lock()
            .map(|value| {
                if value.generation == generation {
                    value.snapshot
                } else {
                    Snapshot::default()
                }
            })
            .unwrap_or_default()
    }

    pub fn is_obstructed(&self) -> bool {
        self.snapshot().obstructed
    }
}

impl Drop for TaskbarButtons {
    fn drop(&mut self) {
        if let Ok(mut hook) = self.hook.lock() {
            // SAFETY: this thread installed and owns the hook.
            unsafe { hook.remove() };
        }
        self.stop.store(true, Ordering::Release);
        if self.wake != 0 {
            // SAFETY: waking the worker makes shutdown immediate even when it
            // has settled into the slow interval.
            unsafe { SetEvent(self.wake as HANDLE) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if self.wake != 0 {
            // SAFETY: the callback and worker are gone after the join.
            unsafe { CloseHandle(self.wake as HANDLE) };
        }
    }
}

fn run(
    request: Arc<Mutex<Request>>,
    published: Arc<Mutex<Published>>,
    stop: Arc<AtomicBool>,
    wake: usize,
) {
    // SAFETY: this is a fresh worker thread and is uninitialized for COM.
    let initialized = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) } >= 0;
    if !initialized {
        return;
    }

    // SAFETY: COM is initialized above and the returned pointer is owned by
    // the query. Failure simply leaves UI Automation unavailable for this run.
    if let Some(automation) = unsafe { AutomationQuery::new() } {
        let mut state = Obstruction::default();
        let mut cadence = Cadence::default();
        let mut generation = u64::MAX;
        while !stop.load(Ordering::Acquire) {
            let query_start = Instant::now();
            let current = request.lock().map(|value| *value).unwrap_or_default();
            if current.generation != generation {
                state = Obstruction::default();
                cadence = Cadence::default();
                generation = current.generation;
            }

            let sample = if current.expected == 0 {
                Some(Sample::default())
            } else {
                // SAFETY: the COM query and all cached interfaces belong to
                // this MTA thread.
                unsafe { sample(&automation, current.expected) }
            };

            if let Some(sample) = sample {
                let next = finish_sample(&mut state, sample);
                let previous = published
                    .lock()
                    .map(|value| {
                        if value.generation == generation {
                            value.snapshot
                        } else {
                            Snapshot::default()
                        }
                    })
                    .unwrap_or_default();
                let changed =
                    next.valid != previous.valid || (next.valid && next.region != previous.region);

                // A count change can happen while Explorer is answering the
                // UIA call. Tag every result and discard one from an old count.
                let still_current = request
                    .lock()
                    .map(|value| value.generation == generation)
                    .unwrap_or(false);
                if still_current {
                    if let Ok(mut value) = published.lock() {
                        *value = Published {
                            generation,
                            snapshot: next,
                        };
                    }
                    if crate::trace::on() {
                        crate::trace::line(&format!(
                            "uia expected {} found {} region {},{}-{},{} valid {} obstructed {}",
                            current.expected,
                            next.found,
                            next.region.left,
                            next.region.top,
                            next.region.right,
                            next.region.bottom,
                            next.valid,
                            next.obstructed
                        ));
                    }
                    cadence.observe(changed, next.region);
                }
            }

            wait_for_next(wake, cadence.interval, &stop, query_start);
        }
    }

    // SAFETY: paired with the successful initialization on this thread.
    unsafe { CoUninitialize() };
}

#[derive(Default)]
struct Sample {
    /// The union of the placeholders UI Automation could actually see.
    reserved: Rect,
    found: usize,
    /// Every other button in the taskbar.
    others: Vec<Rect>,
}

unsafe fn sample(automation: &AutomationQuery, expected: usize) -> Option<Sample> {
    let taskbar = FindWindowW(TASKBAR_CLASS.as_ptr(), ptr::null());
    if taskbar.is_null() {
        return None;
    }
    let mut raw: RECT = mem::zeroed();
    if GetWindowRect(taskbar, &mut raw) == 0 {
        return None;
    }
    let bar = Rect {
        left: raw.left,
        top: raw.top,
        right: raw.right,
        bottom: raw.bottom,
    };
    if bar.is_empty() {
        return None;
    }

    classify(expected, automation.taskbar_buttons(taskbar, bar)?)
}

fn finish_sample(state: &mut Obstruction, sample: Sample) -> Snapshot {
    let obstructed = state.observe(sample.reserved, &sample.others);
    Snapshot {
        valid: sample.found > 0 && !sample.reserved.is_empty(),
        found: sample.found,
        region: sample.reserved,
        obstructed,
    }
}

#[derive(Debug)]
struct Cadence {
    last_region: Rect,
    stable: u32,
    interval: Duration,
}

impl Default for Cadence {
    fn default() -> Self {
        Self {
            last_region: Rect::default(),
            stable: 0,
            interval: QUERY_INTERVAL_FAST,
        }
    }
}

impl Cadence {
    fn observe(&mut self, changed: bool, region: Rect) {
        if changed || region != self.last_region {
            self.stable = 0;
            self.interval = QUERY_INTERVAL_FAST;
        } else {
            self.stable = self.stable.saturating_add(1);
            if self.stable >= STABLE_COUNT_FOR_SLOW {
                self.interval = QUERY_INTERVAL_SLOW;
            }
        }
        self.last_region = region;
    }
}

fn wait_for_next(wake: usize, interval: Duration, stop: &AtomicBool, started: Instant) {
    if wake != 0 {
        // SAFETY: the event remains open until the worker exits.
        unsafe { WaitForSingleObject(wake as HANDLE, interval.as_millis() as u32) };
        let elapsed = started.elapsed();
        if !stop.load(Ordering::Acquire) && elapsed < MIN_QUERY_GAP {
            thread::sleep(MIN_QUERY_GAP - elapsed);
        }
    } else {
        let until = Instant::now() + interval;
        while !stop.load(Ordering::Acquire) && Instant::now() < until {
            thread::sleep(
                Duration::from_millis(50).min(until.saturating_duration_since(Instant::now())),
            );
        }
    }
}

struct Hook {
    handle: HWINEVENTHOOK,
    taskbar: HWND,
    wake: usize,
}

impl Hook {
    fn new(wake: usize) -> Self {
        WAKE_EVENT.store(wake, Ordering::Release);
        Self {
            handle: ptr::null_mut(),
            taskbar: ptr::null_mut(),
            wake,
        }
    }

    fn ensure(&mut self) {
        // SAFETY: class lookup and an out-of-context hook scoped to Explorer's
        // taskbar thread. The callback never enters Explorer's process.
        unsafe {
            let taskbar = FindWindowW(TASKBAR_CLASS.as_ptr(), ptr::null());
            if !self.handle.is_null() && taskbar == self.taskbar {
                return;
            }
            self.remove();
            if taskbar.is_null() || self.wake == 0 {
                return;
            }
            let mut process = 0;
            let thread = GetWindowThreadProcessId(taskbar, &mut process);
            if process == 0 || thread == 0 {
                return;
            }
            self.taskbar = taskbar;
            HOOKED_TASKBAR.store(taskbar as usize, Ordering::Release);
            self.handle = SetWinEventHook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_LOCATIONCHANGE,
                ptr::null_mut(),
                Some(win_event_proc),
                process,
                thread,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if self.handle.is_null() {
                self.taskbar = ptr::null_mut();
                HOOKED_TASKBAR.store(0, Ordering::Release);
            }
        }
    }

    unsafe fn remove(&mut self) {
        if !self.handle.is_null() {
            UnhookWinEvent(self.handle);
            self.handle = ptr::null_mut();
        }
        self.taskbar = ptr::null_mut();
        HOOKED_TASKBAR.store(0, Ordering::Release);
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: this worker owns the hook.
        unsafe { self.remove() };
        WAKE_EVENT.store(0, Ordering::Release);
    }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != OBJID_WINDOW || child != CHILDID_SELF as i32 || hwnd.is_null() {
        return;
    }
    let taskbar = HOOKED_TASKBAR.load(Ordering::Acquire) as HWND;
    if taskbar.is_null() || GetAncestor(hwnd, GA_ROOT) != taskbar {
        return;
    }
    let wake = WAKE_EVENT.load(Ordering::Acquire);
    if wake != 0 {
        SetEvent(wake as HANDLE);
    }
    // The same event that wakes the sweep is the moment the shell restacks
    // the tray, and the readout is a topmost popup that gets restacked under.
    // TrafficMonitor re-asserts topmost from the layout-changed message its
    // sweep posts; this is that message. Posted, not sent: this runs on the
    // sweep's thread and must never wait on the one that owns the window.
    let strip = crate::window::strip_window();
    if !strip.is_null() {
        PostMessageW(strip, crate::window::WM_RAISE, 0, 0);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Button {
    rect: Rect,
    name: String,
}

fn classify(expected: usize, buttons: Vec<Button>) -> Option<Sample> {
    let mut reserved = Rect::default();
    let mut others = Vec::new();
    let mut found = 0;
    for button in buttons {
        // Matched by substring rather than equality, because the shell
        // decorates the tooltip and strict equality would recognize none.
        if button.name.contains(RESERVE_TIP) {
            reserved = reserved.union(button.rect);
            found += 1;
        } else {
            others.push(button.rect);
        }
    }

    // UI Automation intermittently enumerates nothing at all inside the
    // taskbar. The most reproducible way to see it is the taskbar's own
    // right-click menu: a normal sweep can see dozens of buttons and then zero
    // while that menu is up. Every placeholder is still
    // there; only the view of them is gone. Reporting that honestly makes the
    // caller conclude the region has vanished and hide the readout, so the
    // sample is thrown away and the previous answer stands.
    //
    // Only a completely empty enumeration counts. If other programs' icons
    // still came back then the enumeration worked, and placeholders genuinely
    // missing from it should be reported as missing.
    if expected > 0 && found == 0 && others.is_empty() {
        return None;
    }

    // Whatever placeholders are genuinely on screen are enough. Demanding all
    // of them makes the sample worthless whenever one lags behind, which is
    // exactly what the chevron causes, and the readout then never settles.
    Some(Sample {
        reserved,
        found,
        others,
    })
}

type AutomationQuery = ComPtr;

struct ComPtr(*mut c_void);

impl ComPtr {
    unsafe fn new() -> Option<Self> {
        create_automation()
    }

    unsafe fn from_raw(value: *mut c_void) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }

    /// Every button in the taskbar, with the name the shell gives it.
    ///
    /// One `FindAllBuildCache` with a cache request, rather than a property
    /// read per element. Reading each name and rectangle in its own
    /// cross-process call made the taskbar visibly sluggish because Explorer's
    /// taskbar thread was spending its time answering us.
    unsafe fn taskbar_buttons(&self, taskbar: HWND, bar: Rect) -> Option<Vec<Button>> {
        let automation = vtable::<AutomationVtable>(self.0);

        let mut root = ptr::null_mut();
        if (automation.element_from_handle)(self.0, taskbar, &mut root) < 0 {
            return None;
        }
        let root = ComPtr::from_raw(root)?;

        let mut cache = ptr::null_mut();
        if (automation.create_cache_request)(self.0, &mut cache) < 0 {
            return None;
        }
        let cache = ComPtr::from_raw(cache)?;
        let cache_vtable = vtable::<CacheRequestVtable>(cache.0);
        if (cache_vtable.add_property)(cache.0, UIA_NamePropertyId) < 0
            || (cache_vtable.add_property)(cache.0, UIA_BoundingRectanglePropertyId) < 0
            || (cache_vtable.set_tree_scope)(cache.0, TreeScope_Element) < 0
            || (cache_vtable.set_automation_element_mode)(cache.0, AutomationElementMode_None) < 0
        {
            return None;
        }

        let mut value: VARIANT = mem::zeroed();
        value.Anonymous.Anonymous.vt = VT_I4;
        value.Anonymous.Anonymous.Anonymous.lVal = UIA_ButtonControlTypeId;
        let mut condition = ptr::null_mut();
        if (automation.create_property_condition)(
            self.0,
            UIA_ControlTypePropertyId,
            value,
            &mut condition,
        ) < 0
        {
            return None;
        }
        let condition = ComPtr::from_raw(condition)?;

        let root_vtable = vtable::<ElementVtable>(root.0);
        let mut elements = ptr::null_mut();
        if (root_vtable.find_all_build_cache)(
            root.0,
            TreeScope_Descendants,
            condition.0,
            cache.0,
            &mut elements,
        ) < 0
        {
            return None;
        }
        let elements = ComPtr::from_raw(elements)?;
        let array_vtable = vtable::<ElementArrayVtable>(elements.0);
        let mut length = 0;
        if (array_vtable.length)(elements.0, &mut length) < 0 {
            return None;
        }

        let mut result = Vec::with_capacity(length.max(0) as usize);
        for index in 0..length {
            let mut element = ptr::null_mut();
            if (array_vtable.get_element)(elements.0, index, &mut element) < 0 {
                continue;
            }
            let Some(element) = ComPtr::from_raw(element) else {
                continue;
            };
            let element_vtable = vtable::<ElementVtable>(element.0);
            let mut rect: RECT = mem::zeroed();
            if (element_vtable.cached_bounding_rectangle)(element.0, &mut rect) < 0 {
                continue;
            }
            let rect = Rect {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            };
            // Nonsense is discarded: while the taskbar is being moved UI
            // Automation briefly answers with empty rectangles and with
            // elements that are not inside it at all. Trusting those throws
            // the readout off screen.
            if rect.is_empty() || !rect.intersects(bar) {
                continue;
            }
            let mut name: BSTR = ptr::null();
            let name = if (element_vtable.cached_name)(element.0, &mut name) >= 0 {
                take_bstr(name)
            } else {
                String::new()
            };
            result.push(Button { rect, name });
        }
        Some(result)
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        // SAFETY: every ComPtr owns one COM reference.
        unsafe { (vtable::<UnknownVtable>(self.0).release)(self.0) };
    }
}

unsafe fn create_automation() -> Option<ComPtr> {
    let mut automation = ptr::null_mut();
    if CoCreateInstance(
        &CUIAutomation,
        ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_IUIAUTOMATION,
        &mut automation,
    ) < 0
    {
        return None;
    }
    ComPtr::from_raw(automation)
}

unsafe fn vtable<'a, T>(object: *mut c_void) -> &'a T {
    &**(object as *const *const T)
}

#[repr(C)]
struct UnknownVtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct AutomationVtable {
    base: UnknownVtable,
    before_element_from_handle: [usize; 3],
    element_from_handle: unsafe extern "system" fn(*mut c_void, HWND, *mut *mut c_void) -> HRESULT,
    before_create_cache_request: [usize; 13],
    create_cache_request: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    create_true_condition: usize,
    create_false_condition: usize,
    create_property_condition:
        unsafe extern "system" fn(*mut c_void, i32, VARIANT, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct CacheRequestVtable {
    base: UnknownVtable,
    add_property: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    before_set_tree_scope: [usize; 3],
    set_tree_scope: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    before_set_automation_element_mode: [usize; 3],
    set_automation_element_mode: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
}

#[repr(C)]
struct ElementVtable {
    base: UnknownVtable,
    before_find_all_build_cache: [usize; 5],
    find_all_build_cache: unsafe extern "system" fn(
        *mut c_void,
        i32,
        *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    before_cached_name: [usize; 46],
    cached_name: unsafe extern "system" fn(*mut c_void, *mut BSTR) -> HRESULT,
    before_cached_bounding_rectangle: [usize; 19],
    cached_bounding_rectangle: unsafe extern "system" fn(*mut c_void, *mut RECT) -> HRESULT,
}

#[repr(C)]
struct ElementArrayVtable {
    base: UnknownVtable,
    length: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    get_element: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> HRESULT,
}

unsafe fn take_bstr(value: BSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    let length = SysStringLen(value) as usize;
    let text = String::from_utf16_lossy(std::slice::from_raw_parts(value, length));
    SysFreeString(value);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, right: i32) -> Rect {
        Rect {
            left,
            top: 0,
            right,
            bottom: 32,
        }
    }

    fn ours(left: i32, right: i32) -> Button {
        Button {
            rect: rect(left, right),
            name: RESERVE_TIP.to_owned(),
        }
    }

    fn theirs(left: i32, right: i32, name: &str) -> Button {
        Button {
            rect: rect(left, right),
            name: name.to_owned(),
        }
    }

    #[test]
    fn our_own_placeholders_are_never_counted_as_foreign() {
        let sample = classify(2, vec![ours(100, 140), ours(140, 180)]).unwrap();
        assert!(sample.others.is_empty());
        assert_eq!(sample.reserved, rect(100, 180));
    }

    #[test]
    fn a_decorated_tooltip_still_identifies_a_placeholder() {
        let decorated = format!("{RESERVE_TIP} (1 new notification)");
        let sample = classify(1, vec![theirs(100, 140, &decorated)]).unwrap();
        assert!(sample.others.is_empty());
    }

    #[test]
    fn a_foreign_icon_dragged_between_two_placeholders_obstructs_the_region() {
        let sample = classify(
            2,
            vec![
                ours(100, 140),
                theirs(140, 180, "Another program"),
                ours(180, 220),
            ],
        )
        .unwrap();
        let mut state = Obstruction::default();
        // Twice, because one sample is the chevron and two is a real drag.
        state.observe(sample.reserved, &sample.others);
        assert!(state.observe(sample.reserved, &sample.others));
    }

    #[test]
    fn a_foreign_icon_beside_the_region_is_not_an_obstruction() {
        let sample = classify(
            2,
            vec![
                theirs(60, 100, "Another program"),
                ours(100, 140),
                ours(140, 180),
            ],
        )
        .unwrap();
        let mut state = Obstruction::default();
        state.observe(sample.reserved, &sample.others);
        assert!(!state.observe(sample.reserved, &sample.others));
    }

    #[test]
    fn an_enumeration_that_sees_nothing_at_all_is_discarded() {
        // The taskbar's right-click menu, which blinds UI Automation
        // completely. The previous answer has to stand.
        assert!(classify(2, Vec::new()).is_none());
    }

    #[test]
    fn placeholders_missing_from_a_working_enumeration_are_reported_honestly() {
        // Other icons came back, so the enumeration worked. Ours really are
        // not there, and the region is simply smaller.
        let sample = classify(2, vec![theirs(60, 100, "Another program")]).unwrap();
        assert!(sample.reserved.is_empty());
        assert_eq!(sample.others, vec![rect(60, 100)]);
    }

    #[test]
    fn one_sweep_publishes_the_region_and_the_number_of_placeholders_that_formed_it() {
        let sample = classify(3, vec![ours(100, 140), ours(140, 180)]).unwrap();
        let snapshot = finish_sample(&mut Obstruction::default(), sample);
        assert!(snapshot.valid);
        assert_eq!(snapshot.found, 2);
        assert_eq!(snapshot.region, rect(100, 180));
        assert!(!snapshot.obstructed);
    }

    #[test]
    fn eight_stable_regions_put_the_periodic_backstop_on_its_slow_interval() {
        let region = rect(100, 180);
        let mut cadence = Cadence::default();
        cadence.observe(true, region);
        for _ in 0..STABLE_COUNT_FOR_SLOW {
            cadence.observe(false, region);
        }
        assert_eq!(cadence.interval, QUERY_INTERVAL_SLOW);

        cadence.observe(true, rect(140, 220));
        assert_eq!(cadence.interval, QUERY_INTERVAL_FAST);
    }
}
