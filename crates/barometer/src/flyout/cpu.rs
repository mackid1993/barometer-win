// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The processor flyout, ported from CPUDropdownView in DropdownViews.swift.
//
// What the Mac shows, in its order: a hero header with the module's tile,
// the busy figure and how it splits between user and kernel time; a History
// card with a range picker, an area graph and three chips; a Cores card with
// a bar for every logical processor; a System card of counts; and the
// processes using the most of it. Everything is said through the Builder in
// ui.rs, so `CpuFlyout` lays out and never draws, and the arithmetic that
// decides where the core grid wraps and which samples the graph plots is a
// function with tests. `CpuContent` is the thin end that the chrome's
// `Content` trait sees: it reads the feed and hands presses on.
//
// Two departures, both from docs/ui-layouts.md section 3.1. Load average is
// a Unix figure Windows does not keep, so the System card counts processes,
// threads and handles instead, which is what Task Manager shows in the same
// place. And the Mac's P/E labels are used only where the OS reports an
// efficiency class: the Swift labels anything not "performance" as E, which
// on a plain eight-core part would call every core an efficiency core, so a
// core of unknown kind is C1 through C8 here.
//
// The processes come from barometer-core's sys::processes worker, sampled on
// its own cadence; the strip's thread folds its latest sample into the
// snapshot it publishes. Ending one is TerminateProcess with no question
// asked, which is what the Mac does for a process of the user's own.
//
// HistoryRange, the timeline, the range picker and the chip helpers live
// here and the GPU and memory flyouts borrow them, as the Swift keeps
// HistoryRange and TimelineGraphData in DropdownViews.swift beside the CPU
// view for the same panels. They belong in ui.rs once the vocabulary takes
// them in.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use barometer_core::format;
use barometer_core::modules::LoadAverage;
use barometer_core::sys::processes;
use barometer_core::ModuleId;

use super::ui::{Accent, Builder, Graph, Id, Ink, Kind, Measure, Style, MEASURING};
use super::{Content, Context, Page, Response};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::model::module_glyph;
use crate::settings_ui::theme::{self, glyph, Color};

pub use processes::CoreKind;

/// The interface glyphs the process and address rows draw, named here for
/// the reason weather.rs names its own: theme.rs's table belongs to the
/// settings window.
pub(super) mod icons {
    /// CommandPrompt, standing where the Mac draws the application's icon.
    /// The Mac's own fallback for a process with no application behind it
    /// is terminal.fill, so a prompt is the honest mark for a process the
    /// shell has no icon for - which is all this stands in for now that
    /// `appicon` fetches the real ones.
    pub const PROCESS: &str = "\u{E756}";
    /// Copy, from the table in docs/ui-design.md section 6.
    pub const COPY: &str = "\u{E8C8}";
}

/// What stands in for a reading that is not there, as in the Swift.
pub const DASH: &str = "\u{2014}";

/// The chip color for a share that belongs to nobody - idle time, free
/// memory, an unselected range. The design's stroke.strong, which washes to
/// a faint neutral over a light card and a dark one alike, where a theme
/// gray would need the theme this layout does not have.
pub(super) const NEUTRAL: Color = Color(0x8A8A8A);

/// The far end of an efficiency core's bar, from CoreBar in the Swift.
const EFFICIENCY_TIP: Color = Color(0x6EE7B7);

/// Every bar in these panels is the track the weather's day rows and the
/// disks' volume rows use, whatever height the Swift gave its own; one
/// weight of bar is what makes the panels read as a set.
pub const BAR_H: f32 = 6.0;

// MARK: - Shared with the GPU and memory flyouts

/// The spans the history graph can show, from HistoryRange.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum HistoryRange {
    OneMinute,
    #[default]
    FiveMinutes,
    ThirtyMinutes,
    ThreeHours,
    TwentyFourHours,
}

/// The picker's chips answer to RANGE_ID plus the range's index, and a
/// panel's other controls start well above them.
const RANGE_ID: u32 = 1;

impl HistoryRange {
    pub const ALL: [HistoryRange; 5] = [
        HistoryRange::OneMinute,
        HistoryRange::FiveMinutes,
        HistoryRange::ThirtyMinutes,
        HistoryRange::ThreeHours,
        HistoryRange::TwentyFourHours,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HistoryRange::OneMinute => "1m",
            HistoryRange::FiveMinutes => "5m",
            HistoryRange::ThirtyMinutes => "30m",
            HistoryRange::ThreeHours => "3h",
            HistoryRange::TwentyFourHours => "24h",
        }
    }

    pub fn seconds(self) -> i64 {
        match self {
            HistoryRange::OneMinute => 60,
            HistoryRange::FiveMinutes => 300,
            HistoryRange::ThirtyMinutes => 1_800,
            HistoryRange::ThreeHours => 10_800,
            HistoryRange::TwentyFourHours => 86_400,
        }
    }

    fn index(self) -> u32 {
        HistoryRange::ALL.iter().position(|r| *r == self).unwrap_or(0) as u32
    }

    /// The control id of this range's chip in the picker.
    pub fn id(self) -> Id {
        Id::Custom(RANGE_ID + self.index())
    }

    /// The range whose chip answers to `id`, if it is one of the picker's.
    pub fn from_id(id: Id) -> Option<HistoryRange> {
        match id {
            Id::Custom(n) if n >= RANGE_ID => HistoryRange::ALL.get((n - RANGE_ID) as usize).copied(),
            _ => None,
        }
    }
}

/// One reading on a module's timeline: when it was taken, and the fraction
/// then. What the strip's thread keeps a day of and publishes with each
/// snapshot, so that the graph is drawn from what happened while the panel
/// was closed as well as while it was open.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Sample {
    pub at_unix: i64,
    /// Zero to one.
    pub value: f32,
}

/// The most points a graph is drawn from: the Mac's downsampled(to: 300).
pub const GRAPH_POINTS: usize = 300;

/// Adds a sample to a timeline and forgets what the widest range can no
/// longer show, which is how the strip's thread keeps a day of readings
/// without the queue growing for the life of the process.
///
/// A `VecDeque` rather than a `Vec`. The widest range is a day at a sample a
/// second, so once the queue is full every tick dropped one sample off the
/// front of 86,400 - and `Vec::drain(..1)` shifts all the rest down a slot,
/// which is a 1.4 MB memmove per timeline per second, forever, on a machine
/// that has been left running. Dropping the front of a deque is an index.
pub fn remember(history: &mut VecDeque<Sample>, sample: Sample) {
    history.push_back(sample);
    let horizon = sample.at_unix - HistoryRange::TwentyFourHours.seconds();
    while history.front().is_some_and(|s| s.at_unix < horizon) {
        history.pop_front();
    }
}

