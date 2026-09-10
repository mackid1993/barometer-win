// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Drawing for the flyouts: GDI for words, GDI+ for everything with a curve
// or a gradient in it.
//
// The settings window's gdi.rs draws rectangles and strings and keeps its
// GDI+ to corners and circles, which is all a settings page needs. A flyout
// is where the Mac app spends its color - gradient area graphs, glowing
// lines, a temperature range as a capsule that runs from blue to red - so
// this file adds the gradient brushes and the polyline pens those need, on
// top of the same Canvas and the same fonts. Text stays GDI and ClearType:
// every surface here is opaque, and the type must match the settings window
// beside it.
//
// `Surface` is what a module's own Painter gets: the canvas, the palette and
// the accent, and these helpers in DIPs. `draw` is the chrome's renderer for
// the shared vocabulary in ui.rs.

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL, HICON};
use windows_sys::Win32::Graphics::Gdi::{
    DrawTextW, GetTextExtentPoint32W, SelectObject, SetBkMode, SetTextCharacterExtra, SetTextColor,
    DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, HFONT, TRANSPARENT,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    DashStyleDash, FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFontFromDC,
    GdipCreateFromHDC, GdipCreateLineBrush, GdipCreatePath, GdipCreatePen1, GdipCreatePen2,
    GdipCreateSolidFill, GdipCreateStringFormat, GdipDeleteBrush, GdipDeleteFont,
    GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDeleteStringFormat, GdipDrawArc,
    GdipDrawEllipse, GdipDrawLine, GdipDrawLines, GdipDrawPath, GdipDrawString, GdipFillEllipse,
    GdipFillPath, GdipFillPie, GdipFillPolygon, GdipSetLinePresetBlend, GdipSetPenDashStyle,
    GdipSetPenEndCap, GdipSetPenLineJoin, GdipSetPenStartCap, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GdipSetStringFormatAlign, GdipSetStringFormatFlags,
    GdipSetStringFormatLineAlign, GdipSetTextRenderingHint, GpBrush, GpFont, GpGraphics, GpPath,
    GpPen, GpStringFormat, LineCapRound, LineJoinRound, PixelOffsetModeHalf, PointF, RectF,
    SmoothingModeAntiAlias, StringAlignmentCenter, StringAlignmentFar, StringAlignmentNear,
    StringFormatFlagsNoClip, StringFormatFlagsNoWrap, TextRenderingHintAntiAliasGridFit, UnitPixel,
    WrapModeTileFlipXY,
};

use super::ui::{Accent, Element, Graph, Id, Ink, Kind, Palette, Style, CARD_RADIUS, PLATE_RADIUS};
use crate::settings_ui::gdi::{wide, Align, Canvas, Face};
use crate::settings_ui::geometry::{hairline_px, snap_dip, Rect};
use crate::settings_ui::theme::{Color, BLACK, BRAND_GROUND, WHITE};
use crate::settings_ui::ui::scroll_thumb;

/// A device context at a scale, the colors in force, and the drawing helpers.
pub struct Surface<'c, 'f> {
    pub canvas: &'c mut Canvas<'f>,
    pub palette: &'c Palette,
    pub accent: Accent,
}

/// GDI+ objects that release themselves.
struct Graphics(*mut GpGraphics);
impl Drop for Graphics {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: ours, deleted once.
            unsafe { GdipDeleteGraphics(self.0) };
        }
    }
}
struct Brush(*mut GpBrush);
impl Drop for Brush {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: ours, deleted once.
            unsafe { GdipDeleteBrush(self.0) };
        }
    }
}
struct Pen(*mut GpPen);
impl Drop for Pen {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: ours, deleted once.
            unsafe { GdipDeletePen(self.0) };
        }
    }
}
struct Path(*mut GpPath);
impl Drop for Path {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: ours, deleted once.
            unsafe { GdipDeletePath(self.0) };
        }
    }
}

fn solid(color: Color, alpha: u8) -> Brush {
    let mut brush = std::ptr::null_mut();
    // SAFETY: an out pointer for a brush the wrapper deletes.
    unsafe { GdipCreateSolidFill(color.argb(alpha), &mut brush) };
    Brush(brush as *mut GpBrush)
}

/// A gradient from one point to another, in pixels.
fn linear(from: (f32, f32), to: (f32, f32), start: (Color, u8), end: (Color, u8)) -> Brush {
    let p1 = PointF { X: from.0, Y: from.1 };
    let p2 = PointF { X: to.0, Y: to.1 };
    let mut brush = std::ptr::null_mut();
    // SAFETY: two points on the stack and an out pointer the wrapper deletes.
    // TileFlipXY so a pixel past either end holds that end's color rather
    // than wrapping round to the other one, which a rounded corner's
    // antialiasing will otherwise sample.
    unsafe {
        GdipCreateLineBrush(&p1, &p2, start.0.argb(start.1), end.0.argb(end.1), WrapModeTileFlipXY, &mut brush)
    };
    Brush(brush as *mut GpBrush)
}

