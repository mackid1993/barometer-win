// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The readout, as a window on the taskbar.
//
// It is a child of Shell_TrayWnd rather than a topmost window floating over
// it. A child moves, hides and restacks with the taskbar for free: auto-hide,
// a full-screen game taking the foreground, Explorer restarting, and the
// taskbar changing monitor all resolve themselves. A floating window has to
// chase every one of those, and gets each of them slightly wrong.
//
// Drawing is GDI. The strip is a few dozen glyphs that repaint only when a
// displayed value changes; a Direct2D device with its swap chain and the DXGI
// machinery behind it would cost more memory at rest than everything else in
// this program put together.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{
    COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, CreateSolidBrush, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetDC, GetStockObject, GetTextExtentPoint32W,
    ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, TextOutW, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BI_RGB, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HFONT,
    LOGFONTW, NULL_PEN, OUT_TT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_SELECTED, ODT_MENU};
use barometer_core::weather::badge::Condition;
use barometer_core::weather::glyph;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    FindWindowW, GetCursorPos, LoadCursorW, SetForegroundWindow, SetMenuInfo,
    TrackPopupMenu, IDC_ARROW, MENUINFO, MF_OWNERDRAW, MF_SEPARATOR, MIM_BACKGROUND, MIM_STYLE, MSGF_MENU, WM_ENTERIDLE,
    MNS_NOCHECK, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_DRAWITEM, WM_LBUTTONUP, WM_MEASUREITEM,
    WM_RBUTTONUP,
    DestroyWindow, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    SetWindowPos, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, WM_APP,
    GetAncestor, GetDesktopWindow, GetWindow, GetWindowThreadProcessId, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_REORDER, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GW_HWNDPREV, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    SWP_NOMOVE, SWP_NOSIZE,
    GetWindowRect, UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, ULW_ALPHA, WM_CLOSE,
    WM_DESTROY, WM_PAINT, WM_QUERYENDSESSION, WM_SETTINGCHANGE, WM_THEMECHANGED,
    WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

/// One module's place on the strip: two rows of already-formatted text.
///
/// Except the weather, which is one row with a mark drawn around it. That is
/// not a special case bolted on - it is the design: the condition lives in the
/// bands above and below the digits that a two-row column would have used for
/// its label, so the weather costs the strip a number's width and no more.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub top: String,
    pub bottom: String,
    /// The widest text this column can ever hold, if it is worth reserving.
    ///
    /// The column is laid out at the wider of this and its content, which is
    /// what stops the readout shuffling: sized to the content alone, "9 KB/s"
    /// becoming "512 KB/s" moves every column to its right, several times a
    /// second, and the whole strip crawls.
    pub reserved: String,
    /// Set on the weather column. The value is drawn beside this condition's
    /// mark, and `top` is ignored.
    pub badge: Option<Condition>,
    /// What hovering this column says. Empty means no tooltip.
    ///
    /// The strip has room for a number and nothing else, so everything else a
    /// module knows goes here.

    /// Whether `top` is a label rather than a second value.
    ///
    /// Only a label is drawn in the heading weight. A network column stacks
    /// two rates and neither of them is a heading, so setting both rows from
    /// the position alone would put the upload in semibold for no reason.
    pub heading: bool,
    /// A stack reading's caption, drawn before the value in the heading
    /// face with a colon: `CPU: 45%`. Empty for a module's own column.
    pub top_label: String,
    pub bottom_label: String,
}

/// What the window draws. Owned by the window, replaced whole on each update.
#[derive(Default, PartialEq)]
pub struct StripModel {
    pub columns: Vec<Column>,
    /// True when the strip draws in one color, which the weather mark has to
    /// honor: the marks are designed to stay distinguishable without it.
    pub monochrome: bool,
    pub text_dip: f32,
    pub two_rows: bool,
    pub font_family: String,
    /// The weight of the values.
    pub font_weight: u32,
    /// The weight of the labels above them.
    ///
    /// A strip reads as two kinds of text, not one: `CPU` names the column and
    /// `24%` is what you came to look at. One weight for both means either the
    /// labels shout or the values whisper, and at nine pixels on a taskbar
    /// that difference is most of the legibility.
    pub heading_font_weight: u32,
    /// Gap between columns and padding at each end, in DIPs.
    ///
    /// Carried on the model rather than left as constants, because the
    /// settings window changes them and the readout has to follow without a
    /// restart.
    pub gap_dip: f32,
    pub padding_dip: f32,
}

/// What a right-click on the readout asked for.
///
/// Returned to the caller rather than acted on here: opening a settings window
/// or shutting the program down is the caller's business, and a window
/// procedure that starts doing either is a window procedure that cannot be
/// tested or reused.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    OpenSettings,
    CheckForUpdates,
    Quit,
    /// A left click landed on a column: its index in the strip, left to
    /// right, and where that column is on the screen, which is what a panel
    /// anchors itself to.
    Column { index: usize, rect: crate::settings_ui::geometry::PxRect },
}

/// Menu item ids. Arbitrary, but distinct and non-zero: zero is what
/// TrackPopupMenu returns when the menu is dismissed without a choice.
const MENU_SETTINGS: u32 = 1;
const MENU_UPDATES: u32 = 2;
const MENU_QUIT: u32 = 3;

/// The menu's rows, in order; None is a separator. Owner-drawn items carry
/// no text of their own, so the labels live here, looked up by id when
/// Windows asks how big a row is and then to draw it.
const MENU_ROWS: [Option<(u32, &str)>; 4] = [
    Some((MENU_SETTINGS, "Settings...")),
    Some((MENU_UPDATES, "Check for updates")),
    None,
    Some((MENU_QUIT, "Exit Barometer")),
];

/// The look of the menu being shown, for the measure and draw messages,
/// which arrive without any state of their own. Set for the life of one
/// TrackPopupMenu call. The font is an HFONT kept as an integer so the slot
/// can live in a static.
static MENU_LOOK: std::sync::Mutex<Option<MenuLook>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct MenuLook {
    theme: crate::settings_ui::theme::Theme,
    font: isize,
    dpi: u32,
}

impl MenuLook {
    /// DIPs to pixels at the menu's DPI.
    fn px(&self, dip: f32) -> i32 {
        (dip * self.dpi as f32 / 96.0).round() as i32
    }
}

/// The row's height in DIPs: WinUI's menu flyout item.
const MENU_ROW_DIP: f32 = 32.0;
const MENU_SEPARATOR_DIP: f32 = 9.0;
/// From the row's edge to its text.
const MENU_PAD_DIP: f32 = 12.0;

