// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The Network flyout, ported from NetworkDropdownView.swift.
//
// The two rates in the header, the activity card with its tiles and the
// download and upload drawn over one another, what is using the network,
// the connection's addresses, and the Wi-Fi card when there is a radio. The
// order is the Mac's and so is every card's content; what differs is how
// much of it Windows will say on a given machine. The strip's NetworkModule
// sums bytes in and out; the interface's name and addresses come from
// `netinfo` beside it and the Wi-Fi card from the radio when there is one.
// Each of those is a typed input here, so a machine that answers none of
// them draws the same cards with a calm line in them rather than an error.
//
// Nothing here draws. `build` lays elements out in DIPs through the shared
// Builder and the chrome paints them.

use std::sync::{Arc, Mutex};

use barometer_core::format::{self, RateUnit};
use barometer_core::ModuleId;

use super::clipboard;
use super::cpu::{icons, latest, process_mark};
use super::ui::{
    Accent, Builder, Graph, Id, Ink, Kind, Measure, Style, Tile, TileIcon, MEASURING, ROW_H,
    TILE_GAP, TILE_H,
};
use super::{Content, Context, Page, Response};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::model::module_glyph;
use crate::settings_ui::theme::{glyph, module_color, Color};

/// How many samples the graph keeps: the Mac's `history.recent(300)`.
pub const HISTORY: usize = 300;

/// The graph's plate, from NetworkHistoryGraph's frame.
pub const GRAPH_H: f32 = 84.0;

/// A process row: the name beside two stacked caption-sized rates.
pub const PROCESS_ROW_H: f32 = 36.0;

/// The rows in the connection card answer to these, in the order the rows
/// are laid out, so a click can be turned back into the text to copy.
const COPY_BASE: u32 = 0x100;

/// What stands in for a reading that is not there.
const DASH: &str = "\u{2014}";

/// Apple's system colors, which is what the Swift's `.purple`, `.orange`
/// and the signal colors resolve to; kept so the chips match across the two
/// apps.
const VPN: Color = Color(0xAF52DE);
const CAUTION: Color = Color(0xFF9500);
const SIGNAL_POOR: Color = Color(0xFF3B30);
const SIGNAL_FAIR: Color = Color(0xFF9500);
const SIGNAL_GOOD: Color = Color(0x34C759);

/// What is known about the connection carrying the rates, as the Mac's
/// NetworkInterfaceSample and the sample's router and DNS.
///
/// Every field starts empty, which is the honest state until the first
/// sample and on a machine that will not name its interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connection {
    /// The interface's friendly name, "Ethernet". None reads as every
    /// interface summed, which is what the strip's figure is.
    pub interface: Option<String>,
    pub is_vpn: bool,
    /// Whether this interface carries the default route.
    pub is_primary: bool,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub router: Option<String>,
    pub dns: Vec<String>,
    /// The public address, looked up only when the user allows it.
    pub public_ipv4: Option<String>,
    pub public_ipv6: Option<String>,
    pub shows_public: bool,
    pub errors_in: u64,
    pub errors_out: u64,
}

/// The radio, as the Mac's WiFiSnapshot. Present only when there is one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub ssid: Option<String>,
    pub rssi: Option<i32>,
    pub noise: Option<i32>,
    pub channel: Option<u32>,
    pub band: Option<String>,
    pub transmit_mbps: Option<f64>,
    pub security: Option<String>,
}

/// One process on the network, as the Mac's NetworkProcessSample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    /// Bytes per second.
    pub down: f64,
    pub up: f64,
}

/// What the strip knows, published on every tick: the latest rates, their
/// history, the user's choices about how to show them, and whatever core
/// has learned about the connection.
///
/// The strip's thread keeps one of these, feeds it each tick's rates
/// through `observe` so the history accumulates while the panel is closed,
/// and publishes a clone.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkSnapshot {
    /// Down and up in bytes per second, from NetworkModule::rates; None
    /// until the first sample.
    pub rates: Option<(f64, f64)>,
    /// Oldest first, never longer than HISTORY.
    pub history: Vec<(f32, f32)>,
    /// Bytes received and sent since the counters last started.
    pub totals: Option<(u64, u64)>,
    /// settings.network_unit.
    pub unit: RateUnit,
    /// settings.network_upload_first: whether the header and the process
    /// rows put upload above download.
    pub upload_first: bool,
    /// ModuleSettings' showsProcesses and processCount on the Mac, at the
    /// Mac's defaults until the Windows settings have the switch.
    pub shows_processes: bool,
    pub process_limit: usize,
    /// None while per-process activity is not collected on this machine,
    /// which the Mac distinguishes from a list that happens to be empty.
    pub processes: Option<Vec<Process>>,
    /// Whether an empty list means the rates are still being measured rather
    /// than that nothing sent anything. A process's bytes a second is the
    /// difference between two reads of every connection's counters, and the
    /// read taken when the panel opened has nothing to difference against -
    /// the accounting is switched on with the panel and off with it, so this
    /// is the state every open starts in.
    pub measuring: bool,
    pub connection: Connection,
    pub wifi: Option<Wifi>,
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        NetworkSnapshot {
            rates: None,
            history: Vec::new(),
            totals: None,
            unit: RateUnit::Bytes,
            upload_first: false,
            shows_processes: true,
            process_limit: 5,
            processes: None,
            measuring: false,
            connection: Connection::default(),
            wifi: None,
        }
    }
}

