// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Holding a block of space in the notification area.
//
// Ties the pieces together: how many placeholders a width needs, adding and
// removing them, retrying the promotions the shell was not ready for, finding
// where they landed, and noticing when somebody drags an icon in among them.

use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::HICON;

use crate::spacer::{
    self, icon_guid, slot_width_floor, slots_for_width, Guid, Rect, MAX_GUID_BLOCKS, MAX_ICONS,
    RETRY_COOLDOWN_MS, ROTATE_GRACE_MS,
};
use crate::taskbar_buttons::TaskbarButtons;
use crate::tray;

/// Consecutive narrower measurements before the region is believed to have
/// shrunk.
///
/// Two, against a one-second tick, so a re-layout has to persist for about two
/// seconds to count. The tray settles in well under one: clicking the overflow
/// chevron re-lays it out for a frame or two, which is the case this exists to
/// ride out. A genuine shrink - somebody dragging an icon in, a placeholder the
/// shell has stopped drawing - lasts until it is undone, so waiting costs
/// nothing but two seconds of the readout keeping the width it already had.
const REGION_CONFIRM: u32 = 2;

/// The reserved region, held steady through the tray's own re-layouts.
///
/// Clicking anything in the notification area - the overflow chevron, or an
/// icon that shows a flyout - makes the shell re-lay the tray out, and for a
/// frame in the middle of that some of our placeholders report no rectangle at
/// all. Believed literally, that is a region a slot or two narrower, or none:
/// the readout resizes or vanishes and is back a tick later. That one-frame
/// change is the flash somebody sees every time they click the tray, and
/// nothing has actually moved.
///
/// So any change to an established region has to be read twice before it is
/// believed, and only a change of our own - a placeholder added or given back,
/// which moves the held count - is taken at once. Startup depends on that
/// exception, since the region fills in one icon at a time and a delay there
/// is a delay before the readout can be shown at all. `observe` says why the
/// rule is about the whole rectangle rather than its width.
#[derive(Default, Debug)]
struct Steady {
    region: Rect,
    /// How many placeholders were held when `region` was measured.
    ///
    /// A region that shrank because we gave slots back is our own doing and is
    /// believed at once; one that shrank while we hold the same slots is the
    /// tray mid-re-layout. This is what tells those two apart.
    held: usize,
    /// A measurement that disagrees with `region`, waiting to be seen again.
    ///
    /// The same rectangle twice, not a count of disagreements: a transient
    /// wobbles, so it reads differently each time and never confirms, while a
    /// real change reads the same until it is undone.
    candidate: Rect,
    /// How many times running `candidate` has come back identical.
    settled: u32,
}

impl Steady {
    /// Folds in one measurement and returns the region to use.
    ///
    /// `seen` is how many of the `held` placeholders actually reported a
    /// rectangle. Fewer than all of them is the tray mid-re-layout, and such a
    /// measurement is not evidence of anything: the placeholders that did not
    /// report are still on screen and still holding their slots.
    fn observe(&mut self, measured: Rect, held: usize, seen: usize) -> Rect {
        // Nothing yet, so anything beats nothing: at startup the placeholders
        // appear one per tick and the readout cannot be shown until there is
        // somewhere to put it. Waiting for a complete set here would mean a
        // machine where one placeholder never appears never shows a readout.
        if self.region.is_empty() {
            self.region = measured;
            self.held = held;
            self.settled = 0;
            return self.region;
        }

        // An incomplete measurement says nothing at all: the placeholders that
        // did not report are still on screen and still holding their slots.
        if seen != held {
            return self.region;
        }

        // Our own doing, nothing to doubt: slots were given back or taken and
        // the region is supposed to have changed. This is also what carries
        // startup, where a placeholder is added every tick, so the region
        // follows the block as it fills without any of the caution below.
        if held != self.held {
            self.region = measured;
            self.held = held;
            self.settled = 0;
            return self.region;
        }

        if measured == self.region {
            self.settled = 0;
            return self.region;
        }

        // Everything else has to say it twice, and this is the whole of the
        // tray-click flash.
        //
        // Every earlier version of this guarded the region's *width*: a
        // narrower one had to repeat, a wider one was taken at once. But the
        // measurement that actually moves is neither. Caught in the trace:
        //
        //   region 1703,1552-1997,1600 (294px)
        //   region 1739,1552-2033,1600 (294px)
        //
        // Same width, same seven placeholders out of seven held, thirty-six
        // pixels to the right. A pure translation, which every width test in
        // the world waves straight through - so the readout jumped sideways
        // and back, which is what the flash always was. It never resized and
        // it never hid. It moved.
        //
        // So the rule is about the rectangle, not its width, and it asks for
        // the same rectangle twice rather than two disagreements in a row. A
        // tray settling mid-re-layout reads differently each time and never
        // confirms; a block that has genuinely moved reads the same until
        // something moves it again.
        if measured == self.candidate {
            self.settled += 1;
        } else {
            self.candidate = measured;
            self.settled = 1;
        }

        if self.settled >= REGION_CONFIRM {
            self.region = measured;
            self.held = held;
            self.settled = 0;
        }
        self.region
    }
}