/// The graph's series for the `seconds` ending at the newest sample, from
/// TimelineGraphData.make: the samples inside that window, thinned to
/// GRAPH_POINTS with the newest always kept, each placed along the width by
/// its time rather than its index, so an hour the machine slept through
/// shows as a gap and not as a seam.
pub fn timeline(samples: &[Sample], seconds: i64) -> (Vec<f32>, Vec<f32>) {
    let Some(end) = samples.last().map(|s| s.at_unix) else { return (Vec::new(), Vec::new()) };
    let duration = seconds.max(1);
    let start = end - duration;
    let visible: Vec<&Sample> = samples.iter().filter(|s| s.at_unix >= start && s.at_unix <= end).collect();
    let stride = visible.len().div_ceil(GRAPH_POINTS).max(1);
    let last = visible.len().saturating_sub(1);
    let mut values = Vec::with_capacity(visible.len() / stride + 1);
    let mut positions = Vec::with_capacity(values.capacity());
    for (index, sample) in visible.iter().enumerate() {
        if (last - index) % stride != 0 {
            continue;
        }
        values.push(sample.value.clamp(0.0, 1.0));
        positions.push(((sample.at_unix - start) as f32 / duration as f32).clamp(0.0, 1.0));
    }
    (values, positions)
}

/// How long the timeline covers, newest to oldest, and never nothing.
pub fn timeline_span(samples: &[Sample]) -> i64 {
    match (samples.first(), samples.last()) {
        (Some(first), Some(last)) => (last.at_unix - first.at_unix).max(1),
        _ => 1,
    }
}

/// An area graph of a timeline over a range, in a module's accent.
pub(super) fn history_graph(samples: &[Sample], seconds: i64, accent: Accent) -> Graph {
    let (values, positions) = timeline(samples, seconds);
    let mut graph = Graph::area(values, accent);
    graph.positions = Some(positions);
    graph
}

/// A chip's height, which chip_at fixes.
const CHIP_H: f32 = 20.0;
/// Between chips laid in a row: the Swift's HStack spacing.
const CHIP_GAP: f32 = 6.0;
/// Between the picker's chips: the Swift's 2 inside CapsulePicker.
const PICKER_GAP: f32 = 2.0;

/// A chip with its left edge at `x`, returning where it landed.
///
/// chip_at right-aligns inside an area. A zero-width area whose right edge
/// is `x` lands the chip just left of it, and moving it by its own width
/// puts it at `x` with the vocabulary's sizing and no second copy of the
/// formula that decides it.
pub(super) fn chip_left(
    b: &mut Builder<'_>,
    x: f32,
    y: f32,
    text: &str,
    color: Color,
    glyph: Option<&'static str>,
) -> Rect {
    b.chip_at(Rect::new(x, y, 0.0, CHIP_H), text, color, glyph);
    let chip = b.elements.last_mut().expect("chip_at pushes one element");
    chip.rect.x += chip.rect.w;
    chip.rect
}

/// Chips left to right from the card's inner edge, wrapping to a new line
/// where they would run past it, and advancing the column past them.
pub(super) fn chip_row(b: &mut Builder<'_>, chips: &[(String, Color)]) {
    let (left, right) = (b.inner_x(), b.inner_x() + b.inner_w());
    let mut x = left;
    let mut y = b.y();
    for (index, (text, color)) in chips.iter().enumerate() {
        let mut placed = chip_left(b, x, y, text, *color, None);
        // Only the placed chip knows its full width, so it is moved down a
        // line after the fact rather than measured twice beforehand.
        if index > 0 && placed.right() > right {
            x = left;
            y += CHIP_H + CHIP_GAP;
            let chip = b.elements.last_mut().expect("chip_left pushes one element");
            chip.rect.x = x;
            chip.rect.y = y;
            placed = chip.rect;
        }
        x = placed.right() + CHIP_GAP;
    }
    b.advance(y - b.y() + CHIP_H);
}

/// The range picker as a section label's trailing accessory: the five spans
/// as chips, right-aligned in `area`, the chosen one in the accent and the
/// rest neutral, each answering to its range's id.
///
/// The Swift's CapsulePicker fills the chosen capsule with the accent and
/// paints the label white on it; the vocabulary's chip is the accent at 14%
/// with the text in the ordinary ink, which is quieter but is the same shape
/// the chips beside it use, and the dot on the chip says which is chosen in
/// monochrome too.
pub(super) fn range_picker(b: &mut Builder<'_>, area: Rect, selected: HistoryRange, accent: Accent) {
    let first = b.elements.len();
    let mut right = area.right();
    for range in HistoryRange::ALL.iter().rev() {
        let color = if *range == selected { accent.primary } else { NEUTRAL };
        b.chip_at(Rect::new(area.x, area.y, (right - area.x).max(0.0), area.h), range.label(), color, None);
        let chip = b.elements.last_mut().expect("chip_at pushes one element");
        chip.id = range.id();
        chip.interactive = true;
        right = chip.rect.x - PICKER_GAP;
    }
    // Placed from the right edge inward, so they were pushed last first;
    // put them back in the picker's own order for whoever walks them.
    b.elements[first..].reverse();
}

/// A share on a track: the vocabulary's capsule, in a rectangle of the
/// caller's rather than across the card, for a bar under a name or in a
/// grid cell.
pub(super) fn share_bar(b: &mut Builder<'_>, bar: Rect, fraction: f32, accent: Accent, glow: bool) {
    b.passive(bar, Kind::Track);
    b.passive(
        bar,
        Kind::Capsule { fraction: fraction.clamp(0.0, 1.0), color: accent.primary, color2: accent.secondary, glow },
    );
}

/// A fraction as a percentage with one decimal, the Swift's "%.1f%%" for a
/// headline figure. The strip's whole-number percent is for a column that
/// must not jitter; a headline has room for the tenth.
pub(super) fn percent1(fraction: f32) -> String {
    format!("{:.1}%", fraction.clamp(0.0, 1.0) * 100.0)
}

/// The latest value in a feed's slot, if the slot can be read.
pub(super) fn latest<T: Clone>(slot: &Mutex<T>) -> Option<T> {
    slot.lock().ok().map(|value| value.clone())
}

// MARK: - The processor

/// One logical processor's share of the interval, from CPUCoreSample.
#[derive(Clone, Debug, PartialEq)]
pub struct CoreLoad {
    pub index: usize,
    pub kind: CoreKind,
    /// Busy time as a fraction, zero to one.
    pub load: f32,
}

/// A process and its share of the whole machine, from CPUProcessSample.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessLoad {
    pub pid: u32,
    pub name: String,
    /// Of every processor together, zero to one.
    pub load: f32,
}

