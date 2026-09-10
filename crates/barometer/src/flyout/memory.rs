// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The memory flyout, ported from MemoryDropdownView in DropdownViews.swift.
//
// What the Mac shows, in its order: a hero header with "used of total" and
// the used share; a Breakdown card with a segmented bar and four chips; a
// Pressure card with a state chip, a graph and two figures; and the
// processes holding the most.
//
// Two departures, both recorded in docs/ui-design.md section 13. The
// breakdown is in Task Manager's terms - In use, Modified, Standby, Free -
// rather than App, Wired, Compressed and Cached, which are Mach's, and the
// trailing caption says "available" rather than "free" because that is
// Windows' word for standby and free together and the figure
// GlobalMemoryStatusEx actually hands back. And memory pressure is a Mach
// figure with no Windows equivalent; the trouble Windows gets into is commit
// charge reaching the commit limit, which is when allocations start to fail,
// so the second card is Commit: the same chip, graph and rows, over that
// ratio, with the paged and non-paged pools where the Swift has swap.

use std::sync::{Arc, Mutex};

use barometer_core::format;
use barometer_core::ModuleId;

use super::cpu::{
    chip_left, history_graph, latest, process_name_and_share, timeline_span, Sample, BAR_H, DASH, NEUTRAL,
    PROCESS_ROW_H,
};
use super::ui::{Accent, Builder, Id, Ink, Kind, Measure, Style};
use super::{Content, Context, Page, Response};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::model::module_glyph;
use crate::settings_ui::theme::{self, Color};

/// The design's warning amber and critical red (docs/ui-design.md, 2.6),
/// for a chip's dot and a bar's segment: never words, so the dark and light
/// variants the theme keeps for text are not wanted here.
const AMBER: Color = Color(0xF59E0B);
const RED: Color = Color(0xEF4444);

/// A process and what it holds, from MemoryProcessSample.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    /// The working set, in bytes: Task Manager's Memory column.
    pub working_set: u64,
}

/// Everything the panel says, from MemorySample.
///
/// Bytes throughout. Total, in use and available come from the strip's own
/// module; the commit figures, the pools and the process list from
/// barometer-core's sys::processes, and the standby/modified/free split from
/// its SystemMemoryListInformation query. The bar falls back to in use
/// against available while that split is missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemorySnapshot {
    /// From MemoryModule::used_and_total, or `from_module`.
    pub total: Option<u64>,
    pub in_use: Option<u64>,
    /// Standby and free together, which is what Windows means by available.
    pub available: Option<u64>,
    pub modified: Option<u64>,
    pub standby: Option<u64>,
    pub free: Option<u64>,
    /// From processes::SystemSummary.
    pub committed: Option<u64>,
    pub commit_limit: Option<u64>,
    pub paged_pool: Option<u64>,
    pub nonpaged_pool: Option<u64>,
    /// Page file in use and its size, where the Swift's swap row is.
    pub page_file_used: Option<u64>,
    pub page_file_total: Option<u64>,
    /// The processes holding the most, largest first: processes::Sample::
    /// top_by_memory, mapped.
    pub top: Vec<ProcessMemory>,
    /// Commit charge as a fraction of the limit at each sample kept, oldest
    /// first, kept with cpu::remember. The Swift's memory graph has no range
    /// picker and plots the whole of what the store holds; so does this.
    pub history: Vec<Sample>,
}

impl MemorySnapshot {
    /// Splits the used figure by the kernel's page lists.
    ///
    /// Modified pages are inside "used" - they are not available - but the
    /// bar draws them in their own color, so in use is what is left of used
    /// once they are taken out. Free stays the kernel's free, not the
    /// module's available, which is free and standby together.
    pub fn with_lists(&mut self, lists: barometer_core::sys::processes::MemoryLists) {
        if let Some(used) = self.in_use {
            self.in_use = Some(used.saturating_sub(lists.modified));
        }
        self.modified = Some(lists.modified);
        self.standby = Some(lists.standby);
        self.free = Some(lists.free);
    }

