// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Weather, on the strip.
//
// The one module that goes out to the internet, which is the whole reason it
// has a thread. A fetch is seconds on a bad connection and the Module contract
// says sample() must not block; so the worker owns the request and sample()
// only picks up whatever last arrived.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::module::{Module, ModuleId, Readout};
use crate::weather::badge::Condition;
use crate::weather::client::{self, CurrentWeather, WeatherError};
use crate::weather::models::{Location, WeatherSettings};

/// How long the worker waits before giving up on a location it cannot reach.
///
/// Not the refresh interval: this is the retry after a failure. A laptop that
/// wakes on a captive-portal network fails once and should try again in a
/// minute, not sit on a stale reading until the next quarter hour.
const RETRY: Duration = Duration::from_secs(60);

/// How many times the wait between address lookups doubles before it stops.
///
/// Five, so the wait runs 1, 2, 4, 8, 16 and then 32 minutes for as long as
/// the lookup keeps failing. Without this the retry stayed at a flat minute
/// forever, and the first-run state - no saved location, address lookup on -
/// is exactly the state a laptop is in when it boots offline or sits behind
/// a captive portal: about 1,440 requests a day against a service whose free
/// tier allows 1,000, which rate-limits the app out of the very lookup it is
/// waiting on.
const LOCATE_DOUBLINGS: u32 = 5;

#[derive(Default)]
struct Shared {
    current: Option<CurrentWeather>,
    error: Option<String>,
    /// Where the worker decided it is, when nothing was saved. Published so
    /// the settings pane can offer it as the first entry in the location list.
    located: Option<Location>,
}

pub struct WeatherModule {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<(Mutex<bool>, Condvar)>,
    running: Arc<AtomicBool>,
    /// What the worker is to fetch, shared with it rather than handed over
    /// at spawn: the place, the units and the interval are all settings, and
    /// settings change while the worker is running.
    wanted: Arc<Mutex<WeatherSettings>>,
    settings: WeatherSettings,
    readout: Readout,
}

impl WeatherModule {
    /// Starts fetching for the primary location in `settings`.
    ///
    /// With no location saved the module still exists and reads unavailable:
    /// the strip keeps its space and the settings pane has a state to explain,
    /// which is better than a module that silently is not there.
    pub fn new(settings: WeatherSettings) -> WeatherModule {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let running = Arc::new(AtomicBool::new(false));

        let wanted = Arc::new(Mutex::new(settings.clone()));
        // No saved location means guess from the IP address, on the worker
        // rather than here: locating is a network call, and doing it on the
        // startup path delays the taskbar readout appearing at all.
        if settings.primary_location().is_some() || settings.uses_current_location {
            start(&running, &shared, &stop, &wanted);
        }

        WeatherModule { shared, stop, running, wanted, settings, readout: Readout::unavailable() }
    }

    /// The location being watched, for the settings pane.
    ///
    /// The saved one if there is one, otherwise whatever the worker guessed.
    pub fn location(&self) -> Option<Location> {
        self.settings
            .primary_location()
            .cloned()
            .or_else(|| self.shared.lock().ok().and_then(|shared| shared.located.clone()))
    }

    /// The full observation, for the detail panel.
    pub fn current(&self) -> Option<CurrentWeather> {
        self.shared.lock().ok().and_then(|shared| shared.current.clone())
    }

    /// Why there is no reading, if there is none.
    pub fn error(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|shared| shared.error.clone())
    }

    /// Everything the detail panel shows, taken in one go.
    pub fn observe(&self) -> Observation {
        Observation {
            location: self.location(),
            units: self.settings.units,
            current: self.current(),
            error: self.error(),
            refresh_minutes: self.settings.clamped_refresh_minutes(),
        }
    }
}

/// What the weather module knows, for a panel to show.
///
/// The strip's readout is one temperature; the panel wants the place, the
/// observation behind the number, the units it was asked for, and - when
/// there is no number - why. Taken as one value so the panel never sees a
/// location from one refresh and a reading from the next.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub location: Option<Location>,
    pub units: crate::weather::models::WeatherUnits,
    pub current: Option<CurrentWeather>,
    pub error: Option<String>,
    /// How often the strip refreshes, which is how long a forecast is
    /// trusted for.
    pub refresh_minutes: u32,
}