#[cfg(test)]
mod steady_tests {
    use super::*;

    fn span(left: i32, right: i32) -> Rect {
        Rect { left, top: 0, right, bottom: 30 }
    }

    #[test]
    fn a_partial_measurement_moves_nothing() {
        // The tray mid-re-layout: the far placeholders still report and the
        // ones between them have not come back yet, so the span is full width
        // and the count is short. Taken as a measurement it is a region that
        // changed; it is not one, and believing it resized the strip.
        let mut steady = Steady::default();
        steady.observe(span(100, 400), 8, 8);
        assert_eq!(steady.observe(span(100, 400), 8, 3), span(100, 400));
        assert_eq!(steady.observe(span(100, 460), 8, 3), span(100, 400));
        assert_eq!(steady.observe(Rect::default(), 8, 0), span(100, 400));
    }

    #[test]
    fn widening_is_not_believed_from_a_partial_measurement_either() {
        // The first version of this debounced narrowing and took widening at
        // once, so a re-layout that briefly spread the placeholders resized the
        // strip immediately and then held the wrong width for two seconds. A
        // wider span from a short count is the same non-evidence as a narrower
        // one.
        let mut steady = Steady::default();
        steady.observe(span(100, 400), 8, 8);
        assert_eq!(steady.observe(span(100, 520), 8, 5), span(100, 400));
        // Complete and wider still has to say it twice, like every other
        // change to a region that is already established.
        assert_eq!(steady.observe(span(100, 520), 8, 8), span(100, 400));
        assert_eq!(steady.observe(span(100, 520), 8, 8), span(100, 520));
    }

    #[test]
    fn a_region_that_slides_sideways_for_one_tick_is_not_followed() {
        // The flash, exactly as the trace caught it: same width, same seven
        // placeholders out of seven held, thirty-six pixels to the right and
        // back again. Every version of this that guarded the region's width
        // waved it straight through, and the readout jumped sideways.
        let mut steady = Steady::default();
        assert_eq!(steady.observe(span(1703, 1997), 7, 7), span(1703, 1997));
        assert_eq!(steady.observe(span(1739, 2033), 7, 7), span(1703, 1997));
        assert_eq!(steady.observe(span(1703, 1997), 7, 7), span(1703, 1997));
    }

    #[test]
    fn a_region_that_really_moved_is_followed_once_it_holds_still() {
        // The tray was rearranged and the block genuinely lives somewhere else
        // now. Refusing forever would leave the readout beside its own
        // reserved space rather than in it.
        let mut steady = Steady::default();
        steady.observe(span(1703, 1997), 7, 7);
        steady.observe(span(1739, 2033), 7, 7);
        assert_eq!(steady.observe(span(1739, 2033), 7, 7), span(1739, 2033));
    }

    #[test]
    fn a_wobble_never_confirms_however_long_it_goes_on() {
        // Two disagreements in a row are not the same as the same
        // disagreement twice. A tray settling reads a different rectangle
        // every tick, and a streak counter would eventually believe one of
        // them; asking for the identical rectangle cannot be fooled that way.
        let mut steady = Steady::default();
        steady.observe(span(1703, 1997), 7, 7);
        for offset in [12, 36, 4, 28, 36, 8, 20] {
            assert_eq!(
                steady.observe(span(1703 + offset, 1997 + offset), 7, 7),
                span(1703, 1997)
            );
        }
    }

    #[test]
    fn the_first_measurement_is_taken_however_partial_it_is() {
        // Startup adds placeholders one per tick, and the readout cannot be
        // shown until there is somewhere to put it. Demanding a complete set
        // here would mean a machine where one placeholder never appears never
        // shows a readout at all.
        let mut steady = Steady::default();
        assert_eq!(steady.observe(span(100, 180), 6, 2), span(100, 180));
    }

