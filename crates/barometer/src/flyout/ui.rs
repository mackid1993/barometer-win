// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The vocabulary every flyout is built from, ported from DropdownViews.swift
// and the tokens in BarometerDesign.swift.
//
// A flyout on the Mac is a column of glass cards, each carrying a section
// label and some mixture of the same dozen things: a hero header with the
// module's tile and headline value, rows of a label and a reading, a grid of
// small tiles, chips, a capsule bar, an area graph. The eight dropdown views
// differ in what they say and not in how; this file is the how, so that a
// module's flyout is a list of what it wants to say and nothing else.
//
// Layout is a list of `Element`s in DIPs, built by a `Builder` that walks
// down the panel, and then used for both drawing and hit testing so the two
// cannot disagree about where anything is - the same arrangement as the
// settings window's ui.rs, from which the idea is borrowed. Nothing here
// knows what a forecast or a core count is: a module that needs a picture the
// vocabulary lacks supplies a `Painter` and the chrome calls it.

use barometer_core::ModuleId;

use crate::settings_ui::gdi::{Align, Face, TextStyle};
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::theme::{self, Color, Theme, WHITE};

/// The panel's width, from every dropdown's contentSize.
///
/// docs/ui-design.md says 320 "matching macOS"; the macOS views are 380, and
/// the Swift is the specification.
pub const PANEL_W: f32 = 380.0;
/// Around the column of cards.
pub const PANEL_PAD: f32 = 12.0;
/// Inside a card.
pub const CARD_PAD: f32 = 12.0;
/// Between cards.
pub const CARD_GAP: f32 = 10.0;
/// Cards are Windows' 8, not the Mac's 14 (docs/ui-design.md, section 13).
pub const CARD_RADIUS: f32 = 8.0;
/// The inset plate inside a card, and a tile.
pub const PLATE_RADIUS: f32 = 6.0;
/// The footer under the scrolling content.
pub const FOOTER_H: f32 = 40.0;
/// Tallest a panel grows before its content scrolls.
pub const MAX_PANEL_H: f32 = 720.0;
/// A section label's line, and the gap under it.
pub const SECTION_LABEL_H: f32 = 16.0;
pub const SECTION_GAP: f32 = 8.0;
/// A metric row.
pub const ROW_H: f32 = 28.0;
/// A label-and-value line in a grid.
pub const KV_ROW_H: f32 = 20.0;
/// A stat tile and the gap in a grid of them.
pub const TILE_H: f32 = 58.0;
pub const TILE_GAP: f32 = 8.0;
/// A footer button, and a square icon button.
pub const BUTTON_H: f32 = 28.0;
pub const ICON_BUTTON: f32 = 32.0;

/// What a control does when it is used.
///
/// The chrome owns `Settings`; everything else is the content's, as a number
/// it interprets, so the vocabulary need not grow a variant for every row of
/// every module.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Id {
    None,
    Settings,
    Custom(u32),
}

/// A color role, resolved against the palette and the module's accent at
/// draw time.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Ink {
    Primary,
    Secondary,
    /// Glyphs only, never words (docs/ui-design.md, 2.1).
    Tertiary,
    /// The module's accent, pushed until it reads as text.
    Accent,
    Caution,
    Critical,
    Custom(Color),
}

/// The type ramp, from docs/ui-design.md section 3 plus the two display
/// sizes the flyouts use for a headline reading.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Style {
    /// 12/16.
    Caption,
    /// 12/16 semibold: section labels, chip text.
    CaptionStrong,
    /// 14/20.
    Body,
    /// 14/20 semibold.
    BodyStrong,
    /// 20/28 semibold: a page's title.
    Subtitle,
    /// 28/36 semibold: the hero header's value.
    Title,
    /// Display semibold at a size of the content's choosing.
    Display(f32),
    /// The icon face at a size, for a glyph drawn as text.
    Icon(f32),
}

impl Style {
    pub fn size(self) -> f32 {
        match self {
            Style::Caption | Style::CaptionStrong => 12.0,
            Style::Body | Style::BodyStrong => 14.0,
            Style::Subtitle => 20.0,
            Style::Title => 28.0,
            Style::Display(size) | Style::Icon(size) => size,
        }
    }

    /// The line box, which is what rows are sized from.
    pub fn line(self) -> f32 {
        match self {
            Style::Caption | Style::CaptionStrong => 16.0,
            Style::Body | Style::BodyStrong => 20.0,
            Style::Subtitle => 28.0,
            Style::Title => 36.0,
            Style::Display(size) => (size * 1.3).round(),
            Style::Icon(size) => size,
        }
    }

