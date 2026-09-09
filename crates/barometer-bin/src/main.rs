// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Barometer draws on the taskbar. That is what it does when you run it, with
// no arguments and no console window.
//
// The console harness is still here behind --console, because a printed
// readout is far easier to watch against real hardware than a strip of
// two-row text at the bottom of the screen, and it is how every module was
// developed. --marks and --reserve are diagnostics of the same kind.

// A GUI subsystem binary, so double-clicking the shortcut does not flash a
// console window behind the taskbar. The diagnostic modes still print: they
// attach to the console they were launched from, if there is one.
#![windows_subsystem = "windows"]

// The readout itself lives in barometer-app. This package is the executable
// and nothing else; see its Cargo.toml for why the two are separate.
use barometer_app::{
    lhm_install, pawnio, reserve, settings_ui, spacer, trace, tray, update, window,
};

use std::env;
use std::collections::VecDeque;
use std::thread;
use std::time::Duration;

use barometer_core::modules::{
    CpuModule, DiskModule, GpuModule, MemoryModule, NetworkModule, SensorsModule, WeatherModule,
};
use barometer_core::sensors::{helper::HelperProvider, hottest, SensorKind, SensorProvider};
use barometer_core::settings::StripFont;
use barometer_core::taskbar::{self, Density};
use barometer_core::{Module, Readout};

/// Matches the Mac app's default cadence. Fast enough that network spikes are
/// visible, slow enough to stay invisible in a CPU graph.
const INTERVAL: Duration = Duration::from_secs(1);

/// Owns the session-wide strip mutex for the life of the process.
struct StripInstance(windows_sys::Win32::Foundation::HANDLE);

impl StripInstance {
    fn claim() -> Option<StripInstance> {
        use windows_sys::Win32::Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS,
        };
        use windows_sys::Win32::System::Threading::CreateMutexW;

        let name: Vec<u16> = "Local\\Barometer.Win32.TaskbarStrip"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: the name is terminated and lives through the call. The
        // handle is retained below so the kernel object exists until exit.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        // GetLastError must be read before any other Windows call. A second
        // process receives a valid handle to the existing object plus this
        // status; it closes that handle and exits before touching the tray.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(StripInstance(handle))
    }
}

