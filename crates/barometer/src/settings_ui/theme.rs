// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The settings window's colors and glyphs.
//
// These are Windows 11's, not Barometer's. The window is meant to read as a
// page of Settings, so the neutral ramp, the accent handling and the status
// colors are WinUI's resolved values (docs/ui-design.md, section 2), and
// Barometer's own identity - the dark ground and the module colors - is kept
// to the preview strip and to the tiles beside the module names.
//
// Every value is resolved to an opaque color here, once per theme change,
// rather than expressed as an alpha over a surface the way WinUI's tokens are.
// GDI has no alpha to composite with, and one opaque brush per token is what
// lets the painter never blend.

use barometer_core::ModuleId;

/// A color, as 0xRRGGBB.
///
/// Not a COLORREF: GDI's byte order is the reverse, and holding the design's
/// own order means every hex value in this file can be read against the
/// design document without swapping bytes in one's head.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    pub fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn b(self) -> u8 {
        self.0 as u8
    }

    /// The COLORREF GDI wants. Getting this wrong is not an error, it is an
    /// amber that draws as blue.
    pub fn colorref(self) -> u32 {
        ((self.b() as u32) << 16) | ((self.g() as u32) << 8) | self.r() as u32
    }

    /// The ARGB GDI+ wants.
    pub fn argb(self, alpha: u8) -> u32 {
        ((alpha as u32) << 24) | (self.0 & 0x00FF_FFFF)
    }

    /// This color composited over `ground` at `alpha`, resolved to opaque.
    pub fn over(self, ground: Color, alpha: f32) -> Color {
        let alpha = alpha.clamp(0.0, 1.0);
        let mix = |top: u8, under: u8| {
            (under as f32 + (top as f32 - under as f32) * alpha).round().clamp(0.0, 255.0) as u8
        };
        Color::rgb(mix(self.r(), ground.r()), mix(self.g(), ground.g()), mix(self.b(), ground.b()))
    }

    pub fn lighten(self, amount: f32) -> Color {
        Color::rgb(255, 255, 255).over(self, amount)
    }

    pub fn darken(self, amount: f32) -> Color {
        Color::rgb(0, 0, 0).over(self, amount)
    }

    /// WCAG relative luminance.
    pub fn luminance(self) -> f64 {
        let channel = |value: u8| {
            let c = value as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r()) + 0.7152 * channel(self.g()) + 0.0722 * channel(self.b())
    }

    /// WCAG contrast ratio between two colors, 1 through 21.
    pub fn contrast(self, other: Color) -> f64 {
        let a = self.luminance();
        let b = other.luminance();
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }
}

pub const WHITE: Color = Color(0xFFFFFF);
pub const BLACK: Color = Color(0x000000);

/// The dark ground the app icon and the strip's backplate share.
pub const BRAND_GROUND: Color = Color(0x1C1E22);

/// Windows' default accent, used until the registry says otherwise.
pub const DEFAULT_ACCENT: Color = Color(0x0067C0);

/// Words need 4.5:1 against what they sit on. WCAG 1.4.3.
const TEXT_CONTRAST: f64 = 4.5;
/// A mark that is not text needs 3:1. WCAG 1.4.11.
const MARK_CONTRAST: f64 = 3.0;

/// The neutral and accent tokens, resolved for one appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub light: bool,
    pub surface_base: Color,
    pub surface_layer: Color,
    pub surface_card: Color,
    pub control_rest: Color,
    pub control_hover: Color,
    pub control_pressed: Color,
    pub control_disabled: Color,
    pub subtle_hover: Color,
    pub subtle_pressed: Color,
    /// A selected list or nav item: a step past hover, so the selection is
    /// still the strongest wash on the page while the pointer is on a row
    /// beside it.
    pub subtle_selected: Color,
    pub stroke_control: Color,
    pub stroke_control_edge: Color,
    pub stroke_strong: Color,
    pub stroke_divider: Color,
    pub stroke_card: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_disabled: Color,
    pub text_on_accent: Color,
    pub accent_fill: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub accent_text: Color,
    pub status_success: Color,
    pub status_caution: Color,
    pub status_critical: Color,
    pub focus_outer: Color,
    pub focus_inner: Color,
}

