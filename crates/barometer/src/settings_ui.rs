// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The settings window.
//
// A Windows 11 Settings page in shape - a navigation pane, a content pane
// of cards, the user's accent - drawn entirely by hand over GDI, because the
// stock Win32 controls cannot be made to look like that and there is no
// toolkit in the dependency list to do it for us (`windows-sys` is the whole
// of the FFI, by rule). The controls are in `ui.rs`, the panes in `panes.rs`,
// the model in `model.rs`; this file is the window: its thread, its messages,
// and the state machine that turns clicks and keys into changes.
//
// It runs on a thread of its own. The readout's thread ticks once a second,
// samples every module and repaints the strip, and a settings window that
// shared it would stall the taskbar for as long as a menu was open. The two
// threads meet through `Shared`: the window writes the model there after
// every change and raises a flag; the app polls the flag on its tick and
// pushes live readings the other way for the preview. Nothing blocks.
//
// Everything applies as it is changed. There is no OK and no Apply, because
// the strip is one window this program owns outright and there is nothing
// to stage; the preview at the top of the pane is the same picture the
// taskbar will show a second later.

pub mod gdi;
mod icon;
pub mod geometry;
pub mod model;
pub mod panes;
pub mod preview;
pub mod system;
pub mod theme;
pub mod transfer;
pub mod ui;

pub use model::{GpuAdapter, GpuChoice, Model, SensorSource, Snapshot, StripItem};

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use barometer_core::weather::client;
use barometer_core::weather::models::Location;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, GetDC, InvalidateRect, ReleaseDC, SelectObject, SetBkColor,
    SetTextColor, HBRUSH, HDC, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::UI::Controls::{EM_SETSEL, WM_MOUSELEAVE};
use windows_sys::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE,
    TRACKMOUSEEVENT, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT,
    VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, MoveWindow,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetCursor,
    SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, EN_CHANGE, EN_KILLFOCUS, ES_AUTOHSCROLL,
    GWLP_USERDATA, GWLP_WNDPROC, HTCLIENT, IDC_ARROW, IDC_HAND, IDI_APPLICATION, MINMAXINFO, MSG,
    SWP_NOACTIVATE,
    SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETCURSOR, WM_SETFONT,
    WM_SETTINGCHANGE, WM_SIZE, WM_SYSKEYDOWN, WM_THEMECHANGED, WM_TIMER, WNDCLASSW, WS_CAPTION,
    WS_CHILD, WS_CLIPCHILDREN, WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_THICKFRAME,
};

use crate::{lhm_install, pawnio, update};
use gdi::{wide_nul, Canvas, Face, FontCache, TextStyle};
use geometry::{to_px, Rect};
use panes::{ChoiceContext, Effect, LhmState, SearchView, UpdateView, View};
use theme::Theme;
use ui::{clamp_scroll, hit, Element, Id, Interaction, Kind, Pane, NAV_W, RADIUS_SURFACE, SCROLL_STEP};

/// What the two threads share.
struct Shared {
    /// The model as the window last left it.
    model: Mutex<Model>,
    /// Raised by the window on every change, lowered by `take_changes`.
    changed: AtomicBool,
    snapshot: Mutex<Snapshot>,
    /// The window's handle, as an integer so it can cross threads; zero
    /// while there is no window.
    hwnd: AtomicIsize,
    /// A pane the app asked the window to show next, as its index plus one;
    /// zero when nothing is asked. Read when the window is built and on
    /// every show, so the request works whether the window exists yet.
    pane_request: std::sync::atomic::AtomicUsize,
    /// Settings the app changed elsewhere while the window was open - the
    /// weather panel's location row is the one that does it - for the
    /// window to take up in place of what it is holding.
    incoming: Mutex<Option<barometer_core::store::Settings>>,
}

/// The app's handle on the window.
///
/// Opening it starts a thread; dropping this destroys the window and joins
/// that thread. Closing the window with its own close button only hides it,
/// so the thread and everything it has loaded - the font list, the search
/// results - survive to be shown again in an instant.
pub struct SettingsWindow {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl SettingsWindow {
    /// Opens the window in front of everything, editing `model`.
    pub fn open(model: Model) -> SettingsWindow {
        SettingsWindow::open_at(model, Pane::Strip)
    }

    /// Opens the window on a particular pane.
    ///
    /// The panel's gear asks for this: a gear on the weather panel that
    /// lands on the Strip pane sends the user hunting for what they were
    /// just looking at.
    pub fn open_at(model: Model, pane: Pane) -> SettingsWindow {
        let shared = Arc::new(Shared {
            model: Mutex::new(model),
            changed: AtomicBool::new(false),
            snapshot: Mutex::new(Snapshot::default()),
            hwnd: AtomicIsize::new(0),
            pane_request: std::sync::atomic::AtomicUsize::new(pane.index() + 1),
            incoming: Mutex::new(None),
        });
        let thread = thread::Builder::new()
            .name("settings".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(shared)
            })
            .ok();
        SettingsWindow { shared, thread }
    }

    /// Brings the window to the front on a particular pane.
    pub fn show_pane(&mut self, pane: Pane) {
        self.shared.pane_request.store(pane.index() + 1, Ordering::Release);
        self.show();
    }

    /// Takes up settings the app changed somewhere else.
    ///
    /// Without this the window would still be holding the settings as they
    /// were when it opened, and the next thing the user changed in it would
    /// put the old value back - so a location chosen in the weather panel
    /// would come undone the moment anything in the settings was touched.
    pub fn adopt(&self, settings: barometer_core::store::Settings) {
        if let Ok(mut slot) = self.shared.incoming.lock() {
            *slot = Some(settings);
        }
        let hwnd = self.shared.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: a handle the window thread published; posting to a
            // window that has since gone is harmless.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_ADOPT, 0, 0) };
        }
    }

    /// Brings the window to the front, or opens it again if it was closed.
    pub fn show(&mut self) {
        let hwnd = self.shared.hwnd.load(Ordering::Acquire);
        let alive = self.thread.as_ref().is_some_and(|t| !t.is_finished());
        if hwnd != 0 && alive {
            // SAFETY: a handle the window thread published; posting to a
            // window that has since gone is harmless.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_SHOW, 0, 0) };
            return;
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.thread = thread::Builder::new()
            .name("settings".into())
            .spawn({
                let shared = Arc::clone(&self.shared);
                move || run(shared)
            })
            .ok();
    }

    /// The model as the user last changed it, if it changed since the last
    /// call. Polled from the app's tick; never blocks for long.
    pub fn take_changes(&self) -> Option<Model> {
        if !self.shared.changed.swap(false, Ordering::AcqRel) {
            return None;
        }
        self.shared.model.lock().ok().map(|model| model.clone())
    }

    /// Gives the window live readings for the preview, the machine's sensors
    /// for the pin picker, and the rest of what it shows but does not own.
    pub fn publish(&self, snapshot: Snapshot) {
        if let Ok(mut slot) = self.shared.snapshot.lock() {
            *slot = snapshot;
        }
        let hwnd = self.shared.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: as in `show`.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_SNAPSHOT, 0, 0) };
        }
    }

    /// Whether the window exists, shown or hidden.
    pub fn is_open(&self) -> bool {
        self.shared.hwnd.load(Ordering::Acquire) != 0
    }

    /// Destroys the window and waits for its thread.
    pub fn close(&mut self) {
        let hwnd = self.shared.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: as in `show`.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_CLOSE, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        self.close();
    }
}

const CLASS_NAME: &str = "BarometerSettings";
const TITLE: &str = "Barometer";

const DEFAULT_W_DIP: f32 = 880.0;
const DEFAULT_H_DIP: f32 = 640.0;
const MIN_W_DIP: f32 = 760.0;
const MIN_H_DIP: f32 = 560.0;

const WM_APP_SNAPSHOT: u32 = WM_APP + 1;
const WM_APP_SEARCH: u32 = WM_APP + 2;
const WM_APP_INSTALL: u32 = WM_APP + 3;
const WM_APP_UPDATE: u32 = WM_APP + 4;
const WM_APP_SHOW: u32 = WM_APP + 5;
const WM_APP_CLOSE: u32 = WM_APP + 6;
const WM_APP_EDIT_KEY: u32 = WM_APP + 7;
const WM_APP_ADOPT: u32 = WM_APP + 8;

const TIMER_REFRESH: usize = 1;
const TIMER_SEARCH: usize = 2;
/// How often the machine is re-asked about PawnIO while the window is up.
const REFRESH_MS: u32 = 10_000;
/// The pause after typing before a search is sent, so a word costs one
/// request and not one per letter.
const SEARCH_DEBOUNCE_MS: u32 = 300;
/// How far a press has to move before it is a drag rather than a click.
const DRAG_THRESHOLD_DIP: f32 = 4.0;
/// The EDIT child's id in WM_COMMAND.
const EDIT_ID: usize = 1;