    pub fn weight(self) -> i32 {
        match self {
            Style::Caption | Style::Body | Style::Icon(_) => 400,
            _ => 600,
        }
    }

    pub fn face(self) -> Face {
        match self {
            Style::Caption | Style::CaptionStrong => Face::Small,
            Style::Body | Style::BodyStrong => Face::Text,
            Style::Subtitle | Style::Title | Style::Display(_) => Face::Display,
            Style::Icon(_) => Face::Icons,
        }
    }

    /// The settings window's named style this is, when it is one, so that
    /// its ellipsis and wrapping text paths can be reused.
    pub fn ramp(self) -> Option<TextStyle> {
        match self {
            Style::Caption => Some(TextStyle::Caption),
            Style::Body => Some(TextStyle::Body),
            Style::BodyStrong => Some(TextStyle::BodyStrong),
            Style::Subtitle => Some(TextStyle::Subtitle),
            _ => None,
        }
    }
}

/// Two-color accent used for a module's tiles, graphs and highlights.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Accent {
    pub primary: Color,
    pub secondary: Color,
}

impl Accent {
    /// The module's signature pair, from ModuleAccent.signature(for:).
    ///
    /// The primary is the same value theme.rs holds for the settings
    /// window's tiles, taken from there so the two cannot drift; the
    /// secondary is the Mac table's.
    pub fn signature(module: ModuleId) -> Accent {
        let secondary = match module {
            ModuleId::Cpu => Color(0x22D3EE),
            ModuleId::Gpu => Color(0xEC4899),
            ModuleId::Memory => Color(0xA78BFA),
            ModuleId::Disks => Color(0x4ADE80),
            ModuleId::Network => Color(0x34D399),
            ModuleId::Sensors => Color(0xEF4444),
            ModuleId::Weather => Color(0xFBBF24),
        };
        Accent { primary: theme::module_color(module), secondary }
    }
}

/// The panel's neutrals, resolved for one appearance.
///
/// Opaque values throughout, for the reason theme.rs gives: GDI has no alpha
/// to composite with, so every "ink at 6% over the card" the design names is
/// worked out once here and painted as one solid color.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub theme: Theme,
    /// The panel's ground: surface.flyout, the solid fallback for acrylic.
    pub ground: Color,
    pub card: Color,
    pub card_stroke: Color,
    /// The inset plate inside a card, for graphs and grids.
    pub plate: Color,
    /// A row under the pointer.
    pub row_hover: Color,
    /// The track a capsule bar sits on.
    pub track: Color,
    pub divider: Color,
}

impl Palette {
    pub fn resolve(theme: Theme) -> Palette {
        let light = theme.light;
        let ground = if light { Color(0xF2F2F2) } else { Color(0x2C2C2C) };
        // docs/ui-design.md section 10: cards are surface.card at 70% in
        // light and white at 6% in dark, over the panel's ground.
        let card =
            if light { theme.surface_card.over(ground, 0.70) } else { WHITE.over(ground, 0.06) };
        let ink = theme.text_primary;
        Palette {
            ground,
            card,
            card_stroke: if light { Color(0xE3E3E3) } else { Color(0x454545) },
            plate: ink.over(card, 0.045),
            row_hover: ink.over(card, 0.06),
            track: ink.over(card, 0.08),
            divider: theme.stroke_divider,
            theme,
        }
    }

    /// A card washed with a module color at 10%, as GlassCard's tint does.
    pub fn tinted_card(&self, tint: Color) -> Color {
        tint.over(self.card, 0.10)
    }

    /// The capsule behind a chip: its color at 14% over the card.
    pub fn chip(&self, color: Color) -> Color {
        color.over(self.card, 0.14)
    }

    /// A chosen row or tile: the accent at 16% over the card.
    pub fn selected(&self, accent: Color) -> Color {
        accent.over(self.card, 0.16)
    }

    /// An accent as words on a card, pushed toward the text ink until it
    /// reads at 4.5:1, which sky blue on a white card does not.
    pub fn accent_text(&self, accent: Color) -> Color {
        let mut amount = if self.theme.light { 0.15 } else { 0.10 };
        for _ in 0..8 {
            let candidate =
                if self.theme.light { accent.darken(amount) } else { accent.lighten(amount) };
            if candidate.contrast(self.card) >= 4.5 {
                return candidate;
            }
            amount += 0.10;
        }
        self.theme.text_primary
    }

    pub fn ink(&self, ink: Ink, accent: Accent) -> Color {
        match ink {
            Ink::Primary => self.theme.text_primary,
            Ink::Secondary => self.theme.text_secondary,
            Ink::Tertiary => self.theme.text_tertiary,
            Ink::Accent => self.accent_text(accent.primary),
            Ink::Caution => self.theme.status_caution,
            Ink::Critical => self.theme.status_critical,
            Ink::Custom(color) => color,
        }
    }
}