/// A gradient through several stops, positions from zero to one.
fn linear_stops(from: (f32, f32), to: (f32, f32), stops: &[(f32, Color, u8)]) -> Brush {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return Brush(std::ptr::null_mut());
    };
    let brush = linear(from, to, (first.1, first.2), (last.1, last.2));
    if brush.0.is_null() || stops.len() < 3 {
        return brush;
    }
    // GDI+ insists the preset blend begin at zero and end at one, and the
    // callers here build their stops that way.
    let colors: Vec<u32> = stops.iter().map(|(_, color, alpha)| color.argb(*alpha)).collect();
    let positions: Vec<f32> = stops.iter().map(|(position, _, _)| position.clamp(0.0, 1.0)).collect();
    // SAFETY: the brush is a line gradient, and both arrays outlive the call.
    unsafe {
        GdipSetLinePresetBlend(brush.0 as *mut _, colors.as_ptr(), positions.as_ptr(), colors.len() as i32);
    }
    brush
}

/// A pen with round ends and joins, which every stroke here wants.
fn pen(color: Color, alpha: u8, width_px: f32) -> Pen {
    let mut pen = std::ptr::null_mut();
    // SAFETY: an out pointer; the caps are set only once the pen exists.
    unsafe {
        GdipCreatePen1(color.argb(alpha), width_px, UnitPixel, &mut pen);
        round(pen);
    }
    Pen(pen)
}

/// A pen that draws with a brush, for a stroke that changes color along its
/// length.
fn brush_pen(brush: &Brush, width_px: f32) -> Pen {
    let mut pen = std::ptr::null_mut();
    // SAFETY: the brush is live for the pen's life.
    unsafe {
        GdipCreatePen2(brush.0, width_px, UnitPixel, &mut pen);
        round(pen);
    }
    Pen(pen)
}

unsafe fn round(pen: *mut GpPen) {
    if !pen.is_null() {
        GdipSetPenStartCap(pen, LineCapRound);
        GdipSetPenEndCap(pen, LineCapRound);
        GdipSetPenLineJoin(pen, LineJoinRound);
    }
}