const STYLE: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_THICKFRAME | WS_CLIPCHILDREN;

/// The window's thread: build it, pump it, tear it down.
fn run(shared: Arc<Shared>) {
    let class = wide_nul(CLASS_NAME);
    // SAFETY: a zeroed class filled in before registration; registering
    // twice fails harmlessly and the class stands.
    unsafe {
        let mut info: WNDCLASSW = std::mem::zeroed();
        info.style = CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS;
        info.lpfnWndProc = Some(window_proc);
        info.lpszClassName = class.as_ptr();
        info.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        // The application icon, from the executable's own resources, so the
        // title bar and Alt+Tab show Barometer rather than the generic default
        // Windows falls back to for a class that names none. LoadIconW with
        // the module handle and the first icon in it is the same icon the
        // build script embedded; there is no separate copy to keep in step.
        let module = GetModuleHandleW(std::ptr::null());
        info.hIcon = LoadIconW(module, 1 as *const u16);
        if info.hIcon.is_null() {
            info.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        }
        RegisterClassW(&info);
    }

    let dpi = crate::window::taskbar_dpi();
    let scale = dpi as f32 / 96.0;
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: to_px(DEFAULT_W_DIP, scale),
        bottom: to_px(DEFAULT_H_DIP, scale),
    };
    // SAFETY: a rect on the stack.
    unsafe { AdjustWindowRectExForDpi(&mut frame, STYLE, 0, WS_EX_APPWINDOW, dpi) };
    let width = frame.right - frame.left;
    let height = frame.bottom - frame.top;
    let (x, y) = match system::primary_work_area() {
        Some(area) => (
            area.left + ((area.right - area.left) - width) / 2,
            area.top + ((area.bottom - area.top) - height) / 2,
        ),
        None => (64, 64),
    };

    let model = shared.model.lock().map(|m| m.clone()).unwrap_or_else(|_| Model::from_settings(Default::default()));
    let installed = system::installed_families();
    let state = Box::new(WindowState::new(Arc::clone(&shared), model, installed, dpi));

    // SAFETY: the class is registered; the state pointer is handed to the
    // window through lpParam and reclaimed in WM_NCDESTROY.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            class.as_ptr(),
            wide_nul(TITLE).as_ptr(),
            STYLE,
            x,
            y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            Box::into_raw(state) as *const _,
        )
    };
    if hwnd.is_null() {
        return;
    }
    shared.hwnd.store(hwnd as isize, Ordering::Release);
    // SAFETY: a live window on this thread.
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }

    // SAFETY: the standard loop on this thread's queue.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    shared.hwnd.store(0, Ordering::Release);
}

/// The elements as last laid out, in DIPs.
struct Layout {
    /// Window coordinates.
    nav: Vec<Element>,
    /// Content coordinates: the origin is the content pane's top-left before
    /// scrolling.
    content: Vec<Element>,
    content_h: f32,
    /// The content pane, in window coordinates.
    viewport: Rect,
}

struct Dropdown {
    id: Id,
    items: Vec<String>,
    /// The item the keyboard is on, which opens as the item in force.
    highlight: usize,
    /// Window coordinates.
    popup: Rect,
    scroll: f32,
    context: ChoiceContext,
}

struct Drag {
    item: StripItem,
    start_y: f32,
    target: usize,
    active: bool,
}

struct WindowState {
    hwnd: HWND,
    shared: Arc<Shared>,
    model: Model,
    snapshot: Snapshot,
    theme: Theme,
    dpi: u32,
    fonts: FontCache,
    /// The families the picker offers: GDI's list with its weight instances
    /// folded away. The cache keeps the unfolded list to resolve against.
    families: Vec<String>,
    /// The same list *before* folding, so a weight can be checked against the
    /// instance families it actually needs - "Segoe UI Light", "Segoe UI
    /// Semibold". Asking the folded list whether a weight exists always says
    /// no, because folding is what took those names out of it.
    instances: Vec<String>,
    /// Every (family, weight) the machine has, so a Weight control can
    /// offer only the weights the chosen family has faces for.
    faces: Vec<(String, i32)>,
    /// Whether DWM accepted a Mica backdrop for this window.
    ///
    /// When it did, the client area is all frame and the alpha the painter
    /// writes decides what shows through; when it did not - an older build of
    /// Windows - nothing is extended and the painting is simply opaque.
    mica: bool,
    pane: Pane,
    selection: Option<StripItem>,
    scroll: [f32; Pane::ALL.len()],
    hover: Option<Id>,
    pressed: Option<Id>,
    focus: Option<Id>,
    /// Focus rings are shown only after the keyboard has been used, the
    /// way Windows does it; a mouse user never sees one.
    focus_visible: bool,
    layout: Option<Layout>,
    dropdown: Option<Dropdown>,
    drag: Option<Drag>,
    slider_drag: Option<Id>,
    /// Offset from the thumb's top to the grab point, while dragging it.
    scroll_drag: Option<f32>,
    tracking_leave: bool,
    edit: HWND,
    edit_proc: isize,
    edit_brush: HBRUSH,
    editing: Option<Id>,
    /// The library path as typed but not yet committed.
    edit_buffer: String,
    search: SearchView,
    search_generation: u64,
    search_results: Arc<Mutex<Option<(u64, Result<Vec<Location>, String>)>>>,
    lhm: LhmState,
    lhm_shared: Arc<Mutex<Option<LhmState>>>,
    update: UpdateView,
    update_shared: Arc<Mutex<Option<String>>>,
    pawnio: pawnio::Status,
    /// The preview's flip: show the other appearance's taskbar.
    preview_light: bool,
    preview_spans: Vec<(StripItem, f32, f32)>,
    preview_hover: Option<StripItem>,
    version: String,
    /// Whether Barometer's logon task is registered and enabled, read when the
    /// window opens and after every change - never kept in the settings file,
    /// since the user can disable it in Task Scheduler and a stale copy would
    /// then draw a switch that lies.
    starts_at_login: bool,
    /// What the last settings import or export said, under its buttons.
    transfer: Option<String>,
}

/// The pane the app asked for, if it asked, consuming the request.
fn take_pane_request(shared: &Shared) -> Option<Pane> {
    let index = shared.pane_request.swap(0, Ordering::AcqRel).checked_sub(1)?;
    Pane::ALL.get(index).copied()
}