/// Everything the panel says, from CPUSample.
///
/// Every field is optional or empty because the pieces arrive from
/// different places at different times - the total from the strip's own
/// module on its tick, the cores and processes from the sys::processes
/// worker on its cadence, the counts from one GetPerformanceInfo call - and
/// what has not arrived is drawn as a dash rather than guessed. Fractions
/// throughout, zero to one, so that the graph, the bars and the chips share
/// one scale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuSnapshot {
    /// Busy time as a fraction of all processor time since the last sample.
    pub total: Option<f32>,
    /// The busy fraction split between user and kernel mode. Idle is what
    /// is left of the whole.
    pub user: Option<f32>,
    pub system: Option<f32>,
    /// How many logical processors there are, for the Cores card's summary
    /// before the first per-core figures arrive.
    pub logical_processors: Option<usize>,
    /// From processes::Sample::cores zipped with processes::core_kinds.
    pub cores: Vec<CoreLoad>,
    /// One, five and fifteen minutes of the ready queue, from
    /// CpuModule::load_average - the Mac's getloadavg by another route.
    pub load_average: Option<LoadAverage>,
    /// From processes::SystemSummary.
    pub uptime_secs: Option<u64>,
    pub processes: Option<u32>,
    pub threads: Option<u32>,
    pub handles: Option<u32>,
    /// The processes using the most, busiest first: processes::Sample::
    /// top_by_cpu, mapped.
    pub top: Vec<ProcessLoad>,
    /// Whether the per-process shares are being measured rather than
    /// missing: a share is a difference between two reads of the processor
    /// times, and the read taken when the panel opened has nothing to
    /// difference against yet. An empty list would say the machine is idle,
    /// which is not what is known - the rule is that a reading which has not
    /// arrived is drawn as unavailable and never as a number.
    pub measuring: bool,
    /// The busy fraction at each sample of the last day, oldest first, kept
    /// with `remember`. The picker chooses how much of it the graph shows.
    pub history: Vec<Sample>,
}

/// What the panel asks for after a press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CpuAction {
    /// The panel changed; lay it out again.
    Rebuild,
    /// The end-task glyph on a process row, from CPUProcessRow.terminate.
    /// The confirmation the Swift shows for another user's process is the
    /// chrome's to put up; the panel only says which process was meant.
    EndTask { pid: u32, name: String },
}

/// A process row, by its place in the list; the row is only ever washed under
/// the pointer, so which row it is is all that matters.
const PROCESS_ID: u32 = 100;

/// The end-task glyph, by the process id of the row it sits on.
///
/// By the row's *index* before, which meant the press ended whatever process
/// had climbed into that slot by the time it landed: the list is re-sampled
/// every two seconds and re-sorted by load, and there is no confirmation
/// between the press and the kill. The row the user aimed at is the one named
/// on it, so the id carries the process rather than the position.
const END_TASK_ID: u32 = 1 << 31;

/// The processor flyout's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuFlyout {
    pub snapshot: CpuSnapshot,
    pub range: HistoryRange,
    /// ModuleSettings.showsProcesses and processCount on the Mac. The
    /// Windows settings have no such switch yet, so these are the Mac's
    /// defaults: shown, five of them.
    pub show_processes: bool,
    pub process_count: usize,
}

impl CpuFlyout {
    pub fn new(snapshot: CpuSnapshot) -> CpuFlyout {
        CpuFlyout { snapshot, range: HistoryRange::default(), show_processes: true, process_count: 5 }
    }

    pub fn title(&self) -> &'static str {
        "CPU"
    }

    /// How tall the content is at a width, in DIPs: what the builder
    /// walked, the gap under the last card included, as Page::finish
    /// counts it.
    pub fn height(&self, width: f32, measure: Measure<'_>) -> f32 {
        let mut b = Builder::new(0.0, 0.0, width, measure);
        self.build(&mut b);
        b.y()
    }

    /// A press on one of the panel's controls.
    pub fn activate(&mut self, id: Id) -> Option<CpuAction> {
        if let Some(range) = HistoryRange::from_id(id) {
            if range == self.range {
                return None;
            }
            self.range = range;
            return Some(CpuAction::Rebuild);
        }
        let Id::Custom(n) = id else { return None };
        if n < END_TASK_ID {
            return None;
        }
        // Still shown, or the row has scrolled out from under the press since
        // it was drawn and there is nothing the user could have meant.
        let pid = n & !END_TASK_ID;
        self.shown_processes()
            .find(|process| process.pid == pid)
            .map(|process| CpuAction::EndTask { pid: process.pid, name: process.name.clone() })
    }

    fn shown_processes(&self) -> impl Iterator<Item = &ProcessLoad> {
        self.snapshot.top.iter().take(if self.show_processes { self.process_count } else { 0 })
    }

    /// Lays the whole panel out, in the Swift's order.
    pub fn build(&self, b: &mut Builder<'_>) {
        let s = &self.snapshot;
        let accent = Accent::signature(ModuleId::Cpu);
        let value = s.total.map(percent1);
        let subtitle = self.subtitle();
        b.hero_header(
            (theme::module_color(ModuleId::Cpu), module_glyph(ModuleId::Cpu)),
            self.title(),
            Some(&subtitle),
            Some(value.as_deref().unwrap_or(DASH)),
            accent,
        );

        // The history card stands whether or not there is a sample yet, as
        // the Swift's does; the graph is simply empty.
        let card = b.card_begin(Some(accent.primary));
        let trailing = b.section_label("History");
        range_picker(b, trailing, self.range, accent);
        b.graph(84.0, history_graph(&s.history, self.range.seconds(), accent));
        if let Some(chips) = self.share_chips(accent) {
            b.gap(8.0);
            chip_row(b, &chips);
        }
        b.card_end(card);

        // Everything below needs a sample.
        if s.total.is_none() {
            return;
        }

        let card = b.card_begin(None);
        let trailing = b.section_label("Cores");
        let summary = core_summary(s);
        if !summary.is_empty() {
            b.text(trailing, &summary, Style::Caption, Ink::Secondary, Align::Right);
        }
        if s.cores.is_empty() {
            b.caption("Per-core load arrives with the next sample.");
        } else {
            core_bars(b, &s.cores, accent);
        }
        b.card_end(card);

        let card = b.card_begin(None);
        b.section_label("System");
        // Above Uptime, as the Swift's System card puts it. The three are
        // written by the type that holds them, so the panel and anything
        // else that shows them agree.
        if let Some(load) = s.load_average {
            b.metric_row(None, "Load average", &load.describe(), Ink::Secondary);
        }
        b.metric_row(None, "Uptime", &uptime(s.uptime_secs), Ink::Secondary);
        b.metric_row(None, "Processes", &processes_text(s), Ink::Secondary);
        b.metric_row(None, "Handles", &count(s.handles), Ink::Secondary);
        b.card_end(card);

        if self.shown_processes().next().is_some() {
            let card = b.card_begin(None);
            b.section_label("Top processes");
            for (index, process) in self.shown_processes().enumerate() {
                process_row(b, index as u32, process, accent);
            }
            b.card_end(card);
        } else if self.show_processes && s.measuring {
            // The card is kept, with a sentence in place of the rows, so
            // that the panel does not grow a card a second after it opened.
            let card = b.card_begin(None);
            b.section_label("Top processes");
            b.caption(MEASURING);
            b.card_end(card);
        }
    }

    /// The line under the title: the user/system/idle split when the
    /// module reports one, busy against idle when it reports only the total.
    fn subtitle(&self) -> String {
        let s = &self.snapshot;
        let Some(total) = s.total else { return "Waiting for the first sample".to_string() };
        let idle = format::percent(1.0 - total);
        match (s.user, s.system) {
            (Some(user), Some(system)) => format!(
                "{} user  \u{00B7}  {} system  \u{00B7}  {} idle",
                format::percent(user),
                format::percent(system),
                idle
            ),
            _ => format!("{} busy  \u{00B7}  {} idle", format::percent(total), idle),
        }
    }

    /// The chips under the graph, in the Swift's colors: user in the
    /// accent, system in its second color, idle in nobody's.
    fn share_chips(&self, accent: Accent) -> Option<Vec<(String, Color)>> {
        let s = &self.snapshot;
        let total = s.total?;
        let idle = (format!("Idle {}", format::percent(1.0 - total)), NEUTRAL);
        Some(match (s.user, s.system) {
            (Some(user), Some(system)) => vec![
                (format!("User {}", format::percent(user)), accent.primary),
                (format!("System {}", format::percent(system)), accent.secondary),
                idle,
            ],
            _ => vec![(format!("Busy {}", format::percent(total)), accent.primary), idle],
        })
    }
}

