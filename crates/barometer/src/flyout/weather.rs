// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The weather flyout, ported from WeatherDropdownView.swift and
// WeatherDayDetailView.swift, as the first Content the panel shows.
//
// Two pages. The overview is the dropdown: the current conditions on a sky
// card, the day ahead as a chart, the ten days as rows, the sun and the
// moon, and a grid of the readings worth a tile. A day's row opens that
// day: its summary, its metrics, its hours with one hour's every reading,
// its sun and moon, and the atmospheric catalog - which on the Mac is a
// second panel that appears beside the row on hover. Windows has no such
// idiom, and a second topmost window beside the first is exactly what the
// dismissal logic would then have to track, so here the day is a page the
// panel turns to and turns back from.
//
// The strip's reading arrives through a feed; the ten-day forecast is this
// content's own business, fetched on a thread of its own the first time the
// panel opens and again when it is stale, never on the panel's thread.

pub mod chart;
pub mod clock;
pub mod format;
pub mod marks;
mod paint;
pub mod sky;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use barometer_core::weather::air::{self, fetch_air, AirQuality};
use barometer_core::weather::badge::Condition;
use barometer_core::weather::client::CurrentWeather;
use barometer_core::weather::detail::{
    fetch_detail, DailyMetric, DailyPoint, DayDetails, DetailForecast, HourlyMetric, HourlyPoint,
    LocalTime,
};
use barometer_core::weather::models::{Location, WeatherUnits};
use barometer_core::ModuleId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_BACK;

use self::format::{Unit, DASH};
use super::cpu::chip_left;
use self::paint::{Chart, Compass, Gauge, Mark, Moon, Range, SkyCard, SkyMark, SkyValue, Sun};
use super::ui::{Accent, Builder, Element, Id, Ink, Kind, Style, Tile, TileIcon, CARD_GAP, ICON_BUTTON};
use super::{Action, Command, Content, Context, Page, Response, Wake};
use crate::settings_ui::gdi::Align;
use crate::settings_ui::geometry::Rect;
use crate::settings_ui::system;
use crate::settings_ui::theme::{glyph, Color};

/// The interface glyphs this panel draws beside its words.
///
/// The weather marks themselves come from glyph.rs; these are the
/// primitives that sweep found and the interface glyphs docs/ui-design.md
/// section 6 verifies, named here because theme.rs's table belongs to the
/// settings window.
mod icons {
    pub const SUN: &str = "\u{E706}";
    pub const CLOUD: &str = "\u{E753}";
    pub const RAINING_CLOUD: &str = "\u{EA91}";
    pub const DROP: &str = "\u{EB42}";
    pub const MIST: &str = "\u{EDA8}";
    pub const EYE: &str = "\u{E7B3}";
    pub const WARNING: &str = "\u{E7BA}";
    pub const BACK: &str = "\u{E72B}";
}

const OPEN_METEO_URL: &str = "https://open-meteo.com/";

/// The controls this content lays out, as the numbers Id::Custom carries.
const ID_BACK: u32 = 1;
const ID_REFRESH: u32 = 2;
const ID_ATTRIBUTION: u32 = 3;
/// The location chips answer to this plus the location's place in the list.
const ID_LOCATION: u32 = 1 << 20;
/// The overview's hourly chart, which scrolls sideways under its plate.
const ID_CHART: u32 = 4;
/// A day's hourly chart, one id per day so each opens on its own hour.
const ID_DAY_CHART: u32 = 10;
const ID_DAY: u32 = 100;
const ID_HOUR: u32 = 200;

fn day_id(index: usize) -> Id {
    Id::Custom(ID_DAY + index as u32)
}

fn day_chart_id(index: usize) -> Id {
    Id::Custom(ID_DAY_CHART + (index as u32).min(ID_DAY - ID_DAY_CHART - 1))
}

fn hour_id(index: usize) -> Id {
    Id::Custom(ID_HOUR + index as u32)
}

fn day_index(id: Id) -> Option<usize> {
    match id {
        Id::Custom(n) if (ID_DAY..ID_HOUR).contains(&n) => Some((n - ID_DAY) as usize),
        _ => None,
    }
}

fn hour_index(id: Id) -> Option<usize> {
    match id {
        // Bounded, not open-ended: the location chips are numbered above
        // the hours, and an unbounded range read a chip as an hour of the
        // day and opened that hour instead of changing the place.
        Id::Custom(n) if (ID_HOUR..ID_LOCATION).contains(&n) => Some((n - ID_HOUR) as usize),
        _ => None,
    }
}

/// A day's row, and the sun plate.
const DAY_ROW_H: f32 = 30.0;
const SUN_PLATE_H: f32 = 96.0;
/// The headline temperature's size, from the Swift.
const HERO_TEMPERATURE: f32 = 40.0;
const HERO_PAD: f32 = 14.0;

/// What the strip knows, published on every tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub location: Option<Location>,
    /// Every saved location, in the order the user arranged them, so the
    /// panel can offer them the way the Swift's Location picker does.
    pub locations: Vec<Location>,
    pub units: WeatherUnits,
    pub current: Option<CurrentWeather>,
    /// Why the strip's last refresh failed, if it did.
    pub error: Option<String>,
    /// How often the strip refreshes, which is how long a forecast is
    /// trusted for here too.
    pub refresh_minutes: u32,
}

/// Which page the panel is on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Shown {
    Overview,
    Day(usize),
}

/// A fetch's answer: which city it is about, the forecast, and the air,
/// which is asked of a different service and may not answer when the
/// forecast does.
///
/// The city rides along because a fetch in flight cannot be called back. Pick
/// London while Austin is still being fetched and the answer that arrives is
/// Austin's; without the city on it, it was accepted and then labeled from
/// whatever the panel was showing by then, so Austin's ten days, chart,
/// sunrise and moon appeared under London's name.
type FetchResult = Option<(u64, String, Result<DetailForecast, String>, Option<AirQuality>)>;

pub struct WeatherContent {
    feed: Arc<Mutex<Snapshot>>,
    snapshot: Snapshot,
    /// When the strip's reading last changed, as seconds since the epoch.
    current_since: Option<i64>,
    forecast: Option<DetailForecast>,
    /// The air at the same place, when the air-quality service answered.
    air: Option<AirQuality>,
    /// The id of the location the forecast is for.
    forecast_for: Option<String>,
    fetched_at: Option<Instant>,
    fetch_error: Option<String>,
    fetching: bool,
    /// Counts fetches, so a slow answer to an old request cannot replace a
    /// newer one.
    generation: u64,
    results: Arc<Mutex<FetchResult>>,
    wake: Option<Wake>,
    shown: Shown,
    selected_hour: Option<usize>,
    /// The minute the page was last laid out in, so the "Updated" line can
    /// tick over without a reading changing.
    built_minute: Option<i64>,
}

impl WeatherContent {
    /// Content reading the strip's weather from a feed's slot.
    pub fn new(feed: Arc<Mutex<Snapshot>>) -> WeatherContent {
        WeatherContent {
            feed,
            snapshot: Snapshot::default(),
            current_since: None,
            forecast: None,
            air: None,
            forecast_for: None,
            fetched_at: None,
            fetch_error: None,
            fetching: false,
            generation: 0,
            results: Arc::new(Mutex::new(None)),
            wake: None,
            shown: Shown::Overview,
            selected_hour: None,
            built_minute: None,
        }
    }

    /// Takes the feed's latest, noting when the reading itself changed.
    fn sync_snapshot(&mut self) -> bool {
        let Some(latest) = self.feed.lock().ok().map(|slot| slot.clone()) else {
            return false;
        };
        if latest == self.snapshot {
            return false;
        }
        if latest.current != self.snapshot.current {
            self.current_since = Some(clock::unix_now());
        }
        self.snapshot = latest;
        true
    }

    /// Whether the forecast held is for somewhere else, or older than the
    /// strip's own refresh interval.
    fn stale(&self) -> bool {
        let Some(location) = self.snapshot.location.as_ref() else {
            return false;
        };
        if self.forecast_for.as_deref() != Some(location.id.as_str()) {
            return true;
        }
        let minutes = u64::from(self.snapshot.refresh_minutes.clamp(5, 60));
        self.fetched_at.is_none_or(|at| at.elapsed() > Duration::from_secs(minutes * 60))
    }

    /// Starts a fetch if one is wanted and none is running.
    fn maybe_fetch(&mut self, force: bool) {
        let Some(location) = self.snapshot.location.clone() else {
            return;
        };
        if self.fetching || !(force || self.forecast.is_none() || self.stale()) {
            return;
        }
        self.generation += 1;
        self.fetching = true;
        self.fetch_error = None;
        let generation = self.generation;
        let units = self.snapshot.units;
        let results = Arc::clone(&self.results);
        let wake = self.wake.clone();
        thread::spawn(move || {
            let outcome = fetch_detail(&location, units).map_err(|why| why.to_string());
            // The air is a second question to a second service; a forecast
            // without it is still a forecast.
            let air = fetch_air(&location).ok().filter(|air| !air.is_empty());
            if let Ok(mut slot) = results.lock() {
                *slot = Some((generation, location.id.clone(), outcome, air));
            }
            if let Some(wake) = wake {
                wake.notify();
            }
        });
    }

    /// Collects a finished fetch, if one is waiting.
    fn take_result(&mut self) -> bool {
        let Some((generation, about, outcome, air)) =
            self.results.lock().ok().and_then(|mut slot| slot.take())
        else {
            return false;
        };
        if generation != self.generation {
            return false;
        }
        self.fetching = false;
        // The city moved while this was in flight. Nothing here describes the
        // city on screen, so none of it is kept, and the fetch the change
        // could not start - `maybe_fetch` refuses while one is running - is
        // started now.
        let showing = self.snapshot.location.as_ref().map(|location| location.id.clone());
        if showing.as_deref() != Some(about.as_str()) {
            self.maybe_fetch(false);
            return true;
        }
        match outcome {
            Ok(forecast) => {
                self.forecast = Some(forecast);
                self.air = air;
                self.forecast_for = showing;
                self.fetched_at = Some(Instant::now());
                self.fetch_error = None;
            }
            Err(why) => self.fetch_error = Some(why),
        }
        true
    }
}

