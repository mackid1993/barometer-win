// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The panel window, ported from AttachedPanel.swift and the parts of
// DropdownController.swift that open, size and close it.
//
// A top-level popup rather than a child of the taskbar, for the reason
// window.rs gives at length: the taskbar composites its children away, and
// a child cannot be layered. Topmost so it sits over whatever the user has
// open; a tool window so it never appears in Alt+Tab; layered so its alpha
// can be ramped for the open and close (docs/ui-design.md section 7), with
// DWM asked for the rounded corners and the shadow Windows 11 gives its own
// flyouts. Everything inside is drawn by hand into a memory bitmap, as the
// settings window does, with the same fonts.
//
// It is a foreground window while open. That is what makes Escape work and
// what makes dismissal free: when the user goes anywhere else the window
// is deactivated, and WA_INACTIVE is the signal. The one place activation
// does not change is a click on the taskbar itself, which never takes
// focus, so a short timer also watches for a button going down outside the
// panel - the same fifty-millisecond poll PopoverDismissalMonitor runs, put
// to the narrower use dismiss.rs describes.

use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use barometer_core::stack::StripItem;
use barometer_core::taskbar;
use barometer_core::ModuleId;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EndPaint, GetDC, GetMonitorInfoW, InvalidateRect, MonitorFromRect, ReleaseDC, ScreenToClient,
    SelectObject, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LBUTTON, VK_MBUTTON, VK_NEXT, VK_PRIOR, VK_RBUTTON,
    VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    GetCursorPos, GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, PostQuitMessage,
    RegisterClassW, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW,
    TranslateMessage, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HTCLIENT,
    HWND_TOPMOST, IDC_ARROW, IDC_HAND, LWA_ALPHA, MSG, SPI_GETCLIENTAREAANIMATION, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, WA_INACTIVE, WM_ACTIVATE,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETCURSOR, WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::dismiss::Monitor;
use super::paint::{self, Surface};
use super::placement;
use super::ui::{
    self, Accent, Element, Id, Kind, Palette, Style, BUTTON_H, FOOTER_H, ICON_BUTTON, MAX_PANEL_H,
    PANEL_PAD, PANEL_W,
};
use super::{
    Action, Anchor, Command, Content, Context, Request, Response, Shared, Wake, WM_APP_CHANGED,
    WM_APP_CLOSE, WM_APP_OPEN, WM_APP_QUIT, WM_APP_REGISTER,
};
use crate::settings_ui::gdi::{wide_nul, Canvas, FontCache};
use crate::settings_ui::geometry::{to_px, PxRect, Rect};
use crate::settings_ui::system;
use crate::settings_ui::ui::{clamp_scroll, scroll_thumb};

const CLASS_NAME: &str = "BarometerFlyout";

/// The gear in the footer: Settings, from the table in docs/ui-design.md
/// section 6. theme.rs's table is the settings window's and has no entry for
/// it, since a settings window needs no button to open itself.
const SETTINGS_GLYPH: &str = "\u{E713}";

const TIMER_DISMISS: usize = 1;
const TIMER_FADE: usize = 2;
const TIMER_TICK: usize = 3;
/// Relaxes the elastic overscroll - see `stretch`.
const TIMER_STRETCH: usize = 4;
/// The Mac monitor's cadence, for the press-outside watch.
const DISMISS_MS: u32 = 50;
const FADE_MS: u32 = 10;
/// How often a content is asked whether time alone changed what it shows.
const TICK_MS: u32 = 5_000;
/// docs/ui-design.md section 7: open in 150 with an 8 DIP slide, close in 100.
const OPEN: Duration = Duration::from_millis(150);
const CLOSE: Duration = Duration::from_millis(100);
const SLIDE_DIP: f32 = 8.0;
const SCROLL_STEP: f32 = 48.0;

/// The stretch after pulling `by` further from `current`.
///
/// The resistance is the usual hyperbolic one: the first pixels come almost
/// free and each one after costs more, so the edge is soft at first touch and
/// firm if leaned on. `STRETCH_MAX` is an asymptote rather than a clamp,
/// which is why the page never hits a second hard stop after the first.
///
/// The sum is done in the un-resisted domain - the pull is undone, added to,
/// and redone - so that repeated notches keep having a smaller effect instead
/// of each one starting the curve again, and so that pulling and then pushing
/// back the same distance lands where it started.
fn stretched(current: f32, by: f32) -> f32 {
    let raw = STRETCH_MAX * current / (STRETCH_MAX - current.abs()).max(f32::EPSILON);
    let raw = raw + by;
    STRETCH_MAX * raw / (STRETCH_MAX + raw.abs())
}

/// How far past its end a page can be pulled, in DIPs.
///
/// The stretch never reaches this: the resistance below is asymptotic, so
/// this is the limit an infinitely determined scroll approaches rather than a
/// distance anybody will see. About a row and a half, which is enough to read
/// as give and not so much that the page looks detached.
const STRETCH_MAX: f32 = 56.0;

/// What fraction of the stretch is left after each further 10 ms.
///
/// Applied against the clock rather than once per timer message. A panel
/// repaint is not free - a chart is a few hundred GDI+ primitives - so at a
/// 60-per-second timer the messages coalesce whenever a paint runs long, and
/// a decay applied per message then moves the page by however many ticks
/// happened to arrive. That is what made the spring-back jerk: not the curve,
/// the bookkeeping. Scaled by elapsed time it covers the same ground in the
/// same fifth of a second whether it gets sixty frames or twenty.
const STRETCH_DECAY: f32 = 0.80;

/// The span `STRETCH_DECAY` is quoted over, in milliseconds.
const STRETCH_DECAY_MS: f32 = 10.0;

/// Below this the stretch is over, in DIPs. Anything smaller cannot be drawn
/// and would otherwise halve forever.
const STRETCH_DONE: f32 = 0.35;

/// How often the stretch is asked to relax, in milliseconds.
///
/// About sixty a second, which is as often as there is any point redrawing.
/// The decay does not depend on this: see `STRETCH_DECAY`.
const STRETCH_MS: u32 = 16;
/// One wheel notch across a sideways-scrolling picture: three hours of the
/// weather chart, which is one labeled block, so the labels stay put against
/// the plate's edge as the chart steps along.
const HSCROLL_STEP: f32 = 84.0;
/// How far a press has to travel before it is a drag and no longer a click.
const DRAG_SLOP: f32 = 4.0;
/// Win32's MK_SHIFT, in the low word of a wheel message's wparam. The
/// constant lives in a windows-sys feature the crate does not otherwise
/// need, and it is one bit.
const MK_SHIFT: usize = 0x0004;