/// The processor's content: the panel's end of the feed, and the layout.
pub struct CpuContent {
    slot: Arc<Mutex<CpuSnapshot>>,
    flyout: CpuFlyout,
    /// What to do about a process the user asked to end. The Swift sends
    /// SIGTERM from the view and so, by default, does this: TerminateProcess
    /// with no question asked. A caller that wants to ask first, or a test,
    /// puts its own action here.
    end_task: Box<dyn FnMut(u32, &str) + Send>,
}

impl CpuContent {
    /// Content reading from a feed's slot, as `Flyout::feed` hands it out.
    pub fn new(slot: Arc<Mutex<CpuSnapshot>>) -> CpuContent {
        CpuContent {
            slot,
            flyout: CpuFlyout::new(CpuSnapshot::default()),
            end_task: Box::new(|pid, _name| {
                // A process that will not be ended - another user's, or
                // Windows' own - is left standing without a word: the Mac
                // asks first for those, and the row is still there to say
                // nothing happened.
                let _ = processes::terminate(pid);
            }),
        }
    }

    /// What to do when the user presses a process row's end-task glyph,
    /// instead of ending it outright.
    pub fn on_end_task(mut self, action: impl FnMut(u32, &str) + Send + 'static) -> CpuContent {
        self.end_task = Box::new(action);
        self
    }
}

impl Content for CpuContent {
    fn module(&self) -> ModuleId {
        ModuleId::Cpu
    }

    fn build(&mut self, cx: &Context) -> Page {
        if let Some(snapshot) = latest(&self.slot) {
            self.flyout.snapshot = snapshot;
        }
        let mut b = cx.builder();
        self.flyout.build(&mut b);
        Page::finish(b)
    }

    fn activate(&mut self, id: Id) -> Response {
        match self.flyout.activate(id) {
            Some(CpuAction::Rebuild) => Response::Relayout,
            Some(CpuAction::EndTask { pid, name }) => {
                (self.end_task)(pid, &name);
                Response::None
            }
            None => Response::None,
        }
    }

    fn tick(&mut self) -> bool {
        self.slot.lock().map(|slot| *slot != self.flyout.snapshot).unwrap_or(false)
    }
}

// MARK: - Cores

/// The Swift's LazyVGrid: columns at least this wide, this far apart.
pub const CORE_MIN_W: f32 = 64.0;
pub const CORE_GAP: f32 = 8.0;
/// A cell: a caption line with the label and the figure, 4 under it, and
/// the bar.
pub const CORE_CELL_H: f32 = 16.0 + 4.0 + BAR_H;

/// How the core bars wrap in a card.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CoreGrid {
    pub columns: usize,
    pub cell_w: f32,
    pub rows: usize,
}

impl CoreGrid {
    /// As many columns of at least CORE_MIN_W as fit, which is what an
    /// adaptive grid does, and never none.
    pub fn fit(count: usize, width: f32) -> CoreGrid {
        let columns = (((width + CORE_GAP) / (CORE_MIN_W + CORE_GAP)).floor() as usize).max(1);
        let cell_w = (width - (columns as f32 - 1.0) * CORE_GAP) / columns as f32;
        CoreGrid { columns, cell_w, rows: count.div_ceil(columns) }
    }

    pub fn height(&self) -> f32 {
        if self.rows == 0 {
            return 0.0;
        }
        self.rows as f32 * CORE_CELL_H + (self.rows as f32 - 1.0) * CORE_GAP
    }

    /// Where the `index`th cell's top-left corner is, from the grid's.
    pub fn origin(&self, index: usize) -> (f32, f32) {
        let column = index % self.columns;
        let row = index / self.columns;
        (column as f32 * (self.cell_w + CORE_GAP), row as f32 * (CORE_CELL_H + CORE_GAP))
    }
}

/// "P1", "E1", or "C1" for a core whose kind the OS does not say.
pub fn core_label(core: &CoreLoad) -> String {
    let letter = match core.kind {
        CoreKind::Performance => 'P',
        CoreKind::Efficiency => 'E',
        CoreKind::Unknown => 'C',
    };
    format!("{letter}{}", core.index + 1)
}

/// The Cores card's summary, from CPUDropdownView.coreSummary: the P/E
/// split when there is one, a plain count when there is not, and the
/// processor count when no core has been measured yet.
pub fn core_summary(s: &CpuSnapshot) -> String {
    if s.cores.is_empty() {
        return match s.logical_processors {
            Some(n) => format!("{n} logical processors"),
            None => String::new(),
        };
    }
    let performance = s.cores.iter().filter(|c| c.kind == CoreKind::Performance).count();
    let efficiency = s.cores.iter().filter(|c| c.kind == CoreKind::Efficiency).count();
    if performance > 0 && efficiency > 0 {
        format!("{performance} performance \u{00B7} {efficiency} efficiency")
    } else {
        format!("{} cores", s.cores.len())
    }
}

fn core_bars(b: &mut Builder<'_>, cores: &[CoreLoad], accent: Accent) {
    let grid = CoreGrid::fit(cores.len(), b.inner_w());
    let (x0, y0) = (b.inner_x(), b.y());
    for (index, core) in cores.iter().enumerate() {
        let (dx, dy) = grid.origin(index);
        let (x, y) = (x0 + dx, y0 + dy);
        let figure = format::percent(core.load);
        let figure_w = b.text_width(&figure, Style::Caption);
        b.text(
            Rect::new(x, y, (grid.cell_w - figure_w - 4.0).max(0.0), 16.0),
            &core_label(core),
            Style::CaptionStrong,
            Ink::Secondary,
            Align::Left,
        );
        b.text(Rect::new(x, y, grid.cell_w, 16.0), &figure, Style::Caption, Ink::Primary, Align::Right);
        let bar = Rect::new(x, y + CORE_CELL_H - BAR_H, grid.cell_w, BAR_H);
        let colors = match core.kind {
            CoreKind::Efficiency => Accent { primary: accent.secondary, secondary: EFFICIENCY_TIP },
            _ => accent,
        };
        share_bar(b, bar, core.load, colors, true);
    }
    b.advance(grid.height());
}