impl Drop for StripInstance {
    fn drop(&mut self) {
        // SAFETY: this struct owns the handle returned by CreateMutexW.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

fn render(readout: &Readout, module: &dyn Module) -> String {
    let label = module.id().label();
    if readout.unavailable {
        return format!("{label} --");
    }
    match &readout.secondary {
        Some(second) => format!("{label} {} / {}", readout.primary, second),
        None => format!("{label} {}", readout.primary),
    }
}

/// Dumps whatever the sensor helper reports, so it can be watched against real
/// hardware. The helper is optional: absent, this says so and moves on, which
/// is what a fresh install looks like.
fn dump_sensors(path: &std::path::Path) {
    println!("Sensor helper: {}", path.display());
    // The same library the strip would use, so what this prints is what the
    // readout would show rather than whatever the helper finds on its own.
    let library = lhm_install::installed();
    match HelperProvider::spawn(path, library.as_deref()) {
        Ok(mut provider) => {
            println!("  helper reports {} devices", provider.devices());
            match provider.read() {
            Ok(sensors) => {
                let temperatures =
                    sensors.iter().filter(|s| s.kind == SensorKind::Temperature).count();
                let readable = sensors.iter().filter(|s| s.value.is_some()).count();
                println!(
                    "  {} sensors from {} ({} readable, {} temperatures)",
                    sensors.len(),
                    provider.name(),
                    readable,
                    temperatures
                );
                for sensor in sensors.iter().filter(|s| s.kind == SensorKind::Temperature).take(8) {
                    println!(" {sensor}");
                }
                match hottest(&sensors) {
                    Some(hot) => println!("  hottest: {hot}"),
                    None => println!("  hottest: nothing readable"),
                }
            }
                Err(why) => println!("  read failed: {why}"),
            }
        }
        Err(why) => println!("  not started: {why}"),
    }
    println!();
}

/// Where the sensor helper is, if it is there at all.
///
/// Beside the executable, which is where the installer puts it. Resolved from
/// the running image rather than the working directory: the strip is started
/// from the Run key and from a shortcut, and neither of those runs it from its
/// own folder.
fn helper_path() -> Option<std::path::PathBuf> {
    let exe = env::current_exe().ok()?;
    Some(exe.with_file_name("barometer-sensors.exe"))
}

/// The Sensors module, with a helper behind it when one can be started.
///
/// A machine with no helper installed still gets the module: it reads as
/// unavailable, which is the truth and is what the settings pane has something
/// to say about. Silently dropping it would look like Barometer does not do
/// temperatures at all.
fn sensors_module() -> SensorsModule {
    let Some(path) = helper_path() else {
        return SensorsModule::unconfigured();
    };
    // The module supervises the helper rather than being handed a running one,
    // because the answer to "where is LibreHardwareMonitor" changes while
    // Barometer is running. Pressing Install in the settings window is the
    // ordinary way it changes and pressing Remove is the other, and a helper
    // started once at launch can only ever report the machine as it stood at
    // launch - which is why installing the library used to do nothing visible
    // until the next restart.
    //
    // Both closures are asked again on every pass of the sensor worker.
    // `installed()` reports Barometer's own copy and nothing else; `None` is
    // not a failure but an ordinary answer, and leaves the helper to its own
    // search, which is what finds an installation the user made themselves.
    SensorsModule::supervised(
        Box::new(move |library| {
            HelperProvider::spawn(&path, library)
                .map(|helper| Box::new(helper) as Box<dyn SensorProvider>)
        }),
        Box::new(lhm_install::installed),
    )
}


/// What the settings window shows that is not a setting.
///
/// Built here and pushed, rather than sampled over there, because this is the
/// thread that owns the modules. The window asking the sensor helper itself
/// would mean two readers of one process, each paying for the other's walk of
/// every device on the machine.
///
/// Without this the window's snapshot stayed at its default for the life of the
/// program: the Sensors pane said "Waiting for the sensor helper" on a machine
/// that had been reporting temperatures for minutes, the pin picker had nothing
/// to pick from, and the preview drew invented numbers.
/// A graphics reading that only the sensor source has, formatted for a stack.
///
/// The adapter is the one the panel names: the chosen one, or the first the
/// module lists, which is how `GpuModule::active` orders them.
fn gpu_from_sensors(
    metric: &barometer_core::stack::StackMetric,
    sensors: &[barometer_core::sensors::Sensor],
    gpus: &[settings_ui::model::GpuAdapter],
    temperature: barometer_core::weather::models::TemperatureUnit,
) -> Option<String> {
    use barometer_core::sensors::{self, SensorKind};
    use barometer_core::stack::StackMetric;

    let adapter = gpus.first().map(|adapter| adapter.name.as_str());
    match metric {
        StackMetric::GpuPower => sensors::gpu_reading(sensors, adapter, SensorKind::Power, &sensors::GPU_POWER)
            .map(|watts| format!("{watts:.1}W")),
        StackMetric::GpuTemperature => {
            sensors::gpu_reading(sensors, adapter, SensorKind::Temperature, &sensors::GPU_TEMPERATURE)
                .map(|celsius| temperature.describe(celsius))
        }
        _ => None,
    }
}

/// What the settings window and the strip's preview are drawn from.
///
/// `for_settings` decides how much of it is built. The strip's preview needs
/// the readouts, the sensors and the stack readings and nothing else; the
/// pickers in the settings window need the graphics adapters, the network
/// interfaces, the volumes, the disks and the state of the sensor source, and
/// those cost a `GetIfTable2` over every interface on the machine, an adapter
/// enumeration and several clones apiece.
///
/// All of it used to be built on every pass, and a pass is not only the
/// once-a-second sample: the readout follows the tray for half a second after
/// any tray change at a sixty-millisecond cadence, so one icon appearing made
/// eight of these. Now the expensive half is built only while there is a
/// window open to show it in.
fn snapshot(
    modules: &[Box<dyn Module>],
    height_dip: f32,
    settings: &barometer_core::store::Settings,
    for_settings: bool,
) -> settings_ui::Snapshot {
    use barometer_core::sensors::health::{self, Health};
    use settings_ui::model::SensorSource;

    // Two questions, not one. The list is what the module last read; the state
    // is what the worker last published. A source can be running and have found
    // nothing, which is not the same as there being no source.
    let sensors = modules
        .iter()
        .find(|module| module.id() == barometer_core::ModuleId::Sensors)
        .map(|module| module.sensors())
        .unwrap_or_default();

    let sensor_source = match if for_settings { health::health() } else { Health::Unknown } {
        Health::Providing { source, sensors, .. } => {
            SensorSource::Providing { name: source, readings: sensors }
        }
        Health::NotConfigured => SensorSource::NotInstalled,
        Health::NotRunning => SensorSource::NotRunning,
        Health::Failed(why) => SensorSource::Failed(why),
        Health::Unknown => SensorSource::Unknown,
    };

    let gpus: Vec<settings_ui::model::GpuAdapter> = modules
        .iter()
        .find(|module| module.id() == barometer_core::ModuleId::Gpu)
        .map(|module| module.gpu_adapters())
        .unwrap_or_default()
        .into_iter()
        .map(|(key, name)| settings_ui::model::GpuAdapter { key, name })
        .collect();

    // Every reading in every stack, asked of its module. Sensor readings
    // are looked up by id from the sensor list instead.
    let mut metric_values: Vec<(barometer_core::stack::StackMetric, String)> = Vec::new();
    for stack in &settings.stacks.stacks {
        for entry in &stack.metrics {
            let metric = &entry.metric;
            if matches!(metric, barometer_core::stack::StackMetric::Sensor(_))
                || metric_values.iter().any(|(m, _)| m == metric)
            {
                continue;
            }
            let value = modules
                .iter()
                .find(|m| m.id() == metric.module())
                .and_then(|m| m.stack_value(metric, settings.sensors.temperature))
                // The GPU's power and temperature are not Windows' figures to
                // give - the engine counters carry neither - so the graphics
                // module cannot answer for them and the sensor source can.
                // Without this the two readings were offered in the picker
                // and could only ever draw a dash.
                .or_else(|| gpu_from_sensors(metric, &sensors, &gpus, settings.sensors.temperature));
            if let Some(value) = value {
                metric_values.push((metric.clone(), value));
            }
        }
    }

    let disks_module = modules
        .iter()
        .find(|module| module.id() == barometer_core::ModuleId::Disks);

    let (disks_module, weather_module) = match for_settings {
        true => (
            disks_module,
            modules.iter().find(|module| module.id() == barometer_core::ModuleId::Weather),
        ),
        false => (None, None),
    };

    settings_ui::Snapshot {
        readouts: modules.iter().map(|module| (module.id(), module.readout())).collect(),
        taskbar_height_dip: height_dip,
        sensors,
        sensor_source,
        gpus,
        // Everything below is for a picker in the settings window, and the
        // interfaces line is the expensive one: it is a whole GetIfTable2.
        interfaces: match for_settings {
            true => modules
                .iter()
                .find(|module| module.id() == barometer_core::ModuleId::Network)
                .map(|module| module.net_interfaces())
                .unwrap_or_default(),
            false => Vec::new(),
        },
        metric_values,
        volumes: disks_module.map(|module| module.volumes()).unwrap_or_default(),
        disks: disks_module.map(|module| module.disk_devices()).unwrap_or_default(),
        located: weather_module.and_then(|module| module.located()),
        weather_error: weather_module.and_then(|module| module.weather_error()),
        ..Default::default()
    }
}

/// Why the readout cannot live on a side-docked taskbar.
///
/// Joined lines rather than one literal spanning source lines: a literal
/// that spans lines carries the source's indentation into the message box,
/// which put twenty-one spaces in the middle of a sentence.
const SIDE_EDGE: &str = concat!(
    "Barometer needs the taskbar on the top or bottom edge.\n\n",
    "Docked to the left or right, widening the notification area frees no ",
    "horizontal space, so the readout has nowhere to go.\n\n",
    "Move the taskbar to the top or the bottom and start Barometer again.",
);

/// Says something the user needs to hear, wherever they can hear it.
///
/// A message box under a shortcut, stderr under a console. The same words
/// either way; the only question is whether anybody is watching a terminal.
fn complain(message: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

    eprintln!("{message}");
    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    };
    // SAFETY: both strings are null-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(message).as_ptr(),
            wide("Barometer").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}

/// Attaches to the console this was launched from, if any.
///
/// A `windows_subsystem = "windows"` binary starts with no standard handles at
/// all, so every `println!` in the diagnostic modes goes nowhere and running
/// `barometer --console` from a terminal looks like the program did nothing.
/// Attaching to the parent gets the output back without ever creating a
/// console window of its own, which is the thing being avoided.
fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: takes no arguments but a sentinel and fails harmlessly when
    // there is no parent console, which is the case under a shortcut.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

fn main() {
    // Before anything measures anything.
    window::declare_dpi_aware();

    // Every mode but the strip prints, and none of them can print until the
    // console is attached.
    let diagnostic = env::args().skip(1).any(|a| a.starts_with("--") && a != "--strip");
    if diagnostic {
        attach_console();
    }

    // One argument, the number of samples to take. Zero means run until
    // interrupted, which is what you want when watching a live machine.
    let samples: u32 = env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(10);

    // Second argument points at the sensor helper, for watching it work.
    // Guarded on looking like a path, so a flag's operand is not mistaken for
    // one - "--reserve 4" was being read as a helper named "4".
    if let Some(path) = env::args().nth(2).filter(|a| a.ends_with(".exe")) {
        dump_sensors(std::path::Path::new(&path));
    }

    if let Some(position) = env::args().position(|a| a == "--marks") {
        let path = env::args().nth(position + 1).unwrap_or_else(|| "marks.bmp".into());
        run_marks_sheet(std::path::Path::new(&path));
        return;
    }

    if env::args().any(|a| a == "--menu") {
        // The right-click menu on its own, for looking at it. See
        // window::preview_menu for why it cannot be photographed any other
        // way. It sits there until dismissed, which is what a screenshot
        // needs.
        window::preview_menu(500, 500);
        return;
    }

    if env::args().any(|a| a == "--settings") {
        // The settings window on its own, with no readout behind it. For
        // looking at the window itself: reaching it through the right-click
        // menu means having the strip up and finding it on the taskbar first,
        // which is a lot of ceremony when the thing being examined is a
        // dialog.
        let store = barometer_core::store::Store::open().ok();
        let settings = store
            .map(|mut store| store.load().settings)
            .unwrap_or_default();
        let open = settings_ui::SettingsWindow::open(settings_ui::Model::from_settings(settings));
        // The window is created on its own thread, so is_open() is false for
        // the first few milliseconds and looping on it alone exits instantly.
        // Wait for it to appear, then wait for it to go.
        for _ in 0..100 {
            if open.is_open() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        while open.is_open() {
            std::thread::sleep(Duration::from_millis(120));
        }
        return;
    }

    if env::args().any(|a| a == "--repair") {
        run_repair();
        return;
    }

    if env::args().any(|a| a == "--install-lhm") {
        // The settings pane will own this. Until it exists, this is how the
        // fetch gets exercised against a real machine.
        match lhm_install::install(Some(&|progress| println!("  {progress:?}"))) {
            Ok(directory) => {
                println!("LibreHardwareMonitor installed to {}", directory.display())
            }
            Err(why) => println!("Could not install LibreHardwareMonitor: {why}"),
        }
        return;
    }

    if env::args().any(|a| a == "--weather") {
        run_weather_probe();
        return;
    }

    // Drawing on the taskbar is what this program is. The console readout is
    // the diagnostic, not the default; it was the other way round only while
    // there was no window to draw in.
    let console_mode = env::args().any(|a| a == "--console");
    if let Some(position) = env::args().position(|a| a == "--reserve") {
        let count = env::args().nth(position + 1).and_then(|n| n.parse().ok()).unwrap_or(4);
        run_reserve_probe(count);
        return;
    }

    let mut modules: Vec<Box<dyn Module>> = vec![
        Box::new(NetworkModule::new()),
        Box::new(CpuModule::new()),
        Box::new(MemoryModule::new()),
        Box::new(GpuModule::new()),
        Box::new(DiskModule::new()),
        Box::new(sensors_module()),
        Box::new(WeatherModule::new(Default::default())),
    ];

    // The strip cannot exist on a side edge, so the app says why and leaves
    // rather than running invisibly.
    match taskbar::taskbar() {
        Some(bar) => {
            if diagnostic {
                let font = StripFont::default();
                let density =
                    Density::choose_with_font(bar.height() as f32, modules.len(), &font);
                println!(
                    "Taskbar: {:?} edge, {}x{}, supported={}",
                    bar.edge,
                    bar.width(),
                    bar.height(),
                    bar.edge.is_supported()
                );
                println!(
                    "Layout:  {}dip {} {}, {} row(s) for {} widgets",
                    density.text_dip,
                    font.family,
                    font.weight.raw_value(),
                    if density.two_rows { 2 } else { 1 },
                    modules.len()
                );
            }
            if !bar.edge.is_supported() {
                // Said out loud rather than printed. Under a shortcut there is
                // no console to print to, and a program that exits silently
                // the moment the taskbar is moved to the left edge looks like
                // a program that crashed.
                complain(SIDE_EDGE);
                return;
            }
        }
        None => {
            if diagnostic {
                println!("Taskbar: not reported by the shell (Explorer may be restarting)");
            }
        }
    }

    if !console_mode {
        // Placeholder GUIDs are process-global shell identities. If two
        // instances manage them, each instance removes icons the other still
        // believes it owns, producing an empty reservation and a disappearing
        // readout. Only the first process is allowed to touch them.
        let Some(_instance) = StripInstance::claim() else { return };
        run_strip(modules);
        return;
    }

    println!("Barometer engine, sampling every {:?}. NET is down / up.", INTERVAL);

    let mut taken = 0u32;
    loop {
        for module in modules.iter_mut() {
            module.sample();
        }

        let line: Vec<String> = modules
            .iter()
            .map(|m| render(&m.readout(), m.as_ref()))
            .collect();
        println!("{}", line.join(" |   "));

        taken += 1;
        if samples != 0 && taken >= samples {
            break;
        }
        thread::sleep(INTERVAL);
    }
}

/// Saves, and says so out loud the first time it cannot.
///
/// Silent before: this is a windows-subsystem binary with no console, so an
/// `eprintln!` here went nowhere. Mark settings.json read-only, or have a
/// scanner hold it long enough for the rename to be refused, and an evening
/// of rearranging applied live and was gone at the next launch with no sign
/// that anything had happened. The corrupt-*load* path has always used a
/// message box; this is the same courtesy on the way out.
///
/// Once per run, because a save follows every change the user makes and a
/// dialog per keystroke would be worse than the silence it replaces.
fn save_settings(store: Option<&mut barometer_core::store::Store>, settings: &barometer_core::store::Settings) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static TOLD: AtomicBool = AtomicBool::new(false);

    let Some(store) = store else { return };
    let Err(why) = store.save(settings) else { return };
    eprintln!("Could not save settings: {why}");
    if TOLD.swap(true, Ordering::Relaxed) {
        return;
    }
    complain(&format!(
        "Barometer could not save your settings.

{why}

The changes you have just made are in effect now, but they will be gone the next time Barometer starts. This is usually a read-only settings file or another program holding it open."
    ));
}

/// A timeline for a panel to draw, or nothing when no panel is open.
///
/// The published snapshot used to carry a copy of the whole day either way.
/// Each timeline saturates at 86,400 samples of 16 bytes, so that was three
/// allocations of about 1.4 MB, three copies of the same, and three frees,
/// every second - for graphs that in the ordinary case nobody is looking at.
fn history_for(
    open: bool,
    history: &VecDeque<barometer_app::flyout::cpu::Sample>,
) -> Vec<barometer_app::flyout::cpu::Sample> {
    match open {
        true => history.iter().copied().collect(),
        false => Vec::new(),
    }
}

/// Now, in Unix seconds, or None if the clock is before 1970.
fn unix_now() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs())
}