impl NetworkSnapshot {
    /// Takes one sample, as `NetworkModule::rates` reports it.
    ///
    /// The graph holds still through a tick that produced nothing rather
    /// than plotting a zero the machine did not do.
    pub fn observe(&mut self, rates: Option<(f64, f64)>) {
        self.rates = rates;
        if let Some((down, up)) = rates {
            self.history.push((down.max(0.0) as f32, up.max(0.0) as f32));
            if self.history.len() > HISTORY {
                let excess = self.history.len() - HISTORY;
                self.history.drain(..excess);
            }
        }
    }

    fn rate(&self, bytes_per_sec: f64) -> String {
        format::rate_in(bytes_per_sec, self.unit)
    }

    /// The connection card's rows, in the order they are laid out, which is
    /// the order `copy_text` counts them in.
    fn copyable(&self) -> Vec<(&'static str, &str)> {
        let c = &self.connection;
        let mut rows: Vec<(&'static str, &str)> = Vec::new();
        rows.extend(c.ipv4.iter().map(|a| ("IPv4", a.as_str())));
        rows.extend(c.ipv6.iter().map(|a| ("IPv6", a.as_str())));
        rows.extend(c.router.iter().map(|a| ("Router", a.as_str())));
        rows.extend(c.dns.iter().map(|a| ("DNS", a.as_str())));
        if c.shows_public {
            rows.extend(c.public_ipv4.iter().map(|a| ("Public", a.as_str())));
            rows.extend(c.public_ipv6.iter().map(|a| ("Public", a.as_str())));
        }
        rows
    }

    /// The text a click on a connection row copies, by the row's id.
    pub fn copy_text(&self, id: Id) -> Option<String> {
        let Id::Custom(raw) = id else {
            return None;
        };
        let index = raw.checked_sub(COPY_BASE)? as usize;
        self.copyable().get(index).map(|(_, value)| value.to_string())
    }

    /// The header's second line: the interface, and the network when the
    /// interface is the radio carrying the default route.
    fn subtitle(&self) -> String {
        if self.rates.is_none() {
            return "Waiting for the first sample".to_string();
        }
        let name = self.connection.interface.as_deref().unwrap_or("All interfaces");
        match self.wifi.as_ref().and_then(|wifi| wifi.ssid.as_deref()) {
            Some(ssid) if self.connection.is_primary => format!("{name}  \u{00B7}  {ssid}"),
            _ => name.to_string(),
        }
    }
}

/// The Network flyout's layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkFlyout {
    pub snapshot: NetworkSnapshot,
    /// The row whose address was just copied, which shows a check where its
    /// copy glyph was until the next sample arrives - a second, near enough
    /// the Mac's 1.2 - so the click is seen to have done something.
    pub copied: Option<Id>,
}

impl NetworkFlyout {
    pub fn new(snapshot: NetworkSnapshot) -> NetworkFlyout {
        NetworkFlyout { snapshot, copied: None }
    }