    #[test]
    fn a_region_that_flickers_narrow_for_one_tick_is_not_believed() {
        // Exactly what clicking the overflow chevron does: the tray re-lays
        // out, a placeholder reports nothing for a frame, and the readout used
        // to resize and snap back - the flash.
        let mut steady = Steady::default();
        assert_eq!(steady.observe(span(100, 400), 8, 8), span(100, 400));
        assert_eq!(steady.observe(span(100, 300), 8, 8), span(100, 400));
        assert_eq!(steady.observe(span(100, 400), 8, 8), span(100, 400));
    }

    #[test]
    fn a_region_that_vanishes_for_one_tick_is_not_believed_either() {
        // The worse version of the same thing: every placeholder comes back
        // empty at once, which read as "no reserved space" and hid the readout.
        let mut steady = Steady::default();
        steady.observe(span(100, 400), 8, 8);
        assert_eq!(steady.observe(Rect::default(), 8, 8), span(100, 400));
    }

    #[test]
    fn a_region_that_stays_narrow_is_believed() {
        // An icon really was dragged in among the placeholders, or the shell
        // has stopped drawing one. Waiting forever would leave the readout
        // claiming space it does not have.
        let mut steady = Steady::default();
        steady.observe(span(100, 400), 8, 8);
        steady.observe(span(100, 300), 8, 8);
        assert_eq!(steady.observe(span(100, 300), 8, 8), span(100, 300));
    }

    #[test]
    fn growing_because_we_added_slots_is_taken_immediately() {
        // Startup adds one placeholder per tick, so the region grows a slot at
        // a time. Confirming growth would delay the readout appearing at all.
        let mut steady = Steady::default();
        assert_eq!(steady.observe(span(100, 140), 1, 1), span(100, 140));
        assert_eq!(steady.observe(span(100, 180), 2, 2), span(100, 180));
        assert_eq!(steady.observe(span(100, 220), 3, 3), span(100, 220));
    }

    #[test]
    fn shrinking_because_we_gave_slots_back_is_taken_immediately() {
        // The user switched a module off, the strip needs less width, and we
        // released placeholders. Making the readout wait two seconds to stop
        // claiming space it no longer wants would look like a stuck setting.
        let mut steady = Steady::default();
        steady.observe(span(100, 400), 8, 8);
        assert_eq!(steady.observe(span(100, 300), 6, 6), span(100, 300));
    }

    #[test]
    fn a_second_flicker_after_a_recovery_still_gets_its_own_grace() {
        // The streak has to reset on every good sample, or a tray that
        // re-lays out now and then would accumulate enough narrow samples to
        // eventually believe one of them.
        let mut steady = Steady::default();
        steady.observe(span(100, 400), 8, 8);
        for _ in 0..5 {
            assert_eq!(steady.observe(span(100, 300), 8, 8), span(100, 400));
            assert_eq!(steady.observe(span(100, 400), 8, 8), span(100, 400));
        }
    }
}

/// How long the readout must want fewer tray slots before it gives one back.
///
/// Only shrinking waits; growing is immediate. Every added or removed
/// placeholder makes the shell repack the notification area and slide every
/// icon in it sideways, so a width that wanders across a slot boundary drags
/// the user's tray icons back and forth with it. Four seconds is longer than
/// any flicker and shorter than anybody's patience.
const SETTLE: Duration = Duration::from_secs(4);

/// A block of reserved tray space.
pub struct Reservation {
    owner: HWND,
    icon: HICON,
    /// Which batch of identities is in use. See `spacer::icon_guid`.
    block: u32,
    /// Slots registered, always the first `held` indices.
    held: usize,
    /// Slots added but not yet marked always-visible, because the shell had
    /// not written their registry entry when we looked.
    pending: Vec<usize>,
    /// Measured pitch, once the placeholders are on screen. The DPI-derived
    /// seed runs high, so a measurement always wins.
    slot_width: i32,
    seed_width: i32,
    /// When the readout first asked for fewer slots than it holds, or None
    /// while it is not asking to shrink. See `settled`.
    shrink_since: Option<Instant>,
    steady: Steady,
    /// Placeholders that reported a rectangle in the taskbar on the last pass,
    /// against `held`. Only interesting to the trace, and the trace is how the
    /// tray-click flash was finally pinned down, so it is worth carrying.
    seen: usize,
    taskbar_buttons: TaskbarButtons,
    /// When the first placeholder of this block was added, for the grace
    /// period before the block is written off as dead.
    first_added: Option<Instant>,
    /// The most placeholders ever located on screen at once for this block.
    ///
    /// Not a boolean any more. "Did anything ever appear" treats a block where
    /// two identities out of nine still work as healthy, and the readout then
    /// sits forever in a sliver of reserved space with no way out. Comparing
    /// against how many are held catches the half-dead case, which is the one
    /// that actually happens - deleting a NotifyIconSettings entry kills that
    /// one identity and leaves its neighbors alone.
    best_found: usize,
    retry_after: Option<Instant>,
    backoff: u32,
}

