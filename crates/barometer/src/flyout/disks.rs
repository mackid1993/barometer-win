// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The Disks flyout, ported from DiskDropdownView.swift.
//
// Read and write at the top, with the last few minutes of each mirrored
// about a centerline; then every mounted volume with a capacity bar; then
// each physical disk with rates of its own. The order is the Mac's and so is
// what every card says. The strip's DiskModule reads the machine's total read
// and write rate; the volumes and the physical disks are enumerated beside it
// and handed in, and each card draws a calm line for as long as its list is
// empty rather than an error - a machine can genuinely have nothing to show.
//
// Nothing here draws. `build` lays elements out in DIPs through the shared
// Builder and the chrome paints them, which is what lets the tests hold the
// arithmetic to the Swift's proportions without a window.

use std::sync::{Arc, Mutex};

use barometer_core::format;
use barometer_core::ModuleId;

use super::cpu::latest;
use super::ui::{Accent, Builder, Graph, Ink, Kind, Measure, Style, TILE_GAP, TILE_H};
use super::{Content, Context, Page};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::model::module_glyph;
use crate::settings_ui::theme::{glyph, module_color, Color};

/// How many samples the graph keeps: the Mac's `history.recent(300)`.
pub const HISTORY: usize = 300;

/// The mirrored graph's plate, from DiskHistoryGraph's frame.
pub const GRAPH_H: f32 = 90.0;

/// A volume row on its inset plate: the name line, the mount line, the bar
/// with its breathing room, and the two captions under it.
pub const VOLUME_ROW_H: f32 = 84.0;

/// A volume this full is colored as a problem, as in VolumeRow.
const CRITICAL_FRACTION: f32 = 0.9;

/// The red a nearly full volume's bar turns.
///
/// A fixed value rather than the theme's critical role, because a capsule is
/// painted from colors decided at layout and the palette is only known at
/// draw time. The words beside the bar do use the theme's role.
const CRITICAL: Color = Color(0xEF4444);

/// What stands in for a reading that is not there.
const DASH: &str = "\u{2014}";

/// A mounted volume, as the Mac's DiskVolumeSample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Volume {
    /// The label, "Windows" or "Data". Empty when the volume has none.
    pub name: String,
    /// Where it is mounted, "C:".
    pub mount: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
}

impl Volume {
    /// How full the volume is, zero to one.
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let used = self.total.saturating_sub(self.available);
        (used as f64 / self.total as f64).clamp(0.0, 1.0) as f32
    }

    pub fn is_critical(&self) -> bool {
        self.used_fraction() >= CRITICAL_FRACTION
    }

    /// The label, or what Explorer calls an unlabeled volume - "Local Disk
    /// (C:)" - so the row does not say "C:" twice.
    fn display_name(&self) -> &str {
        if !self.name.is_empty() {
            &self.name
        } else if self.removable {
            "Removable Disk"
        } else {
            "Local Disk"
        }
    }
}

/// A physical disk, as the Mac's DiskDeviceSample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    /// The model, "Samsung SSD 990 PRO 2TB".
    pub model: String,
    /// The system's own name for it, "Disk 0", shown in a chip beside the
    /// model as the Mac shows its BSD name.
    pub id: String,
    /// Bytes per second.
    pub read: f64,
    pub write: f64,
    /// Operations per second.
    pub read_ops: f64,
    pub write_ops: f64,
}

/// What the strip knows, published on every tick.
///
/// The strip's thread keeps one of these, feeds it each tick's rates
/// through `observe` so the history accumulates while the panel is closed,
/// and publishes a clone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisksSnapshot {
    /// Read and write in bytes per second, from DiskModule::rates; None
    /// until the first sample.
    pub rates: Option<(f64, f64)>,
    /// Oldest first, never longer than HISTORY.
    pub history: Vec<(f32, f32)>,
    /// In the order they should be shown, the selected or system volume
    /// first, which is what the hero reports. Empty until the first sample
    /// enumerates them.
    pub volumes: Vec<Volume>,
    /// Empty until the per-instance PhysicalDisk counters have been read.
    pub devices: Vec<Device>,
}

