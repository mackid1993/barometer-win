// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Temperatures, on the strip.
//
// Every other module is a handful of syscalls and reads inline on the sampling
// thread. This one is not: a helper read walks every device LibreHardwareMonitor
// knows about and takes hundreds of milliseconds, which on the sampling thread
// would be hundreds of milliseconds the readout is not repainting and the
// message queue is not being pumped. So the provider lives on a thread of its
// own and publishes snapshots; `sample` only picks up whatever is there.
//
// That thread also owns the helper's whole life, rather than being handed a
// running one. The source can arrive, move or be deleted while Barometer is
// running - installing LibreHardwareMonitor from the settings window is the
// ordinary way it happens - and a worker given one provider at startup can
// only ever report the machine as it stood at startup. Temperatures then
// appear on the next launch, and the user is left to work out that relaunching
// was the missing step.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::module::{Module, ModuleId, Readout};
use crate::sensors::health::{self, Health};
use crate::sensors::{hottest_cpu, library, Sensor, SensorError, SensorKind, SensorProvider};
use crate::weather::models::TemperatureUnit;

/// How often the helper is asked, before the settings say otherwise.
///
/// Slower than the strip's own tick on purpose. A die temperature moves over
/// seconds, not frames, and the read is the most expensive thing the program
/// does; polling it at the strip's rate would spend most of Barometer's total
/// CPU on a number that had not changed. The Sensors pane can move it, within
/// `store::MIN_POLL_SECONDS`..`MAX_POLL_SECONDS`.
const POLL: Duration = Duration::from_secs(2);

/// How long to wait before starting the helper again after it would not start.
///
/// Doubling from one poll to a minute. Without a backoff, a machine with no
/// library installed - which is every machine before somebody presses Install -
/// would spawn a .NET process every two seconds forever, and the program whose
/// entire pitch is that it is small would be the busiest thing on the taskbar.
///
/// The backoff is reset the instant the library path changes, so waiting it out
/// is never what stands between pressing Install and seeing a temperature.
const FIRST_RETRY: Duration = POLL;
const LONGEST_RETRY: Duration = Duration::from_secs(60);

/// How often the helper is asked while nothing can show what it says.
///
/// Not stopped, and that is the trade. One read walks every device on the
/// machine, so asking twice a second for a temperature nobody has put on
/// the strip is the plain waste; but a helper that has stopped reading is
/// a pin picker with nothing in it and a Sensors pane that cannot say
/// whether the source is alive. A minute keeps both honest, keeps the
/// process warm, and costs a walk an hour instead of eighteen hundred.
///
/// Nothing waits out a minute to see a reading: `SensorsModule::shown`
/// wakes the worker the moment something that can show one appears.
const IDLE_POLL: Duration = Duration::from_secs(60);

/// What, if anything, could show a reading from the sensor source.
///
/// Assembled by the strip's tick, which is the only place that knows all
/// three, and kept as a value so the decision it feeds is a function of
/// its inputs and can be tested as one.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SensorDemand {
    /// A Sensors column on the strip, or a stack carrying a reading only
    /// the source has - a sensor, or the graphics power or temperature,
    /// which Windows' engine counters do not carry.
    pub on_the_strip: bool,
    /// A panel that draws sensor readings is open.
    pub panel_open: bool,
    /// The settings window is on screen, where the pin picker lists every
    /// sensor the machine reports.
    pub settings_visible: bool,
}

impl SensorDemand {
    /// Whether anything at all is waiting on a reading.
    pub fn any(&self) -> bool {
        self.on_the_strip || self.panel_open || self.settings_visible
    }

    /// How long the worker should wait before asking again.
    pub fn interval(&self, poll_seconds: u32) -> Duration {
        interval_for(self.any(), poll_seconds)
    }
}