/// Window state, hung off the window handle.
struct StripState {
    model: StripModel,
    font: Option<HFONT>,
    heading_font: Option<HFONT>,
    font_dpi: u32,
    font_size_dip: f32,
    /// The weather mark's font, and the accent's, cached by pixel size. Two
    /// objects rather than one because the accent is drawn smaller, and a GDI
    /// font carries its size.
    icon_font: Option<HFONT>,
    icon_px: i32,
    accent_font: Option<HFONT>,
    accent_px: i32,
    /// The width the current model needs, kept so an unchanged tick does not
    /// re-measure it.
    width: i32,
    /// What the last right-click asked for, waiting to be collected.
    pending: Option<Command>,
    /// Where each column landed, in client coordinates.
    ///
    /// Recorded by the painter because it is the only code that knows, and
    /// needed by the click that opens a column's panel.
    column_rects: Vec<RECT>,
    /// Raised when something outside the readings changed how they are drawn.
    ///
    /// The unchanged-model early-out below compares only the text, so a
    /// switch between the light and dark taskbar left the old ink on screen
    /// until some reading happened to move - on a weather-only strip, hours
    /// of white on white.
    restyled: bool,
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// Declares this process per-monitor DPI aware, before any window exists.
///
/// Without it Windows virtualises coordinates for the whole process: a
/// 48 pixel taskbar on a 150% display is reported as 32, the strip is built to
/// match, and the result is a readout drawn two thirds of the size it should
/// be with blurred text. Every measurement in this program assumes it is
/// seeing real pixels, and this is what makes that true.
///
/// V2 is the context to ask for: it keeps child windows, dialogs and non-client
/// area scaling correctly when the strip moves between monitors of different
/// scale, which the taskbar does.
pub fn declare_dpi_aware() {
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    // SAFETY: called before any window is created, which is the documented
    // requirement. Failure means an awareness was already set, which is fine.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// The taskbar's own window, which the strip becomes a child of.
pub fn taskbar_window() -> HWND {
    // SAFETY: a class name lookup with no window name.
    unsafe { FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null()) }
}

pub const CLASS_NAME: &str = "BarometerStrip";

/// Posted to the strip when the taskbar re-lays out, from the thread that
/// watches it. The strip climbs back to the top of the topmost band.
pub const WM_RAISE: u32 = WM_APP + 1;

/// The strip, for the restack hook, which has no other way to find it.
static STRIP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static RESTACK_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LAYOUT_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// The third hook. Kept like the other two so `WM_DESTROY` can take it back:
/// this one is system-wide, and a strip rebuilt on every Explorer restart was
/// leaving one behind each time.
static FOREGROUND_HOOK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Set when a window inside the taskbar moved; the main loop takes it and
/// pulls its next tick forward so the readout follows the tray at once.
static TRAY_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn take_tray_changed() -> bool {
    TRAY_CHANGED.swap(false, std::sync::atomic::Ordering::AcqRel)
}

/// Whether the taskbar is stacked above the strip.
///
/// Walks up from the strip through the windows above it. Cheap - a dozen
/// windows - and it is what keeps the hook from feeding itself: once the
/// strip is back on top the taskbar is no longer above it, and the re-assert
/// that follows fires no further event worth acting on.
unsafe fn taskbar_is_above(strip: HWND) -> bool {
    let taskbar = taskbar_window();
    if taskbar.is_null() {
        return false;
    }
    let mut above = GetWindow(strip, GW_HWNDPREV);
    while !above.is_null() {
        if above == taskbar {
            return true;
        }
        above = GetWindow(above, GW_HWNDPREV);
    }
    false
}

/// Puts the strip back above the taskbar the moment the shell restacks it.
///
/// This is the chevron blank, measured. Closing the overflow flyout raises
/// Shell_TrayWnd itself to the top of the topmost band - the strip keeps its
/// topmost style and drops four places, under a taskbar that covers its
/// whole rectangle - and there it stays until the once-a-second re-assert in
/// `place`. Nothing in the readout is hidden or repainted while it happens,
/// which is why the trace shows nothing and why a desktop capture, which
/// changes the composition path, never sees it either.
///
/// Not only the chevron. Clicking any tray icon hands the foreground to that
/// program's popup, and the restack that follows is done from *that*
/// program's thread - so the hook is global, not scoped to Explorer, or the
/// tray icons next to the readout all blank it and the chevron does not.
/// The walk in `taskbar_is_above` is the real filter: whoever restacked,
/// nothing happens unless the taskbar actually ended up over the strip.
///
/// TrafficMonitor's condition for this is `GetForegroundWindow() ==
/// m_hTaskbar`, checked from a timer. This is that check made immediate.
unsafe extern "system" fn restack_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let strip = STRIP.load(std::sync::atomic::Ordering::Acquire) as HWND;
    if strip.is_null() {
        return;
    }
    // Something inside the taskbar moved or resized: the tray re-laying out.
    // Only the taskbar's own windows, and only the window object - the
    // cursor and caret report location changes too, constantly.
    if event == EVENT_OBJECT_LOCATIONCHANGE {
        if object == OBJID_WINDOW && !hwnd.is_null() {
            let taskbar = taskbar_window();
            if !taskbar.is_null() && GetAncestor(hwnd, GA_ROOT) == taskbar {
                TRAY_CHANGED.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        return;
    }
    // A reorder is any container in any process shuffling its children; only
    // the desktop's own is a top-level restack, and that is the only one
    // that can put the taskbar over the strip.
    if event == EVENT_OBJECT_REORDER && hwnd != GetDesktopWindow() {
        return;
    }
    if taskbar_is_above(strip) {
        SetWindowPos(
            strip,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// Installs the restack hook, for every process.
///
/// Out of context, so the callback runs on this thread - the one that owns
/// the strip and pumps its messages - rather than inside anybody else. The
/// events that matter are far apart in the numbering, so they are three
/// hooks rather than one range that would subscribe to everything between
/// them.
///
/// Each handle is kept in a static, and `WM_DESTROY` takes all three back.
unsafe fn hook_restack(strip: HWND) -> HWINEVENTHOOK {
    STRIP.store(strip as usize, std::sync::atomic::Ordering::Release);
    let process = 0;
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let foreground = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        std::ptr::null_mut(),
        Some(restack_proc),
        process,
        0,
        flags,
    );
    FOREGROUND_HOOK.store(foreground as usize, std::sync::atomic::Ordering::Release);
    // Layout changes inside the taskbar, from Explorer only. The scope is
    // what keeps this cheap: every window on the desktop reports these.
    let taskbar = taskbar_window();
    let mut explorer = 0;
    if !taskbar.is_null() {
        GetWindowThreadProcessId(taskbar, &mut explorer);
    }
    let layout = SetWinEventHook(
        EVENT_OBJECT_LOCATIONCHANGE,
        EVENT_OBJECT_LOCATIONCHANGE,
        std::ptr::null_mut(),
        Some(restack_proc),
        explorer,
        0,
        flags,
    );
    LAYOUT_HOOK.store(layout as usize, std::sync::atomic::Ordering::Release);
    SetWinEventHook(
        EVENT_OBJECT_REORDER,
        EVENT_OBJECT_REORDER,
        std::ptr::null_mut(),
        Some(restack_proc),
        process,
        0,
        flags,
    )
}
/// Repaint cadence. The sampling thread decides when values change; this only
/// bounds how stale the strip can look.

/// The strip window.
pub struct Strip {
    hwnd: HWND,
    /// The last rectangle actually asked for, so an unchanged tick makes no
    /// call at all.
    placed: std::cell::Cell<(i32, i32, i32, i32)>,
}

impl Strip {
    /// Creates the strip as a child of the taskbar.
    ///
    /// Returns None when the taskbar cannot be found, which happens while
    /// Explorer is restarting. That is transient: the caller retries rather
    /// than concluding anything from it.
    pub fn create() -> Option<Strip> {
        let parent = taskbar_window();
        if parent.is_null() {
            return None;
        }

        let class = wide(CLASS_NAME);
        // SAFETY: a zeroed class filled in before registration. Registering
        // twice is harmless; the second call fails and the class stands.
        unsafe {
            let mut class_info: WNDCLASSW = std::mem::zeroed();
            class_info.style = CS_HREDRAW | CS_VREDRAW;
            class_info.lpfnWndProc = Some(window_proc);
            class_info.lpszClassName = class.as_ptr();
            // A class with no cursor does not mean "leave the cursor alone",
            // it means the window takes responsibility for setting one and
            // this one does not. The visible result is that moving the pointer
            // onto the readout leaves whatever cursor was last set - which,
            // coming off the taskbar, is the busy spinner, so the strip looks
            // like it is hung. It is not; it just never said what it wanted.
            class_info.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
            RegisterClassW(&class_info);
        }

        // A top-level layered popup, not a child of the taskbar.
        //
        // Windows 11's taskbar hosts its own XAML surface, a
        // Windows.UI.Composition.DesktopWindowContentBridge child of
        // Shell_TrayWnd. A child of the taskbar joins the child z-order
        // underneath it and is composited away: the space is reserved, the
        // paint runs, and nothing is ever seen. Tried twice. A popup gets a
        // surface of its own from DWM, and sits over that one.
        //
        // WS_EX_LAYERED is in the creation style, and that is not a
        // nicety. Applied afterwards through SetWindowLongPtr to a window
        // that was already visible, the style shows up in GetWindowLong and
        // the surface behind it is never set up: every UpdateLayeredWindow
        // from then on fails with ERROR_INVALID_PARAMETER, and the readout is
        // a correctly reserved, entirely blank gap. Taking the style off and
        // putting it back repairs it, which is how this was found.
        //
        // Owned by the taskbar - the parent argument of a WS_POPUP is its
        // owner, not its parent - and this is what makes the chevron blank
        // impossible rather than merely short. Every click in the tray raises
        // Shell_TrayWnd to the top of the topmost band, over the strip. A
        // hook that notices and climbs back is a frame late by construction,
        // and one frame is a visible flicker. The window manager keeps an
        // owned window above its owner and raises the two together, in the
        // same operation, so there is no frame in which the taskbar is on top.
        // The hook stays as the backstop for whatever else restacks us.
        //
        // The owner dying takes the strip with it, which is what already
        // happens when Explorer restarts; the caller rebuilds it.
        //
        // WS_EX_TOOLWINDOW keeps it out of Alt-Tab; WS_EX_NOACTIVATE keeps a
        // click on the readout from stealing focus from whatever the user was
        // typing into.
        // SAFETY: the class is registered and the owner is a live window.
        let hwnd = unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                0,
                0,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            hwnd
        };
        if hwnd.is_null() {
            return None;
        }

        let state = Box::new(StripState {
            model: StripModel::default(),
            font: None,
            heading_font: None,
            font_dpi: 0,
            font_size_dip: 0.0,
            icon_font: None,
            icon_px: 0,
            accent_font: None,
            accent_px: 0,
            width: 0,
            pending: None,
            column_rects: Vec::new(),
            restyled: false,
        });
        // SAFETY: the box is leaked into the window's user data and reclaimed
        // in WM_DESTROY.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            RESTACK_HOOK.store(hook_restack(hwnd) as usize, std::sync::atomic::Ordering::Release);
        }

        Some(Strip { hwnd, placed: std::cell::Cell::new((0, 0, 0, 0)) })
    }

    pub fn handle(&self) -> HWND {
        self.hwnd
    }

    /// Moves the strip to a rectangle in the taskbar's client space.
    ///
    /// Raised to the top of its siblings on every move rather than left where
    /// it was. The shell adds and removes windows inside the taskbar as it
    /// pleases - the overflow flyout, a thumbnail preview, its own XAML
    /// surface being rebuilt - and any of them can end up above the readout.
    pub fn place(&self, x: i32, y: i32, width: i32, height: i32) {
        // SAFETY: the window is live for the life of this struct.
        unsafe {
            if self.placed.get() == (x, y, width, height) {
                // The rectangle has not changed, but the z-order still has to
                // be re-asserted. Skipping the call entirely was a bug: the
                // shell reorders the windows inside the taskbar whenever it
                // feels like it, and once its own tray surface came back above
                // the readout the reserved placeholders showed through and
                // could be hovered - little transparent icons with a tooltip
                // on them, sitting in the middle of the readout.
                //
                // Position and size are both suppressed, so this costs a
                // z-order check and no repaint.
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
                return;
            }
            let (_, _, was_w, was_h) = self.placed.get();
            let resized = was_w != width || was_h != height;
            self.placed.set((x, y, width, height));
            SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE);

            // Repainted only when the size changed, and this is the flash.
            //
            // The reserved block really does move: the tray re-packs when
            // anything in it appears, disappears or changes width, and the
            // trace shows the region sitting at two positions thirty-six
            // pixels apart for tens of seconds each. Following it is correct -
            // the readout has to stay in the space it reserved.
            //
            // What was wrong was repainting to do it. `paint` tears down the
            // layered surface and hands Windows a whole new one through
            // UpdateLayeredWindow, and doing that in the same breath as moving
            // the window is a visible blink. For a pure translation there is
            // nothing to redraw: the pixels are identical and the surface
            // travels with the window. Only a size change needs a new surface,
            // because the surface is the size of the window.
            //
            // Every debounce before this was chasing the move, which was real.
            // The move was never the problem.
            if resized {
                let state = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut StripState;
                if !state.is_null() {
                    paint(self.hwnd, &mut *state);
                }
            }
        }
    }

    /// Takes whatever the last right-click asked for, clearing it.
    pub fn take_command(&self) -> Option<Command> {
        // SAFETY: the pointer was stored at creation and cleared only in
        // WM_DESTROY, after which this struct is gone too.
        unsafe {
            let state = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut StripState;
            if state.is_null() {
                return None;
            }
            (*state).pending.take()
        }
    }

    /// Forgets where the strip was, so the next placement is made again.
    ///
    /// Needed after Explorer restarts: the window is new, and the cached
    /// rectangle belongs to the one that died.
    pub fn forget_placement(&self) {
        self.placed.set((0, 0, 0, 0));
    }

    /// Shows or hides the readout.
    ///
    /// Hiding is not cosmetic. When another program's icon is dragged into the
    /// reserved region the readout would cover it, leaving it invisible and
    /// unclickable, so the readout gets out of the way until the icon leaves.
    pub fn set_visible(&self, visible: bool) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNA};
        // SAFETY: the window is live for the life of this struct. SW_SHOWNA
        // rather than SW_SHOW, because a readout must never take focus.
        unsafe {
            ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }

