// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The weather condition as a character from Windows' own icon font.
//
// The marks in badge.rs are a faithful port of the Mac app's, and on a Retina
// menu bar they are the right answer. On a Windows 11 taskbar the same
// geometry lands on about fourteen device pixels, where two and three pixel
// features stop reading as weather, and next to the shell's own tray icons
// they look like something drawn by a different program - which they were.
// Segoe Fluent Icons is what Windows draws its own iconography in, so a
// condition taken from it sits on the taskbar the way the volume and network
// icons beside it do.
//
// Nothing is redistributed. The font ships with Windows 11 and is referenced
// by family name exactly as Segoe UI is for text, so there is no asset in the
// repository and no license to honor. Segoe MDL2 Assets is the fallback: it
// is the older font of the same family, present back to Windows 10, and it
// carries all nine of the codepoints used here under the same names, drawn in
// a squarer style. Naming it costs nothing and covers a machine that somehow
// lacks the newer font.
//
// The codepoints were found by rendering every glyph in the font's Private Use
// Area to a labeled sheet and reading it, not recalled. Two things that sweep
// turned up are worth knowing before editing this table. The first is that the
// PUA also holds wide CJK punctuation glyphs whose ink spills well outside
// their cell - U+E9B7 looks exactly like a snowflake in a grid and is in fact
// two and a quarter ems of punctuation - so a candidate is only an icon if its
// advance width is one em. The second is the finding that shapes everything
// below.
//
// **Segoe Fluent Icons has no weather set.** It has a sun, a moon, a cloud, a
// cloud shedding drops, a raindrop, a snowflake, a bolt, three drifting lines
// and a question mark, and that is the whole inventory: there is no
// sun-behind-cloud, no cloud with rain hanging from it beyond the one, and
// nothing that says storm rather than lightning. Nine marks have to carry
// fourteen conditions, so five conditions share a glyph with a neighbor.
//
// Each of those five is collapsed onto the condition next to it in severity,
// which is the pairing that loses the least: partly cloudy falls back on the
// clear sun or moon, heavy rain on rain, sleet on snow, and a thunderstorm on
// thunder. The pairs are listed in SHARED_GLYPHS and asserted there, so a
// collapse cannot be introduced or removed without saying so.
//
// That would have cost the project's rule that conditions differ in shape and
// never only in color five pairs. What saves it is `composed` below, which
// draws the nine in combination - a sun *behind* a cloud, a bolt over one -
// and gets all fourteen back as distinct shapes. It is what every painter
// calls; the single-glyph mapping is kept beside it for the record and for
// its tests.

use super::badge::Condition;

/// The font Windows 11 draws its own icons in.
pub const FAMILY: &str = "Segoe Fluent Icons";

/// The same family one generation back, for a machine without the newer font.
///
/// Every codepoint used here is present in it and draws the same mark, in the
/// squarer style that font was designed with.
pub const FALLBACK_FAMILY: &str = "Segoe MDL2 Assets";

/// The pairs of conditions the font forces to share a mark.
///
/// This is a record of a compromise rather than a table anything reads at run
/// time. It is asserted against the mapping, so removing a collapse means
/// removing it from here too, and adding one that is not listed fails the
/// tests rather than quietly costing the user a distinction.
pub const SHARED_GLYPHS: [(Condition, Condition); 5] = [
    (Condition::ClearDay, Condition::PartlyCloudy),
    (Condition::ClearNight, Condition::PartlyCloudyNight),
    (Condition::Rain, Condition::HeavyRain),
    (Condition::Sleet, Condition::Snow),
    (Condition::Thunder, Condition::Thunderstorm),
];