/// How long to wait before asking again, given whether anything is waiting on
/// the answer.
///
/// The user's interval while somebody is looking, the idle one while nobody
/// is. `poll_seconds` is clamped the way the settings pane clamps it, so a
/// stored value from a future build cannot make this a busy loop or an hour.
///
/// Taken as one bool because that is all the worker is told: which of the
/// three questions in `SensorDemand` said yes is the strip's business and
/// changes nothing over here.
pub fn interval_for(waiting: bool, poll_seconds: u32) -> Duration {
    if !waiting {
        return IDLE_POLL;
    }
    let seconds =
        poll_seconds.clamp(crate::store::MIN_POLL_SECONDS, crate::store::MAX_POLL_SECONDS);
    Duration::from_secs(u64::from(seconds.max(1)))
}

/// Opens the helper against a library directory, or says why it could not.
///
/// A closure rather than a call to `HelperProvider::spawn` so this file does
/// not have to spawn a process to be tested: the supervision below is the part
/// with the states in it, and it is worth being able to drive those states from
/// a test that fails in milliseconds.
pub type Open =
    Box<dyn FnMut(Option<&Path>) -> Result<Box<dyn SensorProvider>, SensorError> + Send>;

/// Where the library is, asked again on every pass.
pub type Locate = Box<dyn Fn() -> Option<PathBuf> + Send>;

/// What the worker publishes and the strip reads.
#[derive(Default)]
struct Shared {
    sensors: Vec<Sensor>,
    error: Option<SensorError>,
}

pub struct SensorsModule {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    readout: Readout,
    /// What temperatures are shown in.
    ///
    /// Deliberately not the weather's setting. Wanting the forecast in
    /// Fahrenheit and a die temperature in Celsius is an ordinary combination,
    /// because the two are read against different reference points and nobody
    /// has an instinct for what 85 degrees means for a processor in the units
    /// they use for the weather.
    unit: TemperatureUnit,
    /// The reading the user pinned to the strip, by the source's identifier.
    /// None follows the hottest processor sensor, which is the default and
    /// what the strip showed before there was a picker.
    pinned: Option<String>,
    /// How often the worker asks the helper, in seconds. Shared with the
    /// worker rather than passed at spawn, because it is a setting and
    /// settings change while the worker is running.
    poll: Arc<AtomicU32>,
    /// Whether anything can show a reading right now. See `SensorDemand`.
    demand: Arc<AtomicBool>,
    /// The supervising thread, kept only so a test can wait for it.
    ///
    /// Never joined in `Drop` - see the note there - but a test that does not
    /// wait leaves a worker running into the next one, where it closes a
    /// library that test is still using. That is a test isolation problem
    /// rather than a real one, and this is what fixes it. Outside the tests
    /// nothing reads it, and the compiler is told so.
    #[cfg_attr(not(test), allow(dead_code))]
    worker: Option<thread::JoinHandle<()>>,
}