    /// Commit charge against the limit, zero to one.
    pub fn commit_fraction(&self) -> Option<f32> {
        match (self.committed, self.commit_limit) {
            (Some(committed), Some(limit)) if limit > 0 => Some(committed as f32 / limit as f32),
            _ => None,
        }
    }
}

// MARK: - Breakdown

/// A share of the installed memory, in the bar's drawing order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    InUse,
    Modified,
    Standby,
    Free,
    /// Standby and free together, while Windows has not been asked to tell
    /// them apart.
    Available,
}

impl Segment {
    pub fn label(self) -> &'static str {
        match self {
            Segment::InUse => "In use",
            Segment::Modified => "Modified",
            Segment::Standby => "Standby",
            Segment::Free => "Free",
            Segment::Available => "Available",
        }
    }

    /// In use takes the accent and standby its second color, as app and
    /// wired memory do in the Swift; modified pages are waiting to be
    /// written and take the amber the Swift gives compressed memory; what
    /// is free is nobody's.
    pub fn color(self, accent: Accent) -> Color {
        match self {
            Segment::InUse => accent.primary,
            Segment::Modified => AMBER,
            Segment::Standby => accent.secondary,
            Segment::Free | Segment::Available => NEUTRAL,
        }
    }
}

/// The bar's segments and the scale they are drawn against, from
/// MemoryBreakdownLayout in the Swift.
///
/// The Swift draws five segments side by side with a gap between each and
/// clips the row to one capsule. The vocabulary has no clip, so the bar is
/// drawn as capsules stacked from the widest down: each covers its own
/// share and everything before it, and the one drawn last shows only its
/// own. The free share is not drawn at all; the track it would cover is
/// already the Swift's "primary at 8%".
#[derive(Clone, Debug, PartialEq)]
pub struct Breakdown {
    /// The shares that are drawn, in drawing order.
    pub used: Vec<(Segment, u64)>,
    /// The share that is the track.
    pub free: (Segment, u64),
    /// What a whole bar's width stands for.
    pub scale: u64,
}

impl Breakdown {
    pub fn new(s: &MemorySnapshot) -> Option<Breakdown> {
        let total = s.total?;
        let in_use = s.in_use?;
        let (used, free) = match (s.modified, s.standby, s.free) {
            (Some(modified), Some(standby), Some(free)) => (
                vec![(Segment::InUse, in_use), (Segment::Modified, modified), (Segment::Standby, standby)],
                (Segment::Free, free),
            ),
            _ => (
                vec![(Segment::InUse, in_use)],
                (Segment::Available, s.available.unwrap_or_else(|| total.saturating_sub(in_use))),
            ),
        };
        let counted = used.iter().map(|(_, bytes)| *bytes).sum::<u64>().saturating_add(free.1);
        // A reading that ever exceeds the installed memory scales down
        // rather than overflowing the track.
        Some(Breakdown { used, free, scale: total.max(counted) })
    }

    /// Each drawn share with the fraction of the bar it and everything
    /// before it cover, in drawing order.
    pub fn cumulative(&self) -> Vec<(Segment, f32)> {
        let mut sum = 0u64;
        self.used
            .iter()
            .map(|(segment, bytes)| {
                sum = sum.saturating_add(*bytes);
                (*segment, self.fraction(sum))
            })
            .collect()
    }

    fn fraction(&self, bytes: u64) -> f32 {
        if self.scale == 0 {
            return 0.0;
        }
        (bytes as f32 / self.scale as f32).clamp(0.0, 1.0)
    }

    /// The chips under the bar: every share with its figure.
    pub fn chips(&self, accent: Accent) -> Vec<(String, Color)> {
        self.used
            .iter()
            .chain(std::iter::once(&self.free))
            .map(|(segment, bytes)| (format!("{} {}", segment.label(), format::bytes(*bytes)), segment.color(accent)))
            .collect()
    }
}

/// The gap between chips in the grid under the bar, and a chip's height.
const BREAKDOWN_CHIP_GAP: f32 = 6.0;
const CHIP_H: f32 = 20.0;

// MARK: - Commit

