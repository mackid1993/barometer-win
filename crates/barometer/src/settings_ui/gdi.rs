// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Drawing for the settings window: GDI for text, GDI+ for curves.
//
// Text is GDI. On the window's opaque surfaces ClearType is right, and GDI's
// ClearType is the same rasterizer the rest of the desktop's text goes
// through, so the window's type matches the caption above it.
//
// Every curve is GDI+. AGENTS.md reserves GDI+ for the weather marks on the
// strip, and the reason it gives is exactly the reason it is used here too:
// GDI has no antialiasing, and a 20-DIP toggle knob or a 4-DIP corner radius
// drawn aliased is a staircase that reads as 1998, which is the one thing the
// brief said this window must not look like. The strip's rule stands on the
// strip; this is a different surface with the same problem.
//
// Nothing here knows what a toggle is. It draws rectangles, circles and
// strings at a scale, and `ui.rs` composes those into controls.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateSolidBrush, DeleteObject, DrawTextW, FillRect,
    GetTextExtentPoint32W, IntersectClipRect, RestoreDC, SaveDC, SelectObject, SetBkMode,
    SetTextColor, TextOutW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_PATH_ELLIPSIS, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HDC, HFONT, LOGFONTW, OUT_TT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArc, GdipAddPathLine, GdipClosePathFigure, GdipCreateFromHDC,
    GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics,
    GdipDeletePath, GdipDeletePen, GdipDrawEllipse, GdipDrawPath, GdipFillEllipse, GdipFillPath,
    GdipSetPenEndCap, GdipSetPenLineJoin, GdipSetPenStartCap, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GdipStartPathFigure, GpBrush, GpGraphics, GpPath, GpPen, LineCapRound,
    LineJoinRound, PixelOffsetModeHalf, SmoothingModeAntiAlias, UnitPixel,
};

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, LoadImageW, DI_NORMAL, HICON, IMAGE_ICON, LR_DEFAULTCOLOR,
};

use super::geometry::{hairline_px, to_px, Rect};
use super::theme::Color;

/// UTF-16 without a terminator, for the text calls that take a length.
pub fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().collect()
}

/// UTF-16 with a terminator, for the calls that want one.
pub fn wide_nul(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// The faces the interface draws in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Face {
    /// Segoe UI Variable Text: body sizes.
    Text,
    /// Segoe UI Variable Display: titles.
    Display,
    /// Segoe UI Variable Small: captions.
    Small,
    /// Segoe Fluent Icons.
    Icons,
}

/// The GDI family name for a face at a weight.
///
/// GDI knows nothing of variable fonts. Windows registers each optical size
/// and weight of Segoe UI Variable as its own family, and asking for weight
/// 600 on "Segoe UI Variable Text" gets a synthetic bold - the regular face
/// smeared a pixel to the right - rather than the semibold that was drawn.
/// The names below are the ones GDI actually lists, including the two the
/// 31-character family limit truncates. Without the variable family at all
/// (Windows 10) it is plain Segoe UI, whose semibold GDI does resolve from
/// the weight.
pub fn gdi_face(face: Face, weight: i32, variable: bool, fluent_icons: bool) -> &'static str {
    match face {
        Face::Icons => {
            if fluent_icons {
                "Segoe Fluent Icons"
            } else {
                "Segoe MDL2 Assets"
            }
        }
        _ if !variable => "Segoe UI",
        Face::Text => {
            if weight >= 600 {
                "Segoe UI Variable Text Semibold"
            } else {
                "Segoe UI Variable Text"
            }
        }
        Face::Display => {
            if weight >= 600 {
                "Segoe UI Variable Display Semib"
            } else {
                "Segoe UI Variable Display"
            }
        }
        Face::Small => {
            if weight >= 600 {
                "Segoe UI Variable Small Semibol"
            } else {
                "Segoe UI Variable Small"
            }
        }
    }
}

/// The type ramp. Named sizes rather than arbitrary ones are what keep a
/// window looking designed rather than assembled.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// 12/16, for descriptions and state words.
    Caption,
    /// 14/20, for everything not otherwise listed.
    Body,
    /// 14/20 semibold, for section headers and the inspector's title.
    BodyStrong,
    /// 20/28 semibold, for pane titles.
    Subtitle,
    /// 28/36 semibold, for the one name on the About pane.
    Title,
}

impl TextStyle {
    pub fn size_dip(self) -> f32 {
        match self {
            TextStyle::Caption => 12.0,
            TextStyle::Body | TextStyle::BodyStrong => 14.0,
            TextStyle::Subtitle => 20.0,
            TextStyle::Title => 28.0,
        }
    }

    /// The line box, which is what rows are sized from.
    pub fn line_dip(self) -> f32 {
        match self {
            TextStyle::Caption => 16.0,
            TextStyle::Body | TextStyle::BodyStrong => 20.0,
            TextStyle::Subtitle => 28.0,
            TextStyle::Title => 36.0,
        }
    }