/// A rounded rectangle path in pixels.
fn round_path(left: f32, top: f32, width: f32, height: f32, radius: f32) -> Path {
    let mut path: *mut GpPath = std::ptr::null_mut();
    let r = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let d = r * 2.0;
    // SAFETY: a fresh path, closed before use, deleted by the wrapper.
    unsafe {
        GdipCreatePath(FillModeAlternate, &mut path);
        if path.is_null() {
            return Path(path);
        }
        if r <= 0.0 {
            GdipAddPathArc(path, left, top, 0.0, 0.0, 0.0, 0.0);
        }
        GdipAddPathArc(path, left, top, d, d, 180.0, 90.0);
        GdipAddPathArc(path, left + width - d, top, d, d, 270.0, 90.0);
        GdipAddPathArc(path, left + width - d, top + height - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, left, top + height - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
    }
    Path(path)
}

fn points_px(points: &[(f32, f32)], scale: f32) -> Vec<PointF> {
    points.iter().map(|(x, y)| PointF { X: x * scale, Y: y * scale }).collect()
}

impl<'c, 'f> Surface<'c, 'f> {
    pub fn scale(&self) -> f32 {
        self.canvas.scale
    }

    /// A process icon at `size` DIPs, centered in the rect.
    pub fn icon(&mut self, rect: Rect, icon: isize, size: f32) {
        let scale = self.scale();
        let px = (size * scale).round() as i32;
        let x = ((rect.x + (rect.w - size) / 2.0) * scale).round() as i32;
        let y = ((rect.y + (rect.h - size) / 2.0) * scale).round() as i32;
        // SAFETY: a live device context, and an icon handle the cache keeps
        // alive for the life of the program.
        unsafe {
            DrawIconEx(self.canvas.dc, x, y, icon as HICON, px, px, 0, std::ptr::null_mut(), DI_NORMAL);
        }
    }

    /// A color role, resolved.
    pub fn ink(&self, ink: Ink) -> Color {
        self.palette.ink(ink, self.accent)
    }

    fn graphics(&self) -> Graphics {
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        // SAFETY: a live DC; the graphics object is released by the wrapper.
        unsafe {
            GdipCreateFromHDC(self.canvas.dc, &mut g);
            if !g.is_null() {
                GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
            }
        }
        Graphics(g)
    }

    fn font(&mut self, style: Style) -> HFONT {
        let px = self.canvas.px(style.size());
        self.canvas.fonts.face(style.face(), px, style.weight())
    }

    // ---- words --------------------------------------------------------------

    fn draw_text(&mut self, rect: Rect, text: &str, style: Style, color: Color, flags: u32) {
        // Nothing to draw. Not an optimization: DrawTextW reads the string
        // even when told it is zero long, and an empty Vec's pointer is
        // nowhere - the flyout thread died on exactly that, drawing an
        // engine the counters had given no name.
        if text.is_empty() {
            return;
        }
        let font = self.font(style);
        let px = rect.px(self.scale());
        let mut bounds = RECT { left: px.left, top: px.top, right: px.right, bottom: px.bottom };
        let units = wide(text);
        // The one all-capitals string in the panel is the section label, and
        // it is the one that wants its letters spaced: capitals set solid
        // read as shouting where the same word tracked out reads as a label.
        let tracked = style == Style::CaptionStrong
            && text.chars().any(char::is_alphabetic)
            && text == text.to_uppercase();
        let extra = if tracked { (0.9 * self.scale()).round() as i32 } else { 0 };
        // SAFETY: the DC is live, the font outlives the call, and the string
        // is passed with its length.
        unsafe {
            let previous = SelectObject(self.canvas.dc, font as _);
            SetBkMode(self.canvas.dc, TRANSPARENT as i32);
            SetTextColor(self.canvas.dc, color.colorref());
            SetTextCharacterExtra(self.canvas.dc, extra);
            DrawTextW(self.canvas.dc, units.as_ptr(), units.len() as i32, &mut bounds, flags);
            SetTextCharacterExtra(self.canvas.dc, 0);
            SelectObject(self.canvas.dc, previous);
        }
    }

    fn align_flag(align: Align) -> u32 {
        match align {
            Align::Left => DT_LEFT,
            Align::Center => DT_CENTER,
            Align::Right => DT_RIGHT,
        }
    }

    /// One line, vertically centered, cut with an ellipsis when it does not
    /// fit.
    pub fn text(&mut self, rect: Rect, text: &str, style: Style, color: Color, align: Align) {
        let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX | Surface::align_flag(align);
        self.draw_text(rect, text, style, color, flags);
    }

    /// Text wrapped to the rectangle's width, from its top.
    pub fn wrapped(&mut self, rect: Rect, text: &str, style: Style, color: Color, align: Align) {
        self.draw_text(rect, text, style, color, DT_WORDBREAK | DT_NOPREFIX | Surface::align_flag(align));
    }

    /// An icon-font glyph centered in a rectangle.
    pub fn glyph(&mut self, rect: Rect, glyph: &str, size: f32, color: Color) {
        self.canvas.glyph(rect, glyph, size, color);
    }

    /// One glyph at a size in the icon face, at a position rather than
    /// centered, for marks composed of two glyphs at set offsets.
    pub fn glyph_at(&mut self, x: f32, y: f32, glyph: char, size: f32, color: Color) {
        let px = self.canvas.px(size);
        let font = self.canvas.fonts.face(Face::Icons, px, 400);
        let units = [glyph as u16];
        let scale = self.scale();
        self.canvas.font_text((x * scale).round() as i32, (y * scale).round() as i32, &units, font, color);
    }

    /// Text drawn through GDI+, so it can carry an alpha and a shadow.
    ///
    /// Only where words sit on a gradient and want a shadow under them, as
    /// on the weather's sky card: GDI's text is sharper, and everywhere else
    /// it is what is used.
    #[allow(clippy::too_many_arguments)]
    pub fn text_alpha(
        &mut self,
        rect: Rect,
        text: &str,
        style: Style,
        color: Color,
        alpha: u8,
        align: Align,
        shadow: Option<(f32, u8)>,
    ) {
        if text.is_empty() {
            return;
        }
        let font = self.font(style);
        let scale = self.scale();
        let units = wide(text);
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        // SAFETY: every object below is created here, checked, and deleted
        // before returning; the DC outlives the call.
        unsafe {
            let previous = SelectObject(self.canvas.dc, font as _);
            let mut gp_font: *mut GpFont = std::ptr::null_mut();
            GdipCreateFontFromDC(self.canvas.dc, &mut gp_font);
            SelectObject(self.canvas.dc, previous);
            if gp_font.is_null() {
                return;
            }
            let mut format: *mut GpStringFormat = std::ptr::null_mut();
            GdipCreateStringFormat(0, 0, &mut format);
            if format.is_null() {
                GdipDeleteFont(gp_font);
                return;
            }
            GdipSetStringFormatAlign(
                format,
                match align {
                    Align::Left => StringAlignmentNear,
                    Align::Center => StringAlignmentCenter,
                    Align::Right => StringAlignmentFar,
                },
            );
            GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
            GdipSetStringFormatFlags(format, StringFormatFlagsNoWrap | StringFormatFlagsNoClip);
            GdipSetTextRenderingHint(g.0, TextRenderingHintAntiAliasGridFit);
            let layout = |dy: f32| RectF {
                X: rect.x * scale,
                Y: (rect.y + dy) * scale,
                Width: rect.w * scale,
                Height: rect.h * scale,
            };
            if let Some((dy, shade)) = shadow {
                let brush = solid(BLACK, shade);
                let bounds = layout(dy);
                GdipDrawString(g.0, units.as_ptr(), units.len() as i32, gp_font, &bounds, format, brush.0);
            }
            let brush = solid(color, alpha);
            let bounds = layout(0.0);
            GdipDrawString(g.0, units.as_ptr(), units.len() as i32, gp_font, &bounds, format, brush.0);
            GdipDeleteStringFormat(format);
            GdipDeleteFont(gp_font);
        }
    }

    /// The width and height of text: one line, or wrapped to a width.
    pub fn measure(&mut self, text: &str, style: Style, width: f32) -> (f32, f32) {
        // An empty string is no width and a line tall, and is not handed to
        // the calls below: see `text` for why.
        if text.is_empty() {
            let (_, line) = self.measure(" ", style, 0.0);
            return (0.0, line);
        }
        let font = self.font(style);
        let units = wide(text);
        let scale = self.scale();
        // SAFETY: the DC is live and the string is passed with its length;
        // DT_CALCRECT only writes the rect.
        unsafe {
            let previous = SelectObject(self.canvas.dc, font as _);
            let result = if width > 0.0 {
                let mut bounds = RECT { left: 0, top: 0, right: (width * scale).round() as i32, bottom: 0 };
                let height = DrawTextW(
                    self.canvas.dc,
                    units.as_ptr(),
                    units.len() as i32,
                    &mut bounds,
                    DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
                );
                (width, height as f32 / scale)
            } else {
                let mut size = windows_sys::Win32::Foundation::SIZE { cx: 0, cy: 0 };
                GetTextExtentPoint32W(self.canvas.dc, units.as_ptr(), units.len() as i32, &mut size);
                (size.cx as f32 / scale, size.cy as f32 / scale)
            };
            SelectObject(self.canvas.dc, previous);
            result
        }
    }

    // ---- shapes -------------------------------------------------------------

    pub fn fill_round(&self, rect: Rect, radius: f32, color: Color) {
        self.canvas.fill_round(rect, radius, color);
    }

    pub fn fill_round_alpha(&self, rect: Rect, radius: f32, color: Color, alpha: u8) {
        self.canvas.fill_round_alpha(rect, radius, color, alpha);
    }

    pub fn stroke_round(&self, rect: Rect, radius: f32, color: Color) {
        self.canvas.stroke_round(rect, radius, color);
    }

    /// A rounded fill with a gradient at an angle: zero is left to right,
    /// ninety top to bottom, forty-five the diagonal the Mac's cards use.
    pub fn fill_round_gradient(&self, rect: Rect, radius: f32, start: (Color, u8), end: (Color, u8), angle: f32) {
        let scale = self.scale();
        let px = rect.px(scale);
        if px.width() <= 0 || px.height() <= 0 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let (w, h) = (px.width() as f32, px.height() as f32);
        let (cx, cy) = (px.left as f32 + w / 2.0, px.top as f32 + h / 2.0);
        let (sin, cos) = angle.to_radians().sin_cos();
        // Half the diagonal along the gradient's direction, so the ends of
        // the gradient land on the corners it runs between.
        let reach = (w * cos.abs() + h * sin.abs()) / 2.0;
        let brush = linear((cx - reach * cos, cy - reach * sin), (cx + reach * cos, cy + reach * sin), start, end);
        let path = round_path(px.left as f32, px.top as f32, w, h, radius * scale);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipFillPath(g.0, brush.0, path.0) };
    }

    /// A rounded fill with a gradient through several stops, left to right
    /// or top to bottom.
    pub fn fill_round_stops(&self, rect: Rect, radius: f32, stops: &[(f32, Color, u8)], vertical: bool) {
        let scale = self.scale();
        let px = rect.px(scale);
        if px.width() <= 0 || px.height() <= 0 || stops.is_empty() {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let (from, to) = if vertical {
            ((px.left as f32, px.top as f32), (px.left as f32, px.bottom as f32))
        } else {
            ((px.left as f32, px.top as f32), (px.right as f32, px.top as f32))
        };
        let brush = linear_stops(from, to, stops);
        let path = round_path(px.left as f32, px.top as f32, px.width() as f32, px.height() as f32, radius * scale);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipFillPath(g.0, brush.0, path.0) };
    }

    /// A stroke of a width in DIPs and an alpha, inside a rounded rectangle.
    pub fn stroke_round_alpha(&self, rect: Rect, radius: f32, color: Color, alpha: u8, width_dip: f32) {
        let scale = self.scale();
        let px = rect.px(scale);
        if px.width() <= 0 || px.height() <= 0 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let width = (width_dip * scale).max(1.0);
        let half = width / 2.0;
        let path = round_path(
            px.left as f32 + half,
            px.top as f32 + half,
            px.width() as f32 - width,
            px.height() as f32 - width,
            radius * scale - half,
        );
        let pen = pen(color, alpha, width);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawPath(g.0, pen.0, path.0) };
    }

    /// A line through points, in one color.
    pub fn polyline(&self, points: &[(f32, f32)], width_dip: f32, color: Color, alpha: u8) {
        if points.len() < 2 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let pen = pen(color, alpha, width_dip * self.scale());
        let points = points_px(points, self.scale());
        // SAFETY: live objects; the points outlive the call.
        unsafe { GdipDrawLines(g.0, pen.0, points.as_ptr(), points.len() as i32) };
    }

    /// A line through points that runs from one color at its left to
    /// another at its right.
    pub fn polyline_gradient(&self, points: &[(f32, f32)], width_dip: f32, start: Color, end: Color, alpha: u8) {
        if points.len() < 2 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let scale = self.scale();
        let left = points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min) * scale;
        let right = points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max) * scale;
        let brush = linear((left, 0.0), (right.max(left + 1.0), 0.0), (start, alpha), (end, alpha));
        let pen = brush_pen(&brush, width_dip * scale);
        let points = points_px(points, scale);
        // SAFETY: live objects; the points outlive the call.
        unsafe { GdipDrawLines(g.0, pen.0, points.as_ptr(), points.len() as i32) };
    }

    pub fn polygon(&self, points: &[(f32, f32)], color: Color, alpha: u8) {
        if points.len() < 3 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let brush = solid(color, alpha);
        let points = points_px(points, self.scale());
        // SAFETY: live objects; the points outlive the call.
        unsafe { GdipFillPolygon(g.0, brush.0, points.as_ptr(), points.len() as i32, FillModeAlternate) };
    }

    /// A polygon washed from one color at `top` to another at `bottom`, in
    /// DIPs, which is how an area under a graph line fades out.
    pub fn polygon_vertical(&self, points: &[(f32, f32)], start: (Color, u8), end: (Color, u8), top: f32, bottom: f32) {
        if points.len() < 3 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let scale = self.scale();
        let brush = linear((0.0, top * scale), (0.0, bottom.max(top + 0.5) * scale), start, end);
        let points = points_px(points, scale);
        // SAFETY: live objects; the points outlive the call.
        unsafe { GdipFillPolygon(g.0, brush.0, points.as_ptr(), points.len() as i32, FillModeAlternate) };
    }

    pub fn ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32, color: Color, alpha: u8) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let s = self.scale();
        let brush = solid(color, alpha);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipFillEllipse(g.0, brush.0, (cx - rx) * s, (cy - ry) * s, 2.0 * rx * s, 2.0 * ry * s) };
    }

    pub fn ring(&self, cx: f32, cy: f32, rx: f32, ry: f32, width_dip: f32, color: Color, alpha: u8) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let s = self.scale();
        let pen = pen(color, alpha, (width_dip * s).max(hairline_px(s) as f32));
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawEllipse(g.0, pen.0, (cx - rx) * s, (cy - ry) * s, 2.0 * rx * s, 2.0 * ry * s) };
    }

    /// A wedge of an ellipse, angles in degrees clockwise from three o'clock.
    #[allow(clippy::too_many_arguments)]
    pub fn pie(&self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32, color: Color, alpha: u8) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let s = self.scale();
        let brush = solid(color, alpha);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipFillPie(g.0, brush.0, (cx - rx) * s, (cy - ry) * s, 2.0 * rx * s, 2.0 * ry * s, start, sweep) };
    }

    /// An arc of an ellipse, optionally dashed.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32, width_dip: f32, color: Color, alpha: u8, dashed: bool) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let s = self.scale();
        let pen = pen(color, alpha, (width_dip * s).max(1.0));
        // SAFETY: live objects owned by their wrappers.
        unsafe {
            if dashed {
                GdipSetPenDashStyle(pen.0, DashStyleDash);
            }
            GdipDrawArc(g.0, pen.0, (cx - rx) * s, (cy - ry) * s, 2.0 * rx * s, 2.0 * ry * s, start, sweep);
        }
    }

    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, width_dip: f32, color: Color, alpha: u8) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let s = self.scale();
        let pen = pen(color, alpha, (width_dip * s).max(1.0));
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawLine(g.0, pen.0, x1 * s, y1 * s, x2 * s, y2 * s) };
    }

    /// A horizontal hairline snapped to whole pixels.
    pub fn hline(&self, x1: f32, x2: f32, y: f32, color: Color) {
        self.canvas.hline(x1, x2, y, color);
    }

    /// A soft shadow under a rounded rectangle: three rings of black at a
    /// low alpha, stepping outward and downward, which is what a blur looks
    /// like at this size and costs three fills rather than a filter.
    pub fn soft_shadow(&self, rect: Rect, radius: f32) {
        for (spread, alpha) in [(3.0, 10u8), (2.0, 14), (1.0, 18)] {
            let shade = Rect::new(rect.x - spread, rect.y - spread + 3.0, rect.w + 2.0 * spread, rect.h + 2.0 * spread);
            self.fill_round_alpha(shade, radius + spread, BLACK, alpha);
        }
    }

    /// Restricts drawing to a rectangle until `unclip` is called.
    pub fn clip(&self, rect: Rect) -> i32 {
        self.canvas.clip(rect)
    }

    pub fn unclip(&self, token: i32) {
        self.canvas.unclip(token);
    }
}

