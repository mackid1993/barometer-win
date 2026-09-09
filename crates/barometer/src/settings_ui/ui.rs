// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The controls: what they are, where they go, and how they are drawn.
//
// Stock Win32 controls cannot be made to look like Windows 11's, so every
// control here is a rectangle the window draws and hit-tests itself, the way
// Clicker does with its own Fluent theme. The one exception is the text
// field, which borrows a child EDIT for its caret, selection and IME and
// draws everything around it.
//
// Layout is a list of `Element`s in DIPs, rebuilt from the model whenever
// anything changes and then used for both drawing and hit testing, so the two
// can never disagree about where a control is. The arithmetic that decides
// where things go is in `Builder` and has no handle to a window, which is
// what makes it testable.

use barometer_core::module::ModuleId;

use super::gdi::{Align, Canvas, TextStyle};
use super::geometry::Rect;
use super::icon;
use super::model::StripItem;
use super::theme::{glyph, Color, Theme};

/// The navigation pane's entries.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pane {
    Strip,
    /// The sensor stack: is LibreHardwareMonitor here, install it, remove it,
    /// what PawnIO's state is.
    ///
    /// Its own pane rather than a section inside the Sensors module's
    /// inspector, which is where it started. All of it is a property of the
    /// machine and not of one module's appearance on the strip, and somebody
    /// whose temperatures do not work looks in the sidebar - not inside a
    /// module they may well have switched off.
    /// One module's own page: what it puts on the strip, what it contributes
    /// to a stack, and whatever else belongs to that module alone - the
    /// sensor source under Sensors, the locations and units under Weather.
    ///
    /// A page each, as `SettingsWindowController.swift` gives every module a
    /// sidebar row. They were inspectors inside the Strip pane, which meant
    /// a module's options were reachable only by selecting its row, were
    /// invisible to anyone who had switched that module off and wanted to
    /// know why, and were capped at the width of a 240-DIP column - which is
    /// why settings like the network's byte-or-bit choice had nowhere to go
    /// and ended up unreachable.
    Module(ModuleId),
    /// The stacks: what each one shows, in what order, under what name.
    Stacks,
    Appearance,
    General,
    About,
}

impl Pane {
    pub const ALL: [Pane; 12] = [
        Pane::Strip,
        Pane::Module(ModuleId::Cpu),
        Pane::Module(ModuleId::Gpu),
        Pane::Module(ModuleId::Memory),
        Pane::Module(ModuleId::Disks),
        Pane::Module(ModuleId::Network),
        Pane::Module(ModuleId::Sensors),
        Pane::Module(ModuleId::Weather),
        Pane::Stacks,
        Pane::Appearance,
        Pane::General,
        Pane::About,
    ];

    /// The modules, in the order the sidebar lists them.
    pub const MODULES: [Pane; 7] = [
        Pane::Module(ModuleId::Cpu),
        Pane::Module(ModuleId::Gpu),
        Pane::Module(ModuleId::Memory),
        Pane::Module(ModuleId::Disks),
        Pane::Module(ModuleId::Network),
        Pane::Module(ModuleId::Sensors),
        Pane::Module(ModuleId::Weather),
    ];

    pub fn title(self) -> &'static str {
        match self {
            Pane::Strip => "Strip",
            Pane::Module(id) => crate::settings_ui::model::module_name(id),
            Pane::Stacks => "Stacks",
            Pane::Appearance => "Appearance",
            Pane::General => "General",
            Pane::About => "About",
        }
    }

    /// The mark and color the sidebar row carries, which is what makes the
    /// window read as this app's rather than as a settings dialog.
    pub fn mark(self) -> Option<(Color, Mark)> {
        match self {
            Pane::Module(id) => Some((crate::settings_ui::theme::module_color(id), Mark::Module(id))),
            Pane::Stacks => Some((crate::settings_ui::theme::STACK_COLOR, Mark::Stack)),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        Pane::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// What a control does when it is used. Every interactive element carries
/// one, and the window's input handling is a match over these.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Id {
    None,
    Nav(Pane),
    Row(StripItem),
    RowToggle(StripItem),
    AddStack,
    Show,
    /// The network module's two rates, upload on the top line.
    UploadFirst,
    /// Whether the network panel asks the internet for the public address.
    ShowPublicIp,
    /// Which interface the network module counts.
    NetworkInterface,
    RateUnit,
    DiskDevice,
    DiskVolume,
    Family,
    /// The weight of the module names on the strip.
    HeadingWeight,
    /// The weight of the numbers under them.
    Weight,
    Size,
    Gap,
    Padding,
    GpuAdapter,
    PinnedSensor,
    PollSeconds,
    SensorUnit,
    SensorDecimals,
    LibraryDir,
    InstallLhm,
    RemoveLhm,
    GetPawnIo,
    StackName,
    Layout,
    HideSources,
    AddMetric,
    MetricUp(usize),
    MetricDown(usize),
    MetricRemove(usize),
    /// The caption a stack reading is drawn with on the strip.
    MetricLabel(usize),
    RemoveStack,
    EditStack(u32),
    AddToStack,
    Search,
    SearchResult(usize),
    LocationRemove(usize),
    LocationPrimary(usize),
    CurrentLocation,
    TempUnit,
    WindUnit,
    PressureUnit,
    PrecipUnit,
    RefreshMinutes,
    CheckUpdates,
    StartAtLogin,
    CheckNow,
    ClearSkipped,
    Link(&'static str),
    DropdownItem(usize),
    Preview,
    PreviewFlip,
    Scrollbar,
}

/// A color role, resolved against the theme at draw time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Ink {
    Primary,
    Secondary,
    Tertiary,
    Disabled,
    Accent,
    Success,
    Caution,
    Critical,
    Custom(Color),
}

impl Ink {
    pub fn color(self, theme: &Theme) -> Color {
        match self {
            Ink::Primary => theme.text_primary,
            Ink::Secondary => theme.text_secondary,
            Ink::Tertiary => theme.text_tertiary,
            Ink::Disabled => theme.text_disabled,
            Ink::Accent => theme.accent_text,
            Ink::Success => theme.status_success,
            Ink::Caution => theme.status_caution,
            Ink::Critical => theme.status_critical,
            Ink::Custom(color) => color,
        }
    }
}

/// The mark beside a module's or a stack's name.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Mark {
    /// A module's own symbol, drawn as vectors. See `icon`.
    Module(ModuleId),
    /// The mark for a stack, which is not a module and has no `ModuleId`.
    Stack,
    /// A Segoe Fluent Icons codepoint, for the marks that are not modules -
    /// a search field, a disclosure arrow, the small furniture of a pane.
    Glyph(char),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text { text: String, style: TextStyle, ink: Ink, align: Align, wrap: bool },
    Card,
    Divider,
    Toggle { on: bool, enabled: bool },
    Dropdown { text: String, enabled: bool },
    Slider { value: f32, min: f32, max: f32, step: f32 },
    Button { text: String, accent: bool, external: bool, enabled: bool },
    Link { text: String, critical: bool },
    Row { selected: bool, lifted: bool },
    NavItem { text: String, selected: bool, mark: Option<(Color, Mark)> },
    /// The word over a group of sidebar rows, as the Swift's `Section`.
    NavHeading { text: String },
    /// A module's mark in its own hue, with no tile behind it.
    Mark { color: Color, mark: Mark, dim: bool },
    Glyph { glyph: &'static str, size: f32, ink: Ink },
    Dot { ink: Ink },
    Field { text: String, placeholder: String, search: bool, active: bool },
    Preview,
    IconButton { glyph: &'static str, enabled: bool },
    ListItem { text: String, selected: bool },
    Popup,
    Progress { fraction: f32 },
    /// The application's own icon, from the executable's resources.
    AppIcon,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub id: Id,
    pub rect: Rect,
    pub kind: Kind,
    pub interactive: bool,
    pub focusable: bool,
}

// The spacing scale: 4, 8, 12, 16, 20, 24, 32, 40, 48 and nothing else, per
// docs/ui-design.md section 4.
pub const NAV_W: f32 = 176.0;
pub const PANE_PAD: f32 = 24.0;

/// A row's inset from its card, on every side.
///
/// WinUI's SettingsCard padding, and the one number that decides whether the
/// cards breathe. The Settings app spends its room at the edges of a row and
/// keeps a caption close under its label; twelve at the edges with fourteen
/// between a label and its caption, which is what was here, put the air in
/// the wrong place and read as cramped.
pub const ROW_PAD: f32 = 16.0;
/// A label to the caption under it.
const CAPTION_GAP: f32 = 4.0;
/// A control's band to the caption that has to run under it.
const BAND_GAP: f32 = 8.0;
/// The label column to the control at its right.
const COLUMN_GAP: f32 = 12.0;
/// The narrowest label column that can still hold words beside a control.
/// Under this the control moves beneath them. The alternative, seen at the
/// window's minimum width, was a label of negative width that never drew.
const LABEL_MIN_W: f32 = 120.0;
/// A label column at least this wide takes its caption too, wrapped under the
/// label, which is how a Settings row reads. Narrower, the caption runs the
/// full width of the card under the control instead of becoming a column
/// four words across.
const CAPTION_BESIDE_MIN_W: f32 = 240.0;
/// A bare row: one line of Body with `ROW_PAD` above and below.
pub const ROW_H: f32 = 2.0 * ROW_PAD + 20.0;
pub const CONTROL_H: f32 = 32.0;
pub const DROPDOWN_W: f32 = 160.0;
/// A text field beside its label. Wider than a dropdown because what goes
/// into one - a path, a name - is longer than what comes out of a list.
pub const FIELD_W: f32 = 280.0;
pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 20.0;
pub const RADIUS_CONTROL: f32 = 4.0;
pub const RADIUS_SURFACE: f32 = 8.0;
/// A list row: two lines of text with 8 above and below, denser than a card
/// row because a list is scanned, not read.
pub const LIST_ROW_H: f32 = 52.0;
pub const NAV_ITEM_H: f32 = 36.0;
/// The word over a group of sidebar rows.
pub const NAV_HEADING_H: f32 = 18.0;
pub const POPUP_ITEM_H: f32 = 36.0;
pub const SECTION_GAP: f32 = 24.0;
pub const BUTTON_MIN_W: f32 = 96.0;
pub const SCROLL_STEP: f32 = 48.0;

/// How a row arranges its words around its control, decided by the room the
/// control leaves them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Shape {
    /// Label and caption in a column at the left, the control centered on the
    /// row at the right. The Settings app's row.
    Beside,
    /// Label level with the control, the caption running under both.
    Below,
    /// The words first, then the control across the whole row.
    Stacked,
}

/// Measures text: (text, style, wrap width or 0) to (width, height) in DIPs.
pub type Measure<'m> = &'m dyn Fn(&str, TextStyle, f32) -> (f32, f32);

struct CardState {
    index: usize,
    rows: usize,
}

/// Lays elements out top to bottom in a column.
pub struct Builder<'m> {
    pub elements: Vec<Element>,
    x: f32,
    width: f32,
    y: f32,
    start_y: f32,
    card: Option<CardState>,
    measure: Measure<'m>,
}