impl Content for WeatherContent {
    fn module(&self) -> ModuleId {
        ModuleId::Weather
    }

    fn attach(&mut self, wake: Wake) {
        self.wake = Some(wake);
    }

    fn opened(&mut self) {
        self.sync_snapshot();
        self.shown = Shown::Overview;
        self.selected_hour = None;
        self.maybe_fetch(false);
    }

    fn tick(&mut self) -> bool {
        let changed = self.sync_snapshot();
        let arrived = self.take_result();
        if changed {
            // A location the user just changed wants its own forecast.
            self.maybe_fetch(false);
        }
        let minute = clock::unix_now() / 60;
        changed || arrived || self.built_minute != Some(minute)
    }

    fn actions(&self) -> Vec<Action> {
        vec![Action {
            id: Id::Custom(ID_REFRESH),
            text: "Refresh",
            glyph: Some(glyph::REFRESH),
            enabled: !self.fetching && self.snapshot.location.is_some(),
        }]
    }

    fn activate(&mut self, id: Id) -> Response {
        if let Some(index) = day_index(id) {
            self.shown = Shown::Day(index);
            self.selected_hour = None;
            return Response::Relayout;
        }
        if let Some(index) = hour_index(id) {
            self.selected_hour = Some(index);
            return Response::Relayout;
        }
        match id {
            Id::Custom(ID_BACK) => {
                self.shown = Shown::Overview;
                Response::Relayout
            }
            Id::Custom(ID_REFRESH) => {
                self.maybe_fetch(true);
                // The strip's own reading is the module's to refresh.
                Response::Request(Command::Refresh(ModuleId::Weather))
            }
            id if location_index(id).is_some() => {
                let Some(place) = location_index(id).and_then(|index| self.snapshot.locations.get(index)) else {
                    return Response::None;
                };
                // The settings' primary location is the strip's location and
                // this panel's; the thread that owns the settings changes it,
                // and the new forecast arrives through the feed.
                Response::Request(Command::ShowLocation(place.id.clone()))
            }
            Id::Custom(ID_ATTRIBUTION) => {
                system::open_url(OPEN_METEO_URL);
                Response::None
            }
            _ => Response::None,
        }
    }

    /// Hovering an hour on the day page chooses it, as it does on the Mac.
    fn hover(&mut self, id: Option<Id>) -> bool {
        let Some(index) = id.and_then(hour_index) else {
            return false;
        };
        if matches!(self.shown, Shown::Day(_)) && self.selected_hour != Some(index) {
            self.selected_hour = Some(index);
            return true;
        }
        false
    }

    fn key(&mut self, key: u16) -> Response {
        if key == VK_BACK && matches!(self.shown, Shown::Day(_)) {
            self.shown = Shown::Overview;
            return Response::Relayout;
        }
        Response::None
    }

    fn build(&mut self, cx: &Context) -> Page {
        self.built_minute = Some(cx.now_unix / 60);
        let view = View {
            snapshot: &self.snapshot,
            forecast: self.forecast.as_ref(),
            air: self.air.as_ref(),
            fetching: self.fetching,
            fetch_error: self.fetch_error.as_deref(),
            now_unix: cx.now_unix,
            current_since: self.current_since,
            selected_hour: self.selected_hour,
            accent: cx.accent,
            light: cx.palette.theme.light,
            secondary: cx.palette.theme.text_secondary,
        };
        let mut b = cx.builder();
        match self.shown {
            Shown::Overview => overview(&mut b, &view),
            Shown::Day(index) => day_page(&mut b, &view, index),
        }
        Page::finish(b)
    }
}

/// Everything a page is laid out from.
struct View<'a> {
    snapshot: &'a Snapshot,
    forecast: Option<&'a DetailForecast>,
    air: Option<&'a AirQuality>,
    fetching: bool,
    fetch_error: Option<&'a str>,
    now_unix: i64,
    current_since: Option<i64>,
    selected_hour: Option<usize>,
    accent: Accent,
    /// Whether the cards are light, for the inks that have a darker twin.
    light: bool,
    /// The secondary text ink, for the chip beside the hourly chart.
    secondary: Color,
}

impl View<'_> {
    fn units(&self) -> WeatherUnits {
        self.snapshot.units
    }

    fn rain(&self) -> Ink {
        Ink::Custom(sky::rain_ink(self.light))
    }

    fn sun(&self) -> Ink {
        Ink::Custom(sky::sun_ink(self.light))
    }

    /// The wall clock at the location right now.
    fn now(&self, forecast: &DetailForecast) -> LocalTime {
        clock::local_now(forecast.utc_offset_seconds, self.now_unix)
    }

    /// The forecast's point for the hour in progress.
    fn current_hour<'f>(&self, forecast: &'f DetailForecast) -> Option<&'f HourlyPoint> {
        let hour = clock::floor_hour(self.now(forecast));
        forecast.hourly.iter().find(|point| point.time == hour)
    }

    /// "Add a location in Weather settings." or "Waiting for the first
    /// forecast.", from emptyStateDescription.
    fn empty_state(&self) -> &str {
        if self.snapshot.location.is_none() {
            "Add a location in Weather settings."
        } else {
            "Waiting for the first forecast."
        }
    }

    /// "Updated 3:42 PM · 5 min ago", once the reading's time is known.
    fn updated(&self) -> Option<String> {
        let since = self.current_since?;
        let elapsed = ((self.now_unix - since).max(0) / 60) as u64;
        match self.forecast {
            Some(forecast) => {
                let at = clock::local_now(forecast.utc_offset_seconds, since);
                Some(format::updated(&clock::time_short(at), elapsed))
            }
            // Without the location's zone yet there is no clock to print
            // the time on, and "5 min ago" is still true.
            None => Some(format!("Updated {}", format::relative(elapsed))),
        }
    }
}

// ---------------------------------------------------------------------------
// The overview
// ---------------------------------------------------------------------------

fn overview(b: &mut Builder, view: &View) {
    hero(b, view);
    if view.snapshot.current.is_some() {
        if let Some(why) = view.snapshot.error.as_deref() {
            notice(b, &format!("Refresh failed; showing saved weather. {why}"));
        }
    }
    if let Some(why) = view.fetch_error {
        notice(b, &format!("The forecast could not be fetched. {why}"));
    }
    match view.forecast {
        Some(forecast) => {
            hourly_section(b, view, forecast);
            daily_section(b, view, forecast);
            if let Some(today) = forecast.day(0) {
                sun_moon_section(b, view, forecast, &today, true);
            }
            air_section(b, view);
            details_section(b, view, forecast);
            location_section(b, view);
        }
        None => {
            let card = b.card_begin(None);
            if view.snapshot.location.is_none() {
                b.unavailable(icons::CLOUD, "Weather unavailable", view.empty_state());
            } else if view.fetching {
                b.unavailable(icons::CLOUD, "Fetching the forecast", "Open-Meteo is being asked for the ten days ahead.");
            } else {
                b.unavailable(icons::CLOUD, "Forecast unavailable", "Refresh to try again.");
            }
            b.card_end(card);
        }
    }
    b.centered_link(Id::Custom(ID_ATTRIBUTION), "Weather data by Open-Meteo.com");
}

/// The sky card with the current conditions, from CurrentWeatherCard.
fn hero(b: &mut Builder, view: &View) {
    let units = view.units();
    let current = view.snapshot.current.as_ref();
    let is_day = current.map(|c| c.is_day).unwrap_or(true);
    let condition = current
        .map(|c| Condition::for_code(c.weather_code.unwrap_or(255), c.is_day))
        .unwrap_or(Condition::Unknown);
    let sky = sky::sky(condition, is_day);
    let on_sky = Ink::Custom(sky::on_sky());
    let dim = Ink::Custom(sky::on_sky_dim(sky));

    let card_top = b.y();
    let index = b.elements.len();
    b.passive(Rect::new(b.x(), card_top, b.width(), 0.0), Kind::Custom(Box::new(SkyCard { sky })));

    let top = card_top + HERO_PAD;
    let x = b.x() + HERO_PAD;
    let right = b.right() - HERO_PAD;
    let temperature = format::temperature(current.and_then(|c| c.temperature), units);
    let temperature_w = b.text_width(&temperature, Style::Display(HERO_TEMPERATURE)) + 6.0;
    let mark_w = 60.0;
    let text_x = x + mark_w + 14.0;
    let text_w = (right - temperature_w - 8.0 - text_x).max(40.0);
    // Name, condition, feels-like: 28, 20 and 16 with three between.
    let block_h = 70.0;

    b.passive(
        Rect::new(x, top + (block_h - 46.0) / 2.0, mark_w, 46.0),
        Kind::Custom(Box::new(SkyMark { condition, size: 46.0 })),
    );
    let name = view.snapshot.location.as_ref().map(|l| l.name.clone()).unwrap_or_else(|| "Weather".to_string());
    b.text(Rect::new(text_x, top, text_w, 28.0), &name, Style::Subtitle, on_sky, Align::Left);
    let description = match current {
        Some(current) => format::description(current.weather_code).to_string(),
        None => view.snapshot.error.clone().unwrap_or_else(|| view.empty_state().to_string()),
    };
    b.text(Rect::new(text_x, top + 31.0, text_w, 20.0), &description, Style::Body, on_sky, Align::Left);
    if let Some(feels) = current.and_then(|c| c.apparent_temperature) {
        let feels = format!("Feels like {}", format::temperature(Some(feels), units));
        b.text(Rect::new(text_x, top + 54.0, text_w, 16.0), &feels, Style::Caption, dim, Align::Left);
    }
    b.passive(
        Rect::new(right - temperature_w, top + (block_h - 48.0) / 2.0, temperature_w, 48.0),
        Kind::Custom(Box::new(SkyValue { text: temperature, size: HERO_TEMPERATURE })),
    );

    let mut bottom = top + block_h;
    if let Some(updated) = view.updated() {
        let line_y = bottom + 9.0;
        b.passive(Rect::new(x, line_y, 14.0, 16.0), Kind::Glyph { glyph: glyph::REFRESH, size: 11.0, ink: dim });
        b.text(Rect::new(x + 18.0, line_y, right - x - 18.0, 16.0), &updated, Style::Caption, dim, Align::Left);
        bottom = line_y + 16.0;
    }
    bottom += HERO_PAD;
    b.elements[index].rect.h = bottom - card_top;
    b.advance(bottom - card_top + CARD_GAP);
}