/// Starts the worker, if one is not already running.
fn start(
    running: &Arc<AtomicBool>,
    shared: &Arc<Mutex<Shared>>,
    stop: &Arc<(Mutex<bool>, Condvar)>,
    wanted: &Arc<Mutex<WeatherSettings>>,
) {
    if running.swap(true, Ordering::AcqRel) {
        return;
    }
    thread::spawn({
        let shared = Arc::clone(shared);
        let stop = Arc::clone(stop);
        let wanted = Arc::clone(wanted);
        move || worker(shared, stop, wanted)
    });
}

/// Fetch, publish, wait. Waits on a condition variable rather than sleeping so
/// that exiting is immediate: a plain sleep would hold the process open for up
/// to the refresh interval, which is a quarter of an hour of a window that is
/// already gone.
///
/// The settings are re-read at the top of every pass rather than captured at
/// spawn, so changing the place or the units in the settings window takes
/// effect on the next pass - and `configure` wakes the worker, which makes
/// "next pass" mean "now".
fn worker(
    shared: Arc<Mutex<Shared>>,
    stop: Arc<(Mutex<bool>, Condvar)>,
    wanted: Arc<Mutex<WeatherSettings>>,
) {
    let (lock, signal) = &*stop;
    // Where the address said we are. Resolved once and kept: an IP lookup
    // that keeps being repeated is rude to a free service and tells us
    // nothing new; a laptop that moves far enough to matter is a restart
    // away from being right.
    let mut by_address: Option<Location> = None;
    // How many times the address lookup has failed in a row, which is what
    // the wait below is scaled by.
    let mut refusals: u32 = 0;
    loop {
        let settings = wanted.lock().map(|settings| settings.clone()).unwrap_or_default();
        let units = settings.units;
        let interval = Duration::from_secs(u64::from(settings.clamped_refresh_minutes()) * 60);

        let saved = settings.primary_location().cloned();
        if saved.is_none() && settings.uses_current_location && by_address.is_none() {
            match client::locate_by_ip() {
                Ok(found) => {
                    if let Ok(mut shared) = shared.lock() {
                        shared.located = Some(found.clone());
                    }
                    by_address = Some(found);
                    refusals = 0;
                }
                Err(why) => {
                    if let Ok(mut shared) = shared.lock() {
                        shared.error = Some(why.to_string());
                    }
                    refusals = refusals.saturating_add(1);
                }
            }
        }
        let here = match saved {
            Some(place) => Some(place),
            // The address is only a fallback, and only when it is allowed:
            // somebody who turned that off and deleted their locations has
            // asked for no weather, not for a guess.
            None if settings.uses_current_location => by_address.clone(),
            None => None,
        };

        let Some(here) = here else {
            // Nowhere to ask about yet. Wait rather than spinning on a
            // network that is not there; a location added in the settings
            // wakes this immediately either way.
            let Ok(stopped) = lock.lock() else { return };
            if *stopped {
                return;
            }
            let woken = if settings.uses_current_location {
                // Longer after each refusal - see LOCATE_DOUBLINGS.
                let wait = RETRY * 2u32.pow(refusals.min(LOCATE_DOUBLINGS));
                signal.wait_timeout(stopped, wait).map(|(guard, _)| guard).map_err(|_| ())
            } else {
                // No saved location and no address lookup is not a failure
                // to keep retrying: it is the user saying they want no
                // weather. There is nothing a timer could discover, so this
                // waits for somebody to add a location.
                signal.wait(stopped).map_err(|_| ())
            };
            let Ok(guard) = woken else { return };
            if *guard {
                return;
            }
            continue;
        };

        let wait = match client::fetch(&here, units) {
            Ok(current) => {
                if let Ok(mut shared) = shared.lock() {
                    shared.current = Some(current);
                    shared.error = None;
                }
                interval
            }
            Err(why) => {
                if let Ok(mut shared) = shared.lock() {
                    // The last good reading is kept. Weather, unlike a
                    // temperature sensor, is still roughly true a few minutes
                    // later, and blanking it every time a laptop changes
                    // network would leave the strip mostly empty.
                    shared.error = Some(match why {
                        WeatherError::NoLocation => "no location set".to_string(),
                        other => other.to_string(),
                    });
                }
                RETRY
            }
        };

        // Settings that changed while this pass was in flight. `configure`
        // signals the condition variable, but a signal sent while the worker
        // is inside a fetch - which is most of a pass, and seconds of it on a
        // bad connection - is nobody's to receive and is simply lost. Then
        // the worker sleeps out the whole interval holding the reading it was
        // told to replace: a quarter of an hour of showing Fahrenheit to
        // somebody who has just asked for Celsius. Comparing is what makes
        // the change take, whenever it landed.
        if wanted.lock().map(|latest| *latest != settings).unwrap_or(false) {
            continue;
        }

        let Ok(mut stopped) = lock.lock() else { return };
        if *stopped {
            return;
        }
        let Ok((guard, _)) = signal.wait_timeout(stopped, wait) else { return };
        stopped = guard;
        if *stopped {
            return;
        }
    }
}