    /// Replaces what the strip draws and asks for a repaint.
    ///
    /// Repainting is requested rather than done here: the caller is the
    /// sampling thread and painting belongs to the thread that owns the
    /// window.
    /// Returns the width the new model needs, in physical pixels.
    ///
    /// Measured in the font that will actually draw it rather than estimated
    /// from character counts. An estimate is wrong in both directions and both
    /// are visible: too small clips the rightmost column against the tray, too
    /// large leaves a gap of dead taskbar. The caller reserves this figure, so
    /// the space asked for and the space used are the same number.
    pub fn update(&self, model: StripModel) -> i32 {
        // SAFETY: the pointer was stored at creation and is cleared only in
        // WM_DESTROY, after which this struct is gone too.
        unsafe {
            let state = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut StripState;
            if state.is_null() {
                return 0;
            }
            // Nothing changed, so nothing is redrawn. Values on a taskbar hold
            // still for seconds at a time, and repainting them anyway is both
            // wasted work and, over a translucent taskbar, a visible flicker.
            if (*state).model == model && (*state).width != 0 && !(*state).restyled {
                return (*state).width;
            }
            (*state).restyled = false;
            // A new family or weight needs new font objects. The cache keys
            // on DPI and size, which is what changes on the tick; a weight
            // chosen in settings changed neither, so the old faces kept
            // drawing and the setting looked inert.
            let refaced = (*state).model.font_family != model.font_family
                || (*state).model.font_weight != model.font_weight
                || (*state).model.heading_font_weight != model.heading_font_weight;
            (*state).model = model;
            if refaced {
                for font in [(*state).font.take(), (*state).heading_font.take()].into_iter().flatten() {
                    DeleteObject(font as _);
                }
            }
            (*state).width = content_width(self.hwnd, &mut *state);
            // Drawn here rather than invalidated. A layered window with an
            // alpha channel is not painted on demand: Windows keeps the
            // surface and composites it, and never asks for it again.
            paint(self.hwnd, &mut *state);
            (*state).width
        }
    }
}

/// Measures the model with the real font, on a DC borrowed from the window.
///
/// Shares `column_extents` with the painter so the two can never disagree
/// about how wide a column is.
unsafe fn content_width(hwnd: HWND, state: &mut StripState) -> i32 {
    use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC};

    let dc = GetDC(hwnd);
    if dc.is_null() {
        return 0;
    }
    let dpi = GetDpiForWindow(hwnd).max(96);
    let font = ensure_font(state, dpi);
    let heading = ensure_heading_font(state, dpi);
    let previous = SelectObject(dc, font as _);

    let mark_px = mark_pixels(dc, &state.model);
    let gap = dip_to_px(state.model.gap_dip, dpi);
    // The columns and the gaps between them, and nothing at the ends.
    //
    // The end padding used to be counted here, and it is the wrong thing to
    // ask the tray for: space is reserved in whole icon slots, so a request a
    // few pixels over a slot boundary costs a whole further slot - forty-odd
    // pixels of taskbar to buy eight of margin. The reserved region is
    // quantized upwards anyway, so it is nearly always wider than the readout
    // and the painter centres in whatever it gets; that slack is where the
    // margin actually comes from. `padding_dip` is a preference for how much
    // of it to keep, not a demand for more space, and the chevron end is
    // protected by the inset the placement leaves rather than by this.
    let mut width = 0;
    for (index, column) in state.model.columns.iter().enumerate() {
        if index > 0 {
            width += gap;
        }
        width +=
            column_extents(dc, column, state.model.two_rows, dpi as f32 / 96.0, mark_px, heading)
                .0;
    }

    SelectObject(dc, previous);
    ReleaseDC(hwnd, dc);
    width
}

/// One column's width, and the size of each of its two lines.
///
/// A column is as wide as its widest line, whether or not both are drawn, so
/// that a value gaining a digit does not shuffle every column beside it.
unsafe fn column_extents(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    column: &Column,
    two_rows: bool,
    scale: f32,
    mark_px: i32,
    heading_font: HFONT,
) -> (i32, SIZE, SIZE) {
    let top = wide_no_nul(&column.top);
    let bottom = wide_no_nul(&column.bottom);
    // A label is measured in the weight it will be drawn in. Semibold is wider
    // than regular, and measuring it in the value's weight reserves a column
    // narrower than the label needs - which clips it, and only for the people
    // who chose a heavier heading.
    let top_size = if column.heading && !heading_font.is_null() {
        let previous = SelectObject(dc, heading_font as _);
        let size = measure(dc, &top);
        SelectObject(dc, previous);
        size
    } else {
        measure(dc, &top)
    };
    let bottom_size = measure(dc, &bottom);
    // A stack reading's line is its label in the heading face, a space, and
    // the value; the label's share is what `label_width` reports again when
    // the line is drawn.
    let top_size = SIZE {
        cx: top_size.cx + label_width(dc, &column.top_label, heading_font),
        cy: top_size.cy,
    };
    let bottom_size = SIZE {
        cx: bottom_size.cx + label_width(dc, &column.bottom_label, heading_font),
        cy: bottom_size.cy,
    };
    let held = if column.reserved.is_empty() {
        0
    } else {
        measure(dc, &wide_no_nul(&column.reserved)).cx
    };
    let width = if column.badge.is_some() {
        // Rays fan past the digits and the crescent sits over the degree sign,
        // so the column is the number plus that room. Without it the mark is
        // clipped by whatever column comes next.
        bottom_size.cx.max(held) + mark_px + (MARK_GAP_DIP * scale).round() as i32
    } else if two_rows {
        top_size.cx.max(bottom_size.cx).max(held)
    } else if column.bottom.is_empty() {
        top_size.cx.max(held)
    } else {
        bottom_size.cx.max(held)
    };
    (width, top_size, bottom_size)
}