    pub fn weight(self) -> i32 {
        match self {
            TextStyle::Caption | TextStyle::Body => 400,
            TextStyle::BodyStrong | TextStyle::Subtitle | TextStyle::Title => 600,
        }
    }

    pub fn face(self) -> Face {
        match self {
            TextStyle::Caption => Face::Small,
            TextStyle::Body | TextStyle::BodyStrong => Face::Text,
            TextStyle::Subtitle | TextStyle::Title => Face::Display,
        }
    }
}

/// The family GDI has to be asked for to draw `family` at `weight`.
///
/// GDI knows nothing of variable fonts, so Windows registers each named
/// instance of one as a family of its own - "Segoe UI Variable Small
/// Semibol", cut to the 31 characters a face name holds - and asking the
/// parent family for weight 600 gets a synthetic bold instead. The picker
/// offers the parent family and the weight separately, so this is where the
/// two are put back together: the instance when the machine has one, the
/// family itself when it does not, which for most fonts is the right answer
/// because GDI resolves their weights properly.
///
/// The strip's own painter, in `window.rs`, draws in the family the settings
/// name and should resolve through this too, or the preview and the taskbar
/// will disagree about what semibold looks like.
/// Whether the family has a real face at this weight.
///
/// It matters because GDI does not say no. Asked for a weight the family
/// does not have, it quietly draws the nearest one it does: "Segoe UI" at
/// 500 is Segoe UI Regular, pixel for pixel, so a Medium setting looked
/// inert - which is exactly what it was. Regular is taken as always present,
/// and bold as always available because GDI will embolden a face that has no
/// bold of its own; every other weight has to be a named instance the
/// machine actually has.
pub fn has_weight(family: &str, weight: i32, installed: &[String]) -> bool {
    match weight {
        400 => true,
        w if w >= 700 => true,
        _ => instance_family(family, weight, installed) != family,
    }
}

pub fn instance_family(family: &str, weight: i32, installed: &[String]) -> String {
    let styles: &[&str] = match weight {
        w if w <= 300 => &["Light", "Semilight"],
        500 => &["Medium"],
        600 => &["Semibold", "Demibold"],
        w if w >= 700 => &["Bold"],
        _ => &[],
    };
    for style in styles {
        let wanted = format!("{family} {style}").to_lowercase();
        let hit = installed.iter().find(|name| {
            let name = name.to_lowercase();
            // Equal, or the truncation of it: "Semibol" for "Semibold".
            name == wanted || (name.len() > family.len() + 1 && wanted.starts_with(&name))
        });
        if let Some(hit) = hit {
            return hit.clone();
        }
    }
    family.to_string()
}

#[derive(Clone, PartialEq, Eq)]
struct FontKey {
    family: String,
    px: i32,
    weight: i32,
}

/// How many faces are kept before the least recently used one goes.
///
/// A panel and the settings window between them draw from a ramp of about a
/// dozen faces, so this is never reached by ordinary use. What reaches it is
/// the font picker: the strip preview redraws in whatever family the list is
/// sitting on, so arrowing down four hundred installed families made four
/// hundred live GDI objects that stood until the window was destroyed - and
/// the window is only ever hidden. The per-process quota is ten thousand.
const MAX_FONTS: usize = 64;

/// One cached face, with the pass it was last handed out on.
struct CachedFont {
    key: FontKey,
    font: HFONT,
    /// The value of `FontCache::pass` when this was last asked for.
    used: u64,
}

/// Fonts, made once per (family, size, weight) and kept.
///
/// A GDI font is a kernel handle and the desktop has a finite number of
/// them; a window that makes one per string per paint exhausts them in an
/// afternoon. Cleared on a DPI change, which is the only time the pixel sizes
/// all move at once, and trimmed to `MAX_FONTS` at the start of every pass.
pub struct FontCache {
    fonts: Vec<CachedFont>,
    /// Bumped once per `Canvas`, which is once per layout or paint.
    pass: u64,
    /// The application icon at the one size it was last drawn at. Kept
    /// beside the fonts because it has the same life: decoded from the
    /// executable's resources per size, and stale when the DPI changes.
    icon: Option<(i32, HICON)>,
    /// Every family GDI enumerated, instances included, for `instance_family`.
    installed: Vec<String>,
    /// Whether Segoe UI Variable is installed. Windows 11 always; Windows 10
    /// never.
    pub variable: bool,
    /// Whether Segoe Fluent Icons is installed, as against MDL2 Assets.
    pub fluent_icons: bool,
}

impl FontCache {
    /// Detects the faces from a list of installed families.
    pub fn new(families: &[String]) -> FontCache {
        let has = |name: &str| families.iter().any(|f| f == name);
        FontCache {
            fonts: Vec::new(),
            pass: 0,
            icon: None,
            installed: families.to_vec(),
            variable: has("Segoe UI Variable Text"),
            fluent_icons: has("Segoe Fluent Icons"),
        }
    }