impl WindowState {
    fn new(shared: Arc<Shared>, model: Model, installed: Vec<String>, dpi: u32) -> WindowState {
        let theme = system::current_theme();
        let fonts = FontCache::new(&installed);
        let families = system::families_for_picker(&installed);
        let faces = system::installed_faces();
        let instances = installed;
        let selection = model.order.first().copied();
        let pane = take_pane_request(&shared).unwrap_or(Pane::Strip);
        WindowState {
            hwnd: std::ptr::null_mut(),
            shared,
            model,
            snapshot: Snapshot::default(),
            preview_light: theme.light,
            theme,
            dpi,
            fonts,
            families,
            faces,
            instances,
            pane,
            selection,
            scroll: [0.0; Pane::ALL.len()],
            hover: None,
            pressed: None,
            focus: None,
            focus_visible: false,
            layout: None,
            dropdown: None,
            drag: None,
            slider_drag: None,
            scroll_drag: None,
            tracking_leave: false,
            edit: std::ptr::null_mut(),
            edit_proc: 0,
            edit_brush: std::ptr::null_mut(),
            editing: None,
            edit_buffer: String::new(),
            search: SearchView::default(),
            search_generation: 0,
            search_results: Arc::new(Mutex::new(None)),
            mica: false,
            lhm: LhmState::Idle,
            lhm_shared: Arc::new(Mutex::new(None)),
            update: UpdateView::default(),
            update_shared: Arc::new(Mutex::new(None)),
            pawnio: pawnio::status(),
            preview_spans: Vec::new(),
            preview_hover: None,
            version: update::Version::running().to_string(),
            starts_at_login: crate::startup::is_on(),
            transfer: None,
        }
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// The client area in DIPs.
    fn client(&self) -> (f32, f32) {
        let mut rect: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: a live window and a rect on the stack.
        unsafe { GetClientRect(self.hwnd, &mut rect) };
        let scale = self.scale();
        ((rect.right - rect.left) as f32 / scale, (rect.bottom - rect.top) as f32 / scale)
    }

    fn invalidate(&self) {
        // SAFETY: a live window.
        unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
    }

    /// Something in the model changed: tell the app, relayout, repaint.
    /// Replaces the model with settings changed elsewhere.
    ///
    /// Any edit in progress is ended first and the selection is left where
    /// it is: the panes and the strip items are the same, only their values
    /// have moved. Nothing is committed back - this is the app telling the
    /// window, not the window telling the app - so `changed` stays as it
    /// was and the app does not read its own change back as a new one.
    fn on_adopt(&mut self) {
        let Some(settings) = self.shared.incoming.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        self.end_edit(false);
        self.model = Model::from_settings(settings);
        if let Ok(mut model) = self.shared.model.lock() {
            *model = self.model.clone();
        }
        self.relayout();
    }

    fn commit(&mut self) {
        if let Ok(mut model) = self.shared.model.lock() {
            *model = self.model.clone();
        }
        self.shared.changed.store(true, Ordering::Release);
        self.relayout();
    }

    fn relayout(&mut self) {
        self.layout = None;
        self.invalidate();
    }

    /// Asks DWM for Mica behind the navigation pane, at the current scale.
    ///
    /// Re-applied whenever the scale changes, because the width of the strip
    /// that is declared frame is in physical pixels and the pane it has to
    /// match is in DIPs.
    fn apply_backdrop(&self) -> bool {
        // Past the pane by the content layer's corner radius, so the rounded
        // notches where that layer meets the pane are glass too rather than a
        // pair of solid nicks. Nothing in the light appearance - the frame
        // region blends additively and would wash its labels out.
        if self.theme.light {
            // Zero width, so a strip extended while the dark appearance was
            // on is taken back when the user switches.
            system::apply_backdrop(self.hwnd, 0);
            return false;
        }
        let glass = to_px(ui::NAV_W + ui::RADIUS_SURFACE, self.scale());
        system::apply_backdrop(self.hwnd, glass)
    }

    fn subject(&self) -> Option<StripItem> {
        panes::subject(self.pane, self.selection)
    }

    fn view(&self) -> View<'_> {
        View {
            model: &self.model,
            snapshot: &self.snapshot,
            selection: self.subject(),
            families: &self.families,
            faces: &self.faces,
            instances: &self.instances,
            pawnio: self.pawnio,
            lhm: &self.lhm,
            search: &self.search,
            update: &self.update,
            editing: self.editing,
            drag: self.drag.as_ref().filter(|d| d.active).map(|d| (d.item, d.target)),
            preview_light: self.preview_light,
            version: &self.version,
            starts_at_login: self.starts_at_login,
            transfer: self.transfer.as_deref(),
        }
    }

    /// The width of the widest of some lines, for a popup sized to its items.
    fn widest_line(&mut self, lines: &[String], style: TextStyle) -> f32 {
        // SAFETY: a DC borrowed from the live window and released below.
        let dc = unsafe { GetDC(self.hwnd) };
        let width = {
            let mut canvas = Canvas::new(dc, self.scale(), &mut self.fonts);
            lines.iter().map(|line| canvas.measure(line, style).0).fold(0.0, f32::max)
        };
        // SAFETY: the DC from GetDC above.
        unsafe { ReleaseDC(self.hwnd, dc) };
        width
    }

    /// Lays the panes out if nothing has since they were last laid out.
    fn ensure_layout(&mut self) {
        if self.layout.is_some() {
            return;
        }
        let (width, height) = self.client();
        let scale = self.scale();
        let subject = self.subject();
        // SAFETY: a DC borrowed from the live window and released below.
        let dc = unsafe { GetDC(self.hwnd) };
        let (nav, content, content_h) = {
            let canvas = RefCell::new(Canvas::new(dc, scale, &mut self.fonts));
            let measure = |text: &str, style: TextStyle, wrap: f32| -> (f32, f32) {
                let mut canvas = canvas.borrow_mut();
                if wrap > 0.0 {
                    (wrap, canvas.measure_wrapped(text, style, wrap))
                } else {
                    canvas.measure(text, style)
                }
            };
            let view = View {
                instances: &self.instances,
                model: &self.model,
                snapshot: &self.snapshot,
                selection: subject,
                families: &self.families,
                faces: &self.faces,
                pawnio: self.pawnio,
                lhm: &self.lhm,
                search: &self.search,
                update: &self.update,
                editing: self.editing,
                drag: self.drag.as_ref().filter(|d| d.active).map(|d| (d.item, d.target)),
                preview_light: self.preview_light,
                version: &self.version,
                starts_at_login: self.starts_at_login,
                transfer: self.transfer.as_deref(),
            };
            let nav = panes::nav(height, self.pane, &measure);
            let (content, content_h) = panes::content(self.pane, &view, width - NAV_W, &measure);
            (nav, content, content_h)
        };
        // SAFETY: the DC from GetDC above.
        unsafe { ReleaseDC(self.hwnd, dc) };
        let viewport = Rect::new(NAV_W, 0.0, width - NAV_W, height);
        let pane = self.pane.index();
        self.scroll[pane] = clamp_scroll(self.scroll[pane], viewport.h, content_h);
        self.layout = Some(Layout { nav, content, content_h, viewport });
        self.place_edit();
    }

    fn scroll_here(&self) -> f32 {
        self.scroll[self.pane.index()]
    }

    /// A content element's rectangle in window coordinates.
    fn window_rect(&self, id: Id) -> Option<Rect> {
        let layout = self.layout.as_ref()?;
        if let Some(element) = ui::find(&layout.nav, id) {
            return Some(element.rect);
        }
        ui::find(&layout.content, id).map(|e| e.rect.offset(NAV_W, -self.scroll_here()))
    }

    fn overlay(&self) -> Vec<Element> {
        match &self.dropdown {
            Some(dd) => panes::dropdown_overlay(dd.popup, &dd.items, dd.highlight, dd.scroll),
            None => Vec::new(),
        }
    }

    /// What is under a point in window coordinates.
    fn hit_at(&self, x: f32, y: f32) -> Option<Id> {
        if self.dropdown.is_some() {
            return hit(&self.overlay(), x, y);
        }
        let layout = self.layout.as_ref()?;
        if let Some(id) = hit(&layout.nav, x, y) {
            return Some(id);
        }
        if layout.viewport.contains(x, y) {
            return hit(&layout.content, x - NAV_W, y + self.scroll_here());
        }
        None
    }

    // ---- painting -------------------------------------------------------

    fn paint(&mut self) {
        self.ensure_layout();
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        // SAFETY: BeginPaint/EndPaint bracket the DC for this window.
        let dc = unsafe { BeginPaint(self.hwnd, &mut ps) };
        let mut client: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let (w, h) = (client.right - client.left, client.bottom - client.top);
        if w <= 0 || h <= 0 {
            unsafe { EndPaint(self.hwnd, &ps) };
            return;
        }
        // Into a memory bitmap and out in one blit: the panes are hundreds of
        // small fills and strings, and drawn straight to the window they
        // appear one at a time.
        // SAFETY: GDI objects made and deleted within this function.
        let (memory, bitmap, previous) = unsafe {
            let memory = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, w, h);
            let previous = SelectObject(memory, bitmap as _);
            (memory, bitmap, previous)
        };

        self.paint_into(memory);

