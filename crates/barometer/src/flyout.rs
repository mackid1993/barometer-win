// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The flyouts: the detail panel that opens when a column on the strip is
// clicked, ported from Sources/MenuBarStatsUI/Dropdown/.
//
// On the Mac every module's status item has a dropdown of its own, and the
// eight of them share one shape: a panel that opens under the item
// (AttachedPanel, PopoverPlacement), closes when the user goes elsewhere
// (PopoverDismissalMonitor), and is filled from one vocabulary of cards,
// rows, tiles and graphs (DropdownViews). This module is that shape. One
// panel exists for the whole strip - the Windows strip is one window, so
// there is one place a panel can hang off - and each module contributes a
// `Content` that says what goes in it. Weather was the first; each of the
// other six is one file implementing the trait and one `register` call.
//
// The panel is a top-level, topmost, layered window on a thread of its own,
// for the reasons window.rs and settings_ui.rs each give: it cannot be a
// child of the taskbar, because the taskbar composites its children away,
// and it cannot share the strip's thread, because painting a panel of
// graphs would stall the tick that samples the machine. The two threads
// meet through `Shared`, exactly as the settings window does, and through
// `Feed`s, which are how a module's live readings reach its content.

pub mod clipboard;
pub mod cpu;
pub mod dismiss;
pub mod disks;
pub mod gpu;
pub mod memory;
pub mod network;
pub mod paint;
pub mod panel;
pub mod placement;
#[cfg(test)]
pub mod render;
pub mod sensors;
pub mod stack;
pub mod appicon;
pub mod ui;
pub mod weather;

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use barometer_core::stack::StripItem;
use barometer_core::ModuleId;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::settings_ui::geometry::PxRect;

pub use ui::{
    Accent, Builder, Element, Graph, Id, Ink, Kind, Measure, Painter, Palette, Style, Tile, TileIcon,
};

/// The strip column a panel opens from, in screen pixels.
///
/// The strip records where each column landed (`StripState::column_rects`)
/// and that, converted to the screen, is all a panel needs to know about
/// the thing it belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub column: PxRect,
}

/// What a panel asked the application to do.
///
/// Returned to the strip's thread rather than acted on here, for the same
/// reason the strip's own right-click menu does it that way: opening the
/// settings window is that loop's business.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The footer's gear, or a row that needs the user to set something up.
    OpenSettings(ModuleId),
    /// A panel's Refresh: the module behind it should read again now.
    Refresh(ModuleId),
    /// The weather panel's location row: show this saved location, by its
    /// identity. A settings change, so the thread that owns the settings
    /// makes it.
    ShowLocation(String),
}

/// What a content wants done after a control was used.
#[derive(Debug, PartialEq, Eq)]
pub enum Response {
    None,
    /// Redraw what is laid out.
    Repaint,
    /// Lay the panel out again; something it shows changed shape.
    Relayout,
    /// Hand the strip's thread a command and close: the panel is done.
    Command(Command),
    /// Hand the strip's thread a command and stay open, laid out again:
    /// the panel is waiting on what the command brings.
    Request(Command),
    Close,
}

/// A button in the footer, at the right, which the content answers for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: Id,
    pub text: &'static str,
    pub glyph: Option<&'static str>,
    pub enabled: bool,
}

/// What a content is given to lay itself out with.
pub struct Context<'a> {
    /// The panel's full width, in DIPs.
    pub width: f32,
    pub palette: &'a Palette,
    pub accent: Accent,
    pub measure: Measure<'a>,
    /// What the pointer is on, for content that changes shape under it.
    pub hover: Option<Id>,
    /// The system clock, so that a layout is a pure function of its inputs
    /// and a test can hand one in.
    pub now_unix: i64,
}

impl<'a> Context<'a> {
    /// A builder starting inside the panel's padding.
    pub fn builder(&self) -> Builder<'a> {
        Builder::new(ui::PANEL_PAD, ui::PANEL_PAD, self.width - 2.0 * ui::PANEL_PAD, self.measure)
    }
}

/// One laid-out page of content.
pub struct Page {
    pub elements: Vec<Element>,
    /// The content's height including the panel's padding.
    pub height: f32,
}

impl Page {
    /// The page a builder has laid out, closed with the panel's padding.
    pub fn finish(builder: Builder<'_>) -> Page {
        Page { height: builder.y() + ui::PANEL_PAD, elements: builder.elements }
    }
}