impl Theme {
    /// The tokens for an appearance and an accent.
    ///
    /// WinUI reaches its accent variants (Light2, Dark1 and so on) through a
    /// palette algorithm the shell keeps to itself; only the base accent is
    /// readable from the registry. The steps below approximate those variants
    /// by mixing toward white or black, and then the one property that
    /// actually matters - words on the accent, and the accent as words on a
    /// card, both at 4.5:1 - is checked rather than assumed.
    pub fn resolve(light: bool, accent: Color) -> Theme {
        let surface_card = if light { Color(0xFDFDFD) } else { Color(0x333333) };

        // Dark mode uses a lighter accent for fills so black text can sit on
        // it, which is WinUI's AccentLight2; light mode uses the accent itself.
        let accent_fill = if light { accent } else { accent.lighten(0.30) };
        let accent_hover = accent_fill.over(surface_card, 0.90);
        // Pressed steps away from the surface rather than toward it. WinUI
        // fades pressed to 80%, which drops white text under 4.5:1 on the
        // default blue; a step darker keeps the words readable in every state.
        let accent_pressed = if light { accent.darken(0.12) } else { accent.lighten(0.45) };
        let text_on_accent = on(accent_fill);
        let accent_text = readable_accent(accent, light, surface_card);

        Theme {
            light,
            surface_base: if light { Color(0xF3F3F3) } else { Color(0x202020) },
            surface_layer: if light { Color(0xF9F9F9) } else { Color(0x282828) },
            surface_card,
            control_rest: if light { Color(0xFBFBFB) } else { Color(0x3F3F3F) },
            control_hover: if light { Color(0xF6F6F6) } else { Color(0x444444) },
            control_pressed: if light { Color(0xF1F1F1) } else { Color(0x3A3A3A) },
            control_disabled: if light { Color(0xF5F5F5) } else { Color(0x3C3C3C) },
            subtle_hover: if light { Color(0xEDEDED) } else { Color(0x353535) },
            subtle_pressed: if light { Color(0xF1F1F1) } else { Color(0x313131) },
            subtle_selected: if light { Color(0xE6E6E6) } else { Color(0x3C3C3C) },
            stroke_control: if light { Color(0xE5E5E5) } else { Color(0x4C4C4C) },
            stroke_control_edge: if light { Color(0xD4D4D4) } else { Color(0x5A5A5A) },
            stroke_strong: if light { Color(0x8A8A8A) } else { Color(0x9D9D9D) },
            stroke_divider: if light { Color(0xE9E9E9) } else { Color(0x444444) },
            stroke_card: if light { Color(0xE5E5E5) } else { Color(0x3D3D3D) },
            text_primary: if light { Color(0x1B1B1B) } else { WHITE },
            text_secondary: if light { Color(0x5E5E5E) } else { Color(0xD0D0D0) },
            text_tertiary: if light { Color(0x8E8E8E) } else { Color(0x9D9D9D) },
            text_disabled: if light { Color(0xA0A0A0) } else { Color(0x767676) },
            text_on_accent,
            accent_fill,
            accent_hover,
            accent_pressed,
            accent_text,
            status_success: if light { Color(0x0F7B0F) } else { Color(0x6CCB5F) },
            status_caution: if light { Color(0x9D5D00) } else { Color(0xFCE100) },
            status_critical: if light { Color(0xC42B1C) } else { Color(0xFF99A4) },
            focus_outer: if light { BLACK } else { WHITE },
            focus_inner: if light { WHITE } else { BLACK },
        }
    }

    /// A module's hue as a glyph on the content pane.
    ///
    /// The signature colors are the macOS app's and were picked for its dark
    /// grounds; on the light pane the teal, the sky blue and the orange fall
    /// under 3:1. Each is stepped toward the text ink only as far as it has
    /// to go, so the set keeps its hues and gives up just the brightness the
    /// pane cannot carry, and in dark mode most are used as they are. The
    /// glyph is what gets tinted, never the label beside it: the name has to
    /// read at 4.5:1 and the mark only has to be told from its neighbors.
    pub fn mark_ink(&self, color: Color) -> Color {
        let mut amount = 0.0;
        loop {
            let candidate = if self.light { color.darken(amount) } else { color.lighten(amount) };
            if candidate.contrast(self.surface_layer) >= MARK_CONTRAST || amount >= 0.8 {
                return candidate;
            }
            amount += 0.08;
        }
    }

