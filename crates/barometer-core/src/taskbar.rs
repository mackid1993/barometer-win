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

/// How the strip lays itself out: its text size. The rows are always two.
///
/// The size is the user's, one figure for the whole strip, and it starts
/// small. The Mac app steps its type down from 12 as widgets are added; that
/// ladder is not ported, and neither is the ceiling an earlier version of
/// this port put on top of it, which could only ever lower the automatic
/// size and confused more than it helped. On a taskbar the readout's width
/// is paid for by the task buttons beside it, which the shell gives up a
/// whole button at a time, so the size is a plain choice with a small
/// default rather than something that grows whenever the strip has fewer
/// items. Nine is the bottom of the Mac ladder and the smallest size at
/// which two rows stay legible.
///
/// Two rows, always: a label over its value. There used to be a one-row
/// fallback for a bar too short for two, and a size the bar could not hold
/// in two rows fell into it - the whole strip turned sideways, every label
/// beside its number, wider than anything the user had asked for. That is
/// not a layout anybody wants, so there is no such layout: a size two rows
/// cannot fit is held down to one they can, and the caption under the
/// slider says so.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Density {
    /// Text size in DIPs.
    pub text_dip: f32,
}

/// The size a fresh install draws at, in DIPs. See [`Density`].
pub const TEXT_DIP: f32 = 9.0;

/// The largest a single row of type may be, as a share of the bar's height,
/// with the strip drawing two of them. A line's box is about 1.33 times its
/// size, so two rows at this share fill nine tenths of the bar and leave the
/// rest as margin above and below.
///
/// A proportion rather than a table of thresholds, because 26H2 lets the
/// taskbar be several heights and a table only ever covers the ones somebody
/// thought to measure. It also has to be measured in DIPs: the small taskbar
/// at 150% is 48 physical pixels, exactly what the large one is at 100%, and
/// reading the two as the same bar gets both wrong.
const TWO_ROW_FRACTION: f32 = 0.34;

impl Density {
    /// The size the strip draws at, for a bar height and the size the user
    /// set.
    ///
    /// The user's size wherever two rows of it fit the bar, which at the
    /// default size is every height Windows 11 offers, the 32dip small
    /// taskbar included. Where two rows of it do not fit, the largest size
    /// two rows do: the strip never trades its second row for a bigger
    /// first one.
    pub fn choose(height_dip: f32, text_dip: f32) -> Density {
        let asked = text_dip.clamp(
            crate::settings::StripFont::MIN_SIZE_DIP,
            crate::settings::StripFont::MAX_SIZE_DIP,
        );
        let fits = height_dip * TWO_ROW_FRACTION;
        Density { text_dip: asked.min(fits).max(crate::settings::StripFont::MIN_SIZE_DIP) }
    }

    /// Whether the bar held the size down to fit two rows.
    pub fn held_to_the_bar(&self, asked_dip: f32) -> bool {
        self.text_dip < asked_dip.clamp(
            crate::settings::StripFont::MIN_SIZE_DIP,
            crate::settings::StripFont::MAX_SIZE_DIP,
        )
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
    fn the_size_is_the_users_wherever_two_rows_of_it_fit() {
        for height in [32.0, 48.0, 64.0] {
            assert_eq!(Density::choose(height, TEXT_DIP).text_dip, TEXT_DIP);
        }
        assert_eq!(Density::choose(48.0, 12.0).text_dip, 12.0);
        assert!(!Density::choose(48.0, 12.0).held_to_the_bar(12.0));
    }

    #[test]
    fn an_absurd_size_is_held_to_the_legible_range() {
        assert_eq!(Density::choose(48.0, 1.0).text_dip, StripFont::MIN_SIZE_DIP);
        assert!(Density::choose(400.0, 400.0).text_dip <= StripFont::MAX_SIZE_DIP);
    }

    #[test]
    fn the_default_size_fits_two_rows_in_every_taskbar_windows_offers() {
        // 32dip is the small taskbar 26H2 brought back, 48 the default. Two
        // lines plus their leading have to leave the small bar some margin,
        // or the readout looks pasted over the taskbar instead of set into it.
        assert!(TEXT_DIP * 2.0 * 1.33 < 32.0);
        assert!(32.0 * TWO_ROW_FRACTION >= TEXT_DIP);
    }

    #[test]
    fn a_size_two_rows_cannot_fit_is_held_down_never_laid_out_as_one_row() {
        let d = Density::choose(32.0, 24.0);
        assert!(d.text_dip < 24.0);
        assert!(d.held_to_the_bar(24.0));
        // Two rows of the held size still fit inside the bar.
        assert!(d.text_dip * 2.0 * 1.33 <= 32.0);
        // The default bar has more room, so it holds less.
        assert!(Density::choose(48.0, 24.0).text_dip > d.text_dip);
    }

    #[test]
    fn a_bar_too_short_for_two_rows_at_the_smallest_size_still_gets_two_rows() {
        // Nothing Windows draws is this short; the point is that the answer
        // is a smaller size, never a different layout.
        let d = Density::choose(16.0, TEXT_DIP);
        assert_eq!(d.text_dip, StripFont::MIN_SIZE_DIP);
    }
}