/// What a module puts in the panel.
///
/// Every method but `module` and `build` has a default that does nothing,
/// because most content is a picture of the module's last reading and
/// nothing more: it lays itself out, and the chrome does the rest.
pub trait Content: Send {
    /// The module whose settings the footer's gear opens and whose accent
    /// the panel takes. For a stack's content this is the module of the
    /// reading it is showing, which changes as the user switches.
    fn module(&self) -> ModuleId;

    /// The strip item this content answers for, which is what the panel
    /// keys its contents by and what a click on a column asks for. A
    /// module's content is its module; a stack's is the stack, because two
    /// stacks can carry the same modules and each needs a panel of its own.
    fn item(&self) -> StripItem {
        StripItem::Module(self.module())
    }

    /// The panel's accent while this content is showing.
    fn accent(&self) -> Accent {
        Accent::signature(self.module())
    }

    /// Lays the content out. Called on the panel's thread whenever anything
    /// it shows may have changed, and never while it is hidden.
    fn build(&mut self, cx: &Context) -> Page;

    /// A control the content laid out was used.
    fn activate(&mut self, id: Id) -> Response {
        let _ = id;
        Response::None
    }

    /// The pointer moved onto or off a control. True when the content
    /// changes shape under the pointer and needs laying out again.
    fn hover(&mut self, id: Option<Id>) -> bool {
        let _ = id;
        false
    }

    /// A key the chrome did not take. Escape closes the panel before the
    /// content is asked.
    fn key(&mut self, key: u16) -> Response {
        let _ = key;
        Response::None
    }

    /// The panel's thread adopted this content. A content with a worker of
    /// its own keeps the handle and wakes the panel when the worker is done.
    fn attach(&mut self, wake: Wake) {
        let _ = wake;
    }

    /// The panel opened showing this content.
    fn opened(&mut self) {}

    /// The panel closed, or switched to another module's content.
    fn closed(&mut self) {}

    /// Something outside the content changed: a feed published, a worker
    /// finished, or the panel's clock ticked. True to lay out again.
    fn tick(&mut self) -> bool {
        false
    }

    /// The footer's buttons.
    fn actions(&self) -> Vec<Action> {
        Vec::new()
    }

    /// Opens the content on one sensor, by the source's identifier, or on
    /// all of them. Only the sensors content does anything with it; a
    /// stack's tab for a sensor reading is what asks.
    fn focus_sensor(&mut self, id: Option<&str>) {
        let _ = id;
    }
}

/// A request from the strip's thread to the panel's.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub item: StripItem,
    pub anchor: Anchor,
    /// Close instead, if this item's panel is the one already open.
    pub toggle: bool,
}

/// What the two threads share.
pub(crate) struct Shared {
    /// The panel's handle as an integer so it can cross threads; zero while
    /// there is no window.
    pub hwnd: AtomicIsize,
    pub open: AtomicBool,
    pub open_item: Mutex<Option<StripItem>>,
    /// Contents registered but not yet adopted by the panel's thread.
    pub pending: Mutex<Vec<Box<dyn Content>>>,
    pub request: Mutex<Option<Request>>,
    pub commands: Mutex<Vec<Command>>,
}

impl Shared {
    fn post(&self, message: u32) {
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: a handle the panel thread published; posting to a
            // window that has since gone is harmless.
            unsafe { PostMessageW(hwnd as HWND, message, 0, 0) };
        }
    }
}

pub(crate) const WM_APP_OPEN: u32 = WM_APP + 21;
pub(crate) const WM_APP_CLOSE: u32 = WM_APP + 22;
pub(crate) const WM_APP_CHANGED: u32 = WM_APP + 23;
pub(crate) const WM_APP_REGISTER: u32 = WM_APP + 24;
pub(crate) const WM_APP_QUIT: u32 = WM_APP + 25;

/// Wakes the panel: something a content shows has changed.
///
/// Cheap to clone and safe from any thread, which is the point: a content's
/// fetch worker finishes on a thread of its own and has to tell the panel
/// so, and a module's feed publishes from the strip's tick.
#[derive(Clone)]
pub struct Wake {
    shared: Arc<Shared>,
}

impl Wake {
    pub fn notify(&self) {
        self.shared.post(WM_APP_CHANGED);
    }
}

