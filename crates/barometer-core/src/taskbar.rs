// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Where the taskbar is, how big it is, and whether we can run there at all.

use std::mem;


use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};

/// Which screen edge the taskbar is docked to.
///
/// Values match the ABE_* constants so the conversion is a match rather than
/// arithmetic on a magic number.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

impl Edge {
    fn from_abe(value: u32) -> Option<Edge> {
        match value {
            0 => Some(Edge::Left),
            1 => Some(Edge::Top),
            2 => Some(Edge::Right),
            3 => Some(Edge::Bottom),
            _ => None,
        }
    }

    /// Whether a horizontal readout can exist on this edge at all.
    ///
    /// The strip claims width by widening the notification area, which makes
    /// Windows repack the task buttons aside. Docked left or right the tray is
    /// a vertical grid: widening it frees no horizontal space and there is
    /// nowhere for a readout to go. That is geometry, not a rendering problem,
    /// so a side edge is refused rather than accommodated.
    pub fn is_supported(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }
}

/// The taskbar as it is right now.
#[derive(Copy, Clone)]
pub struct Taskbar {
    pub edge: Edge,
    pub rect: RECT,
}

/// windows-sys derives nothing on RECT, and the four raw edges are less
/// useful in a log line than the shape they describe.
impl std::fmt::Debug for Taskbar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Taskbar")
            .field("edge", &self.edge)
            .field("width", &self.width())
            .field("height", &self.height())
            .finish()
    }
}

impl Taskbar {
    pub fn width(self) -> i32 {
        (self.rect.right - self.rect.left).max(0)
    }

    pub fn height(self) -> i32 {
        (self.rect.bottom - self.rect.top).max(0)
    }
}

/// Reads the taskbar's edge and rectangle.
///
/// Returns None when the shell does not answer, which happens if Explorer is
/// mid-restart. That is a transient: callers retry rather than concluding
/// anything from it.
pub fn taskbar() -> Option<Taskbar> {
    // SAFETY: APPBARDATA is plain data. cbSize is set as the API requires and
    // the struct outlives the call.
    let mut data: APPBARDATA = unsafe { mem::zeroed() };
    data.cbSize = mem::size_of::<APPBARDATA>() as u32;
    let ok = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) };
    if ok == 0 {
        return None;
    }
    Some(Taskbar { edge: Edge::from_abe(data.uEdge)?, rect: data.rc })
}

/// How much room the strip has, and therefore how big it draws.
///
/// The Mac app steps type down as more widgets are enabled. Windows adds a
/// second axis, because 26H2 brought back the small taskbar: the strip has to
/// shrink into a short bar and grow into a tall one. Both axes are consulted
/// and the smaller answer wins, since a size that fits the widget count but
/// not the bar is no use.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Density {
    /// Text size in DIPs.
    pub text_dip: f32,
    /// Whether there is room for a label above a value.
    pub two_rows: bool,
}

/// Text sizes, largest first, matching the Mac app's ladder.
const TEXT_LADDER_DIP: [f32; 4] = [12.0, 11.0, 10.0, 9.0];

/// The widget-count rung, from the Mac app: 12pt up to 8 widgets, then 11, 10
/// and 9 as the strip fills up.
fn rung_for_widgets(widgets: usize) -> usize {
    match widgets {
        0..=8 => 0,
        9..=11 => 1,
        12..=14 => 2,
        _ => 3,
    }
}

/// Type size as a fraction of the bar's height, for one row and for two.
///
/// A proportion rather than a table of thresholds, because 26H2 lets the
/// taskbar be several heights and a table only ever covers the ones somebody
/// thought to measure. It also has to be measured in DIPs: the small taskbar
/// at 150% is 48 physical pixels, exactly what the large one is at 100%, and
/// reading the two as the same bar gets both wrong.
///
/// The figures are set against the small bar, which is the hard case at 32dip.
/// They are deliberately generous. The first pass sized two rows at about 60%
/// of the bar, which is correct in the sense that it fits and useless in the
/// sense that nobody could read it: on a 32dip bar that is nine device pixels
/// of cap height, well under the clock sitting next to it. A readout too small
/// to read at a glance is not a readout.
///
/// So one row now lands a little above Windows' own clock, and two rows fill
/// most of the bar with a margin left at top and bottom. This is the *default*
/// and the user can move it either way; it is not a limit.
const ONE_ROW_FRACTION: f32 = 0.42;
const TWO_ROW_FRACTION: f32 = 0.31;