/// The panel's thread: build the window hidden, pump it, tear it down.
pub(crate) fn run(shared: Arc<Shared>) {
    let class = wide_nul(CLASS_NAME);
    // SAFETY: a zeroed class filled in before registration; registering
    // twice fails harmlessly and the class stands.
    unsafe {
        let mut info: WNDCLASSW = std::mem::zeroed();
        info.style = CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS;
        info.lpfnWndProc = Some(window_proc);
        info.lpszClassName = class.as_ptr();
        info.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        RegisterClassW(&info);
    }

    let families = system::installed_families();
    let state = Box::new(Panel::new(Arc::clone(&shared), families));
    // SAFETY: the class is registered; the state pointer is handed to the
    // window through lpParam and reclaimed in WM_NCDESTROY.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            10,
            10,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            Box::into_raw(state) as *const _,
        )
    };
    if hwnd.is_null() {
        return;
    }
    // A layered window draws nothing until its attributes have been set
    // once, whatever the alpha.
    // SAFETY: a live window on this thread.
    unsafe { SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) };
    shared.hwnd.store(hwnd as isize, Ordering::Release);
    // Contents registered before the window existed have been waiting.
    // SAFETY: the state pointer was stored in WM_NCCREATE and is live.
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel;
        if !state.is_null() {
            (*state).adopt_pending();
        }
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
    shared.open.store(false, Ordering::Release);
}

/// One alpha and position ramp, opening or closing.
struct Fade {
    start: Instant,
    duration: Duration,
    from_alpha: u8,
    to_alpha: u8,
    from_y: i32,
    to_y: i32,
    hide_after: bool,
}

/// The elements as last laid out, in DIPs.
struct Layout {
    /// Content coordinates: the origin is the panel's top-left before
    /// scrolling.
    content: Vec<Element>,
    content_h: f32,
    /// Footer coordinates: the origin is the footer's top-left.
    footer: Vec<Element>,
}

/// A drag across a sideways-scrolling picture, from the press that began it.
struct Drag {
    id: Id,
    /// Where the press landed, in DIPs, and the picture's offset then.
    from_x: f32,
    from_offset: f32,
    /// Whether the pointer has gone further than a click wobbles. Once it
    /// has, the release is the end of a drag and not a click on whatever is
    /// under it.
    moved: bool,
}

struct Panel {
    hwnd: HWND,
    shared: Arc<Shared>,
    contents: Vec<Box<dyn Content>>,
    active: Option<usize>,
    anchor: Option<Anchor>,
    palette: Palette,
    dpi: u32,
    fonts: FontCache,
    layout: Option<Layout>,
    scroll: f32,
    /// How far the page is pulled past one of its ends, in DIPs,
    /// positive meaning the content has been dragged down off the top.
    ///
    /// Kept apart from `scroll` rather than folded into it, so that
    /// everything which asks where in the page we are - the scrollbar
    /// thumb, the clamp, a layout rebuilt on a tick - keeps getting the
    /// honest answer. Only the drawing and the hit testing are offset by
    /// it, and those two together are what makes this a moved page rather
    /// than a picture sliding over its own controls.
    stretch: f32,
    /// Whether the relax timer is running, so a long scroll against the
    /// end does not arm it once per notch.
    stretching: bool,
    /// When the stretch last changed, so the relax is measured against
    /// the clock rather than against however many timer messages arrived.
    stretched_at: Instant,
    /// Where each sideways-scrolling picture has been scrolled to, by its
    /// id, so that a layout rebuilt on a tick or a hover keeps the chart
    /// where the user left it. Forgotten when the panel opens afresh.
    hscroll: Vec<(Id, f32)>,
    drag: Option<Drag>,
    hover: Option<Id>,
    pressed: Option<Id>,
    tracking_leave: bool,
    monitor: Monitor,
    fade: Option<Fade>,
    open: bool,
    /// Where the panel is while open, in screen pixels.
    frame: PxRect,
}