/// How near the commit charge is to its limit, standing in for
/// MemoryPressureLevel.
///
/// The Swift's bands are 60 and 80 on a pressure figure that Mach keeps
/// low on a healthy machine. Commit is not like that: a working day sits
/// at half the limit or more, so the bands here start later. Past 70% the
/// page file is doing real work; past 90% the next large allocation may be
/// the one that fails.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CommitLevel {
    Normal,
    High,
    Critical,
    Unknown,
}

pub const HIGH_COMMIT: f32 = 0.70;
pub const CRITICAL_COMMIT: f32 = 0.90;

impl CommitLevel {
    pub fn for_fraction(fraction: Option<f32>) -> CommitLevel {
        match fraction {
            None => CommitLevel::Unknown,
            Some(f) if f >= CRITICAL_COMMIT => CommitLevel::Critical,
            Some(f) if f >= HIGH_COMMIT => CommitLevel::High,
            Some(_) => CommitLevel::Normal,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CommitLevel::Normal => "Normal",
            CommitLevel::High => "High",
            CommitLevel::Critical => "Critical",
            CommitLevel::Unknown => "Unknown",
        }
    }

    /// The chip's color and the graph's line, from pressureColor: the
    /// accent while all is well, then the warning and critical colors.
    pub fn color(self, accent: Accent) -> Color {
        match self {
            CommitLevel::Normal => accent.primary,
            CommitLevel::High => AMBER,
            CommitLevel::Critical => RED,
            CommitLevel::Unknown => NEUTRAL,
        }
    }
}

// MARK: - The panel

/// A process row, by its index.
const PROCESS_ID: u32 = 100;

/// The memory flyout's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryFlyout {
    pub snapshot: MemorySnapshot,
    /// ModuleSettings.showsProcesses and processCount on the Mac, at the
    /// Mac's defaults until the Windows settings have the switch.
    pub show_processes: bool,
    pub process_count: usize,
}

impl MemoryFlyout {
    pub fn new(snapshot: MemorySnapshot) -> MemoryFlyout {
        MemoryFlyout { snapshot, show_processes: true, process_count: 5 }
    }

    pub fn title(&self) -> &'static str {
        "Memory"
    }

    /// How tall the content is at a width, in DIPs: what the builder
    /// walked, the gap under the last card included, as Page::finish
    /// counts it.
    pub fn height(&self, width: f32, measure: Measure<'_>) -> f32 {
        let mut b = Builder::new(0.0, 0.0, width, measure);
        self.build(&mut b);
        b.y()
    }

    /// A press on one of the panel's controls: true when the panel changed
    /// and wants laying out again. The rows answer to ids for the hover
    /// wash, and nothing here changes on a press.
    pub fn activate(&mut self, _id: Id) -> bool {
        false
    }

    fn shown_processes(&self) -> impl Iterator<Item = &ProcessMemory> {
        self.snapshot.top.iter().take(if self.show_processes { self.process_count } else { 0 })
    }

    /// Lays the whole panel out, in the Swift's order. Without a sample
    /// there is the header and nothing else, as in the Swift.
    pub fn build(&self, b: &mut Builder<'_>) {
        let s = &self.snapshot;
        let accent = Accent::signature(ModuleId::Memory);
        let value = self.used_percent();
        let subtitle = self.subtitle();
        b.hero_header(
            (theme::module_color(ModuleId::Memory), module_glyph(ModuleId::Memory)),
            self.title(),
            Some(&subtitle),
            Some(value.as_deref().unwrap_or(DASH)),
            accent,
        );

        let Some(breakdown) = Breakdown::new(s) else { return };

        let card = b.card_begin(Some(accent.primary));
        let trailing = b.section_label("Breakdown");
        if let Some(available) = s.available {
            b.text(
                trailing,
                &format!("{} available", format::bytes(available)),
                Style::Caption,
                Ink::Secondary,
                Align::Right,
            );
        }
        breakdown_bar(b, &breakdown, accent);
        b.gap(9.0);
        chip_grid(b, &breakdown.chips(accent));
        b.card_end(card);

        let level = CommitLevel::for_fraction(s.commit_fraction());
        let card = b.card_begin(None);
        let trailing = b.section_label("Commit");
        b.chip_at(trailing, level.name(), level.color(accent), None);
        let graph_accent = Accent { primary: level.color(accent), secondary: accent.secondary };
        b.graph(72.0, history_graph(&s.history, timeline_span(&s.history), graph_accent));
        b.gap(8.0);
        b.metric_row(None, "Committed", &of(s.committed, s.commit_limit), Ink::Secondary);
        if s.page_file_total.is_some() {
            b.metric_row(None, "Page file", &of(s.page_file_used, s.page_file_total), Ink::Secondary);
        }
        b.metric_row(None, "Paged pool", &bytes(s.paged_pool), Ink::Secondary);
        b.metric_row(None, "Non-paged pool", &bytes(s.nonpaged_pool), Ink::Secondary);
        b.card_end(card);

        if self.shown_processes().next().is_some() {
            let card = b.card_begin(None);
            b.section_label("Top processes");
            let total = s.total.unwrap_or(0);
            for (index, process) in self.shown_processes().enumerate() {
                process_row(b, index as u32, process, total, accent);
            }
            b.card_end(card);
        }
    }

    /// "12.4 GB of 32.0 GB used", from usedText.
    fn subtitle(&self) -> String {
        let s = &self.snapshot;
        match (s.in_use, s.total) {
            (Some(used), Some(total)) => format!("{} of {} used", format::bytes(used), format::bytes(total)),
            _ => "Waiting for the first sample".to_string(),
        }
    }

    /// The used share as a whole percent, from usedPercent, which is the
    /// one headline the Swift rounds to a whole number.
    fn used_percent(&self) -> Option<String> {
        let s = &self.snapshot;
        match (s.in_use, s.total) {
            (Some(used), Some(total)) if total > 0 => Some(format::percent(used as f32 / total as f32)),
            _ => None,
        }
    }
}