/// A bearing as a compass point.
///
/// Eight points rather than sixteen: "north-northeast" is more precision than
/// anybody reads off a tooltip, and it is wider than the tooltip wants to be.
fn compass(bearing: f64) -> &'static str {
    const POINTS: [&str; 8] =
        ["north", "northeast", "east", "southeast", "south", "southwest", "west", "northwest"];
    // Wind direction is the direction it blows *from*, which is what the
    // provider reports and what the sentence above says.
    let index = (((bearing % 360.0 + 360.0) % 360.0) / 45.0).round() as usize % 8;
    POINTS[index]
}

impl Drop for WeatherModule {
    fn drop(&mut self) {
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        let (lock, signal) = &*self.stop;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
        }
        signal.notify_all();
    }
}

impl Module for WeatherModule {
    fn weather(&self) -> Option<Observation> {
        Some(self.observe())
    }

    /// Takes up a change made in the settings window: a different place, a
    /// different unit, a different interval.
    ///
    /// The reading is dropped when the place or the units change, because a
    /// temperature for somewhere else - or in the other scale - is not a
    /// stale reading of the right thing, it is the wrong thing. It is kept
    /// across an interval change, which does not alter what was measured.
    fn configure(&mut self, settings: &crate::store::Settings) {
        let weather = &settings.weather;
        if *weather == self.settings {
            return;
        }
        let elsewhere = weather.primary_location().map(|place| &place.id)
            != self.settings.primary_location().map(|place| &place.id);
        let rescaled = weather.units != self.settings.units;
        self.settings = weather.clone();
        if let Ok(mut wanted) = self.wanted.lock() {
            *wanted = weather.clone();
        }
        if elsewhere || rescaled {
            if let Ok(mut shared) = self.shared.lock() {
                shared.current = None;
                shared.error = None;
            }
            self.readout = Readout::unavailable();
        }
        // A module that had nowhere to ask about has a worker only once it
        // does; one that already has a worker is woken instead.
        if self.settings.primary_location().is_some() || self.settings.uses_current_location {
            start(&self.running, &self.shared, &self.stop, &self.wanted);
        }
        self.refresh();
    }