    pub fn title(&self) -> &str {
        "Network"
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
        let accent = Accent::signature(ModuleId::Network);
        let (down, up) = match s.rates {
            Some((down, up)) => (s.rate(down), s.rate(up)),
            None => (DASH.to_string(), DASH.to_string()),
        };

        // The header carries no headline value; the two rates stand where
        // the Mac's HeroRate accessory does, so the subtitle is cut short of
        // them rather than run underneath.
        let rates_w = b.text_width(&down, Style::BodyStrong).max(b.text_width(&up, Style::BodyStrong)) + 16.0;
        let subtitle = fit(b, &s.subtitle(), Style::Caption, b.width() - 2.0 - 36.0 - 12.0 - rates_w - 8.0);
        let top = b.y();
        b.hero_header(
            (module_color(ModuleId::Network), module_glyph(ModuleId::Network)),
            self.title(),
            Some(&subtitle),
            None,
            accent,
        );
        hero_rates(b, top, &down, &up, s.upload_first, accent);

        // Activity. The Mac's interface picker trails the label; there is
        // no dropdown in the vocabulary yet and nothing to pick from.
        let card = b.card_begin(Some(accent.primary));
        b.section_label("Activity");
        rate_tiles(
            b,
            [(glyph::DOWN, "Download", &down, accent.primary), (glyph::UP, "Upload", &up, accent.secondary)],
        );
        b.gap(8.0);
        dual_graph(b, &s.history, accent);
        if s.rates.is_some() {
            b.gap(8.0);
            let name = s.connection.interface.clone().unwrap_or_else(|| "All interfaces".to_string());
            let mut chips = vec![(name, accent.primary)];
            if s.connection.is_vpn {
                chips.push(("VPN".to_string(), VPN));
            }
            if s.connection.is_primary {
                chips.push(("Primary".to_string(), accent.secondary));
            }
            let totals = s.totals.map(|(received, sent)| {
                format!("{} received  \u{00B7}  {} sent", format::bytes(received), format::bytes(sent))
            });
            chips_row(b, &chips, totals.as_deref());
        }
        b.card_end(card);

        if s.rates.is_none() {
            return;
        }

        if s.shows_processes {
            let card = b.card_begin(None);
            b.section_label("Top network activity");
            match &s.processes {
                None => b.caption("Per-process activity unavailable"),
                // Measuring first: an empty list a second after the panel
                // opened is the accounting having no interval yet, not a
                // quiet machine, and the two must not read alike.
                Some(processes) if processes.is_empty() && s.measuring => b.caption(MEASURING),
                Some(processes) if processes.is_empty() => b.caption("No recent network activity"),
                Some(processes) => {
                    for process in processes.iter().take(s.process_limit) {
                        process_row(b, s, process, accent);
                    }
                }
            }
            b.card_end(card);
        }

        let card = b.card_begin(None);
        let area = b.section_label("Connection");
        let c = &s.connection;
        if c.errors_in > 0 || c.errors_out > 0 {
            b.chip_at(area, &format!("{} in \u{00B7} {} out errors", c.errors_in, c.errors_out), CAUTION, None);
        }
        let rows = s.copyable();
        if rows.is_empty() {
            b.caption("Addresses, router and DNS are not collected yet.");
        }
        for (index, (label, value)) in rows.iter().enumerate() {
            let id = Id::Custom(COPY_BASE + index as u32);
            copy_row(b, id, label, value, self.copied == Some(id));
        }
        if c.shows_public && c.public_ipv4.is_none() && c.public_ipv6.is_none() {
            b.metric_row(None, "Public address", "Lookup unavailable", Ink::Custom(accent.primary));
        }
        b.card_end(card);

        if let Some(wifi) = &s.wifi {
            let card = b.card_begin(None);
            let area = b.section_label("Wi-Fi");
            if let Some(rssi) = wifi.rssi {
                b.chip_at(area, &format!("{rssi} dBm"), signal_color(rssi), None);
            }
            b.metric_row(None, "Network", wifi.ssid.as_deref().unwrap_or("Name unavailable"), Ink::Custom(accent.primary));
            let mut tiles = Vec::new();
            if let Some(noise) = wifi.noise {
                tiles.push(tile("Noise", format!("{noise} dBm")));
            }
            if let Some(channel) = wifi.channel {
                tiles.push(tile("Channel", format!("{channel} \u{00B7} {}", wifi.band.as_deref().unwrap_or(DASH))));
            }
            if let Some(rate) = wifi.transmit_mbps {
                tiles.push(tile("Transmit rate", format!("{rate:.0} Mbps")));
            }
            if let Some(security) = &wifi.security {
                tiles.push(tile("Security", security.clone()));
            }
            if !tiles.is_empty() {
                b.gap(8.0);
                b.stat_tiles(tiles);
            }
            b.card_end(card);
        }
    }
}

/// The Network content: the panel's end of the feed, and the layout.
pub struct NetworkContent {
    slot: Arc<Mutex<NetworkSnapshot>>,
    flyout: NetworkFlyout,
    /// Puts a row's address on the clipboard. The clipboard itself by
    /// default; a test puts its own here.
    copy: Box<dyn FnMut(&str) -> bool + Send>,
}

impl NetworkContent {
    /// Content reading from a feed's slot, as `Flyout::feed` hands it out.
    pub fn new(slot: Arc<Mutex<NetworkSnapshot>>) -> NetworkContent {
        NetworkContent { slot, flyout: NetworkFlyout::default(), copy: Box::new(clipboard::copy_text) }
    }