        // SAFETY: as above.
        unsafe {
            BitBlt(dc, 0, 0, w, h, memory, 0, 0, SRCCOPY);
            SelectObject(memory, previous);
            DeleteObject(bitmap as _);
            DeleteDC(memory);
            EndPaint(self.hwnd, &ps);
        }
    }

    fn paint_into(&mut self, dc: HDC) {
        let scale = self.scale();
        let (width, height) = self.client();
        let theme = self.theme.clone();
        let Some(layout) = self.layout.as_ref() else { return };
        let scroll = self.scroll[self.pane.index()];
        let interaction = Interaction {
            hover: self.hover,
            pressed: self.pressed,
            focus: self.focus,
            focus_visible: self.focus_visible,
        };
        let preview_light = self.preview_light;
        let preview_hover = self.preview_hover;
        let overlay = self.overlay();
        let model = &self.model;
        let snapshot = &self.snapshot;
        let mut spans: Vec<(StripItem, f32, f32)> = Vec::new();

        let mut canvas = Canvas::new(dc, scale, &mut self.fonts);
        let whole = Rect::new(0.0, 0.0, width, height);
        // Black where the backdrop is to show. The client area is extended
        // into the frame, and inside an extended frame DWM renders pure black
        // as glass - which here is Mica, so the navigation pane is the
        // desktop's own tint and the content layer over it is solid. That is
        // the same division Windows 11's own Settings makes. Without a
        // backdrop the window is painted the ordinary way.
        // Black is glass inside the extended frame, which is the left strip
        // and so the navigation pane; the content layer is painted over the
        // rest of it and is unaffected either way.
        // Black is glass inside the extended frame, which is the left strip
        // and so the navigation pane. `mica` is only ever true in the dark
        // appearance - see `system::apply_backdrop` - so the light one paints
        // its pane the ordinary way.
        let ground = if self.mica { theme::Color::rgb(0, 0, 0) } else { theme.surface_base };
        canvas.fill_rect(whole, ground);
        // The content layer, rounded only where it meets the navigation
        // pane: the other corners run off the window.
        let layer = Rect::new(NAV_W, 0.0, width - NAV_W + RADIUS_SURFACE, height + RADIUS_SURFACE);
        canvas.fill_round(layer, RADIUS_SURFACE, theme.surface_layer);

        ui::draw(&mut canvas, &theme, &layout.nav, interaction, 0.0, 0.0, whole, &mut |_, _| {});

        let clip = canvas.clip(layout.viewport);
        {
            let mut preview = |canvas: &mut Canvas, rect: Rect| {
                spans = preview::paint(canvas, &theme, rect, model, snapshot, preview_light, preview_hover);
            };
            ui::draw(
                &mut canvas,
                &theme,
                &layout.content,
                interaction,
                NAV_W,
                -scroll,
                layout.viewport,
                &mut preview,
            );
        }
        if let Some((y, h)) = ui::scroll_thumb(layout.viewport.h, layout.content_h, scroll) {
            let thumb = Rect::new(width - 8.0, layout.viewport.y + y, 4.0, h);
            let ink = if matches!(self.hover, Some(Id::Scrollbar)) || self.scroll_drag.is_some() {
                theme.stroke_strong
            } else {
                theme.stroke_control_edge
            };
            canvas.fill_round(thumb, 2.0, ink);
        }
        canvas.unclip(clip);

        if !overlay.is_empty() {
            ui::draw(&mut canvas, &theme, &overlay, interaction, 0.0, 0.0, whole, &mut |_, _| {});
        }
        drop(canvas);
        self.preview_spans = spans;
    }

    // ---- the EDIT child -------------------------------------------------

    fn create_edit(&mut self) {
        // SAFETY: a child of the live window; the class is Windows' own.
        unsafe {
            self.edit = CreateWindowExW(
                0,
                wide_nul("EDIT").as_ptr(),
                std::ptr::null(),
                WS_CHILD | ES_AUTOHSCROLL as u32,
                0,
                0,
                10,
                10,
                self.hwnd,
                EDIT_ID as _,
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if self.edit.is_null() {
                return;
            }
            let subclass: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = edit_proc;
            self.edit_proc = SetWindowLongPtrW(self.edit, GWLP_WNDPROC, subclass as *const () as isize);
        }
        self.refresh_edit_font();
        self.refresh_edit_brush();
    }

    fn refresh_edit_font(&mut self) {
        if self.edit.is_null() {
            return;
        }
        let px = to_px(TextStyle::Body.size_dip(), self.scale());
        let font = self.fonts.face(Face::Text, px, 400);
        // SAFETY: a live child; the font lives as long as the cache.
        unsafe { SendMessageW(self.edit, WM_SETFONT, font as usize, 1) };
    }

    fn refresh_edit_brush(&mut self) {
        // SAFETY: a brush this state owns, replaced whole.
        unsafe {
            if !self.edit_brush.is_null() {
                DeleteObject(self.edit_brush as _);
            }
            self.edit_brush = CreateSolidBrush(self.theme.control_rest.colorref());
        }
    }

    /// The text a field holds, from the model.
    fn field_text(&self, id: Id) -> String {
        match id {
            Id::Search => self.search.query.clone(),
            Id::LibraryDir => self.model.settings.library_directory.clone().unwrap_or_default(),
            Id::StackName => match self.selection {
                Some(StripItem::Stack(stack)) => {
                    self.model.stack(stack).map(|s| s.name.clone()).unwrap_or_default()
                }
                _ => String::new(),
            },
            Id::MetricLabel(index) => match self.selection {
                Some(StripItem::Stack(stack)) => self
                    .model
                    .stack(stack)
                    .and_then(|s| s.metrics.get(index))
                    .and_then(|e| e.label.clone())
                    .unwrap_or_default(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    /// Puts the EDIT child over a field and gives it the caret.
    fn begin_edit(&mut self, id: Id) {
        if self.edit.is_null() || self.editing == Some(id) {
            return;
        }
        self.editing = Some(id);
        self.edit_buffer = self.field_text(id);
        let text = wide_nul(&self.edit_buffer);
        self.relayout();
        self.ensure_layout();
        // SAFETY: a live child; the text is terminated.
        unsafe {
            SetWindowTextW(self.edit, text.as_ptr());
            let end = self.edit_buffer.encode_utf16().count();
            SendMessageW(self.edit, EM_SETSEL, end, end as isize);
            ShowWindow(self.edit, SW_SHOW);
            SetFocus(self.edit);
        }
    }

    /// Takes the EDIT away again, keeping what was typed unless told not to.
    fn end_edit(&mut self, keep: bool) {
        let Some(id) = self.editing.take() else { return };
        if keep && id == Id::LibraryDir {
            let path = self.edit_buffer.trim();
            self.model.settings.library_directory = (!path.is_empty()).then(|| path.to_string());
            self.commit();
        }
        // SAFETY: a live child.
        unsafe { ShowWindow(self.edit, SW_HIDE) };
        self.relayout();
    }

    /// Moves the EDIT to wherever its field is now.
    fn place_edit(&self) {
        let Some(id) = self.editing else { return };
        let Some(layout) = self.layout.as_ref() else { return };
        let Some(element) = ui::find(&layout.content, id) else { return };
        let Kind::Field { search, .. } = element.kind else { return };
        let field = element.rect.offset(NAV_W, -self.scroll_here());
        let inner = ui::field_edit_rect(field, search);
        let px = inner.px(self.scale());
        let visible = layout.viewport.contains(field.x, field.y)
            && layout.viewport.contains(field.right() - 1.0, field.bottom() - 1.0);
        // SAFETY: a live child.
        unsafe {
            MoveWindow(self.edit, px.left, px.top, px.width(), px.height(), 1);
            ShowWindow(self.edit, if visible { SW_SHOW } else { SW_HIDE });
        }
    }

    fn read_edit(&self) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
        // SAFETY: a live child; the buffer is one longer than the text.
        unsafe {
            let length = GetWindowTextLengthW(self.edit).max(0) as usize;
            let mut buffer = vec![0u16; length + 1];
            let copied = GetWindowTextW(self.edit, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
            String::from_utf16_lossy(&buffer[..copied])
        }
    }

    /// The EDIT's text changed.
    fn on_edit_changed(&mut self) {
        let text = self.read_edit();
        match self.editing {
            Some(Id::Search) => {
                self.search.query = text;
                self.search.answered = false;
                self.search.error = None;
                // SAFETY: a live window; a one-shot timer replaced on each key.
                unsafe { SetTimer(self.hwnd, TIMER_SEARCH, SEARCH_DEBOUNCE_MS, None) };
            }
            Some(Id::StackName) => {
                if let Some(StripItem::Stack(stack)) = self.selection {
                    self.model.set_stack_name(stack, &text);
                    // The name shows in the list beside the field, live, so
                    // the whole pane relays; the EDIT keeps the caret.
                    self.commit();
                }
            }
            Some(Id::MetricLabel(index)) => {
                if let Some(StripItem::Stack(stack)) = self.selection {
                    self.model.set_metric_label(stack, index, &text);
                    // The preview above shows the label live.
                    self.commit();
                }
            }
            Some(Id::LibraryDir) => self.edit_buffer = text,
            _ => {}
        }
    }

    /// Enter, Escape or Down inside the EDIT.
    fn on_edit_key(&mut self, key: u16) {
        match (self.editing, key) {
            (Some(Id::Search), VK_RETURN) => {
                if let Some(place) = self.search.results.first().cloned() {
                    self.add_place(&place);
                }
            }
            (Some(Id::Search), VK_DOWN) if !self.search.results.is_empty() => {
                self.focus = Some(Id::SearchResult(0));
                self.focus_visible = true;
                // SAFETY: a live window.
                unsafe { SetFocus(self.hwnd) };
            }
            (Some(_), VK_RETURN) => {
                // SAFETY: a live window. Taking focus back commits the field.
                unsafe { SetFocus(self.hwnd) };
            }
            (Some(_), VK_ESCAPE) => {
                self.end_edit(false);
                // SAFETY: a live window.
                unsafe { SetFocus(self.hwnd) };
            }
            (Some(_), VK_TAB) => {
                // The field commits, focus comes back to the window, and the
                // window's own Tab carries on from the control the field
                // belongs to - so tabbing through the pane passes through a
                // text field rather than stopping in it.
                // SAFETY: a live window.
                unsafe { SetFocus(self.hwnd) };
                self.focus_visible = true;
                self.focus_step(WindowState::key_down(VK_SHIFT));
            }
            _ => {}
        }
    }

    // ---- the search worker ---------------------------------------------

    fn start_search(&mut self) {
        let query = self.search.query.trim().to_string();
        if query.chars().count() < 2 {
            self.search.results.clear();
            self.search.pending = false;
            self.relayout();
            return;
        }
        self.search_generation += 1;
        self.search.pending = true;
        self.relayout();
        let generation = self.search_generation;
        let results = Arc::clone(&self.search_results);
        let hwnd = self.hwnd as isize;
        thread::spawn(move || {
            let outcome = client::search(&query).map_err(|why| why.to_string());
            if let Ok(mut slot) = results.lock() {
                *slot = Some((generation, outcome));
            }
            // SAFETY: the handle was live when the search started; posting
            // to a window that has since gone is harmless.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_SEARCH, 0, 0) };
        });
    }

    fn on_search_result(&mut self) {
        let Some((generation, outcome)) = self.search_results.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        // A slower answer to an older query arriving after a newer one
        // would replace the right results with the wrong ones.
        if generation != self.search_generation {
            return;
        }
        self.search.pending = false;
        self.search.answered = true;
        match outcome {
            Ok(found) => {
                self.search.results = found;
                self.search.error = None;
            }
            Err(why) => {
                self.search.results.clear();
                self.search.error = Some(why);
            }
        }
        self.relayout();
    }

    fn add_place(&mut self, place: &Location) {
        panes::add_location(&mut self.model, place);
        self.search = SearchView::default();
        if self.editing == Some(Id::Search) {
            self.end_edit(false);
            // SAFETY: a live window.
            unsafe { SetFocus(self.hwnd) };
        }
        self.commit();
    }

    // ---- the install and update workers ----------------------------------

    /// Deletes the fetched copy of LibreHardwareMonitor.
    ///
    /// It has to be offered, and here is the only place it can be: the library
    /// is unpacked into AppData rather than installed, so it has no entry in
    /// Add or Remove Programs and nothing else on the machine knows it exists.
    /// The uninstaller closes the sensor helper first, so the library is not
    /// held open by the time the files go.
    /// On a thread, like installing, searching and the update check.
    ///
    /// `uninstall` asks the sensor helper to let go and waits up to three
    /// seconds for it before three passes of deleting a directory tree. Run
    /// from the window procedure, that is three seconds of a settings window
    /// Windows has grayed out and labeled "Not Responding" - the one call in
    /// this pane that had been left on the message thread.
    fn remove_library(&mut self) {
        if matches!(self.lhm, LhmState::Installing(_) | LhmState::Removing) {
            return;
        }
        self.lhm = LhmState::Removing;
        self.relayout();
        let shared = Arc::clone(&self.lhm_shared);
        let hwnd = self.hwnd as isize;
        thread::spawn(move || {
            let state = match crate::lhm_install::uninstall() {
                Ok(()) => LhmState::Idle,
                Err(why) => LhmState::Failed(why.to_string()),
            };
            if let Ok(mut slot) = shared.lock() {
                *slot = Some(state);
            }
            // SAFETY: as in the search worker.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_INSTALL, 0, 0) };
        });
    }

    fn start_install(&mut self) {
        if matches!(self.lhm, LhmState::Installing(_)) {
            return;
        }
        self.lhm = LhmState::Installing(lhm_install::Progress::Downloading { received: 0, total: lhm_install::ASSET_BYTES });
        self.relayout();
        let shared = Arc::clone(&self.lhm_shared);
        let hwnd = self.hwnd as isize;
        thread::spawn(move || {
            let report = |state: LhmState| {
                if let Ok(mut slot) = shared.lock() {
                    *slot = Some(state);
                }
                // SAFETY: as in the search worker.
                unsafe { PostMessageW(hwnd as HWND, WM_APP_INSTALL, 0, 0) };
            };
            let progress = |progress: lhm_install::Progress| report(LhmState::Installing(progress));
            match lhm_install::install(Some(&progress)) {
                Ok(path) => report(LhmState::Installed(path.to_string_lossy().into_owned())),
                Err(why) => report(LhmState::Failed(why.to_string())),
            }
        });
    }

    fn on_install_progress(&mut self) {
        let Some(state) = self.lhm_shared.lock().ok().and_then(|mut s| s.take()) else { return };
        if let LhmState::Installed(path) = &state {
            // The path the helper wants is the one the installer chose; the
            // app restarts the helper when it sees this change.
            self.model.settings.library_directory = Some(path.clone());
            self.lhm = state;
            self.commit();
            return;
        }
        self.lhm = state;
        self.relayout();
    }

    fn start_update_check(&mut self) {
        if self.update.checking {
            return;
        }
        self.update.checking = true;
        self.relayout();
        let shared = Arc::clone(&self.update_shared);
        let hwnd = self.hwnd as isize;
        thread::spawn(move || {
            let words = match update::check() {
                Ok(update::Outcome::UpToDate(version)) => format!("Up to date. {version} is the newest release."),
                Ok(update::Outcome::Newer(release)) => {
                    format!("{} is available from the project's releases page.", release.version)
                }
                Err(why) => format!("Couldn't check: {why}"),
            };
            if let Ok(mut slot) = shared.lock() {
                *slot = Some(words);
            }
            // SAFETY: as in the search worker.
            unsafe { PostMessageW(hwnd as HWND, WM_APP_UPDATE, 0, 0) };
        });
    }

    fn on_update_result(&mut self) {
        if let Some(words) = self.update_shared.lock().ok().and_then(|mut s| s.take()) {
            self.update.checking = false;
            self.update.result = Some(words);
            self.relayout();
        }
    }

    // ---- input ------------------------------------------------------------

    fn mouse_dip(&self, lparam: LPARAM) -> (f32, f32) {
        let x = (lparam & 0xFFFF) as u16 as i16 as i32;
        let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
        let scale = self.scale();
        (x as f32 / scale, y as f32 / scale)
    }

    fn set_hover(&mut self, hover: Option<Id>) {
        if self.hover != hover {
            self.hover = hover;
            self.invalidate();
        }
    }

    fn on_mouse_move(&mut self, lparam: LPARAM) {
        if !self.tracking_leave {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: a struct on the stack describing this window.
            unsafe { TrackMouseEvent(&mut track) };
            self.tracking_leave = true;
        }
        let (x, y) = self.mouse_dip(lparam);
        self.ensure_layout();

        if let Some(id) = self.slider_drag {
            self.slide_to(id, x);
            return;
        }
        if let Some(grab) = self.scroll_drag {
            let Some(layout) = self.layout.as_ref() else { return };
            if let Some((_, thumb_h)) = ui::scroll_thumb(layout.viewport.h, layout.content_h, self.scroll_here()) {
                let travel = (layout.viewport.h - thumb_h).max(1.0);
                let fraction = ((y - grab) / travel).clamp(0.0, 1.0);
                let scroll = fraction * (layout.content_h - layout.viewport.h);
                self.set_scroll(scroll);
            }
            return;
        }
        if let Some(drag) = self.drag.as_mut() {
            if !drag.active && (y - drag.start_y).abs() >= DRAG_THRESHOLD_DIP {
                drag.active = true;
            }
            if drag.active {
                let content_y = y + self.scroll_here();
                let rows = self.layout.as_ref().map(|l| panes::composer_rows(&l.content)).unwrap_or_default();
                let target = panes::drop_index(&rows, content_y);
                let drag = self.drag.as_mut().expect("checked above");
                if drag.target != target {
                    drag.target = target;
                    self.relayout();
                }
            }
            return;
        }

        let hover = self.hit_at(x, y);
        let preview_hover = if hover == Some(Id::Preview) {
            preview::item_at(&self.preview_spans, x)
        } else {
            None
        };
        if preview_hover != self.preview_hover {
            self.preview_hover = preview_hover;
            self.invalidate();
        }
        if let Some(dd) = self.dropdown.as_mut() {
            if let Some(Id::DropdownItem(index)) = hover {
                if dd.highlight != index {
                    dd.highlight = index;
                    self.invalidate();
                }
            }
        }
        self.set_hover(hover);
    }

    fn on_mouse_leave(&mut self) {
        self.tracking_leave = false;
        self.set_hover(None);
        if self.preview_hover.take().is_some() {
            self.invalidate();
        }
    }

    fn on_button_down(&mut self, lparam: LPARAM) {
        let (x, y) = self.mouse_dip(lparam);
        self.ensure_layout();
        self.focus_visible = false;

        if let Some(dd) = self.dropdown.as_ref() {
            match hit(&self.overlay(), x, y) {
                Some(Id::DropdownItem(index)) => self.pick(index),
                Some(_) => {}
                // A click anywhere else closes the list and goes no further,
                // which is how every Windows popup behaves.
                None => {
                    let _ = dd;
                    self.close_dropdown();
                }
            }
            return;
        }

        let target = self.hit_at(x, y);
        match target {
            Some(id @ (Id::Search | Id::StackName | Id::LibraryDir | Id::MetricLabel(_))) => {
                self.begin_edit(id);
                return;
            }
            _ => {
                // SAFETY: a live window. Focus leaving the EDIT commits it.
                unsafe { SetFocus(self.hwnd) };
            }
        }
        self.pressed = target;
        match target {
            Some(id @ (Id::Size | Id::Gap | Id::PollSeconds | Id::RefreshMinutes)) => {
                self.slider_drag = Some(id);
                // SAFETY: a live window.
                unsafe { SetCapture(self.hwnd) };
                self.slide_to(id, x);
            }
            Some(Id::Row(item)) => {
                self.selection = Some(item);
                self.focus = Some(Id::Row(item));
                let target = self.model.position(item).unwrap_or(0);
                self.drag = Some(Drag { item, start_y: y, target, active: false });
                // SAFETY: a live window.
                unsafe { SetCapture(self.hwnd) };
                self.relayout();
            }
            Some(Id::Scrollbar) => {}
            Some(Id::Preview) => {
                if let Some(item) = preview::item_at(&self.preview_spans, x) {
                    self.selection = Some(item);
                    self.pane = Pane::Strip;
                    self.relayout();
                }
            }
            _ => {
                if self.on_scrollbar(x, y) {
                    let Some(layout) = self.layout.as_ref() else { return };
                    if let Some((thumb_y, thumb_h)) =
                        ui::scroll_thumb(layout.viewport.h, layout.content_h, self.scroll_here())
                    {
                        let grab = if y >= thumb_y && y < thumb_y + thumb_h { y - thumb_y } else { thumb_h / 2.0 };
                        self.scroll_drag = Some(grab);
                        // SAFETY: a live window.
                        unsafe { SetCapture(self.hwnd) };
                        self.on_mouse_move(lparam);
                    }
                }
            }
        }
        self.invalidate();
    }

    /// Whether a point is on the scrollbar's track.
    fn on_scrollbar(&self, x: f32, y: f32) -> bool {
        let Some(layout) = self.layout.as_ref() else { return false };
        let (width, _) = self.client();
        layout.content_h > layout.viewport.h && x >= width - 12.0 && layout.viewport.contains(x, y)
    }

    fn on_button_up(&mut self, lparam: LPARAM) {
        let (x, y) = self.mouse_dip(lparam);
        // SAFETY: a live window; releasing an uncaptured mouse is harmless.
        unsafe { ReleaseCapture() };
        self.slider_drag = None;
        self.scroll_drag = None;
        if let Some(drag) = self.drag.take() {
            if drag.active {
                if let Some(from) = self.model.position(drag.item) {
                    self.model.move_item(from, drag.target);
                }
                self.commit();
            }
        }
        let pressed = self.pressed.take();
        if pressed.is_some() && pressed == self.hit_at(x, y) {
            if let Some(id) = pressed {
                self.activate(id);
            }
        }
        self.invalidate();
    }

    fn on_wheel(&mut self, wparam: WPARAM) {
        let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
        if let Some(dd) = self.dropdown.as_mut() {
            let content = dd.items.len() as f32 * ui::POPUP_ITEM_H + 8.0;
            dd.scroll = clamp_scroll(dd.scroll - delta * SCROLL_STEP, dd.popup.h, content);
            self.invalidate();
            return;
        }
        let scroll = self.scroll_here() - delta * SCROLL_STEP;
        self.set_scroll(scroll);
    }

    fn set_scroll(&mut self, scroll: f32) {
        let Some(layout) = self.layout.as_ref() else { return };
        let clamped = clamp_scroll(scroll, layout.viewport.h, layout.content_h);
        let pane = self.pane.index();
        if (self.scroll[pane] - clamped).abs() > f32::EPSILON {
            self.scroll[pane] = clamped;
            self.place_edit();
            self.invalidate();
        }
    }

    /// Scrolls so that an element is inside the viewport.
    fn reveal(&mut self, id: Id) {
        let Some(layout) = self.layout.as_ref() else { return };
        let Some(element) = ui::find(&layout.content, id) else { return };
        let rect = element.rect;
        let top = rect.y - 16.0;
        let bottom = rect.bottom() + 16.0;
        let scroll = self.scroll_here();
        if top < scroll {
            self.set_scroll(top);
        } else if bottom > scroll + layout.viewport.h {
            self.set_scroll(bottom - layout.viewport.h);
        }
    }

    fn slide_to(&mut self, id: Id, x: f32) {
        let Some(rect) = self.window_rect(id) else { return };
        let Some(layout) = self.layout.as_ref() else { return };
        let Some(element) = ui::find(&layout.content, id) else { return };
        let Kind::Slider { min, max, step, value } = element.kind else { return };
        let new = ui::slider_value_at(rect, x, min, max, step);
        if (new - value).abs() > f32::EPSILON {
            panes::slide(&mut self.model, id, new);
            self.commit();
        }
    }

    fn open_dropdown(&mut self, id: Id) {
        self.ensure_layout();
        let Some(anchor) = self.window_rect(id) else { return };
        let (_, height) = self.client();
        let (items, selected, context) = {
            let view = self.view();
            let (items, selected) = panes::dropdown_items(&view, id);
            (items, selected, ChoiceContext::capture(&view))
        };
        if items.is_empty() {
            return;
        }
        // The list is as wide as its widest item: the item's 4 inset and
        // 12 text padding on each side, plus the widest line.
        let items_w = self.widest_line(&items, TextStyle::Body) + 2.0 * (4.0 + 12.0);
        let popup = ui::popup_rect(anchor, items.len(), height, items_w);
        let highlight = if selected < items.len() { selected } else { 0 };
        // Open on the current item, scrolled into view.
        let content = items.len() as f32 * ui::POPUP_ITEM_H + 8.0;
        let scroll = clamp_scroll(highlight as f32 * ui::POPUP_ITEM_H - popup.h / 2.0, popup.h, content);
        self.dropdown = Some(Dropdown { id, items, highlight, popup, scroll, context });
        self.focus = Some(id);
        self.invalidate();
    }

    fn close_dropdown(&mut self) {
        if self.dropdown.take().is_some() {
            self.invalidate();
        }
    }

    /// Chooses an item of the open dropdown.
    fn pick(&mut self, index: usize) {
        let Some(dd) = self.dropdown.take() else { return };
        let effect = panes::choose(&mut self.model, &dd.context, dd.id, index);
        if let Effect::Select(item) = effect {
            self.show_item(item);
        }
        self.commit();
    }

    /// Selects a strip item and goes to the page it is edited on.
    ///
    /// Every way of reaching an item goes through here - a row in the Strip
    /// list, the "Edit" link beside a stack, making a stack, putting a
    /// module's reading into one. Before the panes were split, selecting was
    /// enough because the editor was on whatever pane you were already on;
    /// now it is somewhere, and not going there leaves a control that looks
    /// like it did nothing.
    fn show_item(&mut self, item: StripItem) {
        self.selection = Some(item);
        self.focus = Some(Id::Row(item));
        let page = match item {
            StripItem::Module(id) => Pane::Module(id),
            StripItem::Stack(_) => Pane::Stacks,
        };
        if self.pane != page {
            self.end_edit(true);
            self.pane = page;
        }
    }

    /// What a control does when it is used.
    fn activate(&mut self, id: Id) {
        match id {
            Id::None | Id::Preview | Id::Scrollbar => {}
            Id::Nav(pane) => {
                if self.pane != pane {
                    self.end_edit(true);
                    self.pane = pane;
                    self.relayout();
                }
            }
            Id::Row(item) => {
                // The Strip list is the map: a row opens the page its item's
                // options live on.
                self.show_item(item);
                self.relayout();
            }
            Id::RowToggle(_)
            | Id::Show
            | Id::UploadFirst
            | Id::ShowPublicIp
            | Id::HideSources
            | Id::CurrentLocation
            | Id::CheckUpdates => {
                let subject = self.subject();
                panes::toggle(&mut self.model, subject, id);
                self.commit();
            }
            Id::AddStack => {
                let stack = self.model.add_stack(None);
                self.show_item(StripItem::Stack(stack));
                self.commit();
            }
            Id::Family
            | Id::HeadingFamily
            | Id::HeadingWeight
            | Id::Weight
            | Id::GpuAdapter
            | Id::NetworkInterface
            | Id::RateUnit
            | Id::DiskVolume
            | Id::DiskDevice
            | Id::PinnedSensor
            | Id::SensorUnit
            | Id::SensorDecimals
            | Id::Layout
            | Id::AddMetric
            | Id::AddToStack
            | Id::TempUnit
            | Id::WindUnit
            | Id::PressureUnit
            | Id::PrecipUnit => self.open_dropdown(id),
            Id::DropdownItem(index) => self.pick(index),
            Id::MetricUp(index) => {
                if let Some(StripItem::Stack(stack)) = self.selection {
                    self.model.move_metric(stack, index, index.saturating_sub(1));
                    self.commit();
                }
            }
            Id::MetricDown(index) => {
                if let Some(StripItem::Stack(stack)) = self.selection {
                    self.model.move_metric(stack, index, index + 1);
                    self.commit();
                }
            }
            Id::MetricRemove(index) => {
                if let Some(StripItem::Stack(stack)) = self.selection {
                    self.model.remove_metric(stack, index);
                    self.commit();
                }
            }
            Id::RemoveStack => {
                if let Some(StripItem::Stack(stack)) = self.selection {
                    self.end_edit(false);
                    self.model.remove_stack(stack);
                    self.selection = self.model.order.first().copied();
                    self.commit();
                }
            }
            Id::EditStack(stack) => {
                // "Edit" is a link to somewhere, and since the panes were
                // split that somewhere is the Stacks page.
                self.show_item(StripItem::Stack(stack));
                self.relayout();
            }
            Id::InstallLhm => self.start_install(),
            Id::RemoveLhm => self.remove_library(),
            Id::GetPawnIo => system::open_url(panes::PAWNIO_URL),
            Id::Link(url) => system::open_url(url),
            Id::Search | Id::StackName | Id::LibraryDir | Id::MetricLabel(_) => self.begin_edit(id),
            Id::SearchResult(index) => {
                if let Some(place) = self.search.results.get(index).cloned() {
                    self.add_place(&place);
                }
            }
            Id::LocationRemove(index) => {
                panes::remove_location(&mut self.model, index);
                self.commit();
            }
            Id::LocationPrimary(index) => {
                panes::set_primary_location(&mut self.model, index);
                self.commit();
            }
            Id::CheckNow => self.start_update_check(),
            Id::ExportSettings => {
                self.transfer = transfer::export(self.hwnd, &self.model.to_settings());
                self.relayout();
            }
            Id::ImportSettings => match transfer::import(self.hwnd) {
                Some(Ok(settings)) => {
                    // Replaced whole, then committed like any other change,
                    // so the strip follows and the app's own file is written
                    // from it - which is what makes the import stick.
                    self.model = model::Model::from_settings(settings);
                    self.transfer = Some("Imported. The strip is showing what the file said.".to_string());
                    self.commit();
                }
                Some(Err(why)) => {
                    self.transfer = Some(why);
                    self.relayout();
                }
                None => {}
            },
            Id::StartAtLogin => {
                // Asked, then read back: a machine that refuses the write -
                // policy, a locked-down profile - leaves the switch showing
                // what is actually so rather than what was pressed.
                crate::startup::set(!self.starts_at_login);
                self.starts_at_login = crate::startup::is_on();
                self.relayout();
            }
            Id::ClearSkipped => {
                self.model.settings.skipped_update = None;
                self.commit();
            }
            Id::PreviewFlip => {
                self.preview_light = !self.preview_light;
                self.invalidate();
            }
            Id::Size | Id::Gap | Id::PollSeconds | Id::RefreshMinutes => {}
        }
    }

    // ---- keyboard ---------------------------------------------------------

    fn key_down(key: u16) -> bool {
        // SAFETY: reads key state; no side effects.
        unsafe { GetKeyState(key as i32) < 0 }
    }

    /// Every element the keyboard can land on, in reading order.
    fn focus_order(&self) -> Vec<Id> {
        let Some(layout) = self.layout.as_ref() else { return Vec::new() };
        let mut order: Vec<Id> = Vec::new();
        for element in layout.nav.iter().chain(&layout.content) {
            if element.focusable && element.interactive && !order.contains(&element.id) {
                order.push(element.id);
            }
        }
        order
    }

    fn focus_step(&mut self, backwards: bool) {
        let order = self.focus_order();
        if order.is_empty() {
            return;
        }
        let current = self.focus.and_then(|id| order.iter().position(|o| *o == id));
        let next = match (current, backwards) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(i), false) => (i + 1) % order.len(),
            (Some(i), true) => (i + order.len() - 1) % order.len(),
        };
        self.focus = Some(order[next]);
        self.focus_visible = true;
        self.reveal(order[next]);
        self.invalidate();
    }

    fn on_key(&mut self, key: u16, alt: bool) -> bool {
        let ctrl = WindowState::key_down(VK_CONTROL);
        let shift = WindowState::key_down(VK_SHIFT);

        if let Some(dd) = self.dropdown.as_mut() {
            match key {
                VK_ESCAPE => self.close_dropdown(),
                VK_RETURN | VK_SPACE => {
                    let index = dd.highlight;
                    self.pick(index);
                }
                VK_UP | VK_DOWN => {
                    let count = dd.items.len();
                    dd.highlight = if key == VK_UP {
                        dd.highlight.saturating_sub(1)
                    } else {
                        (dd.highlight + 1).min(count.saturating_sub(1))
                    };
                    let content = count as f32 * ui::POPUP_ITEM_H + 8.0;
                    let top = dd.highlight as f32 * ui::POPUP_ITEM_H;
                    if top < dd.scroll {
                        dd.scroll = top;
                    } else if top + ui::POPUP_ITEM_H > dd.scroll + dd.popup.h {
                        dd.scroll = top + ui::POPUP_ITEM_H - dd.popup.h + 8.0;
                    }
                    dd.scroll = clamp_scroll(dd.scroll, dd.popup.h, content);
                    self.invalidate();
                }
                _ => return false,
            }
            return true;
        }

        match key {
            VK_ESCAPE => {
                // SAFETY: a live window.
                unsafe { ShowWindow(self.hwnd, SW_HIDE) };
                true
            }
            VK_TAB if ctrl => {
                let index = self.pane.index();
                // Around every pane there is, not the four there were when
                // this was written: Weather and About were both added after.
                let panes = Pane::ALL.len();
                let next = if shift { (index + panes - 1) % panes } else { (index + 1) % panes };
                self.activate(Id::Nav(Pane::ALL[next]));
                true
            }
            VK_TAB => {
                self.focus_step(shift);
                true
            }
            VK_SPACE | VK_RETURN => {
                let Some(id) = self.focus else { return false };
                self.focus_visible = true;
                self.activate(id);
                true
            }
            VK_UP | VK_DOWN => {
                let Some(id) = self.focus else {
                    self.set_scroll(self.scroll_here() + if key == VK_UP { -SCROLL_STEP } else { SCROLL_STEP });
                    return true;
                };
                self.focus_visible = true;
                match id {
                    Id::Row(item) if alt => {
                        if let Some(from) = self.model.position(item) {
                            let to = if key == VK_UP { from.saturating_sub(1) } else { from + 1 };
                            self.model.move_item(from, to);
                            self.commit();
                        }
                    }
                    Id::Row(item) => {
                        if let Some(index) = self.model.position(item) {
                            let next = if key == VK_UP { index.saturating_sub(1) } else { (index + 1).min(self.model.order.len() - 1) };
                            let target = self.model.order[next];
                            self.selection = Some(target);
                            self.focus = Some(Id::Row(target));
                            self.relayout();
                            self.ensure_layout();
                            self.reveal(Id::Row(target));
                        }
                    }
                    Id::Nav(pane) => {
                        let index = pane.index();
                        let next =
                            if key == VK_UP { index.saturating_sub(1) } else { (index + 1).min(Pane::ALL.len() - 1) };
                        self.focus = Some(Id::Nav(Pane::ALL[next]));
                        self.activate(Id::Nav(Pane::ALL[next]));
                    }
                    Id::Family
                    | Id::HeadingFamily
                    | Id::HeadingWeight
                    | Id::Weight
                    | Id::GpuAdapter
                    | Id::NetworkInterface
                    | Id::RateUnit
                    | Id::DiskVolume
                    | Id::DiskDevice
                    | Id::PinnedSensor
                    | Id::SensorUnit
                    | Id::SensorDecimals
                    | Id::Layout
                    | Id::AddMetric
                    | Id::AddToStack
                    | Id::TempUnit
                    | Id::WindUnit
                    | Id::PressureUnit
                    | Id::PrecipUnit => self.open_dropdown(id),
                    _ => self.set_scroll(self.scroll_here() + if key == VK_UP { -SCROLL_STEP } else { SCROLL_STEP }),
                }
                true
            }
            VK_LEFT | VK_RIGHT | VK_PRIOR | VK_NEXT | VK_HOME | VK_END => {
                let Some(id) = self.focus else { return false };
                let Some(layout) = self.layout.as_ref() else { return false };
                let Some(element) = ui::find(&layout.content, id) else { return false };
                let Kind::Slider { min, max, step, value } = element.kind else { return false };
                let new = match key {
                    VK_LEFT => value - step,
                    VK_RIGHT => value + step,
                    VK_PRIOR => value - step * 5.0,
                    VK_NEXT => value + step * 5.0,
                    VK_HOME => min,
                    _ => max,
                }
                .clamp(min, max);
                self.focus_visible = true;
                panes::slide(&mut self.model, id, new);
                self.commit();
                true
            }
            VK_DELETE => match self.focus {
                Some(Id::Row(StripItem::Stack(stack))) => {
                    self.selection = Some(StripItem::Stack(stack));
                    self.activate(Id::RemoveStack);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    // ---- environment ------------------------------------------------------

    fn on_theme_changed(&mut self) {
        let theme = system::current_theme();
        if theme == self.theme {
            return;
        }
        // The preview follows the appearance unless it was flipped away
        // from the old one, in which case it keeps showing the other side.
        let was_flipped = self.preview_light != self.theme.light;
        self.theme = theme;
        self.preview_light = if was_flipped { !self.theme.light } else { self.theme.light };
        system::apply_chrome(self.hwnd, &self.theme);
        self.mica = self.apply_backdrop();
        self.refresh_edit_brush();
        self.invalidate();
    }

    fn on_dpi_changed(&mut self, dpi: u32, suggested: &RECT) {
        self.dpi = dpi.max(96);
        // The glass strip is in physical pixels and the pane it has to match
        // is in DIPs, so the two part company on every scale change.
        self.mica = self.apply_backdrop();
        // SAFETY: a live window and the rect Windows suggested.
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.fonts.clear();
        self.refresh_edit_font();
        self.relayout();
    }

    fn on_snapshot(&mut self) {
        let Some(snapshot) = self.shared.snapshot.lock().ok().map(|s| s.clone()) else { return };
        if snapshot != self.snapshot {
            self.snapshot = snapshot;
            self.relayout();
        }
    }

    fn on_refresh_timer(&mut self) {
        let status = pawnio::status();
        if status != self.pawnio {
            self.pawnio = status;
            self.relayout();
        }
    }

    fn min_size_px(&self) -> (i32, i32) {
        let scale = self.scale();
        let mut frame = RECT { left: 0, top: 0, right: to_px(MIN_W_DIP, scale), bottom: to_px(MIN_H_DIP, scale) };
        // SAFETY: a rect on the stack.
        unsafe { AdjustWindowRectExForDpi(&mut frame, STYLE, 0, WS_EX_APPWINDOW, self.dpi) };
        (frame.right - frame.left, frame.bottom - frame.top)
    }
}

impl Drop for WindowState {
    fn drop(&mut self) {
        // SAFETY: a brush this state made, if it made one.
        unsafe {
            if !self.edit_brush.is_null() {
                DeleteObject(self.edit_brush as _);
            }
        }
    }
}

/// Catches the keys the EDIT would otherwise swallow or beep at.
unsafe extern "system" fn edit_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
    let parent = GetParent(hwnd);
    let state = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut WindowState;
    if state.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let key = wparam as u16;
    match message {
        // Tab among them: an EDIT does nothing with it, and this window
        // runs its own focus ring rather than IsDialogMessage, so a keyboard
        // user who tabbed into the city search could leave it only with
        // Return, Escape or the mouse.
        WM_KEYDOWN
            if key == VK_RETURN || key == VK_ESCAPE || key == VK_DOWN || key == VK_TAB =>
        {
            PostMessageW(parent, WM_APP_EDIT_KEY, wparam, 0);
            0
        }
        // The EDIT beeps at a key it cannot use; there is nothing to beep
        // about, and Tab is one of them.
        WM_CHAR if key == VK_RETURN || key == VK_ESCAPE || key == VK_TAB => 0,
        _ => {
            let previous = (*state).edit_proc;
            if previous == 0 {
                DefWindowProcW(hwnd, message, wparam, lparam)
            } else {
                CallWindowProcW(Some(std::mem::transmute(previous)), hwnd, message, wparam, lparam)
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        if !create.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
        }
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
    if state.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = &mut *state;

    match message {
        WM_CREATE => {
            state.hwnd = hwnd;
            system::apply_chrome(hwnd, &state.theme);
            state.mica = state.apply_backdrop();
            system::adopt_executable_icon(hwnd);
            state.create_edit();
            SetTimer(hwnd, TIMER_REFRESH, REFRESH_MS, None);
            0
        }
        WM_PAINT => {
            state.paint();
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            state.relayout();
            0
        }
        WM_GETMINMAXINFO => {
            let info = lparam as *mut MINMAXINFO;
            if !info.is_null() && !state.hwnd.is_null() {
                let (w, h) = state.min_size_px();
                (*info).ptMinTrackSize = POINT { x: w, y: h };
            }
            0
        }
        WM_DPICHANGED => {
            let dpi = (wparam & 0xFFFF) as u32;
            let suggested = lparam as *const RECT;
            if !suggested.is_null() {
                state.on_dpi_changed(dpi, &*suggested);
            }
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            state.on_theme_changed();
            0
        }
        WM_MOUSEMOVE => {
            state.on_mouse_move(lparam);
            0
        }
        WM_MOUSELEAVE => {
            state.on_mouse_leave();
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            state.on_button_down(lparam);
            0
        }
        WM_LBUTTONUP => {
            state.on_button_up(lparam);
            0
        }
        WM_MOUSEWHEEL => {
            state.on_wheel(wparam);
            0
        }
        WM_KEYDOWN => {
            if state.on_key(wparam as u16, false) {
                0
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_SYSKEYDOWN => {
            let alt = WindowState::key_down(VK_MENU);
            if alt && state.on_key(wparam as u16, true) {
                0
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_COMMAND => {
            let id = wparam & 0xFFFF;
            let code = (wparam >> 16) as u32;
            if id == EDIT_ID {
                match code {
                    EN_CHANGE => state.on_edit_changed(),
                    EN_KILLFOCUS => state.end_edit(true),
                    _ => {}
                }
            }
            0
        }
        WM_CTLCOLOREDIT => {
            let dc = wparam as HDC;
            SetTextColor(dc, state.theme.text_primary.colorref());
            SetBkColor(dc, state.theme.control_rest.colorref());
            state.edit_brush as LRESULT
        }
        WM_SETCURSOR => {
            if (lparam & 0xFFFF) as u32 == HTCLIENT {
                let hand = matches!(
                    state.hover,
                    Some(Id::Link(_))
                        | Some(Id::EditStack(_))
                        | Some(Id::RemoveStack)
                        | Some(Id::LocationPrimary(_))
                        | Some(Id::ClearSkipped)
                );
                SetCursor(LoadCursorW(std::ptr::null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
                1
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_TIMER => {
            match wparam {
                TIMER_REFRESH => state.on_refresh_timer(),
                TIMER_SEARCH => {
                    KillTimer(hwnd, TIMER_SEARCH);
                    state.start_search();
                }
                _ => {}
            }
            0
        }
        WM_ACTIVATE => {
            if wparam & 0xFFFF == 0 {
                state.close_dropdown();
            }
            0
        }
        WM_APP_SNAPSHOT => {
            state.on_snapshot();
            0
        }
        WM_APP_ADOPT => {
            state.on_adopt();
            0
        }
        WM_APP_SEARCH => {
            state.on_search_result();
            0
        }
        WM_APP_INSTALL => {
            state.on_install_progress();
            0
        }
        WM_APP_UPDATE => {
            state.on_update_result();
            0
        }
        WM_APP_EDIT_KEY => {
            state.on_edit_key(wparam as u16);
            0
        }
        WM_APP_SHOW => {
            if let Some(pane) = take_pane_request(&state.shared) {
                state.pane = pane;
                state.relayout();
            }
            ShowWindow(hwnd, SW_RESTORE);
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            0
        }
        WM_APP_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_CLOSE => {
            // Hidden, not destroyed: the app keeps running in the strip and
            // the window comes back in an instant. Nothing needs saving,
            // because nothing is pending.
            state.end_edit(true);
            state.close_dropdown();
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_REFRESH);
            KillTimer(hwnd, TIMER_SEARCH);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            let state = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut WindowState;
            if !state.is_null() {
                drop(Box::from_raw(state));
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