/// Measures text: (text, style, wrap width or 0) to (width, height) in DIPs.
pub type Measure<'m> = &'m dyn Fn(&str, Style, f32) -> (f32, f32);

/// A picture the vocabulary does not have, drawn by the module that needs it.
///
/// The chrome hands it a surface with the palette and the accent resolved
/// and the rectangle it was laid out in. What it draws is its own business.
pub trait Painter: std::fmt::Debug + Send {
    fn paint(&self, surface: &mut super::paint::Surface, rect: Rect, hover: bool);
}

/// A normalized series, for AreaGraph, Sparkline and their two-series kin.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    /// Values from zero to one, oldest first.
    pub values: Vec<f32>,
    /// Where along the width each value sits, zero to one, when the samples
    /// are not evenly spaced. Otherwise spread evenly.
    pub positions: Option<Vec<f32>>,
    pub color: Color,
    pub color2: Color,
    pub line: f32,
    pub marker: bool,
    pub glow: bool,
    pub grid: bool,
    /// Drawn hanging from the top rather than standing on the bottom, for
    /// the lower half of a mirrored pair.
    pub flipped: bool,
    /// The wash's opacity at the line and at the baseline.
    pub fill: (f32, f32),
    pub inset: f32,
}

impl Graph {
    /// AreaGraph: gridlines, a glowing line and a live-point marker.
    pub fn area(values: Vec<f32>, accent: Accent) -> Graph {
        Graph {
            values,
            positions: None,
            color: accent.primary,
            color2: accent.secondary,
            line: 1.75,
            marker: true,
            glow: true,
            grid: true,
            flipped: false,
            fill: (0.42, 0.03),
            inset: 3.0,
        }
    }

    /// Sparkline: a minimal gradient line for a table row.
    pub fn sparkline(values: Vec<f32>, color: Color) -> Graph {
        Graph {
            values,
            positions: None,
            color,
            color2: color,
            line: 1.25,
            marker: false,
            glow: false,
            grid: false,
            flipped: false,
            fill: (0.35, 0.02),
            inset: 1.5,
        }
    }
}

