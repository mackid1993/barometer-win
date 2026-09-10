// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Persisted settings, ported from Sources/MenuBarStatsCore/Settings/.
//
// Every module has its own settings and its own item. Stacks are an addition
// to that, not a replacement for it: a stack composes readings from several
// modules into one item, and can hide those modules' own items, but CPU, GPU
// and Memory each remain individual modules with their own renderer mode.
//
// Not ported: StatusItemSpacing. It exists on macOS to make AppKit build a
// narrower shell around a status item, and there is no such shell here — the
// Windows strip is one window whose layout we own outright.

/// Which graphics adapter the GPU module reads.
///
/// The key is the module's: what identifies an adapter stably across boots
/// (a LUID does not - it is reassigned at every start). The name is kept
/// beside the key so an adapter that has since been removed can still be
/// named in the picker rather than shown as a blank.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum GpuChoice {
    /// The adapter with the most memory of its own - the discrete card where
    /// there is one - as modules/gpu.rs picks it.
    #[default]
    Automatic,
    Adapter { key: String, name: String },
}

/// Graph styles available to module renderers.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum GraphStyle {
    #[default]
    Line,
    Area,
    Bars,
}

impl GraphStyle {
    pub fn raw_value(self) -> &'static str {
        match self {
            GraphStyle::Line => "line",
            GraphStyle::Area => "area",
            GraphStyle::Bars => "bars",
        }
    }

    pub fn from_raw(raw: &str) -> Option<GraphStyle> {
        match raw {
            "line" => Some(GraphStyle::Line),
            "area" => Some(GraphStyle::Area),
            "bars" => Some(GraphStyle::Bars),
            _ => None,
        }
    }
}

/// Built-in complete appearance palettes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AppearancePreset {
    #[default]
    System,
    Ocean,
    Sunset,
    Forest,
    Neon,
    Custom,
}

impl AppearancePreset {
    pub fn raw_value(self) -> &'static str {
        match self {
            AppearancePreset::System => "system",
            AppearancePreset::Ocean => "ocean",
            AppearancePreset::Sunset => "sunset",
            AppearancePreset::Forest => "forest",
            AppearancePreset::Neon => "neon",
            AppearancePreset::Custom => "custom",
        }
    }

    pub fn from_raw(raw: &str) -> Option<AppearancePreset> {
        match raw {
            "system" => Some(AppearancePreset::System),
            "ocean" => Some(AppearancePreset::Ocean),
            "sunset" => Some(AppearancePreset::Sunset),
            "forest" => Some(AppearancePreset::Forest),
            "neon" => Some(AppearancePreset::Neon),
            "custom" => Some(AppearancePreset::Custom),
            _ => None,
        }
    }
}

/// Whether Barometer follows the system appearance or picks one.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum InterfaceAppearance {
    #[default]
    System,
    Light,
    Dark,
}

impl InterfaceAppearance {
    pub fn raw_value(self) -> &'static str {
        match self {
            InterfaceAppearance::System => "system",
            InterfaceAppearance::Light => "light",
            InterfaceAppearance::Dark => "dark",
        }
    }

    pub fn from_raw(raw: &str) -> Option<InterfaceAppearance> {
        match raw {
            "system" => Some(InterfaceAppearance::System),
            "light" => Some(InterfaceAppearance::Light),
            "dark" => Some(InterfaceAppearance::Dark),
            _ => None,
        }
    }

    /// Name shown in settings.
    pub fn display_name(self) -> &'static str {
        match self {
            InterfaceAppearance::System => "System",
            InterfaceAppearance::Light => "Light",
            InterfaceAppearance::Dark => "Dark",
        }
    }
}

/// Supported readout type weights.
///
/// The macOS app offers regular, medium and semibold. Bold is added here
/// because Windows taskbars are more varied in background than a Mac menu bar
/// - translucent over an arbitrary wallpaper, tinted with an accent color -
/// and there are backgrounds where semibold is still not enough separation.
/// Light is the addition at the other end, and its reason is on the variant.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FontWeight {
    /// Segoe UI has a real Light face, and at the sizes a taskbar uses it is
    /// the one weight that reads clearly lighter than regular.
    Light,
    #[default]
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl FontWeight {
    pub fn raw_value(self) -> &'static str {
        match self {
            FontWeight::Light => "light",
            FontWeight::Regular => "regular",
            FontWeight::Medium => "medium",
            FontWeight::Semibold => "semibold",
            FontWeight::Bold => "bold",
        }
    }

    /// The DirectWrite weight this maps to.
    pub fn dwrite_weight(self) -> u32 {
        match self {
            FontWeight::Light => 300,
            FontWeight::Regular => 400,
            FontWeight::Medium => 500,
            FontWeight::Semibold => 600,
            FontWeight::Bold => 700,
        }
    }

    pub fn from_raw(raw: &str) -> Option<FontWeight> {
        match raw {
            "light" => Some(FontWeight::Light),
            "regular" => Some(FontWeight::Regular),
            "medium" => Some(FontWeight::Medium),
            "semibold" => Some(FontWeight::Semibold),
            "bold" => Some(FontWeight::Bold),
            _ => None,
        }
    }
}