/// The one character a condition would be drawn with, where one had to do.
///
/// Nothing paints from this - `composed` is what the painters call. It is kept
/// for the record of which nine marks the font actually has, and for the tests
/// below that hold the mapping to them. The match is exhaustive on purpose: a
/// condition added to the enum has to be given a mark here before this
/// compiles.
pub fn glyph(condition: Condition) -> char {
    match condition {
        // A sun with detached rays. The other suns in the font differ from it
        // only in the fill of the center disc, which is invisible at fifteen
        // pixels, so this is the one sun the set can use.
        Condition::ClearDay => '\u{E706}',

        // A crescent. The font carries three more crescents that differ in
        // weight and fill alone, so they cannot be spent on a second night
        // condition without pretending a shape distinction that is not there.
        Condition::ClearNight => '\u{E708}',

        // Shares the clear sun. There is no sun behind a cloud in the font,
        // and the alternative - giving partly cloudy the plain cloud - would
        // make it identical to overcast and, since there is no moon behind a
        // cloud either, would collapse the day and night variants into each
        // other as well. Losing partly against clear keeps both the day/night
        // split and the far more useful sunny against overcast one.
        Condition::PartlyCloudy => '\u{E706}',

        // The night half of the same compromise.
        Condition::PartlyCloudyNight => '\u{E708}',

        Condition::Cloudy => '\u{E753}',

        // Three drifting lines. Read as mist rather than as the stack of
        // straight rules the font's other line glyphs give, which are list and
        // alignment icons and say nothing about weather.
        Condition::Fog => '\u{EDA8}',

        // A single drop, with no cloud above it: the lightest wet mark the
        // font has, and shaped quite unlike the raining cloud that rain gets.
        Condition::Drizzle => '\u{EB42}',

        // A cloud with drops falling from it.
        Condition::Rain => '\u{EA91}',

        // Shares rain's cloud. Nothing in the font distinguishes heavy rain
        // from rain, and of the two marks available for the wet family this is
        // the heavier, so a downpour at least never shows as the drizzle drop.
        Condition::HeavyRain => '\u{EA91}',

        // Shares snow's flake. Freezing rain is half rain and half ice, and the
        // Mac mark says so by putting a flake between two strokes; with one
        // character the ice is the half worth keeping, because it is the half
        // that changes what the user does about it.
        Condition::Sleet => '\u{EA38}',

        Condition::Snow => '\u{EA38}',

        Condition::Thunder => '\u{E945}',

        // Shares the bolt. The font has one lightning glyph and nothing that
        // combines it with rain.
        Condition::Thunderstorm => '\u{E945}',

        Condition::Unknown => '\u{E897}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_condition_maps_to_a_private_use_character() {
        // A codepoint outside the Private Use Area would be a typo that draws
        // a letter from the fallback font rather than an icon, and the strip
        // has no way to notice.
        for condition in Condition::ALL {
            let c = glyph(condition) as u32;
            assert!(
                (0xE000..=0xF8FF).contains(&c),
                "{condition:?} maps to U+{c:04X}, which is not in the Private Use Area"
            );
        }
    }

    #[test]
    fn the_mapping_covers_every_condition() {
        // The match is exhaustive, so this holds the count that ALL and the
        // enum have to agree on rather than the mapping itself.
        assert_eq!(Condition::ALL.len(), 14);
    }

    #[test]
    fn the_night_and_day_glyphs_are_not_the_same() {
        assert_ne!(glyph(Condition::ClearDay), glyph(Condition::ClearNight));
        assert_ne!(
            glyph(Condition::PartlyCloudy),
            glyph(Condition::PartlyCloudyNight)
        );
    }

    #[test]
    fn the_only_conditions_sharing_a_mark_are_the_ones_written_down() {
        // The point of this test is the direction nobody expects: it fails as
        // loudly when a collapse is quietly fixed as when one is quietly
        // added, so SHARED_GLYPHS cannot drift away from what is drawn.
        let mut found = Vec::new();
        for (i, a) in Condition::ALL.iter().enumerate() {
            for b in &Condition::ALL[i + 1..] {
                if glyph(*a) == glyph(*b) {
                    found.push((*a, *b));
                }
            }
        }
        assert_eq!(
            found.len(),
            SHARED_GLYPHS.len(),
            "conditions sharing a mark: {found:?}, but SHARED_GLYPHS lists {SHARED_GLYPHS:?}"
        );
        for pair in SHARED_GLYPHS {
            assert!(
                found.contains(&pair) || found.contains(&(pair.1, pair.0)),
                "{pair:?} is listed as sharing a mark but does not"
            );
        }
    }

    #[test]
    fn a_shared_mark_is_only_ever_shared_with_the_nearest_condition() {
        // Sleet against snow is a compromise; sleet against clear sky would be
        // a bug. Each pair listed has to be adjacent in the order Condition
        // declares, which is severity order.
        let position = |wanted: Condition| {
            Condition::ALL
                .iter()
                .position(|c| *c == wanted)
                .expect("every condition is in ALL")
        };
        for (a, b) in SHARED_GLYPHS {
            let gap = position(b).abs_diff(position(a));
            assert!(
                gap <= 2,
                "{a:?} and {b:?} share a mark but are {gap} apart in severity"
            );
        }
    }

    #[test]
    fn the_font_is_named_rather_than_shipped() {
        // Both are Windows' own fonts, referenced the way Segoe UI is. A path
        // or a file name here would mean an asset had crept into the tree.
        assert_eq!(FAMILY, "Segoe Fluent Icons");
        assert_eq!(FALLBACK_FAMILY, "Segoe MDL2 Assets");
        assert_ne!(FAMILY, FALLBACK_FAMILY);
    }
}