impl Reservation {
    /// Creates the owner window and the transparent icon.
    ///
    /// Also clears entries left by a previous run, which is the one moment
    /// Leftovers are *not* deleted, which is the opposite of what it looks
    /// like they should be. See the comment in the body.
    pub fn new(dpi: u32) -> Option<Reservation> {
        // Deliberately not purging leftovers here.
        //
        // Deleting a NotifyIconSettings entry makes the shell go on
        // recognizing that GUID while never writing its entry again, so the
        // identity is dead forever. Purging at startup therefore killed the
        // very identities this run was about to use: the placeholders could no
        // longer be shown, the tray never widened, and the readout collapsed
        // onto the tray chevron. Seen exactly once, which was enough.
        //
        // Nothing is left behind by not purging. The GUIDs are derived from a
        // fixed base, so a leftover entry from a crashed run is the entry this
        // run wants, and re-adding the icon reclaims it.

        let owner = tray::create_owner_window();
        if owner.is_null() {
            return None;
        }
        let icon = tray::blank_icon();
        if icon.is_null() {
            return None;
        }

        let seed = spacer::dpi_seed_width(dpi);
        Some(Reservation {
            owner,
            icon,
            // Where this machine last got to, not zero. Starting at zero every
            // launch means rediscovering the same dead identities every launch
            // and showing nothing for the seconds that takes.
            block: crate::reserve_block::stored_block(),
            held: 0,
            pending: Vec::new(),
            slot_width: seed,
            seed_width: seed,
            shrink_since: None,
            steady: Steady::default(),
            seen: 0,
            taskbar_buttons: TaskbarButtons::new(),
            first_added: None,
            best_found: 0,
            retry_after: None,
            backoff: 0,
        })
    }

    fn guid(&self, index: usize) -> Guid {
        Guid::from_u128(icon_guid(self.block, index))
    }

    /// Tells the reservation which monitor's DPI applies now.
    ///
    /// The measured pitch is discarded on a change rather than carried across.
    /// Keeping it lets the previous screen's pitch decide how many
    /// placeholders to add: too few and the readout will not fit, too many and
    /// it squats on a strip of the tray for nothing.
    pub fn set_dpi(&mut self, dpi: u32) {
        let seed = spacer::dpi_seed_width(dpi);
        if seed == self.seed_width {
            return;
        }
        self.seed_width = seed;
        self.slot_width = seed;
    }

    pub fn region(&self) -> Rect {
        self.steady.region
    }

    pub fn is_obstructed(&self) -> bool {
        self.taskbar_buttons.is_obstructed()
    }

    pub fn held(&self) -> usize {
        self.held
    }

    /// How many placeholders reported a usable rectangle on the last pass.
    pub fn seen(&self) -> usize {
        self.seen
    }

    pub fn slot_width(&self) -> i32 {
        self.slot_width
    }

    /// Asks for a width, in physical pixels. Zero releases everything.
    ///
    /// Call it every tick: this is where placeholders are added, promotions
    /// retried, the region re-measured, and a dead block rotated away from.
    pub fn set_width(&mut self, width: i32) {
        let wanted = self.settled(slots_for_width(width, self.slot_width));
        if wanted == 0 {
            self.release();
            return;
        }

        self.retry_promotions();
        self.match_slot_count(wanted);
        self.measure();
        self.rotate_if_dead();
        // The sweep only needs to know how many of ours to expect. Where they
        // are is measured here, by GUID, which is exact and costs nothing.
        self.taskbar_buttons.set_placeholders(self.held);
    }