/// Says what the weekly check found, and returns a new value for
/// `skipped_update` when the user asked to skip this release.
///
/// Three answers rather than two, because the settings pane already offers to
/// stop skipping a release and nothing could ever start. The buttons are
/// named in the message: Windows will not relabel them.
fn offer_update(found: &update::Outcome, skipped: Option<&str>) -> Option<Option<String>> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDCANCEL, IDYES, MB_ICONINFORMATION, MB_YESNOCANCEL,
    };

    let update::Outcome::Newer(release) = found else { return None };
    let version = release.version.to_string();
    // A version the user has already declined stays quiet, which is the whole
    // difference between this check and the one they asked for.
    if !update::should_offer(&version, skipped, false) {
        return None;
    }
    let message = format!(
        "Barometer {version} is available. You are running {}.

{}

Yes - open the download page
No - remind me next week
Cancel - skip this release",
        update::Version::running(),
        release.notes
    );
    let wide = |text: &str| -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    };
    // SAFETY: both strings are null-terminated and outlive the call.
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(&message).as_ptr(),
            wide("Barometer - update available").as_ptr(),
            MB_YESNOCANCEL | MB_ICONINFORMATION,
        )
    };
    match answer {
        IDYES => {
            open_url(&format!("https://github.com/{}/{}/releases", update::OWNER, update::REPOSITORY));
            None
        }
        IDCANCEL => Some(Some(version)),
        _ => None,
    }
}

/// Opens a page in whatever the user's browser is.
fn open_url(url: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    };
    // SAFETY: NUL-terminated strings; the returned pseudo-handle is not a
    // resource and is deliberately dropped.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL as i32,
        );
    }
}

/// Whether anything the user has switched on actually wants a temperature.
///
/// The Sensors column is the obvious one, but a sensor reading put into a
/// stack wants the driver just as much and can be the only thing that does.
fn wants_temperatures(settings: &barometer_core::store::Settings) -> bool {
    use barometer_core::ModuleId;
    let column = settings
        .modules
        .iter()
        .any(|entry| entry.id == ModuleId::Sensors && entry.enabled);
    let in_a_stack = settings.stacks.stacks.iter().any(|stack| {
        stack.is_enabled
            && stack
                .metrics
                .iter()
                .any(|entry| entry.metric.module() == ModuleId::Sensors)
    });
    column || in_a_stack
}

