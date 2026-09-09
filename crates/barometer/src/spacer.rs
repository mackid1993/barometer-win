// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Claiming space on the taskbar.
//
// The hard-won details are the point. Almost none of this behavior is
// discoverable from the API.
//
// How it works: the Windows 11 taskbar is a two-column layout, and the task
// button strip's right edge *is* the notification area's left edge. Widen the
// tray and the strip repacks to stay clear of it; it never encroaches on the
// tray. So Barometer adds fully transparent placeholder icons to the tray, the
// tray gets wider, Windows itself moves the buttons aside, and the readout
// sits on the space that opened up.
//
// This is unlike placeholder task *buttons*, which Windows demotes into the
// overflow flyout as soon as the taskbar fills. Tray icons are never pushed
// out, which is what makes this the only mechanism that genuinely reserves
// space - and why the readout can no longer overlap a button at all.

/// Most placeholders we will ever add.
///
/// Deliberately generous: the readout can carry many items, and at roughly 42
/// physical pixels a slot, 48 covers about 2000 pixels of strip.
pub const MAX_ICONS: usize = 48;

/// Width one tray icon occupies. A seed only: the real pitch is measured once
/// the icons are on screen, because this estimate runs high. On the author's
/// hardware the DPI-derived guess was 63 against a real 42.
pub const ICON_SLOT_WIDTH: i32 = 42;

/// Consecutive samples that must see another program's icon inside our region
/// before we believe it.
///
/// Two. The fastest query interval is 250ms, so this asks for about half a
/// second of overlap: long enough to ignore the momentary re-layout when the
/// overflow chevron is clicked, far shorter than any real drag.
pub const OBSTRUCT_CONFIRM: u32 = 2;

/// Adding an icon can fail: the shell refuses, or the tray is not ready.
/// Retrying every tick makes the tray flicker, so failures back off - but never
/// give up, or the region stays too narrow forever and the readout never moves.
pub const RETRY_COOLDOWN_MS: u64 = 2000;
pub const MAX_BACKOFF_STEPS: u32 = 8;

/// Cap on GUID block rotations. See [`icon_guid`].
///
/// Toggling the feature off and on within one Windows session consumes blocks,
/// and doing that a dozen times is normal, so this cannot be small.
pub const MAX_GUID_BLOCKS: u32 = 64;

/// Icons added but no usable region appearing within this long means the batch
/// of identities is dead, and a fresh block is tried.
pub const ROTATE_GRACE_MS: u64 = 6000;

/// The base identity for Barometer's placeholders.
///
/// Fixed and reused every run rather than generated per launch, so the shell
/// recognizes the same icons each time instead of accumulating a fresh pile of
/// registry entries on every start.
/// The low 32 bits are deliberately zero: the block and slot are OR-ed into
/// them, and a base with bits set there would collide - block 16 and block 0
/// producing the same identity, so two placeholders the shell treats as one
/// icon reserving one slot.
// The first identity set was registered without NIF_TIP. Explorer remembers
// that first empty tooltip, so adding a name later does not make those buttons
// identifiable through UI Automation. A new fixed prefix gives the named
// placeholders fresh identities once, without generating new ones per run.
const LEGACY_GUID_BASE: u128 = 0x4241_524F_4D45_5445_5253_5452_0000_0000;
const GUID_BASE: u128 = 0x4241_524F_4D45_5445_5253_5432_0000_0000;

/// The GUID for one placeholder.
///
/// Carries a *block* number as well as a slot index, and the block is the
/// subtle part. Once a batch of GUIDs has had its registry entries deleted
/// from outside, the shell's memory and the registry disagree: it still
/// recognizes those identities but will never write their entries again, so
/// the batch can never be shown and is permanently dead. Moving to another
/// block is a fresh set of identities that sidesteps it.
///
/// This is also why entries are never cleaned up on an ordinary exit. Deleting
/// them is what creates the condition.
pub fn icon_guid(block: u32, index: usize) -> u128 {
    GUID_BASE | ((block as u128) << 16) | (index as u128 & 0xFFFF)
}