/// Gap between the weather mark and its temperature, in DIPs.
const MARK_GAP_DIP: f32 = 3.0;

/// Gap between columns, in DIPs, before the user has said otherwise.
///
/// Wide enough that two columns of digits read as two numbers rather than one
/// long one, and no wider. Fourteen was far too much: on a taskbar the readout
/// competes for room with the task buttons, and space spent between columns is
/// space the buttons lose for nothing.
pub const COLUMN_GAP_DIP: f32 = 6.0;
/// Padding at each end of the strip, in DIPs.
///
/// None, and that is deliberate. The reserved region is quantized to whole
/// tray slots, so it is already up to a slot wider than the readout needs and
/// the readout is centered in it - there is margin at both ends whether we ask
/// for it or not. Adding padding on top only doubles a gap that was already
/// too big, on a taskbar where every pixel spent is a pixel the task buttons
/// do not get.
pub const PADDING_DIP: f32 = 0.0;

/// The color that gets keyed out, leaving the taskbar showing through.
///
/// How opaque the parts of the readout that draw nothing are.
///
/// Invisible to the eye and solid to the mouse, which is the whole trick. See
/// `colorize`.
const BACKGROUND_ALPHA: u8 = 6;

/// Text, one for each taskbar theme.
///
/// The taskbar follows the system theme independently of apps, so the readout
/// asks the shell which one is in force rather than assuming dark.
const TEXT_ON_DARK: COLORREF = 0x00F2_F2F2;
const TEXT_ON_LIGHT: COLORREF = 0x0019_1919;

/// Whether the taskbar is drawing itself light.
///
/// `SystemUsesLightTheme` is the shell's own switch - the one that moves the
/// taskbar and Start - and is deliberately separate from `AppsUseLightTheme`,
/// which is what an ordinary window would follow. The readout lives on the
/// taskbar, so it follows the taskbar. Absent means dark, which is the default
/// on a fresh install.
fn taskbar_is_light() -> bool {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
    };
    let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the buffer and its size are ours and agree.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut _,
            &mut size,
        )
    };
    status == 0 && value != 0
}

fn dip_to_px(dip: f32, dpi: u32) -> i32 {
    (dip * dpi as f32 / 96.0).round() as i32
}

/// Builds the font, reusing the last one while nothing that shapes it changed.
///
/// Fonts are a limited GDI resource and creating one per paint is how a
/// long-running program runs a desktop out of handles.
/// The font for the values.
fn ensure_font(state: &mut StripState, dpi: u32) -> HFONT {
    let weight = state.model.font_weight;
    let font = build_font(state, dpi, weight, |state| &mut state.font);
    font
}

/// The font for the labels, which is a different weight and nothing else.
fn ensure_heading_font(state: &mut StripState, dpi: u32) -> HFONT {
    let weight = state.model.heading_font_weight;
    build_font(state, dpi, weight, |state| &mut state.heading_font)
}

/// Builds and caches one of the two faces.
///
/// Both are invalidated together, on DPI or size, because they only ever
/// differ in weight - so one cache pair of (dpi, size) covers them and there
/// is no state in which one is stale and the other is not.
fn build_font(
    state: &mut StripState,
    dpi: u32,
    weight: u32,
    slot: fn(&mut StripState) -> &mut Option<HFONT>,
) -> HFONT {
    let size = state.model.text_dip;
    let stale = state.font_dpi != dpi || (state.font_size_dip - size).abs() >= f32::EPSILON;
    if stale {
        // Both faces go, not just the one being asked for: they share the
        // cached size, so leaving the other behind would keep drawing the
        // labels at the previous DPI.
        for slot in [
            (|s: &mut StripState| &mut s.font) as fn(&mut StripState) -> &mut Option<HFONT>,
            |s: &mut StripState| &mut s.heading_font,
        ] {
            if let Some(font) = slot(state).take() {
                // SAFETY: a font this function created and is about to replace.
                unsafe { DeleteObject(font as _) };
            }
        }
    } else if let Some(font) = *slot(state) {
        return font;
    }

    let mut logical: LOGFONTW = unsafe { std::mem::zeroed() };
    // Negative height asks for a character height rather than a cell height,
    // which is what a type size means.
    logical.lfHeight = -dip_to_px(size, dpi);
    let weight = if weight == 0 { FW_NORMAL as i32 } else { weight as i32 };
    logical.lfCharSet = DEFAULT_CHARSET;
    logical.lfOutPrecision = OUT_TT_PRECIS;
    logical.lfClipPrecision = CLIP_DEFAULT_PRECIS;
    // Grayscale antialiasing, asked for by name. The default quality on a
    // 32-bit DIB is aliased, which is the jagged text; and ClearType, which
    // the settings preview uses, cannot work here - its color fringes are
    // computed against an opaque background this surface does not have, and
    // `colorize` reads coverage from the brightest channel, which only means
    // something when the antialiasing is gray.
    logical.lfQuality = ANTIALIASED_QUALITY;
    // The weight's named instance when the machine has one. GDI knows
    // nothing of variable fonts: asked for "Segoe UI Variable Text" at 600
    // it smears the regular face bolder, which is the very, very bold that
    // semibold headings came out as. The preview resolves the same way, so
    // the two agree about what semibold looks like.
    let family = crate::settings_ui::gdi::instance_family(
        &state.model.font_family,
        weight,
        installed_families(),
    );
    // A named instance carries its weight in the face itself - "Segoe UI
    // Semibold" is its own family, not Segoe UI asked to be bolder - so the
    // weight is spent once, not twice. Asking that family for 600 as well
    // invites GDI to embolden a face that is already semibold. TrafficMonitor
    // writes `font_name = Segoe UI Semibold` with `font_style = 0` for the
    // same reason, and this is the same request.
    logical.lfWeight = if family == state.model.font_family { weight } else { FW_NORMAL as i32 };
    let family = wide(&family);
    for (index, unit) in family.iter().take(logical.lfFaceName.len()).enumerate() {
        logical.lfFaceName[index] = *unit;
    }

    // SAFETY: logical is fully initialized above.
    let font = unsafe { CreateFontIndirectW(&logical) };
    *slot(state) = Some(font);
    state.font_dpi = dpi;
    state.font_size_dip = size;
    font
}

/// The machine's font families, enumerated once. Fonts installed during a
/// run are not seen until the next; the settings window has the same limit.
fn installed_families() -> &'static [String] {
    static INSTALLED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(crate::settings_ui::system::installed_families)
}

/// The room a stack reading's label takes before its value: the label with
/// its colon in the heading face, and a space in the value's face. Zero
/// for a line with no label.
unsafe fn label_width(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    label: &str,
    heading_font: HFONT,
) -> i32 {
    if label.is_empty() {
        return 0;
    }
    let text = wide_no_nul(&crate::settings_ui::preview::label_text(label));
    let head = if heading_font.is_null() {
        measure(dc, &text)
    } else {
        let previous = SelectObject(dc, heading_font as _);
        let size = measure(dc, &text);
        SelectObject(dc, previous);
        size
    };
    head.cx + measure(dc, &wide_no_nul(" ")).cx
}

/// Draws a stack reading's label at `x`, in the heading face, and returns
/// where the value goes.
unsafe fn draw_label(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    x: i32,
    y: i32,
    label: &str,
    heading_font: HFONT,
) -> i32 {
    if label.is_empty() {
        return x;
    }
    let text = wide_no_nul(&crate::settings_ui::preview::label_text(label));
    let previous = if heading_font.is_null() { std::ptr::null_mut() } else { SelectObject(dc, heading_font as _) };
    TextOutW(dc, x, y, text.as_ptr(), text.len() as i32);
    if !previous.is_null() {
        SelectObject(dc, previous);
    }
    x + label_width(dc, label, heading_font)
}

/// Measures a string in the currently selected font.
fn measure(dc: windows_sys::Win32::Graphics::Gdi::HDC, text: &[u16]) -> SIZE {
    let mut size = SIZE { cx: 0, cy: 0 };
    // SAFETY: text is a live slice; length excludes the terminator.
    unsafe {
        GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
    }
    size
}

/// Paints the strip into a memory bitmap and blits it in one go.
///
/// Drawing straight to the window would flicker: the taskbar is a busy surface
/// and the strip repaints on a timer, so a partially drawn frame is visible.
/// Diagnostic only: how many times the strip has actually repainted.
pub static PAINTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