#[derive(Debug)]
pub enum Kind {
    Text { text: String, style: Style, ink: Ink, align: Align },
    Wrapped { text: String, style: Style, ink: Ink, align: Align },
    Card { tint: Option<Color> },
    Plate,
    /// A row's background: washed when chosen or under the pointer.
    Row { selected: bool },
    /// A module tile: a flat square of the module's color with a glyph.
    /// A module's mark on a colored rounded square, from the Swift's
    /// `IconTile`: the accent's two colors as a diagonal gradient, a gloss
    /// down the face, a hairline edge, and the glyph in whichever of white
    /// or near-black stands off the fill.
    Tile { color: Color, color2: Color, glyph: char },
    Glyph { glyph: &'static str, size: f32, ink: Ink },
    /// A process's icon, a raw HICON the icon cache keeps alive, drawn at
    /// `size` DIPs square and centered in the rect.
    Icon { icon: isize, size: f32 },
    Chip { text: String, color: Color, glyph: Option<&'static str> },
    /// The track a capsule bar sits on.
    Track,
    /// A gradient capsule filling a fraction of its rectangle.
    Capsule { fraction: f32, color: Color, color2: Color, glow: bool },
    Graph(Graph),
    Divider,
    Button { text: String, glyph: Option<&'static str>, enabled: bool },
    IconButton { glyph: &'static str },
    Link { text: String, style: Style },
    Custom(Box<dyn Painter>),
    /// A picture wider than its rectangle, scrolled sideways under it: the
    /// painter is handed `content_w` by the rectangle's height, shifted left
    /// by `offset`, and clipped to the rectangle. The panel owns the offset
    /// and moves it on a horizontal wheel, the wheel with Shift, or a drag;
    /// zones are in the picture's own coordinates and are hit through the
    /// offset.
    Scroll { content_w: f32, offset: f32, painter: Box<dyn Painter> },
}

#[derive(Debug)]
pub struct Element {
    pub id: Id,
    pub rect: Rect,
    pub kind: Kind,
    pub interactive: bool,
    /// Parts of the element that answer to their own id, for a picture with
    /// several things in it - the columns of a chart, say.
    pub zones: Vec<(Rect, Id)>,
}

/// The icon at a stat tile's corner.
#[derive(Debug)]
pub enum TileIcon {
    Glyph(&'static str),
    Custom(Box<dyn Painter>),
    None,
}

/// A stat tile: a symbol, a caption, and a value.
#[derive(Debug)]
pub struct Tile {
    pub icon: TileIcon,
    pub label: String,
    pub value: String,
    pub tint: Ink,
}

/// Lays elements out top to bottom in a column.
pub struct Builder<'m> {
    pub elements: Vec<Element>,
    x: f32,
    width: f32,
    y: f32,
    measure: Measure<'m>,
}

impl<'m> Builder<'m> {
    pub fn new(x: f32, y: f32, width: f32, measure: Measure<'m>) -> Builder<'m> {
        Builder { elements: Vec::new(), x, width, y, measure }
    }

    pub fn x(&self) -> f32 {
        self.x
    }

    pub fn y(&self) -> f32 {
        self.y
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Where content inside a card starts, and how wide it is.
    pub fn inner_x(&self) -> f32 {
        self.x + CARD_PAD
    }

    pub fn inner_w(&self) -> f32 {
        self.width - 2.0 * CARD_PAD
    }

    pub fn advance(&mut self, dy: f32) {
        self.y += dy;
    }

    pub fn push(&mut self, element: Element) {
        self.elements.push(element);
    }

    pub fn passive(&mut self, rect: Rect, kind: Kind) {
        self.elements.push(Element { id: Id::None, rect, kind, interactive: false, zones: Vec::new() });
    }

    pub fn control(&mut self, id: Id, rect: Rect, kind: Kind) {
        self.elements.push(Element { id, rect, kind, interactive: true, zones: Vec::new() });
    }

    pub fn text(&mut self, rect: Rect, text: &str, style: Style, ink: Ink, align: Align) {
        self.passive(rect, Kind::Text { text: text.to_string(), style, ink, align });
    }

    /// The width of one line of text.
    pub fn text_width(&self, text: &str, style: Style) -> f32 {
        (self.measure)(text, style, 0.0).0
    }

    /// Wrapped text at the column's current position, as tall as it needs.
    pub fn wrapped(&mut self, text: &str, style: Style, ink: Ink, align: Align) -> f32 {
        self.wrapped_in(self.inner_x(), self.inner_w(), text, style, ink, align)
    }

    /// Wrapped text in an explicit column.
    pub fn wrapped_in(&mut self, x: f32, width: f32, text: &str, style: Style, ink: Ink, align: Align) -> f32 {
        let (_, height) = (self.measure)(text, style, width);
        let height = height.max(style.line());
        let rect = Rect::new(x, self.y, width, height);
        self.passive(rect, Kind::Wrapped { text: text.to_string(), style, ink, align });
        self.y += height;
        height
    }

    /// Module header: icon tile, title, subtitle, and a headline value in
    /// the accent, from HeroHeader.
    pub fn hero_header(
        &mut self,
        tile: (Color, char),
        title: &str,
        subtitle: Option<&str>,
        value: Option<&str>,
        accent: Accent,
    ) {
        let size = 36.0;
        let top = self.y;
        let height = size + 4.0;
        let tile_rect = Rect::new(self.x + 2.0, top + 2.0, size, size);
        self.passive(tile_rect, Kind::Tile { color: tile.0, color2: accent.secondary, glyph: tile.1 });
        let value_w = value.map(|value| self.text_width(value, Style::Title)).unwrap_or(0.0);
        let text_x = tile_rect.right() + 12.0;
        let text_w = (self.right() - 2.0 - value_w - 8.0 - text_x).max(24.0);
        match subtitle {
            Some(subtitle) => {
                self.text(Rect::new(text_x, top, text_w, 20.0), title, Style::BodyStrong, Ink::Primary, Align::Left);
                self.text(
                    Rect::new(text_x, top + 20.0, text_w, 16.0),
                    subtitle,
                    Style::Caption,
                    Ink::Secondary,
                    Align::Left,
                );
            }
            None => self.text(Rect::new(text_x, top, text_w, height), title, Style::BodyStrong, Ink::Primary, Align::Left),
        }
        if let Some(value) = value {
            self.text(
                Rect::new(self.right() - 2.0 - value_w, top + (height - 36.0) / 2.0, value_w, 36.0),
                value,
                Style::Title,
                Ink::Custom(accent.primary),
                Align::Right,
            );
        }
        self.y = top + height + CARD_GAP;
    }

    /// Starts a card, optionally tinted. Content goes inside until `card_end`.
    pub fn card_begin(&mut self, tint: Option<Color>) -> usize {
        let index = self.elements.len();
        self.passive(Rect::new(self.x, self.y, self.width, 0.0), Kind::Card { tint });
        self.y += CARD_PAD;
        index
    }

    pub fn card_end(&mut self, index: usize) {
        self.y += CARD_PAD;
        let top = self.elements[index].rect.y;
        self.elements[index].rect.h = self.y - top;
        self.y += CARD_GAP;
    }

    /// A small-caps section label. Returns the room to its right, for a chip
    /// or a button that trails it.
    pub fn section_label(&mut self, text: &str) -> Rect {
        let rect = Rect::new(self.inner_x(), self.y, self.inner_w(), SECTION_LABEL_H);
        self.text(rect, &text.to_uppercase(), Style::CaptionStrong, Ink::Secondary, Align::Left);
        self.y += SECTION_LABEL_H + SECTION_GAP;
        let label_w = self.text_width(&text.to_uppercase(), Style::CaptionStrong) + 8.0;
        Rect::new(rect.x + label_w, rect.y - 2.0, (rect.w - label_w).max(0.0), SECTION_LABEL_H + 4.0)
    }

    /// A chip right-aligned in a rectangle, as a section label's trailing
    /// accessory.
    pub fn chip_at(&mut self, area: Rect, text: &str, color: Color, glyph: Option<&'static str>) {
        let width = self.text_width(text, Style::CaptionStrong) + 14.0 + if glyph.is_some() { 13.0 } else { 10.0 };
        let rect = Rect::new(area.right() - width, area.center_y() - 10.0, width, 20.0);
        self.passive(rect, Kind::Chip { text: text.to_string(), color, glyph });
    }

    /// Label and value on one line with an optional symbol, from MetricRow.
    pub fn metric_row(&mut self, glyph: Option<&'static str>, label: &str, value: &str, tint: Ink) {
        let row = Rect::new(self.inner_x(), self.y, self.inner_w(), ROW_H);
        let mut x = row.x + 6.0;
        if let Some(glyph) = glyph {
            self.passive(Rect::new(x, row.y, 16.0, row.h), Kind::Glyph { glyph, size: 12.0, ink: tint });
            x += 16.0 + 8.0;
        }
        let value_w = self.text_width(value, Style::Body).min(row.w / 2.0);
        let label_w = (row.right() - 6.0 - value_w - 10.0 - x).max(24.0);
        self.text(Rect::new(x, row.y, label_w, row.h), label, Style::Body, Ink::Secondary, Align::Left);
        self.text(Rect::new(row.right() - 6.0 - value_w, row.y, value_w, row.h), value, Style::Body, Ink::Primary, Align::Right);
        self.y += ROW_H;
    }

    /// A grid of label and value lines, from WeatherDetailGrid.
    pub fn kv_rows(&mut self, rows: &[(String, String)]) {
        self.kv_rows_in(self.inner_x(), self.inner_w(), rows);
    }

    /// The same grid in an explicit column, for rows inside a plate.
    pub fn kv_rows_in(&mut self, x: f32, width: f32, rows: &[(String, String)]) {
        for (label, value) in rows {
            let row = Rect::new(x, self.y, width, KV_ROW_H);
            let value_w = self.text_width(value, Style::Caption).min(row.w / 2.0);
            self.text(Rect::new(row.x, row.y, row.w - value_w - 12.0, row.h), label, Style::Caption, Ink::Secondary, Align::Left);
            self.text(Rect::new(row.right() - value_w, row.y, value_w, row.h), value, Style::Caption, Ink::Primary, Align::Right);
            self.y += KV_ROW_H;
        }
    }

    /// Stat tiles in two columns, from StatTile in a LazyVGrid.
    pub fn stat_tiles(&mut self, tiles: Vec<Tile>) {
        let width = (self.inner_w() - TILE_GAP) / 2.0;
        for (index, tile) in tiles.into_iter().enumerate() {
            let column = index % 2;
            if column == 0 && index > 0 {
                self.y += TILE_H + TILE_GAP;
            }
            let x = self.inner_x() + column as f32 * (width + TILE_GAP);
            let rect = Rect::new(x, self.y, width, TILE_H);
            self.passive(rect, Kind::Plate);
            let pad = 9.0;
            let mut label_x = x + pad;
            let icon = Rect::new(x + pad, rect.y + pad + 1.0, 14.0, 14.0);
            match tile.icon {
                TileIcon::Glyph(glyph) => {
                    self.passive(icon, Kind::Glyph { glyph, size: 12.0, ink: tile.tint });
                    label_x += 14.0 + 5.0;
                }
                TileIcon::Custom(painter) => {
                    self.passive(icon, Kind::Custom(painter));
                    label_x += 14.0 + 5.0;
                }
                TileIcon::None => {}
            }
            self.text(
                Rect::new(label_x, rect.y + pad, rect.right() - pad - label_x, 16.0),
                &tile.label,
                Style::Caption,
                Ink::Secondary,
                Align::Left,
            );
            self.text(
                Rect::new(x + pad, rect.y + pad + 16.0 + 4.0, width - 2.0 * pad, 20.0),
                &tile.value,
                Style::BodyStrong,
                Ink::Primary,
                Align::Left,
            );
        }
        self.y += TILE_H;
    }

    /// An inset plate `height` tall across the card, returning where it is.
    pub fn plate(&mut self, height: f32) -> Rect {
        let rect = Rect::new(self.inner_x(), self.y, self.inner_w(), height);
        self.passive(rect, Kind::Plate);
        self.y += height;
        rect
    }

    /// A graph on a plate.
    pub fn graph(&mut self, height: f32, graph: Graph) -> Rect {
        let rect = self.plate(height);
        self.passive(rect, Kind::Graph(graph));
        rect
    }

    /// A capsule bar across the card.
    pub fn capsule(&mut self, fraction: f32, accent: Accent, height: f32) {
        let rect = Rect::new(self.inner_x(), self.y, self.inner_w(), height);
        self.passive(rect, Kind::Track);
        self.passive(rect, Kind::Capsule { fraction, color: accent.primary, color2: accent.secondary, glow: true });
        self.y += height;
    }

    /// A hairline across the card.
    pub fn divider(&mut self) {
        self.passive(Rect::new(self.inner_x(), self.y, self.inner_w(), 1.0), Kind::Divider);
        self.y += 1.0;
    }

    pub fn gap(&mut self, dy: f32) {
        self.y += dy;
    }

    /// A caption in the secondary ink, wrapped.
    pub fn caption(&mut self, text: &str) {
        self.wrapped(text, Style::Caption, Ink::Secondary, Align::Left);
    }

    /// The empty state, from ContentUnavailableView: a glyph, a title, and a
    /// sentence, centered.
    pub fn unavailable(&mut self, glyph: &'static str, title: &str, description: &str) {
        self.y += 12.0;
        self.passive(Rect::new(self.inner_x(), self.y, self.inner_w(), 36.0), Kind::Glyph { glyph, size: 28.0, ink: Ink::Tertiary });
        self.y += 36.0 + 8.0;
        self.text(Rect::new(self.inner_x(), self.y, self.inner_w(), 20.0), title, Style::BodyStrong, Ink::Primary, Align::Center);
        self.y += 20.0 + 4.0;
        self.wrapped(description, Style::Caption, Ink::Secondary, Align::Center);
        self.y += 12.0;
    }

    /// A link centered on its own line, for the attribution every provider
    /// is owed.
    pub fn centered_link(&mut self, id: Id, text: &str) {
        let width = self.text_width(text, Style::Caption);
        let rect = Rect::new(self.x + (self.width - width) / 2.0, self.y, width, 16.0);
        self.control(id, rect, Kind::Link { text: text.to_string(), style: Style::Caption });
        self.y += 16.0;
    }

    /// A row that highlights under the pointer and answers to an id, laid
    /// behind whatever the caller draws in it next.
    pub fn row_begin(&mut self, id: Id, height: f32, selected: bool) -> Rect {
        let rect = Rect::new(self.inner_x(), self.y, self.inner_w(), height);
        self.control(id, rect, Kind::Row { selected });
        rect
    }
}

/// The topmost element under a point, or one of its zones.
///
/// Last wins: elements are pushed back to front, so the last one containing
/// the point is the one drawn on top, and it is the one the pointer is on. A
/// zone inside an element answers before the element does, whether or not
/// the element itself is interactive.
pub fn hit(elements: &[Element], x: f32, y: f32) -> Option<Id> {
    for element in elements.iter().rev() {
        if !element.rect.contains(x, y) {
            continue;
        }
        // A scrolled picture's zones moved with it; the point is asked for
        // in the picture's own coordinates.
        let zone_x = match &element.kind {
            Kind::Scroll { offset, .. } => x + offset,
            _ => x,
        };
        if let Some((_, id)) = element.zones.iter().find(|(zone, _)| zone.contains(zone_x, y)) {
            return Some(*id);
        }
        if element.interactive {
            return Some(element.id);
        }
    }
    None
}

/// The topmost sideways-scrolling picture under a point that has more to
/// show than its rectangle does, by its id: what a horizontal wheel or a drag
/// at that point moves.
pub fn scroll_at(elements: &[Element], x: f32, y: f32) -> Option<Id> {
    elements.iter().rev().find_map(|element| match &element.kind {
        Kind::Scroll { content_w, .. } if element.rect.contains(x, y) && *content_w > element.rect.w => Some(element.id),
        _ => None,
    })
}

/// The interactive element with an id.
pub fn find(elements: &[Element], id: Id) -> Option<&Element> {
    elements.iter().find(|element| element.id == id && element.interactive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_ui::theme::DEFAULT_ACCENT;

    pub fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        // Seven DIPs a character at body size, scaled by the style; wrapped
        // by width when one is given.
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    #[test]
    fn the_hero_value_sits_clear_of_the_title_beside_it() {
        let mut b = Builder::new(12.0, 12.0, 356.0, &measure);
        b.hero_header((Color(0x3B82F6), '\u{E950}'), "Processor", Some("8 cores"), Some("100.0%"), Accent::signature(ModuleId::Cpu));
        let title = b.elements.iter().find(|e| matches!(&e.kind, Kind::Text { text, .. } if text == "Processor")).unwrap();
        let value = b.elements.iter().find(|e| matches!(&e.kind, Kind::Text { text, .. } if text == "100.0%")).unwrap();
        assert!(title.rect.right() <= value.rect.x, "title runs into the value");
        assert!((value.rect.right() - (12.0 + 356.0 - 2.0)).abs() < 1e-3, "value is right-aligned");
        assert!(b.y() > 12.0 + 36.0);
    }

    #[test]
    fn a_card_grows_to_hold_what_was_put_in_it() {
        let mut b = Builder::new(0.0, 0.0, 356.0, &measure);
        let card = b.card_begin(None);
        b.section_label("History");
        b.plate(72.0);
        b.card_end(card);
        let rect = b.elements[card].rect;
        assert!(matches!(b.elements[card].kind, Kind::Card { tint: None }));
        assert_eq!(rect.h, CARD_PAD + SECTION_LABEL_H + SECTION_GAP + 72.0 + CARD_PAD);
        assert_eq!(b.y(), rect.h + CARD_GAP);
    }

    #[test]
    fn a_section_label_leaves_its_right_hand_side_for_an_accessory() {
        let mut b = Builder::new(0.0, 0.0, 356.0, &measure);
        let trailing = b.section_label("Cores");
        assert!(trailing.x > CARD_PAD);
        assert!((trailing.right() - (356.0 - CARD_PAD)).abs() < 1e-3);
        b.chip_at(trailing, "8", Color(0x3B82F6), None);
        let chip = b.elements.last().unwrap();
        assert!(matches!(chip.kind, Kind::Chip { .. }));
        assert!((chip.rect.right() - trailing.right()).abs() < 1e-3);
    }

    #[test]
    fn stat_tiles_fill_two_columns_and_wrap_to_a_new_row() {
        let mut b = Builder::new(0.0, 0.0, 356.0, &measure);
        let tile = |label: &str| Tile { icon: TileIcon::Glyph("\u{E706}"), label: label.into(), value: "1".into(), tint: Ink::Secondary };
        b.stat_tiles(vec![tile("a"), tile("b"), tile("c")]);
        let plates: Vec<Rect> = b.elements.iter().filter(|e| matches!(e.kind, Kind::Plate)).map(|e| e.rect).collect();
        assert_eq!(plates.len(), 3);
        assert_eq!(plates[0].y, plates[1].y);
        assert!(plates[1].x > plates[0].right());
        assert!(plates[2].y > plates[0].bottom());
        assert_eq!(plates[2].x, plates[0].x);
        assert_eq!(b.y(), 2.0 * TILE_H + TILE_GAP);
    }

    #[test]
    fn a_metric_row_keeps_its_value_at_the_right_edge_without_crossing_the_label() {
        let mut b = Builder::new(0.0, 0.0, 356.0, &measure);
        b.metric_row(Some("\u{E706}"), "A label that is quite long indeed", "1.23 GB of 4.00 GB", Ink::Secondary);
        let texts: Vec<&Element> = b.elements.iter().filter(|e| matches!(e.kind, Kind::Text { .. })).collect();
        assert_eq!(texts.len(), 2);
        assert!(texts[0].rect.right() <= texts[1].rect.x);
        assert!((texts[1].rect.right() - (356.0 - CARD_PAD - 6.0)).abs() < 1e-3);
        assert_eq!(b.y(), ROW_H);
    }

    #[test]
    fn wrapped_text_takes_the_height_it_measures() {
        let mut b = Builder::new(0.0, 0.0, 100.0, &measure);
        let long = "x".repeat(60);
        let height = b.wrapped(&long, Style::Caption, Ink::Secondary, Align::Left);
        assert!(height > Style::Caption.line());
        assert_eq!(b.y(), height);
        let mut b = Builder::new(0.0, 0.0, 100.0, &measure);
        assert_eq!(b.wrapped("short", Style::Caption, Ink::Secondary, Align::Left), Style::Caption.line());
    }

    #[test]
    fn a_zone_inside_a_picture_answers_before_the_picture_and_the_row_behind_it() {
        let mut b = Builder::new(0.0, 0.0, 356.0, &measure);
        b.row_begin(Id::Custom(1), 100.0, false);
        b.push(Element {
            id: Id::None,
            rect: Rect::new(12.0, 0.0, 200.0, 100.0),
            kind: Kind::Plate,
            interactive: false,
            zones: vec![(Rect::new(12.0, 0.0, 100.0, 100.0), Id::Custom(7))],
        });
        assert_eq!(hit(&b.elements, 50.0, 50.0), Some(Id::Custom(7)));
        // Inside the picture but outside its zones falls through to the row.
        assert_eq!(hit(&b.elements, 150.0, 50.0), Some(Id::Custom(1)));
        assert_eq!(hit(&b.elements, 300.0, 50.0), Some(Id::Custom(1)));
        assert_eq!(hit(&b.elements, 300.0, 150.0), None);
    }

    /// A picture that draws nothing, for a scroller in a test.
    #[derive(Debug)]
    struct Blank;
    impl Painter for Blank {
        fn paint(&self, _surface: &mut super::super::paint::Surface, _rect: Rect, _hover: bool) {}
    }

    #[test]
    fn a_scrolled_pictures_zones_are_hit_where_they_have_scrolled_to() {
        let mut b = Builder::new(0.0, 0.0, 356.0, &measure);
        let plate = Rect::new(12.0, 0.0, 332.0, 100.0);
        // Twelve columns of 84 in a 1008-wide picture, scrolled 200 left.
        let zones = (0..12).map(|i| (Rect::new(12.0 + i as f32 * 84.0, 0.0, 84.0, 100.0), Id::Custom(10 + i))).collect();
        b.push(Element {
            id: Id::Custom(1),
            rect: plate,
            kind: Kind::Scroll { content_w: 1008.0, offset: 200.0, painter: Box::new(Blank) },
            interactive: false,
            zones,
        });
        // The plate's left edge shows the picture at x = 212, which is the
        // third column, not the first.
        assert_eq!(hit(&b.elements, 13.0, 50.0), Some(Id::Custom(12)));
        assert_eq!(hit(&b.elements, 13.0 + 84.0, 50.0), Some(Id::Custom(13)));
        assert_eq!(hit(&b.elements, 13.0, 150.0), None);
        // The scroller answers for the whole plate, and only while it has
        // more to show than the plate does.
        assert_eq!(scroll_at(&b.elements, 300.0, 50.0), Some(Id::Custom(1)));
        assert_eq!(scroll_at(&b.elements, 300.0, 150.0), None);
        if let Kind::Scroll { content_w, .. } = &mut b.elements[0].kind {
            *content_w = 300.0;
        }
        assert_eq!(scroll_at(&b.elements, 300.0, 50.0), None);
    }

    #[test]
    fn the_signature_accents_share_their_primary_with_the_settings_tiles() {
        for module in ModuleId::ALL {
            let accent = Accent::signature(module);
            assert_eq!(accent.primary, theme::module_color(module));
            assert_ne!(accent.primary, accent.secondary, "{module:?}");
        }
        assert_eq!(Accent::signature(ModuleId::Weather).secondary, Color(0xFBBF24));
    }

    #[test]
    fn the_palette_keeps_words_readable_on_its_cards_in_both_appearances() {
        for light in [false, true] {
            let palette = Palette::resolve(Theme::resolve(light, DEFAULT_ACCENT));
            assert!(palette.theme.text_primary.contrast(palette.card) >= 4.5, "light={light}");
            assert!(palette.theme.text_secondary.contrast(palette.card) >= 4.5, "light={light}");
            for module in ModuleId::ALL {
                let accent = Accent::signature(module);
                let text = palette.ink(Ink::Accent, accent);
                assert!(text.contrast(palette.card) >= 4.5, "{module:?} light={light}: {text:?}");
            }
        }
    }

    #[test]
    fn the_type_ramp_names_the_documented_sizes() {
        assert_eq!(Style::Caption.size(), 12.0);
        assert_eq!(Style::Body.line(), 20.0);
        assert_eq!(Style::Title.size(), 28.0);
        assert_eq!(Style::Title.weight(), 600);
        assert_eq!(Style::Display(40.0).size(), 40.0);
        assert_eq!(Style::CaptionStrong.ramp(), None);
        assert_eq!(Style::Body.ramp(), Some(TextStyle::Body));
    }
}