    /// The count to actually hold, given what this tick asked for.
    ///
    /// Growth is immediate: too few slots means the readout is clipped, and
    /// there is nothing to be gained by being slow about it. Shrinking waits
    /// until the smaller figure has held for `SETTLE`.
    ///
    /// The asymmetry is the whole point. Adding or removing a placeholder
    /// makes the shell repack the notification area, which slides every icon
    /// in it sideways - so a readout whose width wanders across a slot
    /// boundary drags the user's tray icons back and forth with it. The
    /// readout's own columns are already held at a fixed width, but the total
    /// still crosses a boundary from time to time: a sensor that starts
    /// reporting, a rate that reaches a wider unit, the weather arriving.
    /// Letting the count fall only after it has stayed down for a few seconds
    /// turns a shuffle into a single move.
    fn settled(&mut self, wanted: usize) -> usize {
        let now = Instant::now();
        if wanted >= self.held {
            // Wider, or no change: take it now and start the clock again.
            self.shrink_since = None;
            return wanted;
        }
        match self.shrink_since {
            // Narrower, and it has been narrower long enough to believe.
            Some(since) if now.duration_since(since) >= SETTLE => {
                self.shrink_since = None;
                wanted
            }
            Some(_) => self.held,
            None => {
                self.shrink_since = Some(now);
                self.held
            }
        }
    }

    /// Adds or removes placeholders until the count matches.
    fn match_slot_count(&mut self, wanted: usize) {
        let wanted = wanted.min(MAX_ICONS);

        while self.held > wanted {
            let index = self.held - 1;
            tray::remove_icon(self.owner, self.guid(index));
            self.pending.retain(|slot| *slot != index);
            self.held -= 1;
        }

        if self.held >= wanted {
            return;
        }
        // Adding can fail when the shell is busy or the tray is not ready.
        // Retrying every tick makes the notification area flicker, so failures
        // back off - but never give up, or the region stays too narrow forever
        // and the readout never moves onto it.
        if self.retry_after.is_some_and(|when| Instant::now() < when) {
            return;
        }

        // Every missing placeholder in one pass, not one per tick. Adding them
        // one at a time makes the strip take one second per slot to reach its
        // width, which is nine seconds of a readout clipped against the tray
        // on every start and after every Explorer restart - the times somebody
        // is most likely to be looking at it.
        while self.held < wanted {
            let index = self.held;
            if !tray::add_icon(self.owner, self.icon, self.guid(index)) {
                // Stop at the first refusal rather than hammering the rest.
                // The shell refuses in a run, not one icon at a time.
                self.backoff = (self.backoff + 1).min(spacer::MAX_BACKOFF_STEPS);
                let wait = RETRY_COOLDOWN_MS * u64::from(self.backoff);
                self.retry_after = Some(Instant::now() + Duration::from_millis(wait));
                return;
            }
            self.held += 1;
            self.backoff = 0;
            self.retry_after = None;
            self.first_added.get_or_insert_with(Instant::now);
            // The entry usually does not exist yet, so this normally fails and
            // the slot joins the retry list rather than being lost.
            if !tray::promote(self.guid(index)) {
                self.pending.push(index);
            }
        }
    }
}

impl Reservation {
    /// Second and later attempts at the promotions the shell was not ready for.
    ///
    /// Matched by GUID rather than by watching for a new key, so it works
    /// however late the entry appears. An icon that is never promoted stays in
    /// the overflow flyout and reserves nothing at all.
    fn retry_promotions(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let mut promoted = Vec::new();
        for index in self.pending.clone() {
            let guid = self.guid(index);
            if tray::promote(guid) {
                // Re-add so the shell notices the change.
                tray::remove_icon(self.owner, guid);
                tray::add_icon(self.owner, self.icon, guid);
                promoted.push(index);
            }
        }
        self.pending.retain(|slot| !promoted.contains(slot));
    }