    /// The application's own icon, `px` square, from the executable's
    /// resources. Null in a binary without one, such as the test harness,
    /// and the painter draws nothing for null.
    ///
    /// Read from the resources rather than from a file so the installer has
    /// nothing extra to ship: the build script already compiles the icon in
    /// under id 1, which is where `LoadImageW` looks.
    pub fn app_icon(&mut self, px: i32) -> HICON {
        if let Some((size, icon)) = self.icon {
            if size == px {
                return icon;
            }
        }
        self.drop_icon();
        // SAFETY: the module handle is this executable's; the name is the
        // integer resource id 1 in the form the call expects.
        let icon = unsafe {
            LoadImageW(GetModuleHandleW(std::ptr::null()), 1 as *const u16, IMAGE_ICON, px, px, LR_DEFAULTCOLOR)
        } as HICON;
        self.icon = Some((px, icon));
        icon
    }

    fn drop_icon(&mut self) {
        if let Some((_, icon)) = self.icon.take() {
            if !icon.is_null() {
                // SAFETY: an icon this cache loaded, destroyed once.
                unsafe { DestroyIcon(icon) };
            }
        }
    }

    /// The font for a face of the ramp, at a pixel size.
    pub fn face(&mut self, face: Face, px: i32, weight: i32) -> HFONT {
        let family = gdi_face(face, weight, self.variable, self.fluent_icons);
        self.get(family, px, weight)
    }

    /// A font by family name, for the strip preview, which draws in whatever
    /// the user chose, resolved to the named instance for the weight when
    /// the machine has one.
    pub fn get(&mut self, family: &str, px: i32, weight: i32) -> HFONT {
        // Keyed on what was asked for, not on what it resolves to, so a hit
        // costs one comparison. `instance_family` lowercases every family on
        // the machine looking for a named instance - several hundred string
        // allocations - and it was being run for every word drawn in a panel,
        // where the answer cannot change: the installed list is fixed for the
        // life of a cache, so the resolution is a pure function of this key.
        let pass = self.pass;
        if let Some(cached) = self
            .fonts
            .iter_mut()
            .find(|c| c.key.px == px && c.key.weight == weight && c.key.family == family)
        {
            cached.used = pass;
            return cached.font;
        }
        let key = FontKey { family: family.to_string(), px, weight };
        let family = instance_family(family, weight, &self.installed);
        // SAFETY: a zeroed LOGFONTW filled in below.
        let font = unsafe {
            let mut logical: LOGFONTW = std::mem::zeroed();
            // Negative asks for a character height rather than a cell height,
            // which is what a type size means.
            logical.lfHeight = -px.max(1);
            logical.lfWeight = weight;
            logical.lfCharSet = DEFAULT_CHARSET;
            logical.lfOutPrecision = OUT_TT_PRECIS;
            logical.lfClipPrecision = CLIP_DEFAULT_PRECIS;
            // ClearType: the surfaces are opaque, so there is nothing for the
            // color fringes to fringe against wrongly.
            logical.lfQuality = CLEARTYPE_QUALITY;
            for (index, unit) in wide(&family).iter().take(31).enumerate() {
                logical.lfFaceName[index] = *unit;
            }
            CreateFontIndirectW(&logical)
        };
        self.fonts.push(CachedFont { key, font, used: pass });
        font
    }

    /// Starts a pass and gives back anything beyond the ceiling.
    ///
    /// Called from `Canvas::new` and nowhere else, and that is what makes it
    /// safe. A caller holds a face across a whole draw - `preview::strip`
    /// takes its text and heading faces at the top and uses them to the
    /// bottom - so a font must never be deleted while a pass is running. At
    /// the start of one, nothing has been handed out yet.
    ///
    /// Least recently used goes first, counted in passes rather than in
    /// calls: everything a frame draws with is equally recent, and what a
    /// frame did not touch is what the picker left behind.
    pub fn begin_pass(&mut self) {
        self.pass = self.pass.wrapping_add(1);
        while self.fonts.len() > MAX_FONTS {
            let Some(oldest) = self
                .fonts
                .iter()
                .enumerate()
                .min_by_key(|(_, cached)| cached.used)
                .map(|(index, _)| index)
            else {
                return;
            };
            let cached = self.fonts.swap_remove(oldest);
            // SAFETY: a font this cache created, deleted once, and not
            // selected into any device context - see above.
            unsafe { DeleteObject(cached.font as _) };
        }
    }

    /// How many faces are held, for the test that pins the ceiling.
    #[cfg(test)]
    fn held(&self) -> usize {
        self.fonts.len()
    }

    /// Whether one face is still held, by the key it was asked for under.
    ///
    /// Asked by key rather than by comparing handles, because GDI is free to
    /// hand a deleted object's number back out and a test that believed it
    /// would pass while the cache threw the face away.
    #[cfg(test)]
    fn holds(&self, family: &str, px: i32, weight: i32) -> bool {
        self.fonts
            .iter()
            .any(|c| c.key.family == family && c.key.px == px && c.key.weight == weight)
    }

    pub fn clear(&mut self) {
        for cached in self.fonts.drain(..) {
            // SAFETY: fonts this cache created, deleted once.
            unsafe { DeleteObject(cached.font as _) };
        }
        self.drop_icon();
    }
}