/// Settings common to every module.
///
/// Each module owns one of these and has its own item on the strip. `mode` is
/// a renderer identifier the module interprets: CPU understands a percentage,
/// a label over a value, a history graph, per-core bars and an icon with a
/// value; GPU understands a percentage, a graph, or CPU and GPU rows together.
/// The valid modes therefore live with each module's renderer, not here, which
/// is why this is a string rather than a shared enum.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleSettings {
    /// Whether this module's own item is visible.
    ///
    /// Independent of stacks. A module can be shown on its own, be part of a
    /// stack, or both; a stack with `hides_source_items` set is what turns the
    /// individual item off.
    pub is_enabled: bool,
    /// Renderer mode identifier, interpreted by the module.
    pub mode: String,
    /// Sampling interval in seconds.
    pub interval: f64,
    pub graph_style: GraphStyle,
    /// Whether text renderers reserve a stable width.
    pub uses_fixed_width: bool,
    /// Whether the detail panel includes process rows.
    pub shows_processes: bool,
    /// Maximum process rows in a panel.
    pub process_count: u32,
    /// Light-appearance color, hexadecimal.
    pub light_color: String,
    /// Dark-appearance color, hexadecimal.
    pub dark_color: String,
    /// Graph stroke colors. None falls back to the normal role.
    pub graph_light_color: Option<String>,
    pub graph_dark_color: Option<String>,
    /// Graph fill colors. None falls back to the graph role.
    pub fill_light_color: Option<String>,
    pub fill_dark_color: Option<String>,
    /// Warning colors. None falls back to the application warning role.
    pub warning_light_color: Option<String>,
    pub warning_dark_color: Option<String>,
    /// Critical colors. None falls back to the application critical role.
    pub critical_light_color: Option<String>,
    pub critical_dark_color: Option<String>,
}

impl Default for ModuleSettings {
    fn default() -> Self {
        ModuleSettings {
            // Off by default: the strip starts quiet and the user chooses what
            // appears, rather than arriving full and needing pruning.
            is_enabled: false,
            mode: "percentage".to_string(),
            interval: 3.0,
            graph_style: GraphStyle::Line,
            uses_fixed_width: true,
            shows_processes: true,
            process_count: 5,
            light_color: "#2F7CF6".to_string(),
            dark_color: "#6BA4FF".to_string(),
            graph_light_color: None,
            graph_dark_color: None,
            fill_light_color: None,
            fill_dark_color: None,
            warning_light_color: None,
            warning_dark_color: None,
            critical_light_color: None,
            critical_dark_color: None,
        }
    }
}

impl ModuleSettings {
    /// Enabled, with a renderer mode.
    pub fn enabled(mode: &str) -> Self {
        ModuleSettings { is_enabled: true, mode: mode.to_string(), ..Default::default() }
    }

    /// The graph stroke color for an appearance, falling back to the normal
    /// role when no graph color is set.
    pub fn graph_color(&self, dark: bool) -> &str {
        let specific = if dark { &self.graph_dark_color } else { &self.graph_light_color };
        specific.as_deref().unwrap_or(if dark { &self.dark_color } else { &self.light_color })
    }

