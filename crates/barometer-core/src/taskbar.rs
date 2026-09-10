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

/// How the strip lays itself out: its text size and whether it has two rows.
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
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Density {
    /// Text size in DIPs.
    pub text_dip: f32,
    /// Whether there is room for a label above a value.
    pub two_rows: bool,
}

/// The size a fresh install draws at, in DIPs. See [`Density`].
pub const TEXT_DIP: f32 = 9.0;

/// The largest a single row of type may be, as a share of the bar's height,
/// while the strip is drawing two of them.
///
/// A proportion rather than a table of thresholds, because 26H2 lets the
/// taskbar be several heights and a table only ever covers the ones somebody
/// thought to measure. It also has to be measured in DIPs: the small taskbar
/// at 150% is 48 physical pixels, exactly what the large one is at 100%, and
/// reading the two as the same bar gets both wrong.
const TWO_ROW_FRACTION: f32 = 0.31;

/// The largest one row of type may be, as a share of the bar's height.
///
/// A line's box is about 1.35 times its size, so seven tenths fills 95% of
/// the bar: the most a single row can be without its ascenders and
/// descenders leaving the taskbar. This is a physical limit, not a taste,
/// and it is the only thing that ever overrides the size the user chose.
const ONE_ROW_CAP: f32 = 0.7;

impl Density {
    /// Chooses the row count for a bar height at the size the user set.
    ///
    /// Two rows whenever the bar has room for two lines at that size. At the
    /// default size every height Windows 11 offers has, the 32dip small
    /// taskbar included; a larger size drops to one row where two no longer
    /// fit rather than shrinking back, because the size was the user's
    /// choice and a row nobody asked for is the lesser thing to lose. Only a
    /// size the bar cannot hold even in one row is reduced, to the largest
    /// that fits, since text hanging out of the taskbar is not a choice
    /// anybody meant to make.
    pub fn choose(height_dip: f32, text_dip: f32) -> Density {
        let text_dip = text_dip.clamp(
            crate::settings::StripFont::MIN_SIZE_DIP,
            crate::settings::StripFont::MAX_SIZE_DIP,
        );
        if height_dip * TWO_ROW_FRACTION >= text_dip {
            return Density { text_dip, two_rows: true };
        }
        Density { text_dip: text_dip.min(height_dip * ONE_ROW_CAP), two_rows: false }
    }

    /// Whether the bar held the size down to fit one row.
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
    fn the_size_is_the_users_whatever_the_bar() {
        for height in [24.0, 32.0, 48.0, 64.0] {
            assert_eq!(Density::choose(height, TEXT_DIP).text_dip, TEXT_DIP);
            assert_eq!(Density::choose(height, 12.0).text_dip, 12.0);
        }
    }

    #[test]
    fn an_absurd_size_is_held_to_the_legible_range() {
        assert_eq!(Density::choose(48.0, 1.0).text_dip, StripFont::MIN_SIZE_DIP);
        assert_eq!(Density::choose(48.0, 400.0).text_dip, StripFont::MAX_SIZE_DIP);
    }

    #[test]
    fn every_taskbar_windows_offers_gets_two_rows_at_the_default_size() {
        // 32dip is the small taskbar 26H2 brought back, 48 the default.
        assert!(Density::choose(32.0, TEXT_DIP).two_rows);
        assert!(Density::choose(48.0, TEXT_DIP).two_rows);
        // Two lines plus their leading have to leave the small bar some
        // margin, or the readout looks pasted over the taskbar instead of set
        // into it.
        assert!(TEXT_DIP * 2.0 * 1.35 < 32.0);
    }

    #[test]
    fn a_size_two_rows_cannot_fit_drops_to_one_rather_than_shrinking() {
        let d = Density::choose(32.0, 12.0);
        assert!(!d.two_rows);
        assert_eq!(d.text_dip, 12.0);
        // The default bar still has the room at that size.
        assert!(Density::choose(48.0, 12.0).two_rows);
    }

    #[test]
    fn a_size_the_bar_cannot_hold_in_one_row_is_held_to_the_largest_that_fits() {
        let d = Density::choose(32.0, 24.0);
        assert!(!d.two_rows);
        assert!(d.text_dip < 24.0);
        assert!(d.held_to_the_bar(24.0));
        // A line's box is about 1.35 times the size, and it has to stay inside the bar.
        assert!(d.text_dip * 1.35 <= 32.0);
        // The default bar holds the whole range in one row.
        assert_eq!(Density::choose(48.0, 24.0).text_dip, 24.0);
        assert!(!Density::choose(48.0, 24.0).held_to_the_bar(24.0));
    }

    #[test]
    fn a_bar_too_short_for_two_rows_drops_to_one_rather_than_shrinking_them() {
        let d = Density::choose(24.0, TEXT_DIP);
        assert!(!d.two_rows);
        assert_eq!(d.text_dip, TEXT_DIP);
    }
}