impl Drop for FontCache {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A device context at a scale, with the fonts to draw into it.
pub struct Canvas<'a> {
    pub dc: HDC,
    pub scale: f32,
    pub fonts: &'a mut FontCache,
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

impl<'a> Canvas<'a> {
    pub fn new(dc: HDC, scale: f32, fonts: &'a mut FontCache) -> Canvas<'a> {
        start_gdi_plus();
        // One canvas is one pass over the window, and the moment before it
        // draws anything is the only moment at which retiring a face cannot
        // pull it out from under a caller holding one.
        fonts.begin_pass();
        Canvas { dc, scale, fonts }
    }

    pub fn px(&self, dip: f32) -> i32 {
        to_px(dip, self.scale)
    }

    /// A GDI+ surface over the DC, antialiased, with pixel centers at
    /// integer coordinates so a one-pixel pen at an integer coordinate lands
    /// on one pixel rather than straddling two.
    fn graphics(&self) -> Graphics {
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        // SAFETY: a live DC; the graphics object is released by the wrapper.
        unsafe {
            GdipCreateFromHDC(self.dc, &mut g);
            if !g.is_null() {
                GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
            }
        }
        Graphics(g)
    }

    fn brush(color: Color, alpha: u8) -> Brush {
        let mut b = std::ptr::null_mut();
        // SAFETY: an out pointer for a brush the wrapper deletes.
        unsafe { GdipCreateSolidFill(color.argb(alpha), &mut b) };
        Brush(b as *mut GpBrush)
    }

    fn pen(color: Color, width_px: f32) -> Pen {
        let mut p = std::ptr::null_mut();
        // SAFETY: an out pointer for a pen the wrapper deletes.
        unsafe { GdipCreatePen1(color.argb(255), width_px, UnitPixel, &mut p) };
        Pen(p)
    }

    /// A rounded rectangle path in device pixels.
    fn round_path(left: f32, top: f32, width: f32, height: f32, radius: f32) -> Path {
        let mut p: *mut GpPath = std::ptr::null_mut();
        // The radius cannot exceed half the shorter side or the arcs cross.
        let r = radius.min(width / 2.0).min(height / 2.0).max(0.0);
        let d = r * 2.0;
        // SAFETY: a fresh path, closed before use, deleted by the wrapper.
        unsafe {
            GdipCreatePath(FillModeAlternate, &mut p);
            if p.is_null() {
                return Path(p);
            }
            if r <= 0.0 {
                GdipAddPathArc(p, left, top, 0.0, 0.0, 0.0, 0.0);
            }
            GdipAddPathArc(p, left, top, d, d, 180.0, 90.0);
            GdipAddPathArc(p, left + width - d, top, d, d, 270.0, 90.0);
            GdipAddPathArc(p, left + width - d, top + height - d, d, d, 0.0, 90.0);
            GdipAddPathArc(p, left, top + height - d, d, d, 90.0, 90.0);
            GdipClosePathFigure(p);
        }
        Path(p)
    }

    /// A plain filled rectangle, in GDI, which is exact at every scale.
    pub fn fill_rect(&self, r: Rect, color: Color) {
        let px = r.px(self.scale);
        let rect = RECT { left: px.left, top: px.top, right: px.right, bottom: px.bottom };
        // SAFETY: a brush made and deleted here, and a rect on the stack.
        unsafe {
            let brush = CreateSolidBrush(color.colorref());
            FillRect(self.dc, &rect, brush);
            DeleteObject(brush as _);
        }
    }

    pub fn fill_round(&self, r: Rect, radius: f32, color: Color) {
        self.fill_round_alpha(r, radius, color, 255);
    }

    /// A rounded fill with an alpha, composited by GDI+ over what is there.
    ///
    /// The only place alpha is used: the hover wash over a strip item in the
    /// preview, which the design defines as ink at 8%, and which no opaque
    /// token can stand in for because the ground varies.
    pub fn fill_round_alpha(&self, r: Rect, radius: f32, color: Color, alpha: u8) {
        let px = r.px(self.scale);
        if px.width() <= 0 || px.height() <= 0 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let path = Canvas::round_path(
            px.left as f32,
            px.top as f32,
            px.width() as f32,
            px.height() as f32,
            radius * self.scale,
        );
        let brush = Canvas::brush(color, alpha);
        // SAFETY: all three objects are live and owned by their wrappers.
        unsafe { GdipFillPath(g.0, brush.0, path.0) };
    }

    /// A hairline stroke just inside the bounds of a rounded rectangle.
    pub fn stroke_round(&self, r: Rect, radius: f32, color: Color) {
        self.stroke_round_px(r, radius, color, hairline_px(self.scale) as f32);
    }

    /// A stroke of a given width in DIPs, for the focus ring, which is the
    /// one outline that is not a hairline.
    pub fn stroke_round_width(&self, r: Rect, radius: f32, color: Color, width_dip: f32) {
        self.stroke_round_px(r, radius, color, (width_dip * self.scale).round().max(1.0));
    }

    fn stroke_round_px(&self, r: Rect, radius: f32, color: Color, width: f32) {
        let px = r.px(self.scale);
        if px.width() <= 0 || px.height() <= 0 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        // Inset by half the pen so the stroke sits inside the rectangle
        // rather than half outside it, where the next control would cover it.
        let half = width / 2.0;
        let path = Canvas::round_path(
            px.left as f32 + half,
            px.top as f32 + half,
            px.width() as f32 - width,
            px.height() as f32 - width,
            radius * self.scale - half,
        );
        let pen = Canvas::pen(color, width);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawPath(g.0, pen.0, path.0) };
    }