// MARK: - System

/// "3d 4h 12m", or "4h 12m" inside the first day, from CPUDropdownView.uptime.
pub fn uptime(secs: Option<u64>) -> String {
    let Some(secs) = secs else { return "Unavailable".to_string() };
    let days = secs / 86_400;
    let hours = secs % 86_400 / 3_600;
    let minutes = secs % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else {
        format!("{hours}h {minutes}m")
    }
}

fn count(value: Option<u32>) -> String {
    value.map(|n| n.to_string()).unwrap_or_else(|| DASH.to_string())
}

/// "1234  ·  18765 threads", or whichever half is known.
fn processes_text(s: &CpuSnapshot) -> String {
    match (s.processes, s.threads) {
        (Some(processes), Some(threads)) => format!("{processes}  \u{00B7}  {threads} threads"),
        (Some(processes), None) => processes.to_string(),
        (None, Some(threads)) => format!("{threads} threads"),
        (None, None) => DASH.to_string(),
    }
}

// MARK: - Top processes

/// A process row, the network panel's height, so the three panels that
/// list processes list them alike.
pub const PROCESS_ROW_H: f32 = 36.0;
/// The share bar under the name goes no wider than this, as ProcessRow's.
const PROCESS_BAR_W: f32 = 120.0;

/// The Mac's 16-point application icon, and the gap after it.
pub(super) const PROCESS_MARK: f32 = 16.0;
const PROCESS_MARK_GAP: f32 = 8.0;

/// The mark at a process row's left, where the Mac draws the application's
/// icon, and where the name then starts.
///
/// The executable's own icon where the shell has one - `appicon` does that
/// lookup off this thread, because it wants the path the process list does
/// not carry and a shell call per row - and a neutral glyph in its place
/// where it does not, so the row keeps the Mac's anatomy either way.
pub(super) fn process_mark(b: &mut Builder<'_>, row: Rect, icon: Option<isize>) -> f32 {
    let x = row.x + 6.0;
    let cell = Rect::new(x, row.y + (row.h - PROCESS_MARK) / 2.0, PROCESS_MARK, PROCESS_MARK);
    // The executable's own icon, as the Mac's rows show the app's; the
    // neutral mark stands in for a process the shell has no icon for.
    match icon {
        Some(icon) => b.passive(cell, Kind::Icon { icon, size: PROCESS_MARK }),
        None => b.passive(cell, Kind::Glyph { glyph: icons::PROCESS, size: 12.0, ink: Ink::Tertiary }),
    }
    x + PROCESS_MARK + PROCESS_MARK_GAP
}

/// A process's name over its share of the machine, in a row of the network
/// panel's anatomy: the mark and the name at the row's inset, the figure at
/// the far inset, and here a bar under the name because the Swift's CPU and
/// memory rows carry one where its network rows do not.
pub(super) fn process_name_and_share(
    b: &mut Builder<'_>,
    row: Rect,
    name: &str,
    fraction: f32,
    right: f32,
    accent: Accent,
    icon: Option<isize>,
) {
    let x = process_mark(b, row, icon);
    let name_w = (right - x).max(24.0);
    // A 20 line and the bar with 4 between them, centered in the row.
    let top = row.y + (row.h - (20.0 + 4.0 + BAR_H)) / 2.0;
    b.text(Rect::new(x, top, name_w, 20.0), name, Style::Body, Ink::Primary, Align::Left);
    let bar = Rect::new(x, top + 20.0 + 4.0, name_w.min(PROCESS_BAR_W), BAR_H);
    share_bar(b, bar, fraction, accent, false);
}