/// A line with a warning glyph, for a refresh that failed.
fn notice(b: &mut Builder, text: &str) {
    let card = b.card_begin(None);
    let y = b.y();
    b.passive(Rect::new(b.inner_x(), y, 16.0, 16.0), Kind::Glyph { glyph: icons::WARNING, size: 12.0, ink: Ink::Caution });
    b.wrapped_in(b.inner_x() + 24.0, b.inner_w() - 24.0, text, Style::Caption, Ink::Caution, Align::Left);
    b.card_end(card);
}

/// The two days ahead as a chart that scrolls sideways under its plate,
/// from HourlyForecastSection.
fn hourly_section(b: &mut Builder, view: &View, forecast: &DetailForecast) {
    let hours = chart::hours_to_plot(&forecast.hourly, view.now(forecast), chart::PLOTTED_HOURS);
    let card = b.card_begin(Some(view.accent.primary));
    let trailing = b.section_label("Next 48 hours");
    if hours.is_empty() {
        b.caption("Hourly details are unavailable.");
    } else {
        // The Swift's chip, in its words: a plate that scrolls sideways
        // inside a panel that scrolls up and down has to say so.
        b.chip_at(trailing, "Scroll for more", view.secondary, Some(glyph::CHEVRON_RIGHT));
        let plate = b.plate(chart::HEIGHT);
        let content_w = chart::content_width(hours.len(), chart::OVERVIEW_MIN_W);
        let geometry = chart::Geometry::new(&hours, content_w, plate.h);
        b.push(Element {
            id: Id::Custom(ID_CHART),
            rect: plate,
            kind: Kind::Scroll { content_w, offset: 0.0, painter: Box::new(Chart { geometry, selected: None }) },
            interactive: false,
            zones: Vec::new(),
        });
    }
    b.card_end(card);
}

/// Where a day's range sits on its track, from TemperatureRangeBar.
///
/// The two bounds come from different fields, so a day reporting a low with
/// no high can start past the end, and a range narrower than the minimum
/// width would otherwise be pushed off the track.
pub fn range_bar(track: Rect, low: Option<f64>, high: Option<f64>, minimum: f64, maximum: f64) -> Rect {
    let spread = (maximum - minimum).max(1.0);
    let low = low.unwrap_or(minimum);
    let high = high.unwrap_or(maximum);
    let start = ((low - minimum) / spread).clamp(0.0, 1.0);
    let end = ((high - minimum) / spread).clamp(0.0, 1.0).max(start);
    let length = (((end - start) as f32) * track.w).max(6.0).min(track.w);
    let offset = ((start as f32) * track.w).min(track.w - length);
    Rect::new(track.x + offset, track.y, length, track.h)
}

/// The ten days as rows, from DailyForecastSection.
fn daily_section(b: &mut Builder, view: &View, forecast: &DetailForecast) {
    let daily = &forecast.daily;
    if daily.is_empty() {
        return;
    }
    let units = view.units();
    let minimum = daily.iter().filter_map(|day| day.low).fold(f64::INFINITY, f64::min);
    let minimum = if minimum.is_finite() { minimum } else { 0.0 };
    let maximum = daily.iter().filter_map(|day| day.high).fold(f64::NEG_INFINITY, f64::max);
    let maximum = if maximum.is_finite() { maximum } else { minimum + 1.0 };

    let card = b.card_begin(None);
    b.section_label("10-day forecast");
    for (index, day) in daily.iter().enumerate() {
        let row = b.row_begin(day_id(index), DAY_ROW_H, false);
        let (y, h) = (row.y, row.h);
        let mut x = row.x + 6.0;
        b.text(Rect::new(x, y, 36.0, h), clock::weekday_short(day.date), Style::Body, Ink::Primary, Align::Left);
        x += 36.0 + 9.0;
        if let Some(code) = day.weather_code {
            let condition = Condition::for_code(code, true);
            b.passive(Rect::new(x, y + (h - 22.0) / 2.0, 22.0, 22.0), Kind::Custom(Box::new(Mark { condition, size: 22.0 })));
        }
        x += 22.0 + 9.0;
        if let Some(chance) = day.precipitation_probability.filter(|chance| *chance > 0.0) {
            b.text(Rect::new(x, y, 30.0, h), &format::percent(Some(chance)), Style::Caption, view.rain(), Align::Right);
        }
        x += 30.0 + 9.0;
        b.text(Rect::new(x, y, 32.0, h), &format::degree(day.low), Style::Body, Ink::Secondary, Align::Right);
        x += 32.0 + 9.0;
        // A chevron at the end, as Windows' own rows that open a page carry
        // one: the hover wash alone did not say these rows go anywhere.
        let chevron_w = 12.0;
        let high_x = row.right() - 6.0 - chevron_w - 6.0 - 32.0;
        let track = Rect::new(x, y + (h - 6.0) / 2.0, (high_x - 9.0 - x).max(20.0), 6.0);
        b.passive(track, Kind::Track);
        let bar = range_bar(track, day.low, day.high, minimum, maximum);
        let low_c = sky::Scale::celsius(day.low.unwrap_or(minimum), units.temperature);
        let high_c = sky::Scale::celsius(day.high.unwrap_or(maximum), units.temperature);
        b.passive(
            bar,
            Kind::Custom(Box::new(Range { stops: sky::Scale::gradient(low_c, high_c), glow: sky::Scale::color(high_c) })),
        );
        b.text(Rect::new(high_x, y, 32.0, h), &format::degree(day.high), Style::Body, Ink::Primary, Align::Right);
        b.passive(
            Rect::new(row.right() - 6.0 - chevron_w, y, chevron_w, h),
            Kind::Glyph { glyph: glyph::CHEVRON_RIGHT, size: 9.0, ink: Ink::Secondary },
        );
        b.advance(DAY_ROW_H);
    }
    b.card_end(card);
}

/// "Moonrise 2:10 AM", or the sentence for a day without one.
///
/// A day with no moonrise is ordinary - the moon rises about fifty minutes
/// later each day and skips one roughly monthly - so it is said as a plain
/// fact rather than as a value that failed to arrive.
fn moon_event(name: &str, time: Option<LocalTime>, day: &DailyPoint) -> String {
    match time {
        Some(time) => format!("{name} {}", clock::time_short(time)),
        None if day.moon_events_available => format!("No {} today", name.to_lowercase()),
        None => format!("{name} unavailable"),
    }
}

/// Sunrise, sunset, daylight, and the moon, from SunMoonSection and the
/// sunAndMoon card of the day view.
///
/// The Mac's overview shows three tiles and its day view a list; both are
/// here as one card with the sun drawn on its arc, since the arc says at a
/// glance what "Sunrise 6:58 AM · Sunset 7:41 PM" makes the reader work out.
fn sun_moon_section(b: &mut Builder, view: &View, forecast: &DetailForecast, day: &DayDetails, today: bool) {
    let card = b.card_begin(None);
    b.section_label("Sun & moon");
    let plate = b.plate(SUN_PLATE_H);
    let progress = if today { marks::day_progress(day.day.sunrise, day.day.sunset, view.now(forecast)) } else { None };
    let path = marks::sun_path(Rect::new(0.0, 0.0, plate.w, plate.h), progress);
    b.passive(plate, Kind::Custom(Box::new(Sun { path })));
    let text_y = plate.bottom() - marks::SUN_PATH_FOOTER + 8.0;
    // A time is "12:00 AM" at the widest; the daylight sentence in the
    // middle is the one that needs the room.
    let side = 84.0;
    b.passive(Rect::new(plate.x + 10.0, text_y, 12.0, 16.0), Kind::Glyph { glyph: glyph::UP, size: 10.0, ink: Ink::Secondary });
    b.text(Rect::new(plate.x + 24.0, text_y, side - 24.0, 16.0), &clock::time_or_dash(day.day.sunrise), Style::Caption, Ink::Primary, Align::Left);
    b.text(Rect::new(plate.x + side, text_y, plate.w - 2.0 * side, 16.0), &format::daylight(day.daylight_duration()), Style::Caption, Ink::Secondary, Align::Center);
    b.text(Rect::new(plate.right() - side, text_y, side - 24.0, 16.0), &clock::time_or_dash(day.day.sunset), Style::Caption, Ink::Primary, Align::Right);
    b.passive(Rect::new(plate.right() - 22.0, text_y, 12.0, 16.0), Kind::Glyph { glyph: glyph::DOWN, size: 10.0, ink: Ink::Secondary });

    b.gap(12.0);
    b.divider();
    b.gap(12.0);

    let moon = 44.0;
    let y = b.y();
    b.passive(Rect::new(b.inner_x() + 4.0, y + 6.0, moon, moon), Kind::Custom(Box::new(Moon { disc: marks::moon(day.moon_phase, day.moon_illumination) })));
    let text_x = b.inner_x() + 4.0 + moon + 14.0;
    let text_w = b.inner_w() - (text_x - b.inner_x());
    b.text(Rect::new(text_x, y, text_w, 20.0), day.moon_phase.name(), Style::BodyStrong, Ink::Primary, Align::Left);
    b.text(Rect::new(text_x, y + 20.0, text_w, 20.0), &format::illumination(day.moon_illumination), Style::Body, Ink::Secondary, Align::Left);
    // Said out loud, as the Swift says it: an estimate is a different kind
    // of number from a reading.
    let source = if day.uses_estimated_moon_phase { "Estimated at local noon" } else { "Daily lunar phase \u{00B7} Open-Meteo" };
    b.text(Rect::new(text_x, y + 40.0, text_w, 16.0), source, Style::Caption, Ink::Secondary, Align::Left);
    b.advance(56.0 + 8.0);

    let half = b.inner_w() / 2.0;
    let y = b.y();
    b.text(Rect::new(b.inner_x(), y, half, 16.0), &moon_event("Moonrise", day.day.moonrise, &day.day), Style::Caption, Ink::Secondary, Align::Left);
    b.text(Rect::new(b.inner_x() + half, y, half, 16.0), &moon_event("Moonset", day.day.moonset, &day.day), Style::Caption, Ink::Secondary, Align::Right);
    b.advance(16.0);
    b.card_end(card);
}