    /// What to do with an address the user clicked, instead of copying it.
    pub fn on_copy(mut self, action: impl FnMut(&str) -> bool + Send + 'static) -> NetworkContent {
        self.copy = Box::new(action);
        self
    }
}

impl Content for NetworkContent {
    fn module(&self) -> ModuleId {
        ModuleId::Network
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
        let Some(text) = self.flyout.snapshot.copy_text(id) else {
            return Response::None;
        };
        if (self.copy)(&text) {
            self.flyout.copied = Some(id);
            Response::Repaint
        } else {
            Response::None
        }
    }

    fn tick(&mut self) -> bool {
        let changed = self.slot.lock().map(|slot| *slot != self.flyout.snapshot).unwrap_or(false);
        if changed {
            self.flyout.copied = None;
        }
        changed
    }
}

/// One process, from ProcessRow with the two rates stacked at its right in
/// the order the user chose, and its application's icon at the left, where
/// the Mac draws one.
fn process_row(b: &mut Builder, s: &NetworkSnapshot, process: &Process, accent: Accent) {
    let row = Rect::new(b.inner_x(), b.y(), b.inner_w(), PROCESS_ROW_H);
    // The arrows are U+2193 and U+2191 as escapes, for the reason the
    // core's network module gives: a literal pair of them was once turned
    // to mojibake by a text round trip.
    let down = (format!("\u{2193}{}", s.rate(process.down)), accent.primary);
    let up = (format!("\u{2191}{}", s.rate(process.up)), accent.secondary);
    let (first, second) = if s.upload_first { (up, down) } else { (down, up) };
    let w = b.text_width(&first.0, Style::Caption).max(b.text_width(&second.0, Style::Caption));
    let name_x = process_mark(b, row, super::appicon::icon_for(process.pid));
    let name_w = (row.right() - 6.0 - w - 8.0 - name_x).max(24.0);
    b.text(Rect::new(name_x, row.y, name_w, row.h), &process.name, Style::Body, Ink::Primary, Align::Left);
    let x = row.right() - 6.0 - w;
    b.text(Rect::new(x, row.y + 2.0, w, 16.0), &first.0, Style::Caption, Ink::Custom(first.1), Align::Right);
    b.text(Rect::new(x, row.y + 18.0, w, 16.0), &second.0, Style::Caption, Ink::Custom(second.1), Align::Right);
    b.advance(PROCESS_ROW_H);
}

fn tile(label: &str, value: String) -> Tile {
    Tile { icon: TileIcon::None, label: label.to_string(), value, tint: Ink::Secondary }
}

/// The Mac's signalColor: red at -80 dBm and below, orange to -67, green
/// above.
pub fn signal_color(rssi: i32) -> Color {
    if rssi <= -80 {
        SIGNAL_POOR
    } else if rssi <= -67 {
        SIGNAL_FAIR
    } else {
        SIGNAL_GOOD
    }
}

/// Text cut to a width with an ellipsis, so a line laid under something
/// else stops short of it. The painter's own ellipsis only knows the
/// rectangle it was given.
fn fit(b: &Builder, text: &str, style: Style, width: f32) -> String {
    if b.text_width(text, style) <= width {
        return text.to_string();
    }
    let mut kept: String = text.trim_end().to_string();
    while kept.pop().is_some() {
        let candidate = format!("{}\u{2026}", kept.trim_end());
        if b.text_width(&candidate, style) <= width {
            return candidate;
        }
    }
    "\u{2026}".to_string()
}

/// The two rates at the header's right, from HeroRate: an arrow and a bold
/// reading, download over upload unless the user turned them around.
fn hero_rates(b: &mut Builder, top: f32, down: &str, up: &str, upload_first: bool, accent: Accent) {
    let down = (glyph::DOWN, down, accent.primary);
    let up = (glyph::UP, up, accent.secondary);
    let lines = if upload_first { [up, down] } else { [down, up] };
    let right = b.right() - 2.0;
    for (index, (arrow, value, color)) in lines.into_iter().enumerate() {
        let y = top + index as f32 * 20.0;
        let w = b.text_width(value, Style::BodyStrong);
        b.passive(
            Rect::new(right - w - 4.0 - 12.0, y, 12.0, 20.0),
            Kind::Glyph { glyph: arrow, size: 11.0, ink: Ink::Custom(color) },
        );
        b.text(Rect::new(right - w, y, w, 20.0), value, Style::BodyStrong, Ink::Custom(color), Align::Right);
    }
}

/// Two rate tiles side by side, from RateTile: a large symbol, a caption
/// and a reading big enough to be the point of the card.
///
/// disks.rs carries the same helper for the Mac's DiskRateTile, which is
/// the same view under another name; it belongs in ui.rs once the chrome
/// adopts it.
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