    /// Wakes the worker out of its wait, which sends it round its loop and
    /// back to the provider at once. The panel's Refresh re-fetches its own
    /// forecast; this is what makes the strip's reading follow.
    ///
    /// The lock is held while signaling so the wake cannot fall between the
    /// worker checking its flag and starting to wait, where it would be
    /// lost; the worker only ever holds that lock for the length of a check.
    fn refresh(&mut self) {
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        let (lock, signal) = &*self.stop;
        let _held = lock.lock();
        signal.notify_all();
    }

    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        _temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        match metric {
            crate::stack::StackMetric::WeatherTemperature if !self.readout.unavailable => {
                Some(self.readout.primary.clone())
            }
            _ => None,
        }
    }

    fn id(&self) -> ModuleId {
        ModuleId::Weather
    }

    fn sample(&mut self) {
        let Ok(shared) = self.shared.lock() else {
            self.readout = Readout::unavailable();
            return;
        };
        let Some(current) = shared.current.as_ref() else {
            self.readout = Readout::unavailable();
            return;
        };
        // The mark is drawn whether or not a temperature came back, and the
        // condition is Unknown when no code did: something is better than a
        // blank column, and Unknown is the honest something.
        let condition = Condition::for_code(current.weather_code.unwrap_or(255), current.is_day);
        self.readout = match current.temperature {
            Some(degrees) => {
                Readout::one(format!(
                    "{}{}",
                    crate::format::whole(degrees),
                    self.settings.units.temperature.symbol()
                ))
                    .reserving(format!("-99{}", self.settings.units.temperature.symbol()))
                    .with_badge(condition)
            }
            None => Readout::unavailable(),
        };
    }

    fn detail(&self) -> Vec<String> {
        let mut lines = vec![match self.location() {
            Some(place) => place.display_name(),
            None => self.id().title().to_string(),
        }];

        let Some(now) = self.current() else {
            // The error is worth showing rather than swallowing: "no location
            // set" and "could not reach the service" want different things
            // from the user, and only one of them is worth waiting out.
            lines.push(self.error().unwrap_or_else(|| "Not available".to_string()));
            return lines;
        };

        let units = self.settings.units;
        if let Some(temperature) = now.temperature {
            lines.push(format!("{temperature:.0}{}", units.temperature.symbol()));
        }
        if let Some(feels) = now.apparent_temperature {
            lines.push(format!("Feels like {feels:.0}{}", units.temperature.symbol()));
        }
        if let Some(humidity) = now.relative_humidity {
            lines.push(format!("Humidity {humidity:.0}%"));
        }
        if let Some(wind) = now.wind_speed {
            match now.wind_direction {
                Some(bearing) => lines.push(format!(
                    "Wind {wind:.0} {} from the {}",
                    units.wind_speed.symbol(),
                    compass(bearing)
                )),
                None => lines.push(format!("Wind {wind:.0} {}", units.wind_speed.symbol())),
            }
        }
        // The reading the program is named after, so it is never the one left
        // out for space.
        if let Some(hectopascals) = now.pressure_hectopascals {
            lines.push(format!(
                "Pressure {:.2} {}",
                units.pressure.from_hectopascals(hectopascals),
                units.pressure.symbol()
            ));
        }
        lines
    }

    fn readout(&self) -> Readout {
        self.readout.clone()
    }

    fn located(&self) -> Option<Location> {
        self.shared.lock().ok().and_then(|shared| shared.located.clone())
    }

    fn weather_error(&self) -> Option<String> {
        WeatherModule::error(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Settings;
    use crate::weather::models::{TemperatureUnit, WeatherUnits};

    /// Settings with nowhere to ask about, so no worker starts and the test
    /// touches no network: what is under test is whether a change in the
    /// settings window reaches the module at all, which it did not before.
    fn quiet(units: WeatherUnits, minutes: u32) -> Settings {
        Settings {
            weather: WeatherSettings {
                locations: Vec::new(),
                primary_location_id: None,
                uses_current_location: false,
                units,
                refresh_interval_minutes: minutes,
                ..WeatherSettings::default()
            },
            ..Settings::default()
        }
    }

    #[test]
    fn a_unit_chosen_in_the_settings_reaches_the_module_and_the_reading_it_scaled_is_dropped() {
        let mut module = WeatherModule::new(quiet(WeatherUnits::IMPERIAL, 15).weather);
        assert_eq!(module.observe().units.temperature, TemperatureUnit::Fahrenheit);

        // A reading arrives in the units that were asked for.
        if let Ok(mut shared) = module.shared.lock() {
            shared.current = Some(CurrentWeather { temperature: Some(74.0), ..CurrentWeather::default() });
        }
        module.sample();
        assert!(!module.readout().unavailable);

        // The scale changes: the number that was measured in the old one is
        // not a stale reading of the right thing, it is the wrong thing.
        let metric = WeatherUnits { temperature: TemperatureUnit::Celsius, ..WeatherUnits::IMPERIAL };
        module.configure(&quiet(metric, 15));
        assert_eq!(module.observe().units.temperature, TemperatureUnit::Celsius);
        assert_eq!(module.observe().current, None);
        assert!(module.readout().unavailable);
    }

    #[test]
    fn an_interval_chosen_in_the_settings_reaches_the_module_and_keeps_the_reading() {
        let mut module = WeatherModule::new(quiet(WeatherUnits::IMPERIAL, 15).weather);
        assert_eq!(module.observe().refresh_minutes, 15);
        if let Ok(mut shared) = module.shared.lock() {
            shared.current = Some(CurrentWeather { temperature: Some(74.0), ..CurrentWeather::default() });
        }

        // How often it is asked does not change what was measured.
        module.configure(&quiet(WeatherUnits::IMPERIAL, 45));
        assert_eq!(module.observe().refresh_minutes, 45);
        assert!(module.observe().current.is_some());

        // And the interval the settings cannot express is still clamped.
        module.configure(&quiet(WeatherUnits::IMPERIAL, 600));
        assert_eq!(module.observe().refresh_minutes, 60);
    }
}