/// Below this the second row is not worth having, and the strip shows values
/// only. Two rows of type this small are a gray smear at arm's length, and one
/// legible row beats two nobody can read.
const TWO_ROW_FLOOR_DIP: f32 = 9.0;

/// The height axis: a type size taken from the bar, and whether two rows fit.
///
/// Tried at two rows first, and dropped to one only when two would need type
/// under the floor. A short bar therefore gets larger, single-row text rather
/// than two illegible ones.
fn size_for_height(height_dip: f32) -> (f32, bool) {
    let two_row = height_dip * TWO_ROW_FRACTION;
    if two_row >= TWO_ROW_FLOOR_DIP {
        (two_row, true)
    } else {
        (height_dip * ONE_ROW_FRACTION, false)
    }
}

impl Density {
    /// Chooses a text size and row count, then applies the user's ceiling.
    ///
    /// Two automatic axes and one manual limit, in that order. The automatic
    /// pair decide what fits; the ceiling only ever makes it smaller, so a
    /// preference can never overflow a taskbar it was not set on.
    pub fn choose_with_font(
        height_dip: f32,
        widgets: usize,
        font: &crate::settings::StripFont,
    ) -> Density {
        let mut density = Density::choose(height_dip, widgets);
        density.text_dip = font.clamp(density.text_dip);
        density
    }

    /// Chooses a text size and row count for a bar height and a widget count.
    ///
    /// The bar decides the size and the widget count can only lower it. That
    /// order matters: a crowded strip in a tall bar must still fit, and a
    /// sparse strip in a short one must not grow out of it.
    pub fn choose(height_dip: f32, widgets: usize) -> Density {
        let (from_height, two_rows) = size_for_height(height_dip);
        let ceiling = TEXT_LADDER_DIP[rung_for_widgets(widgets)];
        let text_dip = from_height
            .min(ceiling)
            .clamp(crate::settings::StripFont::MIN_SIZE_DIP, crate::settings::StripFont::MAX_SIZE_DIP);
        Density { text_dip, two_rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::StripFont;

    #[test]
    fn side_edges_are_refused() {
        assert!(!Edge::Left.is_supported());
        assert!(!Edge::Right.is_supported());
        assert!(Edge::Top.is_supported());
        assert!(Edge::Bottom.is_supported());
    }

    #[test]
    fn a_tall_bar_with_few_widgets_gets_two_rows_that_fit_inside_it() {
        let d = Density::choose(48.0, 3);
        assert!(d.two_rows);
        // Two lines plus their leading have to leave the bar some margin, or
        // the readout looks pasted over the taskbar instead of set into it.
        assert!(d.text_dip * 2.0 * 1.35 < 48.0, "{} is too tall for 48", d.text_dip);
        // And it must not be so timid that it is smaller than the clock beside it.
        assert!(d.text_dip >= 9.0);
    }

    #[test]
    fn a_small_bar_drops_to_one_row_rather_than_two_illegible_ones() {
        let d = Density::choose(24.0, 3);
        assert!(!d.two_rows);
        assert!(d.text_dip >= StripFont::MIN_SIZE_DIP);
        assert!(d.text_dip < 12.0);
    }

    #[test]
    fn the_type_grows_and_shrinks_with_the_bar() {
        let small = Density::choose(32.0, 3);
        let large = Density::choose(48.0, 3);
        assert!(large.text_dip > small.text_dip);
    }

    #[test]
    fn the_font_ceiling_only_ever_shrinks_the_automatic_choice() {
        let roomy = Density::choose(48.0, 2);
        let capped = Density::choose_with_font(48.0, 2, &StripFont { max_size_dip: 8.0, ..Default::default() });
        assert_eq!(capped.text_dip, 8.0);
        assert!(roomy.text_dip > capped.text_dip);
        // A generous ceiling cannot grow text past what the bar admits.
        let bare = Density::choose(24.0, 2);
        let generous = Density::choose_with_font(24.0, 2, &StripFont { max_size_dip: 18.0, ..Default::default() });
        assert_eq!(generous.text_dip, bare.text_dip);
    }

    #[test]
    fn the_smaller_of_the_two_axes_wins() {
        // A tall bar cannot rescue a strip crowded with widgets.
        let crowded = Density::choose(48.0, 15);
        assert_eq!(crowded.text_dip, 9.0);
        // And few widgets cannot rescue a short bar.
        let short = Density::choose(26.0, 2);
        assert!(short.text_dip < Density::choose(48.0, 2).text_dip);
    }
}