/// A module's live readings, published from the strip's thread and read by
/// its content on the panel's.
///
/// The slot is a plain mutex rather than a channel because the panel only
/// ever wants the latest value: a sample that arrived while the panel was
/// hidden is not worth queuing, and the one after it supersedes it.
pub struct Feed<T> {
    slot: Arc<Mutex<T>>,
    wake: Wake,
}

impl<T> Feed<T> {
    /// Replaces the value and wakes the panel if it is showing.
    pub fn publish(&self, value: T) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = value;
        }
        if self.wake.shared.open.load(Ordering::Acquire) {
            self.wake.notify();
        }
    }

    /// The content's end of the feed.
    pub fn slot(&self) -> Arc<Mutex<T>> {
        Arc::clone(&self.slot)
    }
}

/// The application's handle on the panel.
///
/// Creating it starts the panel's thread with the window hidden, so the
/// first click on the strip opens a panel that already exists; dropping it
/// destroys the window and joins the thread.
pub struct Flyout {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Default for Flyout {
    fn default() -> Self {
        Flyout::new()
    }
}

impl Flyout {
    pub fn new() -> Flyout {
        let shared = Arc::new(Shared {
            hwnd: AtomicIsize::new(0),
            open: AtomicBool::new(false),
            open_item: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
            request: Mutex::new(None),
            commands: Mutex::new(Vec::new()),
        });
        let thread = thread::Builder::new()
            .name("flyout".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || panel::run(shared)
            })
            .ok();
        Flyout { shared, thread }
    }

    /// Adds a module's content. The panel adopts it on its own thread.
    pub fn register(&self, content: Box<dyn Content>) {
        if let Ok(mut pending) = self.shared.pending.lock() {
            pending.push(content);
        }
        self.shared.post(WM_APP_REGISTER);
    }

    /// A feed for a content to read from.
    pub fn feed<T: Send + 'static>(&self, initial: T) -> Feed<T> {
        Feed { slot: Arc::new(Mutex::new(initial)), wake: self.wake() }
    }

    /// A handle for waking the panel from anywhere.
    pub fn wake(&self) -> Wake {
        Wake { shared: Arc::clone(&self.shared) }
    }

    fn request(&self, item: StripItem, anchor: Anchor, toggle: bool) {
        if let Ok(mut request) = self.shared.request.lock() {
            *request = Some(Request { item, anchor, toggle });
        }
        self.shared.post(WM_APP_OPEN);
    }

    /// Opens a module's panel at a column, or closes it if that module's
    /// panel is the one already open - which is what a second click on the
    /// same column means.
    pub fn toggle(&self, module: ModuleId, anchor: Anchor) {
        self.toggle_item(StripItem::Module(module), anchor);
    }

    /// Opens a stack's panel at one of its columns, or closes it if that
    /// stack's panel is the one already open. The stack must have been
    /// registered as a `stack::StackContent` with this id.
    pub fn toggle_stack(&self, id: u32, anchor: Anchor) {
        self.toggle_item(StripItem::Stack(id), anchor);
    }

    /// `toggle` for either kind of strip item.
    pub fn toggle_item(&self, item: StripItem, anchor: Anchor) {
        self.request(item, anchor, true);
    }

    /// Opens a module's panel at a column, swapping out whatever was open.
    pub fn open(&self, module: ModuleId, anchor: Anchor) {
        self.request(StripItem::Module(module), anchor, false);
    }

    pub fn close(&self) {
        self.shared.post(WM_APP_CLOSE);
    }

    pub fn is_open(&self) -> bool {
        self.shared.open.load(Ordering::Acquire)
    }

    /// Which item's panel is showing, so the strip can draw that item's
    /// columns with their open backplate.
    pub fn open_item(&self) -> Option<StripItem> {
        if !self.is_open() {
            return None;
        }
        self.shared.open_item.lock().ok().and_then(|item| *item)
    }

    /// Which module's panel is showing. None while a stack's panel is the
    /// one open, even though that panel is showing some module's content:
    /// the strip lights the column that was clicked, not its source.
    pub fn open_module(&self) -> Option<ModuleId> {
        match self.open_item() {
            Some(StripItem::Module(module)) => Some(module),
            _ => None,
        }
    }

    /// Takes the next thing a panel asked for, if anything.
    pub fn take_command(&self) -> Option<Command> {
        let mut commands = self.shared.commands.lock().ok()?;
        if commands.is_empty() {
            None
        } else {
            Some(commands.remove(0))
        }
    }
}

impl Drop for Flyout {
    fn drop(&mut self) {
        self.shared.post(WM_APP_QUIT);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