impl<'m> Builder<'m> {
    pub fn new(x: f32, y: f32, width: f32, measure: Measure<'m>) -> Builder<'m> {
        Builder { elements: Vec::new(), x, width, y, start_y: y, card: None, measure }
    }

    pub fn y(&self) -> f32 {
        self.y
    }

    pub fn x(&self) -> f32 {
        self.x
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn advance(&mut self, dy: f32) {
        self.y += dy;
    }

    pub fn push(&mut self, element: Element) {
        self.elements.push(element);
    }

    fn passive(&mut self, rect: Rect, kind: Kind) {
        self.elements.push(Element { id: Id::None, rect, kind, interactive: false, focusable: false });
    }

    fn control(&mut self, id: Id, rect: Rect, kind: Kind) {
        self.elements.push(Element { id, rect, kind, interactive: true, focusable: true });
    }

    pub fn text(&mut self, rect: Rect, text: &str, style: TextStyle, ink: Ink, align: Align) {
        self.passive(rect, Kind::Text { text: text.to_string(), style, ink, align, wrap: false });
    }

    /// The pane's title.
    pub fn subtitle(&mut self, text: &str) {
        let rect = Rect::new(self.x, self.y, self.width, TextStyle::Subtitle.line_dip());
        self.text(rect, text, TextStyle::Subtitle, Ink::Primary, Align::Left);
        self.y += TextStyle::Subtitle.line_dip() + 4.0;
    }

    /// A wrapped caption in the secondary ink.
    pub fn caption(&mut self, text: &str) {
        self.wrapped(text, TextStyle::Caption, Ink::Secondary, self.x, self.width);
    }

    fn wrapped(&mut self, text: &str, style: TextStyle, ink: Ink, x: f32, width: f32) {
        let (_, height) = (self.measure)(text, style, width);
        let height = height.max(style.line_dip());
        let rect = Rect::new(x, self.y, width, height);
        self.passive(rect, Kind::Text { text: text.to_string(), style, ink, align: Align::Left, wrap: true });
        self.y += height;
    }

    /// A section header, 24 below whatever came before and 8 above its card.
    pub fn section(&mut self, header: &str) {
        if self.y > self.start_y {
            self.y += SECTION_GAP;
        }
        let rect = Rect::new(self.x, self.y, self.width, TextStyle::BodyStrong.line_dip());
        self.text(rect, header, TextStyle::BodyStrong, Ink::Primary, Align::Left);
        self.y += TextStyle::BodyStrong.line_dip() + 8.0;
    }

    /// Starts a card. Rows go inside it until `card_end`.
    pub fn card_begin(&mut self) {
        let index = self.elements.len();
        self.passive(Rect::new(self.x, self.y, self.width, 0.0), Kind::Card);
        self.card = Some(CardState { index, rows: 0 });
    }

    pub fn card_end(&mut self) {
        if let Some(card) = self.card.take() {
            let top = self.elements[card.index].rect.y;
            self.elements[card.index].rect.h = self.y - top;
        }
    }

    /// A row of the current card, `height` tall, with a hairline above it
    /// when it is not the first.
    pub fn row(&mut self, height: f32) -> Rect {
        if let Some(card) = self.card.as_mut() {
            if card.rows > 0 {
                let divider = Rect::new(self.x, self.y, self.width, 1.0);
                self.elements.push(Element {
                    id: Id::None,
                    rect: divider,
                    kind: Kind::Divider,
                    interactive: false,
                    focusable: false,
                });
            }
            card.rows += 1;
        }
        let rect = Rect::new(self.x, self.y, self.width, height);
        self.y += height;
        rect
    }

    /// The height of text wrapped to a width, never under one line.
    fn text_height(&self, text: &str, style: TextStyle, width: f32) -> f32 {
        (self.measure)(text, style, width).1.max(style.line_dip())
    }

    /// Text that wraps when its rectangle has room for more than one line
    /// and is cut with an ellipsis when it has not, so a long label loses
    /// its tail visibly rather than running under the control beside it.
    fn words(&mut self, rect: Rect, text: &str, style: TextStyle, ink: Ink) {
        let wrap = rect.h > style.line_dip();
        self.passive(rect, Kind::Text { text: text.to_string(), style, ink, align: Align::Left, wrap });
    }

    /// A row with a label at the left, an optional caption, and a slot for a
    /// control `control_w` by `control_h`. Returns the row and the slot.
    ///
    /// The slot is at the right of the row when the words fit beside it,
    /// which is where the Settings app puts a control, and across the whole
    /// row beneath the words when they do not: the inspector is 376 wide, and
    /// a 240 dropdown leaves 92 for a label, which is not a column anybody can
    /// read. Between the two, when the label fits but its caption would wrap
    /// into a ribbon, the caption alone runs under the control. Every case
    /// measures the words it draws, so a row is never shorter than its text.
    fn labeled_row(&mut self, label: &str, caption: Option<&str>, control_w: f32, control_h: f32) -> (Rect, Rect) {
        let inner_w = self.width - 2.0 * ROW_PAD;
        let beside_w = if control_w > 0.0 { inner_w - control_w - COLUMN_GAP } else { inner_w };
        let shape = if beside_w >= CAPTION_BESIDE_MIN_W || (caption.is_none() && beside_w >= LABEL_MIN_W) {
            Shape::Beside
        } else if beside_w >= LABEL_MIN_W {
            Shape::Below
        } else {
            Shape::Stacked
        };
        let label_w = if shape == Shape::Stacked { inner_w } else { beside_w };
        let caption_w = if shape == Shape::Beside { beside_w } else { inner_w };
        let label_h = self.text_height(label, TextStyle::Body, label_w);
        let caption_h = caption.map(|c| self.text_height(c, TextStyle::Caption, caption_w));
        let words_h = label_h + caption_h.map_or(0.0, |h| CAPTION_GAP + h);
        let top = self.y;
        let right = self.x + self.width - ROW_PAD;
        let (height, label_y, caption_y, slot) = match shape {
            Shape::Beside => {
                // A control taller than a line - the dropdown pair - keeps the
                // clearance a 32 control has in a bare row, rather than
                // touching the row's edges.
                let clearance = ROW_H - CONTROL_H;
                let height = (2.0 * ROW_PAD + words_h).max(ROW_H).max(control_h + clearance);
                let slot = Rect::new(right - control_w, top + (height - control_h) / 2.0, control_w, control_h);
                (height, top + ROW_PAD, top + ROW_PAD + label_h + CAPTION_GAP, slot)
            }
            Shape::Below => {
                // The label is centered on the control's band so the two read
                // as one line, whatever the caption does underneath.
                let band_h = label_h.max(control_h);
                let height = 2.0 * ROW_PAD + band_h + caption_h.map_or(0.0, |h| BAND_GAP + h);
                let slot = Rect::new(right - control_w, top + ROW_PAD + (band_h - control_h) / 2.0, control_w, control_h);
                (height, top + ROW_PAD + (band_h - label_h) / 2.0, top + ROW_PAD + band_h + BAND_GAP, slot)
            }
            Shape::Stacked => {
                let height = 2.0 * ROW_PAD + words_h + COLUMN_GAP + control_h;
                let slot = Rect::new(self.x + ROW_PAD, top + ROW_PAD + words_h + COLUMN_GAP, inner_w, control_h);
                (height, top + ROW_PAD, top + ROW_PAD + label_h + CAPTION_GAP, slot)
            }
        };
        let row = self.row(height);
        let x = row.x + ROW_PAD;
        self.words(Rect::new(x, label_y, label_w, label_h), label, TextStyle::Body, Ink::Primary);
        if let (Some(caption), Some(caption_h)) = (caption, caption_h) {
            self.words(Rect::new(x, caption_y, caption_w, caption_h), caption, TextStyle::Caption, Ink::Secondary);
        }
        (row, slot)
    }

    /// A toggle switch at the right, with its state word to its left so the
    /// state is never carried by color alone.
    pub fn row_toggle(&mut self, id: Id, label: &str, caption: Option<&str>, on: bool, enabled: bool) {
        let word_w = 28.0;
        // Where the row's own background belongs in the list: behind the text.
        //
        // It used to be pushed after, and since drawing is simply the list in
        // order, hovering the row painted its wash straight over the label,
        // the caption and the state word - they vanished under the pointer and
        // came back when it left.
        let behind = self.elements.len();
        let (row, slot) = self.labeled_row(label, caption, TOGGLE_W + COLUMN_GAP + word_w, TOGGLE_H);
        let toggle = Rect::new(slot.right() - TOGGLE_W, slot.y, TOGGLE_W, TOGGLE_H);
        let word = Rect::new(toggle.x - COLUMN_GAP - word_w, toggle.y - 2.0, word_w, 24.0);
        let ink = if enabled { Ink::Secondary } else { Ink::Disabled };
        self.text(word, if on { "On" } else { "Off" }, TextStyle::Caption, ink, Align::Right);
        // The label and the word are the hit target too: the whole row reads
        // as the switch, which is how the Settings app behaves.
        let hit = Rect::new(row.x, row.y, row.w, row.h);
        self.elements.insert(
            behind,
            Element {
                id,
                rect: hit,
                kind: Kind::Row { selected: false, lifted: false },
                interactive: enabled,
                focusable: false,
            },
        );
        self.elements.push(Element {
            id,
            rect: toggle,
            kind: Kind::Toggle { on, enabled },
            interactive: enabled,
            focusable: enabled,
        });
    }

    pub fn row_dropdown(
        &mut self,
        id: Id,
        label: &str,
        caption: Option<&str>,
        value: &str,
        width: f32,
        enabled: bool,
    ) {
        let (_, rect) = self.labeled_row(label, caption, width, CONTROL_H);
        self.elements.push(Element {
            id,
            rect,
            kind: Kind::Dropdown { text: value.to_string(), enabled },
            interactive: enabled,
            focusable: enabled,
        });
    }

    /// Two dropdowns side by side under one label, each with a word over it
    /// saying which is which.
    ///
    /// For a pair that only means anything together - the weight of the
    /// strip's headings beside the weight of its values. Two full rows would
    /// have put the relationship a divider apart.
    pub fn row_dropdown_pair(&mut self, label: &str, caption: Option<&str>, pair: [DropdownSpec; 2]) {
        let name_h = TextStyle::Caption.line_dip();
        let block_h = name_h + CAPTION_GAP + CONTROL_H;
        let (_, slot) = self.labeled_row(label, caption, 2.0 * DROPDOWN_W + 8.0, block_h);
        let each_w = (slot.w - 8.0) / 2.0;
        for (index, spec) in pair.into_iter().enumerate() {
            let x = slot.x + index as f32 * (each_w + 8.0);
            self.text(Rect::new(x, slot.y, each_w, name_h), spec.name, TextStyle::Caption, Ink::Secondary, Align::Left);
            let rect = Rect::new(x, slot.y + name_h + CAPTION_GAP, each_w, CONTROL_H);
            self.control(spec.id, rect, Kind::Dropdown { text: spec.value.to_string(), enabled: true });
        }
    }

    /// A slider with its value always visible beside it. No tooltip: a label
    /// needs no popup and no timer, and is readable at rest.
    #[allow(clippy::too_many_arguments)]
    pub fn row_slider(
        &mut self,
        id: Id,
        label: &str,
        caption: Option<&str>,
        value: f32,
        min: f32,
        max: f32,
        step: f32,
        value_text: &str,
    ) {
        let value_w = 56.0;
        let slider_w = 200.0;
        let (_, slot) = self.labeled_row(label, caption, slider_w + 8.0 + value_w, 20.0);
        let value_rect = Rect::new(slot.right() - value_w, slot.y, value_w, slot.h);
        self.text(value_rect, value_text, TextStyle::Caption, Ink::Secondary, Align::Right);
        // The rail takes whatever the slot gives it: its 200 beside a label,
        // the whole row when it has been put under one.
        let rect = Rect::new(slot.x, slot.y, slot.w - 8.0 - value_w, slot.h);
        self.control(id, rect, Kind::Slider { value, min, max, step });
    }

    /// A text field. Full width of the card when it has no label, which is
    /// what a search field wants; otherwise `FIELD_W` at the right.
    #[allow(clippy::too_many_arguments)]
    pub fn row_field(
        &mut self,
        id: Id,
        label: Option<&str>,
        caption: Option<&str>,
        text: &str,
        placeholder: &str,
        search: bool,
        active: bool,
    ) {
        let rect = match label {
            Some(label) => self.labeled_row(label, caption, FIELD_W, CONTROL_H).1,
            None => {
                let inner_w = self.width - 2.0 * ROW_PAD;
                let caption_h = caption.map(|c| self.text_height(c, TextStyle::Caption, inner_w));
                // The row is sized from the field, not from a line of text: a
                // plain 48 row once left the field four pixels off the bottom
                // of its card, looking like it had fallen out.
                let height = 2.0 * ROW_PAD + CONTROL_H + caption_h.map_or(0.0, |h| BAND_GAP + h);
                let row = self.row(height);
                if let (Some(caption), Some(caption_h)) = (caption, caption_h) {
                    let rect = Rect::new(row.x + ROW_PAD, row.y + ROW_PAD + CONTROL_H + BAND_GAP, inner_w, caption_h);
                    self.words(rect, caption, TextStyle::Caption, Ink::Secondary);
                }
                Rect::new(row.x + ROW_PAD, row.y + ROW_PAD, inner_w, CONTROL_H)
            }
        };
        self.control(
            id,
            rect,
            Kind::Field { text: text.to_string(), placeholder: placeholder.to_string(), search, active },
        );
    }

    /// A row with nothing to operate: a label and a caption.
    pub fn row_info(&mut self, label: &str, caption: Option<&str>) {
        self.labeled_row(label, caption, 0.0, 0.0);
    }

    /// A status line: a dot, a sentence, and optionally a caption.
    ///
    /// The sentence wraps. It is where an error message lands, and an error
    /// message is the one string nobody gets to choose the length of.
    pub fn row_status(&mut self, dot: Ink, line: &str, caption: Option<&str>) {
        let dot_w = 8.0;
        let text_x = self.x + ROW_PAD + dot_w + 8.0;
        let text_w = self.x + self.width - ROW_PAD - text_x;
        let line_h = self.text_height(line, TextStyle::Body, text_w);
        let caption_h = caption.map(|c| self.text_height(c, TextStyle::Caption, text_w));
        let height = 2.0 * ROW_PAD + line_h + caption_h.map_or(0.0, |h| CAPTION_GAP + h);
        let row = self.row(height);
        let line_y = row.y + ROW_PAD;
        // On the middle of the first line, however many the sentence takes.
        let dot_y = line_y + (TextStyle::Body.line_dip() - dot_w) / 2.0;
        self.passive(Rect::new(row.x + ROW_PAD, dot_y, dot_w, dot_w), Kind::Dot { ink: dot });
        self.words(Rect::new(text_x, line_y, text_w, line_h), line, TextStyle::Body, Ink::Primary);
        if let (Some(caption), Some(caption_h)) = (caption, caption_h) {
            let rect = Rect::new(text_x, line_y + line_h + CAPTION_GAP, text_w, caption_h);
            self.words(rect, caption, TextStyle::Caption, Ink::Secondary);
        }
    }

    /// A paragraph inside a card.
    pub fn row_paragraph(&mut self, text: &str) {
        let width = self.width - 2.0 * ROW_PAD;
        let height = self.text_height(text, TextStyle::Body, width);
        let row = self.row(height + 2.0 * ROW_PAD);
        self.words(Rect::new(row.x + ROW_PAD, row.y + ROW_PAD, width, height), text, TextStyle::Body, Ink::Secondary);
    }

    /// Buttons side by side, with a caption under them when there is one.
    pub fn row_buttons(&mut self, caption: Option<&str>, buttons: &[ButtonSpec]) {
        let width = self.width - 2.0 * ROW_PAD;
        let caption_h = caption.map(|c| self.text_height(c, TextStyle::Caption, width));
        let height = 2.0 * ROW_PAD + CONTROL_H + caption_h.map_or(0.0, |h| BAND_GAP + h);
        let row = self.row(height);
        let mut x = row.x + ROW_PAD;
        let y = row.y + ROW_PAD;
        for button in buttons {
            let w = self.button_width(button.text, button.external);
            let rect = Rect::new(x, y, w, CONTROL_H);
            self.elements.push(Element {
                id: button.id,
                rect,
                kind: Kind::Button {
                    text: button.text.to_string(),
                    accent: button.accent,
                    external: button.external,
                    enabled: button.enabled,
                },
                interactive: button.enabled,
                focusable: button.enabled,
            });
            x += w + 8.0;
        }
        if let (Some(caption), Some(caption_h)) = (caption, caption_h) {
            let rect = Rect::new(row.x + ROW_PAD, y + CONTROL_H + BAND_GAP, width, caption_h);
            self.words(rect, caption, TextStyle::Caption, Ink::Secondary);
        }
    }

    /// A button's width: its text plus padding, never under the minimum.
    pub fn button_width(&self, text: &str, external: bool) -> f32 {
        let (w, _) = (self.measure)(text, TextStyle::Body, 0.0);
        let glyph = if external { 8.0 + 12.0 } else { 0.0 };
        (w + 24.0 + glyph).max(BUTTON_MIN_W)
    }

    /// A label and caption with one button at the right.
    pub fn row_button(&mut self, label: &str, caption: Option<&str>, button: ButtonSpec) {
        let w = self.button_width(button.text, button.external);
        let (_, slot) = self.labeled_row(label, caption, w, CONTROL_H);
        // A button keeps its own width even when it has been put under the
        // words: a button stretched across a card reads as a banner.
        let rect = Rect::new(slot.x, slot.y, w, CONTROL_H);
        self.elements.push(Element {
            id: button.id,
            rect,
            kind: Kind::Button {
                text: button.text.to_string(),
                accent: button.accent,
                external: button.external,
                enabled: button.enabled,
            },
            interactive: button.enabled,
            focusable: button.enabled,
        });
    }

    /// A label and caption with a hyperlink at the right.
    pub fn row_with_link(&mut self, id: Id, label: &str, caption: Option<&str>, link: &str) {
        let (w, _) = (self.measure)(link, TextStyle::Body, 0.0);
        let (_, slot) = self.labeled_row(label, caption, w, TextStyle::Body.line_dip());
        let rect = Rect::new(slot.x, slot.y, w.min(slot.w), slot.h);
        self.control(id, rect, Kind::Link { text: link.to_string(), critical: false });
    }

    /// A link that sits outside any card.
    pub fn link(&mut self, id: Id, text: &str) {
        let (w, _) = (self.measure)(text, TextStyle::Body, 0.0);
        let rect = Rect::new(self.x, self.y, w, 20.0);
        self.control(id, rect, Kind::Link { text: text.to_string(), critical: false });
        self.y += 20.0;
    }

    /// One reading inside a stack: tile, name, module, the three actions,
    /// and - when the caller has one - a field for what the strip calls it.
    ///
    /// With the field the row is two lines: the reading's name with its
    /// module beside it, then the field across the row's width. Beside the
    /// name it would have had eighty DIPs in the inspector, which is a field
    /// two words long.
    #[allow(clippy::too_many_arguments)]
    pub fn row_metric(
        &mut self,
        index: usize,
        color: Color,
        mark: Mark,
        name: &str,
        module: &str,
        label: Option<MetricLabel>,
        first: bool,
        last: bool,
    ) {
        let height = if label.is_some() { 8.0 + 20.0 + CAPTION_GAP + CONTROL_H + 8.0 } else { LIST_ROW_H };
        let row = self.row(height);
        // On the name's line, as the mark in the composer list is.
        let tile = Rect::new(row.x + ROW_PAD, row.y + (LIST_ROW_H - 24.0) / 2.0, 24.0, 24.0);
        self.passive(tile, Kind::Mark { color, mark, dim: false });
        let buttons_w = 3.0 * 32.0 + 2.0 * 4.0;
        let text_x = tile.right() + 12.0;
        let text_w = row.right() - ROW_PAD - buttons_w - 8.0 - text_x;
        match label {
            None => {
                self.text(Rect::new(text_x, row.y + 8.0, text_w, 20.0), name, TextStyle::Body, Ink::Primary, Align::Left);
                self.text(
                    Rect::new(text_x, row.y + 28.0, text_w, 16.0),
                    module,
                    TextStyle::Caption,
                    Ink::Secondary,
                    Align::Left,
                );
            }
            Some(label) => {
                let (name_w, _) = (self.measure)(name, TextStyle::Body, 0.0);
                let name_w = name_w.min(text_w);
                self.text(Rect::new(text_x, row.y + 8.0, name_w, 20.0), name, TextStyle::Body, Ink::Primary, Align::Left);
                let module_x = text_x + name_w + 8.0;
                self.text(
                    Rect::new(module_x, row.y + 8.0, (text_x + text_w - module_x).max(0.0), 20.0),
                    module,
                    TextStyle::Caption,
                    Ink::Secondary,
                    Align::Left,
                );
                let field = Rect::new(text_x, row.y + 8.0 + 20.0 + CAPTION_GAP, text_w, CONTROL_H);
                self.control(
                    Id::MetricLabel(index),
                    field,
                    Kind::Field {
                        text: label.text.to_string(),
                        placeholder: label.placeholder.to_string(),
                        search: false,
                        active: label.active,
                    },
                );
            }
        }
        let mut x = row.right() - ROW_PAD - buttons_w;
        for (id, glyph, enabled) in [
            (Id::MetricUp(index), glyph::UP, !first),
            (Id::MetricDown(index), glyph::DOWN, !last),
            (Id::MetricRemove(index), glyph::CANCEL, true),
        ] {
            let rect = Rect::new(x, row.y + (row.h - 32.0) / 2.0, 32.0, 32.0);
            self.elements.push(Element {
                id,
                rect,
                kind: Kind::IconButton { glyph, enabled },
                interactive: enabled,
                focusable: enabled,
            });
            x += 36.0;
        }
    }

    /// A saved weather location.
    pub fn row_location(&mut self, index: usize, name: &str, region: &str, primary: bool) {
        let row = self.row(LIST_ROW_H);
        let remove = Rect::new(row.right() - ROW_PAD - 32.0, row.y + (row.h - 32.0) / 2.0, 32.0, 32.0);
        self.control(
            Id::LocationRemove(index),
            remove,
            Kind::IconButton { glyph: glyph::CANCEL, enabled: true },
        );
        let tag = if primary { "Primary" } else { "Set primary" };
        let (tag_w, _) = (self.measure)(tag, TextStyle::Caption, 0.0);
        let tag_rect = Rect::new(remove.x - 12.0 - tag_w, row.y + (row.h - 20.0) / 2.0, tag_w, 20.0);
        if primary {
            self.text(tag_rect, tag, TextStyle::Caption, Ink::Secondary, Align::Right);
        } else {
            self.control(
                Id::LocationPrimary(index),
                tag_rect,
                Kind::Link { text: tag.to_string(), critical: false },
            );
        }
        let text_x = row.x + ROW_PAD;
        let text_w = tag_rect.x - 12.0 - text_x;
        self.text(Rect::new(text_x, row.y + 8.0, text_w, 20.0), name, TextStyle::Body, Ink::Primary, Align::Left);
        self.text(
            Rect::new(text_x, row.y + 28.0, text_w, 16.0),
            region,
            TextStyle::Caption,
            Ink::Secondary,
            Align::Left,
        );
    }

    /// A search result under the search field.
    pub fn row_result(&mut self, index: usize, name: &str, region: &str) {
        let row = self.row(LIST_ROW_H);
        self.elements.push(Element {
            id: Id::SearchResult(index),
            rect: row,
            kind: Kind::Row { selected: false, lifted: false },
            interactive: true,
            focusable: true,
        });
        let text_x = row.x + ROW_PAD;
        let text_w = row.w - 2.0 * ROW_PAD;
        self.text(Rect::new(text_x, row.y + 8.0, text_w, 20.0), name, TextStyle::Body, Ink::Primary, Align::Left);
        self.text(
            Rect::new(text_x, row.y + 28.0, text_w, 16.0),
            region,
            TextStyle::Caption,
            Ink::Secondary,
            Align::Left,
        );
    }

    /// One row of the composer list.
    #[allow(clippy::too_many_arguments)]
    pub fn list_row(
        &mut self,
        item: StripItem,
        color: Color,
        mark: Mark,
        name: &str,
        caption: &str,
        on: bool,
        selected: bool,
        lifted: bool,
    ) {
        let toggle_x = self.x + self.width - 8.0 - TOGGLE_W;
        let text_x = self.x + 60.0;
        let text_w = toggle_x - COLUMN_GAP - text_x;
        // The caption wraps rather than losing its tail: "Hottest processor
        // temperature" cut to "Hottest processor tem..." in a 136 DIP column
        // told nobody what the module showed. A second line costs the row
        // sixteen, which a list can afford.
        let caption_h = self.text_height(caption, TextStyle::Caption, text_w);
        let height = (8.0 + 20.0 + caption_h + 8.0).max(LIST_ROW_H);
        let row = Rect::new(self.x, self.y, self.width, height);
        self.y += height;
        self.elements.push(Element {
            id: Id::Row(item),
            rect: row,
            kind: Kind::Row { selected, lifted },
            interactive: true,
            focusable: true,
        });
        let grip = Rect::new(row.x + 8.0, row.y, 16.0, row.h);
        self.passive(grip, Kind::Glyph { glyph: glyph::GRIPPER, size: 16.0, ink: Ink::Tertiary });
        // The tile and the switch stay on the name's line rather than the
        // row's middle, so a row that grew a second caption line still lines
        // its marks up with the rows around it.
        let tile = Rect::new(row.x + 32.0, row.y + (LIST_ROW_H - 24.0) / 2.0, 24.0, 24.0);
        self.passive(tile, Kind::Mark { color, mark, dim: !on });
        let toggle = Rect::new(toggle_x, row.y + (LIST_ROW_H - TOGGLE_H) / 2.0, TOGGLE_W, TOGGLE_H);
        self.text(Rect::new(text_x, row.y + 8.0, text_w, 20.0), name, TextStyle::Body, Ink::Primary, Align::Left);
        self.words(Rect::new(text_x, row.y + 28.0, text_w, caption_h), caption, TextStyle::Caption, Ink::Secondary);
        self.elements.push(Element {
            id: Id::RowToggle(item),
            rect: toggle,
            kind: Kind::Toggle { on, enabled: true },
            interactive: true,
            focusable: false,
        });
    }

    /// A standard button on its own line, outside a card.
    pub fn button(&mut self, id: Id, text: &str, accent: bool) {
        let w = self.button_width(text, false);
        let rect = Rect::new(self.x, self.y, w, CONTROL_H);
        self.control(id, rect, Kind::Button { text: text.to_string(), accent, external: false, enabled: true });
        self.y += CONTROL_H;
    }

    /// The inspector's header: a 32 mark, a name, and a line about it.
    ///
    /// The name's line box starts where the builder is, so a column header
    /// laid out beside it at the same `y` sits on the same line.
    pub fn header(&mut self, color: Color, mark: Mark, name: &str, blurb: &str) {
        let tile = Rect::new(self.x, self.y, 36.0, 36.0);
        self.passive(tile, Kind::Mark { color, mark, dim: false });
        let text_x = tile.right() + COLUMN_GAP;
        // Measured from the tile as drawn: the width was once taken from a
        // 32 tile after the tile had grown to 36, which ran the blurb four
        // DIPs past the column and wrapped it a word early.
        let text_w = self.width - tile.w - COLUMN_GAP;
        let name_h = TextStyle::BodyStrong.line_dip();
        self.text(Rect::new(text_x, self.y, text_w, name_h), name, TextStyle::BodyStrong, Ink::Primary, Align::Left);
        let blurb_h = self.text_height(blurb, TextStyle::Caption, text_w);
        self.words(Rect::new(text_x, self.y + name_h + CAPTION_GAP, text_w, blurb_h), blurb, TextStyle::Caption, Ink::Secondary);
        self.y += (name_h + CAPTION_GAP + blurb_h).max(tile.h);
    }

    /// The About pane's masthead: the application icon at 64, the name in
    /// the largest style of the ramp with a line under it, and a sentence
    /// about the program beneath both.
    ///
    /// The one place the window is allowed to be presentation. The icon is
    /// the app's identity and every other pane borrows Windows'; here it is
    /// drawn from the executable's own resources at a size that reads as a
    /// mark rather than as a tray glyph.
    pub fn masthead(&mut self, name: &str, line: &str, blurb: &str) {
        let icon_size = 64.0;
        let icon = Rect::new(self.x, self.y, icon_size, icon_size);
        self.passive(icon, Kind::AppIcon);
        let text_x = icon.right() + 20.0;
        let text_w = self.width - icon_size - 20.0;
        let name_h = TextStyle::Title.line_dip();
        let line_h = TextStyle::Body.line_dip();
        // The two lines are centered on the icon, not hung from its top.
        let text_y = self.y + (icon_size - name_h - line_h) / 2.0;
        self.text(Rect::new(text_x, text_y, text_w, name_h), name, TextStyle::Title, Ink::Primary, Align::Left);
        self.text(Rect::new(text_x, text_y + name_h, text_w, line_h), line, TextStyle::Body, Ink::Secondary, Align::Left);
        self.y += icon_size + 12.0;
        let blurb_h = self.text_height(blurb, TextStyle::Caption, self.width);
        self.words(Rect::new(self.x, self.y, self.width, blurb_h), blurb, TextStyle::Caption, Ink::Secondary);
        self.y += blurb_h;
    }

    /// A column header, level with the line `header` puts a name on.
    pub fn column_header(&mut self, text: &str) {
        let rect = Rect::new(self.x, self.y, self.width, TextStyle::BodyStrong.line_dip());
        self.text(rect, text, TextStyle::BodyStrong, Ink::Primary, Align::Left);
        self.y += TextStyle::BodyStrong.line_dip() + 8.0;
    }
}

/// What a button in a row is.
pub struct ButtonSpec<'t> {
    pub id: Id,
    pub text: &'t str,
    pub accent: bool,
    pub external: bool,
    pub enabled: bool,
}

/// One of a pair of dropdowns: which one it is, and what it shows.
pub struct DropdownSpec<'t> {
    pub id: Id,
    pub name: &'t str,
    pub value: &'t str,
}