/// A module's mark on its colored square, from `IconTile` in the Swift's
/// BarometerDesign.
///
/// Five layers, and each one earns its place: the accent's two colors on the
/// diagonal so the square is not a flat swatch, a gloss down the face that
/// makes it read as a raised key rather than a sticker, a hairline of white
/// to lift its edge off a dark card, and the glyph in whichever of white or
/// near-black stands off the fill - a light accent with a white glyph on it
/// measured 1.29 to 1 on the Mac, which is what the contrast test is for.
/// The shadow underneath is two rounded fills rather than a real blur: GDI+
/// has no cheap one, and at this size the difference is not visible.
fn draw_tile(surface: &mut Surface, rect: Rect, color: Color, color2: Color, glyph: char) {
    // The Swift's cornerRadius is `size * 0.28`.
    let radius = rect.w * 0.28;
    for (drop, alpha) in [(3.0, 14u8), (2.0, 20u8)] {
        let under = Rect::new(rect.x + 1.0, rect.y + drop, rect.w - 2.0, rect.h);
        surface.fill_round_alpha(under, radius, color, alpha);
    }
    surface.fill_round_gradient(rect, radius, (color, 255), (color2, 255), 45.0);
    // White at 35% down to 2%, as the Swift's gloss.
    surface.fill_round_gradient(rect, radius, (WHITE, 89), (WHITE, 5), 90.0);
    surface.stroke_round_alpha(rect, radius, WHITE, 71, 0.75);
    surface.canvas.text_face(
        rect,
        &glyph.to_string(),
        Face::Icons,
        rect.w * 0.5,
        600,
        tile_ink(color),
        Align::Center,
    );
}