    /// Finds where the placeholders actually are.
    ///
    /// Uses whatever is genuinely on screen rather than demanding all of them.
    /// Requiring the full set makes the region permanently invalid whenever one
    /// icon lags or stays hidden, and the readout then never moves onto the
    /// space that does exist. A partial region simply fits less.
    ///
    /// What is measured is handed to `Steady` rather than used directly, and
    /// that is where the tray-click flash is held off: clicking anything in the
    /// tray - the overflow chevron, or an icon with a flyout of its own - makes
    /// the shell re-lay the tray out, and for a frame in the middle of that
    /// some of our placeholders report no rectangle at all. Taken at face value
    /// that is a region a slot or two narrower, or none, so the readout resizes
    /// or disappears and is back a tick later. Nothing actually moved.
    fn measure(&mut self) {
        // Only rectangles that are actually in the taskbar, and this is the
        // whole of the tray-click flash.
        //
        // A placeholder that has been pushed into the overflow still answers
        // Shell_NotifyIconGetRect - but while the "Show hidden icons" flyout is
        // open it answers with where it is *in the flyout*, which is a popup
        // window sitting above the taskbar. Every icon reports, so the
        // measurement is complete by any count, and the union of them spans
        // from the tray up into the popup. The region explodes, the span
        // dwarfs the slots it holds, and the span tell that used to decide
        // obstruction read that as an icon dragged in among the placeholders -
        // so the readout hid itself, on open and again on close.
        //
        // Obstruction goes to the UI Automation sweep now, as TrafficMonitor's
        // does - see the block at the end of this function - so that particular
        // misreading is gone. The rectangles are still dropped: one that is not
        // in the taskbar at all says nothing about where the region is, and
        // what is left is an incomplete measurement, which changes nothing.
        let bar = barometer_core::taskbar::taskbar().map(|bar| Rect {
            left: bar.rect.left,
            top: bar.rect.top,
            right: bar.rect.right,
            bottom: bar.rect.bottom,
        });
        let mut rects = Vec::with_capacity(self.held);
        for index in 0..self.held {
            let Some(rect) = tray::icon_rect(self.owner, self.guid(index)) else { continue };
            // No taskbar means the shell is mid-restart, and a measurement then
            // is not worth having; the next tick will find it.
            match bar {
                Some(bar) if rect.within(bar) => rects.push(rect),
                Some(_) => {}
                None => return,
            }
        }
        self.seen = rects.len();
        let measured = rects.iter().fold(Rect::default(), |all, one| all.union(*one));
        // Followed through an animation, not held through one. Holding the
        // old region while the tray spreads out was tried, and it parks the
        // readout where the chevron is about to be. TrafficMonitor rides the
        // union it measures and re-places on every change; the caller now
        // re-measures at the sweep's cadence while the tray moves, so the
        // readout stays inside the placeholders wherever they are.
        self.steady.observe(measured, self.held, rects.len());

        if !rects.is_empty() && !measured.is_empty() {
            self.best_found = self.best_found.max(rects.len());
            // Measured from one icon's own rectangle rather than from the span
            // over all of them, so a placeholder the shell has not drawn yet
            // and a foreign icon dragged in between two of ours both leave the
            // figure alone. A measurement always beats the DPI estimate, which
            // measurably runs high.
            //
            // Kept as the smallest credible measurement rather than the latest
            // one, and that is not a refinement - it is what stops a feedback
            // loop. The tray re-lays out as icons are added, so the same
            // placeholders measure 42 pixels at eight of them and 46 at nine.
            // Taking the latest gives: 42 asks for nine slots, nine slots
            // measure 46, 46 asks for eight, eight measure 42 - forever, once
            // a second, with the readout jumping a slot each time. Seen live.
            //
            // Downward is also the safe direction. Too small a pitch asks for
            // a slot more than needed and the extra is absorbed as slack
            // inside the strip; too large clips the last column against the
            // tray. The floor keeps a rectangle caught mid-animation from
            // latching something absurd, and a DPI change or an Explorer
            // restart clears it, since neither leaves the old figure meaning
            // anything.
            if let Some(measured) = spacer::slot_pitch(&rects) {
                if measured >= slot_width_floor(self.seed_width) && measured < self.slot_width {
                    self.slot_width = measured;
                }
            }
        }

        // Obstruction is not inferred from this span, and the region is not
        // taken from the sweep that decides it. They are two questions with
        // two right answers.
        //
        // Where our placeholders are is answered exactly by
        // Shell_NotifyIconGetRect, keyed on GUIDs this program owns. Whether
        // somebody else's icon is sitting among them cannot be answered that
        // way at all - it needs to see icons we cannot name - so it goes to
        // the UI Automation sweep, which is what TrafficMonitor does.
        //
        // Sourcing the region from the sweep as well was tried and is what
        // left a correctly reserved but empty gap on the taskbar: a sweep that
        // is momentarily stale or blacked out then takes the region away with
        // it, and the readout has nowhere to be drawn. The measurement below
        // cannot fail that way.
        if rects.len() != self.held && crate::trace::on() {
            crate::trace::line(&format!(
                "  partial measurement: {} of {} placeholders in the taskbar, region held",
                rects.len(),
                self.held
            ));
        }
    }