/// One process, from ProcessRow with CPUProcessRow's trailing end-task
/// button. The row answers to an id so the chrome can wash it under the
/// pointer; the glyph answers to its own.
fn process_row(b: &mut Builder<'_>, index: u32, process: &ProcessLoad, accent: Accent) {
    let row = b.row_begin(Id::Custom(PROCESS_ID + index), PROCESS_ROW_H, false);
    let end = Rect::new(row.right() - 6.0 - 20.0, row.y, 20.0, row.h);
    let figure = percent1(process.load);
    let figure_w = b.text_width(&figure, Style::Body);
    let figure_x = end.x - 8.0 - figure_w;
    let icon = super::appicon::icon_for(process.pid);
    process_name_and_share(b, row, &process.name, process.load, figure_x - 10.0, accent, icon);
    b.text(Rect::new(figure_x, row.y, figure_w, row.h), &figure, Style::Body, Ink::Secondary, Align::Right);
    b.control(
        Id::Custom(END_TASK_ID | process.pid),
        end,
        Kind::Glyph { glyph: glyph::CANCEL, size: 12.0, ink: Ink::Tertiary },
    );
    b.advance(PROCESS_ROW_H);
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Element, Palette, CARD_GAP, CARD_PAD, PANEL_PAD, PANEL_W};
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

    /// Seven DIPs a character at body size, scaled by the style, as the
    /// vocabulary's own tests measure.
    pub fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    const COLUMN_W: f32 = PANEL_W - 2.0 * PANEL_PAD;

    fn cores(count: usize, kind: CoreKind) -> Vec<CoreLoad> {
        (0..count).map(|index| CoreLoad { index, kind, load: 0.25 + index as f32 * 0.05 }).collect()
    }

    /// A sample every two seconds for `count` samples, ending at `end`.
    fn samples(count: usize, end: i64) -> Vec<Sample> {
        (0..count)
            .map(|n| Sample { at_unix: end - 2 * (count as i64 - 1 - n as i64), value: (n % 10) as f32 / 10.0 })
            .collect()
    }

    fn full() -> CpuSnapshot {
        CpuSnapshot {
            total: Some(0.42),
            user: Some(0.38),
            system: Some(0.04),
            logical_processors: Some(8),
            cores: cores(8, CoreKind::Unknown),
            load_average: Some(LoadAverage { one: 1.24, five: 0.98, fifteen: 0.75 }),
            uptime_secs: Some(3 * 86_400 + 4 * 3_600 + 12 * 60),
            processes: Some(234),
            threads: Some(3_456),
            handles: Some(98_765),
            top: (0..7)
                .map(|n| ProcessLoad { pid: 1000 + n, name: format!("process{n}"), load: 0.3 - n as f32 * 0.03 })
                .collect(),
            measuring: false,
            history: samples(30, 1_000_000),
        }
    }

    fn layout(flyout: &CpuFlyout) -> Vec<Element> {
        let mut b = Builder::new(PANEL_PAD, PANEL_PAD, COLUMN_W, &measure);
        flyout.build(&mut b);
        b.elements
    }

    fn cards(elements: &[Element]) -> Vec<Rect> {
        elements.iter().filter(|e| matches!(e.kind, Kind::Card { .. })).map(|e| e.rect).collect()
    }

    fn text_of(element: &Element) -> Option<&str> {
        match &element.kind {
            Kind::Text { text, .. } | Kind::Chip { text, .. } => Some(text),
            _ => None,
        }
    }

    #[test]
    fn the_timeline_keeps_the_range_ending_at_the_newest_sample_and_places_each_by_its_time() {
        // Thirty samples two seconds apart: a minute's range takes the last
        // thirty-one seconds' worth, the newest at the right edge.
        let history = samples(30, 1_000_000);
        let (values, positions) = timeline(&history, HistoryRange::OneMinute.seconds());
        assert_eq!(values.len(), 30);
        assert_eq!(positions.len(), 30);
        assert_eq!(*positions.last().unwrap(), 1.0);
        // The oldest is 58 seconds back, so 2/60 of the way in.
        assert!((positions[0] - 2.0 / 60.0).abs() < 1e-6);
        assert!(positions.windows(2).all(|p| p[1] > p[0]));
        // Five minutes covers the same thirty samples, now bunched at the end.
        let (values, positions) = timeline(&history, HistoryRange::FiveMinutes.seconds());
        assert_eq!(values.len(), 30);
        assert!(positions[0] > 0.8);
        // A gap in the samples is a gap on the width, not a seam.
        let mut gapped = samples(10, 1_000_000);
        gapped.extend(samples(10, 1_000_100));
        let (_, positions) = timeline(&gapped, 200);
        assert!(positions[10] - positions[9] > 0.4);
        assert_eq!(timeline(&[], 60), (Vec::new(), Vec::new()));
    }

    #[test]
    fn the_timeline_is_thinned_to_three_hundred_points_with_the_newest_kept() {
        let history = samples(1_000, 1_000_000);
        let (values, positions) = timeline(&history, HistoryRange::ThreeHours.seconds());
        assert!(values.len() <= GRAPH_POINTS, "{}", values.len());
        assert!(values.len() > GRAPH_POINTS / 2);
        assert_eq!(*positions.last().unwrap(), 1.0);
        assert_eq!(*values.last().unwrap(), history.last().unwrap().value);
        // Values are clamped to the graph's scale.
        let wild = vec![Sample { at_unix: 0, value: -1.0 }, Sample { at_unix: 1, value: 7.0 }];
        assert_eq!(timeline(&wild, 10).0, vec![0.0, 1.0]);
        assert_eq!(timeline_span(&wild), 1);
        assert_eq!(timeline_span(&[]), 1);
        assert_eq!(timeline_span(&samples(30, 1_000_000)), 58);
    }

    #[test]
    fn a_remembered_timeline_keeps_a_day_and_forgets_the_rest() {
        let mut history = VecDeque::new();
        for n in 0..5 {
            remember(&mut history, Sample { at_unix: n * 3_600, value: 0.5 });
        }
        assert_eq!(history.len(), 5);
        // A sample a day and a second past the first pushes the first out
        // and keeps the rest.
        remember(&mut history, Sample { at_unix: 86_401, value: 0.5 });
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].at_unix, 3_600);
        assert_eq!(history.back().unwrap().at_unix, 86_401);
    }

    #[test]
    fn a_process_the_shell_has_no_icon_for_falls_back_to_the_neutral_mark() {
        let elements = layout(&CpuFlyout::new(full()));
        let marks: Vec<&Element> = elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Glyph { glyph, .. } if glyph == icons::PROCESS))
            .collect();
        assert_eq!(marks.len(), 5);
        let row = elements.iter().find(|e| matches!(e.kind, Kind::Row { .. })).unwrap();
        assert!((marks[0].rect.x - (row.rect.x + 6.0)).abs() < 1e-3);
        let name = elements.iter().find(|e| text_of(e) == Some("process0")).unwrap();
        assert!(name.rect.x >= marks[0].rect.right());
    }

    #[test]
    fn the_core_grid_takes_as_many_columns_as_the_card_fits_and_never_none() {
        let grid = CoreGrid::fit(8, COLUMN_W - 2.0 * CARD_PAD);
        assert_eq!(grid.columns, 4);
        assert_eq!(grid.rows, 2);
        assert!((grid.cell_w - 77.0).abs() < 1e-4);
        assert_eq!(grid.height(), 2.0 * CORE_CELL_H + CORE_GAP);
        // One core still gets a column's width, not the whole card.
        assert_eq!(CoreGrid::fit(1, 332.0).columns, 4);
        assert_eq!(CoreGrid::fit(0, 332.0).height(), 0.0);
        // A card too narrow for two columns has one, however narrow.
        let narrow = CoreGrid::fit(3, 30.0);
        assert_eq!(narrow.columns, 1);
        assert_eq!(narrow.rows, 3);
        assert_eq!(narrow.cell_w, 30.0);
        assert_eq!(grid.origin(5), (grid.cell_w + CORE_GAP, CORE_CELL_H + CORE_GAP));
    }

    #[test]
    fn a_fifth_core_starts_a_second_row_and_adds_one_cell_and_its_gap() {
        let mut four = CpuFlyout::new(full());
        four.snapshot.cores = cores(4, CoreKind::Unknown);
        let mut five = four.clone();
        five.snapshot.cores = cores(5, CoreKind::Unknown);
        let mut one = four.clone();
        one.snapshot.cores = cores(1, CoreKind::Unknown);
        let h4 = four.height(COLUMN_W, &measure);
        assert_eq!(one.height(COLUMN_W, &measure), h4);
        assert!((five.height(COLUMN_W, &measure) - h4 - (CORE_CELL_H + CORE_GAP)).abs() < 1e-3);
    }

    #[test]
    fn the_system_card_leads_with_the_load_average() {
        let elements = layout(&CpuFlyout::new(full()));
        let words: Vec<&str> = elements.iter().filter_map(text_of).collect();
        assert!(words.contains(&"Load average"));
        assert!(words.contains(&"1.24  \u{00B7}  0.98  \u{00B7}  0.75"), "{words:?}");
        // Above Uptime, as the Swift's card has it.
        let row = |wanted: &str| elements.iter().find(|e| text_of(e) == Some(wanted)).expect(wanted).rect.y;
        assert!(row("Load average") < row("Uptime"));
        // A machine that has not answered yet shows no row at all rather
        // than a dash where a number belongs.
        let mut without = CpuFlyout::new(full());
        without.snapshot.load_average = None;
        let quiet = layout(&without);
        assert!(!quiet.iter().filter_map(text_of).any(|word| word == "Load average"));
    }

    #[test]
    fn the_cards_come_in_the_macs_order_and_stack_without_overlapping() {
        let flyout = CpuFlyout::new(full());
        let elements = layout(&flyout);
        let cards = cards(&elements);
        assert_eq!(cards.len(), 4, "history, cores, system, top processes");
        for pair in cards.windows(2) {
            assert!(pair[1].y >= pair[0].bottom(), "{pair:?}");
        }
        let labels: Vec<&str> = elements
            .iter()
            .filter_map(text_of)
            .filter(|t| ["History", "Cores", "System", "Top processes"].contains(t))
            .collect();
        assert_eq!(labels, ["History", "Cores", "System", "Top processes"]);
        // The first card is the tinted one, as GlassCard(tint:) is.
        assert!(matches!(
            elements.iter().find(|e| matches!(e.kind, Kind::Card { .. })).unwrap().kind,
            Kind::Card { tint: Some(_) }
        ));
        assert!(cards.iter().all(|c| c.x == PANEL_PAD && (c.w - COLUMN_W).abs() < 1e-3));
        // The graph is the Swift's 84, plotted by time.
        let graph = elements.iter().find(|e| matches!(e.kind, Kind::Graph(_))).unwrap();
        assert_eq!(graph.rect.h, 84.0);
        assert!(matches!(&graph.kind, Kind::Graph(g) if g.values.len() == 30 && g.positions.is_some()));
    }

    #[test]
    fn the_height_is_what_the_builder_walked() {
        let flyout = CpuFlyout::new(full());
        let elements = layout(&flyout);
        let last = cards(&elements).last().unwrap().bottom();
        assert!((flyout.height(COLUMN_W, &measure) + PANEL_PAD - CARD_GAP - last).abs() < 1e-3);
    }

    #[test]
    fn without_a_sample_only_the_header_and_the_history_card_are_shown() {
        let flyout = CpuFlyout::new(CpuSnapshot::default());
        let elements = layout(&flyout);
        assert_eq!(cards(&elements).len(), 1);
        assert!(elements.iter().any(|e| text_of(e) == Some("Waiting for the first sample")));
        assert!(elements.iter().any(|e| text_of(e) == Some(DASH)));
        // No chips under an empty graph.
        assert!(!elements.iter().any(|e| matches!(&e.kind, Kind::Chip { text, .. } if text.starts_with("Idle"))));
    }

    #[test]
    fn the_processes_card_is_left_out_when_there_are_none_or_they_are_switched_off() {
        let mut flyout = CpuFlyout::new(full());
        flyout.snapshot.top.clear();
        assert_eq!(cards(&layout(&flyout)).len(), 3);
        // While the shares are being measured the card stays, with a
        // sentence where the rows will be, so that the panel does not grow a
        // card a second after it opened - and so that a list which is empty
        // because there is no interval yet does not read as an idle machine.
        flyout.snapshot.measuring = true;
        let elements = layout(&flyout);
        assert_eq!(cards(&elements).len(), 4);
        assert!(elements.iter().any(|e| matches!(&e.kind, Kind::Wrapped { text, .. } if text == MEASURING)));
        // Switched off it stays off, measuring or not: the card the user
        // asked not to see is not a place to put a status line.
        flyout.show_processes = false;
        assert_eq!(cards(&layout(&flyout)).len(), 3);
        let mut flyout = CpuFlyout::new(full());
        flyout.show_processes = false;
        assert_eq!(cards(&layout(&flyout)).len(), 3);
        // Five of the seven, with the end-task glyph on each.
        let flyout = CpuFlyout::new(full());
        let elements = layout(&flyout);
        let rows = elements.iter().filter(|e| matches!(e.kind, Kind::Row { .. })).count();
        assert_eq!(rows, 5);
        let glyphs = elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Glyph { glyph, .. } if glyph == glyph::CANCEL))
            .count();
        assert_eq!(glyphs, 5);
    }

    #[test]
    fn the_range_picker_answers_to_its_ids_and_a_new_range_asks_for_a_rebuild() {
        for range in HistoryRange::ALL {
            assert_eq!(HistoryRange::from_id(range.id()), Some(range));
        }
        assert_eq!(HistoryRange::from_id(Id::Custom(RANGE_ID + 5)), None);
        assert_eq!(HistoryRange::from_id(Id::Settings), None);
        let mut flyout = CpuFlyout::new(full());
        assert_eq!(flyout.activate(HistoryRange::FiveMinutes.id()), None);
        assert_eq!(flyout.activate(HistoryRange::OneMinute.id()), Some(CpuAction::Rebuild));
        assert_eq!(flyout.range, HistoryRange::OneMinute);
        // Five chips, laid left to right in the picker's order, all of them
        // controls, the chosen one in the accent.
        let elements = layout(&flyout);
        let picker: Vec<&Element> = elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Chip { .. }) && HistoryRange::from_id(e.id).is_some())
            .collect();
        assert_eq!(picker.len(), 5);
        assert!(picker.iter().all(|e| e.interactive));
        assert!(picker.windows(2).all(|p| p[1].rect.x > p[0].rect.right()));
        let accent = Accent::signature(ModuleId::Cpu).primary;
        let chosen: Vec<Id> = picker
            .iter()
            .filter(|e| matches!(e.kind, Kind::Chip { color, .. } if color == accent))
            .map(|e| e.id)
            .collect();
        assert_eq!(chosen, [HistoryRange::OneMinute.id()]);
        assert!((picker.last().unwrap().rect.right() - (PANEL_PAD + COLUMN_W - CARD_PAD)).abs() < 1e-3);
    }

    #[test]
    fn an_end_task_press_names_the_process_it_was_drawn_for_and_not_a_position() {
        let mut flyout = CpuFlyout::new(full());
        assert_eq!(
            flyout.activate(Id::Custom(END_TASK_ID | 1001)),
            Some(CpuAction::EndTask { pid: 1001, name: "process1".to_string() })
        );
        // The list re-sorts under the pointer between the draw and the press.
        // The press must still mean the process it was drawn for, so the same
        // id names the same process from its new place - and names nothing at
        // all once that process is off the list.
        let mut shuffled = full();
        shuffled.top.swap(0, 3);
        flyout.snapshot = shuffled;
        assert_eq!(
            flyout.activate(Id::Custom(END_TASK_ID | 1001)),
            Some(CpuAction::EndTask { pid: 1001, name: "process1".to_string() })
        );
        assert_eq!(flyout.activate(Id::Custom(END_TASK_ID | 9999)), None);
        assert_eq!(flyout.activate(Id::Custom(PROCESS_ID)), None);
        assert_eq!(flyout.activate(Id::None), None);
    }

    #[test]
    fn uptime_reads_like_the_mac() {
        assert_eq!(uptime(Some(3 * 86_400 + 4 * 3_600 + 12 * 60)), "3d 4h 12m");
        assert_eq!(uptime(Some(4 * 3_600 + 12 * 60 + 59)), "4h 12m");
        assert_eq!(uptime(Some(0)), "0h 0m");
        assert_eq!(uptime(None), "Unavailable");
    }

    #[test]
    fn the_subtitle_says_what_is_known_and_no_more() {
        let flyout = CpuFlyout::new(full());
        assert_eq!(flyout.subtitle(), "38% user  \u{00B7}  4% system  \u{00B7}  58% idle");
        let mut flyout = CpuFlyout::new(full());
        flyout.snapshot.user = None;
        assert_eq!(flyout.subtitle(), "42% busy  \u{00B7}  58% idle");
        assert_eq!(flyout.share_chips(Accent::signature(ModuleId::Cpu)).unwrap().len(), 2);
        assert_eq!(CpuFlyout::new(CpuSnapshot::default()).subtitle(), "Waiting for the first sample");
        assert_eq!(processes_text(&full()), "234  \u{00B7}  3456 threads");
        assert_eq!(processes_text(&CpuSnapshot::default()), DASH);
        assert_eq!(processes_text(&CpuSnapshot { threads: Some(9), ..Default::default() }), "9 threads");
    }

    #[test]
    fn hybrid_cores_are_labeled_p_and_e_and_plain_ones_c() {
        let p = CoreLoad { index: 0, kind: CoreKind::Performance, load: 0.5 };
        let e = CoreLoad { index: 7, kind: CoreKind::Efficiency, load: 0.5 };
        let c = CoreLoad { index: 3, kind: CoreKind::Unknown, load: 0.5 };
        assert_eq!(core_label(&p), "P1");
        assert_eq!(core_label(&e), "E8");
        assert_eq!(core_label(&c), "C4");
        let mut hybrid = cores(4, CoreKind::Performance);
        hybrid.extend(cores(4, CoreKind::Efficiency));
        assert_eq!(
            core_summary(&CpuSnapshot { cores: hybrid, ..Default::default() }),
            "4 performance \u{00B7} 4 efficiency"
        );
        assert_eq!(core_summary(&CpuSnapshot { cores: cores(8, CoreKind::Unknown), ..Default::default() }), "8 cores");
        assert_eq!(
            core_summary(&CpuSnapshot { logical_processors: Some(12), ..Default::default() }),
            "12 logical processors"
        );
        assert_eq!(core_summary(&CpuSnapshot::default()), "");
    }

    #[test]
    fn an_efficiency_core_takes_the_second_color_and_a_performance_core_the_first() {
        let mut flyout = CpuFlyout::new(full());
        flyout.snapshot.cores = vec![
            CoreLoad { index: 0, kind: CoreKind::Performance, load: 0.5 },
            CoreLoad { index: 1, kind: CoreKind::Efficiency, load: 0.5 },
        ];
        let elements = layout(&flyout);
        let accent = Accent::signature(ModuleId::Cpu);
        let bars: Vec<(Color, Color)> = elements
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Capsule { color, color2, glow: true, .. } => Some((color, color2)),
                _ => None,
            })
            .collect();
        assert_eq!(bars, [(accent.primary, accent.secondary), (accent.secondary, EFFICIENCY_TIP)]);
    }

    #[test]
    fn chips_lay_left_to_right_with_the_vocabularys_own_width_and_wrap_at_the_edge() {
        let mut b = Builder::new(0.0, 0.0, COLUMN_W, &measure);
        let placed = chip_left(&mut b, 40.0, 10.0, "User 38%", NEUTRAL, None);
        assert_eq!(placed.x, 40.0);
        assert_eq!(placed.y, 10.0);
        assert_eq!(placed.h, CHIP_H);
        assert!(placed.w > measure("User 38%", Style::CaptionStrong, 0.0).0);

        let mut b = Builder::new(0.0, 0.0, COLUMN_W, &measure);
        chip_row(&mut b, &[("User 38%".into(), NEUTRAL), ("System 4%".into(), NEUTRAL)]);
        let chips: Vec<Rect> = b.elements.iter().map(|e| e.rect).collect();
        assert_eq!(chips[0].x, CARD_PAD);
        assert!((chips[1].x - (chips[0].right() + CHIP_GAP)).abs() < 1e-3);
        assert_eq!(chips[0].y, chips[1].y);
        assert_eq!(b.y(), CHIP_H);

        // Ten wide chips cannot share one line of a 356 column.
        let mut b = Builder::new(0.0, 0.0, COLUMN_W, &measure);
        let many: Vec<(String, Color)> = (0..10).map(|n| (format!("Reading {n} 100%"), NEUTRAL)).collect();
        chip_row(&mut b, &many);
        let rows = b.elements.iter().map(|e| e.rect.y as i32).collect::<std::collections::BTreeSet<_>>().len();
        assert!(rows > 1);
        assert!(b.elements.iter().all(|e| e.rect.right() <= COLUMN_W - CARD_PAD + 1e-3));
        assert_eq!(b.y(), rows as f32 * CHIP_H + (rows as f32 - 1.0) * CHIP_GAP);
    }

    #[test]
    fn a_headline_percent_keeps_one_decimal_and_stays_inside_the_scale() {
        assert_eq!(percent1(0.4234), "42.3%");
        assert_eq!(percent1(1.5), "100.0%");
        assert_eq!(percent1(-0.1), "0.0%");
    }

    #[test]
    fn the_content_follows_its_feed_and_hands_presses_on() {
        let slot = Arc::new(Mutex::new(CpuSnapshot::default()));
        let ended = Arc::new(Mutex::new(None));
        let mut content = CpuContent::new(Arc::clone(&slot)).on_end_task({
            let ended = Arc::clone(&ended);
            move |pid, name| *ended.lock().unwrap() = Some((pid, name.to_string()))
        });
        let palette = Palette::resolve(Theme::resolve(false, DEFAULT_ACCENT));
        let cx = Context {
            width: PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Cpu),
            measure: &measure,
            hover: None,
            now_unix: 0,
        };
        assert_eq!(content.module(), ModuleId::Cpu);
        // Nothing has been published, so nothing to lay out again.
        assert!(!content.tick());
        *slot.lock().unwrap() = full();
        assert!(content.tick());
        let page = content.build(&cx);
        assert!(!content.tick());
        assert!((page.height - (CpuFlyout::new(full()).height(PANEL_W - 2.0 * PANEL_PAD, &measure) + 2.0 * PANEL_PAD)).abs() < 1e-3);
        assert_eq!(cards(&page.elements).len(), 4);
        // A range press changes the shape; the end-task glyph reaches the handler.
        assert_eq!(content.activate(HistoryRange::ThreeHours.id()), Response::Relayout);
        assert_eq!(content.activate(HistoryRange::ThreeHours.id()), Response::None);
        assert_eq!(content.activate(Id::Custom(END_TASK_ID | 1002)), Response::None);
        assert_eq!(*ended.lock().unwrap(), Some((1002, "process2".to_string())));
        assert_eq!(content.activate(Id::Settings), Response::None);
    }

    /// Paints the panel for a person to look at; see `flyout::render`.
    #[test]
    #[ignore]
    fn render_the_panel_to_bitmaps() {
        let slot = Arc::new(Mutex::new(full()));
        let mut content = CpuContent::new(Arc::clone(&slot));
        content.tick();
        for light in [false, true] {
            crate::flyout::render::to_bitmap(&mut content, "cpu", light, 0);
        }
    }
}