/// A mark built from one glyph, or from two.
///
/// Segoe Fluent Icons carries nine weather marks for fourteen conditions, so
/// picking one glyph each forces five conditions onto a mark that already
/// belongs to another - see `SHARED_GLYPHS`. Composing a second, smaller glyph
/// beside the first gets all fourteen back without leaving the font, and
/// without shipping icon files that would then have to be attributed,
/// rasterized and kept.
///
/// It is also how the marks it does have are *meant* to combine: a cloud with
/// a sun tucked behind it is the shape everybody already reads as partly
/// cloudy, and the font simply does not draw that one for us.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Composed {
    pub base: char,
    pub accent: Option<Accent>,
}

/// A second glyph, drawn smaller and offset from the first.
///
/// Offsets and scale are fractions of the base glyph's size rather than
/// pixels, so a mark is the same mark at every DPI and every type size.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Accent {
    pub glyph: char,
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
}

/// Where a sun or moon sits against a cloud: up and to the right, peeking out
/// from behind it. Small enough to read as behind rather than beside.
///
/// It sticks out above the cloud's own box on purpose - that is what makes it
/// read as behind - which means the pair is taller than one glyph. A painter
/// that has been given a cell to fill draws `fitted()` rather than this, so
/// the overhang is paid for by shrinking the pair, not by drawing into the
/// row above.
const BEHIND: (f32, f32, f32) = (0.34, -0.30, 0.60);

/// Where weather hanging out of a cloud sits: below and slightly right, so it
/// reads as falling rather than as a second icon.
const BENEATH: (f32, f32, f32) = (0.30, 0.32, 0.58);

/// The mark for a condition, as one glyph or two.
///
/// Eight of the fourteen are one glyph and six are a pair, and all fourteen
/// are distinct, which is asserted below. The
/// shape rule from `badge.rs` survives: nothing here depends on color.
pub fn composed(condition: Condition) -> Composed {
    let one = |base: char| Composed { base, accent: None };
    let two = |base: char, glyph: char, (dx, dy, scale): (f32, f32, f32)| Composed {
        base,
        accent: Some(Accent { glyph, dx, dy, scale }),
    };

    // The primitives, named so the compositions below read as what they draw.
    const SUN: char = '\u{E706}';
    const MOON: char = '\u{E708}';
    const CLOUD: char = '\u{E753}';
    const RAINING_CLOUD: char = '\u{EA91}';
    const DROP: char = '\u{EB42}';
    const FLAKE: char = '\u{EA38}';
    const BOLT: char = '\u{E945}';
    const MIST: char = '\u{EDA8}';
    const QUERY: char = '\u{E897}';

    match condition {
        Condition::ClearDay => one(SUN),
        Condition::ClearNight => one(MOON),
        // The two the font refuses to draw, and the whole reason for this.
        Condition::PartlyCloudy => two(CLOUD, SUN, BEHIND),
        Condition::PartlyCloudyNight => two(CLOUD, MOON, BEHIND),
        Condition::Cloudy => one(CLOUD),
        Condition::Fog => one(MIST),
        // A bare drop for drizzle against a raining cloud for rain: less water,
        // and less of a cloud to fall from.
        Condition::Drizzle => one(DROP),
        Condition::Rain => one(RAINING_CLOUD),
        // The same cloud with more coming out of it.
        Condition::HeavyRain => two(RAINING_CLOUD, DROP, BENEATH),
        // Rain and snow together, which is what sleet is.
        Condition::Sleet => two(RAINING_CLOUD, FLAKE, BENEATH),
        Condition::Snow => two(CLOUD, FLAKE, BENEATH),
        // A bolt on its own is thunder heard; a bolt under a raining cloud is
        // the storm it came from.
        Condition::Thunder => one(BOLT),
        Condition::Thunderstorm => two(RAINING_CLOUD, BOLT, BENEATH),
        Condition::Unknown => one(QUERY),
    }
}