    /// Rotates to a fresh batch of identities when this one is dead.
    ///
    /// Icons added successfully yet never all appearing means some of this
    /// batch's registry entries were deleted at some point: the shell goes on
    /// recognizing those identities but will never write their entries again,
    /// so they can never be shown, and it does not recover on its own. A block
    /// only has to be partly dead to be useless, because the space it reserves
    /// is short by exactly the identities that are gone.
    fn rotate_if_dead(&mut self) {
        // Two ways a block dies, and both end here.
        //
        // The first is that the adds succeed and the icons never all appear,
        // which means some of the registry entries were deleted.
        //
        // The second is that the adds themselves are refused, and that one is
        // easy to miss because nothing is ever held and the old test - "we
        // hold icons but cannot see them" - never fires. It happens because a
        // GUID registered through NIF_GUID is bound by the shell to the path
        // of the executable that registered it. Move the program, or install
        // it after running it from a build directory, and every identity it
        // ever used is refused from the new location, permanently. Seen
        // exactly that way: the repository moved and the readout collapsed
        // onto the tray with no placeholders at all.
        let refused = self.held == 0 && self.backoff >= spacer::MAX_BACKOFF_STEPS;
        let incomplete = self.held > 0 && self.best_found < self.held;
        if !refused && !incomplete {
            return;
        }
        // A refusal is immediate and repeatable, so there is nothing to wait
        // for; only the "added but not shown" case needs the shell time.
        if incomplete {
            let Some(first) = self.first_added else { return };
            if first.elapsed() < Duration::from_millis(ROTATE_GRACE_MS) {
                return;
            }
        }
        if self.block + 1 >= MAX_GUID_BLOCKS {
            // Out of identities entirely, so start over rather than stopping.
            // The oldest blocks are the likeliest to have come back: what
            // usually kills one is a path change, and a reinstall undoes that.
            for index in 0..self.held {
                tray::remove_icon(self.owner, self.guid(index));
            }
            self.block = 0;
            crate::reserve_block::store_block(0);
            self.held = 0;
            self.pending.clear();
            self.first_added = None;
            self.retry_after = None;
            self.backoff = 0;
            self.best_found = 0;
            return;
        }

        for index in 0..self.held {
            tray::remove_icon(self.owner, self.guid(index));
        }
        self.block += 1;
        // Written down straight away, so the next launch starts here rather
        // than walking the dead blocks again.
        crate::reserve_block::store_block(self.block);
        self.held = 0;
        self.taskbar_buttons.set_placeholders(0);
        self.pending.clear();
        self.first_added = None;
        self.retry_after = None;
        self.backoff = 0;
        // Carrying the old block's tally forward would make the new one look
        // healthy before a single icon of it had been seen.
        self.best_found = 0;
    }

    /// Gives every placeholder back.
    pub fn release(&mut self) {
        for index in 0..self.held {
            tray::remove_icon(self.owner, self.guid(index));
        }
        self.held = 0;
        self.pending.clear();
        self.steady = Steady::default();
        self.first_added = None;
    }

    /// Called when Explorer has restarted.
    ///
    /// Its copy of the placeholders is gone, so the local record has to go too.
    /// This is the trap: without it, the wanted count and the held count agree,
    /// nothing is ever re-added, and the reservation never comes back for the
    /// life of the process.
    pub fn on_shell_restarted(&mut self) {
        // The latched pitch goes with it. The new shell lays the tray out
        // afresh and the old figure is only a memory of the one that died.
        //
        // The sweep is told to forget too. Its answers describe buttons in a
        // taskbar that no longer exists, and an obstruction verdict carried
        // across the restart would hide the readout against nothing.
        self.taskbar_buttons.invalidate();
        self.slot_width = self.seed_width;
        self.held = 0;
        self.pending.clear();
        self.steady = Steady::default();
        self.first_added = None;
        self.retry_after = None;
        self.backoff = 0;
        self.best_found = 0;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        // Hand the icons back, but never delete their registry entries. That is
        // what makes a batch of identities permanently unusable next time.
        self.release();
    }
}