/// Download and upload on one plate, the first behind the second, from
/// DualAreaGraph.
fn dual_graph(b: &mut Builder, history: &[(f32, f32)], accent: Accent) {
    let (down, up) = dual_series(history);
    let plate = b.plate(GRAPH_H);
    let mut download = Graph::area(down, Accent { primary: accent.primary, secondary: accent.primary });
    download.line = 1.5;
    let mut upload = Graph::area(up, Accent { primary: accent.secondary, secondary: accent.secondary });
    upload.line = 1.5;
    // One set of gridlines between the two, not two sets over each other.
    upload.grid = false;
    b.passive(plate, Kind::Graph(download));
    b.passive(plate, Kind::Graph(upload));
}

/// Both series scaled to one ceiling: the larger peak, never less than one,
/// with a tenth of headroom. The Mac's fixed-scale setting is not in core
/// yet, so the scale is always automatic.
pub fn dual_series(history: &[(f32, f32)]) -> (Vec<f32>, Vec<f32>) {
    let peak = history.iter().fold(1.0f32, |peak, (down, up)| peak.max(*down).max(*up));
    let ceiling = peak * 1.1;
    let down = history.iter().map(|(down, _)| (down / ceiling).min(1.0)).collect();
    let up = history.iter().map(|(_, up)| (up / ceiling).min(1.0)).collect();
    (down, up)
}

/// Chips left to right, and a caption right-aligned in what is left. The
/// caption gives way before the chips do, as the Mac's layout priority
/// arranges.
fn chips_row(b: &mut Builder, chips: &[(String, Color)], trailing: Option<&str>) {
    let row = Rect::new(b.inner_x(), b.y(), b.inner_w(), 20.0);
    let mut x = row.x;
    for (text, color) in chips {
        // The chrome's own chip width for the dot variant.
        let w = b.text_width(text, Style::CaptionStrong) + 24.0;
        b.passive(Rect::new(x, row.y, w, row.h), Kind::Chip { text: text.clone(), color: *color, glyph: None });
        x += w + 6.0;
    }
    if let Some(text) = trailing {
        let w = row.right() - x - 6.0;
        if w > 0.0 {
            b.text(Rect::new(x + 6.0, row.y, w, row.h), text, Style::Caption, Ink::Secondary, Align::Right);
        }
    }
    b.advance(row.h);
}

/// A label and an address that copies when clicked, from
/// CopyableNetworkValue: the copy glyph at the row's right, which the Mac
/// dims until the pointer arrives and this draws in the tertiary ink, and a
/// check in its place once the address has been copied.
fn copy_row(b: &mut Builder, id: Id, label: &str, value: &str, copied: bool) {
    let row = b.row_begin(id, ROW_H, false);
    let label_w = 44.0;
    b.text(Rect::new(row.x + 6.0, row.y, label_w, row.h), label, Style::Body, Ink::Secondary, Align::Left);
    let mark = Rect::new(row.right() - 6.0 - 16.0, row.y, 16.0, row.h);
    let x = row.x + 6.0 + label_w + 8.0;
    b.text(Rect::new(x, row.y, mark.x - 8.0 - x, row.h), value, Style::Body, Ink::Primary, Align::Left);
    if copied {
        b.passive(mark, Kind::Glyph { glyph: glyph::CHECK, size: 12.0, ink: Ink::Accent });
    } else {
        b.passive(mark, Kind::Glyph { glyph: icons::COPY, size: 12.0, ink: Ink::Tertiary });
    }
    b.advance(ROW_H);
}

#[cfg(test)]
mod tests {
    use super::super::ui::{Element, Palette, PANEL_W};
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