/// The memory content: the panel's end of the feed, and the layout.
pub struct MemoryContent {
    slot: Arc<Mutex<MemorySnapshot>>,
    flyout: MemoryFlyout,
}

impl MemoryContent {
    /// Content reading from a feed's slot, as `Flyout::feed` hands it out.
    pub fn new(slot: Arc<Mutex<MemorySnapshot>>) -> MemoryContent {
        MemoryContent { slot, flyout: MemoryFlyout::new(MemorySnapshot::default()) }
    }
}

impl Content for MemoryContent {
    fn module(&self) -> ModuleId {
        ModuleId::Memory
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
        if self.flyout.activate(id) {
            Response::Relayout
        } else {
            Response::None
        }
    }

    fn tick(&mut self) -> bool {
        self.slot.lock().map(|slot| *slot != self.flyout.snapshot).unwrap_or(false)
    }
}

/// The stacked bar: the track, then each cumulative share from the widest
/// down, so that every segment shows exactly its own width.
fn breakdown_bar(b: &mut Builder<'_>, breakdown: &Breakdown, accent: Accent) {
    let bar = Rect::new(b.inner_x(), b.y(), b.inner_w(), BAR_H);
    b.passive(bar, Kind::Track);
    for (segment, fraction) in breakdown.cumulative().into_iter().rev() {
        let color = segment.color(accent);
        b.passive(bar, Kind::Capsule { fraction, color, color2: color, glow: false });
    }
    b.advance(BAR_H);
}

/// Chips in two columns, from the Swift's LazyVGrid of two flexible
/// columns: each chip at the left of its cell, rows six apart.
fn chip_grid(b: &mut Builder<'_>, chips: &[(String, Color)]) {
    let column_w = (b.inner_w() - BREAKDOWN_CHIP_GAP) / 2.0;
    let (x0, y0) = (b.inner_x(), b.y());
    let rows = chips.len().div_ceil(2);
    for (index, (text, color)) in chips.iter().enumerate() {
        let x = x0 + (index % 2) as f32 * (column_w + BREAKDOWN_CHIP_GAP);
        let y = y0 + (index / 2) as f32 * (CHIP_H + BREAKDOWN_CHIP_GAP);
        chip_left(b, x, y, text, *color, None);
    }
    if rows > 0 {
        b.advance(rows as f32 * CHIP_H + (rows as f32 - 1.0) * BREAKDOWN_CHIP_GAP);
    }
}