    /// A pen with round caps and joins.
    ///
    /// Every stroke in an icon uses one. It is most of what makes a drawn mark
    /// look like a symbol rather than like a diagram: a butt cap ends a line in
    /// a hard corner, and at sixteen pixels a dozen hard corners read as grit.
    fn round_pen(color: Color, width_px: f32) -> Pen {
        let pen = Canvas::pen(color, width_px);
        // SAFETY: a pen this function just created and owns.
        unsafe {
            GdipSetPenStartCap(pen.0, LineCapRound);
            GdipSetPenEndCap(pen.0, LineCapRound);
            GdipSetPenLineJoin(pen.0, LineJoinRound);
        }
        pen
    }

    /// Icon strokes are never hairlines: they scale with the mark so a symbol
    /// keeps its weight at every size, where a hairline would turn spidery.
    fn icon_width(&self, width_dip: f32) -> f32 {
        (width_dip * self.scale).max(1.0)
    }

    /// A stroked polyline through points given in DIPs.
    pub fn stroke_poly(&self, points: &[(f32, f32)], width_dip: f32, color: Color, closed: bool) {
        if points.len() < 2 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let Some(path) = self.poly_path(points, closed) else { return };
        let pen = Canvas::round_pen(color, self.icon_width(width_dip));
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawPath(g.0, pen.0, path.0) };
    }

    /// A filled polygon through points given in DIPs.
    pub fn fill_poly(&self, points: &[(f32, f32)], color: Color) {
        if points.len() < 3 {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let Some(path) = self.poly_path(points, true) else { return };
        let brush = Canvas::brush(color, 255);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipFillPath(g.0, brush.0, path.0) };
    }

    fn poly_path(&self, points: &[(f32, f32)], closed: bool) -> Option<Path> {
        let mut p: *mut GpPath = std::ptr::null_mut();
        // SAFETY: an out pointer for a path the wrapper deletes.
        unsafe { GdipCreatePath(FillModeAlternate, &mut p) };
        if p.is_null() {
            return None;
        }
        let path = Path(p);
        // SAFETY: a path this function owns; every coordinate is finite.
        unsafe {
            GdipStartPathFigure(path.0);
            for pair in points.windows(2) {
                GdipAddPathLine(
                    path.0,
                    pair[0].0 * self.scale,
                    pair[0].1 * self.scale,
                    pair[1].0 * self.scale,
                    pair[1].1 * self.scale,
                );
            }
            if closed {
                GdipClosePathFigure(path.0);
            }
        }
        Some(path)
    }