/// The field on a stack reading's row: the caption the user typed, if any,
/// and the one the strip falls back to, shown as the placeholder so that an
/// empty field visibly means "the usual".
pub struct MetricLabel<'t> {
    pub text: &'t str,
    pub placeholder: &'t str,
    pub active: bool,
}

/// The topmost interactive element under a point.
///
/// Last wins: elements are pushed back to front, so the last one containing
/// the point is the one drawn on top, and it is the one the pointer is on.
pub fn hit(elements: &[Element], x: f32, y: f32) -> Option<Id> {
    elements
        .iter()
        .rev()
        .find(|e| e.interactive && e.rect.contains(x, y))
        .map(|e| e.id)
}

/// The element with an id.
pub fn find(elements: &[Element], id: Id) -> Option<&Element> {
    elements.iter().find(|e| e.id == id && e.interactive)
}

/// Where a slider's value falls between its ends, 0 through 1.
pub fn slider_fraction(value: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

/// The value at a pointer position along a slider, snapped to its step.
pub fn slider_value_at(rect: Rect, x: f32, min: f32, max: f32, step: f32) -> f32 {
    let usable = (rect.w - 20.0).max(1.0);
    let fraction = ((x - rect.x - 10.0) / usable).clamp(0.0, 1.0);
    let raw = min + fraction * (max - min);
    let stepped = if step > 0.0 { (raw / step).round() * step } else { raw };
    stepped.clamp(min, max)
}

/// Where a dropdown's list opens: below the control with a 4 gap, or above
/// it when there is more room there, never taller than the space it has.
///
/// As wide as its widest item when that is wider than the control, growing
/// to the left: the control sits at the right edge of a card, so that is
/// where the room is. The control shows a value cut to fit; the list is
/// where every choice gets read in full.
pub fn popup_rect(anchor: Rect, items: usize, viewport_h: f32, items_w: f32) -> Rect {
    let w = items_w.max(anchor.w);
    let x = if w > anchor.w { (anchor.right() - w).max(8.0) } else { anchor.x };
    let wanted = items as f32 * POPUP_ITEM_H + 8.0;
    let below = viewport_h - anchor.bottom() - 4.0;
    let above = anchor.y - 4.0;
    if wanted <= below || below >= above {
        let h = wanted.min(below.max(POPUP_ITEM_H + 8.0));
        Rect::new(x, anchor.bottom() + 4.0, w, h)
    } else {
        let h = wanted.min(above.max(POPUP_ITEM_H + 8.0));
        Rect::new(x, anchor.y - 4.0 - h, w, h)
    }
}

/// The scrollbar thumb for a scrolled pane, or none when it all fits.
pub fn scroll_thumb(viewport_h: f32, content_h: f32, scroll: f32) -> Option<(f32, f32)> {
    if content_h <= viewport_h || viewport_h <= 0.0 {
        return None;
    }
    let h = (viewport_h * viewport_h / content_h).max(32.0).min(viewport_h);
    let travel = viewport_h - h;
    let y = travel * (scroll / (content_h - viewport_h)).clamp(0.0, 1.0);
    Some((y, h))
}

pub fn clamp_scroll(scroll: f32, viewport_h: f32, content_h: f32) -> f32 {
    scroll.clamp(0.0, (content_h - viewport_h).max(0.0))
}

/// The interaction state the painter needs.
#[derive(Copy, Clone, Debug, Default)]
pub struct Interaction {
    pub hover: Option<Id>,
    pub pressed: Option<Id>,
    pub focus: Option<Id>,
    pub focus_visible: bool,
}

/// Draws elements offset by (dx, dy), in order.
///
/// The preview strip is painted by the caller through `preview`, since it is
/// the one element drawn from the model rather than from what the element
/// carries.
pub fn draw(
    canvas: &mut Canvas,
    theme: &Theme,
    elements: &[Element],
    ui: Interaction,
    dx: f32,
    dy: f32,
    visible: Rect,
    preview: &mut dyn FnMut(&mut Canvas, Rect),
) {
    for element in elements {
        let rect = element.rect.offset(dx, dy);
        if !rect.intersects(&visible) {
            continue;
        }
        let hover = ui.hover == Some(element.id) && element.interactive;
        let pressed = ui.pressed == Some(element.id) && element.interactive;
        match &element.kind {
            Kind::Text { text, style, ink, align, wrap } => {
                if *wrap {
                    canvas.text_wrapped(rect, text, *style, ink.color(theme));
                } else {
                    canvas.text(rect, text, *style, ink.color(theme), *align);
                }
            }
            Kind::Card => {
                canvas.fill_round(rect, RADIUS_CONTROL, theme.surface_card);
                canvas.stroke_round(rect, RADIUS_CONTROL, theme.stroke_card);
            }
            Kind::Divider => canvas.hline(rect.x, rect.right(), rect.y, theme.stroke_divider),
            Kind::Toggle { on, enabled } => draw_toggle(canvas, theme, rect, *on, *enabled, hover, pressed),
            Kind::Dropdown { text, enabled } => {
                draw_control_face(canvas, theme, rect, *enabled, hover, pressed);
                let ink = if *enabled { theme.text_primary } else { theme.text_disabled };
                canvas.text(Rect::new(rect.x + 12.0, rect.y, rect.w - 40.0, rect.h), text, TextStyle::Body, ink, Align::Left);
                let chevron = Rect::new(rect.right() - 12.0 - 12.0, rect.y, 12.0, rect.h);
                canvas.glyph(chevron, glyph::CHEVRON_DOWN, 12.0, theme.text_tertiary);
            }
            Kind::Slider { value, min, max, step: _ } => draw_slider(canvas, theme, rect, *value, *min, *max, hover, pressed),
            Kind::Button { text, accent, external, enabled } => {
                draw_button(canvas, theme, rect, text, *accent, *external, *enabled, hover, pressed)
            }
            Kind::Link { text, critical } => {
                let color = if *critical { theme.status_critical } else { theme.accent_text };
                canvas.text(rect, text, TextStyle::Body, color, Align::Left);
                if hover {
                    canvas.hline(rect.x, rect.right(), rect.bottom() - 2.0, color);
                }
            }
            Kind::Row { selected, lifted } => {
                if *lifted {
                    canvas.fill_round(rect, RADIUS_CONTROL, theme.surface_card);
                    canvas.stroke_round(rect, RADIUS_CONTROL, theme.stroke_strong);
                } else {
                    draw_item_wash(canvas, theme, rect, *selected, hover, pressed);
                }
                if *selected {
                    draw_pill(canvas, theme, rect);
                }
            }
            Kind::NavItem { text, selected, mark } => {
                draw_item_wash(canvas, theme, rect, *selected, hover, pressed);
                if *selected {
                    draw_pill(canvas, theme, rect);
                }
                // A module's row carries its mark, in its own color, so the
                // sidebar reads as the modules rather than as a word list.
                let text_x = match mark {
                    Some((color, mark)) => {
                        let size = 18.0;
                        let box_ = Rect::new(rect.x + 12.0, rect.y + (rect.h - size) / 2.0, size, size);
                        draw_mark(canvas, theme, box_, *color, *mark, false);
                        box_.right() + 10.0
                    }
                    None => rect.x + 12.0,
                };
                canvas.text(
                    Rect::new(text_x, rect.y, rect.right() - text_x, rect.h),
                    text,
                    TextStyle::Body,
                    theme.text_primary,
                    Align::Left,
                );
            }
            Kind::NavHeading { text } => canvas.text(
                Rect::new(rect.x + 12.0, rect.y, rect.w - 12.0, rect.h),
                &text.to_uppercase(),
                TextStyle::Caption,
                theme.text_secondary,
                Align::Left,
            ),
            Kind::Mark { color, mark, dim } => draw_mark(canvas, theme, rect, *color, *mark, *dim),
            Kind::Glyph { glyph, size, ink } => canvas.glyph(rect, glyph, *size, ink.color(theme)),
            Kind::Dot { ink } => canvas.fill_circle(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0, rect.w / 2.0, ink.color(theme)),
            Kind::Field { text, placeholder, search, active } => {
                draw_field(canvas, theme, rect, text, placeholder, *search, *active)
            }
            Kind::Preview => preview(canvas, rect),
            Kind::IconButton { glyph, enabled } => {
                if *enabled && (hover || pressed) {
                    let fill = if pressed { theme.subtle_pressed } else { theme.subtle_hover };
                    canvas.fill_round(rect, RADIUS_CONTROL, fill);
                }
                let ink = if *enabled { theme.text_secondary } else { theme.text_disabled };
                canvas.glyph(rect, glyph, 16.0, ink);
            }
            Kind::ListItem { text, selected } => {
                draw_item_wash(canvas, theme, rect, *selected, hover, pressed);
                if *selected {
                    draw_pill(canvas, theme, rect);
                }
                canvas.text(Rect::new(rect.x + 12.0, rect.y, rect.w - 16.0, rect.h), text, TextStyle::Body, theme.text_primary, Align::Left);
            }
            Kind::Popup => {
                canvas.fill_round(rect, RADIUS_CONTROL, theme.surface_card);
                canvas.stroke_round(rect, RADIUS_CONTROL, theme.stroke_control);
            }
            Kind::Progress { fraction } => {
                let rail = Rect::new(rect.x, rect.center_y() - 2.0, rect.w, 4.0);
                canvas.fill_round(rail, 2.0, theme.stroke_control);
                let done = Rect::new(rail.x, rail.y, rail.w * fraction.clamp(0.0, 1.0), rail.h);
                if done.w > 0.0 {
                    canvas.fill_round(done, 2.0, theme.accent_fill);
                }
            }
            Kind::AppIcon => canvas.app_icon(rect),
        }
        if ui.focus_visible && ui.focus == Some(element.id) && element.focusable {
            let radius = match element.kind {
                Kind::Toggle { .. } => TOGGLE_H / 2.0 + 2.0,
                _ => RADIUS_CONTROL + 2.0,
            };
            canvas.stroke_round_width(rect.inset(-3.0), radius, theme.focus_outer, 2.0);
            canvas.stroke_round_width(rect.inset(-1.0), radius - 2.0, theme.focus_inner, 1.0);
        }
    }
}

/// The 3 by 16 accent bar at a selected item's left edge.
fn draw_pill(canvas: &mut Canvas, theme: &Theme, rect: Rect) {
    let pill = Rect::new(rect.x, rect.center_y() - 8.0, 3.0, 16.0);
    canvas.fill_round(pill, 1.5, theme.accent_fill);
}

/// The wash behind a list item, nav item or popup row.
///
/// A selected item is washed a step stronger than a hovered one, and stays
/// that way under the pointer. With the two the same fill, moving the mouse
/// down the composer made every row look selected in turn and the real
/// selection was carried by the pill alone.
fn draw_item_wash(canvas: &mut Canvas, theme: &Theme, rect: Rect, selected: bool, hover: bool, pressed: bool) {
    let fill = if pressed {
        theme.subtle_pressed
    } else if selected {
        theme.subtle_selected
    } else if hover {
        theme.subtle_hover
    } else {
        return;
    };
    canvas.fill_round(rect, RADIUS_CONTROL, fill);
}

/// The fill and stroke every standard control shares.
///
/// The stroke has a "lift": one edge in a slightly stronger tone, the bottom
/// in light and the top in dark, which is the one-pixel bevel WinUI gives
/// controls so they read as sitting on the surface rather than drawn on it.
fn draw_control_face(canvas: &mut Canvas, theme: &Theme, rect: Rect, enabled: bool, hover: bool, pressed: bool) {
    let fill = if !enabled {
        theme.control_disabled
    } else if pressed {
        theme.control_pressed
    } else if hover {
        theme.control_hover
    } else {
        theme.control_rest
    };
    canvas.fill_round(rect, RADIUS_CONTROL, fill);
    canvas.stroke_round(rect, RADIUS_CONTROL, theme.stroke_control);
    if enabled && !pressed {
        let y = if theme.light { rect.bottom() - 1.0 } else { rect.y };
        canvas.hline(rect.x + RADIUS_CONTROL, rect.right() - RADIUS_CONTROL, y, theme.stroke_control_edge);
    }
}

fn draw_toggle(canvas: &mut Canvas, theme: &Theme, rect: Rect, on: bool, enabled: bool, hover: bool, pressed: bool) {
    let radius = rect.h / 2.0;
    let knob = if pressed || hover { 7.0 } else { 6.0 };
    if on {
        let fill = if !enabled {
            theme.control_disabled
        } else if pressed {
            theme.accent_pressed
        } else if hover {
            theme.accent_hover
        } else {
            theme.accent_fill
        };
        canvas.fill_round(rect, radius, fill);
        let ink = if enabled { theme.text_on_accent } else { theme.text_disabled };
        canvas.fill_circle(rect.right() - 4.0 - 6.0, rect.center_y(), knob, ink);
    } else {
        if enabled && (hover || pressed) {
            let fill = if pressed { theme.subtle_pressed } else { theme.subtle_hover };
            canvas.fill_round(rect, radius, fill);
        }
        let ring = if enabled { theme.stroke_strong } else { theme.stroke_control };
        canvas.stroke_round(rect, radius, ring);
        let ink = if enabled { theme.text_secondary } else { theme.text_disabled };
        canvas.fill_circle(rect.x + 4.0 + 6.0, rect.center_y(), knob, ink);
    }
}

fn draw_slider(canvas: &mut Canvas, theme: &Theme, rect: Rect, value: f32, min: f32, max: f32, hover: bool, pressed: bool) {
    let rail = Rect::new(rect.x + 10.0, rect.center_y() - 2.0, rect.w - 20.0, 4.0);
    canvas.fill_round(rail, 2.0, theme.stroke_strong);
    let fraction = slider_fraction(value, min, max);
    let filled = Rect::new(rail.x, rail.y, rail.w * fraction, rail.h);
    if filled.w > 0.0 {
        canvas.fill_round(filled, 2.0, theme.accent_fill);
    }
    let cx = rail.x + rail.w * fraction;
    let cy = rect.center_y();
    canvas.fill_circle(cx, cy, 10.0, theme.control_rest);
    canvas.stroke_circle(cx, cy, 10.0, theme.stroke_control);
    let dot = if pressed {
        5.0
    } else if hover {
        7.0
    } else {
        6.0
    };
    canvas.fill_circle(cx, cy, dot, theme.accent_fill);
}

#[allow(clippy::too_many_arguments)]
fn draw_button(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    text: &str,
    accent: bool,
    external: bool,
    enabled: bool,
    hover: bool,
    pressed: bool,
) {
    let ink = if accent && enabled {
        let fill = if pressed {
            theme.accent_pressed
        } else if hover {
            theme.accent_hover
        } else {
            theme.accent_fill
        };
        canvas.fill_round(rect, RADIUS_CONTROL, fill);
        theme.text_on_accent
    } else {
        draw_control_face(canvas, theme, rect, enabled, hover, pressed);
        if !enabled {
            theme.text_disabled
        } else if pressed {
            theme.text_secondary
        } else {
            theme.text_primary
        }
    };
    if external {
        let (w, _) = canvas.measure(text, TextStyle::Body);
        let total = w + 8.0 + 12.0;
        let x = rect.x + (rect.w - total) / 2.0;
        canvas.text(Rect::new(x, rect.y, w + 2.0, rect.h), text, TextStyle::Body, ink, Align::Left);
        canvas.glyph(Rect::new(x + w + 8.0, rect.y, 12.0, rect.h), glyph::OPEN_IN_NEW, 12.0, ink);
    } else {
        canvas.text(rect, text, TextStyle::Body, ink, Align::Center);
    }
}

/// A module's mark: its glyph in its own hue, and nothing behind it.
///
/// This is the macOS list's look - a colored symbol beside a name - rather
/// than the Settings app's, which has no per-item color at all. The tile
/// that was here before put the hue in a square and the glyph in white, and
/// at 20 DIPs the square was what the eye caught and the glyph was a shape
/// inside it; the symbol on its own is the thing itself.
fn draw_mark(canvas: &mut Canvas, theme: &Theme, rect: Rect, color: Color, mark: Mark, dim: bool) {
    // An off row keeps its hue, faded into the pane, so the mark still says
    // which module this is; a gray mark says nothing.
    let ink = theme.mark_ink(color);
    let ink = if dim { ink.over(theme.surface_layer, 0.45) } else { ink };
    // The whole box. The 0.85 here was inherited from the icon font, whose
    // glyphs carry their own margin inside the em - a drawn mark has no such
    // margin, so keeping the inset just made it small.
    let size = rect.w;
    let base = Rect::new(rect.x + (rect.w - size) / 2.0, rect.y + (rect.h - size) / 2.0, size, size);
    match mark {
        Mark::Module(id) => icon::module(canvas, base, id, ink),
        Mark::Stack => icon::stack(canvas, base, ink),
        Mark::Glyph(glyph) => canvas.glyph(base, &glyph.to_string(), size, ink),
    }
}

fn draw_field(canvas: &mut Canvas, theme: &Theme, rect: Rect, text: &str, placeholder: &str, search: bool, active: bool) {
    canvas.fill_round(rect, RADIUS_CONTROL, theme.control_rest);
    canvas.stroke_round(rect, RADIUS_CONTROL, theme.stroke_control);
    // The bottom edge is the field's tell: strong at rest, accent and two
    // pixels thick while it has the caret.
    if active {
        canvas.fill_rect(Rect::new(rect.x + 2.0, rect.bottom() - 2.0, rect.w - 4.0, 2.0), theme.accent_fill);
    } else {
        canvas.hline(rect.x + RADIUS_CONTROL, rect.right() - RADIUS_CONTROL, rect.bottom() - 1.0, theme.stroke_strong);
    }
    let mut x = rect.x + 12.0;
    if search {
        canvas.glyph(Rect::new(x, rect.y, 12.0, rect.h), glyph::SEARCH, 12.0, theme.text_tertiary);
        x += 12.0 + 8.0;
    }
    // While the EDIT child is over the field it draws the text itself.
    if !active {
        let (shown, ink) = if text.is_empty() {
            (placeholder, theme.text_secondary)
        } else {
            (text, theme.text_primary)
        };
        let inner = Rect::new(x, rect.y, rect.right() - 12.0 - x, rect.h);
        // A path loses its middle, not its end: the end is the part that
        // says which folder it is.
        if shown.contains('\\') {
            canvas.text_path(inner, shown, TextStyle::Body, ink);
        } else {
            canvas.text(inner, shown, TextStyle::Body, ink, Align::Left);
        }
    }
}

/// Where the EDIT child sits inside a field: past the search glyph, one line
/// tall, centered.
pub fn field_edit_rect(field: Rect, search: bool) -> Rect {
    let x = field.x + 12.0 + if search { 20.0 } else { 0.0 };
    Rect::new(x, field.y + (field.h - 20.0) / 2.0, field.right() - 12.0 - x, 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use barometer_core::ModuleId;

    fn measure(text: &str, style: TextStyle, width: f32) -> (f32, f32) {
        // Seven DIPs a character, wrapped by width when one is given.
        let w = text.chars().count() as f32 * 7.0;
        if width > 0.0 && w > width {
            let lines = (w / width).ceil();
            (width, lines * style.line_dip())
        } else {
            (w, style.line_dip())
        }
    }

    #[test]
    fn the_topmost_interactive_element_wins_a_hit() {
        let mut b = Builder::new(0.0, 0.0, 400.0, &measure);
        b.card_begin();
        b.row_toggle(Id::Show, "Show", None, true, true);
        b.card_end();
        // The toggle sits over the row, and both answer to Id::Show; the
        // card and the label behind them never do.
        assert_eq!(hit(&b.elements, 380.0, 24.0), Some(Id::Show));
        assert_eq!(hit(&b.elements, 20.0, 24.0), Some(Id::Show));
        assert_eq!(hit(&b.elements, 20.0, 200.0), None);
    }

    #[test]
    fn a_disabled_control_is_not_hit() {
        let mut b = Builder::new(0.0, 0.0, 400.0, &measure);
        b.card_begin();
        b.row_dropdown(Id::GpuAdapter, "Adapter", None, "Automatic", DROPDOWN_W, false);
        b.card_end();
        assert_eq!(hit(&b.elements, 300.0, 24.0), None);
    }

    #[test]
    fn rows_stack_with_a_hairline_between_them_and_the_card_grows_to_fit() {
        let mut b = Builder::new(0.0, 0.0, 400.0, &measure);
        b.card_begin();
        b.row_toggle(Id::Show, "A", None, true, true);
        b.row_toggle(Id::CheckUpdates, "B", Some("A caption."), false, true);
        b.card_end();
        let card = &b.elements[0];
        assert!(matches!(card.kind, Kind::Card));
        // The card is exactly its rows, and the captioned one is taller than
        // the bare one - asserted as a relationship rather than as a number,
        // because the number moves whenever the caption has to clear a taller
        // control and a test that pins it just has to be edited each time.
        assert!(card.rect.h > 2.0 * ROW_H);
        assert_eq!(b.y(), card.rect.h);
        let dividers = b.elements.iter().filter(|e| matches!(e.kind, Kind::Divider)).count();
        assert_eq!(dividers, 1);
    }

    /// The parts of a captioned dropdown row: (label, caption, control).
    fn captioned_dropdown(width: f32, control_w: f32) -> (Rect, Rect, Rect) {
        let mut b = Builder::new(0.0, 0.0, width, &measure);
        b.card_begin();
        b.row_dropdown(Id::AddToStack, "Put in a stack", Some("A long enough caption."), "Choose", control_w, true);
        b.card_end();
        let label = b
            .elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { style, .. } if *style == TextStyle::Body))
            .expect("the label")
            .rect;
        let caption = b
            .elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { style, .. } if *style == TextStyle::Caption))
            .expect("the caption")
            .rect;
        let control = b.elements.iter().find(|e| matches!(e.kind, Kind::Dropdown { .. })).expect("the dropdown").rect;
        (label, caption, control)
    }

    /// The bug this guards is the one the author saw: a caption drawn straight
    /// through the control beside it, both sets of glyphs in the same pixels.
    /// Whatever shape the row takes, its words and its control never share
    /// a pixel and never leave the row.
    #[test]
    fn words_and_the_control_never_share_pixels_at_any_width() {
        for width in [300.0, 400.0, 500.0, 656.0] {
            let (label, caption, control) = captioned_dropdown(width, 240.0);
            assert!(!label.intersects(&control), "label {label:?} meets control {control:?} at {width}");
            assert!(!caption.intersects(&control), "caption {caption:?} meets control {control:?} at {width}");
            assert!(!label.intersects(&caption), "label {label:?} meets caption {caption:?} at {width}");
            assert!(label.w > 0.0 && control.right() <= width - ROW_PAD + 0.01, "at {width}");
        }
    }

    /// A wide row is a Settings row: caption under the label in the label's
    /// column, the control centered on the row at the right.
    #[test]
    fn a_wide_row_keeps_the_caption_under_its_label_beside_the_control() {
        let (label, caption, control) = captioned_dropdown(656.0, 240.0);
        assert_eq!(caption.x, label.x);
        assert!(caption.y >= label.bottom());
        assert!(caption.right() <= control.x);
        assert!(control.x > label.right());
    }

    /// Where the label fits beside the control but its caption would wrap
    /// into a ribbon, the caption alone runs under the control at the whole
    /// card's width. Held to the label's width it became a column four words
    /// across.
    #[test]
    fn a_narrower_row_runs_the_caption_under_the_control_at_full_width() {
        let (label, caption, control) = captioned_dropdown(500.0, 240.0);
        assert!(control.x > label.right());
        assert!(caption.y >= control.bottom());
        assert!(caption.w > 500.0 - 240.0, "caption is only {} wide", caption.w);
    }

    /// And where the label cannot fit beside the control at all - the
    /// inspector at the window's minimum width - the control goes under the
    /// words and takes the row. The alternative was a label of negative
    /// width that never drew.
    #[test]
    fn a_row_too_narrow_for_its_label_puts_the_control_underneath() {
        let (label, caption, control) = captioned_dropdown(300.0, 240.0);
        assert!(control.y >= caption.bottom());
        assert_eq!(control.x, label.x);
        assert_eq!(control.w, 300.0 - 2.0 * ROW_PAD);
    }

    /// Every row's words are inset by the same padding from the card, top
    /// and side, whatever the row holds.
    #[test]
    fn every_row_insets_its_first_line_by_the_row_padding() {
        let mut b = Builder::new(0.0, 0.0, 656.0, &measure);
        b.card_begin();
        b.row_toggle(Id::Show, "A", Some("Caption."), true, true);
        b.row_info("B", Some("Caption."));
        b.row_status(Ink::Success, "C", Some("Caption."));
        b.row_paragraph("D");
        b.card_end();
        let rows: Vec<Rect> = b
            .elements
            .iter()
            .filter(|e| matches!(&e.kind, Kind::Text { text, style, .. } if text.len() == 1 && *style == TextStyle::Body))
            .map(|e| e.rect)
            .collect();
        assert_eq!(rows.len(), 4);
        for rect in rows {
            assert!(rect.x >= ROW_PAD, "{rect:?}");
            let row_top = b
                .elements
                .iter()
                .filter(|e| matches!(e.kind, Kind::Divider))
                .map(|e| e.rect.y)
                .filter(|y| *y <= rect.y)
                .fold(0.0, f32::max);
            assert_eq!(rect.y - row_top, ROW_PAD, "{rect:?}");
        }
    }

    /// A composer caption that does not fit on one line wraps and the row
    /// grows, rather than being cut mid-word.
    #[test]
    fn a_composer_caption_wraps_instead_of_being_cut() {
        let mut b = Builder::new(0.0, 0.0, 256.0, &measure);
        let item = StripItem::Module(ModuleId::Weather);
        b.list_row(item, Color(0x38BDF8), Mark::Glyph('W'), "Weather", "Condition mark and temperature", true, false, false);
        let caption = b
            .elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { style, wrap, .. } if *style == TextStyle::Caption && *wrap))
            .expect("a wrapped caption");
        let row = b.elements.iter().find(|e| e.id == Id::Row(item)).unwrap().rect;
        assert!(caption.rect.h > TextStyle::Caption.line_dip());
        assert!(row.h > LIST_ROW_H);
        assert!(caption.rect.bottom() <= row.bottom());
    }

    #[test]
    fn a_stack_reading_with_a_label_field_puts_it_under_the_name() {
        let mut b = Builder::new(0.0, 0.0, 376.0, &measure);
        b.card_begin();
        let label = MetricLabel { text: "", placeholder: "CPU", active: false };
        b.row_metric(0, Color(0x3B82F6), Mark::Glyph('C'), "CPU total", "CPU", Some(label), true, true);
        b.card_end();
        let field = b.elements.iter().find(|e| e.id == Id::MetricLabel(0)).expect("the field");
        let name = b.elements.iter().find(|e| matches!(&e.kind, Kind::Text { text, .. } if text == "CPU total")).unwrap();
        assert!(field.rect.y >= name.rect.bottom());
        assert!(matches!(&field.kind, Kind::Field { placeholder, .. } if placeholder == "CPU"));
        let remove = b.elements.iter().find(|e| e.id == Id::MetricRemove(0)).unwrap();
        assert!(field.rect.right() <= remove.rect.x);
    }

    #[test]
    fn the_inspector_header_keeps_its_words_inside_the_column() {
        let mut b = Builder::new(24.0, 0.0, 376.0, &measure);
        b.header(Color(0x3B82F6), Mark::Module(ModuleId::Cpu), "Processor", "What the processor is doing, as a percentage.");
        let tile = b.elements.iter().find(|e| matches!(e.kind, Kind::Mark { .. })).unwrap().rect;
        for element in b.elements.iter().filter(|e| matches!(e.kind, Kind::Text { .. })) {
            assert!(element.rect.x >= tile.right(), "{:?} runs under the tile", element.rect);
            assert!(element.rect.right() <= 24.0 + 376.0 + 1e-3, "{:?} runs past the column", element.rect);
        }
    }

    #[test]
    fn a_dropdown_pair_names_each_half_and_shares_one_row() {
        let mut b = Builder::new(0.0, 0.0, 656.0, &measure);
        b.card_begin();
        b.row_dropdown_pair(
            "Weight",
            None,
            [
                DropdownSpec { id: Id::HeadingWeight, name: "Headings", value: "Semibold" },
                DropdownSpec { id: Id::Weight, name: "Values", value: "Regular" },
            ],
        );
        b.card_end();
        let headings = b.elements.iter().find(|e| e.id == Id::HeadingWeight).unwrap().rect;
        let values = b.elements.iter().find(|e| e.id == Id::Weight).unwrap().rect;
        assert_eq!(headings.y, values.y);
        assert!(headings.right() < values.x);
        let dividers = b.elements.iter().filter(|e| matches!(e.kind, Kind::Divider)).count();
        assert_eq!(dividers, 0);
        assert!(b.elements.iter().any(|e| matches!(&e.kind, Kind::Text { text, .. } if text == "Headings")));
    }

    #[test]
    fn a_long_caption_makes_its_row_taller() {
        let mut b = Builder::new(0.0, 0.0, 300.0, &measure);
        b.card_begin();
        let long = "x".repeat(120);
        b.row_toggle(Id::Show, "A", Some(&long), true, true);
        b.card_end();
        assert!(b.y() > 64.0);
    }

    #[test]
    fn sections_are_spaced_only_after_the_first() {
        let mut b = Builder::new(0.0, 100.0, 300.0, &measure);
        b.section("First");
        assert_eq!(b.y(), 100.0 + 20.0 + 8.0);
        b.section("Second");
        assert_eq!(b.y(), 128.0 + SECTION_GAP + 28.0);
    }

    #[test]
    fn the_composer_row_carries_a_toggle_that_does_not_select() {
        let mut b = Builder::new(0.0, 0.0, 256.0, &measure);
        let item = StripItem::Module(ModuleId::Cpu);
        b.list_row(item, Color(0x3B82F6), Mark::Glyph('\u{E950}'), "CPU", "Label over value", true, false, false);
        assert_eq!(hit(&b.elements, 240.0, 26.0), Some(Id::RowToggle(item)));
        assert_eq!(hit(&b.elements, 100.0, 26.0), Some(Id::Row(item)));
        assert_eq!(b.y(), LIST_ROW_H);
    }

    #[test]
    fn slider_values_snap_to_the_step_and_stay_in_range() {
        let rect = Rect::new(0.0, 0.0, 220.0, 20.0);
        assert_eq!(slider_value_at(rect, -50.0, 7.0, 18.0, 1.0), 7.0);
        assert_eq!(slider_value_at(rect, 500.0, 7.0, 18.0, 1.0), 18.0);
        let middle = slider_value_at(rect, 110.0, 0.0, 10.0, 1.0);
        assert_eq!(middle, 5.0);
        assert_eq!(slider_value_at(rect, 110.0, 5.0, 60.0, 5.0), 35.0);
        assert_eq!(slider_fraction(12.0, 7.0, 18.0), 5.0 / 11.0);
        assert_eq!(slider_fraction(3.0, 5.0, 5.0), 0.0);
    }

    #[test]
    fn a_popup_opens_below_when_it_fits_and_above_when_there_is_more_room_there() {
        let anchor = Rect::new(100.0, 100.0, 160.0, 32.0);
        let below = popup_rect(anchor, 4, 640.0, 0.0);
        assert_eq!(below.y, anchor.bottom() + 4.0);
        assert_eq!(below.h, 4.0 * POPUP_ITEM_H + 8.0);
        assert_eq!(below.x, anchor.x);
        assert_eq!(below.w, anchor.w);
        let low = Rect::new(100.0, 560.0, 160.0, 32.0);
        let above = popup_rect(low, 4, 640.0, 0.0);
        assert_eq!(above.bottom(), low.y - 4.0);
        // Never taller than the room it has.
        let tall = popup_rect(anchor, 40, 640.0, 0.0);
        assert!(tall.bottom() <= 640.0);
    }

    /// A list wider than its control grows to the left, keeping its right
    /// edge on the control's, and never past the window's edge.
    #[test]
    fn a_popup_wider_than_its_control_grows_to_the_left() {
        let anchor = Rect::new(400.0, 100.0, 160.0, 32.0);
        let wide = popup_rect(anchor, 3, 640.0, 300.0);
        assert_eq!(wide.w, 300.0);
        assert_eq!(wide.right(), anchor.right());
        let near_edge = Rect::new(20.0, 100.0, 160.0, 32.0);
        let clamped = popup_rect(near_edge, 3, 640.0, 300.0);
        assert!(clamped.x >= 8.0);
    }

    #[test]
    fn the_scroll_thumb_exists_only_when_there_is_something_to_scroll() {
        assert_eq!(scroll_thumb(600.0, 500.0, 0.0), None);
        let (y, h) = scroll_thumb(600.0, 1200.0, 0.0).unwrap();
        assert_eq!(y, 0.0);
        assert_eq!(h, 300.0);
        let (y, _) = scroll_thumb(600.0, 1200.0, 600.0).unwrap();
        assert_eq!(y, 300.0);
        assert_eq!(clamp_scroll(900.0, 600.0, 1200.0), 600.0);
        assert_eq!(clamp_scroll(-5.0, 600.0, 1200.0), 0.0);
    }

    #[test]
    fn buttons_never_shrink_below_the_minimum_and_grow_for_a_glyph() {
        let b = Builder::new(0.0, 0.0, 300.0, &measure);
        assert_eq!(b.button_width("OK", false), BUTTON_MIN_W);
        let long = b.button_width("Install LibreHardwareMonitor", false);
        assert!(long > BUTTON_MIN_W);
        assert_eq!(b.button_width("Install LibreHardwareMonitor", true), long + 20.0);
    }

    #[test]
    fn the_edit_child_sits_past_the_search_glyph() {
        let field = Rect::new(10.0, 10.0, 300.0, 32.0);
        assert_eq!(field_edit_rect(field, false).x, 22.0);
        assert_eq!(field_edit_rect(field, true).x, 42.0);
        assert_eq!(field_edit_rect(field, false).h, 20.0);
    }
}