unsafe fn paint(hwnd: HWND, state: &mut StripState) {
    PAINTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut window: RECT = std::mem::zeroed();
    GetWindowRect(hwnd, &mut window);
    let width = window.right - window.left;
    let height = window.bottom - window.top;
    if width <= 0 || height <= 0 {
        return;
    }

    let dpi = GetDpiForWindow(hwnd).max(96);
    let screen = GetDC(std::ptr::null_mut());
    let memory_dc = CreateCompatibleDC(screen);
    ReleaseDC(std::ptr::null_mut(), screen);

    // A top-down 32-bit surface whose bytes can be read back, which a
    // compatible bitmap could not be.
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB as u32;
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bitmap =
        CreateDIBSection(memory_dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        DeleteDC(memory_dc);
        return;
    }
    let previous_bitmap = SelectObject(memory_dc, bitmap as _);
    // Nothing to clear: CreateDIBSection hands back zeroed memory, which is
    // the transparent black this starts from.

    let font = ensure_font(state, dpi);
    let heading_font = ensure_heading_font(state, dpi);
    let previous_font = SelectObject(memory_dc, font as _);
    SetBkMode(memory_dc, TRANSPARENT as i32);
    let light = taskbar_is_light();
    let text_color = if light { TEXT_ON_LIGHT } else { TEXT_ON_DARK };
    // Drawn white whatever the theme; colorize applies the real color once the
    // coverage has been read back out of the brightness.
    SetTextColor(memory_dc, 0x00FF_FFFF);
    // The marks are drawn in the design's own units, which are DIPs, so the
    // same mark is the same size on every display rather than the same number
    // of pixels.
    let scale = dpi as f32 / 96.0;

    // The mark fonts are made before the columns are walked. Making them
    // inside the loop would borrow the state mutably while the columns are
    // borrowed from it, and there is only ever one weather column, so there is
    // nothing to gain by deferring it.
    let mark_px = mark_pixels(memory_dc, &state.model);
    let accent_px = accent_pixels(&state.model, mark_px);
    let (icon_font, accent_font) = if mark_px > 0 {
        (ensure_icon_font(state, mark_px), ensure_accent_font(state, accent_px.max(1)))
    } else {
        (std::ptr::null_mut(), std::ptr::null_mut())
    };

    let gap = dip_to_px(state.model.gap_dip, dpi);

    // Centered in whatever width the window ended up with, rather than started
    // hard against the left edge. The reserved region is quantized to whole
    // tray slots, so it is almost always a little wider than the readout needs,
    // and left-aligning piles all of that slack against the right-hand end
    // where it reads as a gap somebody forgot to close. Split evenly it reads
    // as margin.
    // Centred in whatever width the window ended up with. The reserved region
    // is quantized to whole tray slots, so it is almost always a little wider
    // than the readout needs; splitting that slack evenly is where the margin
    // at each end comes from, and it costs no tray space because it is space
    // the slot had already been paid for. Left-aligning would pile all of it
    // against the right-hand end, where it reads as a gap somebody forgot to
    // close.
    let content = content_extent(memory_dc, state, gap, scale, mark_px, heading_font);
    let mut x = ((width - content) / 2).max(0);

    let mut rects: Vec<RECT> = Vec::with_capacity(state.model.columns.len());
    for column in &state.model.columns {
        let top = wide_no_nul(&column.top);
        let bottom = wide_no_nul(&column.bottom);
        let (column_width, top_size, bottom_size) =
            column_extents(memory_dc, column, state.model.two_rows, scale, mark_px, heading_font);

        if let Some(condition) = column.badge {
            // The weather column: the condition as a Segoe Fluent Icons mark,
            // then the temperature beside it.
            //
            // Beside rather than around, which is what the ported macOS marks
            // did. That layout assumes a menu bar item taller than its type,
            // and a Windows taskbar sized so the readout is legible leaves no
            // bands above and below to put anything in - the cloud ended up
            // sitting on the digits. Side by side costs a little width and
            // reads correctly at every bar height.
            let mark = glyph::composed(condition);
            let mark_size = mark_px;
            let text = &bottom;

            let gap = dip_to_px(MARK_GAP_DIP, dpi);
            let text_x = x + mark_size + gap;
            let text_y = (height - bottom_size.cy) / 2;

            let previous = SelectObject(memory_dc, icon_font as _);
            let mark_y = (height - mark_size) / 2;
            let base = [mark.base as u16];
            TextOutW(memory_dc, x, mark_y, base.as_ptr(), 1);

            // The accent is a second, smaller glyph from the same font. It
            // needs its own font object, so it is drawn only when there is one
            // - which is nine conditions out of fourteen.
            if let Some(accent) = mark.accent {
                SelectObject(memory_dc, accent_font as _);
                let glyphs = [accent.glyph as u16];
                TextOutW(
                    memory_dc,
                    x + ((mark_size as f32) * accent.dx).round() as i32,
                    mark_y + ((mark_size as f32) * accent.dy).round() as i32,
                    glyphs.as_ptr(),
                    1,
                );
            }
            SelectObject(memory_dc, previous);

            TextOutW(memory_dc, text_x, text_y, text.as_ptr(), text.len() as i32);
        } else if state.model.two_rows {
            // Two rows sharing the strip's vertical center, with the leading
            // between them coming from the font rather than a constant.
            let line = top_size.cy.max(bottom_size.cy);
            let block = line * 2;
            let top_y = (height - block) / 2;
            // Left-aligned inside the column, both rows sharing one edge.
            //
            // Right-aligning is the usual advice for numbers, and it is wrong
            // here. The column is laid out at its *reserved* width, which is
            // wider than the value nearly all the time, so pinning the right
            // edge leaves the digits sliding left and right inside it as the
            // number changes - the whole strip crawling, which is what the
            // reserved width was meant to stop. A fixed left edge holds still.
            if column.heading {
                let previous = SelectObject(memory_dc, heading_font as _);
                TextOutW(memory_dc, x, top_y, top.as_ptr(), top.len() as i32);
                SelectObject(memory_dc, previous);
            } else {
                let value_x = draw_label(memory_dc, x, top_y, &column.top_label, heading_font);
                TextOutW(memory_dc, value_x, top_y, top.as_ptr(), top.len() as i32);
            }
            let value_x = draw_label(memory_dc, x, top_y + line, &column.bottom_label, heading_font);
            TextOutW(memory_dc, value_x, top_y + line, bottom.as_ptr(), bottom.len() as i32);
        } else {
            // One row: the value, with the label dropped rather than crushed.
            let (text, label) = if column.bottom.is_empty() {
                (&top, &column.top_label)
            } else {
                (&bottom, &column.bottom_label)
            };
            let size = measure(memory_dc, text);
            let y = (height - size.cy) / 2;
            let value_x = draw_label(memory_dc, x, y, label, heading_font);
            TextOutW(memory_dc, value_x, y, text.as_ptr(), text.len() as i32);
        }

        // Where this column ended up, for the tooltip that attaches to it and
        // for the click that opens the weather panel. Recorded here because
        // this is the only place that knows.
        rects.push(RECT { left: x, top: 0, right: x + column_width, bottom: height });

        x += column_width + gap;
    }

    state.column_rects = rects;
    // No tooltips. Every column opens a panel on a click now, and a tip that
    // pops up on the way to clicking is noise in front of the thing the
    // click is about to show.

    // Everything was drawn white onto nothing. Turn that into premultiplied
    // color and the alpha it implies, then hand the whole surface over.
    colorize(bits as *mut u8, width, height, text_color);

    let mut source = POINT { x: 0, y: 0 };
    let mut position = POINT { x: window.left, y: window.top };
    let mut size = SIZE { cx: width, cy: height };
    let mut blend: BLENDFUNCTION = std::mem::zeroed();
    blend.BlendOp = AC_SRC_OVER as u8;
    blend.SourceConstantAlpha = 255;
    blend.AlphaFormat = AC_SRC_ALPHA as u8;
    UpdateLayeredWindow(
        hwnd,
        std::ptr::null_mut(),
        &mut position,
        &mut size,
        memory_dc,
        &mut source,
        0,
        &mut blend,
        ULW_ALPHA,
    );

    SelectObject(memory_dc, previous_font);
    SelectObject(memory_dc, previous_bitmap);
    DeleteObject(bitmap as _);
    DeleteDC(memory_dc);
}

/// Turns a white-on-nothing drawing into premultiplied color plus alpha.
///
/// GDI wrote brightness where it drew and left the alpha byte alone, so the
/// brightest channel of each pixel is exactly the coverage that glyph had,
/// including the partial coverage along an antialiased edge. That becomes the
/// alpha; the color is the readout's own, premultiplied as UpdateLayeredWindow
/// requires. The readout is one color throughout, so nothing is lost by
/// applying it afterwards rather than drawing in it.
///
/// Untouched pixels get a small alpha rather than zero. They look identical -
/// six parts in two hundred and fifty-five, over a taskbar - but a fully
/// transparent pixel is transparent to the *mouse* as well, and the whole
/// reason for this rendering path is that the readout has to swallow the
/// hovers and clicks that would otherwise reach the placeholder icons beneath
/// it and light them up.
///
/// One was tried first and is not enough: with an alpha of one the composited
/// result still hit-tested through to the tray underneath, so the threshold is
/// evidently not a simple "greater than zero". Six is the smallest value found
/// to hold, and it is still invisible.
unsafe fn colorize(bits: *mut u8, width: i32, height: i32, color: COLORREF) {
    // COLORREF is 0x00BBGGRR and the surface is BGRA in memory.
    let (r, g, b) = (
        (color & 0xFF) as u32,
        ((color >> 8) & 0xFF) as u32,
        ((color >> 16) & 0xFF) as u32,
    );
    let pixels = std::slice::from_raw_parts_mut(bits, (width * height * 4) as usize);
    for pixel in pixels.chunks_exact_mut(4) {
        let coverage = pixel[0].max(pixel[1]).max(pixel[2]) as u32;
        if coverage == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            pixel[3] = BACKGROUND_ALPHA;
            continue;
        }
        pixel[0] = ((b * coverage) / 255) as u8;
        pixel[1] = ((g * coverage) / 255) as u8;
        pixel[2] = ((r * coverage) / 255) as u8;
        pixel[3] = coverage as u8;
    }
}