impl DisksSnapshot {
    /// Takes one sample, as `DiskModule::rates` reports it.
    ///
    /// The graph holds still through a tick that produced nothing, rather
    /// than plotting a zero the machine did not do; a reading that stops
    /// arriving is shown as unavailable, not as quiet.
    pub fn observe(&mut self, rates: Option<(f64, f64)>) {
        self.rates = rates;
        if let Some((read, write)) = rates {
            self.history.push((read.max(0.0) as f32, write.max(0.0) as f32));
            if self.history.len() > HISTORY {
                let excess = self.history.len() - HISTORY;
                self.history.drain(..excess);
            }
        }
    }
}

/// The Disks flyout's layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisksFlyout {
    pub snapshot: DisksSnapshot,
}

impl DisksFlyout {
    pub fn new(snapshot: DisksSnapshot) -> DisksFlyout {
        DisksFlyout { snapshot }
    }

    pub fn title(&self) -> &str {
        "Disks"
    }

    /// How tall the content is at a width, in DIPs.
    pub fn height(&self, width: f32, measure: Measure) -> f32 {
        let mut b = Builder::new(0.0, 0.0, width, measure);
        self.build(&mut b);
        b.y()
    }

    /// Lays the panel out, top to bottom, in the Mac's order.
    pub fn build(&self, b: &mut Builder) {
        let s = &self.snapshot;
        let accent = Accent::signature(ModuleId::Disks);
        let selected = s.volumes.first();

        // The hero is the Mac's: the selected volume's name and free space,
        // with how full it is as the headline. Until volumes are enumerated
        // the headline stays empty rather than showing a dash forever, which
        // reads as a fault; the dash is kept for the one honest case, no
        // sample at all.
        let subtitle = match (s.rates, selected) {
            (None, _) => "Waiting for the first sample".to_string(),
            (Some(_), Some(volume)) => format!(
                "{}  \u{00B7}  {} free",
                volume.display_name(),
                format::bytes(volume.available)
            ),
            (Some(_), None) => "Read and write across every disk".to_string(),
        };
        let value = match (s.rates, selected) {
            (None, _) => Some(DASH.to_string()),
            (Some(_), Some(volume)) => Some(format::percent(volume.used_fraction())),
            (Some(_), None) => None,
        };
        b.hero_header(
            (module_color(ModuleId::Disks), module_glyph(ModuleId::Disks)),
            self.title(),
            Some(&subtitle),
            value.as_deref(),
            accent,
        );

        // Activity: the two rate tiles and the mirrored graph, on the tinted
        // card, shown whether or not a sample has arrived, as the Mac does.
        let card = b.card_begin(Some(accent.primary));
        let (read, write) = match s.rates {
            Some((read, write)) => (format::rate(read), format::rate(write)),
            None => (DASH.to_string(), DASH.to_string()),
        };
        rate_tiles(b, [(glyph::DOWN, "Read", &read, accent.primary), (glyph::UP, "Write", &write, accent.secondary)]);
        b.gap(8.0);
        mirrored_graph(b, &s.history, accent);
        b.card_end(card);

        if s.rates.is_none() {
            let card = b.card_begin(None);
            empty_state(b, "Disk data unavailable", None);
            b.card_end(card);
            return;
        }

        let card = b.card_begin(None);
        b.section_label("Volumes");
        if s.volumes.is_empty() {
            b.caption("Volume capacity is not collected yet.");
        }
        for (index, volume) in s.volumes.iter().enumerate() {
            if index > 0 {
                b.gap(10.0);
            }
            volume_row(b, volume, accent);
        }
        b.card_end(card);

        if !s.devices.is_empty() {
            let card = b.card_begin(None);
            b.section_label("Physical disks");
            for (index, device) in s.devices.iter().enumerate() {
                if index > 0 {
                    b.gap(8.0);
                }
                device_block(b, device, accent);
            }
            b.card_end(card);
        }
    }
}

/// The Disks content: the panel's end of the feed, and the layout.
pub struct DisksContent {
    slot: Arc<Mutex<DisksSnapshot>>,
    flyout: DisksFlyout,
}