/// "1.20 GB of 4.00 GB", from the swap row, or a dash for either half.
fn of(used: Option<u64>, total: Option<u64>) -> String {
    match (used, total) {
        (Some(used), Some(total)) => format!("{} of {}", format::bytes(used), format::bytes(total)),
        _ => DASH.to_string(),
    }
}

fn bytes(value: Option<u64>) -> String {
    value.map(format::bytes).unwrap_or_else(|| DASH.to_string())
}

/// One process: its name over a bar of its share of the installed memory,
/// and what it holds, from ProcessRow, in the row the CPU panel's processes
/// use. No end-task glyph: the Swift's memory rows have none.
fn process_row(b: &mut Builder<'_>, index: u32, process: &ProcessMemory, total: u64, accent: Accent) {
    let row = b.row_begin(Id::Custom(PROCESS_ID + index), PROCESS_ROW_H, false);
    let figure = format::bytes(process.working_set);
    let figure_w = b.text_width(&figure, Style::Body);
    let figure_x = row.right() - 6.0 - figure_w;
    let icon = super::appicon::icon_for(process.pid);
    process_name_and_share(b, row, &process.name, share(process.working_set, total), figure_x - 10.0, accent, icon);
    b.text(Rect::new(figure_x, row.y, figure_w, row.h), &figure, Style::Body, Ink::Secondary, Align::Right);
    b.advance(PROCESS_ROW_H);
}