/// The readings worth a tile, from WeatherDetailsSection.
///
/// The Swift's six plus the UV index and visibility, and without the
/// precipitation total that the chart's bars already show. Cloud cover,
/// gusts, UV and visibility come from the forecast's hour in progress,
/// because the strip's own reading does not carry them.
fn details_section(b: &mut Builder, view: &View, forecast: &DetailForecast) {
    let units = view.units();
    let current = view.snapshot.current.as_ref();
    let hour = view.current_hour(forecast);
    let rain = view.rain();
    let amber = view.sun();

    let humidity = current.and_then(|c| c.relative_humidity).or(hour.and_then(|h| h.humidity));
    let wind_speed = current.and_then(|c| c.wind_speed).or(hour.and_then(|h| h.wind_speed));
    let wind_direction = current.and_then(|c| c.wind_direction).or(hour.and_then(|h| h.wind_direction));
    let pressure = current.and_then(|c| c.pressure_hectopascals).or(hour.and_then(|h| h.details.get(HourlyMetric::SeaLevelPressure)));
    let gusts = hour.and_then(|h| h.details.get(HourlyMetric::WindGusts));
    let precipitation = current.and_then(|c| c.precipitation).or(hour.and_then(|h| h.precipitation));

    let tiles = vec![
        Tile { icon: TileIcon::Glyph(icons::DROP), label: "Humidity".into(), value: format::percent(humidity), tint: rain },
        Tile {
            icon: match wind_direction {
                Some(bearing) => TileIcon::Custom(Box::new(Compass { bearing })),
                None => TileIcon::Glyph(icons::MIST),
            },
            label: "Wind".into(),
            value: format::wind(wind_speed, wind_direction, units),
            tint: Ink::Accent,
        },
        Tile {
            icon: match pressure {
                Some(hpa) => TileIcon::Custom(Box::new(Gauge { fraction: marks::pressure_fraction(hpa) })),
                None => TileIcon::None,
            },
            label: "Pressure".into(),
            value: format::pressure(pressure, units),
            tint: Ink::Accent,
        },
        Tile { icon: TileIcon::Glyph(icons::SUN), label: "UV index".into(), value: format::number(hour.and_then(|h| h.uv_index)), tint: amber },
        Tile {
            icon: TileIcon::Glyph(icons::CLOUD),
            label: "Cloud cover".into(),
            value: format::percent(hour.and_then(|h| h.cloud_cover)),
            tint: Ink::Custom(sky::cloud_ink(view.light)),
        },
        Tile { icon: TileIcon::Glyph(icons::MIST), label: "Gusts".into(), value: format::wind(gusts, None, units), tint: Ink::Accent },
        Tile { icon: TileIcon::Glyph(icons::RAINING_CLOUD), label: "Precipitation".into(), value: format::precipitation(precipitation, units), tint: rain },
        Tile {
            icon: TileIcon::Glyph(icons::EYE),
            label: "Visibility".into(),
            value: format::visibility(hour.and_then(|h| h.visibility), units),
            tint: Ink::Custom(sky::mist_ink(view.light)),
        },
    ];
    let card = b.card_begin(None);
    b.section_label("Details");
    b.stat_tiles(with_values(tiles));
    b.card_end(card);
}

// ---------------------------------------------------------------------------
// Where this is the weather for
// ---------------------------------------------------------------------------

/// The location row, from locationAndActions: the place this forecast is
/// for, and the other saved places beside it to switch to.
///
/// The Swift shows a chip for one location and a menu for several; a menu
/// inside a panel that is itself a menu is a control nobody expects, so
/// several are chips too - the chosen one in the accent, the rest neutral,
/// which is the same picker the stack's tabs and the CPU's ranges use.
/// Refresh and "open the Weather app" are not here: Refresh is already the
/// footer's action, and Windows has no weather application to open that
/// every machine has.
fn location_section(b: &mut Builder, view: &View) {
    // With nothing saved - a machine that found itself by address, which is
    // this app's default where the Swift's is not - the row is the one place
    // the forecast is for, as the Swift's else-branch is a chip of exactly
    // that. It is not a control: there is nothing else to choose.
    let saved = &view.snapshot.locations;
    let lone: Vec<Location>;
    let places = if saved.is_empty() {
        let Some(here) = view.snapshot.location.clone() else { return };
        lone = vec![here];
        &lone
    } else {
        saved
    };
    let here = view.snapshot.location.as_ref().map(|location| location.id.as_str());
    let card = b.card_begin(None);
    b.section_label("Location");
    let (left, right) = (b.inner_x(), b.inner_x() + b.inner_w());
    let (mut x, mut y) = (left, b.y());
    for (index, place) in places.iter().enumerate() {
        let chosen = here == Some(place.id.as_str());
        let color = if chosen { view.accent.primary } else { LOCATION_NEUTRAL };
        let mut placed = chip_left(b, x, y, &place.name, color, None);
        let chip = b.elements.last_mut().expect("chip_left pushes one element");
        // Only the placed chip knows its width, so one that ran past the
        // edge is moved down a line after the fact, as the stack's tabs are.
        if index > 0 && placed.right() > right {
            x = left;
            y += LOCATION_CHIP_H + LOCATION_LINE_GAP;
            chip.rect.x = x;
            chip.rect.y = y;
            placed = chip.rect;
        }
        // The place already shown is not a control; pressing it would ask
        // for what is already true.
        if !chosen {
            chip.id = Id::Custom(ID_LOCATION + index as u32);
            chip.interactive = true;
        }
        x = placed.right() + LOCATION_CHIP_GAP;
    }
    b.advance(y - b.y() + LOCATION_CHIP_H);
    b.card_end(card);
}

/// The chip metrics the stack's switcher uses, so the two rows of chips
/// are the same row of chips.
const LOCATION_CHIP_H: f32 = 20.0;
const LOCATION_CHIP_GAP: f32 = 2.0;
const LOCATION_LINE_GAP: f32 = 6.0;
/// An unchosen chip, as the CPU panel's idle share is.
const LOCATION_NEUTRAL: Color = Color(0x8A8A8A);