/// How many whole slots a width needs.
///
/// Ceiling, never rounding: the reservation is quantized to tray slots, and
/// asking for less than the readout needs puts it back on top of a task
/// button, which is the failure this exists to prevent.
pub fn slots_for_width(width: i32, slot_width: i32) -> usize {
    if width <= 0 || slot_width <= 0 {
        return 0;
    }
    // Widened to 64 bits before the ceiling arithmetic: width comes from a
    // measured layout, and (i32::MAX + slot_width) overflows.
    let slots = (width as i64 + slot_width as i64 - 1) / slot_width as i64;
    (slots.max(0) as usize).min(MAX_ICONS)
}

/// The DPI-scaled seed pitch for a monitor.
pub fn dpi_seed_width(dpi: u32) -> i32 {
    ICON_SLOT_WIDTH * dpi.max(96) as i32 / 96
}

/// The floor for a measured pitch, given a seed.
///
/// Half the estimate. The estimate measurably runs high, so anything below
/// this is a bad measurement rather than a narrow tray, and trusting it would
/// add far too few icons.
pub fn slot_width_floor(dpi_seed: i32) -> i32 {
    (dpi_seed / 2).max(1)
}

/// Windows' GUID layout, for the shell and registry calls.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    /// Splits the 128-bit identity into Windows' four fields, most significant
    /// first, so the textual form below matches the bit layout.
    pub fn from_u128(value: u128) -> Guid {
        let bytes = value.to_be_bytes();
        Guid {
            data1: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            data2: u16::from_be_bytes([bytes[4], bytes[5]]),
            data3: u16::from_be_bytes([bytes[6], bytes[7]]),
            data4: [
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ],
        }
    }

    /// The registry's spelling: braced, upper case, hyphenated.
    ///
    /// This has to match what the shell writes into `IconGuid` byte for byte,
    /// because that comparison is the only thing standing between marking our
    /// own placeholder always-visible and doing it to somebody else's icon.
    pub fn to_registry_string(self) -> String {
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_guid_is_stable_for_a_slot_and_block() {
        assert_eq!(icon_guid(0, 3), icon_guid(0, 3));
    }

    #[test]
    fn no_slot_and_block_pair_collides() {
        // A collision would give two placeholders the same identity, and the
        // shell would treat them as one icon reserving one slot.
        let mut seen = HashSet::new();
        for block in 0..MAX_GUID_BLOCKS {
            for index in 0..MAX_ICONS {
                assert!(seen.insert(icon_guid(block, index)), "collision at {block}/{index}");
            }
        }
    }

    #[test]
    fn a_width_always_rounds_up_to_whole_slots() {
        // Reserving less than the readout needs puts it back over a button.
        assert_eq!(slots_for_width(1, 42), 1);
        assert_eq!(slots_for_width(42, 42), 1);
        assert_eq!(slots_for_width(43, 42), 2);
    }

    #[test]
    fn nothing_is_reserved_for_nothing() {
        assert_eq!(slots_for_width(0, 42), 0);
        assert_eq!(slots_for_width(-10, 42), 0);
        assert_eq!(slots_for_width(100, 0), 0);
    }

    #[test]
    fn the_slot_count_is_capped() {
        assert_eq!(slots_for_width(i32::MAX, 1), MAX_ICONS);
    }

    #[test]
    fn a_guid_formats_the_way_the_registry_spells_it() {
        // The shell writes IconGuid in this exact form, and comparing against
        // it is what keeps us from promoting another program's icon.
        let guid = Guid::from_u128(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF);
        assert_eq!(guid.to_registry_string(), "{01234567-89AB-CDEF-0123-456789ABCDEF}");
    }

    #[test]
    fn distinct_slots_format_to_distinct_guids() {
        let a = Guid::from_u128(icon_guid(0, 0)).to_registry_string();
        let b = Guid::from_u128(icon_guid(0, 1)).to_registry_string();
        let c = Guid::from_u128(icon_guid(1, 0)).to_registry_string();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn both_placeholder_identity_generations_remain_recognizable() {
        let current = Guid::from_u128(icon_guid(3, 7)).to_registry_string();
        let legacy = Guid::from_u128(LEGACY_GUID_BASE | (3 << 16) | 7).to_registry_string();
        assert!(is_ours(&current));
        assert!(is_ours(&legacy));
    }

    #[test]
    fn the_pitch_seed_scales_with_dpi_and_its_floor_is_half() {
        assert_eq!(dpi_seed_width(96), 42);
        assert_eq!(dpi_seed_width(144), 63);
        assert_eq!(slot_width_floor(63), 31);
        assert_eq!(slot_width_floor(0), 1, "a floor of zero would accept any measurement");
    }
}

/// A screen rectangle, in physical pixels.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// Whether two rectangles share any area at all.
    pub fn intersects(self, other: Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    /// The smallest rectangle containing both, ignoring empty ones.
    /// Whether this rectangle lies entirely inside `other`.
    pub fn within(self, other: Rect) -> bool {
        !self.is_empty()
            && self.left >= other.left
            && self.right <= other.right
            && self.top >= other.top
            && self.bottom <= other.bottom
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// Whether a registry GUID string is one of ours.
///
/// Every placeholder identity has a fixed base with a block and an index in
/// its low 32 bits. The legacy base remains recognizable so repair and cleanup
/// can still find identities created before the UI Automation name was added.
pub fn is_ours(registry_guid: &str) -> bool {
    // The low 32 bits are the only part that varies, which is the last eight
    // hex digits before the closing brace.
    [GUID_BASE, LEGACY_GUID_BASE].into_iter().any(|value| {
        let base = Guid::from_u128(value).to_registry_string();
        let fixed = base.len() - 9;
        registry_guid.len() == base.len()
            && registry_guid[..fixed].eq_ignore_ascii_case(&base[..fixed])
    })
}

/// The pitch of one tray slot, from the placeholders themselves.
///
/// The distance from one placeholder's left edge to the next one's, not the
/// width of a rectangle and not a span divided by a count. A rectangle's
/// width is whatever the shell felt like reporting for that icon at that
/// moment - measured live: 42, then 36, then 45, then 46 over four seconds
/// while the tray settled after a batch of adds - and a single 36 latched
/// as "the smallest credible measurement" asked for eleven slots where ten
/// would do, which is a slot of empty taskbar beside the readout for the
/// rest of the run. The spacing between neighbors is the slot itself, and
/// it reads 42 whatever width the rectangles report.
///
/// Only spacings that do not overlap count, since neighbors squeezed
/// together mid-animation are not a pitch either. One placeholder alone has
/// no neighbor and yields nothing - not its width. Its width was the whole
/// problem: the first placeholder of a run stands alone for a tick, the
/// shell reports it 36 wide, and 36 latched as the pitch asks for a slot
/// too many for the rest of the run. The DPI seed stands until there are
/// two to measure between.
pub fn slot_pitch(rects: &[Rect]) -> Option<i32> {
    let mut placed: Vec<Rect> = rects.iter().copied().filter(|r| !r.is_empty()).collect();
    if placed.is_empty() {
        return None;
    }
    placed.sort_unstable_by_key(|r| r.left);
    let widest = placed.iter().map(|r| r.right - r.left).max().unwrap_or(0);
    let mut spacings: Vec<i32> = placed
        .windows(2)
        .map(|pair| pair[1].left - pair[0].left)
        .filter(|spacing| *spacing >= widest)
        .collect();
    if spacings.is_empty() {
        return None;
    }
    // The lower median. A foreign icon between two of ours makes one
    // spacing larger, never smaller, and with only two spacings the upper
    // median would be the gap. Larger cannot latch anyway - the pitch only
    // ever narrows - so erring low is erring harmless.
    spacings.sort_unstable();
    Some(spacings[(spacings.len() - 1) / 2])
}

/// Tracks whether another program's icon is sitting inside our reserved space.
///
/// Tray icons can be reordered by dragging, so somebody else's icon can land
/// between two of our placeholders, *inside* the region. Drawing the readout
/// there as usual would cover that icon: invisible and unclickable until the
/// readout moves away. So while this is set the caller hides the readout, and
/// once the icon is dragged out it clears and the readout comes back on its
/// own.
///
/// The hysteresis is deliberately asymmetric, and both halves were arrived at
/// by watching it happen.
///
/// **Slow to hide.** Opening or closing the "Show Hidden Icons" flyout makes
/// the tray re-lay out briefly, and some icon's rectangle lands inside the
/// region for an instant. Hiding on that one sample makes the readout blink
/// every time the user clicks the chevron - measured at one blink on open and
/// one on close, about 100ms each, with no actual change of position. An icon
/// genuinely dragged in stays there, so waiting a sample or two costs nothing.
///
/// **Instant to return.** The moment the overlap clears, the readout comes
/// back. There is no reason to make someone wait to see their own monitor
/// again, and the failure this guards against has already ended.
#[derive(Default, Debug)]
pub struct Obstruction {
    streak: u32,
    obstructed: bool,
}

impl Obstruction {
    /// Folds in one sample: our region, and the rectangles of every other
    /// program's tray icon.
    pub fn observe(&mut self, reserved: Rect, others: &[Rect]) -> bool {
        let overlapping =
            !reserved.is_empty() && others.iter().any(|other| reserved.intersects(*other));
        self.observe_overlap(overlapping)
    }

    /// Folds in one sample expressed as a span.
    ///
    /// A deliberate departure from the original, which enumerated every button
    /// in the taskbar through UI Automation to see what else was inside the
    /// region. The placeholders are contiguous, so their union should be
    /// exactly `count * slot_width` wide; anything wider means something has
    /// been dragged in between them, which is precisely the case that matters.
    ///
    /// It costs no COM, no apartment, no background thread and no cross-process
    /// round trips, where the enumeration cost one to two hundred of them per
    /// query and made the taskbar visibly sluggish until it was cached. It sees
    /// less - an icon beside the region rather than inside it goes unnoticed -
    /// but an icon beside the region is not covered by the readout, so there is
    /// nothing to notice.
    pub fn observe_span(&mut self, region: Rect, count: usize, slot_width: i32) -> bool {
        if region.is_empty() || count == 0 || slot_width <= 0 {
            return self.observe_overlap(false);
        }
        let expected = count as i32 * slot_width;
        // Half a slot of tolerance: the tray does not always pack to the pixel,
        // and a rounding difference is not an intruder.
        let intruded = (region.right - region.left) > expected + slot_width / 2;
        self.observe_overlap(intruded)
    }

    fn observe_overlap(&mut self, overlapping: bool) -> bool {
        if overlapping {
            if self.streak < OBSTRUCT_CONFIRM {
                self.streak += 1;
            }
        } else {
            self.streak = 0;
        }

        self.obstructed = self.streak >= OBSTRUCT_CONFIRM;
        self.obstructed
    }

    /// Whether the readout should currently be hidden.
    pub fn is_obstructed(&self) -> bool {
        self.obstructed
    }
}

#[cfg(test)]
mod obstruction_tests {
    use super::*;

    fn rect(left: i32, right: i32) -> Rect {
        Rect { left, top: 0, right, bottom: 32 }
    }

    #[test]
    fn a_single_overlapping_sample_does_not_hide_the_readout() {
        // This is the chevron blink: the tray re-lays out for about 100ms and
        // some icon lands inside the region for one sample.
        let mut state = Obstruction::default();
        assert!(!state.observe(rect(100, 300), &[rect(150, 200)]));
        assert!(!state.is_obstructed());
    }

    #[test]
    fn a_sustained_overlap_hides_it() {
        let mut state = Obstruction::default();
        state.observe(rect(100, 300), &[rect(150, 200)]);
        assert!(state.observe(rect(100, 300), &[rect(150, 200)]));
        assert!(state.is_obstructed());
    }

    #[test]
    fn the_readout_returns_the_instant_the_overlap_clears() {
        // No hysteresis on the way back: the failure has already ended, and
        // nobody should wait to see their own monitor again.
        let mut state = Obstruction::default();
        state.observe(rect(100, 300), &[rect(150, 200)]);
        state.observe(rect(100, 300), &[rect(150, 200)]);
        assert!(state.is_obstructed());
        assert!(!state.observe(rect(100, 300), &[rect(400, 450)]));
        assert!(!state.is_obstructed());
    }

    #[test]
    fn a_flickering_overlap_never_accumulates_into_hiding() {
        // Alternating samples must not creep up to the threshold, or a tray
        // that re-lays out repeatedly would hide the readout for no reason.
        let mut state = Obstruction::default();
        for _ in 0..10 {
            assert!(!state.observe(rect(100, 300), &[rect(150, 200)]));
            assert!(!state.observe(rect(100, 300), &[rect(400, 450)]));
        }
    }

    #[test]
    fn an_empty_region_is_never_obstructed() {
        // Before any placeholder appears there is nothing to be covered.
        let mut state = Obstruction::default();
        assert!(!state.observe(rect(100, 100), &[rect(50, 200)]));
    }

    #[test]
    fn touching_edges_do_not_count_as_overlap() {
        // An icon abutting the region is beside it, not on it.
        let mut state = Obstruction::default();
        state.observe(rect(100, 300), &[rect(300, 340)]);
        assert!(!state.observe(rect(100, 300), &[rect(300, 340)]));
    }

    #[test]
    fn a_union_ignores_empty_rectangles() {
        // Placeholders that have not appeared yet must not drag the region to
        // the origin, which would make it overlap everything.
        let region = Rect::default().union(rect(100, 142));
        assert_eq!(region, rect(100, 142));
        assert_eq!(rect(100, 142).union(rect(142, 184)), rect(100, 184));
    }
}

#[cfg(test)]
mod span_tests {
    use super::*;

    fn region(width: i32) -> Rect {
        Rect { left: 1000, top: 0, right: 1000 + width, bottom: 48 }
    }

    #[test]
    fn contiguous_placeholders_are_not_obstructed() {
        let mut state = Obstruction::default();
        // Four slots of 42, exactly as measured on real hardware.
        state.observe_span(region(168), 4, 42);
        assert!(!state.observe_span(region(168), 4, 42));
    }

    #[test]
    fn an_icon_wedged_between_them_widens_the_span_and_is_caught() {
        let mut state = Obstruction::default();
        // Still four placeholders, but now spanning five slots.
        state.observe_span(region(210), 4, 42);
        assert!(state.observe_span(region(210), 4, 42));
    }

    #[test]
    fn small_packing_differences_are_not_intruders() {
        // The tray does not always pack to the pixel, and a few pixels of
        // rounding must not hide the readout.
        let mut state = Obstruction::default();
        state.observe_span(region(180), 4, 42);
        assert!(!state.observe_span(region(180), 4, 42));
    }

    #[test]
    fn nothing_reserved_is_never_obstructed() {
        let mut state = Obstruction::default();
        assert!(!state.observe_span(Rect::default(), 0, 42));
        assert!(!state.observe_span(region(168), 0, 42));
    }

    #[test]
    fn an_intruder_that_leaves_restores_the_readout_at_once() {
        let mut state = Obstruction::default();
        state.observe_span(region(210), 4, 42);
        state.observe_span(region(210), 4, 42);
        assert!(state.is_obstructed());
        assert!(!state.observe_span(region(168), 4, 42));
    }
}

#[cfg(test)]
mod pitch_tests {
    use super::*;

    fn slot(left: i32, width: i32) -> Rect {
        Rect { left, top: 0, right: left + width, bottom: 48 }
    }

    #[test]
    fn the_pitch_is_one_icons_width() {
        let rects = [slot(100, 42), slot(142, 42), slot(184, 42)];
        assert_eq!(slot_pitch(&rects), Some(42));
    }

    #[test]
    fn a_gap_between_icons_does_not_change_the_pitch() {
        // Somebody else's icon sits between the second and third. Dividing the
        // span by the count would report 56; the pitch is still 42.
        let rects = [slot(100, 42), slot(142, 42), slot(226, 42)];
        assert_eq!(slot_pitch(&rects), Some(42));
    }

    #[test]
    fn one_rectangle_caught_mid_animation_does_not_drag_the_answer() {
        let rects = [slot(100, 42), slot(142, 9), slot(184, 42), slot(226, 42)];
        assert_eq!(slot_pitch(&rects), Some(42));
    }

    #[test]
    fn nothing_located_means_no_measurement_rather_than_zero() {
        assert_eq!(slot_pitch(&[]), None);
        assert_eq!(slot_pitch(&[Rect::default()]), None);
    }

    #[test]
    fn a_lone_icon_is_not_a_measurement() {
        // Its width is not a pitch. The first placeholder of a run stands
        // alone for a tick and the shell reports it 36 wide; latched, that
        // over-reserves by a slot for the rest of the run.
        assert_eq!(slot_pitch(&[slot(100, 36)]), None);
    }
}