/// A process's share of the installed memory, and nothing to divide by
/// when the total is unknown.
pub fn share(working_set: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (working_set as f32 / total as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Element, Palette, CARD_GAP, CARD_PAD, PANEL_PAD, PANEL_W};
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

    /// Seven DIPs a character at body size, scaled by the style, as the
    /// vocabulary's own tests measure.
    fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    const COLUMN_W: f32 = PANEL_W - 2.0 * PANEL_PAD;
    const GB: u64 = 1024 * 1024 * 1024;

    /// A sample every two seconds for `count` samples, ending at `end`.
    fn samples(count: usize, end: i64) -> Vec<Sample> {
        (0..count)
            .map(|n| Sample { at_unix: end - 2 * (count as i64 - 1 - n as i64), value: 0.4 + (n % 3) as f32 / 20.0 })
            .collect()
    }

    /// The Swift test's machine: 16 GB, with the file cache inside "free".
    fn full() -> MemorySnapshot {
        MemorySnapshot {
            total: Some(16 * GB),
            in_use: Some(8 * GB),
            available: Some(8 * GB),
            modified: Some(GB),
            standby: Some(3 * GB),
            free: Some(4 * GB),
            committed: Some(12 * GB),
            commit_limit: Some(24 * GB),
            paged_pool: Some(GB / 2),
            nonpaged_pool: Some(GB / 4),
            page_file_used: Some(GB + GB / 5),
            page_file_total: Some(4 * GB),
            top: (0..7)
                .map(|n| ProcessMemory { pid: 1000 + n, name: format!("process{n}"), working_set: (7 - n as u64) * GB / 8 })
                .collect(),
            history: samples(30, 1_000_000),
        }
    }

    fn layout(flyout: &MemoryFlyout) -> Vec<Element> {
        let mut b = Builder::new(PANEL_PAD, PANEL_PAD, COLUMN_W, &measure);
        flyout.build(&mut b);
        b.elements
    }

    fn cards(elements: &[Element]) -> Vec<Rect> {
        elements.iter().filter(|e| matches!(e.kind, Kind::Card { .. })).map(|e| e.rect).collect()
    }

    fn texts<'a>(elements: &'a [Element]) -> Vec<&'a str> {
        elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Text { text, .. } | Kind::Chip { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_breakdown_shares_and_the_track_come_to_exactly_the_bar() {
        // The Swift's test: with the cache inside free, the shares must sum
        // to the installed memory and not a fifth past it.
        let breakdown = Breakdown::new(&full()).unwrap();
        assert_eq!(breakdown.scale, 16 * GB);
        let cumulative = breakdown.cumulative();
        assert_eq!(cumulative.len(), 3);
        assert!((cumulative[0].1 - 0.5).abs() < 1e-6, "in use");
        assert!((cumulative[1].1 - 0.5625).abs() < 1e-6, "plus modified");
        assert!((cumulative[2].1 - 0.75).abs() < 1e-6, "plus standby");
        assert_eq!(breakdown.free, (Segment::Free, 4 * GB));
        // What the track shows past the last share is the free quarter.
        assert!((1.0 - cumulative[2].1 - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_reading_past_the_installed_memory_scales_down_rather_than_overflowing() {
        let mut s = full();
        s.standby = Some(9 * GB);
        let breakdown = Breakdown::new(&s).unwrap();
        assert_eq!(breakdown.scale, 22 * GB);
        let last = breakdown.cumulative().last().unwrap().1;
        assert!(last <= 1.0);
        assert!((last - 18.0 / 22.0).abs() < 1e-6);
    }

    #[test]
    fn without_the_split_the_bar_is_in_use_and_available() {
        let s = MemorySnapshot { total: Some(16 * GB), in_use: Some(6 * GB), available: Some(10 * GB), ..Default::default() };
        let breakdown = Breakdown::new(&s).unwrap();
        assert_eq!(breakdown.used, vec![(Segment::InUse, 6 * GB)]);
        assert_eq!(breakdown.free, (Segment::Available, 10 * GB));
        let accent = Accent::signature(ModuleId::Memory);
        let chips = breakdown.chips(accent);
        assert_eq!(chips.len(), 2);
        assert_eq!(chips[0], ("In use 6.00 GB".to_string(), accent.primary));
        assert_eq!(chips[1], ("Available 10.0 GB".to_string(), NEUTRAL));
        // Available is worked out when the module did not say.
        let s = MemorySnapshot { total: Some(16 * GB), in_use: Some(6 * GB), ..Default::default() };
        assert_eq!(Breakdown::new(&s).unwrap().free.1, 10 * GB);
        assert_eq!(Breakdown::new(&MemorySnapshot::default()), None);
    }

    #[test]
    fn the_page_lists_take_modified_pages_out_of_in_use() {
        let mut s = MemorySnapshot { total: Some(16 * GB), in_use: Some(9 * GB), available: Some(7 * GB), ..Default::default() };
        s.with_lists(barometer_core::sys::processes::MemoryLists { modified: GB, standby: 5 * GB, free: 2 * GB });
        assert_eq!(s.in_use, Some(8 * GB));
        let breakdown = Breakdown::new(&s).unwrap();
        assert_eq!(breakdown.used.len(), 3, "in use, modified, standby");
        assert_eq!(breakdown.free, (Segment::Free, 2 * GB));
        assert_eq!(breakdown.scale, 16 * GB);
    }

    #[test]
    fn stacked_capsules_are_drawn_widest_first_so_each_shows_its_own_share() {
        let elements = layout(&MemoryFlyout::new(full()));
        let breakdown_y = elements.iter().find(|e| matches!(e.kind, Kind::Track)).unwrap().rect.y;
        let bar: Vec<(f32, Color)> = elements
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Capsule { fraction, color, glow: false, .. } if e.rect.y == breakdown_y => Some((fraction, color)),
                _ => None,
            })
            .collect();
        let accent = Accent::signature(ModuleId::Memory);
        assert_eq!(bar.len(), 3);
        assert!(bar.windows(2).all(|p| p[0].0 > p[1].0), "{bar:?}");
        assert_eq!(bar[0].1, accent.secondary, "standby is the widest and goes down first");
        assert_eq!(bar[1].1, AMBER);
        assert_eq!(bar[2].1, accent.primary, "in use is drawn last and on top");
        // All on one track across the card, at the height every bar shares.
        let track = elements.iter().find(|e| matches!(e.kind, Kind::Track)).unwrap();
        assert_eq!(track.rect.h, BAR_H);
        assert!((track.rect.w - (COLUMN_W - 2.0 * CARD_PAD)).abs() < 1e-3);
    }

    #[test]
    fn the_chips_sit_in_two_columns_and_wrap_to_two_rows() {
        let elements = layout(&MemoryFlyout::new(full()));
        let chips: Vec<&Element> = elements
            .iter()
            .filter(|e| matches!(&e.kind, Kind::Chip { text, .. } if text.contains(" GB")))
            .collect();
        assert_eq!(chips.len(), 4);
        assert_eq!(chips[0].rect.x, chips[2].rect.x);
        assert_eq!(chips[1].rect.x, chips[3].rect.x);
        assert!(chips[1].rect.x > chips[0].rect.right());
        assert_eq!(chips[0].rect.y, chips[1].rect.y);
        assert!((chips[2].rect.y - chips[0].rect.y - (CHIP_H + BREAKDOWN_CHIP_GAP)).abs() < 1e-3);
        let column_w = (COLUMN_W - 2.0 * CARD_PAD - BREAKDOWN_CHIP_GAP) / 2.0;
        assert!((chips[1].rect.x - chips[0].rect.x - column_w - BREAKDOWN_CHIP_GAP).abs() < 1e-3);
    }

    #[test]
    fn commit_levels_follow_the_thresholds_and_color_the_chip_and_the_graph() {
        assert_eq!(CommitLevel::for_fraction(None), CommitLevel::Unknown);
        assert_eq!(CommitLevel::for_fraction(Some(0.5)), CommitLevel::Normal);
        assert_eq!(CommitLevel::for_fraction(Some(HIGH_COMMIT)), CommitLevel::High);
        assert_eq!(CommitLevel::for_fraction(Some(0.89)), CommitLevel::High);
        assert_eq!(CommitLevel::for_fraction(Some(CRITICAL_COMMIT)), CommitLevel::Critical);
        assert_eq!(full().commit_fraction(), Some(0.5));
        assert_eq!(MemorySnapshot { commit_limit: Some(0), committed: Some(1), ..Default::default() }.commit_fraction(), None);

        let mut s = full();
        s.committed = Some(23 * GB);
        let elements = layout(&MemoryFlyout::new(s));
        let chip = elements.iter().find(|e| matches!(&e.kind, Kind::Chip { text, .. } if text == "Critical")).unwrap();
        assert!(matches!(chip.kind, Kind::Chip { color, .. } if color == RED));
        let graph = elements.iter().find(|e| matches!(e.kind, Kind::Graph(_))).unwrap();
        assert!(matches!(&graph.kind, Kind::Graph(g) if g.color == RED && g.values.len() == 30));
        // The whole history, first sample at the left edge and last at the right.
        assert!(matches!(&graph.kind, Kind::Graph(g) if g.positions.as_ref().unwrap()[0] == 0.0));
        assert!(matches!(&graph.kind, Kind::Graph(g) if *g.positions.as_ref().unwrap().last().unwrap() == 1.0));
        assert_eq!(graph.rect.h, 72.0);
    }

    #[test]
    fn the_hero_says_used_of_total_as_a_whole_percent() {
        let flyout = MemoryFlyout::new(full());
        assert_eq!(flyout.subtitle(), "8.00 GB of 16.0 GB used");
        assert_eq!(flyout.used_percent().as_deref(), Some("50%"));
        let elements = layout(&flyout);
        let words = texts(&elements);
        assert!(words.contains(&"8.00 GB available"));
        assert!(words.contains(&"12.0 GB of 24.0 GB"));
        assert!(words.contains(&"512 MB") && words.contains(&"256 MB"));
        let empty = MemoryFlyout::new(MemorySnapshot::default());
        assert_eq!(empty.subtitle(), "Waiting for the first sample");
        assert_eq!(empty.used_percent(), None);
        assert_eq!(of(None, Some(1)), DASH);
    }

    #[test]
    fn without_a_sample_only_the_header_is_shown() {
        let elements = layout(&MemoryFlyout::new(MemorySnapshot::default()));
        assert!(cards(&elements).is_empty());
        assert!(texts(&elements).contains(&DASH));
        assert!(elements.iter().any(|e| matches!(e.kind, Kind::Tile { .. })));
    }

    #[test]
    fn the_cards_come_in_the_macs_order_and_stack_without_overlapping() {
        let elements = layout(&MemoryFlyout::new(full()));
        let cards = cards(&elements);
        assert_eq!(cards.len(), 3, "breakdown, commit, top processes");
        for pair in cards.windows(2) {
            assert!(pair[1].y >= pair[0].bottom(), "{pair:?}");
        }
        let labels: Vec<&str> = texts(&elements).into_iter().filter(|t| ["Breakdown", "Commit", "Top processes"].contains(t)).collect();
        assert_eq!(labels, ["Breakdown", "Commit", "Top processes"]);
        assert!(matches!(
            elements.iter().find(|e| matches!(e.kind, Kind::Card { .. })).unwrap().kind,
            Kind::Card { tint: Some(_) }
        ));
    }

    #[test]
    fn the_process_rows_show_five_with_their_share_of_the_installed_memory() {
        let flyout = MemoryFlyout::new(full());
        let elements = layout(&flyout);
        let rows = elements.iter().filter(|e| matches!(e.kind, Kind::Row { .. })).count();
        assert_eq!(rows, 5);
        let rows: Vec<Rect> = elements.iter().filter(|e| matches!(e.kind, Kind::Row { .. })).map(|e| e.rect).collect();
        assert!(rows.iter().all(|r| r.h == PROCESS_ROW_H));
        let shares: Vec<f32> = elements
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Capsule { fraction, glow: false, .. } if rows.iter().any(|r| r.contains(e.rect.x, e.rect.y)) => Some(fraction),
                _ => None,
            })
            .collect();
        // process0 holds 7/8 GB of 16 GB.
        assert!((shares[0] - 7.0 / 128.0).abs() < 1e-6);
        assert!(shares.windows(2).all(|p| p[0] > p[1]));
        assert_eq!(share(GB, 0), 0.0);
        assert_eq!(share(2 * GB, GB), 1.0);
        let mut off = flyout.clone();
        off.show_processes = false;
        assert_eq!(cards(&layout(&off)).len(), 2);
        assert!(!off.activate(Id::Custom(PROCESS_ID)));
    }

    #[test]
    fn the_height_is_what_the_builder_walked() {
        let flyout = MemoryFlyout::new(full());
        let elements = layout(&flyout);
        let last = cards(&elements).last().unwrap().bottom();
        assert!((flyout.height(COLUMN_W, &measure) + PANEL_PAD - CARD_GAP - last).abs() < 1e-3);
        // The header alone, when there is no sample: the hero's 40 and the
        // gap under it.
        let empty = MemoryFlyout::new(MemorySnapshot::default());
        assert_eq!(empty.height(COLUMN_W, &measure), 40.0 + CARD_GAP);
    }

    #[test]
    fn the_content_follows_its_feed_and_nothing_in_it_changes_shape_on_a_press() {
        let slot = Arc::new(Mutex::new(MemorySnapshot::default()));
        let mut content = MemoryContent::new(Arc::clone(&slot));
        let palette = Palette::resolve(Theme::resolve(false, DEFAULT_ACCENT));
        let cx = Context {
            width: PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Memory),
            measure: &measure,
            hover: None,
            now_unix: 0,
        };
        assert_eq!(content.module(), ModuleId::Memory);
        assert!(!content.tick());
        let page = content.build(&cx);
        assert!(cards(&page.elements).is_empty());
        *slot.lock().unwrap() = full();
        assert!(content.tick());
        let page = content.build(&cx);
        assert!(!content.tick());
        assert_eq!(cards(&page.elements).len(), 3);
        assert!((page.height - (MemoryFlyout::new(full()).height(COLUMN_W, &measure) + 2.0 * PANEL_PAD)).abs() < 1e-3);
        assert_eq!(content.activate(Id::Custom(PROCESS_ID)), Response::None);
    }

    /// Paints the panel for a person to look at; see `flyout::render`.
    #[test]
    #[ignore]
    fn render_the_panel_to_bitmaps() {
        let slot = Arc::new(Mutex::new(full()));
        let mut content = MemoryContent::new(Arc::clone(&slot));
        content.tick();
        for light in [false, true] {
            crate::flyout::render::to_bitmap(&mut content, "memory", light, 0);
        }
    }
}