/// One glyph of a mark placed in the mark's cell: its top-left corner and its
/// size, all as fractions of the cell's side.
///
/// A glyph from this font occupies exactly the square its size names - the
/// face's ascent is its em and its descent is zero, measured rather than
/// assumed - so a placement is also the box the ink stays inside.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Placed {
    pub glyph: char,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// A mark laid out inside a unit cell, nothing outside it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Fitted {
    pub base: Placed,
    pub accent: Option<Placed>,
}

impl Composed {
    /// The box the two glyphs cover together, in units of the base glyph's
    /// size with the base filling (0, 0) to (1, 1): left, top, right, bottom.
    pub fn bounds(self) -> (f32, f32, f32, f32) {
        let mut bounds = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
        if let Some(accent) = self.accent {
            bounds.0 = bounds.0.min(accent.dx);
            bounds.1 = bounds.1.min(accent.dy);
            bounds.2 = bounds.2.max(accent.dx + accent.scale);
            bounds.3 = bounds.3.max(accent.dy + accent.scale);
        }
        bounds
    }

    /// The mark scaled and shifted, as one piece, until it lies inside the
    /// unit cell, and centered in it along whichever axis has room.
    ///
    /// A mark that already fits comes back exactly as it is, so a plain glyph
    /// and every mark whose accent hangs below the cloud still fill their
    /// cell; only a mark that overhangs gives up size for it. The arrangement
    /// is never changed - the accent keeps its place against the base - so
    /// the sun stays behind the cloud rather than being pushed onto it, and
    /// the night pair still matches the day pair.
    pub fn fitted(self) -> Fitted {
        let (left, top, right, bottom) = self.bounds();
        let (w, h) = (right - left, bottom - top);
        let scale = 1.0 / w.max(h);
        let dx = (1.0 - w * scale) / 2.0 - left * scale;
        let dy = (1.0 - h * scale) / 2.0 - top * scale;
        let place = |glyph: char, x: f32, y: f32, size: f32| Placed {
            glyph,
            x: dx + x * scale,
            y: dy + y * scale,
            size: size * scale,
        };
        Fitted {
            base: place(self.base, 0.0, 0.0, 1.0),
            accent: self.accent.map(|a| place(a.glyph, a.dx, a.dy, a.scale)),
        }
    }
}

impl Fitted {
    /// The box the placed glyphs cover: left, top, right, bottom.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut bounds = (self.base.x, self.base.y, self.base.x + self.base.size, self.base.y + self.base.size);
        if let Some(accent) = &self.accent {
            bounds.0 = bounds.0.min(accent.x);
            bounds.1 = bounds.1.min(accent.y);
            bounds.2 = bounds.2.max(accent.x + accent.size);
            bounds.3 = bounds.3.max(accent.y + accent.size);
        }
        bounds
    }
}

#[cfg(test)]
mod composition_tests {
    use super::*;

    #[test]
    fn composing_gets_all_fourteen_conditions_back() {
        // The point of the exercise. Picking one glyph each collapses five
        // pairs; composing a second glyph must leave none.
        let mut seen: Vec<(Condition, Composed)> = Vec::new();
        for condition in Condition::ALL {
            let mark = composed(condition);
            for (other, previous) in &seen {
                assert_ne!(
                    *previous, mark,
                    "{condition:?} composes the same mark as {other:?}"
                );
            }
            seen.push((condition, mark));
        }
    }