impl Panel {
    fn new(shared: Arc<Shared>, families: Vec<String>) -> Panel {
        Panel {
            hwnd: std::ptr::null_mut(),
            shared,
            contents: Vec::new(),
            active: None,
            anchor: None,
            palette: Palette::resolve(system::current_theme()),
            dpi: 96,
            fonts: FontCache::new(&families),
            layout: None,
            scroll: 0.0,
            stretch: 0.0,
            stretching: false,
            stretched_at: Instant::now(),
            hscroll: Vec::new(),
            drag: None,
            hover: None,
            pressed: None,
            tracking_leave: false,
            monitor: Monitor::default(),
            fade: None,
            open: false,
            frame: PxRect::default(),
        }
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// The client area in DIPs.
    fn client(&self) -> (f32, f32) {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: a live window and a rect on the stack.
        unsafe { GetClientRect(self.hwnd, &mut rect) };
        let scale = self.scale();
        ((rect.right - rect.left) as f32 / scale, (rect.bottom - rect.top) as f32 / scale)
    }

    fn viewport(&self) -> Rect {
        let (w, h) = self.client();
        Rect::new(0.0, 0.0, w, (h - FOOTER_H).max(0.0))
    }

    fn invalidate(&self) {
        // SAFETY: a live window.
        unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
    }

    /// Lays the content out again and, while the panel is showing, resizes
    /// the window to it: the forecast arriving turns a short "fetching"
    /// page into a tall one, and the Mac's fitContent() grows the panel on
    /// every tick for the same reason.
    fn relayout(&mut self) {
        self.layout = None;
        if self.open {
            self.ensure_layout();
            self.fit_frame();
        }
        self.invalidate();
    }

    /// Where the window belongs for the content as laid out.
    fn compute_frame(&self, anchor: Anchor, work: PxRect) -> (PxRect, Option<taskbar::Edge>) {
        let scale = self.scale();
        let content_h = self.layout.as_ref().map(|layout| layout.content_h).unwrap_or(0.0);
        let work_h = (work.bottom - work.top) as f32 / scale;
        let height = placement::panel_height(content_h, FOOTER_H, work_h, placement::GAP_DIP, MAX_PANEL_H);
        let size = (to_px(PANEL_W, scale), to_px(height, scale));
        let bar = taskbar::taskbar().map(|bar| {
            (PxRect { left: bar.rect.left, top: bar.rect.top, right: bar.rect.right, bottom: bar.rect.bottom }, bar.edge)
        });
        let gap = to_px(placement::GAP_DIP, scale);
        (placement::place(anchor.column, bar, size, work, gap), bar.map(|(_, edge)| edge))
    }

    /// Moves and sizes the open window to fit its content.
    fn fit_frame(&mut self) {
        let Some(anchor) = self.anchor else { return };
        let (work, _) = Panel::monitor_of(anchor);
        let (frame, _) = self.compute_frame(anchor, work);
        if frame == self.frame {
            return;
        }
        let delta = frame.top - self.frame.top;
        self.frame = frame;
        match self.fade.as_mut().filter(|fade| !fade.hide_after) {
            // Mid-fade the timer owns the position; it is told where the
            // window is heading and the size is set on its own.
            Some(fade) => {
                fade.from_y += delta;
                fade.to_y = frame.top;
                // SAFETY: a live window.
                unsafe {
                    SetWindowPos(
                        self.hwnd,
                        std::ptr::null_mut(),
                        0,
                        0,
                        frame.right - frame.left,
                        frame.bottom - frame.top,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            None => {
                // SAFETY: a live window.
                unsafe {
                    SetWindowPos(
                        self.hwnd,
                        std::ptr::null_mut(),
                        frame.left,
                        frame.top,
                        frame.right - frame.left,
                        frame.bottom - frame.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
        self.monitor = Monitor::start(vec![frame, anchor.column]);
    }

    fn accent(&self) -> Accent {
        self.active.map(|index| self.contents[index].accent()).unwrap_or(Accent::signature(ModuleId::Cpu))
    }

    /// Adopts contents registered from the other thread.
    fn adopt_pending(&mut self) {
        let pending: Vec<Box<dyn Content>> =
            self.shared.pending.lock().map(|mut pending| pending.drain(..).collect()).unwrap_or_default();
        for mut content in pending {
            content.attach(Wake { shared: Arc::clone(&self.shared) });
            // An item registered twice replaces its earlier content rather
            // than joining it.
            let item = content.item();
            self.contents.retain(|existing| existing.item() != item);
            self.contents.push(content);
        }
    }

    fn content_index(&self, item: StripItem) -> Option<usize> {
        self.contents.iter().position(|content| content.item() == item)
    }

    // ---- opening and closing ---------------------------------------------

    fn on_request(&mut self) {
        let Some(request) = self.shared.request.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        let same = self.open && self.active.is_some_and(|index| self.contents[index].item() == request.item);
        if request.toggle && same {
            self.hide("toggled");
            return;
        }
        self.open_at(request);
    }

    /// The monitor the anchor is on: its work area and its DPI.
    fn monitor_of(anchor: Anchor) -> (PxRect, u32) {
        let rect = RECT {
            left: anchor.column.left,
            top: anchor.column.top,
            right: anchor.column.right,
            bottom: anchor.column.bottom,
        };
        let mut work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        let mut dpi = crate::window::taskbar_dpi();
        // SAFETY: the info struct's size is set as the call requires, and the
        // out parameters are locals.
        unsafe {
            let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
            if !monitor.is_null() {
                let mut info: MONITORINFO = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(monitor, &mut info) != 0 {
                    work = PxRect {
                        left: info.rcWork.left,
                        top: info.rcWork.top,
                        right: info.rcWork.right,
                        bottom: info.rcWork.bottom,
                    };
                }
                let (mut x, mut y) = (0u32, 0u32);
                if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) == 0 && x > 0 {
                    dpi = x;
                }
            }
        }
        (work, dpi)
    }

    fn open_at(&mut self, request: Request) {
        let Some(index) = self.content_index(request.item) else {
            return;
        };
        let was_open = self.open;
        if was_open {
            if let Some(previous) = self.active.filter(|previous| *previous != index) {
                self.contents[previous].closed();
            }
        }
        self.active = Some(index);
        self.anchor = Some(request.anchor);
        self.palette = Palette::resolve(system::current_theme());
        self.apply_chrome();

        let (work, dpi) = Panel::monitor_of(request.anchor);
        if dpi != self.dpi {
            self.dpi = dpi;
            self.fonts.clear();
        }
        self.scroll = 0.0;
        self.stretch = 0.0;
        self.hscroll.clear();
        self.end_drag();
        self.hover = None;
        self.pressed = None;
        self.fade = None;
        self.layout = None;
        self.contents[index].opened();
        self.ensure_layout();

        let scale = self.scale();
        let (frame, edge) = self.compute_frame(request.anchor, work);
        self.frame = frame;

        // The slide comes from the taskbar's side: up from a bottom bar,
        // down from a top one.
        let slide = to_px(SLIDE_DIP, scale);
        let from_y = match edge {
            Some(taskbar::Edge::Top) => frame.top - slide,
            _ => frame.top + slide,
        };
        let animate = !was_open && !reduced_motion();
        let (start_y, start_alpha) = if animate { (from_y, 0) } else { (frame.top, 255) };
        // SAFETY: a live window on this thread.
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                frame.left,
                start_y,
                frame.right - frame.left,
                frame.bottom - frame.top,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
            SetLayeredWindowAttributes(self.hwnd, 0, start_alpha, LWA_ALPHA);
            // Foreground, so Escape reaches it and going elsewhere closes it.
            // Allowed because the click that got here was on this process's
            // own strip, which is the input event the foreground lock keys on.
            SetForegroundWindow(self.hwnd);
        }
        if animate {
            self.fade = Some(Fade {
                start: Instant::now(),
                duration: OPEN,
                from_alpha: 0,
                to_alpha: 255,
                from_y,
                to_y: frame.top,
                hide_after: false,
            });
            // SAFETY: a live window.
            unsafe { SetTimer(self.hwnd, TIMER_FADE, FADE_MS, None) };
        }

        self.monitor = Monitor::start(vec![frame, request.anchor.column]);
        // SAFETY: a live window.
        unsafe {
            SetTimer(self.hwnd, TIMER_DISMISS, DISMISS_MS, None);
            SetTimer(self.hwnd, TIMER_TICK, TICK_MS, None);
        }
        self.open = true;
        self.shared.open.store(true, Ordering::Release);
        if let Ok(mut item) = self.shared.open_item.lock() {
            *item = Some(request.item);
        }
        self.invalidate();
    }

    /// Closes the panel. `why` is written to stderr under BAROMETER_TRACE,
    /// because a panel that closed for no visible reason is impossible to
    /// reason about from the outside - the strip traces under the same
    /// variable for the same reason.
    fn hide(&mut self, why: &'static str) {
        if !self.open {
            return;
        }
        if std::env::var_os("BAROMETER_TRACE").is_some() {
            eprintln!("flyout: closed ({why})");
        }
        self.open = false;
        self.shared.open.store(false, Ordering::Release);
        if let Ok(mut item) = self.shared.open_item.lock() {
            *item = None;
        }
        self.end_drag();
        self.monitor.stop();
        // SAFETY: a live window; killing a timer that is not set is harmless.
        unsafe {
            KillTimer(self.hwnd, TIMER_DISMISS);
            KillTimer(self.hwnd, TIMER_TICK);
            // A page left mid-stretch when the panel closed would keep a
            // timer running against a hidden window, and come back still
            // bent the next time it opened.
            KillTimer(self.hwnd, TIMER_STRETCH);
        }
        self.stretch = 0.0;
        self.stretching = false;
        if let Some(index) = self.active {
            self.contents[index].closed();
        }
        self.hover = None;
        self.pressed = None;
        if reduced_motion() {
            self.fade = None;
            // SAFETY: a live window.
            unsafe {
                KillTimer(self.hwnd, TIMER_FADE);
                ShowWindow(self.hwnd, SW_HIDE);
            }
            return;
        }
        let from_alpha = self.fade.as_ref().map(Fade::current_alpha).unwrap_or(255);
        self.fade = Some(Fade {
            start: Instant::now(),
            duration: CLOSE,
            from_alpha,
            to_alpha: 0,
            from_y: self.frame.top,
            to_y: self.frame.top,
            hide_after: true,
        });
        // SAFETY: a live window.
        unsafe { SetTimer(self.hwnd, TIMER_FADE, FADE_MS, None) };
    }

    fn on_fade_timer(&mut self) {
        let Some(fade) = self.fade.as_ref() else {
            // SAFETY: a live window.
            unsafe { KillTimer(self.hwnd, TIMER_FADE) };
            return;
        };
        let t = fade.progress();
        let alpha = fade.current_alpha();
        let y = fade.from_y + ((fade.to_y - fade.from_y) as f32 * Fade::ease(t)).round() as i32;
        let done = t >= 1.0;
        let hide_after = fade.hide_after;
        // SAFETY: a live window.
        unsafe {
            SetWindowPos(self.hwnd, std::ptr::null_mut(), self.frame.left, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);
        }
        if done {
            self.fade = None;
            // SAFETY: a live window.
            unsafe {
                KillTimer(self.hwnd, TIMER_FADE);
                if hide_after {
                    ShowWindow(self.hwnd, SW_HIDE);
                }
            }
        }
    }

    /// The fifty-millisecond watch for a press outside the panel, which is
    /// how a click on the taskbar - a thing that never takes activation -
    /// still closes it.
    fn on_dismiss_timer(&mut self) {
        // A drag across the chart holds the button down and can leave the
        // panel sideways; that is not a press outside it.
        if !self.open || self.drag.is_some() {
            return;
        }
        // SAFETY: reads button state; no side effects.
        let down = unsafe {
            [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
                .iter()
                .any(|key| (GetAsyncKeyState(*key as i32) as u16) & 0x8000 != 0)
        };
        if !down {
            return;
        }
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: a point on the stack.
        unsafe { GetCursorPos(&mut point) };
        if self.monitor.press(point.x, point.y) {
            self.hide("pressed outside");
        }
    }

    fn on_changed(&mut self) {
        self.adopt_pending();
        if !self.open {
            return;
        }
        let Some(index) = self.active else { return };
        if self.contents[index].tick() {
            self.relayout();
        }
    }

    // ---- layout ---------------------------------------------------------

    fn ensure_layout(&mut self) {
        if self.layout.is_some() {
            return;
        }
        let Some(index) = self.active else { return };
        let scale = self.scale();
        let accent = self.contents[index].accent();
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        // SAFETY: a DC borrowed from the live window and released below.
        let dc = unsafe { GetDC(self.hwnd) };
        let (page, footer) = {
            let palette = &self.palette;
            let hover = self.hover;
            let content = &mut self.contents[index];
            let mut canvas = Canvas::new(dc, scale, &mut self.fonts);
            let surface = RefCell::new(Surface { canvas: &mut canvas, palette, accent });
            let measure = |text: &str, style: Style, wrap: f32| -> (f32, f32) {
                surface.borrow_mut().measure(text, style, wrap)
            };
            let context = Context { width: PANEL_W, palette, accent, measure: &measure, hover, now_unix };
            let page = content.build(&context);
            let footer = footer_elements(&content.actions(), PANEL_W, &measure);
            (page, footer)
        };
        // SAFETY: the DC from GetDC above.
        unsafe { ReleaseDC(self.hwnd, dc) };
        let viewport_h = self.viewport().h;
        self.scroll = clamp_scroll(self.scroll, viewport_h, page.height);
        let mut content = page.elements;
        settle_scrollers(&mut content, &mut self.hscroll);
        self.layout = Some(Layout { content, content_h: page.height, footer });
    }

    /// The sideways-scrolling picture under a point in window coordinates,
    /// when it has more to show.
    fn scroller_at(&self, x: f32, y: f32) -> Option<Id> {
        let layout = self.layout.as_ref()?;
        if !self.viewport().contains(x, y) {
            return None;
        }
        ui::scroll_at(&layout.content, x, y + self.scroll - self.stretch)
    }

    /// Moves the picture with `id` to an offset, and repaints if it moved.
    fn scroll_sideways_to(&mut self, id: Id, to: f32) {
        let Some(layout) = self.layout.as_mut() else { return };
        if scroll_sideways(&mut layout.content, &mut self.hscroll, id, to) {
            self.invalidate();
        }
    }

    /// Scrolls the picture under a point sideways by `by`; whether there
    /// was one to scroll.
    fn scroll_sideways_at(&mut self, x: f32, y: f32, by: f32) -> bool {
        let Some(id) = self.scroller_at(x, y) else { return false };
        let current = self.layout.as_ref().and_then(|layout| scroller_offset(&layout.content, id)).unwrap_or(0.0);
        self.scroll_sideways_to(id, current + by);
        true
    }

    /// Lets go of a drag and of the mouse capture it held.
    fn end_drag(&mut self) {
        if self.drag.take().is_some() {
            // SAFETY: releases whatever capture this thread holds; harmless
            // when it holds none.
            unsafe { ReleaseCapture() };
        }
    }

    /// What is under a point in window coordinates.
    fn hit_at(&self, x: f32, y: f32) -> Option<Id> {
        let layout = self.layout.as_ref()?;
        let (_, height) = self.client();
        let footer_top = height - FOOTER_H;
        if y >= footer_top {
            return ui::hit(&layout.footer, x, y - footer_top);
        }
        ui::hit(&layout.content, x, y + self.scroll - self.stretch)
    }

    // ---- painting -------------------------------------------------------

    fn paint(&mut self) {
        self.ensure_layout();
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        // SAFETY: BeginPaint/EndPaint bracket the DC for this window.
        let dc = unsafe { BeginPaint(self.hwnd, &mut ps) };
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let (w, h) = (client.right - client.left, client.bottom - client.top);
        if w <= 0 || h <= 0 {
            unsafe { EndPaint(self.hwnd, &ps) };
            return;
        }
        // Into a memory bitmap and out in one blit, so a panel of gradients
        // never shows itself half drawn.
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
        let viewport = self.viewport();
        let accent = self.accent();
        let (hover, pressed, scroll) = (self.hover, self.pressed, self.scroll - self.stretch);
        let Some(layout) = self.layout.as_ref() else { return };
        let palette = &self.palette;
        let mut canvas = Canvas::new(dc, scale, &mut self.fonts);
        let mut surface = Surface { canvas: &mut canvas, palette, accent };
        let whole = Rect::new(0.0, 0.0, width, height);
        surface.canvas.fill_rect(whole, palette.ground);

        let clip = surface.clip(viewport);
        paint::draw(&mut surface, &layout.content, hover, pressed, 0.0, -scroll, viewport);
        // The thumb rides the real position, not the stretched one: it marks
        // where in the page you are, and the page has not moved.
        if let Some((y, h)) = scroll_thumb(viewport.h, layout.content_h, self.scroll) {
            let thumb = Rect::new(width - 6.0, viewport.y + y + 4.0, 3.0, (h - 8.0).max(16.0));
            surface.fill_round(thumb, 1.5, palette.theme.stroke_strong);
        }
        surface.unclip(clip);

        let footer_top = height - FOOTER_H;
        let footer = Rect::new(0.0, footer_top, width, FOOTER_H);
        paint::draw(&mut surface, &layout.footer, hover, pressed, 0.0, footer_top, footer);
    }

    fn apply_chrome(&self) {
        let corner = DWMWCP_ROUND;
        let border = self.palette.card_stroke.colorref();
        // SAFETY: each pointer is to a local of the size stated. Neither call
        // is checked: on a Windows without these attributes the panel is
        // square-cornered and unbordered, which is correct there.
        unsafe {
            DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &corner as *const i32 as *const _,
                std::mem::size_of::<i32>() as u32,
            );
            DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_BORDER_COLOR as u32,
                &border as *const u32 as *const _,
                std::mem::size_of::<u32>() as u32,
            );
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
        if self.hover == hover {
            return;
        }
        self.hover = hover;
        let relayout = self.active.is_some_and(|index| self.contents[index].hover(hover));
        if relayout {
            self.relayout();
        } else {
            self.invalidate();
        }
    }

    fn on_mouse_move(&mut self, lparam: LPARAM) {
        let (x, y) = self.mouse_dip(lparam);
        let dragging = self.drag.as_mut().and_then(|drag| {
            let dx = x - drag.from_x;
            if drag.moved || dx.abs() >= DRAG_SLOP {
                drag.moved = true;
                Some((drag.id, drag.from_offset - dx))
            } else {
                None
            }
        });
        if let Some((id, to)) = dragging {
            // A drag is not a click: the press is forgotten so that the
            // release does nothing, and the hover is left alone so that the
            // columns sliding under the pointer do not each get chosen.
            self.pressed = None;
            self.scroll_sideways_to(id, to);
            return;
        }
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
        self.ensure_layout();
        let hover = self.hit_at(x, y);
        self.set_hover(hover);
    }

    fn on_mouse_leave(&mut self) {
        self.tracking_leave = false;
        // Under capture the pointer is still ours outside the window, and
        // the drag needs the hover kept for when it comes back.
        if self.drag.is_none() {
            self.set_hover(None);
        }
    }

    fn on_button_down(&mut self, lparam: LPARAM) {
        let (x, y) = self.mouse_dip(lparam);
        self.ensure_layout();
        self.pressed = self.hit_at(x, y);
        if let Some(id) = self.scroller_at(x, y) {
            let from_offset = self.layout.as_ref().and_then(|layout| scroller_offset(&layout.content, id)).unwrap_or(0.0);
            self.drag = Some(Drag { id, from_x: x, from_offset, moved: false });
            // Captured, so a drag that runs off the panel's edge keeps
            // scrolling and its release is still heard.
            // SAFETY: a live window on this thread.
            unsafe { SetCapture(self.hwnd) };
        }
        self.invalidate();
    }

    fn on_button_up(&mut self, lparam: LPARAM) {
        let (x, y) = self.mouse_dip(lparam);
        let dragged = self.drag.as_ref().is_some_and(|drag| drag.moved);
        self.end_drag();
        let pressed = self.pressed.take();
        if !dragged && pressed.is_some() && pressed == self.hit_at(x, y) {
            if let Some(id) = pressed {
                self.activate(id);
            }
        }
        self.invalidate();
    }

    /// Where a wheel message happened, in window DIPs. Unlike the button
    /// messages, a wheel's position arrives in screen coordinates.
    fn wheel_dip(&self, lparam: LPARAM) -> (f32, f32) {
        let mut point = POINT {
            x: (lparam & 0xFFFF) as u16 as i16 as i32,
            y: ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
        };
        // SAFETY: a live window and a point on the stack.
        unsafe { ScreenToClient(self.hwnd, &mut point) };
        let scale = self.scale();
        (point.x as f32 / scale, point.y as f32 / scale)
    }

    fn activate(&mut self, id: Id) {
        match id {
            Id::None => {}
            Id::Settings => {
                if let Some(index) = self.active {
                    let module = self.contents[index].module();
                    self.command(Command::OpenSettings(module));
                }
                self.hide("settings");
            }
            Id::Custom(_) => {
                let Some(index) = self.active else { return };
                let response = self.contents[index].activate(id);
                self.respond(response);
            }
        }
    }

    fn respond(&mut self, response: Response) {
        match response {
            Response::None => {}
            Response::Repaint => self.invalidate(),
            Response::Relayout => self.relayout(),
            Response::Command(command) => {
                self.command(command);
                self.hide("command");
            }
            Response::Request(command) => {
                self.command(command);
                self.relayout();
            }
            Response::Close => self.hide("content"),
        }
    }

    fn command(&self, command: Command) {
        if let Ok(mut commands) = self.shared.commands.lock() {
            commands.push(command);
        }
    }

    fn on_wheel(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
        // Shift turns the wheel sideways over a picture that scrolls that
        // way, as it does in Explorer and every browser: up is left. Over
        // anything else the panel scrolls as usual, Shift or not.
        if wparam & MK_SHIFT != 0 {
            let (x, y) = self.wheel_dip(lparam);
            if self.scroll_sideways_at(x, y, -delta * HSCROLL_STEP) {
                return;
            }
        }
        self.scroll_elastically(self.scroll - delta * SCROLL_STEP);
    }

    /// A horizontal wheel, or a trackpad's sideways swipe: positive is to
    /// the right, and it moves only the picture under the pointer.
    fn on_hwheel(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
        let (x, y) = self.wheel_dip(lparam);
        self.scroll_sideways_at(x, y, delta * HSCROLL_STEP);
    }

    fn set_scroll(&mut self, scroll: f32) {
        let Some(layout) = self.layout.as_ref() else { return };
        let clamped = clamp_scroll(scroll, self.viewport().h, layout.content_h);
        if (self.scroll - clamped).abs() > f32::EPSILON {
            self.scroll = clamped;
            self.invalidate();
        }
    }

    /// Scrolls, and puts whatever would have gone past either end into the
    /// stretch instead.
    ///
    /// The page does not simply stop at its end: it gives, by a diminishing
    /// amount, and springs back when the wheel does. Only the part that could
    /// not be scrolled becomes stretch, so a wheel notch in the middle of a
    /// long page behaves exactly as it did.
    fn scroll_elastically(&mut self, to: f32) {
        let Some(layout) = self.layout.as_ref() else { return };
        let clamped = clamp_scroll(to, self.viewport().h, layout.content_h);
        // Positive is the top edge pulled down, which is the direction the
        // content moves; past the bottom it is negative.
        let past = clamped - to;
        if past.abs() > f32::EPSILON {
            self.stretch_by(past);
        }
        self.set_scroll(clamped);
    }

    /// Adds to the stretch, with the resistance that makes it feel elastic.
    ///
    /// The resistance is the usual hyperbolic one: the first pixels come
    /// almost free and each one after costs more, so the edge is soft at
    /// first touch and firm if leaned on. `STRETCH_MAX` is the asymptote
    /// rather than a clamp, which is why the page never visibly hits a second
    /// hard stop after the first.
    fn stretch_by(&mut self, by: f32) {
        let was = self.stretch;
        self.stretch = stretched(was, by);
        if (self.stretch - was).abs() > f32::EPSILON {
            self.start_stretch_timer();
            self.invalidate();
        }
    }

    fn start_stretch_timer(&mut self) {
        // Re-based on every pull, so a stretch that is being added to does not
        // relax by the time since the *first* pull the moment it is released.
        self.stretched_at = Instant::now();
        if self.stretching {
            return;
        }
        self.stretching = true;
        // SAFETY: a live window; the timer is killed when the stretch ends
        // and again in WM_DESTROY.
        unsafe { SetTimer(self.hwnd, TIMER_STRETCH, STRETCH_MS, None) };
    }

    /// Lets the page back to where it belongs, by however long it has been.
    fn on_stretch_timer(&mut self) {
        let elapsed = self.stretched_at.elapsed().as_secs_f32() * 1000.0;
        self.stretched_at = Instant::now();
        self.stretch *= STRETCH_DECAY.powf(elapsed / STRETCH_DECAY_MS);
        if self.stretch.abs() < STRETCH_DONE {
            self.stretch = 0.0;
            self.stretching = false;
            // SAFETY: a live window and a timer this function owns.
            unsafe { KillTimer(self.hwnd, TIMER_STRETCH) };
        }
        self.invalidate();
    }

    fn on_key(&mut self, key: u16) -> bool {
        match key {
            VK_ESCAPE => {
                self.hide("escape");
                true
            }
            VK_UP | VK_DOWN => {
                let step = if key == VK_UP { -SCROLL_STEP } else { SCROLL_STEP };
                self.set_scroll(self.scroll + step);
                true
            }
            VK_PRIOR | VK_NEXT => {
                let page = self.viewport().h;
                self.set_scroll(self.scroll + if key == VK_PRIOR { -page } else { page });
                true
            }
            VK_HOME => {
                self.set_scroll(0.0);
                true
            }
            VK_END => {
                self.set_scroll(f32::MAX);
                true
            }
            _ => {
                let Some(index) = self.active else { return false };
                let response = self.contents[index].key(key);
                let handled = response != Response::None;
                self.respond(response);
                handled
            }
        }
    }

    // ---- environment ------------------------------------------------------

    fn on_theme_changed(&mut self) {
        let palette = Palette::resolve(system::current_theme());
        if palette == self.palette {
            return;
        }
        self.palette = palette;
        self.apply_chrome();
        self.relayout();
    }
}

impl Fade {
    fn progress(&self) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Decelerate, cubic-bezier(0, 0, 0, 1) near enough: fast out, settling.
    fn ease(t: f32) -> f32 {
        1.0 - (1.0 - t).powi(3)
    }

    fn current_alpha(&self) -> u8 {
        let t = Fade::ease(self.progress());
        (self.from_alpha as f32 + (self.to_alpha as f32 - self.from_alpha as f32) * t).round() as u8
    }
}

/// Gives every sideways-scrolling picture in a fresh layout the offset it
/// had before, held to what it can show now, or remembers the offset it was
/// laid out with when it is new.
fn settle_scrollers(elements: &mut [Element], remembered: &mut Vec<(Id, f32)>) {
    for element in elements.iter_mut() {
        let width = element.rect.w;
        let Kind::Scroll { content_w, offset, .. } = &mut element.kind else { continue };
        let limit = (*content_w - width).max(0.0);
        match remembered.iter_mut().find(|(id, _)| *id == element.id) {
            Some((_, kept)) => {
                *kept = kept.clamp(0.0, limit);
                *offset = *kept;
            }
            None => {
                *offset = offset.clamp(0.0, limit);
                remembered.push((element.id, *offset));
            }
        }
    }
}

/// Moves the picture with `id` to an offset, held to its content, and
/// remembers it; whether anything moved.
fn scroll_sideways(elements: &mut [Element], remembered: &mut Vec<(Id, f32)>, id: Id, to: f32) -> bool {
    let mut moved = false;
    for element in elements.iter_mut().filter(|element| element.id == id) {
        let width = element.rect.w;
        let Kind::Scroll { content_w, offset, .. } = &mut element.kind else { continue };
        let clamped = to.clamp(0.0, (*content_w - width).max(0.0));
        if (clamped - *offset).abs() > f32::EPSILON {
            *offset = clamped;
            moved = true;
        }
        match remembered.iter_mut().find(|(kept, _)| *kept == id) {
            Some((_, kept)) => *kept = clamped,
            None => remembered.push((id, clamped)),
        }
    }
    moved
}

/// Where the picture with `id` is scrolled to, if there is one.
fn scroller_offset(elements: &[Element], id: Id) -> Option<f32> {
    elements.iter().find_map(|element| match &element.kind {
        Kind::Scroll { offset, .. } if element.id == id => Some(*offset),
        _ => None,
    })
}

/// Whether the user has turned animations off.
fn reduced_motion() -> bool {
    let mut animate: i32 = 1;
    // SAFETY: a BOOL on the stack, which is what the call writes.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut animate as *mut i32 as *mut _, 0)
    };
    ok != 0 && animate == 0
}

/// The footer: a hairline, the gear at the left, and the content's actions
/// at the right, in the footer's own coordinates.
fn footer_elements(actions: &[Action], width: f32, measure: ui::Measure) -> Vec<Element> {
    let mut elements = Vec::new();
    elements.push(Element {
        id: Id::None,
        rect: Rect::new(0.0, 0.0, width, 1.0),
        kind: Kind::Divider,
        interactive: false,
        zones: Vec::new(),
    });
    elements.push(Element {
        id: Id::Settings,
        rect: Rect::new(PANEL_PAD, (FOOTER_H - ICON_BUTTON) / 2.0, ICON_BUTTON, ICON_BUTTON),
        kind: Kind::IconButton { glyph: SETTINGS_GLYPH },
        interactive: true,
        zones: Vec::new(),
    });
    let mut x = width - PANEL_PAD;
    for action in actions.iter().rev() {
        let (label_w, _) = measure(action.text, Style::Body, 0.0);
        let w = label_w + 24.0 + if action.glyph.is_some() { 18.0 } else { 0.0 };
        elements.push(Element {
            id: action.id,
            rect: Rect::new(x - w, (FOOTER_H - BUTTON_H) / 2.0, w, BUTTON_H),
            kind: Kind::Button { text: action.text.to_string(), glyph: action.glyph, enabled: action.enabled },
            interactive: action.enabled,
            zones: Vec::new(),
        });
        x -= w + 8.0;
    }
    elements
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        if !create.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            let state = (*create).lpCreateParams as *mut Panel;
            if !state.is_null() {
                (*state).hwnd = hwnd;
            }
        }
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel;
    if state.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let state = &mut *state;

    match message {
        WM_PAINT => {
            state.paint();
            0
        }
        WM_ERASEBKGND => 1,
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
            state.on_wheel(wparam, lparam);
            0
        }
        WM_MOUSEHWHEEL => {
            state.on_hwheel(wparam, lparam);
            0
        }
        // Capture taken away - a system menu, another window - ends the drag
        // without a release ever arriving.
        WM_CAPTURECHANGED => {
            state.drag = None;
            0
        }
        WM_KEYDOWN => {
            if state.on_key(wparam as u16) {
                0
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_SETCURSOR => {
            if (lparam & 0xFFFF) as u32 == HTCLIENT {
                let hand = state.hover.is_some_and(|id| id != Id::None);
                SetCursor(LoadCursorW(std::ptr::null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
                1
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_TIMER => {
            match wparam {
                TIMER_DISMISS => state.on_dismiss_timer(),
                TIMER_FADE => state.on_fade_timer(),
                TIMER_TICK => state.on_changed(),
                TIMER_STRETCH => state.on_stretch_timer(),
                _ => {}
            }
            0
        }
        WM_ACTIVATE => {
            if wparam & 0xFFFF == WA_INACTIVE as usize && state.monitor.deactivated() {
                state.hide("deactivated");
            }
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            state.on_theme_changed();
            0
        }
        // The panel is placed against a taskbar that has just moved or
        // changed scale, and its place is wrong now. Closing is the honest
        // answer; the next click opens it in the right one.
        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            state.hide("display changed");
            0
        }
        WM_APP_OPEN => {
            state.adopt_pending();
            state.on_request();
            0
        }
        WM_APP_CLOSE => {
            state.hide("asked");
            0
        }
        WM_APP_CHANGED => {
            state.on_changed();
            0
        }
        WM_APP_REGISTER => {
            state.adopt_pending();
            0
        }
        WM_APP_QUIT => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_DISMISS);
            KillTimer(hwnd, TIMER_FADE);
            KillTimer(hwnd, TIMER_TICK);
            KillTimer(hwnd, TIMER_STRETCH);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            let state = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Panel;
            if !state.is_null() {
                drop(Box::from_raw(state));
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_page_gives_less_the_harder_it_is_pulled_and_never_comes_off() {
        // The first notch past the end should move the page a good way and
        // each one after it less, which is what reads as elastic rather than
        // as a page that simply kept scrolling.
        let first = stretched(0.0, SCROLL_STEP);
        let second = stretched(first, SCROLL_STEP) - first;
        let third = stretched(stretched(first, SCROLL_STEP), SCROLL_STEP)
            - stretched(first, SCROLL_STEP);
        assert!(first > 0.0);
        assert!(second < first, "{second} should be less than {first}");
        assert!(third < second, "{third} should be less than {second}");

        // Leaned on indefinitely it approaches the limit and never reaches
        // it, so there is no second hard stop to hit.
        let mut far = 0.0;
        for _ in 0..500 {
            far = stretched(far, SCROLL_STEP);
        }
        assert!(far < STRETCH_MAX, "{far} reached the asymptote");
        assert!(far > STRETCH_MAX * 0.9, "{far} never got near it");
    }

    #[test]
    fn pulling_and_pushing_back_the_same_distance_lands_where_it_started() {
        // The resistance is applied to the total, not to each step, so the
        // page does not creep away from its end while it is scrolled to and
        // fro against it.
        let pulled = stretched(stretched(0.0, 30.0), 20.0);
        let returned = stretched(pulled, -50.0);
        assert!(returned.abs() < 0.01, "came back to {returned}");
    }

    #[test]
    fn the_other_end_behaves_the_same_way_upside_down() {
        assert!((stretched(0.0, -SCROLL_STEP) + stretched(0.0, SCROLL_STEP)).abs() < 0.01);
    }

    #[test]
    fn the_stretch_relaxes_in_the_same_time_however_many_frames_it_gets() {
        // The decay is against the clock, not against timer messages, so a
        // panel that paints slowly springs back over the same span as one
        // that paints quickly - it just does it in fewer, larger steps. This
        // is the difference between a spring and a stutter.
        let settle = |frame_ms: f32| -> f32 {
            let mut left: f32 = STRETCH_MAX;
            let mut elapsed = 0.0;
            while left.abs() >= STRETCH_DONE && elapsed < 5_000.0 {
                left *= STRETCH_DECAY.powf(frame_ms / STRETCH_DECAY_MS);
                elapsed += frame_ms;
            }
            elapsed
        };
        let quick = settle(STRETCH_MS as f32);
        let slow = settle(50.0);
        assert!((quick - slow).abs() < 60.0, "{quick} ms against {slow} ms");
        // Long enough to be seen, short enough not to be waited through.
        assert!((80.0..500.0).contains(&quick), "settles in {quick} ms");
    }
    use super::*;

    fn measure(text: &str, style: Style, _wrap: f32) -> (f32, f32) {
        (text.chars().count() as f32 * 7.0, style.line())
    }

    #[test]
    fn the_footer_has_the_gear_at_the_left_and_actions_stacked_from_the_right() {
        let actions = vec![
            Action { id: Id::Custom(1), text: "Refresh", glyph: Some(SETTINGS_GLYPH), enabled: true },
            Action { id: Id::Custom(2), text: "Copy", glyph: None, enabled: false },
        ];
        let footer = footer_elements(&actions, PANEL_W, &measure);
        let gear = ui::find(&footer, Id::Settings).expect("the gear");
        assert_eq!(gear.rect.x, PANEL_PAD);
        let refresh = footer.iter().find(|e| e.id == Id::Custom(1)).expect("refresh");
        let copy = footer.iter().find(|e| e.id == Id::Custom(2)).expect("copy");
        // The last action sits against the right edge; the one before it to
        // its left, and a disabled one is not a hit target.
        assert!((copy.rect.right() - (PANEL_W - PANEL_PAD)).abs() < 1e-3);
        assert!(refresh.rect.right() < copy.rect.x);
        assert!(!copy.interactive);
        assert_eq!(ui::hit(&footer, refresh.rect.x + 1.0, FOOTER_H / 2.0), Some(Id::Custom(1)));
        assert_eq!(ui::hit(&footer, copy.rect.x + 1.0, FOOTER_H / 2.0), None);
        // Everything fits in the footer's band.
        for element in &footer {
            assert!(element.rect.bottom() <= FOOTER_H + 1e-3);
        }
    }

    /// A picture that draws nothing, for a scroller in a test.
    #[derive(Debug)]
    struct Blank;
    impl ui::Painter for Blank {
        fn paint(&self, _surface: &mut Surface, _rect: Rect, _hover: bool) {}
    }

    fn scroller(id: Id, content_w: f32, offset: f32) -> Element {
        Element {
            id,
            rect: Rect::new(24.0, 100.0, 332.0, 150.0),
            kind: Kind::Scroll { content_w, offset, painter: Box::new(Blank) },
            interactive: false,
            zones: Vec::new(),
        }
    }

    #[test]
    fn a_rebuilt_layout_keeps_each_chart_where_it_was_scrolled_to() {
        let mut remembered = Vec::new();
        // The first layout: the overview chart at its start, the day chart
        // opened on the current hour.
        let mut first = vec![scroller(Id::Custom(4), 1344.0, 0.0), scroller(Id::Custom(10), 672.0, 240.0)];
        settle_scrollers(&mut first, &mut remembered);
        assert_eq!(scroller_offset(&first, Id::Custom(10)), Some(240.0));
        // The user scrolls the overview; a tick rebuilds the layout.
        assert!(scroll_sideways(&mut first, &mut remembered, Id::Custom(4), 500.0));
        assert!(!scroll_sideways(&mut first, &mut remembered, Id::Custom(4), 500.0), "no move is no repaint");
        let mut second = vec![scroller(Id::Custom(4), 1344.0, 0.0), scroller(Id::Custom(10), 672.0, 0.0)];
        settle_scrollers(&mut second, &mut remembered);
        assert_eq!(scroller_offset(&second, Id::Custom(4)), Some(500.0));
        assert_eq!(scroller_offset(&second, Id::Custom(10)), Some(240.0), "the day chart's opening offset is kept too");
    }

    #[test]
    fn a_chart_never_scrolls_past_either_end_even_when_its_content_shrinks() {
        let mut remembered = Vec::new();
        let mut elements = vec![scroller(Id::Custom(4), 1344.0, 0.0)];
        settle_scrollers(&mut elements, &mut remembered);
        assert!(!scroll_sideways(&mut elements, &mut remembered, Id::Custom(4), -50.0));
        assert_eq!(scroller_offset(&elements, Id::Custom(4)), Some(0.0));
        assert!(scroll_sideways(&mut elements, &mut remembered, Id::Custom(4), 9999.0));
        assert_eq!(scroller_offset(&elements, Id::Custom(4)), Some(1344.0 - 332.0));
        // Fewer hours in the next forecast: the remembered offset is pulled
        // back to the new end rather than leaving the plate empty.
        let mut shorter = vec![scroller(Id::Custom(4), 720.0, 0.0)];
        settle_scrollers(&mut shorter, &mut remembered);
        assert_eq!(scroller_offset(&shorter, Id::Custom(4)), Some(720.0 - 332.0));
        // Content that fits does not scroll at all.
        let mut fits = vec![scroller(Id::Custom(4), 300.0, 0.0)];
        settle_scrollers(&mut fits, &mut remembered);
        assert_eq!(scroller_offset(&fits, Id::Custom(4)), Some(0.0));
        assert_eq!(scroller_offset(&fits, Id::Custom(99)), None);
    }

    #[test]
    fn a_fade_settles_on_its_target_and_never_overshoots() {
        let fade = Fade {
            start: Instant::now() - Duration::from_secs(5),
            duration: OPEN,
            from_alpha: 0,
            to_alpha: 255,
            from_y: 100,
            to_y: 92,
            hide_after: false,
        };
        assert_eq!(fade.progress(), 1.0);
        assert_eq!(fade.current_alpha(), 255);
        assert_eq!(Fade::ease(0.0), 0.0);
        assert_eq!(Fade::ease(1.0), 1.0);
        // Decelerating: past halfway by the time half the time has gone.
        assert!(Fade::ease(0.5) > 0.5);
        let instant = Fade { duration: Duration::ZERO, ..fade };
        assert_eq!(instant.progress(), 1.0);
    }
}