/// The saved location a chip stands for.
fn location_index(id: Id) -> Option<usize> {
    match id {
        Id::Custom(n) if (ID_LOCATION..ID_LOCATION + 1024).contains(&n) => Some((n - ID_LOCATION) as usize),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The air
// ---------------------------------------------------------------------------

/// The air, from AirQualitySection: the US index with its band as a chip, a
/// scale with the reading on it, and the pollutants as tiles. Nothing at all
/// when the service did not answer, as the Swift shows nothing.
fn air_section(b: &mut Builder, view: &View) {
    let Some(air) = view.air else { return };
    if air.is_empty() {
        return;
    }
    let card = b.card_begin(None);
    let trailing = b.section_label("Air quality");
    if let Some(aqi) = air.us_aqi {
        let color = aqi_color(aqi);
        b.chip_at(trailing, air::describe(aqi), color, None);
        let line = Rect::new(b.inner_x(), b.y(), b.inner_w(), AQI_LINE_H);
        let number = aqi.to_string();
        let number_w = b.text_width(&number, Style::Title) + 4.0;
        b.text(Rect::new(line.x, line.y, number_w, line.h), &number, Style::Title, Ink::Custom(color), Align::Left);
        b.text(
            Rect::new(line.x + number_w + 8.0, line.y, (line.w - number_w - 8.0).max(24.0), line.h),
            "US AQI",
            Style::Caption,
            Ink::Secondary,
            Align::Left,
        );
        b.advance(AQI_LINE_H + 4.0);
        // The scale runs to 300, as the Swift's does; hazardous air pins the
        // mark to the end rather than off it.
        let fraction = (aqi as f32 / 300.0).clamp(0.0, 1.0);
        let track = Rect::new(b.inner_x(), b.y(), b.inner_w(), 8.0);
        b.passive(track, Kind::Track);
        b.passive(track, Kind::Capsule { fraction, color, color2: color, glow: true });
        b.advance(8.0 + 10.0);
    }
    let tile = |label: &str, value: Option<f64>| {
        value.map(|v| Tile {
            icon: TileIcon::Glyph(icons::MIST),
            label: label.to_string(),
            value: format!("{v:.1} \u{00B5}g/m\u{00B3}"),
            tint: Ink::Accent,
        })
    };
    let tiles: Vec<Tile> = [tile("PM2.5", air.pm2_5), tile("PM10", air.pm10), tile("Ozone", air.ozone)]
        .into_iter()
        .flatten()
        .collect();
    if !tiles.is_empty() {
        b.stat_tiles(tiles);
    }
    b.card_end(card);
}

/// The height of the line the index sits on: a title-sized number.
const AQI_LINE_H: f32 = 34.0;

/// The EPA's color for a band, as the Swift's WeatherValue.aqiColor.
fn aqi_color(us_aqi: i64) -> Color {
    match us_aqi {
        ..=50 => Color(0x34C759),
        ..=100 => Color(0xFFCC00),
        ..=150 => Color(0xFF9500),
        ..=200 => Color(0xFF3B30),
        ..=300 => Color(0xAF52DE),
        _ => Color(0xA2845E),
    }
}

// ---------------------------------------------------------------------------
// One day
// ---------------------------------------------------------------------------

/// A day's page, from WeatherDayDetailView.
fn day_page(b: &mut Builder, view: &View, index: usize) {
    let y = b.y();
    b.control(Id::Custom(ID_BACK), Rect::new(b.x(), y, ICON_BUTTON, ICON_BUTTON), Kind::IconButton { glyph: icons::BACK });
    let Some((forecast, day)) = view.forecast.and_then(|forecast| forecast.day(index).map(|day| (forecast, day))) else {
        b.text(Rect::new(b.x() + 40.0, y, b.width() - 40.0, ICON_BUTTON), "Forecast", Style::Subtitle, Ink::Primary, Align::Left);
        b.advance(ICON_BUTTON + 10.0);
        let card = b.card_begin(None);
        b.unavailable(icons::CLOUD, "Day unavailable", "The forecast no longer has this day.");
        b.card_end(card);
        return;
    };
    b.text(Rect::new(b.x() + 40.0, y, b.width() - 40.0, ICON_BUTTON), &clock::date_title(day.day.date), Style::Subtitle, Ink::Primary, Align::Left);
    b.advance(ICON_BUTTON + 10.0);

    let units = view.units();
    day_summary(b, view, &day);
    day_metrics(b, view, &day.day);
    extra_section(b, "Rain, snow & sunshine", daily_rows(&day.day, units, PRECIPITATION_ROWS), None);
    extra_section(b, "Air & comfort", daily_rows(&day.day, units, AIR_ROWS), None);
    hour_by_hour(b, view, forecast, &day, index);
    sun_moon_section(b, view, forecast, &day, index == 0);
    extra_section(b, "Atmosphere & growing conditions", daily_rows(&day.day, units, ATMOSPHERE_ROWS), None);
    b.centered_link(Id::Custom(ID_ATTRIBUTION), "Weather data by Open-Meteo.com");
}

/// The day's summary card.
fn day_summary(b: &mut Builder, view: &View, day: &DayDetails) {
    let units = view.units();
    let card = b.card_begin(Some(view.accent.primary));
    let y = b.y();
    let condition = day.day.weather_code.map(|code| Condition::for_code(code, true)).unwrap_or(Condition::Unknown);
    let text_x = b.inner_x() + 55.0 + 14.0;
    let text_w = b.inner_w() - 55.0 - 14.0;
    let name = view.snapshot.location.as_ref().map(|l| l.name.as_str()).unwrap_or("Weather");
    b.text(Rect::new(text_x, y, text_w, 16.0), name, Style::Caption, Ink::Secondary, Align::Left);
    b.text(Rect::new(text_x, y + 19.0, text_w, 20.0), format::description(day.day.weather_code), Style::BodyStrong, Ink::Primary, Align::Left);
    let high = format::temperature(day.day.high, units);
    let high_w = b.text_width(&high, Style::Display(30.0)) + 4.0;
    let line_y = y + 44.0;
    b.text(Rect::new(text_x, line_y, high_w, 39.0), &high, Style::Display(30.0), Ink::Primary, Align::Left);
    let low = format!("Low {}", format::temperature(day.day.low, units));
    b.text(Rect::new(text_x + high_w + 10.0, line_y + 19.0, (text_w - high_w - 10.0).max(20.0), 20.0), &low, Style::Body, Ink::Secondary, Align::Left);
    let mut block_h = 44.0 + 39.0;
    if view.snapshot.error.is_some() {
        b.text(Rect::new(text_x, y + block_h + 4.0, text_w, 16.0), "Saved forecast \u{00B7} may be out of date", Style::Caption, Ink::Caution, Align::Left);
        block_h += 20.0;
    }
    b.passive(Rect::new(b.inner_x(), y + (block_h - 55.0) / 2.0, 55.0, 55.0), Kind::Custom(Box::new(Mark { condition, size: 42.0 })));
    b.advance(block_h);
    b.card_end(card);
}

/// The day's headline figures, from dailyMetrics.
fn day_metrics(b: &mut Builder, view: &View, day: &DailyPoint) {
    let units = view.units();
    let tiles = vec![
        Tile {
            icon: TileIcon::Glyph(icons::SUN),
            label: "Feels like high / low".into(),
            value: format!("{} / {}", format::temperature(day.apparent_high, units), format::temperature(day.apparent_low, units)),
            tint: Ink::Accent,
        },
        Tile { icon: TileIcon::Glyph(icons::DROP), label: "Precipitation chance".into(), value: format::percent(day.precipitation_probability), tint: view.rain() },
        Tile { icon: TileIcon::Glyph(icons::RAINING_CLOUD), label: "Precipitation total".into(), value: format::precipitation(day.precipitation, units), tint: view.rain() },
        Tile { icon: TileIcon::Glyph(icons::SUN), label: "Maximum UV index".into(), value: format::number(day.uv_index_max), tint: view.sun() },
        Tile { icon: TileIcon::Glyph(icons::MIST), label: "Maximum wind".into(), value: format::wind(day.wind_speed_max, None, units), tint: Ink::Accent },
        Tile { icon: TileIcon::Glyph(icons::MIST), label: "Maximum gusts".into(), value: format::wind(day.wind_gusts_max, None, units), tint: Ink::Accent },
    ];
    let card = b.card_begin(None);
    b.section_label("Day at a glance");
    b.stat_tiles(with_values(tiles));
    b.card_end(card);
}

/// The tiles that have a value.
///
/// A tile printing a dash tells the reader nothing about the weather and
/// something untrue about the provider, and a grid of eight with two dashes
/// in it looks broken. The grid closes up around what is known, which is
/// what the catalog rows below already do.
fn with_values(tiles: Vec<Tile>) -> Vec<Tile> {
    tiles.into_iter().filter(|tile| tile.value != DASH).collect()
}

/// A row of a catalog section: its label, its metric, how it is shown.
type DailyRow = (&'static str, DailyMetric, Unit);
type HourlyRow = (&'static str, HourlyMetric, Unit);

const PRECIPITATION_ROWS: &[DailyRow] = &[
    ("Rain", DailyMetric::RainSum, Unit::Precipitation),
    ("Showers", DailyMetric::ShowersSum, Unit::Precipitation),
    ("Snowfall", DailyMetric::SnowfallSum, Unit::Snowfall),
    ("Precipitation hours", DailyMetric::PrecipitationHours, Unit::Hours),
    ("Sunshine", DailyMetric::SunshineDuration, Unit::Duration),
    ("Clear-sky UV maximum", DailyMetric::UVIndexClearSkyMax, Unit::Number),
];

const AIR_ROWS: &[DailyRow] = &[
    ("Average temperature", DailyMetric::TemperatureMean, Unit::Temperature),
    ("Average feels like", DailyMetric::ApparentTemperatureMean, Unit::Temperature),
    ("Average humidity", DailyMetric::HumidityMean, Unit::Percent),
    ("Minimum humidity", DailyMetric::HumidityMin, Unit::Percent),
    ("Maximum humidity", DailyMetric::HumidityMax, Unit::Percent),
    ("Average dew point", DailyMetric::DewPointMean, Unit::Temperature),
    ("Average cloud cover", DailyMetric::CloudCoverMean, Unit::Percent),
    ("Prevailing wind", DailyMetric::WindDirectionDominant, Unit::Direction),
    ("Average visibility", DailyMetric::VisibilityMean, Unit::Visibility),
    ("Minimum visibility", DailyMetric::VisibilityMin, Unit::Visibility),
    ("Average pressure", DailyMetric::PressureMean, Unit::Pressure),
];

const ATMOSPHERE_ROWS: &[DailyRow] = &[
    ("Surface pressure", DailyMetric::SurfacePressureMean, Unit::Pressure),
    ("Average wet-bulb temperature", DailyMetric::WetBulbTemperatureMean, Unit::Temperature),
    ("Maximum CAPE", DailyMetric::CapeMax, Unit::Cape),
    ("Maximum vapor pressure deficit", DailyMetric::VaporPressureDeficitMax, Unit::Kilopascals),
    ("Solar energy", DailyMetric::ShortwaveRadiationSum, Unit::SolarEnergy),
    ("Reference evapotranspiration", DailyMetric::ReferenceEvapotranspiration, Unit::Precipitation),
];

/// The hour's catalog, grouped as WeatherHourExtraSections groups it.
const HOUR_GROUPS: &[(&str, &[HourlyRow])] = &[
    (
        "Precipitation & wind",
        &[
            ("Rain", HourlyMetric::Rain, Unit::Precipitation),
            ("Showers", HourlyMetric::Showers, Unit::Precipitation),
            ("Snowfall", HourlyMetric::Snowfall, Unit::Snowfall),
            ("Snow depth", HourlyMetric::SnowDepth, Unit::SnowDepth),
            ("Wind gusts", HourlyMetric::WindGusts, Unit::Wind),
        ],
    ),
    (
        "Sky & atmosphere",
        &[
            ("Low clouds", HourlyMetric::CloudCoverLow, Unit::Percent),
            ("Middle clouds", HourlyMetric::CloudCoverMid, Unit::Percent),
            ("High clouds", HourlyMetric::CloudCoverHigh, Unit::Percent),
            ("Sea-level pressure", HourlyMetric::SeaLevelPressure, Unit::Pressure),
            ("Surface pressure", HourlyMetric::SurfacePressure, Unit::Pressure),
            ("Wet-bulb temperature", HourlyMetric::WetBulbTemperature, Unit::Temperature),
            ("Freezing level", HourlyMetric::FreezingLevelHeight, Unit::Height),
            ("CAPE", HourlyMetric::Cape, Unit::Cape),
            ("Vapor pressure deficit", HourlyMetric::VaporPressureDeficit, Unit::Kilopascals),
        ],
    ),
    (
        "Sunlight",
        &[
            ("Sunshine", HourlyMetric::SunshineDuration, Unit::Duration),
            ("Clear-sky UV", HourlyMetric::UVIndexClearSky, Unit::Number),
            ("Solar radiation", HourlyMetric::ShortwaveRadiation, Unit::Radiation),
            ("Direct radiation", HourlyMetric::DirectRadiation, Unit::Radiation),
            ("Diffuse radiation", HourlyMetric::DiffuseRadiation, Unit::Radiation),
            ("Direct normal irradiance", HourlyMetric::DirectNormalIrradiance, Unit::Radiation),
        ],
    ),
    (
        "Ground & growing conditions",
        &[
            ("Surface soil temperature", HourlyMetric::SoilTemperature0cm, Unit::Temperature),
            ("Soil temperature \u{00B7} 6 cm", HourlyMetric::SoilTemperature6cm, Unit::Temperature),
            ("Soil temperature \u{00B7} 18 cm", HourlyMetric::SoilTemperature18cm, Unit::Temperature),
            ("Soil temperature \u{00B7} 54 cm", HourlyMetric::SoilTemperature54cm, Unit::Temperature),
            ("Soil moisture \u{00B7} 0\u{2013}1 cm", HourlyMetric::SoilMoisture0To1cm, Unit::SoilMoisture),
            ("Soil moisture \u{00B7} 1\u{2013}3 cm", HourlyMetric::SoilMoisture1To3cm, Unit::SoilMoisture),
            ("Soil moisture \u{00B7} 3\u{2013}9 cm", HourlyMetric::SoilMoisture3To9cm, Unit::SoilMoisture),
            ("Soil moisture \u{00B7} 9\u{2013}27 cm", HourlyMetric::SoilMoisture9To27cm, Unit::SoilMoisture),
            ("Soil moisture \u{00B7} 27\u{2013}81 cm", HourlyMetric::SoilMoisture27To81cm, Unit::SoilMoisture),
            ("Evapotranspiration", HourlyMetric::Evapotranspiration, Unit::Precipitation),
            ("Reference evapotranspiration", HourlyMetric::ReferenceEvapotranspiration, Unit::Precipitation),
        ],
    ),
];

/// The rows of a daily section that have a value.
///
/// The Swift prints a dash for each metric the provider left out; here a
/// row that says nothing is left out, and a section with no rows is left
/// out with it. A column of dashes tells the reader nothing about the
/// weather and something untrue about the provider.
fn daily_rows(day: &DailyPoint, units: WeatherUnits, spec: &[DailyRow]) -> Vec<(String, String)> {
    spec.iter()
        .map(|(label, metric, unit)| (label.to_string(), unit.format(day.details.get(*metric), units)))
        .filter(|(_, value)| value != DASH)
        .collect()
}

fn hourly_rows(point: &HourlyPoint, units: WeatherUnits, spec: &[HourlyRow]) -> Vec<(String, String)> {
    spec.iter()
        .map(|(label, metric, unit)| (label.to_string(), unit.format(point.details.get(*metric), units)))
        .filter(|(_, value)| value != DASH)
        .collect()
}

/// A card of catalog rows, when there are any.
fn extra_section(b: &mut Builder, title: &str, rows: Vec<(String, String)>, tint: Option<Color>) {
    if rows.is_empty() {
        return;
    }
    let card = b.card_begin(tint);
    b.section_label(title);
    b.kv_rows(&rows);
    b.card_end(card);
}

/// The day's hours as a chart with one hour opened up, from hourlyForecast.
///
/// The Mac shows a strip of 72-point tiles the pointer slides along; here
/// the chart's own columns are the tiles, because the chart already has an
/// hour under every horizontal position and a second row of twenty-four
/// boxes would say the same thing twice. Like the Mac's, the chart is wider
/// than its plate and scrolls under it.
fn hour_by_hour(b: &mut Builder, view: &View, forecast: &DetailForecast, day: &DayDetails, index: usize) {
    let today = index == 0;
    let units = view.units();
    let card = b.card_begin(Some(view.accent.primary));
    b.section_label("Hour by hour");
    if day.hourly.is_empty() {
        b.caption("Hourly details are unavailable for this day.");
        b.card_end(card);
        return;
    }
    let points: Vec<&HourlyPoint> = day.hourly.iter().collect();
    let plate = b.plate(chart::HEIGHT);
    let content_w = chart::content_width(points.len(), chart::DAY_MIN_W);
    let geometry = chart::Geometry::new(&points, content_w, plate.h);
    // The Mac opens on the day's first hour; today that is midnight, long
    // gone, so today opens on the hour in progress.
    let default = if today {
        let now = clock::floor_hour(view.now(forecast));
        points.iter().position(|point| point.time == now).unwrap_or(0)
    } else {
        0
    };
    let selected = view.selected_hour.filter(|index| *index < points.len()).unwrap_or(default);
    // The zones are in the chart's own coordinates; the chrome hits them
    // through whatever the chart has been scrolled to.
    let zones = (0..points.len())
        .map(|index| (Rect::new(plate.x + index as f32 * geometry.step, plate.y, geometry.step, plate.h), hour_id(index)))
        .collect();
    // Opens with the chosen hour in the middle of the plate; after that the
    // panel keeps the chart wherever the user scrolls it.
    let offset = chart::offset_showing(selected, geometry.step, content_w, plate.w);
    b.push(Element {
        id: day_chart_id(index),
        rect: plate,
        kind: Kind::Scroll { content_w, offset, painter: Box::new(Chart { geometry, selected: Some(selected) }) },
        interactive: false,
        zones,
    });
    b.gap(10.0);

    let point = points[selected];
    let plate_index = b.elements.len();
    b.passive(Rect::new(b.inner_x(), b.y(), b.inner_w(), 0.0), Kind::Plate);
    let inset = 10.0;
    let x = b.inner_x() + inset;
    let w = b.inner_w() - 2.0 * inset;
    b.gap(inset);
    let y = b.y();
    let headline = clock::hour_headline(point.time);
    let headline_w = b.text_width(&headline, Style::BodyStrong) + 4.0;
    b.text(Rect::new(x, y, headline_w, 20.0), &headline, Style::BodyStrong, Ink::Primary, Align::Left);
    b.text(Rect::new(x + headline_w + 8.0, y, w - headline_w - 8.0, 20.0), format::description(point.weather_code), Style::Body, Ink::Secondary, Align::Right);
    b.gap(20.0 + 6.0);
    let rows = vec![
        ("Feels like".to_string(), format::temperature(point.apparent_temperature, units)),
        ("Precipitation".to_string(), format::precipitation(point.precipitation, units)),
        ("Wind".to_string(), format::wind(point.wind_speed, point.wind_direction, units)),
        ("Humidity".to_string(), format::percent(point.humidity)),
        ("Dew point".to_string(), format::temperature(point.dew_point, units)),
        ("Cloud cover".to_string(), format::percent(point.cloud_cover)),
        ("UV index".to_string(), format::number(point.uv_index)),
        ("Visibility".to_string(), format::visibility(point.visibility, units)),
    ];
    let rows: Vec<(String, String)> = rows.into_iter().filter(|(_, value)| value != DASH).collect();
    b.kv_rows_in(x, w, &rows);
    for (title, spec) in HOUR_GROUPS {
        let rows = hourly_rows(point, units, spec);
        if rows.is_empty() {
            continue;
        }
        b.gap(10.0);
        b.text(Rect::new(x, b.y(), w, 16.0), title, Style::CaptionStrong, Ink::Primary, Align::Left);
        b.gap(16.0 + 4.0);
        b.kv_rows_in(x, w, &rows);
    }
    b.gap(inset);
    let top = b.elements[plate_index].rect.y;
    b.elements[plate_index].rect.h = b.y() - top;
    b.card_end(card);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flyout::ui::{self, Palette};
    use crate::settings_ui::theme::{Theme, DEFAULT_ACCENT};
    use barometer_core::weather::detail::parse_detail;

    fn measure(text: &str, style: Style, width: f32) -> (f32, f32) {
        let w = text.chars().count() as f32 * 7.0 * style.size() / 14.0;
        if width > 0.0 && w > width {
            (width, (w / width).ceil() * style.line())
        } else {
            (w, style.line())
        }
    }

    /// Three days of hours and two daily summaries, one with the provider's
    /// moon phase and one without, at a New York offset.
    fn forecast() -> DetailForecast {
        let mut times = Vec::new();
        let mut temperatures = Vec::new();
        let mut chances = Vec::new();
        for hour in 0..72u32 {
            let date = ["2026-09-08", "2026-09-09", "2026-09-10"][(hour / 24) as usize];
            let clock = hour % 24;
            times.push(format!("\"{date}T{clock:02}:00\""));
            temperatures.push(format!("{}", 60 + (hour % 24)));
            chances.push(if (14..18).contains(&(hour % 24)) { "40" } else { "0" }.to_string());
        }
        let body = format!(
            r#"{{"timezone":"America/New_York","utc_offset_seconds":-14400,
              "hourly":{{"time":[{}],"temperature_2m":[{}],"precipitation_probability":[{}],
                "weather_code":[{}],"is_day":[{}],"relative_humidity_2m":[{}],"uv_index":[{}],
                "cloud_cover":[{}],"wind_gusts_10m":[{}],"rain":[{}]}},
              "daily":{{"time":["2026-09-08","2026-09-09"],"weather_code":[2,61],
                "temperature_2m_max":[84,79],"temperature_2m_min":[68,66],
                "apparent_temperature_max":[88,80],"apparent_temperature_min":[70,66],
                "sunrise":["2026-09-08T06:32","2026-09-09T06:33"],"sunset":["2026-09-08T19:12","2026-09-09T19:10"],
                "uv_index_max":[7.5,4.0],"precipitation_probability_max":[40,80],
                "wind_speed_10m_max":[12,18],"wind_gusts_10m_max":[22,31],
                "rain_sum":[0.0,0.42],"moon_phase":[0.75,null],"moonrise":["2026-09-08T02:10",null],"moonset":["2026-09-08T16:40","2026-09-09T17:20"]}}}}"#,
            times.join(","),
            temperatures.join(","),
            chances.join(","),
            (0..72).map(|_| "2").collect::<Vec<_>>().join(","),
            (0..72).map(|h| if (7..19).contains(&(h % 24)) { "1" } else { "0" }).collect::<Vec<_>>().join(","),
            (0..72).map(|_| "55").collect::<Vec<_>>().join(","),
            (0..72).map(|_| "3.2").collect::<Vec<_>>().join(","),
            (0..72).map(|_| "30").collect::<Vec<_>>().join(","),
            (0..72).map(|_| "17").collect::<Vec<_>>().join(","),
            (0..72).map(|_| "0.0").collect::<Vec<_>>().join(","),
        );
        parse_detail(&body).unwrap()
    }

    fn austin() -> Location {
        Location {
            id: "austin".into(),
            name: "Austin".into(),
            admin: Some("Texas".into()),
            country: "United States".into(),
            latitude: "30.27".into(),
            longitude: "-97.74".into(),
            time_zone: "America/Chicago".into(),
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            location: Some(austin()),
            locations: vec![austin()],
            units: WeatherUnits::IMPERIAL,
            current: Some(CurrentWeather {
                temperature: Some(91.4),
                apparent_temperature: Some(97.2),
                relative_humidity: Some(44.0),
                precipitation: Some(0.0),
                wind_speed: Some(8.9),
                wind_direction: Some(160.0),
                pressure_hectopascals: Some(1004.7),
                surface_pressure_hectopascals: Some(995.1),
                weather_code: Some(2),
                is_day: true,
            }),
            error: None,
            refresh_minutes: 15,
        }
    }

    /// 2026-09-08 14:30 in New York, which is 18:30 UTC.
    fn now() -> i64 {
        LocalTime::parse("2026-09-08T18:30").unwrap().wall_clock_seconds()
    }

    fn content(snapshot: Snapshot, forecast: Option<DetailForecast>) -> WeatherContent {
        let mut content = WeatherContent::new(Arc::new(Mutex::new(snapshot)));
        content.sync_snapshot();
        content.current_since = Some(now() - 5 * 60);
        content.forecast = forecast;
        content
    }

    fn build(content: &mut WeatherContent) -> Page {
        let palette = Palette::resolve(Theme::resolve(false, DEFAULT_ACCENT));
        let cx = Context {
            width: ui::PANEL_W,
            palette: &palette,
            accent: Accent::signature(ModuleId::Weather),
            measure: &measure,
            hover: None,
            now_unix: now(),
        };
        content.build(&cx)
    }

    fn texts(elements: &[Element]) -> Vec<String> {
        elements
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Text { text, .. } | Kind::Wrapped { text, .. } | Kind::Link { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_air_card_shows_the_index_its_band_and_the_pollutants_and_is_absent_without_an_answer() {
        let mut content = content(snapshot(), Some(forecast()));
        assert!(!texts(&build(&mut content).elements).iter().any(|t| t == "US AQI"));

        content.air = Some(AirQuality { us_aqi: Some(42), pm2_5: Some(8.4), pm10: Some(15.1), ozone: None });
        let elements = build(&mut content).elements;
        let words = texts(&elements);
        assert!(words.iter().any(|t| t == "42"));
        assert!(words.iter().any(|t| t == "US AQI"));
        assert!(words.iter().any(|t| t == "PM2.5") && words.iter().any(|t| t == "PM10"));
        assert!(!words.iter().any(|t| t == "Ozone"), "a pollutant not reported is not a tile");
        assert!(elements.iter().any(|e| matches!(&e.kind, Kind::Chip { text, .. } if text == "Good")));
        // The scale sits at the index's share of 300.
        assert!(elements.iter().any(|e| matches!(e.kind, Kind::Capsule { fraction, .. } if (fraction - 0.14).abs() < 1e-6)));
        // The card sits between the sun and the details, as the Swift's does.
        let air_y = find_text(&elements, "Air quality").rect.y;
        assert!(find_text(&elements, "Sun & moon").rect.y < air_y);
        assert!(air_y < find_text(&elements, "Details").rect.y);
    }

    /// A second saved place, so the location row has something to offer.
    fn london() -> Location {
        Location {
            id: "london".into(),
            name: "London".into(),
            admin: None,
            country: "United Kingdom".into(),
            latitude: "51.51".into(),
            longitude: "-0.13".into(),
            time_zone: "Europe/London".into(),
        }
    }

    #[test]
    fn the_location_row_offers_the_saved_places_and_asks_for_the_one_pressed() {
        let mut two = snapshot();
        two.locations = vec![austin(), london()];
        let mut content = content(two, Some(forecast()));
        let elements = build(&mut content).elements;
        let words = texts(&elements);
        assert!(words.iter().any(|word| word == "Location"));
        // Both places, the one being shown in the accent and the other not.
        let chip = |name: &str| {
            elements
                .iter()
                .find(|e| matches!(&e.kind, Kind::Chip { text, .. } if text == name))
                .unwrap_or_else(|| panic!("{name} is a chip"))
        };
        assert!(!chip("Austin").interactive, "the place already shown is not a control");
        assert!(chip("London").interactive);
        // Pressing the other one asks the app to change the setting; the
        // panel does not change it itself.
        let response = content.activate(chip("London").id);
        assert_eq!(response, Response::Request(Command::ShowLocation("london".to_string())));
        // One saved place is still a row, so the panel always says where
        // the forecast is for.
        let mut alone = snapshot();
        alone.locations = vec![austin()];
        let mut content = content_of(alone);
        assert!(texts(&build(&mut content).elements).iter().any(|word| word == "Location"));
        // None saved - a machine that located itself by address - still
        // says where the forecast is for, and that chip is not a control.
        let mut nowhere = snapshot();
        nowhere.locations.clear();
        let mut content = content_of(nowhere);
        let elements = build(&mut content).elements;
        assert!(texts(&elements).iter().any(|word| word == "Location"));
        let austin = elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Chip { text, .. } if text == "Austin"))
            .expect("the place it located itself as");
        assert!(!austin.interactive);
        // And with nowhere at all, no row.
        let mut unknown = snapshot();
        unknown.locations.clear();
        unknown.location = None;
        let mut content = content_of(unknown);
        assert!(!texts(&build(&mut content).elements).iter().any(|word| word == "Location"));
    }

    /// A content over one snapshot with the usual forecast.
    fn content_of(snapshot: Snapshot) -> WeatherContent {
        content(snapshot, Some(forecast()))
    }

    fn find_text<'e>(elements: &'e [Element], wanted: &str) -> &'e Element {
        elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { text, .. } if text == wanted))
            .unwrap_or_else(|| panic!("no text {wanted:?} among {:?}", texts(elements)))
    }

    #[test]
    fn the_overview_lays_its_sections_out_in_the_swifts_order() {
        let mut content = content(snapshot(), Some(forecast()));
        let page = build(&mut content);
        let words = texts(&page.elements);
        let position = |wanted: &str| words.iter().position(|w| w == wanted).unwrap_or_else(|| panic!("{wanted} missing"));
        assert!(position("Austin") < position("Next 48 hours"));
        assert!(position("Next 48 hours") < position("10-day forecast"));
        assert!(position("10-day forecast") < position("Sun & moon"));
        assert!(position("Sun & moon") < position("Details"));
        assert!(position("Details") < position("Weather data by Open-Meteo.com"));
        assert!(words.contains(&"Partly cloudy".to_string()));
        assert!(words.contains(&"Feels like 97\u{00B0}F".to_string()));
        assert!(page.height > 700.0, "the overview is the tallest panel and scrolls");
    }

    #[test]
    fn the_updated_line_sits_under_the_name_block_and_reads_the_locations_clock() {
        let mut content = content(snapshot(), Some(forecast()));
        let page = build(&mut content);
        let name = find_text(&page.elements, "Austin");
        let feels = find_text(&page.elements, "Feels like 97\u{00B0}F");
        let updated = page
            .elements
            .iter()
            .find(|e| matches!(&e.kind, Kind::Text { text, .. } if text.starts_with("Updated ")))
            .expect("the updated line");
        assert!(updated.rect.y >= feels.rect.bottom(), "the updated line overlaps the name block");
        assert!(name.rect.y < feels.rect.y);
        // Five minutes ago, at 2:25 PM in the forecast's own zone.
        if let Kind::Text { text, .. } = &updated.kind {
            assert_eq!(text, "Updated 2:25 PM \u{00B7} 5 min ago");
        }
    }

    #[test]
    fn the_hero_value_clears_the_words_beside_it() {
        let mut content = content(snapshot(), Some(forecast()));
        let page = build(&mut content);
        let name = find_text(&page.elements, "Austin");
        let value = page.elements.iter().find(|e| matches!(&e.kind, Kind::Custom(p) if format!("{p:?}").contains("SkyValue"))).expect("the temperature");
        assert!(name.rect.right() <= value.rect.x);
    }

    #[test]
    fn each_day_row_answers_to_its_day_and_opens_that_days_page() {
        let mut content = content(snapshot(), Some(forecast()));
        let page = build(&mut content);
        let rows: Vec<&Element> = page.elements.iter().filter(|e| matches!(e.kind, Kind::Row { .. })).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, day_id(0));
        assert_eq!(ui::hit(&page.elements, rows[1].rect.x + 5.0, rows[1].rect.center_y()), Some(day_id(1)));
        assert_eq!(content.activate(day_id(1)), Response::Relayout);
        let page = build(&mut content);
        let words = texts(&page.elements);
        assert!(words.contains(&"Wednesday, Sep 9".to_string()));
        assert!(words.contains(&"Day at a glance".to_string()));
        assert!(words.contains(&"Hour by hour".to_string()));
        assert!(ui::find(&page.elements, Id::Custom(ID_BACK)).is_some());
        // Backspace goes back.
        assert_eq!(content.key(VK_BACK), Response::Relayout);
        assert!(texts(&build(&mut content).elements).contains(&"10-day forecast".to_string()));
    }

    /// The sideways scroller with an id: its plate, its content width and
    /// its offset.
    fn scroller(elements: &[Element], id: Id) -> (Rect, f32, f32) {
        elements
            .iter()
            .find_map(|e| match &e.kind {
                Kind::Scroll { content_w, offset, .. } if e.id == id => Some((e.rect, *content_w, *offset)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no scroller {id:?}"))
    }

    #[test]
    fn the_next_48_hours_scroll_sideways_under_the_plate_and_say_so() {
        let mut content = content(snapshot(), Some(forecast()));
        let page = build(&mut content);
        let (plate, content_w, offset) = scroller(&page.elements, Id::Custom(ID_CHART));
        // Forty-eight hours at the Swift's 28 points, under a plate the
        // card's width, starting at the next hour.
        assert_eq!(content_w, 48.0 * chart::HOUR_W);
        assert_eq!(plate.w, ui::PANEL_W - 2.0 * ui::PANEL_PAD - 2.0 * ui::CARD_PAD);
        assert!(content_w > plate.w * 2.0, "the chart is well wider than what shows of it");
        assert_eq!(offset, 0.0);
        assert_eq!(plate.h, chart::HEIGHT);
        // And the chip that says it scrolls, on the section label's line.
        let chip = page.elements.iter().find(|e| matches!(&e.kind, Kind::Chip { text, .. } if text == "Scroll for more")).expect("the chip");
        let label = find_text(&page.elements, "Next 48 hours");
        assert!(chip.rect.y < label.rect.bottom() && chip.rect.bottom() > label.rect.y);
        assert!(chip.rect.x > label.rect.x);
        assert!(chip.rect.right() <= plate.right() + 1e-3);
    }

    #[test]
    fn todays_chart_opens_scrolled_to_the_hour_in_progress_and_its_columns_are_hit_where_they_show() {
        let mut content = content(snapshot(), Some(forecast()));
        content.activate(day_id(0));
        let page = build(&mut content);
        let (plate, content_w, offset) = scroller(&page.elements, day_chart_id(0));
        assert_eq!(content_w, 24.0 * chart::HOUR_W);
        // 14:30 in New York: the two o'clock column, brought to the middle.
        let expected = chart::offset_showing(14, chart::HOUR_W, content_w, plate.w);
        assert!(expected > 0.0);
        assert_eq!(offset, expected);
        let chart = page.elements.iter().find(|e| e.id == day_chart_id(0)).unwrap();
        let (zone, id) = chart.zones[14];
        assert_eq!(id, hour_id(14));
        // The zone is in the chart's coordinates; on the plate it shows
        // `offset` further left, and that is where a hit lands on it.
        assert_eq!(ui::hit(&page.elements, zone.x - offset + 1.0, zone.y + 1.0), Some(hour_id(14)));
        assert_ne!(ui::hit(&page.elements, zone.x + 1.0, zone.y + 1.0), Some(hour_id(14)));
        // Another day's chart is its own scroller and opens at its start.
        content.activate(day_id(1));
        let page = build(&mut content);
        assert_eq!(scroller(&page.elements, day_chart_id(1)).2, 0.0);
    }

    #[test]
    fn the_day_pages_chart_has_one_zone_per_hour_and_hovering_one_chooses_it() {
        let mut content = content(snapshot(), Some(forecast()));
        content.activate(day_id(1));
        let page = build(&mut content);
        let chart = page.elements.iter().find(|e| !e.zones.is_empty()).expect("the chart");
        assert_eq!(chart.zones.len(), 24);
        let (zone, id) = chart.zones[5];
        assert_eq!(id, hour_id(5));
        assert_eq!(ui::hit(&page.elements, zone.x + 1.0, zone.y + 1.0), Some(hour_id(5)));
        // The day opens on its first hour; hovering the sixth changes shape.
        assert!(texts(&page.elements).contains(&"12 AM".to_string()));
        assert!(content.hover(Some(hour_id(5))));
        assert!(texts(&build(&mut content).elements).contains(&"5 AM".to_string()));
        assert!(!content.hover(Some(hour_id(5))), "the same hour again is not a change");
    }

    #[test]
    fn today_opens_on_the_hour_in_progress() {
        let mut content = content(snapshot(), Some(forecast()));
        content.activate(day_id(0));
        let page = build(&mut content);
        // 14:30 in New York: the two o'clock hour.
        assert!(texts(&page.elements).contains(&"2 PM".to_string()));
    }

    #[test]
    fn the_moon_says_when_it_is_estimated_and_when_the_provider_sent_it() {
        let mut content = content(snapshot(), Some(forecast()));
        let words = texts(&build(&mut content).elements);
        assert!(words.contains(&"Daily lunar phase \u{00B7} Open-Meteo".to_string()));
        assert!(words.contains(&"Last Quarter".to_string()));
        assert!(words.contains(&"About 50% illuminated".to_string()));
        content.activate(day_id(1));
        let words = texts(&build(&mut content).elements);
        assert!(words.contains(&"Estimated at local noon".to_string()));
        assert!(words.contains(&"No moonrise today".to_string()));
    }

    #[test]
    fn without_a_location_the_hero_says_where_to_add_one() {
        let mut content = content(Snapshot::default(), None);
        let words = texts(&build(&mut content).elements);
        assert!(words.contains(&"Add a location in Weather settings.".to_string()));
        assert!(words.contains(&"Weather unavailable".to_string()));
        // And the footer has nothing to refresh.
        assert!(!content.actions()[0].enabled);
    }

    #[test]
    fn a_failed_refresh_is_said_and_the_forecast_still_shows() {
        let mut snapshot = snapshot();
        snapshot.error = Some("timed out".to_string());
        let mut content = content(snapshot, Some(forecast()));
        let words = texts(&build(&mut content).elements);
        assert!(words.iter().any(|w| w.starts_with("Refresh failed; showing saved weather. timed out")));
        assert!(words.contains(&"10-day forecast".to_string()));
    }

    #[test]
    fn a_range_bar_never_leaves_its_track_and_never_vanishes() {
        let track = Rect::new(100.0, 0.0, 120.0, 6.0);
        let bar = range_bar(track, Some(68.0), Some(84.0), 66.0, 84.0);
        assert!(bar.x >= track.x && bar.right() <= track.right() + 1e-3);
        assert!(bar.w > 6.0);
        // A day with no high starts at its low and reaches the end.
        let open = range_bar(track, Some(70.0), None, 66.0, 84.0);
        assert!((open.right() - track.right()).abs() < 1e-3);
        // A flat day is still six wide, and a day off the top is pulled back on.
        assert_eq!(range_bar(track, Some(70.0), Some(70.0), 66.0, 84.0).w, 6.0);
        let past = range_bar(track, Some(84.0), Some(90.0), 66.0, 84.0);
        assert!((past.right() - track.right()).abs() < 1e-3);
    }

    #[test]
    fn catalog_rows_that_say_nothing_are_left_out() {
        let forecast = forecast();
        let rows = daily_rows(&forecast.daily[1], WeatherUnits::IMPERIAL, PRECIPITATION_ROWS);
        assert_eq!(rows, vec![("Rain".to_string(), "0.42 in".to_string())]);
        assert!(daily_rows(&forecast.daily[0], WeatherUnits::IMPERIAL, ATMOSPHERE_ROWS).is_empty());
        let hour = hourly_rows(&forecast.hourly[0], WeatherUnits::IMPERIAL, HOUR_GROUPS[0].1);
        assert_eq!(hour, vec![("Rain".to_string(), "0.00 in".to_string()), ("Wind gusts".to_string(), "17 mph".to_string())]);
    }

    #[test]
    fn the_forecast_is_stale_for_another_place_and_after_the_refresh_interval() {
        let mut content = content(snapshot(), Some(forecast()));
        content.forecast_for = Some("austin".into());
        content.fetched_at = Some(Instant::now());
        assert!(!content.stale());
        content.forecast_for = Some("elsewhere".into());
        assert!(content.stale());
        content.forecast_for = Some("austin".into());
        content.fetched_at = Some(Instant::now() - Duration::from_secs(16 * 60));
        assert!(content.stale());
    }

    #[test]
    fn a_ticks_answer_is_whether_the_page_changed_shape() {
        let mut content = content(snapshot(), Some(forecast()));
        content.built_minute = Some(clock::unix_now() / 60);
        assert!(!content.tick());
        content.built_minute = Some(0);
        assert!(content.tick(), "a new minute moves the updated line");
    }

    #[test]
    fn a_slow_answer_to_an_old_request_is_thrown_away() {
        let mut content = content(snapshot(), None);
        content.generation = 3;
        *content.results.lock().unwrap() = Some((2, "austin".into(), Ok(forecast()), None));
        assert!(!content.take_result());
        assert!(content.forecast.is_none());
        *content.results.lock().unwrap() =
            Some((3, "austin".into(), Err("no route".into()), None));
        assert!(content.take_result());
        assert_eq!(content.fetch_error.as_deref(), Some("no route"));
        assert!(!content.fetching);
    }

    #[test]
    fn a_forecast_that_arrives_for_the_city_you_just_left_is_never_shown_under_the_new_one() {
        // A fetch in flight cannot be called back, and picking another city
        // while one is running does not start a second - so the answer that
        // came back was accepted and then labeled from whatever the panel had
        // moved on to. Ten days of Austin weather under London's name.
        let mut content = content(snapshot(), None);
        content.generation = 1;
        content.fetching = true;
        *content.results.lock().unwrap() =
            Some((1, "london".into(), Ok(forecast()), None));
        assert!(content.take_result());
        assert!(content.forecast.is_none(), "another city's forecast was kept");
        assert_eq!(content.forecast_for, None);
        // And the panel does not sit there empty: the fetch the change could
        // not start, because one was already running, is started now.
        assert!(content.fetching, "no fetch for the city actually on screen");
    }

    #[test]
    fn ids_round_trip_through_their_numbers() {
        assert_eq!(day_index(day_id(4)), Some(4));
        assert_eq!(hour_index(hour_id(23)), Some(23));
        assert_eq!(day_index(hour_id(0)), None);
        assert_eq!(hour_index(day_id(0)), None);
        assert_eq!(day_index(Id::Custom(ID_BACK)), None);
        // A day's chart never takes a day's or an hour's number, however
        // many days the provider sends.
        for index in [0, 9, 15, 500] {
            assert_eq!(day_index(day_chart_id(index)), None);
            assert_eq!(hour_index(day_chart_id(index)), None);
            assert_ne!(day_chart_id(index), Id::Custom(ID_CHART));
        }
    }

    /// Paints the overview and a day page for a person to look at; see
    /// `flyout::render`.
    #[test]
    #[ignore]
    fn render_the_weather_pages_to_bitmaps() {
        for light in [false, true] {
            for (page, shown) in [("weather-overview", Shown::Overview), ("weather-day", Shown::Day(1))] {
                let mut content = content(snapshot(), Some(forecast()));
                content.shown = shown;
                content.air = None;
                crate::flyout::render::to_bitmap(&mut content, page, light, now());
            }
        }
    }
}