impl SensorsModule {
    /// Starts a worker that keeps a helper running against whatever library is
    /// installed, for as long as the module lives.
    ///
    /// `open` and `locate` are both called from the worker thread and nowhere
    /// else, which is what lets the provider stay a plain `&mut self` interface
    /// with no locking of its own.
    pub fn supervised(open: Open, locate: Locate) -> SensorsModule {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));

        let poll = Arc::new(AtomicU32::new(POLL.as_secs() as u32));
        // True until the strip says otherwise, so a build or a test that
        // never calls `shown` behaves exactly as it did before.
        let demand = Arc::new(AtomicBool::new(true));
        let worker = thread::spawn({
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            let poll = Arc::clone(&poll);
            let demand = Arc::clone(&demand);
            move || supervise(open, locate, &shared, &stop, &poll, &demand)
        });

        SensorsModule {
            shared,
            stop,
            readout: Readout::unavailable(),
            unit: TemperatureUnit::Celsius,
            pinned: None,
            poll,
            demand,
            worker: Some(worker),
        }
    }

    /// Stops the worker and waits for the helper to be gone.
    ///
    /// Tests only. The program itself must not wait - see `Drop` - but a test
    /// that lets its worker outlive it has that worker closing the library
    /// while the next test is depending on it being open.
    #[cfg(test)]
    fn shut_down(mut self) {
        self.stop.store(true, Ordering::Release);
        library::wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    /// A module with no source behind it.
    ///
    /// This is what a fresh install looks like, before the user has installed
    /// LibreHardwareMonitor. It reads as unavailable rather than as absent, so
    /// the strip keeps the space and the settings pane has something to
    /// explain.
    pub fn unconfigured() -> SensorsModule {
        let shared = Arc::new(Mutex::new(Shared {
            sensors: Vec::new(),
            error: Some(SensorError::NotConfigured),
        }));
        SensorsModule {
            shared,
            stop: Arc::new(AtomicBool::new(true)),
            readout: Readout::unavailable(),
            unit: TemperatureUnit::Celsius,
            pinned: None,
            poll: Arc::new(AtomicU32::new(POLL.as_secs() as u32)),
            demand: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

/// Keeps a helper running against the library that is installed right now.
///
/// The whole point of this loop is that "right now" changes. Someone presses
/// Install and a library appears where there was none; someone presses Remove
/// and it goes away under a helper that has it open; an install replaces the
/// files a running helper mapped. Each of those has to end with the strip
/// showing the truth without anybody restarting anything, so the library path
/// is re-read on every pass and the helper is opened, closed and reopened
/// around it.
fn supervise(
    mut open: Open,
    locate: Locate,
    shared: &Arc<Mutex<Shared>>,
    stop: &AtomicBool,
    poll: &AtomicU32,
    demand: &AtomicBool,
) {
    // Publishing to two places, and they are not the same place. `shared` is
    // the snapshot the strip samples from; `health` is what the settings window
    // reads, and it has no other way to find out - the strip owns this module
    // and the dialog does not. Without the second one, the Sensors pane said it
    // was waiting for a helper that had been answering for minutes.
    let publish = |name: &str, result: Result<Vec<Sensor>, SensorError>| {
        match &result {
            Ok(sensors) => health::publish(Health::Providing {
                source: name.to_string(),
                sensors: sensors.len(),
                temperatures: sensors.iter().filter(|s| s.kind == SensorKind::Temperature).count(),
            }),
            Err(why) => health::publish(Health::from_error(why)),
        }
        // The lock is taken only to swap the snapshot in, never held across a
        // read, so the strip never waits on the helper.
        if let Ok(mut shared) = shared.lock() {
            match result {
                Ok(sensors) => {
                    shared.sensors = sensors;
                    shared.error = None;
                }
                // The last good reading is dropped rather than left standing:
                // a temperature that stopped arriving is not the temperature
                // it was.
                Err(why) => {
                    shared.sensors.clear();
                    shared.error = Some(why);
                }
            }
        }
    };

    // The provider names itself, and the name is wanted for reporting even
    // after it has been dropped, so it is kept rather than borrowed.
    let mut name = String::from("sensor helper");
    let mut provider: Option<Box<dyn SensorProvider>> = None;
    // The directory the live helper was opened against, so a change can be
    // recognized. `None` is a real value here - it means "opened with no path,
    // left to its own search" - which is why it is compared rather than tested
    // for emptiness.
    let mut opened_with: Option<PathBuf> = None;
    let mut retry = FIRST_RETRY;
    let mut retry_at = Instant::now();

    while !stop.load(Ordering::Acquire) {
        // Anybody installing or removing the library gets it, first, before
        // anything else this pass might do with it.
        if library::wanted() {
            let was_running = provider.take().is_some();
            library::closed();
            if was_running {
                // Readings stop while the files are being replaced, and the
                // strip says so rather than holding the last temperature up as
                // though it were current.
                publish(&name, Err(SensorError::NotRunning));
            }
            // Whatever happens to the files while we are out, the helper that
            // comes back is opening against a different installation than the
            // one that just closed. Waiting out a backoff earned by the old
            // state would be exactly the delay this whole loop exists to
            // remove.
            retry = FIRST_RETRY;
            retry_at = Instant::now();
            library::wait_while_held(POLL);
            continue;
        }

        let wanted_library = locate();
        if provider.is_some() && wanted_library != opened_with {
            // The library moved, arrived or was replaced. The running helper
            // has the old one mapped and will hold it open forever; only a new
            // one reads what is there now.
            provider = None;
            library::closed();
            retry = FIRST_RETRY;
            retry_at = Instant::now();
        }

        if provider.is_none() && Instant::now() >= retry_at {
            // `opening` is the claim and the check in one step. Testing
            // `wanted` and then spawning would leave a window for a removal to
            // start deleting between the two.
            if library::opening() {
                match open(wanted_library.as_deref()) {
                    Ok(fresh) => {
                        name = fresh.name().to_string();
                        provider = Some(fresh);
                        opened_with = wanted_library.clone();
                        retry = FIRST_RETRY;
                    }
                    Err(why) => {
                        library::closed();
                        publish(&name, Err(why));
                        retry_at = Instant::now() + retry;
                        retry = (retry * 2).min(LONGEST_RETRY);
                    }
                }
            }
        }

        if let Some(helper) = provider.as_mut() {
            match helper.read() {
                Ok(sensors) => publish(&name, Ok(sensors)),
                Err(why) => {
                    // The helper died mid-read, which is the shape a crashed
                    // one takes. Let it go and let the next pass start another
                    // rather than reading a closed pipe forever.
                    provider = None;
                    library::closed();
                    publish(&name, Err(why));
                    retry_at = Instant::now() + retry;
                    retry = (retry * 2).min(LONGEST_RETRY);
                }
            }
        }

        // Both re-read every pass: the Sensors pane's slider moves the
        // interval while the worker is running, and a slider that only takes
        // effect on restart is a slider that does nothing as far as anybody
        // can tell; the demand changes whenever a panel opens or a column is
        // switched on. `rest` returns early when either does.
        library::rest(interval_for(
            demand.load(Ordering::Relaxed),
            poll.load(Ordering::Relaxed),
        ));
    }

    // Dropping the provider is what stops the helper process; saying so is
    // what lets a removal waiting on it go ahead.
    drop(provider);
    library::closed();
    health::publish(Health::Unknown);
}

impl Drop for SensorsModule {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wakes it out of its sleep rather than leaving it to notice up to a
        // poll later. Shutdown is the one moment where that delay is visible:
        // the helper is what holds the sensor driver open, and a Barometer
        // that has closed its window but not yet let go of the driver is a
        // Barometer that cannot be reinstalled over.
        library::wake();
        // The thread notices on its next pass and lets the provider go, which
        // is what stops the helper process. Not joined: it may be inside a
        // read, and waiting up to POLL to exit would show as the strip
        // hanging on shutdown.
    }
}

impl Module for SensorsModule {
    fn id(&self) -> ModuleId {
        ModuleId::Sensors
    }

    fn stack_value(
        &self,
        metric: &crate::stack::StackMetric,
        temperature: crate::weather::models::TemperatureUnit,
    ) -> Option<String> {
        use crate::stack::StackMetric::*;
        let shared = self.shared.lock().ok()?;
        let sensor = match metric {
            SensorsHottest => hottest_cpu(&shared.sensors),
            SensorsFan => shared.sensors.iter().find(|s| s.kind == SensorKind::Fan && s.value.is_some()),
            _ => None,
        }?;
        sensor.value.is_some().then(|| crate::stack::format_sensor(sensor, temperature))
    }

    fn sensor_error(&self) -> Option<SensorError> {
        self.shared.lock().ok().and_then(|shared| shared.error.clone())
    }

    /// Tells the worker whether anything can show a reading.
    ///
    /// Turning it on wakes the worker rather than leaving it to notice on
    /// its next pass, which is the difference between a Sensors panel that
    /// fills in as it opens and one that shows dashes for up to a minute.
    fn shown(&mut self, shown: bool) {
        if !self.demand.swap(shown, Ordering::Release) && shown {
            library::wake();
        }
    }

    fn configure(&mut self, settings: &crate::store::Settings) {
        self.unit = settings.sensors.temperature;
        self.pinned = settings.sensors.pinned_sensor_id.clone();
        self.poll.store(
            settings.sensors.poll_seconds.clamp(crate::store::MIN_POLL_SECONDS, crate::store::MAX_POLL_SECONDS),
            Ordering::Relaxed,
        );
    }

    /// Cloned rather than borrowed: the caller is the settings window's pin
    /// picker, reached from the sampling thread, and handing out a guard on the
    /// lock the worker publishes into would have the two contend for it on
    /// every tick.
    fn sensors(&self) -> Vec<Sensor> {
        self.shared.lock().map(|shared| shared.sensors.clone()).unwrap_or_default()
    }

    fn sample(&mut self) {
        let Ok(shared) = self.shared.lock() else {
            self.readout = Readout::unavailable();
            return;
        };
        // The pinned reading if there is one and the source still reports it;
        // a pin left behind by hardware that is no longer here falls back to
        // the hottest rather than reading unavailable forever.
        let shown = self
            .pinned
            .as_deref()
            .and_then(|id| shared.sensors.iter().find(|sensor| sensor.id == id))
            .or_else(|| hottest_cpu(&shared.sensors));
        self.readout = match shown {
            Some(sensor) => match sensor.value {
                Some(degrees) => {
                    Readout::one(self.unit.describe(degrees)).reserving(self.unit.widest())
                }
                None => Readout::unavailable(),
            },
            None => Readout::unavailable(),
        };
    }

    fn detail(&self) -> Vec<String> {
        let Ok(shared) = self.shared.lock() else {
            return vec![self.id().title().to_string(), "Not available".to_string()];
        };
        let mut lines = vec![self.id().title().to_string()];

        if let Some(why) = &shared.error {
            // The reason matters more here than anywhere else in the program:
            // "no source configured" is an install away from working and
            // "refused" is a UAC prompt away, and the user can act on either
            // once they know which one they have.
            lines.push(why.to_string());
            return lines;
        }

        // The one on the strip first, named, so it is obvious what the number
        // on the taskbar actually measures.
        if let Some(hot) = hottest_cpu(&shared.sensors) {
            if let Some(value) = hot.value {
                lines.push(format!(
                    "{} - {} {}",
                    hot.hardware,
                    hot.name,
                    self.unit.describe(value)
                ));
            }
        }

        // Then the next few hottest, which is what somebody who stops to look
        // is usually looking for.
        let mut rest: Vec<&Sensor> = shared
            .sensors
            .iter()
            .filter(|s| s.kind == SensorKind::Temperature && s.value.is_some())
            .collect();
        rest.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
        for sensor in rest.iter().take(5) {
            lines.push(format!(
                "{} - {} {}",
                sensor.hardware,
                sensor.name,
                self.unit.describe(sensor.value.unwrap_or_default())
            ));
        }
        if lines.len() == 1 {
            lines.push("No readable temperatures".to_string());
        }
        lines
    }

    fn readout(&self) -> Readout {
        self.readout.clone()
    }
}

#[cfg(test)]
mod cadence_tests {
    use super::*;

    #[test]
    fn the_helper_is_asked_at_the_users_rate_only_while_something_can_show_what_it_says() {
        let nobody = SensorDemand::default();
        assert!(!nobody.any());
        // Slowed, never stopped: a picker with nothing in it and a pane that
        // cannot say whether the source is alive would be the worse trade.
        assert_eq!(nobody.interval(2), IDLE_POLL);
        assert_eq!(nobody.interval(60), IDLE_POLL);

        // Any one of the three is enough, and each of them is somebody looking.
        for demand in [
            SensorDemand { on_the_strip: true, ..Default::default() },
            SensorDemand { panel_open: true, ..Default::default() },
            SensorDemand { settings_visible: true, ..Default::default() },
        ] {
            assert!(demand.any());
            assert_eq!(demand.interval(2), Duration::from_secs(2), "the user's interval, unchanged");
        }
    }

    #[test]
    fn a_stored_interval_from_outside_the_sliders_range_is_brought_back_into_it() {
        let looking = SensorDemand { on_the_strip: true, ..Default::default() };
        // Zero would be a busy loop against another process, and an hour would
        // be a strip whose temperature is an hour old while somebody watches
        // it. The pane clamps what it writes; this clamps what it reads.
        assert_eq!(looking.interval(0), Duration::from_secs(crate::store::MIN_POLL_SECONDS.into()));
        assert_eq!(
            looking.interval(u32::MAX),
            Duration::from_secs(crate::store::MAX_POLL_SECONDS.into())
        );
        // And the idle cadence is longer than anything the slider can ask for,
        // or switching everything off would speed the helper up.
        assert!(IDLE_POLL >= looking.interval(crate::store::MAX_POLL_SECONDS));
    }
}

#[cfg(test)]
mod supervision_tests {
    use super::*;
    use crate::sensors::SensorKind;
    use std::sync::atomic::AtomicUsize;

    /// A provider that reads without touching hardware, and says when it dies.
    ///
    /// The flag is the whole point of it: "the helper was closed" is the
    /// property most of these tests turn on, and it is only observable from the
    /// provider's own `Drop`.
    struct FakeHelper {
        closed: Arc<AtomicBool>,
    }

    impl Drop for FakeHelper {
        fn drop(&mut self) {
            self.closed.store(true, Ordering::Release);
        }
    }

    impl SensorProvider for FakeHelper {
        fn name(&self) -> &str {
            "fake"
        }

        fn read(&mut self) -> Result<Vec<Sensor>, SensorError> {
            Ok(vec![Sensor {
                id: "/amdcpu/0/temperature/0".into(),
                name: "CPU Package".into(),
                hardware: "Fake".into(),
                kind: SensorKind::Temperature,
                value: Some(42.0),
            }])
        }
    }

    /// Waits for a condition, so a broken worker fails as an assertion rather
    /// than by hanging the suite.
    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if done() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    /// Drives one pass of the worker without waiting out POLL.
    ///
    /// Not a hook bolted on for the tests: this is exactly what an install
    /// does - take the library, put it back - and the worker coming round
    /// promptly for it is the behavior being relied on in production.
    fn kick() {
        drop(library::hold());
    }

    #[test]
    fn a_library_that_appears_while_running_starts_the_helper_without_a_restart() {
        let _sequence = library::sequence();

        // What `lhm_install::installed()` would report. Nothing, yet.
        let installed: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let attempts = Arc::new(AtomicUsize::new(0));
        let opened_against: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

        let module = {
            let attempts = Arc::clone(&attempts);
            let opened_against = Arc::clone(&opened_against);
            let installed = Arc::clone(&installed);
            SensorsModule::supervised(
                Box::new(move |library| {
                    attempts.fetch_add(1, Ordering::AcqRel);
                    // Standing in for the real helper, which exits reporting
                    // "no-library" when there is nothing for it to load.
                    let Some(directory) = library else {
                        return Err(SensorError::NotConfigured);
                    };
                    *opened_against.lock().unwrap() = Some(directory.to_path_buf());
                    Ok(Box::new(FakeHelper { closed: Arc::new(AtomicBool::new(false)) })
                        as Box<dyn SensorProvider>)
                }),
                Box::new(move || installed.lock().unwrap().clone()),
            )
        };

        until("the first attempt with no library", || attempts.load(Ordering::Acquire) >= 1);
        assert!(
            opened_against.lock().unwrap().is_none(),
            "opened a helper against a library that is not there"
        );

        // The user presses Install and a library lands. Nothing is restarted.
        *installed.lock().unwrap() = Some(PathBuf::from(r"C:\Fake\LibreHardwareMonitor"));
        kick();

        until("the helper to open against the new library", || {
            opened_against.lock().unwrap().is_some()
        });
        assert_eq!(
            opened_against.lock().unwrap().as_deref(),
            Some(Path::new(r"C:\Fake\LibreHardwareMonitor"))
        );

        // And the readings reach the strip, which is what the user was looking
        // at when they had to restart Barometer by hand to get it.
        until("a temperature to reach the strip", || {
            module.shared.lock().map(|s| !s.sensors.is_empty()).unwrap_or(false)
        });

        module.shut_down();
    }

    #[test]
    fn removing_the_library_closes_the_helper_before_the_files_go() {
        let _sequence = library::sequence();

        let closed = Arc::new(AtomicBool::new(false));
        let installed = Arc::new(Mutex::new(Some(PathBuf::from(r"C:\Fake\LHM"))));
        let opens = Arc::new(AtomicUsize::new(0));

        let module = {
            let closed = Arc::clone(&closed);
            let opens = Arc::clone(&opens);
            let installed = Arc::clone(&installed);
            SensorsModule::supervised(
                Box::new(move |_| {
                    opens.fetch_add(1, Ordering::AcqRel);
                    Ok(Box::new(FakeHelper { closed: Arc::clone(&closed) })
                        as Box<dyn SensorProvider>)
                }),
                Box::new(move || installed.lock().unwrap().clone()),
            )
        };

        until("the helper to start", || opens.load(Ordering::Acquire) >= 1);
        assert!(!closed.load(Ordering::Acquire));

        // This is what `uninstall` does before it deletes anything. By the time
        // it returns, the file about to be removed must not be open in a live
        // process: Windows will not delete it otherwise, and the Remove button
        // fails with an error the user can do nothing about.
        let held = library::hold();
        assert!(
            closed.load(Ordering::Acquire),
            "hold() returned while the helper still had the library open"
        );

        let opens_during = opens.load(Ordering::Acquire);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            opens.load(Ordering::Acquire),
            opens_during,
            "the worker started a new helper while the library was being deleted"
        );

        drop(held);
        until("the helper to come back once the removal is done", || {
            opens.load(Ordering::Acquire) > opens_during
        });

        module.shut_down();
    }

    #[test]
    fn a_library_that_is_replaced_gets_a_fresh_helper() {
        let _sequence = library::sequence();

        let installed = Arc::new(Mutex::new(Some(PathBuf::from(r"C:\Fake\First"))));
        let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

        let module = {
            let opened = Arc::clone(&opened);
            let installed = Arc::clone(&installed);
            SensorsModule::supervised(
                Box::new(move |library| {
                    opened.lock().unwrap().push(library.unwrap_or(Path::new("")).to_path_buf());
                    Ok(Box::new(FakeHelper { closed: Arc::new(AtomicBool::new(false)) })
                        as Box<dyn SensorProvider>)
                }),
                Box::new(move || installed.lock().unwrap().clone()),
            )
        };

        until("the first helper", || !opened.lock().unwrap().is_empty());

        // A running helper has the old library mapped and will hold it open for
        // as long as it lives. Only a new one reads what is on disk now, which
        // is what makes Reinstall mean anything.
        *installed.lock().unwrap() = Some(PathBuf::from(r"C:\Fake\Second"));
        kick();

        until("a helper against the replacement", || {
            opened.lock().unwrap().iter().any(|p| p == Path::new(r"C:\Fake\Second"))
        });

        module.shut_down();
    }

    #[test]
    fn a_helper_that_will_not_start_is_not_respawned_in_a_tight_loop() {
        let _sequence = library::sequence();

        let attempts = Arc::new(AtomicUsize::new(0));
        let module = {
            let attempts = Arc::clone(&attempts);
            SensorsModule::supervised(
                Box::new(move |_| {
                    attempts.fetch_add(1, Ordering::AcqRel);
                    Err(SensorError::NotConfigured)
                }),
                Box::new(|| None),
            )
        };

        until("the first attempt", || attempts.load(Ordering::Acquire) >= 1);
        thread::sleep(Duration::from_millis(700));
        // Without the backoff this is a .NET process spawned every POLL,
        // forever, on every machine that has not installed a library - which is
        // all of them until somebody presses Install.
        let so_far = attempts.load(Ordering::Acquire);
        assert!(so_far <= 2, "spawned the helper {so_far} times in under a second");

        module.shut_down();
    }
}