    /// A stroked circle of a given width, in DIPs.
    pub fn stroke_circle_width(&self, cx: f32, cy: f32, radius: f32, width_dip: f32, color: Color) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let width = self.icon_width(width_dip);
        let pen = Canvas::round_pen(color, width);
        let d = radius * 2.0 * self.scale - width;
        if d <= 0.0 {
            return;
        }
        // SAFETY: live objects owned by their wrappers.
        unsafe {
            GdipDrawEllipse(
                g.0,
                pen.0,
                (cx - radius) * self.scale + width / 2.0,
                (cy - radius) * self.scale + width / 2.0,
                d,
                d,
            )
        };
    }

    /// A stroked figure made of arcs, each an ellipse box in DIPs with a start
    /// and a sweep in degrees.
    ///
    /// GDI+ joins consecutive arcs with a straight line and, when the figure is
    /// closed, runs one more from the last point back to the first. That is
    /// what draws the flat underside of a cloud without it having to be listed:
    /// three lobes and a close.
    ///
    /// Angles are the GDI+ convention, which is not the mathematical one: zero
    /// points along +x, and because y runs downward a positive sweep travels
    /// clockwise on screen. So 180 to 360 is the *top* of a circle, which is
    /// the opposite of what it reads like.
    pub fn stroke_arcs(
        &self,
        arcs: &[(Rect, f32, f32)],
        width_dip: f32,
        color: Color,
        closed: bool,
    ) {
        if arcs.is_empty() {
            return;
        }
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let mut p: *mut GpPath = std::ptr::null_mut();
        // SAFETY: an out pointer for a path the wrapper deletes.
        unsafe { GdipCreatePath(FillModeAlternate, &mut p) };
        if p.is_null() {
            return;
        }
        let path = Path(p);
        // SAFETY: a path this function owns; every coordinate is finite.
        unsafe {
            GdipStartPathFigure(path.0);
            for (box_, start, sweep) in arcs {
                GdipAddPathArc(
                    path.0,
                    box_.x * self.scale,
                    box_.y * self.scale,
                    box_.w * self.scale,
                    box_.h * self.scale,
                    *start,
                    *sweep,
                );
            }
            if closed {
                GdipClosePathFigure(path.0);
            }
        }
        let pen = Canvas::round_pen(color, self.icon_width(width_dip));
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawPath(g.0, pen.0, path.0) };
    }

    /// A stroked rounded rectangle of a given width, in DIPs, with round joins.
    pub fn stroke_round_icon(&self, r: Rect, radius: f32, width_dip: f32, color: Color) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let width = self.icon_width(width_dip);
        let half = width / 2.0;
        let path = Canvas::round_path(
            r.x * self.scale + half,
            r.y * self.scale + half,
            r.w * self.scale - width,
            r.h * self.scale - width,
            radius * self.scale - half,
        );
        let pen = Canvas::round_pen(color, width);
        // SAFETY: live objects owned by their wrappers.
        unsafe { GdipDrawPath(g.0, pen.0, path.0) };
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, radius: f32, color: Color) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let brush = Canvas::brush(color, 255);
        let d = radius * 2.0 * self.scale;
        // SAFETY: live objects owned by their wrappers.
        unsafe {
            GdipFillEllipse(g.0, brush.0, (cx - radius) * self.scale, (cy - radius) * self.scale, d, d)
        };
    }

    pub fn stroke_circle(&self, cx: f32, cy: f32, radius: f32, color: Color) {
        let g = self.graphics();
        if g.0.is_null() {
            return;
        }
        let width = hairline_px(self.scale) as f32;
        let pen = Canvas::pen(color, width);
        let d = radius * 2.0 * self.scale - width;
        // SAFETY: live objects owned by their wrappers.
        unsafe {
            GdipDrawEllipse(
                g.0,
                pen.0,
                (cx - radius) * self.scale + width / 2.0,
                (cy - radius) * self.scale + width / 2.0,
                d,
                d,
            )
        };
    }

    /// A horizontal hairline, snapped to whole pixels so it is never blurred
    /// across two rows.
    pub fn hline(&self, x1: f32, x2: f32, y: f32, color: Color) {
        let thickness = hairline_px(self.scale);
        let top = self.px(y);
        let rect = RECT { left: self.px(x1), top, right: self.px(x2), bottom: top + thickness };
        // SAFETY: a brush made and deleted here.
        unsafe {
            let brush = CreateSolidBrush(color.colorref());
            FillRect(self.dc, &rect, brush);
            DeleteObject(brush as _);
        }
    }

    pub fn vline(&self, x: f32, y1: f32, y2: f32, color: Color) {
        let thickness = hairline_px(self.scale);
        let left = self.px(x);
        let rect = RECT { left, top: self.px(y1), right: left + thickness, bottom: self.px(y2) };
        // SAFETY: a brush made and deleted here.
        unsafe {
            let brush = CreateSolidBrush(color.colorref());
            FillRect(self.dc, &rect, brush);
            DeleteObject(brush as _);
        }
    }

    fn select_style(&mut self, style: TextStyle) -> HFONT {
        let px = self.px(style.size_dip());
        self.fonts.face(style.face(), px, style.weight())
    }

    /// One line of text in a rectangle, vertically centered, cut with an
    /// ellipsis rather than clipped mid-glyph when it does not fit.
    pub fn text(&mut self, r: Rect, text: &str, style: TextStyle, color: Color, align: Align) {
        let font = self.select_style(style);
        let flags = DT_SINGLELINE
            | DT_VCENTER
            | DT_END_ELLIPSIS
            | DT_NOPREFIX
            | match align {
                Align::Left => DT_LEFT,
                Align::Center => DT_CENTER,
                Align::Right => DT_RIGHT,
            };
        self.draw_text(r, text, font, color, flags);
    }

    /// Text wrapped to the rectangle's width, from its top.
    pub fn text_wrapped(&mut self, r: Rect, text: &str, style: TextStyle, color: Color) {
        let font = self.select_style(style);
        self.draw_text(r, text, font, color, DT_WORDBREAK | DT_NOPREFIX);
    }

    /// One line of a file path, cut in the middle when it does not fit, so
    /// the drive and the final folder both survive.
    pub fn text_path(&mut self, r: Rect, text: &str, style: TextStyle, color: Color) {
        let font = self.select_style(style);
        self.draw_text(r, text, font, color, DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS | DT_NOPREFIX | DT_LEFT);
    }

    fn draw_text(&mut self, r: Rect, text: &str, font: HFONT, color: Color, flags: u32) {
        let px = r.px(self.scale);
        let mut rect = RECT { left: px.left, top: px.top, right: px.right, bottom: px.bottom };
        let units = wide(text);
        // SAFETY: the DC is live, the font outlives the call, and the string
        // is passed with its length.
        unsafe {
            let previous = SelectObject(self.dc, font as _);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, color.colorref());
            DrawTextW(self.dc, units.as_ptr(), units.len() as i32, &mut rect, flags);
            SelectObject(self.dc, previous);
        }
    }

    /// The width and height of one line, in DIPs.
    pub fn measure(&mut self, text: &str, style: TextStyle) -> (f32, f32) {
        let font = self.select_style(style);
        let units = wide(text);
        let mut size = SIZE { cx: 0, cy: 0 };
        // SAFETY: the DC is live and the string is passed with its length.
        unsafe {
            let previous = SelectObject(self.dc, font as _);
            GetTextExtentPoint32W(self.dc, units.as_ptr(), units.len() as i32, &mut size);
            SelectObject(self.dc, previous);
        }
        (size.cx as f32 / self.scale, size.cy as f32 / self.scale)
    }

    /// The height text takes wrapped to a width, in DIPs.
    pub fn measure_wrapped(&mut self, text: &str, style: TextStyle, width: f32) -> f32 {
        let font = self.select_style(style);
        let units = wide(text);
        let mut rect = RECT { left: 0, top: 0, right: self.px(width), bottom: 0 };
        // SAFETY: DT_CALCRECT only writes the rect.
        let height = unsafe {
            let previous = SelectObject(self.dc, font as _);
            let height = DrawTextW(
                self.dc,
                units.as_ptr(),
                units.len() as i32,
                &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(self.dc, previous);
            height
        };
        height as f32 / self.scale
    }

    /// One line in a face of the ramp at a size and weight the ramp does not
    /// name, for the initial on a tile.
    #[allow(clippy::too_many_arguments)]
    pub fn text_face(
        &mut self,
        r: Rect,
        text: &str,
        face: Face,
        size_dip: f32,
        weight: i32,
        color: Color,
        align: Align,
    ) {
        let px = self.px(size_dip);
        let font = self.fonts.face(face, px, weight);
        let flags = DT_SINGLELINE
            | DT_VCENTER
            | DT_NOPREFIX
            | match align {
                Align::Left => DT_LEFT,
                Align::Center => DT_CENTER,
                Align::Right => DT_RIGHT,
            };
        self.draw_text(r, text, font, color, flags);
    }

    /// The application icon, filling a square rectangle.
    pub fn app_icon(&mut self, r: Rect) {
        let px = r.px(self.scale);
        let size = px.width().min(px.height());
        let icon = self.fonts.app_icon(size);
        if icon.is_null() {
            return;
        }
        // SAFETY: a live DC and an icon the cache owns for as long as it is
        // drawn with; no flicker brush, so the icon's own alpha is used.
        unsafe { DrawIconEx(self.dc, px.left, px.top, icon, size, size, 0, std::ptr::null_mut(), DI_NORMAL) };
    }

    /// An icon-font glyph centered in a rectangle.
    pub fn glyph(&mut self, r: Rect, glyph: &str, size_dip: f32, color: Color) {
        let px = self.px(size_dip);
        let font = self.fonts.face(Face::Icons, px, 400);
        self.draw_text(r, glyph, font, color, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
    }

    /// Raw text at a pixel position in an explicit font, for the preview,
    /// which draws exactly what the strip draws.
    pub fn font_text(&self, x: i32, y: i32, text: &[u16], font: HFONT, color: Color) {
        // SAFETY: the DC is live and the font outlives the call.
        unsafe {
            let previous = SelectObject(self.dc, font as _);
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, color.colorref());
            TextOutW(self.dc, x, y, text.as_ptr(), text.len() as i32);
            SelectObject(self.dc, previous);
        }
    }

    pub fn font_measure(&self, text: &[u16], font: HFONT) -> SIZE {
        let mut size = SIZE { cx: 0, cy: 0 };
        // SAFETY: the DC is live and the string is passed with its length.
        unsafe {
            let previous = SelectObject(self.dc, font as _);
            GetTextExtentPoint32W(self.dc, text.as_ptr(), text.len() as i32, &mut size);
            SelectObject(self.dc, previous);
        }
        size
    }

    /// Restricts drawing to a rectangle until `unclip` is called with the
    /// returned token.
    pub fn clip(&self, r: Rect) -> i32 {
        let px = r.px(self.scale);
        // SAFETY: SaveDC/IntersectClipRect on a live DC.
        unsafe {
            let saved = SaveDC(self.dc);
            IntersectClipRect(self.dc, px.left, px.top, px.right, px.bottom);
            saved
        }
    }

    pub fn unclip(&self, saved: i32) {
        // SAFETY: a token SaveDC returned for this DC.
        unsafe { RestoreDC(self.dc, saved) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semibold_resolves_to_the_named_instances_gdi_actually_lists() {
        // Asking GDI for weight 600 on the variable family would synthesize
        // a bold; these are the registered family names, two of them cut by
        // the 31-character limit.
        assert_eq!(gdi_face(Face::Text, 600, true, true), "Segoe UI Variable Text Semibold");
        assert_eq!(gdi_face(Face::Display, 600, true, true), "Segoe UI Variable Display Semib");
        assert_eq!(gdi_face(Face::Small, 600, true, true), "Segoe UI Variable Small Semibol");
        assert_eq!(gdi_face(Face::Text, 400, true, true), "Segoe UI Variable Text");
    }

    #[test]
    fn without_the_variable_family_everything_is_plain_segoe_ui() {
        for face in [Face::Text, Face::Display, Face::Small] {
            assert_eq!(gdi_face(face, 400, false, true), "Segoe UI");
            assert_eq!(gdi_face(face, 600, false, true), "Segoe UI");
        }
        assert_eq!(gdi_face(Face::Icons, 400, false, false), "Segoe MDL2 Assets");
        assert_eq!(gdi_face(Face::Icons, 400, true, true), "Segoe Fluent Icons");
    }

    #[test]
    fn the_type_ramp_is_the_documented_one() {
        assert_eq!(TextStyle::Caption.size_dip(), 12.0);
        assert_eq!(TextStyle::Body.size_dip(), 14.0);
        assert_eq!(TextStyle::Subtitle.size_dip(), 20.0);
        assert_eq!(TextStyle::BodyStrong.weight(), 600);
        assert_eq!(TextStyle::Body.line_dip(), 20.0);
        assert_eq!(TextStyle::Title.size_dip(), 28.0);
        assert_eq!(TextStyle::Title.line_dip(), 36.0);
        assert_eq!(TextStyle::Title.face(), Face::Display);
    }

    #[test]
    fn a_pass_never_loses_a_face_it_is_using_and_the_next_one_gives_the_stale_ones_back() {
        // The font picker is what makes this happen: the strip preview
        // redraws in whichever family the list is sitting on, so walking a
        // machine's families made one live GDI object apiece.
        let mut cache = FontCache::new(&[]);
        let over = MAX_FONTS + 40;
        for index in 0..over {
            cache.get(&format!("Face {index}"), 12, 400);
        }
        assert_eq!(
            cache.held(),
            over,
            "a face handed out during a pass has to survive that pass"
        );

        cache.begin_pass();
        assert_eq!(cache.held(), MAX_FONTS, "the next pass gives the excess back");

        // Least recently used, counted in passes: what this pass draws with
        // stays, and what only an older pass wanted is what goes.
        let kept = "Face 0";
        cache.get(kept, 12, 400);
        for index in over..(over + MAX_FONTS) {
            cache.get(&format!("Face {index}"), 12, 400);
        }
        cache.begin_pass();
        assert_eq!(cache.held(), MAX_FONTS);
        assert!(cache.holds(kept, 12, 400), "the face this pass used was thrown away");

        // And a size or a weight of its own is a face of its own, so the
        // ceiling counts what it is really holding.
        cache.get(kept, 13, 400);
        cache.get(kept, 12, 700);
        assert!(cache.holds(kept, 13, 400) && cache.holds(kept, 12, 700));

        cache.clear();
        assert_eq!(cache.held(), 0);
    }

    #[test]
    fn font_detection_reads_the_installed_list() {
        let installed = vec!["Arial".to_string(), "Segoe UI Variable Text".to_string()];
        let cache = FontCache::new(&installed);
        assert!(cache.variable);
        assert!(!cache.fluent_icons);
    }

    /// A family chosen from the picker plus a weight from the other control
    /// resolves to the instance GDI actually has, truncated name and all,
    /// and to the family itself where there is none to find.
    #[test]
    fn a_family_and_a_weight_resolve_to_the_registered_instance() {
        let installed: Vec<String> = [
            "Segoe UI Variable Small",
            "Segoe UI Variable Small Light",
            "Segoe UI Variable Small Semibol",
            "Segoe UI Variable Text",
            "Segoe UI Variable Text Semibold",
            "Arial",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(instance_family("Segoe UI Variable Small", 600, &installed), "Segoe UI Variable Small Semibol");
        assert_eq!(instance_family("Segoe UI Variable Text", 600, &installed), "Segoe UI Variable Text Semibold");
        assert_eq!(instance_family("Segoe UI Variable Small", 300, &installed), "Segoe UI Variable Small Light");
        assert_eq!(instance_family("Segoe UI Variable Small", 400, &installed), "Segoe UI Variable Small");
        // No Medium instance exists, and no bold one: the family carries on.
        assert_eq!(instance_family("Segoe UI Variable Small", 500, &installed), "Segoe UI Variable Small");
        assert_eq!(instance_family("Arial", 700, &installed), "Arial");
    }
}

/// Starts GDI+, once for the process.
///
/// Everything that draws through GDI+ - the flyout's rounded cards, the
/// weather's sky, the settings window's marks - goes through a canvas made
/// here, so this is where the library is brought up. It used to live beside
/// a vector badge renderer that no longer exists.
fn start_gdi_plus() {
    use std::sync::Once;
    use windows_sys::Win32::Graphics::GdiPlus::{GdiplusStartup, GdiplusStartupInput};
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: a zeroed input filled in below, and a token we discard along
        // with the shutdown it would be used for.
        unsafe {
            let mut input: GdiplusStartupInput = std::mem::zeroed();
            input.GdiplusVersion = 1;
            let mut token: usize = 0;
            GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        }
    });
}