/// UTF-16 without the terminator, which the text-drawing calls do not want.
fn wide_no_nul(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().collect()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            // Answered and nothing more. The surface is handed over by
            // UpdateLayeredWindow when a value changes; leaving this
            // unanswered would have Windows ask again forever.
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        // The taskbar re-laid itself out - the overflow chevron, a thumbnail,
        // its own surface being rebuilt - and the shell puts its windows above
        // this one when it does. Climb straight back. Position and size are
        // both suppressed, so this is a z-order check and no repaint; the
        // once-a-second re-assert in `place` is the backstop, and a second is
        // exactly the blank the user sees, so this one is immediate instead.
        WM_RAISE => {
            TRAY_CHANGED.store(true, std::sync::atomic::Ordering::Release);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
            0
        }
        // The taskbar's own light-or-dark switch, which decides the ink.
        // Both messages, because the shell sends WM_SETTINGCHANGE with
        // "ImmersiveColorSet" for the color change and WM_THEMECHANGED when
        // the visual style itself is swapped, and either can arrive alone.
        // Nothing is painted here: the next tick paints, and the flag is
        // only what stops it taking the unchanged-model early-out.
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StripState;
            if !state.is_null() {
                (*state).restyled = true;
            }
            0
        }
        // The menu window has just been created and is about to be shown.
        // It belongs to the system - class "#32768" - so this is the only
        // moment there is to ask DWM for the backdrop and the corners that
        // make it look like a Windows 11 menu rather than a gray box.
        WM_ENTERIDLE => {
            if wparam as u32 == MSGF_MENU {
                dress_menu_window(lparam as HWND);
            }
            0
        }

        WM_LBUTTONUP => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StripState;
            if crate::trace::on() {
                crate::trace::line(&format!("WM_LBUTTONUP lparam={lparam:#x} state_null={}", state.is_null()));
            }
            if !state.is_null() {
                let state = &mut *state;
                // Client coordinates in the message; the column rectangles
                // are recorded in the same space when they are laid out.
                let x = (lparam & 0xFFFF) as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                let hit = state
                    .column_rects
                    .iter()
                    .position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom);
                if crate::trace::on() {
                    crate::trace::line(&format!("  hit {hit:?} of {} columns at {x},{y}", state.column_rects.len()));
                }
                if let Some(index) = hit {
                    // To the screen, because the panel is a window of its own
                    // and anchors to where the column actually is. A popup
                    // has no frame, so its window origin is its client origin.
                    let mut window: RECT = std::mem::zeroed();
                    GetWindowRect(hwnd, &mut window);
                    let r = state.column_rects[index];
                    state.pending = Some(Command::Column {
                        index,
                        rect: crate::settings_ui::geometry::PxRect {
                            left: window.left + r.left,
                            top: window.top + r.top,
                            right: window.left + r.right,
                            bottom: window.top + r.bottom,
                        },
                    });
                }
            }
            0
        }
        WM_RBUTTONUP => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StripState;
            if !state.is_null() {
                (*state).pending = show_menu(hwnd);
            }
            0
        }
        // The right-click menu's rows, which are drawn here rather than by
        // the shell: a stock menu is light whatever the taskbar's theme, and
        // it is the one surface left that would not match the rest.
        WM_MEASUREITEM => {
            let item = lparam as *mut MEASUREITEMSTRUCT;
            if !item.is_null() && (*item).CtlType == ODT_MENU {
                menu_measure(hwnd, &mut *item);
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_DRAWITEM => {
            let item = lparam as *const DRAWITEMSTRUCT;
            if !item.is_null() && (*item).CtlType == ODT_MENU {
                menu_draw(&*item);
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        // Restart Manager, which is what the installer uses to get the old
        // copy out of the way before it overwrites the files. It asks a
        // top-level window to close and waits.
        //
        // Without this the default handler took it: DefWindowProcW destroys the
        // window on WM_CLOSE, so the strip vanished and the *process* carried
        // on with no window at all - still holding barometer.exe and the sensor
        // helper open, which is exactly what Restart Manager was asking it to
        // stop doing. The installer then reported that it could not close the
        // application, having been told that it had.
        //
        // Reported as a command rather than acted on here, so shutdown goes
        // through the same path as Quit from the menu: the main loop returns,
        // which is what hands the tray placeholders back and lets the sensor
        // helper's job object take the .NET process down with it. Tearing the
        // window down from inside its own procedure would skip all of that and
        // leave the placeholders in the tray until the shell noticed.
        WM_CLOSE => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StripState;
            if !state.is_null() {
                (*state).pending = Some(Command::Quit);
            }
            0
        }
        // Sign-out and shutdown. Answering TRUE says we will not hold the
        // session up; the session then ends whether or not we have finished, so
        // the quit is set here rather than waiting for WM_ENDSESSION.
        WM_QUERYENDSESSION => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut StripState;
            if !state.is_null() {
                (*state).pending = Some(Command::Quit);
            }
            1
        }
        WM_DESTROY => {
            STRIP.store(0, std::sync::atomic::Ordering::Release);
            for slot in [&RESTACK_HOOK, &LAYOUT_HOOK, &FOREGROUND_HOOK] {
                let hook = slot.swap(0, std::sync::atomic::Ordering::AcqRel) as HWINEVENTHOOK;
                if !hook.is_null() {
                    UnhookWinEvent(hook);
                }
            }
            let state = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut StripState;
            if !state.is_null() {
                let mut state = Box::from_raw(state);
                if let Some(font) = state.font.take() {
                    DeleteObject(font as _);
                }
                if let Some(font) = state.heading_font.take() {
                    DeleteObject(font as _);
                }
                if let Some(font) = state.icon_font.take() {
                    DeleteObject(font as _);
                }
                if let Some(font) = state.accent_font.take() {
                    DeleteObject(font as _);
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Converts a screen point into the taskbar's client space.
///
/// The reserved region is reported in screen coordinates, and the readout is a
/// child of the taskbar, so one has to be expressed in the other's terms.
pub fn screen_to_taskbar(x: i32, y: i32) -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;

    let taskbar = taskbar_window();
    if taskbar.is_null() {
        return None;
    }
    let mut point = POINT { x, y };
    // SAFETY: taskbar is live and point is ours.
    let ok = unsafe { ScreenToClient(taskbar, &mut point) };
    (ok != 0).then_some((point.x, point.y))
}

/// The DPI of the monitor the taskbar is on.
///
/// The reservation needs it to seed its estimate of the icon pitch before it
/// has measured one, and again whenever the taskbar moves to a display of a
/// different scale. Falls back to 96 when there is no taskbar to ask, which is
/// only true while Explorer is restarting.
pub fn taskbar_dpi() -> u32 {
    let taskbar = taskbar_window();
    if taskbar.is_null() {
        return 96;
    }
    // SAFETY: a live window handle; the call has no side effects.
    let dpi = unsafe { GetDpiForWindow(taskbar) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

/// Diagnostic only: where the strip really is, and whether Windows agrees it
/// is visible.
pub fn describe(hwnd: HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindowVisible};
    // SAFETY: a live handle; both calls are read-only.
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut rect);
        format!(
            "screen {},{}-{},{} visible={} paints={}",
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            IsWindowVisible(hwnd) != 0,
            PAINTS.load(std::sync::atomic::Ordering::Relaxed)
        )
    }
}

/// How large the weather mark is drawn, given the height of a line of text.
///
/// A little larger than the type it sits beside. An icon matched exactly to
/// the cap height reads as smaller than the text next to it, because a letter
/// fills its box and a glyph with a cloud in it does not.
fn icon_size(line_height: i32) -> i32 {
    ((line_height as f32) * 1.05).round().max(8.0) as i32
}

/// The icon font at a pixel size, reusing the last one while it still fits.
///
/// Fonts are a limited GDI resource and creating one per paint is how a
/// long-running program runs a desktop out of handles.
unsafe fn ensure_icon_font(state: &mut StripState, px: i32) -> HFONT {
    if let Some(font) = state.icon_font {
        if state.icon_px == px {
            return font;
        }
        DeleteObject(font as _);
    }
    let font = create_icon_font(px);
    state.icon_font = Some(font);
    state.icon_px = px;
    font
}

/// The same, for the smaller accent glyph.
unsafe fn ensure_accent_font(state: &mut StripState, px: i32) -> HFONT {
    if let Some(font) = state.accent_font {
        if state.accent_px == px {
            return font;
        }
        DeleteObject(font as _);
    }
    let font = create_icon_font(px);
    state.accent_font = Some(font);
    state.accent_px = px;
    font
}

/// Segoe Fluent Icons at a pixel size, falling back to Segoe MDL2 Assets.
///
/// The fallback matters on Windows 10, where Fluent is not present and MDL2
/// carries nearly the same weather glyphs at the same codepoints. GDI picks a
/// substitute silently when a family is missing, so naming the fallback is the
/// difference between the right mark and whatever the substitution table felt
/// like.
unsafe fn create_icon_font(px: i32) -> HFONT {
    let mut logical: LOGFONTW = std::mem::zeroed();
    // Positive height, because an icon font's glyphs are designed against the
    // em box rather than against a cap height.
    logical.lfHeight = px;
    logical.lfWeight = FW_NORMAL as i32;
    let family = if font_exists(glyph::FAMILY) { glyph::FAMILY } else { glyph::FALLBACK_FAMILY };
    for (index, unit) in wide(family).iter().take(logical.lfFaceName.len()).enumerate() {
        logical.lfFaceName[index] = *unit;
    }
    CreateFontIndirectW(&logical)
}

/// Whether a font family is installed.
unsafe fn font_exists(family: &str) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{
        EnumFontFamiliesExW, GetDC, ReleaseDC, LOGFONTW as EnumFont, DEFAULT_CHARSET,
    };

    unsafe extern "system" fn found(
        _font: *const EnumFont,
        _metric: *const std::ffi::c_void,
        _kind: u32,
        found: LPARAM,
    ) -> i32 {
        *(found as *mut bool) = true;
        // Stop at the first match; one is all the question needs.
        0
    }

    let dc = GetDC(std::ptr::null_mut());
    if dc.is_null() {
        return false;
    }
    let mut probe: EnumFont = std::mem::zeroed();
    probe.lfCharSet = DEFAULT_CHARSET;
    for (index, unit) in wide(family).iter().take(probe.lfFaceName.len()).enumerate() {
        probe.lfFaceName[index] = *unit;
    }
    let mut present = false;
    EnumFontFamiliesExW(
        dc,
        &probe,
        Some(std::mem::transmute::<
            unsafe extern "system" fn(*const EnumFont, *const std::ffi::c_void, u32, LPARAM) -> i32,
            _,
        >(found)),
        &mut present as *mut bool as LPARAM,
        0,
    );
    ReleaseDC(std::ptr::null_mut(), dc);
    present
}


/// The mark size for a model, or zero when it has no weather column.
///
/// Measured from the text the weather column actually draws, so the mark
/// tracks the type size rather than a constant that would be wrong the moment
/// somebody changes the font.
unsafe fn mark_pixels(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    model: &StripModel,
) -> i32 {
    let Some(column) = model.columns.iter().find(|c| c.badge.is_some()) else {
        return 0;
    };
    let line = measure(dc, &wide_no_nul(&column.bottom)).cy;
    icon_size(line)
}

/// The accent size for a model, given its mark size.
///
/// Read from the condition rather than assumed, because the two accent
/// placements use slightly different scales and a font built for the wrong one
/// puts the glyph a pixel or two off where the composition intends.
fn accent_pixels(model: &StripModel, mark_px: i32) -> i32 {
    model
        .columns
        .iter()
        .find_map(|c| c.badge)
        .and_then(|condition| glyph::composed(condition).accent)
        .map(|accent| ((mark_px as f32) * accent.scale).round() as i32)
        .unwrap_or(mark_px)
}


/// How wide the columns come out, laid out end to end.
///
/// Measured with the same helper the painter uses, so the two cannot disagree
/// about where the block starts and end up drawing it off-center.
unsafe fn content_extent(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    state: &StripState,
    gap: i32,
    scale: f32,
    mark_px: i32,
    heading_font: HFONT,
) -> i32 {
    let mut total = 0;
    for (index, column) in state.model.columns.iter().enumerate() {
        if index > 0 {
            total += gap;
        }
        total +=
            column_extents(dc, column, state.model.two_rows, scale, mark_px, heading_font).0;
    }
    total
}


/// The label for a menu row, by the id Windows hands back.
fn menu_label(id: u32) -> Option<&'static str> {
    MENU_ROWS.iter().flatten().find(|(row, _)| *row == id).map(|(_, label)| *label)
}

/// The interface font at 9 points for the menu: Segoe UI Variable Text, which
/// is what the shell's own menus use.
unsafe fn menu_font(dpi: u32) -> HFONT {
    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfHeight = -((12.0 * dpi as f32 / 96.0).round() as i32);
    lf.lfWeight = FW_NORMAL as i32;
    lf.lfCharSet = DEFAULT_CHARSET as u8;
    lf.lfOutPrecision = OUT_TT_PRECIS as u8;
    lf.lfClipPrecision = CLIP_DEFAULT_PRECIS as u8;
    lf.lfQuality = ANTIALIASED_QUALITY as u8;
    let face = wide("Segoe UI Variable Text");
    let n = face.len().min(lf.lfFaceName.len() - 1);
    lf.lfFaceName[..n].copy_from_slice(&face[..n]);
    CreateFontIndirectW(&lf)
}

/// How big a menu row is: WinUI's 32-DIP item around the label, or the
/// separator's 9.
unsafe fn menu_measure(hwnd: HWND, item: &mut MEASUREITEMSTRUCT) {
    let Some(look) = MENU_LOOK.lock().ok().and_then(|look| look.clone()) else { return };
    let Some(label) = menu_label(item.itemID) else {
        item.itemWidth = 0;
        item.itemHeight = look.px(MENU_SEPARATOR_DIP) as u32;
        return;
    };
    let dc = GetDC(hwnd);
    let previous = SelectObject(dc, look.font as _);
    let text: Vec<u16> = label.encode_utf16().collect();
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
    SelectObject(dc, previous);
    ReleaseDC(hwnd, dc);
    item.itemWidth = (size.cx + 2 * look.px(MENU_PAD_DIP)) as u32;
    item.itemHeight = look.px(MENU_ROW_DIP) as u32;
}

/// Draws one menu row: the ground, a rounded plate under the row the mouse
/// is on, and the label; or a hairline for a separator.
unsafe fn menu_draw(item: &DRAWITEMSTRUCT) {
    let Some(look) = MENU_LOOK.lock().ok().and_then(|look| look.clone()) else { return };
    let dc = item.hDC;
    let theme = &look.theme;
    // The row paints its own background, because with MF_OWNERDRAW nothing
    // else does: Windows hands over the item rectangle and whatever is in it
    // is whatever was in the buffer. Leaving it unpainted to let the menu's
    // own backdrop through does not reveal a backdrop, it reveals garbage -
    // rows came up as dark plates over a white menu with white text on them.
    let ground = CreateSolidBrush(theme.surface_card.colorref());
    FillRect(dc, &item.rcItem, ground);
    DeleteObject(ground as _);

    let Some(label) = menu_label(item.itemID) else {
        let inset = look.px(8.0);
        let y = (item.rcItem.top + item.rcItem.bottom) / 2;
        let line = RECT { left: item.rcItem.left + inset, top: y, right: item.rcItem.right - inset, bottom: y + 1 };
        let ink = CreateSolidBrush(theme.stroke_divider.colorref());
        FillRect(dc, &line, ink);
        DeleteObject(ink as _);
        return;
    };

    if item.itemState & ODS_SELECTED != 0 {
        let (dx, dy, radius) = (look.px(4.0), look.px(2.0), look.px(4.0));
        let plate = CreateSolidBrush(theme.subtle_hover.colorref());
        let previous_brush = SelectObject(dc, plate as _);
        let previous_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        RoundRect(
            dc,
            item.rcItem.left + dx,
            item.rcItem.top + dy,
            item.rcItem.right - dx,
            item.rcItem.bottom - dy,
            radius,
            radius,
        );
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous_brush);
        DeleteObject(plate as _);
    }

    let previous = SelectObject(dc, look.font as _);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, theme.text_primary.colorref());
    let text: Vec<u16> = label.encode_utf16().collect();
    let pad = look.px(MENU_PAD_DIP);
    let mut bounds = RECT {
        left: item.rcItem.left + pad,
        top: item.rcItem.top,
        right: item.rcItem.right - pad,
        bottom: item.rcItem.bottom,
    };
    DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut bounds, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    SelectObject(dc, previous);
}