/// Runs the readout on the taskbar until the window goes away.
///
/// Sampling and painting share this thread. The modules are a handful of
/// syscalls each, so the work per tick is far below the frame it sits in; the
/// sensor helper is the exception and belongs on its own thread before it is
/// wired in here.
fn run_strip(mut modules: Vec<Box<dyn Module>>) {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG,
        PM_REMOVE, QS_ALLINPUT, WM_QUIT,
    };

    let Some(mut strip) = window::Strip::create() else {
        eprintln!("Could not find the taskbar. Explorer may be restarting.");
        return;
    };

    trace::opened();

    // Whatever the user last chose. A corrupt file is reported and defaults
    // are used; it is never silently overwritten - the store renames it aside
    // so the user can get it back.
    let mut store = barometer_core::store::Store::open().ok();
    let loaded = store.as_mut().map(|s| s.load());
    if let Some(barometer_core::store::Load { corrupt: Some(corrupt), .. }) = &loaded {
        match &corrupt.saved_as {
            Some(path) => complain(&format!(
                "Barometer could not read its settings, so it has started with the defaults.

The file it could not read has been kept, at:
{}",
                path.display()
            )),
            None => complain(&format!(
                "Barometer could not read its settings and could not move them aside, so it has started with the defaults and will not save over them.

{}",
                corrupt.reason
            )),
        }
    }
    let mut settings = loaded.map(|l| l.settings).unwrap_or_default();

    // After the settings are read, and after the single-instance claim, so
    // that the answer is the user's own. The literal `true` this used to be
    // given ran before either: `advice` opens by returning nothing when
    // sensors are off - "nothing about a temperature driver is worth a dialog
    // to somebody who is not asking for temperatures" - and that check could
    // never fire, so a first run with Sensors switched off still got a modal
    // driver dialog, and a second launch could get one before exiting.
    // Before the readout goes up, though, so somebody who cannot get
    // temperatures is told while they are still looking at the thing they
    // just started.
    pawnio::prompt_if_needed(wants_temperatures(&settings));

    let mut font = settings.font.clone();
    // Kept across passes rather than rebuilt on each one. Building it deep
    // clones the whole settings document - every location, stack, module
    // entry and font string - and then reconciles the strip order, which is
    // quadratic in the number of items. None of that changes between two
    // readings of a processor.
    let mut model = settings_ui::Model::from_settings(settings.clone());
    let mut settings_window: Option<settings_ui::SettingsWindow> = None;
    let mut next_sample = Instant::now();
    // What the weekly check found, filled by a thread of its own.
    let weekly: std::sync::Arc<std::sync::Mutex<Option<update::Outcome>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut weekly_running = false;
    const FORCE_GAP: Duration = Duration::from_millis(60);
    /// How long after a tray event the readout keeps riding the tray.
    const FOLLOW_FOR: Duration = Duration::from_millis(500);
    /// Room left empty at the chevron end of the reserved region, in pixels.
    const LEFT_INSET: i32 = 6;

    let mut last_forced = Instant::now() - FORCE_GAP;
    let mut follow_until: Option<Instant> = None;

    apply_to_modules(&mut modules, &settings);

    // Claim space in the notification area, which is what makes Windows repack
    // the task buttons aside. Without it the readout has nowhere to go that is
    // not already somebody else's.
    let mut dpi = window::taskbar_dpi();
    // How long the readout stays hidden waiting for a reserved region before
    // it gives up and places itself the ordinary way.
    //
    // The placeholders go in one per sampling tick, so a normal eight-slot
    // strip takes about eight seconds to finish. Leave several more ticks for
    // promotion and the first complete position sample.
    // What it is really guarding against is the region never arriving at all -
    // placeholders the shell refuses, a block of identities that turns out to
    // be dead, the user switching them off in Settings. Barometer used to hide
    // forever in that case, which looks exactly like the program crashing:
    // blank placeholders sitting in the tray and no readout, for as long as
    // you care to wait. Never being visible at all is not an option.
    const HOLD_TIMEOUT: Duration = Duration::from_secs(12);
    // Started on the first pass, and restarted when the shell does.
    let mut hold_deadline: Option<Instant> = None;
    // Which module each strip column belongs to, left to right, as of the
    // last layout - a stack's columns belong to no module. A click looks its
    // column up here.
    let mut column_items: Vec<barometer_core::stack::StripItem> = Vec::new();

    // The detail panel. Created hidden on a thread of its own, so the first
    // click opens a window that already exists. One content per module,
    // each reading a feed this loop publishes on every tick, the same way
    // the strip is fed.
    use barometer_app::flyout::{cpu, disks, gpu, memory, network, sensors as sensor_panel, weather};
    let flyout = barometer_app::flyout::Flyout::new();
    let weather_feed = flyout.feed(weather::Snapshot::default());
    flyout.register(Box::new(weather::WeatherContent::new(weather_feed.slot())));
    let cpu_feed = flyout.feed(cpu::CpuSnapshot::default());
    flyout.register(Box::new(cpu::CpuContent::new(cpu_feed.slot())));
    let gpu_feed = flyout.feed(gpu::GpuSnapshot::default());
    flyout.register(Box::new(gpu::GpuContent::new(gpu_feed.slot())));
    let memory_feed = flyout.feed(memory::MemorySnapshot::default());
    flyout.register(Box::new(memory::MemoryContent::new(memory_feed.slot())));
    let disks_feed = flyout.feed(disks::DisksSnapshot::default());
    flyout.register(Box::new(disks::DisksContent::new(disks_feed.slot())));
    let network_feed = flyout.feed(network::NetworkSnapshot::default());
    flyout.register(Box::new(network::NetworkContent::new(network_feed.slot())));
    let sensors_feed = flyout.feed(sensor_panel::SensorsSnapshot::default());
    flyout.register(Box::new(sensor_panel::SensorsContent::new(sensors_feed.slot())));
    // A stack's panel switches between its readings' module panels, so it
    // is handed the same slots those panels read. Stacks come and go in
    // settings, so theirs are registered as they are first seen.
    let module_slots = barometer_app::flyout::stack::ModuleSlots {
        cpu: cpu_feed.slot(),
        gpu: gpu_feed.slot(),
        memory: memory_feed.slot(),
        disks: disks_feed.slot(),
        network: network_feed.slot(),
        sensors: sensors_feed.slot(),
        weather: weather_feed.slot(),
    };
    let mut stack_feeds: std::collections::HashMap<
        u32,
        barometer_app::flyout::Feed<barometer_app::flyout::stack::StackSnapshot>,
    > = std::collections::HashMap::new();
    // The panels' running state: histories and rate windows accumulate
    // here, and a clone is published each tick.
    // Deques, and copied out only when somebody is looking - see
    // `cpu::remember` for the first half of that and `panels_open` for the
    // second. Between them a day of uptime with every panel closed costs
    // nothing per tick where it used to cost several megabytes of copying.
    let mut cpu_history: VecDeque<cpu::Sample> = VecDeque::new();
    let mut gpu_history: VecDeque<cpu::Sample> = VecDeque::new();
    let mut memory_history: VecDeque<cpu::Sample> = VecDeque::new();
    // Whether a panel was open on the previous pass, so that opening one can
    // pull the next sample forward rather than leaving the graph blank for
    // the rest of the second.
    let mut panels_were_open = false;
    let mut disks_snapshot = disks::DisksSnapshot::default();
    let mut network_snapshot = network::NetworkSnapshot::default();
    let mut sensors_snapshot = sensor_panel::SensorsSnapshot::default();
    // Processes are counted on a worker of their own: walking every process
    // is far too much for the strip's tick, and the panel wants it every
    // couple of seconds, not every second.
    let mut processes = barometer_core::sys::processes::Sampler::new();
    // Per-process traffic and the connection's addresses, only while the
    // panel could be looking: the TCP walk is not free, and the addresses
    // do not change between one tick and the next.
    let mut traffic = barometer_core::netinfo::TrafficSampler::new();
    let mut traffic_rates: Vec<barometer_core::netinfo::ProcessRate> = Vec::new();
    let mut traffic_at = Instant::now() - Duration::from_secs(10);
    let mut connection_at = Instant::now() - Duration::from_secs(10);
    // The public address, looked up on a thread of its own when the user
    // allows it and the panel is open, and kept for a quarter of an hour,
    // which is the macOS app's cache policy.
    let public_ip: std::sync::Arc<std::sync::Mutex<Option<barometer_core::netinfo::PublicIp>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let public_ip_busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut public_ip_at: Option<Instant> = None;
    let mut process_sample: Option<barometer_core::sys::processes::Sample> = None;
    let mut processes_at = Instant::now() - Duration::from_secs(10);
    let core_kinds = barometer_core::sys::processes::core_kinds();

    let mut reservation = reserve::Reservation::new(dpi);
    if reservation.is_none() {
        eprintln!("Could not create the tray reservation; the readout will overlay instead.");
    }

    loop {
        // Messages first, so input and repaints are never behind a sample.
        // SAFETY: a standard peek-and-dispatch pump on this thread's queue.
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    return;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        // Whatever the right-click menu asked for, acted on out here rather
        // than inside the window procedure: opening a window or shutting the
        // program down is this loop's business, and a window procedure that
        // starts doing either cannot be reasoned about.
        match strip.take_command() {
            Some(window::Command::Quit) => return,
            Some(window::Command::OpenSettings) => match settings_window.as_mut() {
                Some(open) => open.show(),
                None => {
                    settings_window = Some(settings_ui::SettingsWindow::open(
                        settings_ui::Model::from_settings(settings.clone()),
                    ));
                }
            },
            Some(window::Command::CheckForUpdates) => check_for_updates(),
            // The column order is the settings' order of enabled modules,
            // which is exactly how the strip built its columns.
            Some(window::Command::Column { index, rect }) => {
                let clicked = column_items.get(index).copied();
                if trace::on() {
                    trace::line(&format!("click column {index} -> {clicked:?} at {rect:?}"));
                }
                if let Some(item) = clicked {
                    flyout.toggle_item(item, barometer_app::flyout::Anchor { column: rect });
                }
            }
            None => {}
        }
        // What a panel asked for, acted on by the thread whose business it
        // is, the same as the menu above.
        while let Some(command) = flyout.take_command() {
            match command {
                barometer_app::flyout::Command::OpenSettings(module) => {
                    // Every module has a page of its own now, so a panel's
                    // gear lands on that module's page rather than on a
                    // general one the user then has to search.
                    let pane = settings_ui::ui::Pane::Module(module);
                    match settings_window.as_mut() {
                        Some(open) => open.show_pane(pane),
                        None => {
                            settings_window = Some(settings_ui::SettingsWindow::open_at(
                                settings_ui::Model::from_settings(settings.clone()),
                                pane,
                            ));
                        }
                    }
                }
                barometer_app::flyout::Command::Refresh(module) => {
                    if let Some(module) = modules.iter_mut().find(|m| m.id() == module) {
                        module.refresh();
                    }
                }
                barometer_app::flyout::Command::ShowLocation(id) => {
                    // The same change the Weather pane's list makes, made
                    // from the panel: the weather module re-reads its
                    // settings and fetches for the new place.
                    if settings.weather.locations.iter().any(|place| place.id == id) {
                        settings.weather.primary_location_id = Some(id);
                        model = settings_ui::Model::from_settings(settings.clone());
                        apply_to_modules(&mut modules, &settings);
                        if let Some(open) = settings_window.as_mut() {
                            open.adopt(settings.clone());
                        }
                        save_settings(store.as_mut(), &settings);
                    }
                }
            }
        }

        // The tray re-laid itself out: an icon appeared or went, the chevron
        // opened, the block moved. Waiting for the next tick leaves the
        // readout where the block *was* for up to a second - measured: with
        // the tray's first icon slid under its right end while the
        // microphone indicator came and went. TrafficMonitor moves on its
        // layout-changed message; this is the same, with the tick pulled
        // forward rather than a second placement path. Throttled, because an
        // animation is a burst of these.
        // A placement-only pass, not a sample: the modules are not read
        // again for it, only the placeholders re-measured and the readout
        // moved. Cheap enough to ride the tray through its whole animation -
        // TrafficMonitor re-places on every layout-changed message its
        // sweep posts, at the sweep's fast cadence, and this is that.
        // The tray's HWND resizes once, at the start; the icons then slide
        // for a quarter second or so with no further event. One pass at the
        // start and one at the end left the readout still for the slide in
        // between - measured as a brief overlap on the chevron side. So an
        // event opens a window, and the readout rides the tray at the fast
        // cadence until it closes.
        if window::take_tray_changed() {
            follow_until = Some(Instant::now() + FOLLOW_FOR);
        }
        let mut follow = false;
        if let Some(until) = follow_until {
            if Instant::now() >= until {
                follow_until = None;
                follow = true;
            } else if last_forced.elapsed() >= FORCE_GAP {
                last_forced = Instant::now();
                follow = true;
            }
        }
        let due = Instant::now() >= next_sample;
        if due || follow {
            if due {
                next_sample += INTERVAL;
                // Anything that stops the pump leaves the schedule in the
                // past: a right-click menu, a message box, and above all the
                // machine sleeping - Instant is QPC, which counts the hours
                // spent suspended. Advancing one interval per pass would then
                // burn the backlog off at the millisecond floor below, so
                // waking from an overnight suspend owes some thirty thousand
                // full sample passes and spends a core on them. There is
                // nothing to catch up on - only the newest reading is ever
                // shown - so a schedule that has fallen behind restarts.
                let now = Instant::now();
                if next_sample < now {
                    next_sample = now + INTERVAL;
                }

                if let Some(found) = weekly.lock().ok().and_then(|mut slot| slot.take()) {
                    weekly_running = false;
                    if let Some(version) = offer_update(&found, settings.skipped_update.as_deref()) {
                        settings.skipped_update = version;
                        save_settings(store.as_mut(), &settings);
                        if let Some(open) = settings_window.as_mut() {
                            open.adopt(settings.clone());
                        }
                    }
                }
                let due_now =
                    update::due(settings.check_for_updates, settings.last_update_check, unix_now());
                if !weekly_running && due_now {
                    weekly_running = true;
                    // Stamped and saved before the request rather than after,
                    // so a machine with no network does not ask again every
                    // second for the rest of the week.
                    settings.last_update_check = unix_now();
                    save_settings(store.as_mut(), &settings);
                    // The settings window holds its own copy and writes the
                    // whole thing back on the next change, so without this
                    // the stamp would be undone by somebody moving a slider
                    // and the check would come round again the same day.
                    if let Some(open) = settings_window.as_mut() {
                        open.adopt(settings.clone());
                    }
                    let slot = std::sync::Arc::clone(&weekly);
                    thread::spawn(move || {
                        // A failure is nobody's business: this check was not
                        // asked for, and a dialog about a network that is not
                        // there would be exactly the interruption the setting
                        // promises not to make.
                        if let Ok(found) = update::check() {
                            if let Ok(mut slot) = slot.lock() {
                                *slot = Some(found);
                            }
                        }
                    });
                }
            }

            // Explorer restarting takes the whole shell with it: the tray has
            // forgotten every placeholder, and the strip is owned by a
            // Shell_TrayWnd that no longer exists. Both are rebuilt here,
            // outside the broadcast, against a shell that has finished
            // starting.
            if tray::take_shell_restarted() {
                if let Some(reservation) = reservation.as_mut() {
                    reservation.on_shell_restarted();
                }
                // A fresh shell gets a fresh grace period: the whole tray
                // is being rebuilt and the region will be a second or two
                // behind it, which is not the failure the deadline is for.
                hold_deadline = None;
                flyout.close();
                // The old window goes first, and the order is not a
                // preference. WM_DESTROY unhooks whatever hook handles are in
                // the statics and clears STRIP; destroying the old strip after
                // creating the new one would tear down the new one's hooks and
                // forget its handle. Dropping it here runs that teardown while
                // the statics still describe the window being destroyed.
                drop(std::mem::replace(&mut strip, window::Strip::placeholder()));
                match window::Strip::create() {
                    Some(fresh) => {
                        strip = fresh;
                        strip.forget_placement();
                    }
                    // The new taskbar is not up yet. Nothing is on screen now,
                    // which is correct - there is no taskbar to sit on - and
                    // the next tick tries again.
                    None => continue,
                }
            }

            if due {
                for module in modules.iter_mut() {
                    module.sample();
                }
                // What the panels show, published whether or not one is open:
                // a feed is a slot the panel reads when it is, and publishing
                // is a clone under a lock.
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                // Every two seconds, and only while a panel that shows
                // processes could be looking - the walk is the most expensive
                // thing this loop does.
                if flyout.is_open() && processes_at.elapsed() >= Duration::from_secs(2) {
                    processes_at = Instant::now();
                    process_sample = processes.sample();
                }
                // Asked once and reused: it decides whether several of the
                // panels' more expensive fields are worth building at all.
                let panels_open = flyout.is_open();
                if panels_open && !panels_were_open {
                    // A panel has just been opened. Sample now rather than at
                    // the top of the next second, so its graph is filled in
                    // by the time the fade finishes.
                    next_sample = Instant::now();
                }
                if panels_were_open && !panels_open {
                    // The last panel has closed. The per-process figures are
                    // gathered by asking the kernel to keep extended
                    // statistics on every established connection on the
                    // machine, and that stays on for the life of each
                    // connection unless it is switched off - so a browser's
                    // several hundred sockets would go on being accounted for
                    // long after the panel that wanted them was gone.
                    barometer_core::netinfo::stop_collecting();
                }
                panels_were_open = panels_open;
                let summary = if flyout.is_open() {
                    barometer_core::sys::processes::summary()
                } else {
                    None
                };
                for module in modules.iter() {
                    match module.id() {
                        barometer_core::ModuleId::Weather => {
                            if let Some(observed) = module.weather() {
                                weather_feed.publish(weather::Snapshot {
                                    location: observed.location,
                                    locations: settings.weather.locations.clone(),
                                    units: observed.units,
                                    current: observed.current,
                                    error: observed.error,
                                    refresh_minutes: observed.refresh_minutes,
                                });
                            }
                        }
                        barometer_core::ModuleId::Cpu => {
                            let total = module.fraction();
                            if let Some(value) = total {
                                cpu::remember(&mut cpu_history, cpu::Sample { at_unix: now_unix, value });
                            }
                            let cores = process_sample
                                .as_ref()
                                .map(|s| {
                                    s.cores
                                        .iter()
                                        .enumerate()
                                        .map(|(index, load)| cpu::CoreLoad {
                                            index,
                                            kind: core_kinds.get(index).copied().unwrap_or_default(),
                                            load: *load,
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            let top = process_sample
                                .as_ref()
                                .map(|s| {
                                    s.top_by_cpu(12)
                                        .into_iter()
                                        .map(|p| cpu::ProcessLoad {
                                            pid: p.pid,
                                            name: p.name.clone(),
                                            load: p.cpu.unwrap_or(0.0),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            let split = module.cpu_split();
                            cpu_feed.publish(cpu::CpuSnapshot {
                                total,
                                load_average: module.load_average(),
                                user: split.map(|(user, _)| user),
                                system: split.map(|(_, system)| system),
                                logical_processors: Some(core_kinds.len()).filter(|n| *n > 0),
                                cores,
                                uptime_secs: summary.as_ref().map(|s| s.uptime_secs),
                                processes: summary.as_ref().map(|s| s.processes),
                                threads: summary.as_ref().map(|s| s.threads),
                                handles: summary.as_ref().map(|s| s.handles),
                                top,
                                history: history_for(panels_open, &cpu_history),
                            });
                        }
                        barometer_core::ModuleId::Gpu => {
                            let load = module.fraction();
                            if let Some(value) = load {
                                cpu::remember(&mut gpu_history, cpu::Sample { at_unix: now_unix, value });
                            }
                            let chosen = match &settings.gpu {
                                barometer_core::settings::GpuChoice::Adapter { name, .. } => Some(name.clone()),
                                barometer_core::settings::GpuChoice::Automatic => {
                                    module.gpu_adapters().first().map(|(_, name)| name.clone())
                                }
                            };
                            let mut snapshot = gpu::GpuSnapshot {
                                supported: !module.readout().unavailable || load.is_some(),
                                name: chosen,
                                load,
                                unit: settings.sensors.temperature,
                                history: history_for(panels_open, &gpu_history),
                                ..Default::default()
                            };
                            if flyout.is_open() {
                                snapshot.engines = module
                                    .gpu_engines()
                                    .into_iter()
                                    .map(|(name, load)| gpu::EngineLoad { name, load })
                                    .collect();
                                if let Some((used, total)) = module.gpu_memory() {
                                    snapshot.memory_used = Some(used);
                                    snapshot.memory_total = (total > 0).then_some(total);
                                }
                                // The clock, power and temperature are the
                                // sensor source's; the GPU module has none.
                                if let Some(sensors) =
                                    modules.iter().find(|m| m.id() == barometer_core::ModuleId::Sensors)
                                {
                                    snapshot.adopt_sensors(&sensors.sensors());
                                }
                            }
                            gpu_feed.publish(snapshot);
                        }
                        barometer_core::ModuleId::Memory => {
                            let mut snapshot = memory::MemorySnapshot::default();
                            if let Some((used, total)) = module.memory() {
                                snapshot.total = Some(total);
                                snapshot.in_use = Some(used);
                                snapshot.available = Some(total.saturating_sub(used));
                            }
                            if flyout.is_open() {
                                if let Some(lists) = barometer_core::sys::processes::memory_lists() {
                                    snapshot.with_lists(lists);
                                }
                            }
                            if let Some((used, total)) = module.page_file() {
                                snapshot.page_file_used = Some(used);
                                snapshot.page_file_total = Some(total);
                            }
                            if let Some(s) = &summary {
                                snapshot.committed = Some(s.committed);
                                snapshot.commit_limit = Some(s.commit_limit);
                                snapshot.paged_pool = Some(s.paged_pool);
                                snapshot.nonpaged_pool = Some(s.nonpaged_pool);
                            }
                            snapshot.top = process_sample
                                .as_ref()
                                .map(|s| {
                                    s.top_by_memory(12)
                                        .into_iter()
                                        .map(|p| memory::ProcessMemory {
                                            pid: p.pid,
                                            name: p.name.clone(),
                                            working_set: p.working_set,
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            if let Some(value) = snapshot.commit_fraction() {
                                cpu::remember(&mut memory_history, cpu::Sample { at_unix: now_unix, value });
                            }
                            snapshot.history = history_for(panels_open, &memory_history);
                            memory_feed.publish(snapshot);
                        }
                        barometer_core::ModuleId::Disks => {
                            disks_snapshot.observe(module.rates());
                            if flyout.is_open() {
                                // Whatever the module's own scan last saw,
                                // rather than asking again here. Asking here
                                // meant this thread - the one that paints and
                                // answers clicks - waiting out the SMB
                                // timeout for a sleeping network drive, which
                                // is the exact stall the module's scan was
                                // moved onto a thread of its own to avoid.
                                disks_snapshot.volumes = module
                                    .volumes()
                                    .into_iter()
                                    .map(|v| disks::Volume {
                                        name: v.name,
                                        mount: v.mount,
                                        total: v.total,
                                        available: v.available,
                                        removable: v.removable,
                                    })
                                    .collect();
                                disks_snapshot.devices = module
                                    .disk_devices()
                                    .into_iter()
                                    .map(|d| disks::Device {
                                        // The system's number for the disk
                                        // is the chip; the model is the
                                        // name, or the number again with
                                        // its letters where the drive will
                                        // not say.
                                        id: d.name.split(" (").next().unwrap_or(&d.name).to_string(),
                                        model: d.model.unwrap_or(d.name),
                                        read: d.read,
                                        write: d.write,
                                        read_ops: d.read_ops,
                                        write_ops: d.write_ops,
                                    })
                                    .collect();
                            }
                            disks_feed.publish(disks_snapshot.clone());
                        }
                        barometer_core::ModuleId::Network => {
                            network_snapshot.unit = settings.network_unit;
                            network_snapshot.upload_first = settings.network_upload_first;
                            network_snapshot.observe(module.rates());
                            if flyout.is_open() {
                                if connection_at.elapsed() >= Duration::from_secs(5) {
                                    connection_at = Instant::now();
                                    let found = barometer_core::netinfo::connection();
                                    network_snapshot.connection = match &found {
                                        Some(c) => network::Connection {
                                            interface: Some(c.name.clone()),
                                            is_vpn: c.is_vpn,
                                            is_primary: true,
                                            ipv4: c.ipv4.clone(),
                                            ipv6: c.ipv6.clone(),
                                            router: c.gateways.first().cloned(),
                                            dns: c.dns.clone(),
                                            errors_in: c.errors_in,
                                            errors_out: c.errors_out,
                                            ..Default::default()
                                        },
                                        None => network::Connection::default(),
                                    };
                                    network_snapshot.connection.shows_public = settings.network_shows_public_ip;
                                    network_snapshot.wifi = match found.as_ref().filter(|c| c.is_wifi) {
                                        Some(_) => barometer_core::netinfo::wifi().map(|w| network::Wifi {
                                            ssid: w.ssid,
                                            rssi: w.rssi,
                                            noise: None,
                                            channel: w.channel,
                                            band: w.band,
                                            transmit_mbps: w.transmit_mbps,
                                            security: w.security,
                                        }),
                                        None => None,
                                    };
                                }
                                if settings.network_shows_public_ip {
                                    let due = public_ip_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(900));
                                    if due && !public_ip_busy.load(std::sync::atomic::Ordering::Relaxed) {
                                        public_ip_at = Some(Instant::now());
                                        public_ip_busy.store(true, std::sync::atomic::Ordering::Relaxed);
                                        let slot = std::sync::Arc::clone(&public_ip);
                                        let busy = std::sync::Arc::clone(&public_ip_busy);
                                        std::thread::spawn(move || {
                                            let found = barometer_core::netinfo::public_ip();
                                            if let Ok(mut slot) = slot.lock() {
                                                // A lookup that failed keeps the last answer: the
                                                // address is still that until told otherwise.
                                                if found.is_some() {
                                                    *slot = found;
                                                }
                                            }
                                            busy.store(false, std::sync::atomic::Ordering::Relaxed);
                                        });
                                    }
                                    let known = public_ip.lock().ok().and_then(|slot| slot.clone());
                                    network_snapshot.connection.shows_public = true;
                                    network_snapshot.connection.public_ipv4 = known.as_ref().and_then(|k| k.ipv4.clone());
                                    network_snapshot.connection.public_ipv6 = known.and_then(|k| k.ipv6);
                                } else {
                                    // Switched off: forget the answer, as the Mac does, so
                                    // switching it back on asks afresh.
                                    if let Ok(mut slot) = public_ip.lock() {
                                        *slot = None;
                                    }
                                    public_ip_at = None;
                                    network_snapshot.connection.shows_public = false;
                                    network_snapshot.connection.public_ipv4 = None;
                                    network_snapshot.connection.public_ipv6 = None;
                                }
                                if traffic_at.elapsed() >= Duration::from_secs(2) {
                                    traffic_at = Instant::now();
                                    if let Some(rates) = traffic.sample() {
                                        traffic_rates = rates;
                                    }
                                }
                                // Names from the process walk when there has
                                // been one, else the pid stands in.
                                let names: std::collections::HashMap<u32, String> = process_sample
                                    .as_ref()
                                    .map(|s| s.processes.iter().map(|p| (p.pid, p.name.clone())).collect())
                                    .unwrap_or_default();
                                network_snapshot.processes = Some(
                                    traffic_rates
                                        .iter()
                                        .filter(|r| r.down + r.up > 0.0)
                                        .take(12)
                                        .map(|r| network::Process {
                                            pid: r.pid,
                                            name: names
                                                .get(&r.pid)
                                                .cloned()
                                                .unwrap_or_else(|| format!("PID {}", r.pid)),
                                            down: r.down,
                                            up: r.up,
                                        })
                                        .collect(),
                                );
                            }
                            network_feed.publish(network_snapshot.clone());
                        }
                        barometer_core::ModuleId::Sensors => {
                            sensors_snapshot.configure(
                                settings.sensors.temperature,
                                settings.sensors.decimal_places as usize,
                            );
                            let error = module.sensor_error();
                            sensors_snapshot.observe(&module.sensors(), error.as_ref());
                            sensors_feed.publish(sensors_snapshot.clone());
                        }
                    }
                }
            }

            let Some(bar) = taskbar::taskbar() else {
                // No taskbar to sit on - Explorer is down, or between the
                // shell dying and the new one announcing itself. The readout
                // used to stay where it was and paint over an empty desktop
                // until the shell came back.
                strip.set_visible(false);
                continue;
            };
            if !bar.edge.is_supported() {
                // The same dialog the startup path puts up, for the same
                // reason: this is a windows-subsystem binary with no
                // console, so the eprintln here went nowhere and the
                // readout simply vanished the moment the taskbar was
                // dragged to a side edge - indistinguishable from a crash.
                complain(SIDE_EDGE);
                return;
            }

            // A DPI change means the pitch the reservation measured is in the
            // wrong units. It discards it and re-seeds rather than carrying a
            // stale figure across the move.
            let current_dpi = window::taskbar_dpi();
            if current_dpi != dpi {
                dpi = current_dpi;
                if let Some(reservation) = reservation.as_mut() {
                    reservation.set_dpi(dpi);
                }
                // A panel laid out at the old scale is anchored to a column
                // that is about to move; closing it is cheaper than chasing it.
                flyout.close();
            }

            // The ladder is in DIPs and the bar is measured in pixels, so the
            // bar has to be converted before it is compared with type sizes.
            // Skipping this makes the strip a third too small at 150%, which
            // reads as a font bug rather than a unit one.
            let height_dip = bar.height() as f32 * 96.0 / dpi as f32;
            if trace::on() {
                trace::line(&format!("bar {}px dpi {dpi} -> {height_dip}dip", bar.height()));
            }
            // The strip is what the settings say it is: the modules the user
            // switched on, in the order they arranged them.
            //
            // Every module keeps running whether or not it is shown. Rebuilding
            // the list on each change would restart them, and a restarted
            // module has no previous counter to subtract from - the network
            // would report one tick of nonsense every time somebody toggled
            // anything, and the sensor helper would be respawned, which costs
            // a second and a process.
            // The strip is what the settings say it is: modules and stacks
            // in the order the user arranged them, a module's own column
            // gone while a stack that hides it is on. The cells come from the
            // same function the settings preview draws with, so the preview
            // and the strip cannot disagree about what a stack looks like.
            // Only while there is a window to draw them in - see `snapshot`.
            let for_settings = settings_window.as_ref().is_some_and(|open| open.is_open());
            let live = snapshot(&modules, height_dip, &settings, for_settings);

            // The type ladder steps down as the strip fills, so it counts what
            // is actually shown rather than every module that exists.
            let density =
                Density::choose_with_font(height_dip, model.shown_count().max(1), &font);

            let cells = settings_ui::preview::cells(&model, &live, density.two_rows);
            // Every stack has a panel, registered the first time it is seen
            // and fed on every tick after.
            for stack in &settings.stacks.stacks {
                let feed = stack_feeds.entry(stack.id).or_insert_with(|| {
                    let feed = flyout.feed(barometer_app::flyout::stack::StackSnapshot::default());
                    flyout.register(Box::new(barometer_app::flyout::stack::StackContent::new(
                        stack.id,
                        feed.slot(),
                        module_slots.clone(),
                    )));
                    feed
                });
                feed.publish(barometer_app::flyout::stack::StackSnapshot {
                    id: stack.id,
                    name: stack.display_name().to_string(),
                    entries: stack.metrics.clone(),
                    sensors: live.sensors.clone(),
                });
            }
            column_items.clear();
            let mut columns: Vec<window::Column> = Vec::new();
            for cell in &cells {
                let owner = cell.item;
                for column in &cell.columns {
                    column_items.push(owner);
                    columns.push(window::Column {
                        top: column.top.clone(),
                        bottom: column.bottom.clone(),
                        reserved: column.reserved.clone(),
                        badge: column.badge,
                        heading: column.label_top,
                        top_label: column.top_label.clone(),
                        bottom_label: column.bottom_label.clone(),
                    });
                }
            }

            // The model first: its measured width is what gets reserved, so
            // the space asked for and the space drawn are one number.
            // Anything the settings window changed since the last tick. Applied
            // here rather than from the window's own thread, because this is
            // the thread that owns the readout.
            if let Some(open) = settings_window.as_ref() {
                // Live readings, the machine's sensors and what the sensor
                // source is doing. Only while the window is up: building it
                // costs a clone of every sensor, and nothing reads it when
                // there is nowhere to show it.
                if open.is_open() {
                    open.publish(live.clone());
                }
                if let Some(changed) = open.take_changes() {
                    settings = changed.to_settings();
                    model = settings_ui::Model::from_settings(settings.clone());
                    font = settings.font.clone();
                    apply_to_modules(&mut modules, &settings);
                    save_settings(store.as_mut(), &settings);
                }
            }

            let needed = strip.update(window::StripModel {
                columns,
                gap_dip: settings.column_gap_dip,
                padding_dip: settings.padding_dip,
                monochrome: false,
                text_dip: density.text_dip,
                two_rows: density.two_rows,
                font_family: font.family.clone(),
                font_weight: font.weight.dwrite_weight(),
                heading_font_weight: font.heading_weight.dwrite_weight(),
            });

            let trace = trace::on();
            let mut placed = false;
            if let Some(reservation) = reservation.as_mut() {
                reservation.set_width(needed);
                let region = reservation.region();
                if trace {
                    trace::line(&format!(
                        "need {needed}px  held {}  seen {}  slot {}  region {},{}-{},{} ({}px)  obstructed {}",
                        reservation.held(),
                        reservation.seen(),
                        reservation.slot_width(),
                        region.left,
                        region.top,
                        region.right,
                        region.bottom,
                        region.right - region.left,
                        reservation.is_obstructed()
                    ));
                }

                if reservation.is_obstructed() {
                    // Somebody dragged an icon in among the placeholders. Get
                    // out of the way rather than covering it; the readout comes
                    // back on its own once the icon leaves.
                    if trace {
                        trace::line("  HIDING: obstructed");
                    }
                    strip.set_visible(false);
                    placed = true;
                } else if !region.is_empty() && region.right - region.left >= needed {
                    // Screen coordinates throughout, because the readout is a
                    // window over the taskbar rather than one living inside
                    // it. Horizontal extent from the reserved region, vertical
                    // from the taskbar itself: the icon rectangles are inset
                    // within the bar by a margin the shell owns and varies
                    // with the bar's size, so taking height from them would
                    // make the readout a different height than the bar.
                    //
                    // The whole reserved region, not the width the content
                    // happens to need this tick. The content width moves by
                    // a few pixels as digits change - 385, 388, 391 - and a
                    // window sized to it and centered moves its left edge
                    // with every change: the readout visibly creeps sideways
                    // while nothing else in the tray is moving. The region is
                    // our own placeholders and nothing else; filling it is
                    // exactly right, and the columns are laid out from its
                    // left edge, which only moves when the tray does.
                    //
                    // Inset from the left by a fixed amount when the region
                    // has the room. The chevron is the left neighbor, and
                    // when the tray contracts - the microphone indicator
                    // leaving - it slides right by a slot into whatever is
                    // there; the readout follows within a few frames, and
                    // the inset is what those frames overlap instead of the
                    // digits. Fixed rather than "whatever is left over", so
                    // it never moves with the content width.
                    let width = region.right - region.left;
                    let inset = if width - LEFT_INSET >= needed { LEFT_INSET } else { 0 };
                    strip.place(region.left + inset, bar.rect.top, width - inset, bar.height());
                    strip.set_visible(true);
                    if trace {
                        eprintln!("  {}", window::describe(strip.handle()));
                    }
                    placed = true;
                    // A region arrived, so the clock is not running against
                    // anything. Rearmed from here, so a reservation that is
                    // lost later gets the same grace as one at startup.
                    hold_deadline = None;
                } else if *hold_deadline.get_or_insert_with(|| Instant::now() + HOLD_TIMEOUT)
                    > Instant::now()
                {
                    // The region is still being built. Hidden rather than
                    // placed the ordinary way, because every placeholder that
                    // lands widens the tray and drags the ordinary position
                    // left with it - so placing now means appearing on top of
                    // the task buttons and then jittering leftwards into the
                    // reserved block. Better slightly late than in the wrong
                    // place.
                    if trace {
                        trace::line("  HIDING: waiting for the reserved region");
                    }
                    strip.set_visible(false);
                    placed = true;
                }
            }

            // Past the deadline, or with no reservation at all: placed to the
            // left of the notification area, the way it was before any of this
            // existed.
            //
            // This overlays the task buttons and is not good. It was taken out
            // once for exactly that reason, and that was the wrong trade: it
            // only happens when the reservation has genuinely failed, and the
            // alternative is a readout that is simply never there, which is
            // indistinguishable from the program being broken. Ugly and
            // present beats absent, and the Sensors pane still explains why.
            if !placed {
                match tray::notify_rect() {
                    // Two pixels keep the last column off the tray's own edge.
                    Some(notify) => {
                        strip.place(
                            notify.left - needed + 2,
                            bar.rect.top,
                            needed,
                            bar.height(),
                        );
                        strip.set_visible(true);
                    }
                    // No tray to measure against means the shell is mid
                    // restart; the next tick finds it.
                    None => strip.set_visible(false),
                }
            }
        }

        // Wait for a message or the next sample, whichever comes first, rather
        // than sleeping through input.
        let wake = follow_until.map_or(next_sample, |_| next_sample.min(last_forced + FORCE_GAP));
        let wait = wake.saturating_duration_since(Instant::now());
        let millis = wait.max(Duration::from_millis(1)).as_millis().min(1000) as u32;
        // SAFETY: no handles, so this waits purely on the message queue.
        unsafe {
            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, millis, QS_ALLINPUT);
        }
    }
}

/// Adds placeholder icons and reports what happens to the tray.
///
/// This is the experiment the whole mechanism rests on: if the notification
/// area does not get wider, nothing else in the strip can work.
fn run_reserve_probe(count: usize) {
    use std::thread::sleep;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};

    fn tray_width() -> Option<i32> {
        // SAFETY: class-name lookups; the rect is written only on success.
        unsafe {
            let taskbar = FindWindowW(tray::wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            if taskbar.is_null() {
                return None;
            }
            let notify = FindWindowExW(
                taskbar,
                std::ptr::null_mut(),
                tray::wide("TrayNotifyWnd").as_ptr(),
                std::ptr::null(),
            );
            if notify.is_null() {
                return None;
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(notify, &mut rect) == 0 {
                return None;
            }
            Some(rect.right - rect.left)
        }
    }

    let purged = tray::purge_orphans();
    if purged > 0 {
        println!("Cleared {purged} orphaned entries from a previous run.");
    }

    let owner = tray::create_owner_window();
    if owner.is_null() {
        eprintln!("Could not create the owner window.");
        return;
    }
    let icon = tray::blank_icon();
    if icon.is_null() {
        eprintln!("Could not create the placeholder icon.");
        return;
    }

    let before = tray_width().unwrap_or(-1);
    println!("Tray width before: {before}px");

    let guids: Vec<spacer::Guid> =
        (0..count).map(|index| spacer::Guid::from_u128(spacer::icon_guid(0, index))).collect();

    let added = guids.iter().filter(|guid| tray::add_icon(owner, icon, **guid)).count();
    println!("Added {added} of {count} placeholders.");

    // The shell writes each entry asynchronously, so promotion is retried
    // rather than attempted once. Until an icon is promoted it sits in the
    // overflow flyout and reserves nothing at all.
    let mut promoted = 0;
    for attempt in 1..=10 {
        promoted = guids.iter().filter(|guid| tray::promote(**guid)).count();
        if promoted == added {
            println!("Promoted {promoted} on attempt {attempt}.");
            break;
        }
        sleep(Duration::from_millis(300));
    }
    if promoted != added {
        println!("Promoted {promoted} of {added} after retrying.");
    }

    // Re-add so the promotion takes effect.
    for guid in &guids {
        tray::remove_icon(owner, *guid);
        tray::add_icon(owner, icon, *guid);
    }
    sleep(Duration::from_millis(1200));

    // Where did they actually land? This is what the strip needs in order to
    // place itself, and it is the question UI Automation exists to answer.
    let mut region = spacer::Rect::default();
    let mut located = 0;
    for guid in &guids {
        if let Some(rect) = tray::icon_rect(owner, *guid) {
            located += 1;
            region = region.union(rect);
        }
    }
    println!(
        "Located {located} of {added} by rect; region = {},{} to {},{} ({}px wide)",
        region.left,
        region.top,
        region.right,
        region.bottom,
        region.right - region.left
    );

    let after = tray_width().unwrap_or(-1);
    println!("Tray width after:  {after}px  (gained {}px)", after - before);
    if added > 0 && after > before {
        println!("Per slot: {}px", (after - before) / added as i32);
    }

    println!("Holding for 5 seconds so it can be seen, then cleaning up.");
    sleep(Duration::from_secs(5));

    for guid in &guids {
        tray::remove_icon(owner, *guid);
    }
    sleep(Duration::from_millis(800));
    println!("Tray width restored: {}px", tray_width().unwrap_or(-1));
}

/// Locates the machine, searches, and reads the weather, printing each step.
///
/// The three calls in one place, so a change at the service shows up here
/// rather than as a blank column on the taskbar.
fn run_weather_probe() {
    use barometer_core::weather::{client, models::WeatherUnits};

    println!("Locating by IP address...");
    let here = match client::locate_by_ip() {
        Ok(location) => {
            println!("  {} ({}, {})", location.display_name(), location.latitude, location.longitude);
            location
        }
        Err(why) => {
            println!("  failed: {why}");
            return;
        }
    };

    println!("Searching for \"{}\"...", here.name);
    match client::search(&here.name) {
        Ok(found) => {
            for location in found.iter().take(3) {
                println!("  {}", location.display_name());
            }
            if found.is_empty() {
                println!("  nothing matched");
            }
        }
        Err(why) => println!("  failed: {why}"),
    }

    println!("Reading current conditions...");
    match client::fetch(&here, WeatherUnits::IMPERIAL) {
        Ok(now) => {
            println!("  temperature {:?}", now.temperature);
            println!("  feels like {:?}", now.apparent_temperature);
            println!("  humidity {:?}", now.relative_humidity);
            println!("  wind {:?} at {:?}", now.wind_speed, now.wind_direction);
            println!("  pressure (hPa) {:?}", now.pressure_hectopascals);
            println!("  weather code {:?}", now.weather_code);
            println!("  daytime {}", now.is_day);
        }
        Err(why) => println!("  failed: {why}"),
    }
}
/// Draws all fourteen marks to a bitmap file, for looking at them.
///
/// The taskbar shows one mark at a time, at whatever the weather happens to be
/// doing, which is a hopeless way to judge a set. This lays the whole set out
/// at the size it will really appear and again at four times that, so the
/// pairs that are easy to confuse are side by side.
fn run_marks_sheet(path: &std::path::Path) {
    use barometer_core::weather::badge::Condition;
    use barometer_core::weather::glyph;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, CreateSolidBrush, DeleteDC,
        DeleteObject, FillRect, SelectObject, SetBkMode, SetTextColor, TextOutW, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, FW_NORMAL, LOGFONTW, TRANSPARENT,
    };

    const CELL_W: i32 = 260;
    const CELL_H: i32 = 96;
    const GROUND: u32 = 0x0020_1C1A;
    const SMALL: i32 = 15;
    const LARGE: i32 = 60;

    let rows = Condition::ALL.len() as i32;
    let width = CELL_W;
    let height = CELL_H * rows;

    // SAFETY: a top-down 32-bit DIB whose bits we own, drawn into through a
    // memory DC and written out by hand afterwards.
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB as u32;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap =
            CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            eprintln!("Could not create the bitmap.");
            DeleteDC(dc);
            return;
        }
        let previous_bitmap = SelectObject(dc, bitmap as _);

        let ground = CreateSolidBrush(GROUND);
        FillRect(dc, &RECT { left: 0, top: 0, right: width, bottom: height }, ground);
        DeleteObject(ground as _);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00F2_F2F2);

        let icon_font = |px: i32| {
            let mut logical: LOGFONTW = std::mem::zeroed();
            logical.lfHeight = px;
            logical.lfWeight = FW_NORMAL as i32;
            let family: Vec<u16> = glyph::FAMILY.encode_utf16().collect();
            for (index, unit) in family.iter().take(logical.lfFaceName.len()).enumerate() {
                logical.lfFaceName[index] = *unit;
            }
            CreateFontIndirectW(&logical)
        };

        for (index, condition) in Condition::ALL.iter().enumerate() {
            let mark = glyph::composed(*condition);
            let y = (index as i32) * CELL_H;

            // Both sizes, because a mark that survives at four times life size
            // and dissolves at fifteen pixels has not survived.
            for (size, x) in [(SMALL, 40), (LARGE, 140)] {
                let font = icon_font(size);
                let previous = SelectObject(dc, font as _);
                let base = [mark.base as u16];
                let top = y + (CELL_H - size) / 2;
                TextOutW(dc, x, top, base.as_ptr(), 1);

                if let Some(accent) = mark.accent {
                    let accent_px = ((size as f32) * accent.scale).round().max(1.0) as i32;
                    let accent_font = icon_font(accent_px);
                    SelectObject(dc, accent_font as _);
                    let glyphs = [accent.glyph as u16];
                    TextOutW(
                        dc,
                        x + ((size as f32) * accent.dx).round() as i32,
                        top + ((size as f32) * accent.dy).round() as i32,
                        glyphs.as_ptr(),
                        1,
                    );
                    SelectObject(dc, previous);
                    DeleteObject(accent_font as _);
                } else {
                    SelectObject(dc, previous);
                }
                DeleteObject(font as _);
            }
        }

        SelectObject(dc, previous_bitmap);

        let pixels = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
        let mut file: Vec<u8> = Vec::with_capacity(pixels.len() + 54);
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&(54u32 + pixels.len() as u32).to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&width.to_le_bytes());
        file.extend_from_slice(&(-height).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        file.extend_from_slice(&[0u8; 16]);
        file.extend_from_slice(pixels);
        match std::fs::write(path, &file) {
            Ok(()) => println!("Wrote {}", path.display()),
            Err(why) => eprintln!("Could not write {}: {why}", path.display()),
        }

        DeleteObject(bitmap as _);
        DeleteDC(dc);
    }
}


/// Asks GitHub whether there is a newer release, and says what it found.
///
/// Only ever on request. An update check that runs on its own and interrupts
/// somebody is the thing this menu item exists to avoid.
///
/// On a thread of its own, because the menu item that starts it is on the
/// strip's message pump: a WinHTTP round trip to a network that never answers
/// stopped the readout sampling, repainting and following the tray for the
/// whole timeout, and then again for as long as the box stood. The settings
/// window's own check has always been threaded; this one was not.
fn check_for_updates() {
    thread::spawn(ask_about_updates);
}

fn ask_about_updates() {
    match update::check() {
        Ok(update::Outcome::UpToDate(version)) => {
            complain(&format!("Barometer {version} is the latest release."))
        }
        Ok(update::Outcome::Newer(release)) => complain(&format!(
            "Barometer {} is available. You are running {}.\n\n{}",
            release.version,
            update::Version::running(),
            release.notes
        )),
        Err(why) => complain(&format!("Could not check for updates.\n\n{why}")),
    }
}


/// Abandons the current block of tray identities and moves to the next.
///
/// The settings pane will have this as a button; this is the same thing from a
/// command line, for a machine that is already broken enough that opening the
/// settings pane is not on offer.
///
/// It exists because a block of identities can die permanently and there is no
/// way to ask the shell whether one still works - only to try it. Two things
/// kill a block: deleting its NotifyIconSettings entries, and moving the
/// executable, because a GUID registered through NIF_GUID is bound to the path
/// that registered it. Neither recovers on its own and neither is the user's
/// fault.
fn run_repair() {
    let previous = barometer_app::reserve_block::stored_block();
    let next = barometer_app::reserve_block::repair();
    println!("Tray identities: block {previous} abandoned, now using block {next}.");
    println!();
    println!("Barometer will claim fresh space in the notification area the next");
    println!("time it starts. Nothing was deleted: the old block's registry");
    println!("entries are left alone deliberately, because deleting one is what");
    println!("makes an identity unusable in the first place.");
}


/// Pushes the settings that a module owns into the module itself.
///
/// Only the ones a module has to act on. Which modules appear and in what
/// order is decided when the columns are built, because that costs nothing and
/// keeps every module running whether or not it is shown - a module rebuilt on
/// each change would lose the previous counter it subtracts from, and report
/// one tick of nonsense every time somebody toggled anything.
fn apply_to_modules(modules: &mut [Box<dyn Module>], settings: &barometer_core::store::Settings) {
    for module in modules.iter_mut() {
        module.configure(settings);
    }
}