/// The ink for a glyph on a tile of `color`: white wherever white reads at
/// 3:1, the brand ground where it does not (docs/ui-design.md, 2.5).
///
/// Not "whichever contrasts more": white is what every other tile on the
/// desktop draws its glyph in, and a set of tiles that switched ink on a
/// fraction would read as two kinds of tile.
fn tile_ink(color: Color) -> Color {
    if WHITE.contrast(color) >= 3.0 {
        WHITE
    } else {
        BRAND_GROUND
    }
}

/// Draws elements offset by (dx, dy), in order, skipping what is not visible.
pub fn draw(surface: &mut Surface, elements: &[Element], hover: Option<Id>, pressed: Option<Id>, dx: f32, dy: f32, visible: Rect) {
    for element in elements {
        let rect = element.rect.offset(dx, dy);
        if !rect.intersects(&visible) {
            continue;
        }
        let is_hover = element.interactive && hover == Some(element.id);
        let is_pressed = element.interactive && pressed == Some(element.id);
        let theme = &surface.palette.theme;
        match &element.kind {
            Kind::Text { text, style, ink, align } => {
                let color = surface.ink(*ink);
                surface.text(rect, text, *style, color, *align);
            }
            Kind::Wrapped { text, style, ink, align } => {
                let color = surface.ink(*ink);
                surface.wrapped(rect, text, *style, color, *align);
            }
            Kind::Card { tint } => {
                let fill = tint.map(|tint| surface.palette.tinted_card(tint)).unwrap_or(surface.palette.card);
                surface.fill_round(rect, CARD_RADIUS, fill);
                surface.stroke_round(rect, CARD_RADIUS, surface.palette.card_stroke);
            }
            Kind::Plate => surface.fill_round(rect, PLATE_RADIUS, surface.palette.plate),
            Kind::Row { selected } => {
                if *selected {
                    surface.fill_round(rect, PLATE_RADIUS, surface.palette.selected(surface.accent.primary));
                } else if is_pressed || is_hover {
                    surface.fill_round(rect, PLATE_RADIUS, surface.palette.row_hover);
                }
            }
            Kind::Tile { color, color2, glyph } => draw_tile(surface, rect, *color, *color2, *glyph),
            Kind::Glyph { glyph, size, ink } => {
                let color = surface.ink(*ink);
                surface.glyph(rect, glyph, *size, color);
            }
            Kind::Icon { icon, size } => surface.icon(rect, *icon, *size),
            Kind::Chip { text, color, glyph } => {
                surface.fill_round(rect, rect.h / 2.0, surface.palette.chip(*color));
                let mut x = rect.x + 7.0;
                match glyph {
                    Some(glyph) => {
                        surface.glyph(Rect::new(x, rect.y, 10.0, rect.h), glyph, 9.0, *color);
                        x += 13.0;
                    }
                    None => {
                        surface.ellipse(x + 3.0, rect.center_y(), 3.0, 3.0, *color, 255);
                        x += 10.0;
                    }
                }
                // The label is the text color, never the chip's own, which
                // on a 14% wash of itself the Mac measured at 1.28:1. It is
                // given the chip's right padding to draw into as well: the
                // chip was sized from a measurement in fractional DIPs and
                // the text is drawn in whole pixels, and a label a pixel
                // over its box is cut to "Idle 9..." rather than let run a
                // pixel into the padding.
                let ink = theme.text_primary;
                surface.text(Rect::new(x, rect.y, rect.right() - x, rect.h), text, Style::CaptionStrong, ink, Align::Left);
            }
            Kind::Track => surface.fill_round(rect, rect.h / 2.0, surface.palette.track),
            Kind::Capsule { fraction, color, color2, glow } => {
                let width = (rect.w * fraction.clamp(0.0, 1.0)).max(rect.h);
                let bar = Rect::new(rect.x, rect.y, width, rect.h);
                if *glow {
                    surface.fill_round_alpha(bar.inset(-2.0), bar.h / 2.0 + 2.0, *color, 45);
                    surface.fill_round_alpha(bar.inset(-1.0), bar.h / 2.0 + 1.0, *color, 60);
                }
                surface.fill_round_gradient(bar, bar.h / 2.0, (*color, 255), (*color2, 255), 0.0);
            }
            Kind::Graph(graph) => draw_graph(surface, rect, graph),
            Kind::Divider => surface.hline(rect.x, rect.right(), rect.y, surface.palette.divider),
            Kind::Button { text, glyph, enabled } => {
                let fill = if !enabled {
                    theme.control_disabled
                } else if is_pressed {
                    theme.control_pressed
                } else if is_hover {
                    theme.control_hover
                } else {
                    theme.control_rest
                };
                surface.fill_round(rect, 4.0, fill);
                surface.stroke_round(rect, 4.0, theme.stroke_control);
                let ink = if *enabled { theme.text_primary } else { theme.text_disabled };
                let (label_w, _) = surface.measure(text, Style::Body, 0.0);
                let glyph_w = if glyph.is_some() { 12.0 + 6.0 } else { 0.0 };
                let mut x = rect.x + (rect.w - label_w - glyph_w) / 2.0;
                if let Some(glyph) = glyph {
                    surface.glyph(Rect::new(x, rect.y, 12.0, rect.h), glyph, 12.0, ink);
                    x += glyph_w;
                }
                surface.text(Rect::new(x, rect.y, label_w + 2.0, rect.h), text, Style::Body, ink, Align::Left);
            }
            Kind::IconButton { glyph } => {
                if is_pressed || is_hover {
                    let fill = if is_pressed { theme.subtle_pressed } else { theme.subtle_hover };
                    surface.fill_round(rect, 4.0, fill);
                }
                surface.glyph(rect, glyph, 16.0, theme.text_secondary);
            }
            Kind::Link { text, style } => {
                let color = surface.ink(Ink::Accent);
                surface.text(rect, text, *style, color, Align::Left);
                if is_hover {
                    surface.hline(rect.x, rect.right(), rect.bottom() - 2.0, color);
                }
            }
            Kind::Custom(painter) => painter.paint(surface, rect, is_hover),
            Kind::Scroll { content_w, offset, painter } => {
                // Clipped to the plate: the picture is drawn at its full
                // width, shifted, and the plate shows the part under it.
                let clip = surface.clip(rect);
                // Snapped for the reason the page's own offset is: a drag
                // lands the chart on fractional DIPs, and the hour labels
                // are drawn in whole pixels while the plot behind them is
                // not, so unsnapped they walk against each other by a pixel
                // as the chart moves. The thumb below keeps the true offset:
                // it says where in the chart we are, not where it is drawn.
                let shift = snap_dip(*offset, surface.scale());
                painter.paint(surface, Rect::new(rect.x - shift, rect.y, *content_w, rect.h), is_hover);
                if let Some((x, w)) = scroll_thumb(rect.w, *content_w, *offset) {
                    // A thumb along the plate's bottom edge, like the panel's
                    // own down its right: a plate with more to its right has
                    // to look like it, or nobody scrolls it. Worked out for
                    // the plate's full width, then drawn on a track inset
                    // from its ends, so it reaches both ends exactly.
                    let track_w = rect.w - 2.0 * SCROLL_THUMB_INSET;
                    let thumb = Rect::new(
                        rect.x + SCROLL_THUMB_INSET + x * track_w / rect.w,
                        rect.bottom() - 5.0,
                        w * track_w / rect.w,
                        3.0,
                    );
                    surface.fill_round(thumb, 1.5, surface.palette.theme.stroke_strong);
                }
                surface.unclip(clip);
            }
        }
    }
}