/// Asks DWM for a Windows 11 menu: Mica behind it, and rounded corners.
///
/// The menu is the system's own window, so nothing about it can be set at
/// creation; this is called with its handle the first time the menu loop goes
/// idle, which is as soon as it is on screen. Both attributes are advisory -
/// a build that does not know them refuses and the menu is simply the plain
/// one - so neither answer is checked.
unsafe fn dress_menu_window(menu: HWND) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWM_SYSTEMBACKDROP_TYPE,
    };

    if menu.is_null() {
        return;
    }
    // The transient backdrop rather than the main-window one: this is what
    // Windows uses for the things that appear over an app and go away again,
    // and it is what the shell's own menus are drawn on.
    let backdrop: DWM_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW;
    // SAFETY: a live window handle and a value of the size the attribute wants.
    DwmSetWindowAttribute(
        menu,
        DWMWA_SYSTEMBACKDROP_TYPE as u32,
        &backdrop as *const _ as *const _,
        std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
    );
    let round = DWMWCP_ROUND;
    DwmSetWindowAttribute(
        menu,
        DWMWA_WINDOW_CORNER_PREFERENCE as u32,
        &round as *const _ as *const _,
        std::mem::size_of_val(&round) as u32,
    );
}

/// Pops the right-click menu on its own, at a fixed place, for looking at it.
///
/// The menu is the hardest thing in this program to photograph. It belongs to
/// the system, it only exists while `TrackPopupMenu` is blocking, and the
/// process is manifested to require administrator - so injected clicks from an
/// unelevated capture script are dropped by Windows before they reach the
/// strip, and the menu never opens at all. This pops it directly, with an
/// owner of its own, so a screenshot has something to take.
pub fn preview_menu(x: i32, y: i32) {
    // A class of its own rather than a predefined one: WM_MEASUREITEM and
    // WM_DRAWITEM are sent to the menu's *owner*, so a STATIC window leaves
    // every row unmeasured and the menu comes up a few pixels wide.
    // SAFETY: a class registered once, a window destroyed below, and a menu
    // the callee owns.
    unsafe {
        let class = wide("BarometerMenuPreview");
        let registered = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(preview_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: std::ptr::null_mut(),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&registered);
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if owner.is_null() {
            return;
        }
        windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
        show_menu(owner);
        DestroyWindow(owner);
    }
}