    /// The ground the preview strip estimates the taskbar to be.
    ///
    /// The strip follows the taskbar's theme, not the app's, and the two are
    /// separate switches in Windows; but the preview lives in a window that
    /// follows the same switch, so its ground is the taskbar's estimate for
    /// the appearance the window is showing, and the flip button shows the
    /// other one.
    pub fn strip_ground(light: bool) -> Color {
        if light {
            Color(0xEEEEEE)
        } else {
            Color(0x202020)
        }
    }

    /// The ink the strip draws with on that ground. The values are
    /// `window.rs`'s own, so the preview and the taskbar agree.
    pub fn strip_ink(light: bool) -> Color {
        if light {
            Color(0x191919)
        } else {
            Color(0xF2F2F2)
        }
    }
}

/// White or black, whichever reads better on a fill.
fn on(fill: Color) -> Color {
    if WHITE.contrast(fill) >= BLACK.contrast(fill) {
        WHITE
    } else {
        BLACK
    }
}

/// The accent as text on a card, pushed away from the surface until it reads.
///
/// Starting from the accent one step toward the text and walking further in
/// steps, the way the design's derivation rule says; the accent that never
/// gets there - a pale yellow on a white card - gives way to the ordinary
/// text color rather than to unreadable words.
fn readable_accent(accent: Color, light: bool, card: Color) -> Color {
    let mut amount = if light { 0.15 } else { 0.55 };
    for _ in 0..8 {
        let candidate = if light { accent.darken(amount) } else { accent.lighten(amount) };
        if candidate.contrast(card) >= TEXT_CONTRAST {
            return candidate;
        }
        amount += 0.10;
    }
    if light {
        Color(0x1B1B1B)
    } else {
        WHITE
    }
}

/// Each module's signature color, from BarometerDesign.swift unchanged, so
/// the two apps' marks match. Seven hues a step apart around the wheel -
/// blue, violet, indigo, teal, sky, orange, light blue - which is what
/// lets a row in the composer be found by its color before its name.
pub fn module_color(id: ModuleId) -> Color {
    match id {
        ModuleId::Cpu => Color(0x3B82F6),
        ModuleId::Gpu => Color(0xA855F7),
        ModuleId::Memory => Color(0x6366F1),
        ModuleId::Disks => Color(0x14B8A6),
        ModuleId::Network => Color(0x0EA5E9),
        ModuleId::Sensors => Color(0xF97316),
        ModuleId::Weather => Color(0x38BDF8),
    }
}

/// The color of a stack's mark: the macOS "Combined" item's, since a stack
/// is what Combined became on Windows.
pub const STACK_COLOR: Color = Color(0x2F7CF6);

/// The interface glyphs, by name.
///
/// One table, as in Clicker: an inline `\u{E7xx}` anywhere else is a hole,
/// because it names a code point nobody can find again when the font changes.
/// These are Segoe Fluent Icons code points, and every one is also present at
/// the same code point in Segoe MDL2 Assets, which is the Windows 10 fallback.
pub mod glyph {
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    pub const SEARCH: &str = "\u{E721}";
    pub const CANCEL: &str = "\u{E711}";
    pub const CHECK: &str = "\u{E73E}";
    pub const ADD: &str = "\u{E710}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const OPEN_IN_NEW: &str = "\u{E8A7}";
    pub const GRIPPER: &str = "\u{E784}";
    pub const SUN: &str = "\u{E706}";
    pub const MOON: &str = "\u{E708}";
    pub const UP: &str = "\u{E74A}";
    pub const DOWN: &str = "\u{E74B}";
    pub const DELETE: &str = "\u{E74D}";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_appearances_resolve_to_the_documented_neutrals() {
        let dark = Theme::resolve(false, DEFAULT_ACCENT);
        let light = Theme::resolve(true, DEFAULT_ACCENT);
        assert_eq!(dark.text_primary, WHITE);
        assert_eq!(dark.surface_base, Color(0x202020));
        assert_eq!(light.text_primary, Color(0x1B1B1B));
        assert_eq!(light.surface_base, Color(0xF3F3F3));
        assert!(dark.stroke_strong.contrast(dark.surface_card) >= 3.0);
        assert!(light.stroke_strong.contrast(light.surface_card) >= 3.0);
    }