impl DisksContent {
    /// Content reading from a feed's slot, as `Flyout::feed` hands it out.
    pub fn new(slot: Arc<Mutex<DisksSnapshot>>) -> DisksContent {
        DisksContent { slot, flyout: DisksFlyout::default() }
    }
}

impl Content for DisksContent {
    fn module(&self) -> ModuleId {
        ModuleId::Disks
    }

    fn build(&mut self, cx: &Context) -> Page {
        if let Some(snapshot) = latest(&self.slot) {
            self.flyout.snapshot = snapshot;
        }
        let mut b = cx.builder();
        self.flyout.build(&mut b);
        Page::finish(b)
    }

    fn tick(&mut self) -> bool {
        self.slot.lock().map(|slot| *slot != self.flyout.snapshot).unwrap_or(false)
    }
}

/// Two rate tiles side by side, from DiskRateTile: a large symbol, a caption
/// and a reading big enough to be the point of the card.
///
/// The Mac's RateTile in NetworkDropdownView.swift is the same view under
/// another name, and network.rs carries the same helper for the same reason;
/// it belongs in ui.rs once the chrome adopts it.
fn rate_tiles(b: &mut Builder, tiles: [(&'static str, &str, &str, Color); 2]) {
    let width = (b.inner_w() - TILE_GAP) / 2.0;
    let top = b.y();
    let pad = 9.0;
    for (column, (symbol, label, value, color)) in tiles.into_iter().enumerate() {
        let x = b.inner_x() + column as f32 * (width + TILE_GAP);
        let rect = Rect::new(x, top, width, TILE_H);
        b.passive(rect, Kind::Plate);
        b.passive(
            Rect::new(x + pad, top + (TILE_H - 22.0) / 2.0, 22.0, 22.0),
            Kind::Glyph { glyph: symbol, size: 22.0, ink: Ink::Custom(color) },
        );
        let text_x = x + pad + 22.0 + pad;
        let text_w = rect.right() - pad - text_x;
        b.text(Rect::new(text_x, top + pad, text_w, 16.0), label, Style::Caption, Ink::Secondary, Align::Left);
        b.text(
            Rect::new(text_x, top + pad + 16.0, text_w, 24.0),
            value,
            Style::Display(18.0),
            Ink::Primary,
            Align::Left,
        );
    }
    b.advance(TILE_H);
}

/// Reads above a centerline and writes hanging below it, from
/// MirroredAreaGraph, sharing one scale so the eye can compare them.
fn mirrored_graph(b: &mut Builder, history: &[(f32, f32)], accent: Accent) {
    let plate = b.plate(GRAPH_H);
    if history.len() < 2 {
        // Said in words, as a single area graph says it for itself: two
        // half-plates with nothing on them looked like a failure to draw.
        b.text(plate, "Collecting history{2026}", Style::Caption, Ink::Secondary, Align::Center);
        return;
    }
    let half = plate.h / 2.0;
    let (upper, lower) = mirrored_series(history);
    // The centerline goes down first so the washes sit over it, as the
    // Mac's ZStack orders them.
    b.passive(Rect::new(plate.x, plate.y + half, plate.w, 1.0), Kind::Divider);
    b.passive(Rect::new(plate.x, plate.y, plate.w, half), Kind::Graph(half_series(upper, accent.primary, false)));
    b.passive(
        Rect::new(plate.x, plate.y + half, plate.w, half),
        Kind::Graph(half_series(lower, accent.secondary, true)),
    );
}

/// One half of the mirrored pair: the Mac's NormalizedGraphSeries at line
/// width 1.5 with a marker and no grid, flipped for the lower half.
fn half_series(values: Vec<f32>, color: Color, flipped: bool) -> Graph {
    Graph {
        values,
        positions: None,
        color,
        color2: color,
        line: 1.5,
        marker: true,
        glow: true,
        grid: false,
        flipped,
        fill: (0.42, 0.03),
        inset: 3.0,
    }
}

/// Reads and writes scaled to a shared ceiling: the larger of the two
/// peaks with a tenth of headroom, and never less than one so a quiet disk
/// draws a flat line rather than dividing by zero.
pub fn mirrored_series(history: &[(f32, f32)]) -> (Vec<f32>, Vec<f32>) {
    let peak = history.iter().fold(0.0f32, |peak, (read, write)| peak.max(*read).max(*write));
    let ceiling = (peak * 1.1).max(1.0);
    let upper = history.iter().map(|(read, _)| (read / ceiling).min(1.0)).collect();
    let lower = history.iter().map(|(_, write)| (write / ceiling).min(1.0)).collect();
    (upper, lower)
}

/// One volume on an inset plate, from VolumeRow: name over mount point, how
/// full it is at the right, a capsule bar, and the two captions under it.
///
/// The Mac's eject button for removable volumes is not ported yet: ejecting
/// needs the shell, which the chrome owns. The drive mark beside the name is
/// the module's own tile at row size, which is what the composer list does
/// where the Mac draws an SF Symbol.
fn volume_row(b: &mut Builder, volume: &Volume, accent: Accent) {
    let plate = Rect::new(b.inner_x(), b.y(), b.inner_w(), VOLUME_ROW_H);
    b.passive(plate, Kind::Plate);
    let pad = 8.0;
    let top = plate.y + pad;
    let critical = volume.is_critical();

    let tile = 18.0;
    b.passive(
        Rect::new(plate.x + pad, top + 1.0, tile, tile),
        Kind::Tile { color: accent.primary, color2: accent.secondary, glyph: module_glyph(ModuleId::Disks) },
    );
    let used = format::percent(volume.used_fraction());
    let used_w = b.text_width(&used, Style::BodyStrong);
    let text_x = plate.x + pad + tile + 8.0;
    let text_w = (plate.right() - pad - used_w - 8.0 - text_x).max(24.0);
    b.text(Rect::new(text_x, top, text_w, 20.0), volume.display_name(), Style::BodyStrong, Ink::Primary, Align::Left);
    b.text(
        Rect::new(plate.right() - pad - used_w, top, used_w, 20.0),
        &used,
        Style::BodyStrong,
        if critical { Ink::Critical } else { Ink::Primary },
        Align::Right,
    );
    b.text(Rect::new(text_x, top + 20.0, text_w, 16.0), &volume.mount, Style::Caption, Ink::Secondary, Align::Left);

    let bar = Rect::new(plate.x + pad, top + 20.0 + 16.0 + 5.0, plate.w - 2.0 * pad, 6.0);
    b.passive(bar, Kind::Track);
    b.passive(
        bar,
        Kind::Capsule {
            fraction: volume.used_fraction(),
            color: if critical { CRITICAL } else { accent.primary },
            color2: if critical { CRITICAL } else { accent.secondary },
            glow: true,
        },
    );

    let captions = Rect::new(plate.x + pad, bar.bottom() + 5.0, plate.w - 2.0 * pad, 16.0);
    b.text(captions, &format!("{used} used"), Style::Caption, Ink::Secondary, Align::Left);
    b.text(
        captions,
        &format!("{} free", format::bytes(volume.available)),
        Style::Caption,
        Ink::Secondary,
        Align::Right,
    );
    b.advance(VOLUME_ROW_H);
}

/// One physical disk: its model with the system's name in a chip, then read,
/// write and operations per second.
fn device_block(b: &mut Builder, device: &Device, accent: Accent) {
    let line = Rect::new(b.inner_x(), b.y(), b.inner_w(), 20.0);
    let chip_w = b.text_width(&device.id, Style::CaptionStrong) + 24.0;
    b.text(
        Rect::new(line.x, line.y, (line.w - chip_w - 8.0).max(24.0), line.h),
        &device.model,
        Style::BodyStrong,
        Ink::Primary,
        Align::Left,
    );
    b.chip_at(line, &device.id, accent.primary, None);
    b.advance(20.0 + 2.0);
    b.metric_row(Some(glyph::DOWN), "Read", &format::rate(device.read), Ink::Custom(accent.primary));
    b.metric_row(Some(glyph::UP), "Write", &format::rate(device.write), Ink::Custom(accent.secondary));
    b.metric_row(
        Some(glyph::REFRESH),
        "Operations",
        &format!("{:.1} read \u{00B7} {:.1} write /s", device.read_ops, device.write_ops),
        Ink::Custom(accent.primary),
    );
}

/// The empty state, from ContentUnavailableView: the module's tile, a title,
/// and a sentence when there is one, centered.
///
/// The chrome's `unavailable` wants an icon-font glyph and the module marks
/// are chars for a tile, so the tile stands where the Mac's symbol does.
fn empty_state(b: &mut Builder, title: &str, description: Option<&str>) {
    b.gap(12.0);
    let size = 36.0;
    b.passive(
        Rect::new(b.inner_x() + (b.inner_w() - size) / 2.0, b.y(), size, size),
        Kind::Tile {
            color: module_color(ModuleId::Disks),
            color2: Accent::signature(ModuleId::Disks).secondary,
            glyph: module_glyph(ModuleId::Disks),
        },
    );
    b.advance(size + 8.0);
    b.text(Rect::new(b.inner_x(), b.y(), b.inner_w(), 20.0), title, Style::BodyStrong, Ink::Primary, Align::Center);
    b.advance(20.0);
    if let Some(description) = description {
        b.gap(4.0);
        b.wrapped(description, Style::Caption, Ink::Secondary, Align::Center);
    }
    b.gap(12.0);
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Element, Palette, CARD_GAP, PANEL_PAD, PANEL_W};
    use super::*;
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};

    /// Seven DIPs a character at body size, scaled by the style; wrapped by
    /// width when one is given. The same stand-in ui.rs tests with.
    fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    fn laid_out(snapshot: &DisksSnapshot) -> Vec<Element> {
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        DisksFlyout::new(snapshot.clone()).build(&mut b);
        b.elements
    }

    fn texts(elements: &[Element]) -> Vec<&str> {
        elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Text { text, .. } | Kind::Wrapped { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn volume(name: &str, mount: &str, total: u64, available: u64) -> Volume {
        Volume { name: name.into(), mount: mount.into(), total, available, removable: false }
    }

    fn sampled() -> DisksSnapshot {
        let mut snapshot = DisksSnapshot::default();
        snapshot.observe(Some((0.0, 0.0)));
        snapshot
    }

    #[test]
    fn the_history_keeps_the_last_three_hundred_samples_and_no_more() {
        let mut snapshot = DisksSnapshot::default();
        for i in 0..(HISTORY + 20) {
            snapshot.observe(Some((i as f64, 0.0)));
        }
        assert_eq!(snapshot.history.len(), HISTORY);
        // The oldest twenty went, the newest stayed.
        assert_eq!(snapshot.history[0].0, 20.0);
        assert_eq!(snapshot.history.last().unwrap().0, (HISTORY + 19) as f32);
    }

    #[test]
    fn a_tick_without_a_reading_leaves_the_history_alone() {
        let mut snapshot = DisksSnapshot::default();
        snapshot.observe(Some((10.0, 5.0)));
        snapshot.observe(None);
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.rates, None);
    }

    #[test]
    fn the_mirrored_series_share_one_ceiling_with_a_tenth_of_headroom() {
        let (upper, lower) = mirrored_series(&[(100.0, 50.0), (20.0, 80.0)]);
        // The peak is the read of 100, so the ceiling is 110 for both halves.
        assert!((upper[0] - 100.0 / 110.0).abs() < 1e-6);
        assert!((lower[0] - 50.0 / 110.0).abs() < 1e-6);
        assert!((lower[1] - 80.0 / 110.0).abs() < 1e-6);
        assert!(upper.iter().chain(lower.iter()).all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn a_quiet_disk_scales_against_one_rather_than_dividing_by_zero() {
        let (upper, lower) = mirrored_series(&[(0.0, 0.0), (0.0, 0.0)]);
        assert_eq!(upper, vec![0.0, 0.0]);
        assert_eq!(lower, vec![0.0, 0.0]);
        let (upper, lower) = mirrored_series(&[]);
        assert!(upper.is_empty() && lower.is_empty());
    }

    #[test]
    fn a_volume_nine_tenths_full_is_critical_and_an_empty_one_is_not() {
        assert!(volume("Windows", "C:", 1000, 100).is_critical());
        assert!(!volume("Windows", "C:", 1000, 101).is_critical());
        assert_eq!(volume("Data", "D:", 0, 0).used_fraction(), 0.0);
        // A reading with more free than total cannot push the bar negative.
        assert_eq!(volume("Data", "D:", 100, 200).used_fraction(), 0.0);
    }

    #[test]
    fn without_a_sample_the_activity_card_still_stands_and_the_rest_says_so() {
        let elements = laid_out(&DisksSnapshot::default());
        let texts = texts(&elements);
        assert!(texts.contains(&"Waiting for the first sample"));
        assert!(texts.contains(&"Disk data unavailable"));
        assert!(texts.contains(&DASH), "the tiles show a dash, not a zero");
        assert!(!texts.contains(&"Volumes"));
        // No history yet: the plate says so instead of drawing two empty halves.
        assert!(texts.contains(&"Collecting history{2026}"));
        assert_eq!(elements.iter().filter(|e| matches!(e.kind, Kind::Graph(_))).count(), 0);
    }

    #[test]
    fn with_rates_but_no_volumes_the_hero_carries_no_headline() {
        let mut snapshot = DisksSnapshot::default();
        snapshot.observe(Some((1024.0 * 1024.0, 512.0 * 1024.0)));
        let elements = laid_out(&snapshot);
        let texts = texts(&elements);
        assert!(texts.contains(&"Read and write across every disk"));
        assert!(texts.contains(&"1.00 MB/s"));
        assert!(texts.contains(&"512 KB/s"));
        assert!(texts.contains(&"Volumes"));
        assert!(texts.contains(&"Volume capacity is not collected yet."));
        assert!(!texts.contains(&DASH));
        assert!(!elements.iter().any(|e| matches!(&e.kind, Kind::Text { style: Style::Title, .. })));
    }

    #[test]
    fn the_hero_names_the_first_volume_and_how_full_it_is() {
        let mut snapshot = sampled();
        snapshot.volumes = vec![volume("Windows", "C:", 1000, 550), volume("", "D:", 2000, 1500)];
        let elements = laid_out(&snapshot);
        let texts = texts(&elements);
        assert!(texts.contains(&"Windows  \u{00B7}  550 B free"));
        let headline = elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { style: Style::Title, .. }))
            .expect("a headline value");
        assert!(matches!(&headline.kind, Kind::Text { text, .. } if text == "45%"));
        // Every volume carries its mount under the name, labeled or not.
        assert!(texts.contains(&"D:"));
        assert!(texts.contains(&"25% used"));
    }

    #[test]
    fn each_volume_adds_one_row_of_the_documented_height() {
        let mut one = sampled();
        one.volumes = vec![volume("Windows", "C:", 100, 50)];
        let mut two = one.clone();
        two.volumes = vec![volume("Windows", "C:", 100, 50), volume("Data", "D:", 100, 50)];
        let a = DisksFlyout::new(one).height(356.0, &measure);
        let b = DisksFlyout::new(two).height(356.0, &measure);
        assert!(((b - a) - (VOLUME_ROW_H + 10.0)).abs() < 1e-3, "{a} -> {b}");
    }

    #[test]
    fn a_nearly_full_volume_turns_its_bar_and_its_number_red() {
        let mut snapshot = sampled();
        snapshot.volumes = vec![volume("Windows", "C:", 100, 5)];
        let elements = laid_out(&snapshot);
        let capsule = elements.iter().find(|e| matches!(e.kind, Kind::Capsule { .. })).unwrap();
        assert!(matches!(capsule.kind, Kind::Capsule { color: CRITICAL, fraction, .. } if (fraction - 0.95).abs() < 1e-6));
        assert!(elements.iter().any(|e| matches!(&e.kind, Kind::Text { text, ink: Ink::Critical, .. } if text == "95%")));
    }

    #[test]
    fn the_graph_halves_split_the_plate_and_the_lower_one_hangs() {
        let mut snapshot = DisksSnapshot::default();
        snapshot.observe(Some((10.0, 20.0)));
        snapshot.observe(Some((30.0, 5.0)));
        let elements = laid_out(&snapshot);
        let graphs: Vec<&Element> = elements.iter().filter(|e| matches!(e.kind, Kind::Graph(_))).collect();
        assert_eq!(graphs.len(), 2);
        let plate = elements.iter().find(|e| matches!(e.kind, Kind::Plate) && e.rect.h == GRAPH_H).unwrap();
        assert_eq!(graphs[0].rect.y, plate.rect.y);
        assert_eq!(graphs[0].rect.h, GRAPH_H / 2.0);
        assert_eq!(graphs[1].rect.y, plate.rect.y + GRAPH_H / 2.0);
        assert!(matches!(&graphs[0].kind, Kind::Graph(g) if !g.flipped && g.values.len() == 2));
        assert!(matches!(&graphs[1].kind, Kind::Graph(g) if g.flipped && g.values.len() == 2));
    }

    #[test]
    fn physical_disks_appear_only_when_there_are_any() {
        let mut snapshot = sampled();
        assert!(!texts(&laid_out(&snapshot)).contains(&"Physical disks"));
        snapshot.devices = vec![Device {
            model: "Samsung SSD 990 PRO 2TB".into(),
            id: "Disk 0".into(),
            read: 2048.0,
            write: 0.0,
            read_ops: 12.5,
            write_ops: 3.0,
        }];
        let elements = laid_out(&snapshot);
        let texts = texts(&elements);
        assert!(texts.contains(&"Physical disks"));
        assert!(texts.contains(&"Samsung SSD 990 PRO 2TB"));
        assert!(texts.contains(&"12.5 read \u{00B7} 3.0 write /s"));
        assert!(elements.iter().any(|e| matches!(&e.kind, Kind::Chip { text, .. } if text == "Disk 0")));
    }

    #[test]
    fn the_rate_tiles_sit_side_by_side_at_the_tile_height() {
        let elements = laid_out(&sampled());
        let tiles: Vec<Rect> = elements
            .iter()
            .filter(|e| matches!(e.kind, Kind::Plate) && e.rect.h == TILE_H)
            .map(|e| e.rect)
            .collect();
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].y, tiles[1].y);
        assert!((tiles[1].x - tiles[0].right() - TILE_GAP).abs() < 1e-3);
        assert!((tiles[1].right() - (12.0 + 356.0 - 12.0)).abs() < 1e-3);
    }

    #[test]
    fn the_height_is_what_the_builder_walked() {
        let flyout = DisksFlyout::new(sampled());
        let elements = laid_out(&flyout.snapshot);
        let last_card = elements.iter().rev().find(|e| matches!(e.kind, Kind::Card { .. })).unwrap();
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        flyout.build(&mut b);
        assert!((b.y() - (last_card.rect.bottom() + CARD_GAP)).abs() < 1e-3);
        assert!(flyout.height(356.0, &measure) > 0.0);
    }

    #[test]
    fn the_content_follows_its_feed() {
        let slot = Arc::new(Mutex::new(DisksSnapshot::default()));
        let mut content = DisksContent::new(Arc::clone(&slot));
        let palette = Palette::resolve(Theme::resolve(false, DEFAULT_ACCENT));
        let cx = Context {
            width: PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Disks),
            measure: &measure,
            hover: None,
            now_unix: 0,
        };
        assert_eq!(content.module(), ModuleId::Disks);
        assert!(!content.tick());
        let page = content.build(&cx);
        assert!(texts(&page.elements).contains(&"Disk data unavailable"));
        slot.lock().unwrap().observe(Some((2048.0, 1024.0)));
        assert!(content.tick());
        let page = content.build(&cx);
        assert!(!content.tick());
        assert!(texts(&page.elements).contains(&"2.00 KB/s"));
        let expected = DisksFlyout::new(slot.lock().unwrap().clone()).height(PANEL_W - 2.0 * PANEL_PAD, &measure);
        assert!((page.height - (expected + 2.0 * PANEL_PAD)).abs() < 1e-3);
    }

    /// Paints the panel for a person to look at; see `flyout::render`.
    #[test]
    #[ignore]
    fn render_the_panel_to_bitmaps() {
        let slot = Arc::new(Mutex::new(sampled()));
        let mut content = DisksContent::new(Arc::clone(&slot));
        content.tick();
        for light in [false, true] {
            crate::flyout::render::to_bitmap(&mut content, "disks", light, 0);
        }
    }
}