/// Measures and draws the preview's rows the way the strip's own procedure
/// does, which is the whole reason the preview needs a procedure at all.
unsafe extern "system" fn preview_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_MEASUREITEM => {
            let item = lparam as *mut MEASUREITEMSTRUCT;
            if !item.is_null() && (*item).CtlType == ODT_MENU {
                menu_measure(hwnd, &mut *item);
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_DRAWITEM => {
            let item = lparam as *const DRAWITEMSTRUCT;
            if !item.is_null() && (*item).CtlType == ODT_MENU {
                menu_draw(&*item);
                return 1;
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_ENTERIDLE => {
            if wparam as u32 == MSGF_MENU {
                dress_menu_window(lparam as HWND);
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Pops the right-click menu and returns what was chosen.
///
/// Built each time rather than kept, because it is shown at most a few times a
/// session and a menu that lives for the life of the process is a handle held
/// for nothing.
unsafe fn show_menu(hwnd: HWND) -> Option<Command> {
    use windows_sys::Win32::Foundation::POINT;

    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }

    // The menu follows the taskbar's theme, as the strip does, in the
    // interface font at the taskbar's DPI.
    let theme = crate::settings_ui::system::current_theme();
    let dpi = GetDpiForWindow(hwnd).max(96);
    let font = menu_font(dpi);
    let ground = CreateSolidBrush(theme.surface_card.colorref());
    if let Ok(mut look) = MENU_LOOK.lock() {
        *look = Some(MenuLook { theme, font: font as isize, dpi });
    }
    // The background is the menu's own so the margins around the rows match
    // them. It was briefly left to the system, on the theory that a menu
    // painted by Windows would carry the Windows 11 backdrop - it does not,
    // for an owner-drawn menu: see menu_draw. No check column: nothing here
    // is checkable and the gutter would sit empty.
    let info = MENUINFO {
        cbSize: std::mem::size_of::<MENUINFO>() as u32,
        fMask: MIM_BACKGROUND | MIM_STYLE,
        dwStyle: MNS_NOCHECK,
        cyMax: 0,
        hbrBack: ground,
        dwContextHelpID: 0,
        dwMenuData: 0,
    };
    SetMenuInfo(menu, &info);
    for row in MENU_ROWS {
        match row {
            Some((id, _)) => AppendMenuW(menu, MF_OWNERDRAW, id as usize, std::ptr::null()),
            None => AppendMenuW(menu, MF_OWNERDRAW | MF_SEPARATOR, 0, std::ptr::null()),
        };
    }

    let mut point: POINT = std::mem::zeroed();
    GetCursorPos(&mut point);

    // Foreground first, or the menu will not close when the user clicks away
    // from it - a documented quirk of TrackPopupMenu that leaves a menu
    // stranded on screen, and one this window is especially prone to because
    // it is WS_EX_NOACTIVATE and never becomes foreground on its own.
    SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    DestroyMenu(menu);
    DeleteObject(ground as _);
    if let Ok(mut look) = MENU_LOOK.lock() {
        *look = None;
    }
    DeleteObject(font as _);

    match chosen as u32 {
        MENU_SETTINGS => Some(Command::OpenSettings),
        MENU_UPDATES => Some(Command::CheckForUpdates),
        MENU_QUIT => Some(Command::Quit),
        // Zero, which is what dismissing the menu returns.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every weight the picker offers has to reach the taskbar as a
    /// different face, at the size a taskbar actually draws.
    ///
    /// This is the test the font settings never had. GDI never refuses a
    /// weight: asked for one the family has no face for it silently draws
    /// the nearest, so "Medium" on Segoe UI was Regular pixel for pixel and
    /// the setting looked inert. Twelve DIPs at 144 DPI is eighteen real
    /// pixels, which is what this measures at - at ten the hinting snaps
    /// regular and semibold onto the same stems and even a correct build
    /// cannot tell them apart.
    #[test]
    fn every_weight_the_picker_offers_reaches_the_taskbar_as_a_different_face() {
        let solid = |weight| ink_detail(weight, "Segoe UI", 18.0, true).0;
        let (light, regular, semibold, bold) = (solid(300), solid(400), solid(600), solid(700));
        eprintln!("solid stem pixels: light {light}, regular {regular}, semibold {semibold}, bold {bold}");
        assert!(light < regular, "light {light} is not lighter than regular {regular}");
        assert!(regular < semibold, "semibold {semibold} is not heavier than regular {regular}");
        assert!(semibold < bold, "bold {bold} is not heavier than semibold {semibold}");

        // And the one the machine has no face for is known to have none,
        // which is what the picker says out loud instead of drawing regular
        // and calling it medium.
        let installed = installed_families();
        assert!(!crate::settings_ui::gdi::has_weight("Segoe UI", 500, installed));
        assert_eq!(solid(500), regular, "medium is regular on a family with no medium");
        for weight in [300, 400, 600, 700] {
            assert!(crate::settings_ui::gdi::has_weight("Segoe UI", weight, installed), "{weight}");
        }
    }

    /// Solid stem pixels and total coverage for one face at one size.
    ///
    /// Two numbers because they disagree: a well-hinted face snaps its stems
    /// to whole pixels and spills less gray, so it can total *less* coverage
    /// than a lighter face while plainly looking heavier. The count of solid
    /// pixels is what tracks perceived weight.
    fn ink_detail(weight: i32, family: &str, size_dip: f32, resolve: bool) -> (u64, u64) {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleDC, DeleteDC, SelectObject, SetBkMode, SetTextColor, TextOutW,
        };
        // SAFETY: a DIB, a DC and a font, all released before returning.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let (w, h) = (420, 60);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let previous_bitmap = SelectObject(dc, bitmap as _);
            let mut logical: LOGFONTW = std::mem::zeroed();
            logical.lfHeight = -dip_to_px(size_dip, 96);
            logical.lfWeight = weight;
            logical.lfCharSet = DEFAULT_CHARSET;
            logical.lfOutPrecision = OUT_TT_PRECIS;
            logical.lfClipPrecision = CLIP_DEFAULT_PRECIS;
            logical.lfQuality = ANTIALIASED_QUALITY;
            let resolved = if resolve {
                crate::settings_ui::gdi::instance_family(family, weight, installed_families())
            } else {
                family.to_string()
            };
            let wide_family = wide(&resolved);
            for (index, unit) in wide_family.iter().take(logical.lfFaceName.len()).enumerate() {
                logical.lfFaceName[index] = *unit;
            }
            let font = CreateFontIndirectW(&logical);
            let previous_font = SelectObject(dc, font as _);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00FF_FFFF);
            let text = wide_no_nul("MEM 45% CPU 12%");
            TextOutW(dc, 4, 8, text.as_ptr(), text.len() as i32);
            let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            let mut solid = 0u64;
            let mut total = 0u64;
            for p in pixels.chunks_exact(4) {
                let v = p[0].max(p[1]).max(p[2]);
                total += u64::from(v);
                if v > 200 {
                    solid += 1;
                }
            }
            SelectObject(dc, previous_font);
            DeleteObject(font as _);
            SelectObject(dc, previous_bitmap);
            DeleteObject(bitmap as _);
            DeleteDC(dc);
            (solid, total)
        }
    }

}