    /// The graph fill color, falling back to the graph role, which itself
    /// falls back to the normal role.
    pub fn fill_color(&self, dark: bool) -> &str {
        let specific = if dark { &self.fill_dark_color } else { &self.fill_light_color };
        specific.as_deref().unwrap_or_else(|| self.graph_color(dark))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_enums_round_trip() {
        for style in [GraphStyle::Line, GraphStyle::Area, GraphStyle::Bars] {
            assert_eq!(GraphStyle::from_raw(style.raw_value()), Some(style));
        }
        for preset in [
            AppearancePreset::System,
            AppearancePreset::Ocean,
            AppearancePreset::Sunset,
            AppearancePreset::Forest,
            AppearancePreset::Neon,
            AppearancePreset::Custom,
        ] {
            assert_eq!(AppearancePreset::from_raw(preset.raw_value()), Some(preset));
        }
    }

    #[test]
    fn a_module_is_its_own_item_and_starts_disabled() {
        let settings = ModuleSettings::default();
        assert!(!settings.is_enabled);
        assert_eq!(settings.mode, "percentage");
    }

    #[test]
    fn graph_color_falls_back_to_the_normal_role() {
        let settings = ModuleSettings::default();
        assert_eq!(settings.graph_color(true), "#6BA4FF");
        assert_eq!(settings.graph_color(false), "#2F7CF6");
    }

    #[test]
    fn fill_falls_back_through_graph_to_normal() {
        let mut settings = ModuleSettings::default();
        assert_eq!(settings.fill_color(true), "#6BA4FF");
        settings.graph_dark_color = Some("#112233".into());
        assert_eq!(settings.fill_color(true), "#112233", "fill should defer to graph");
        settings.fill_dark_color = Some("#445566".into());
        assert_eq!(settings.fill_color(true), "#445566");
    }
}

/// How the strip's text is drawn.
///
/// This has no macOS counterpart. The Mac menu bar draws in the system face
/// and offers no family choice, so there was nothing to port; on Windows the
/// taskbar is a more varied surface and people reasonably want to match it.
#[derive(Clone, Debug, PartialEq)]
pub struct StripFont {
    /// Family name, resolved by DirectWrite at draw time.
    ///
    /// Stored as written rather than validated here: a font can be installed
    /// or removed while Barometer runs, so the only honest check is the one
    /// made when drawing, which falls back to the default face.
    pub family: String,
    /// The family the labels are drawn in, when it is not the values' family.
    ///
    /// None means "whatever the values use", which is the sensible default
    /// and what every version before this did. It is a family of its own
    /// rather than only a weight because on Windows a weight often *is* a
    /// family - "Segoe UI Semibold" is its own, with faces somebody drew -
    /// and because a heading in a different face from its value is a
    /// perfectly ordinary piece of typography that the weight control alone
    /// cannot express.
    pub heading_family: Option<String>,
    /// The weight of the values - the numbers themselves.
    pub weight: FontWeight,
    /// The weight of the labels above them.
    ///
    /// Separate from `weight` because a strip reads as two kinds of text, not
    /// one: `CPU` is a heading naming what the column is, and `24%` is the
    /// thing you actually came to look at. Setting both from one control means
    /// either the labels are as loud as the values or the values are as quiet
    /// as the labels, and at nine pixels on a taskbar that difference is most
    /// of the legibility.
    ///
    /// Only the label row uses it, so a network column showing two rates has
    /// both of them in `weight` - neither is a heading.
    pub heading_weight: FontWeight,
    /// The size of the text on the strip, in DIPs.
    ///
    /// The size itself, not a ceiling on an automatic one - that version was
    /// taken out because a control that could only ever lower a figure the
    /// strip had already chosen confused more than it helped. The strip is
    /// always two rows; a size two rows of which the bar cannot hold is held
    /// down to the largest it can. See `taskbar::Density`.
    pub size_dip: f32,
}

impl Default for StripFont {
    fn default() -> Self {
        StripFont {
            // The Windows 11 UI face. Segoe UI Variable Text is the small-size
            // optical size, which is what a taskbar readout is.
            family: "Segoe UI Variable Text".to_string(),
            heading_family: None,
            weight: FontWeight::Regular,
            heading_weight: FontWeight::Semibold,
            size_dip: crate::taskbar::TEXT_DIP,
        }
    }
}

impl StripFont {
    /// The face to fall back to when the chosen family is not installed.
    pub const FALLBACK_FAMILY: &'static str = "Segoe UI";

    /// The ends of the Text size slider.
    ///
    /// Wide on purpose: the size is the user's to choose. Six is where
    /// tabular digits stop being digits at 100%, and twenty-four is two rows
    /// in a bar seventy DIPs tall, more than Windows draws today. What a
    /// given bar can actually hold is decided by `taskbar::Density`, not
    /// here - a size two rows of which have no room is held to the largest
    /// that fits, and the slider's caption says so.
    pub const MIN_SIZE_DIP: f32 = 6.0;
    pub const MAX_SIZE_DIP: f32 = 24.0;
}

#[cfg(test)]
mod font_tests {
    use super::*;

    #[test]
    fn weights_round_trip_and_map_to_dwrite() {
        for weight in
            [FontWeight::Regular, FontWeight::Medium, FontWeight::Semibold, FontWeight::Bold]
        {
            assert_eq!(FontWeight::from_raw(weight.raw_value()), Some(weight));
        }
        assert_eq!(FontWeight::Semibold.dwrite_weight(), 600);
        assert_eq!(FontWeight::Bold.dwrite_weight(), 700);
    }
}