    fn laid_out(snapshot: &NetworkSnapshot) -> Vec<Element> {
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        NetworkFlyout::new(snapshot.clone()).build(&mut b);
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

    fn sampled(unit: RateUnit, upload_first: bool) -> NetworkSnapshot {
        let mut snapshot = NetworkSnapshot { unit, upload_first, ..Default::default() };
        snapshot.observe(Some((1024.0 * 1024.0, 256.0 * 1024.0)));
        snapshot
    }

    #[test]
    fn the_history_keeps_the_last_three_hundred_samples_and_no_more() {
        let mut snapshot = NetworkSnapshot::default();
        for i in 0..(HISTORY + 5) {
            snapshot.observe(Some((i as f64, 0.0)));
        }
        assert_eq!(snapshot.history.len(), HISTORY);
        assert_eq!(snapshot.history[0].0, 5.0);
        snapshot.observe(None);
        assert_eq!(snapshot.history.len(), HISTORY, "a missed tick plots nothing");
        assert_eq!(snapshot.rates, None);
    }

    #[test]
    fn rates_follow_the_users_convention() {
        let bytes = texts(&laid_out(&sampled(RateUnit::Bytes, false))).join("|");
        assert!(bytes.contains("1.00 MB/s"), "{bytes}");
        assert!(bytes.contains("256 KB/s"), "{bytes}");
        let bits = texts(&laid_out(&sampled(RateUnit::Bits, false))).join("|");
        assert!(bits.contains("8.39 Mb/s"), "{bits}");
        assert!(bits.contains("2.10 Mb/s"), "{bits}");
        assert!(!bits.contains("MB/s"), "{bits}");
    }

    #[test]
    fn the_hero_rates_put_upload_first_when_asked() {
        let find = |elements: &[Element], text: &str| -> f32 {
            elements
                .iter()
                .find(|e| matches!(&e.kind, Kind::Text { text: t, style: Style::BodyStrong, .. } if t == text))
                .map(|e| e.rect.y)
                .expect(text)
        };
        let down_first = laid_out(&sampled(RateUnit::Bytes, false));
        assert!(find(&down_first, "1.00 MB/s") < find(&down_first, "256 KB/s"));
        let up_first = laid_out(&sampled(RateUnit::Bytes, true));
        assert!(find(&up_first, "256 KB/s") < find(&up_first, "1.00 MB/s"));
        // Both lines sit inside the header, right-aligned to its edge.
        let value = up_first
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { text, style: Style::BodyStrong, .. } if text == "256 KB/s"))
            .unwrap();
        assert!((value.rect.right() - (12.0 + 356.0 - 2.0)).abs() < 1e-3);
        assert_eq!(value.rect.y, 12.0);
    }

    #[test]
    fn the_series_share_a_ceiling_that_never_drops_below_one() {
        let (down, up) = dual_series(&[(100.0, 40.0), (10.0, 80.0)]);
        assert!((down[0] - 100.0 / 110.0).abs() < 1e-6);
        assert!((up[1] - 80.0 / 110.0).abs() < 1e-6);
        let (down, up) = dual_series(&[(0.5, 0.0)]);
        // A peak under one is scaled against one, so a whisper of traffic
        // does not fill the plate.
        assert!((down[0] - 0.5 / 1.1).abs() < 1e-6);
        assert_eq!(up, vec![0.0]);
        let (down, up) = dual_series(&[]);
        assert!(down.is_empty() && up.is_empty());
    }

    #[test]
    fn the_two_graphs_share_one_plate_and_one_set_of_gridlines() {
        let elements = laid_out(&sampled(RateUnit::Bytes, false));
        let graphs: Vec<&Element> = elements.iter().filter(|e| matches!(e.kind, Kind::Graph(_))).collect();
        assert_eq!(graphs.len(), 2);
        assert_eq!(graphs[0].rect, graphs[1].rect);
        assert_eq!(graphs[0].rect.h, GRAPH_H);
        assert!(matches!(&graphs[0].kind, Kind::Graph(g) if g.grid));
        assert!(matches!(&graphs[1].kind, Kind::Graph(g) if !g.grid));
    }

    #[test]
    fn without_a_sample_only_the_header_and_the_activity_card_stand() {
        let elements = laid_out(&NetworkSnapshot::default());
        let texts = texts(&elements);
        assert!(texts.contains(&"Waiting for the first sample"));
        assert!(texts.contains(&DASH));
        assert!(texts.contains(&"ACTIVITY"));
        assert!(!texts.contains(&"CONNECTION"));
        assert!(!texts.contains(&"TOP NETWORK ACTIVITY"));
        assert!(!elements.iter().any(|e| matches!(e.kind, Kind::Chip { .. })));
    }

    #[test]
    fn the_subtitle_names_the_interface_and_the_network_it_is_on() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        assert_eq!(snapshot.subtitle(), "All interfaces");
        snapshot.connection = Connection { interface: Some("Wi-Fi".into()), is_primary: true, ..Default::default() };
        snapshot.wifi = Some(Wifi { ssid: Some("Home".into()), ..Default::default() });
        assert_eq!(snapshot.subtitle(), "Wi-Fi  \u{00B7}  Home");
        // A radio that is not the default route does not name the network.
        snapshot.connection = Connection { interface: Some("Ethernet".into()), ..Default::default() };
        assert_eq!(snapshot.subtitle(), "Ethernet");
    }

    #[test]
    fn a_long_subtitle_is_cut_before_it_reaches_the_hero_rates() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        snapshot.connection = Connection {
            interface: Some("An interface with a very long friendly name indeed".into()),
            ..Default::default()
        };
        let elements = laid_out(&snapshot);
        let subtitle = elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { style: Style::Caption, text, .. } if text.ends_with('\u{2026}')))
            .expect("an ellipsized subtitle");
        let rate = elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Glyph { .. }) && e.rect.y == 12.0)
            .expect("the first hero arrow");
        let b = Builder::new(12.0, 12.0, 356.0, &measure);
        let text = match &subtitle.kind {
            Kind::Text { text, .. } => text.clone(),
            _ => unreachable!(),
        };
        assert!(subtitle.rect.x + b.text_width(&text, Style::Caption) <= rate.rect.x);
        // A short one is left alone.
        assert_eq!(fit(&b, "Ethernet", Style::Caption, 200.0), "Ethernet");
    }

    #[test]
    fn the_chips_row_names_the_interface_a_vpn_the_primary_and_the_totals() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        snapshot.connection = Connection {
            interface: Some("Ethernet".into()),
            is_vpn: true,
            is_primary: true,
            ..Default::default()
        };
        snapshot.totals = Some((1024, 2048));
        let elements = laid_out(&snapshot);
        let chips: Vec<&str> = elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Chip { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(chips, vec!["Ethernet", "VPN", "Primary"]);
        assert!(texts(&elements).contains(&"1.00 KB received  \u{00B7}  2.00 KB sent"));
        // The chips sit on one line, left to right, and the caption after.
        let rects: Vec<Rect> = elements.iter().filter(|e| matches!(e.kind, Kind::Chip { .. })).map(|e| e.rect).collect();
        assert!(rects.windows(2).all(|pair| pair[0].y == pair[1].y && pair[1].x > pair[0].right()));
    }

    #[test]
    fn per_process_activity_says_it_is_unavailable_until_it_exists() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        assert!(texts(&laid_out(&snapshot)).contains(&"Per-process activity unavailable"));
        snapshot.processes = Some(Vec::new());
        assert!(texts(&laid_out(&snapshot)).contains(&"No recent network activity"));
        // The same empty list a second after the panel opened is the
        // accounting having no interval to divide by yet, and saying the
        // machine was quiet would be inventing a reading.
        snapshot.measuring = true;
        let elements = laid_out(&snapshot);
        let words = texts(&elements);
        assert!(words.contains(&MEASURING));
        assert!(!words.contains(&"No recent network activity"));
        snapshot.measuring = false;
        snapshot.processes = Some((0..7).map(|i| Process { pid: 0, name: format!("app{i}"), down: 1024.0, up: 0.0 }).collect());
        let elements = laid_out(&snapshot);
        let words = texts(&elements);
        assert!(words.contains(&"app4"));
        assert!(!words.contains(&"app5"), "only the configured number of rows");
        assert!(words.contains(&"\u{2193}1.00 KB/s"));
        // Each row carries the neutral mark where the Mac draws an icon.
        let marks = elements.iter().filter(|e| matches!(e.kind, Kind::Glyph { glyph, .. } if glyph == icons::PROCESS)).count();
        assert_eq!(marks, 5);
        snapshot.shows_processes = false;
        assert!(!texts(&laid_out(&snapshot)).contains(&"TOP NETWORK ACTIVITY"));
    }

    #[test]
    fn connection_rows_answer_to_copy_ids_in_the_order_they_are_laid_out() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        snapshot.connection = Connection {
            ipv4: vec!["192.168.1.20".into(), "10.0.0.5".into()],
            router: Some("192.168.1.1".into()),
            dns: vec!["1.1.1.1".into()],
            public_ipv4: Some("203.0.113.9".into()),
            shows_public: false,
            ..Default::default()
        };
        assert_eq!(snapshot.copy_text(Id::Custom(COPY_BASE)).as_deref(), Some("192.168.1.20"));
        assert_eq!(snapshot.copy_text(Id::Custom(COPY_BASE + 1)).as_deref(), Some("10.0.0.5"));
        assert_eq!(snapshot.copy_text(Id::Custom(COPY_BASE + 2)).as_deref(), Some("192.168.1.1"));
        assert_eq!(snapshot.copy_text(Id::Custom(COPY_BASE + 3)).as_deref(), Some("1.1.1.1"));
        // The public address is not offered while the user has not asked.
        assert_eq!(snapshot.copy_text(Id::Custom(COPY_BASE + 4)), None);
        assert_eq!(snapshot.copy_text(Id::Settings), None);
        assert_eq!(snapshot.copy_text(Id::Custom(3)), None);
        let elements = laid_out(&snapshot);
        let rows: Vec<Id> = elements.iter().filter(|e| matches!(e.kind, Kind::Row { .. })).map(|e| e.id).collect();
        assert_eq!(rows, (0..4).map(|i| Id::Custom(COPY_BASE + i)).collect::<Vec<_>>());
        assert!(!texts(&elements).contains(&"Addresses, router and DNS are not collected yet."));
        // Every row carries the copy glyph.
        let copies = elements.iter().filter(|e| matches!(e.kind, Kind::Glyph { glyph, .. } if glyph == icons::COPY)).count();
        assert_eq!(copies, 4);
    }

    #[test]
    fn an_empty_connection_says_what_is_missing_and_errors_get_a_chip() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        assert!(texts(&laid_out(&snapshot)).contains(&"Addresses, router and DNS are not collected yet."));
        snapshot.connection = Connection { errors_in: 3, errors_out: 1, ..Default::default() };
        let elements = laid_out(&snapshot);
        assert!(elements.iter().any(|e| matches!(&e.kind, Kind::Chip { text, color: CAUTION, .. } if text == "3 in \u{00B7} 1 out errors")));
    }

    #[test]
    fn the_wifi_card_appears_only_with_a_radio_and_names_the_signal() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        assert!(!texts(&laid_out(&snapshot)).contains(&"WI-FI"));
        snapshot.wifi = Some(Wifi {
            ssid: None,
            rssi: Some(-70),
            noise: Some(-92),
            channel: Some(36),
            band: Some("5 GHz".into()),
            transmit_mbps: None,
            security: Some("WPA3".into()),
        });
        let elements = laid_out(&snapshot);
        let texts = texts(&elements);
        assert!(texts.contains(&"WI-FI"));
        assert!(texts.contains(&"Name unavailable"));
        assert!(texts.contains(&"36 \u{00B7} 5 GHz"));
        assert!(texts.contains(&"WPA3"));
        assert!(!texts.contains(&"Transmit rate"));
        assert!(elements.iter().any(|e| matches!(&e.kind, Kind::Chip { text, color: SIGNAL_FAIR, .. } if text == "-70 dBm")));
    }

    #[test]
    fn signal_color_steps_at_the_macs_thresholds() {
        assert_eq!(signal_color(-81), SIGNAL_POOR);
        assert_eq!(signal_color(-80), SIGNAL_POOR);
        assert_eq!(signal_color(-79), SIGNAL_FAIR);
        assert_eq!(signal_color(-67), SIGNAL_FAIR);
        assert_eq!(signal_color(-66), SIGNAL_GOOD);
    }

    #[test]
    fn the_height_is_what_the_builder_walked() {
        let flyout = NetworkFlyout::new(sampled(RateUnit::Bytes, false));
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        flyout.build(&mut b);
        assert_eq!(flyout.height(356.0, &measure), b.y() - 12.0);
        assert!(b.y() > 12.0 + 40.0 + GRAPH_H);
    }

    #[test]
    fn a_click_on_an_address_copies_it_and_shows_a_check_until_the_next_sample() {
        let mut snapshot = sampled(RateUnit::Bytes, false);
        snapshot.connection = Connection { ipv4: vec!["192.168.1.20".into()], ..Default::default() };
        let slot = Arc::new(Mutex::new(snapshot));
        let copied = Arc::new(Mutex::new(None));
        let mut content = NetworkContent::new(Arc::clone(&slot)).on_copy({
            let copied = Arc::clone(&copied);
            move |text| {
                *copied.lock().unwrap() = Some(text.to_string());
                true
            }
        });
        let palette = Palette::resolve(Theme::resolve(false, DEFAULT_ACCENT));
        let cx = Context {
            width: PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Network),
            measure: &measure,
            hover: None,
            now_unix: 0,
        };
        assert_eq!(content.module(), ModuleId::Network);
        assert!(content.tick());
        content.build(&cx);
        assert_eq!(content.activate(Id::Custom(COPY_BASE)), Response::Repaint);
        assert_eq!(copied.lock().unwrap().as_deref(), Some("192.168.1.20"));
        let page = content.build(&cx);
        let checks = page.elements.iter().filter(|e| matches!(e.kind, Kind::Glyph { glyph, .. } if glyph == glyph::CHECK)).count();
        assert_eq!(checks, 1);
        // A row that is not an address does nothing; the next sample clears
        // the check.
        assert_eq!(content.activate(Id::Custom(3)), Response::None);
        slot.lock().unwrap().observe(Some((1.0, 1.0)));
        assert!(content.tick());
        let page = content.build(&cx);
        assert!(!page.elements.iter().any(|e| matches!(e.kind, Kind::Glyph { glyph, .. } if glyph == glyph::CHECK)));
    }
}