    /// Selection has to read as more than hover in both appearances, or the
    /// pill is the only thing telling the two apart.
    #[test]
    fn a_selected_item_is_washed_a_step_past_a_hovered_one() {
        for light in [true, false] {
            let theme = Theme::resolve(light, DEFAULT_ACCENT);
            let hover = theme.subtle_hover.contrast(theme.surface_layer);
            let selected = theme.subtle_selected.contrast(theme.surface_layer);
            assert!(selected > hover, "light={light}: {selected} vs {hover}");
        }
    }

    #[test]
    fn words_on_the_accent_read_in_both_appearances() {
        // The default blue takes white in light mode; the lightened fill dark
        // mode uses takes black, which is what WinUI does too.
        let light = Theme::resolve(true, DEFAULT_ACCENT);
        assert_eq!(light.text_on_accent, WHITE);
        assert!(light.text_on_accent.contrast(light.accent_fill) >= TEXT_CONTRAST);
        let dark = Theme::resolve(false, DEFAULT_ACCENT);
        assert!(dark.text_on_accent.contrast(dark.accent_fill) >= TEXT_CONTRAST);
        // Pressed must not lose the words the rest state has.
        assert!(light.text_on_accent.contrast(light.accent_pressed) >= TEXT_CONTRAST);
    }

    #[test]
    fn the_accent_as_text_is_pushed_until_it_reads_on_a_card() {
        for light in [true, false] {
            for accent in [DEFAULT_ACCENT, Color(0xFFB900), Color(0x00CC6A), Color(0xE3008C)] {
                let theme = Theme::resolve(light, accent);
                assert!(
                    theme.accent_text.contrast(theme.surface_card) >= TEXT_CONTRAST,
                    "{accent:?} light={light}"
                );
            }
        }
    }

    /// Every module's mark clears 3:1 on the pane in both appearances, and
    /// keeps its hue doing so: a mark that had to be pushed all the way to
    /// the text ink would be telling nobody which module it was.
    #[test]
    fn every_mark_reads_on_the_pane_in_both_appearances_without_losing_its_hue() {
        for light in [true, false] {
            let theme = Theme::resolve(light, DEFAULT_ACCENT);
            let marks = ModuleId::ALL.iter().map(|id| module_color(*id)).chain([STACK_COLOR]);
            for color in marks {
                let ink = theme.mark_ink(color);
                assert!(ink.contrast(theme.surface_layer) >= MARK_CONTRAST, "{color:?} light={light}");
                assert!(ink.contrast(theme.text_primary) > 1.5, "{color:?} light={light} lost its hue");
            }
        }
        // Dark mode takes the signature blue as it is.
        let dark = Theme::resolve(false, DEFAULT_ACCENT);
        assert_eq!(dark.mark_ink(module_color(ModuleId::Cpu)), module_color(ModuleId::Cpu));
    }

    #[test]
    fn colorref_swaps_the_ends_and_argb_keeps_them() {
        let amber = Color(0xFFC53D);
        assert_eq!(amber.colorref(), 0x003DC5FF);
        assert_eq!(amber.argb(0x80), 0x80FFC53D);
    }

    #[test]
    fn compositing_at_full_and_zero_alpha_are_the_two_inputs() {
        assert_eq!(WHITE.over(BLACK, 1.0), WHITE);
        assert_eq!(WHITE.over(BLACK, 0.0), BLACK);
        assert_eq!(WHITE.over(BLACK, 0.5), Color::rgb(128, 128, 128));
    }

    #[test]
    fn contrast_is_symmetric_and_spans_the_wcag_range() {
        assert!((WHITE.contrast(BLACK) - 21.0).abs() < 0.01);
        assert!((BLACK.contrast(WHITE) - 21.0).abs() < 0.01);
        assert!((WHITE.contrast(WHITE) - 1.0).abs() < 0.001);
    }
}