/// How far the sideways scroller's thumb stays in from the plate's ends.
const SCROLL_THUMB_INSET: f32 = 6.0;

/// A normalized series on a plate, from NormalizedGraphSeries.
fn draw_graph(surface: &mut Surface, rect: Rect, graph: &Graph) {
    let h_inset = if graph.marker { 4.0 } else { 0.0 };
    let v_inset = graph.inset;
    let plot_w = rect.w - 2.0 * h_inset;
    let plot_h = rect.h - 2.0 * v_inset;
    let count = graph.values.len();
    if graph.grid {
        let ink = surface.palette.theme.text_primary;
        for fraction in [0.25, 0.5, 0.75] {
            let y = rect.y + rect.h * (1.0 - fraction);
            surface.arc(rect.x + rect.w / 2.0, y, rect.w / 2.0, 0.0, 180.0, 180.0, 0.5, ink, 24, true);
        }
    }
    if count < 2 {
        return;
    }
    let points: Vec<(f32, f32)> = graph
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let fraction = match &graph.positions {
                Some(positions) if positions.len() == count => positions[index].clamp(0.0, 1.0),
                _ => index as f32 / (count - 1) as f32,
            };
            let value = value.clamp(0.0, 1.0);
            let x = rect.x + h_inset + fraction * plot_w;
            let y = if graph.flipped {
                rect.y + v_inset + value * plot_h
            } else {
                rect.y + v_inset + (1.0 - value) * plot_h
            };
            (x, y)
        })
        .collect();
    let baseline = if graph.flipped { rect.y } else { rect.bottom() };
    let mut area = Vec::with_capacity(count + 2);
    area.push((points[0].0, baseline));
    area.extend(points.iter().copied());
    area.push((points[count - 1].0, baseline));
    let (top, bottom) = if graph.flipped { (rect.bottom(), rect.y) } else { (rect.y, rect.bottom()) };
    surface.polygon_vertical(
        &area,
        (graph.color, (graph.fill.0 * 255.0) as u8),
        (graph.color2, (graph.fill.1 * 255.0) as u8),
        top.min(bottom),
        top.max(bottom),
    );
    if graph.glow {
        // Two wider passes at low alpha stand in for the Mac's blur.
        surface.polyline(&points, graph.line + 3.0, graph.color, 40);
        surface.polyline(&points, graph.line + 1.5, graph.color, 70);
    }
    surface.polyline_gradient(&points, graph.line, graph.color, graph.color2, 255);
    if graph.marker {
        let (x, y) = points[count - 1];
        surface.ellipse(x, y, 5.0, 5.0, graph.color2, 72);
        surface.ellipse(x, y, 2.5, 2.5, graph.color2, 255);
    }
}