    #[test]
    fn the_pairs_the_font_collapses_are_the_ones_composition_rescues() {
        // Each pair that shares a single glyph must differ once composed, or
        // the composition has not earned its place.
        for (a, b) in SHARED_GLYPHS {
            assert_eq!(glyph(a), glyph(b), "{a:?} and {b:?} no longer share a glyph");
            assert_ne!(composed(a), composed(b), "{a:?} and {b:?} still collapse");
        }
    }

    #[test]
    fn a_sun_or_moon_sits_behind_the_cloud_and_weather_falls_below_it() {
        let day = composed(Condition::PartlyCloudy).accent.unwrap();
        let night = composed(Condition::PartlyCloudyNight).accent.unwrap();
        // Above the cloud's center, which is what "behind" looks like.
        assert!(day.dy < 0.0 && night.dy < 0.0);
        // And in the same place, so the pair read as one mark with a different
        // light in it rather than as two unrelated icons.
        assert_eq!((day.dx, day.dy, day.scale), (night.dx, night.dy, night.scale));

        for condition in [Condition::Snow, Condition::Sleet, Condition::Thunderstorm] {
            assert!(composed(condition).accent.unwrap().dy > 0.0, "{condition:?}");
        }
    }

    #[test]
    fn an_accent_is_always_smaller_than_the_mark_it_sits_against() {
        for condition in Condition::ALL {
            if let Some(accent) = composed(condition).accent {
                assert!(accent.scale > 0.0 && accent.scale < 1.0, "{condition:?}");
            }
        }
    }

    #[test]
    fn every_conditions_fitted_mark_stays_inside_its_cell() {
        // The defect this guards: the crescent over the night cloud drew
        // three tenths of a cell above it, into the hour label of the row
        // above. A painter given a cell must be able to draw the fitted mark
        // and touch nothing outside that cell, whatever the condition.
        for condition in Condition::ALL {
            let (left, top, right, bottom) = composed(condition).fitted().bounds();
            assert!(left >= -1e-5 && top >= -1e-5, "{condition:?} overhangs at {left}, {top}");
            assert!(right <= 1.0 + 1e-5 && bottom <= 1.0 + 1e-5, "{condition:?} overhangs at {right}, {bottom}");
        }
    }

    #[test]
    fn a_mark_that_already_fits_is_drawn_at_full_size_and_one_that_overhangs_gives_up_size_evenly() {
        // A plain glyph and the marks whose accent hangs below the cloud are
        // untouched: nothing was gained by shrinking them.
        let cloud = composed(Condition::Cloudy).fitted();
        assert_eq!((cloud.base.x, cloud.base.y, cloud.base.size), (0.0, 0.0, 1.0));
        assert_eq!(composed(Condition::Snow).fitted().base.size, 1.0);

        // The pairs with a light behind the cloud shrink, as one piece, by
        // exactly their overhang, and sit centered in the room that leaves.
        let day = composed(Condition::PartlyCloudy);
        let (_, top, _, _) = day.bounds();
        assert!(top < 0.0, "the sun is meant to peek above the cloud");
        let fitted = day.fitted();
        let expected = 1.0 / (1.0 - top);
        assert!((fitted.base.size - expected).abs() < 1e-5, "base is {}", fitted.base.size);
        let accent = fitted.accent.unwrap();
        assert!(accent.y.abs() < 1e-5, "the sun now touches the top of the cell, not the row above");
        assert!((fitted.base.y + fitted.base.size - 1.0).abs() < 1e-5, "the cloud rests on the cell's floor");
        let slack = 1.0 - fitted.base.size;
        assert!((fitted.base.x - slack / 2.0).abs() < 1e-5, "centered sideways");
        // The arrangement survives the fit: the accent is where BEHIND put
        // it, in the base's own units.
        let original = day.accent.unwrap();
        assert!(((accent.x - fitted.base.x) / fitted.base.size - original.dx).abs() < 1e-5);
        assert!(((accent.y - fitted.base.y) / fitted.base.size - original.dy).abs() < 1e-5);
        assert!((accent.size / fitted.base.size - original.scale).abs() < 1e-5);
        // And the night pair fits the same way as the day pair.
        let night = composed(Condition::PartlyCloudyNight).fitted();
        assert_eq!((night.base.x, night.base.y, night.base.size), (fitted.base.x, fitted.base.y, fitted.base.size));
    }
}
