# Barometer for Windows — working rules

## The core rule: this is a port, not a rewrite

Barometer already exists and is finished software:
**[mackid1993/Barometer](https://github.com/mackid1993/Barometer)** — Swift, GPL-3.0, macOS menu bar.

This repository ports that app to Rust on Win32. It is not a new system monitor that
happens to share a name.

**Before writing any module, read its Swift implementation.** Not the README, the source:

```
Sources/MenuBarStatsCore/Modules/<Name>/<Name>Monitor.swift
Sources/MenuBarStatsCore/Modules/<Name>/<Name>Settings.swift
Sources/MenuBarStatsCore/Engine/*.swift
```

```sh
gh api repos/mackid1993/Barometer/contents/<path> --jq .content | base64 -d
```

The Swift is the specification. Where this port differs from it, the difference is a
decision that gets written down with its reason — in a comment at the site, and here if
it affects the shape of the app. Silent divergence is the failure mode to avoid: it is
how two apps with one name end up behaving differently for no reason anyone remembers.

The Swift source carries design decisions the README does not. One example, from
`NetworkMonitor.swift`: the default network reading sums every active interface **except
loopback and VPN tunnels**, because a tunnel's bytes also traverse the physical device
beneath it and would otherwise be counted twice — while the interface *identity* shown
stays the primary route's. That rule is invisible from the outside and would have been
got wrong by reasoning from first principles.

## Module status

| Module | Swift source read | Ported | Windows notes |
| --- | --- | --- | --- |
| Network | partially | ported | `GetIfTable2`, less NDIS filter layers and tunnels. Per-process through the TCP table's EStats; interface selection in settings |
| CPU | partially | ported | `GetSystemTimes` plus `NtQuerySystemInformation` for per-core, core classes and processes. Load average from `\System\Processor Queue Length`, which has no `getloadavg` to port |
| Memory | partially | ported | `GlobalMemoryStatusEx`, with the kernel's page lists for the breakdown and `GetPerformanceInfo` for commit. Pressure is commit against its limit, not the Mac's kernel figure |
| GPU | partially | ported | PDH `GPU Engine` and `GPU Adapter Memory`; adapters through D3DKMT. Clock, power and temperature come from the sensor source, which is where Windows keeps them |
| Disk | partially | ported | PDH `PhysicalDisk`, volume enumeration, model names through `IOCTL_STORAGE_QUERY_PROPERTY` |
| Sensors | partially | ported | **Needs ring-0.** See below |
| ~~Battery~~ | n/a | **dropped** | Windows already shows battery in the tray with time remaining. Battery *sensors* still arrive through Sensors |
| Weather | partially | ported | Open-Meteo, same source and units as macOS, plus its air-quality service |
| ~~Time~~ | n/a | **dropped** | Windows already shows a clock on the taskbar; a second one spends strip width duplicating the shell |

## Modules and stacks are both real, and they are not the same thing

Every module has its own `ModuleSettings` and its own item on the strip, with a `mode`
string its renderer interprets — CPU alone understands a percentage, a label over a value,
a history graph, per-core bars, and an icon with a value. **Modules are not stacks.**

A **stack** is an addition on top of that: it composes *readings* (`StackMetric`) from
several modules into one item, and can set `hidesSourceItems` to take those modules'
individual items off the strip. That is the space saving, and it is a choice, not the
architecture.

So a valid configuration is any mix: an individual CPU module in graph mode, an individual
Memory module showing a percentage, and a stack carrying GPU, disk and network readings
with its sources hidden.

The valid `mode` values live with each module's renderer in the Swift, not in a shared
enum. Read them per module.

## Packaging and updates

Follow Clicker and Yamato, which already do this:

- **Inno Setup**, `installer/barometer.iss`, alongside `build.ps1` / `version.ps1`.
- **In-app updater**, as in `Clicker/src/update.rs` and `Yamato/src/update.rs` +
  `update_box.rs`, checking GitHub releases.
- `winresource` in `build.rs` to embed the icon and version into the executable, or the
  binary shows the generic default in Explorer, the taskbar and Alt+Tab.
- The app icon is the macOS one, `assets/AppIcon.png`, 1024x1024. `scripts\make-icon.ps1`
  packs it into the multi-size `assets/barometer.ico` the build embeds, measuring and
  cropping the macOS squircle's transparent margin - which reads small at 16px - and
  resampling every size from the 1024 rather than from the size above. `build.ps1` re-runs
  it whenever the artwork is newer than the icon.

## The one architectural divergence

macOS gives Barometer **N independent menu bar items**, one per module, individually
draggable. **Windows has no equivalent** — the taskbar exposes no multi-item API.

So on Windows every enabled module is laid out inside **one strip**, which claims its
space by reserving room in the system tray so Windows repacks the task buttons aside.
On macOS "Combined" is one display option among many; here it is the only mode.

Everything downstream follows from this: module separation is a layout problem inside one
control, and a detail panel is a flyout anchored under the module the user clicked, not
under the strip.

## Taskbar constraints

Windows 11 26H2 gave the taskbar back two things it had lost, and both are core rules here.

### Top and bottom only. Left and right are refused.

The strip claims horizontal room by widening the notification area, which makes Windows
repack the task buttons aside. On a taskbar docked **left or right** the tray is a
vertical grid: widening it frees no horizontal space, and the readout has nowhere to go.
This is not a rendering problem to solve, it is geometry.

So when the taskbar is on a side edge, Barometer **tells the user why and shuts itself
down.** It does not run degraded, it does not draw a squeezed readout, and it does not sit
there silently doing nothing — an app that is running but invisible is worse than one that
explained itself and left.

The same applies while running: if the taskbar is moved to a side edge during a session,
warn and exit rather than trying to reflow.

- Edge and rectangle come from `SHAppBarMessage(ABM_GETTASKBARPOS)`, whose `uEdge` is one
  of `ABE_LEFT`, `ABE_TOP`, `ABE_RIGHT`, `ABE_BOTTOM`.
- Changes arrive as `ABN_POSCHANGED` on a registered appbar callback. Re-check the edge
  there; do not assume the edge found at startup is still true.

### The taskbar can be small again

26H2 restored the Windows 10-style small taskbar. Height is therefore **measured, never
assumed**, and the strip has to lay out at whatever it gets:

- Two-row items (label over value) only when the height actually admits two legible rows
  at the current DPI. Otherwise fall back to one row.
- The Mac app's text-size ladder, which steps type down as more widgets are enabled, is
  not ported. The strip draws at one size the user sets, 9 DIP by default, and only the row
  count follows the bar's height; `Density` in `barometer-core/src/taskbar.rs` says why.
- Recompute on `ABN_POSCHANGED`, on `WM_DPICHANGED`, and on display changes. Nothing about
  taskbar geometry is stable for the life of the process.

### The readout has to be a layered popup the taskbar owns

This cost an afternoon and is invisible from the code, so it is written down.

Windows 11's taskbar hosts its own XAML surface - a
`Windows.UI.Composition.DesktopWindowContentBridge` child of `Shell_TrayWnd`. A window
created with `WS_CHILD` under the taskbar joins the ordinary child z-order *underneath*
that surface and is composited away. Everything else succeeds: the space is reserved,
`GetWindowRect` agrees the window is exactly where it should be, `IsWindowVisible` is true,
`WM_PAINT` runs and the paint counter climbs. Nothing appears. There is no error anywhere
to notice.

Three things together make it visible, and all three are required:

1. **Create it as `WS_POPUP` with the taskbar as its *owner*** - the `hWndParent` argument
   to `CreateWindowExW` for a popup - not as a `WS_CHILD` of it. There is no `SetParent`
   anywhere in the tree: an earlier draft reparented, and being inside the taskbar's child
   z-order is exactly the thing that makes the window disappear.
2. **`WS_EX_LAYERED`, set at creation.** A layered window gets a surface of its own that
   DWM composites. It has to be in the style passed to `CreateWindowExW`: adding it later
   with `SetWindowLongPtrW` leaves `UpdateLayeredWindow` failing with
   `ERROR_INVALID_PARAMETER` and nothing on screen.
3. **Re-assert `HWND_TOPMOST` when the taskbar rises above it.** The shell raises
   `Shell_TrayWnd` whenever anything in the tray is clicked - the overflow chevron, any
   icon - and the readout goes under it for as long as that lasts. `window.rs` hooks
   `EVENT_SYSTEM_FOREGROUND` and `EVENT_OBJECT_REORDER` and restacks when
   `taskbar_is_above` says so.

This is also how TrafficMonitor does it, which is the only reason it was findable.

The background is per-pixel alpha, not a color key. `UpdateLayeredWindow` with
`AC_SRC_ALPHA` composites the readout over whatever the taskbar is showing, so nothing has
to be keyed out and no color is forbidden. The parts of the strip that draw nothing are
written at `BACKGROUND_ALPHA = 6` rather than 0: a fully transparent pixel is
hit-transparent, and the tray icons underneath would take the mouse. GDI writes no alpha
of its own, so `colorize` derives coverage from `max(r, g, b)` and premultiplies - see
`window.rs`.

Text color follows `SystemUsesLightTheme`, not `AppsUseLightTheme`. They are separate
switches and the readout lives on the taskbar, so it follows the taskbar's.

### Size is measured at both ends, in DIPs

The width the strip asks the reservation for is the width its columns actually need in the
font that will draw them, measured on the window's own DC. An estimate from character
counts is wrong in both directions and both are visible: too small clips the rightmost
column against the tray, too large leaves a strip of dead taskbar.

Type size is a **fraction of the bar's height in DIPs**, not a table of pixel thresholds.
The trap: the small 26H2 taskbar at 150% is 48 physical pixels, which is exactly what the
large taskbar is at 100%. Reading those as the same bar gets both wrong - it is 32 DIP in
one case and 48 in the other. Convert with the taskbar monitor's DPI before comparing
anything to a type size.

Repaints and window moves happen **only when something changed**. Values on a taskbar hold
still for seconds at a time; a timer that repaints regardless is wasted work and, over a
translucent taskbar, a visible flicker.

### Why any of this is necessary

Windows 10 supported deskbands, so it has every taskbar monitor anyone could
want. Windows 11 removed deskband support and took all of them with it. There is
no supported way to put anything on a Windows 11 taskbar, which is why this
program claims space by a trick rather than by asking, and why every piece of
that trick is written down below rather than left to be rediscovered.

It is also why Windows 10 is not a target and should not become one. Nobody
there needs this, and supporting it would mean a second placement
implementation - which is exactly what TrafficMonitor carries, in
`ClassicalTaskbarDlg` beside `Win11TaskbarDlg`.

### How the strip gets space of its own

This is the one part carried over from the author's TrafficMonitor fork, and it is his own
work. Windows 11 lays the taskbar out in two columns: the task-button strip's right edge
*is* the notification area's left edge. Nothing can be inserted between them, and anything
drawn over the buttons is drawn over somebody's window button. So Barometer does not ask
for room - it **takes** it, by registering transparent placeholder icons in the
notification area. Widening the tray makes Windows repack the buttons aside on its own,
and the readout then sits on the space that opened up.

Four behaviors came out of that fork the hard way. Each one is load-bearing:

- **Promotion is retried, not attempted once.** The shell writes each icon's registry entry
  asynchronously. An un-promoted icon sits in the overflow flyout and reserves nothing at
  all, so `IsPromoted` is set and the icon re-added until it takes.
- **The pitch is measured, not assumed.** 42px per slot at 100% is the observed figure, but
  it is a seed. `Shell_NotifyIconGetRect` says where the icons really landed, and a
  measurement always beats the estimate, which runs high.
- **Identities rotate when a batch goes dead.** Delete a `NotifyIconSettings` entry and the
  shell keeps recognizing that GUID but will never write its entry again: the icon is
  permanently unshowable. It does not recover, so after a grace period the whole batch is
  abandoned for the next block of 64. For the same reason, **never delete those registry
  entries on exit** - that is what kills a batch.
- **Obstruction hides the readout.** Drag another program's icon in among the placeholders
  and the readout would cover it, leaving it invisible and unclickable. The readout hides,
  and comes back on its own when the icon leaves. Copy this faithfully; it is also what
  keeps the two from fighting over the same pixels. *How* it is detected has changed: the
  span tell described here read a half-finished measurement as an intrusion and hid the
  readout for as long as a flyout stayed open, so `taskbar_buttons.rs` asks UI Automation
  what is actually in the region instead - see HANDOFF §3.

**Explorer restarting** takes the shell's copy of every placeholder with it, and takes the
strip too, since the taskbar owns it. The `TaskbarCreated` broadcast
handler **sets a flag and nothing else** - never call back into the shell from inside the
handling of the shell's own broadcast, because it is half built and will either fail
silently or wedge. On the next tick, **clear the local record of held icons** before
rebuilding. That is the trap: the icons are gone from Explorer but still listed locally, so
the wanted count and the held count agree, nothing is ever re-added, and the reservation
never returns for the life of the process. The broadcast also has to be let through
`ChangeWindowMessageFilterEx`, or it is dropped by UIPI whenever the app runs elevated.

**WinEvent hooks, after all.** This section used to say Barometer polled instead of hooking,
and polling alone was not good enough: at one tick a second the readout sat where the tray
block *was* for up to a second every time an icon came or went - measured with the dictation
microphone, which appears and disappears constantly. There are now four hooks, three on the
strip's own thread (`window.rs`: foreground, reorder, and Explorer-scoped location changes,
the last of which starts a burst of 60 ms placement-only ticks) and one on the taskbar's
thread for the UI Automation sweep (`taskbar_buttons.rs`). Each handle is kept and released
on `WM_DESTROY`; the shell coming back rebuilds them, which is the cost the original C++ paid
too.

### Stage the helper from `publish`, never from the build output beside it

`dotnet publish` used to leave `barometer-sensors.exe` beside `MonoPosixHelper.dll` and
`libMonoPosixHelper.dll` - native, unbundlable, and not optional: copy the exe alone and it
exits immediately, with `greeting: EOF while parsing a value at line 1 column 0` as the only
symptom on this side. Since the LibreHardwareMonitor reference became
`ExcludeAssets="runtime"` those two are no longer in `publish` at all, only in the build
output next to it, and `publish` really is one file. The rule is unchanged and is the reason
that difference is invisible: **ship the whole `publish` folder** - `build.ps1` stages the
exe and either native library that is there, and `barometer.iss` packages the same three -
and never stage from the build output, whose exe is not self-contained.

## Sensors: orchestrate LibreHardwareMonitor, do not ship it

Windows exposes no temperature, fan or voltage API. `MSAcpi_ThermalZoneTemperature` is ACPI
thermal zones, frequently unimplemented and often a pinned constant. Real readings live
behind SuperIO over LPC, CPU MSRs, SMBus, and vendor GPU APIs. All ring-0 or vendor SDK.

**Decision: the user installs LibreHardwareMonitor themselves; Barometer orchestrates it.**

LHM already enumerates NVIDIA (NVML), AMD (ADL), Intel GPUs, CPU packages across Intel and
three AMD families, and the Nuvoton/ITE/Fintek SuperIO chips. Reimplementing that is most
of what LHM *is*. It is not shipped with Barometer, so there is no MPL redistribution
obligation and no 70 MB of .NET in the installer — only integration and credit.

### The integration surface

**Superseded, and kept for the record.** Nothing talks to LHM over HTTP today: `helper/`
loads `LibreHardwareMonitorLib` in process and answers one line at a time, and
`lhm_install.rs` fetches the library itself rather than driving somebody's running copy.
`barometer-core/src/sensors/lhm.rs` still implements the client below and is wired to
nothing; it is what a second source would be built from. The decision one heading up - read
it, never ship it - is unchanged.

LHM's remote interface is its HTTP server (`Utilities/HttpServer.cs`):

| Endpoint | Use |
| --- | --- |
| `GET /data.json` | The full hardware/sensor tree. This is what we read. |
| `GET /metrics` | Prometheus format, flatter. |
| `GET /Sensor?action=Get&id=...` | One sensor. |
| `POST /Sensor?action=Set&id=...&value=` | Sets a control. **We never call this** - Barometer reads, it does not drive fans. |

Default port 8085, configurable, with optional basic auth (username + SHA-256 password).
Configured in LHM under Options, and persisted by `PersistentSettings.cs` into
`LibreHardwareMonitor.config` beside its executable.

The `root\LibreHardwareMonitor` WMI namespace in LHM's `TestScripts/basicwmi.py` is a
stale OpenHardwareMonitor leftover. There is no WMI provider in the tree. Do not build on it.

### What orchestration means here

**Superseded with the section above.** What shipped is narrower and quieter: the Sensors
pane offers to download the pinned release into `%LOCALAPPDATA%`, the helper loads the
library from there or from wherever the user says, and no other program is started,
configured or written to. The list below is the plan it replaced.

1. **Detect** an installed LHM. It is frequently run portable from a zip, so registry
   detection alone is not enough: fall back to common paths and to a user-chosen path.
2. **Configure** the web server on a known port, or walk the user through enabling it.
   Writing another application's config file is only safe while that application is not
   running, and it is their file - treat it as a considered action, not a silent one.
3. **Run** it if it is not running. LHM needs **administrator rights** for most sensors,
   because that is what its driver requires, so either the user sets it to start elevated
   at login or they see a UAC prompt. This is the main friction of this approach and the
   Sensors pane has to be honest about it.
4. **Read** `/data.json` on the sampling interval.
5. **Keep it current** by watching LHM's GitHub releases and telling the user when a newer
   version exists.

### Credit and licenses

Barometer distributes none of it, but still owes credit, in `NOTICE.md` and the About pane:

- LibreHardwareMonitor, MPL-2.0, linked to its repository.
- PawnIO, GPL-2.0, which LHM installs itself for ring-0 access.
- The PawnIO modules LHM ships, LGPL-2.1.

### If a sensor source is absent

The Sensors module is unavailable, and says so calmly with a way to fix it. A fresh install
with no LHM is the *normal* first-run state, not an error, and must never be colored or
worded as one.

## Antivirus and code signing

**Barometer is entirely user-mode. It contains no driver, signs no driver, and installs
no driver.** Ring-0 access, where it happens at all, belongs to PawnIO, which is a third
party's already-signed driver installed by LibreHardwareMonitor's own installer. We never
load it and never talk to it.

That is what keeps this project off the expensive path: **EV certificates are required for
kernel driver signing, and we do not sign a driver.** A standard OV code-signing
certificate is enough for Authenticode, and shipping unsigned while SmartScreen reputation
accumulates is a viable open-source route. Nothing in this architecture forces EV.

### Rules that follow, and that the code has to respect

- **Never bundle WinRing0.** It is the old OpenHardwareMonitor driver, it has published
  vulnerabilities, and it is flagged on sight by Defender and most engines. LibreHardware-
  Monitor moved to PawnIO partly for this reason. If a dependency drags it in, that
  dependency is the problem.
- **Do not pack, compress or obfuscate the binaries.** Packers are the single largest
  heuristic trigger in the business, and a stripped Rust binary needs none of it.
- **No RWX allocations, no writing into another process, no injected threads, no
  installed services.** Nothing in this app needs any of them. The tray reservation claims
  space through documented shell APIs and reads positions through UI Automation; it
  injects nothing, which is worth keeping true.
- **The updater is the riskiest thing we ship**, because "download an executable and run
  it" is exactly the shape of a dropper. Mitigate it deliberately: fetch only from the
  project's own GitHub releases over HTTPS, verify the downloaded installer's hash and
  Authenticode signature before executing, never fetch from a URL that came from a
  response body, and never execute into a temp path an attacker could pre-create.
- **NativeAOT output is occasionally flagged** on first appearance. Signing helps, and so
  does submitting false positives to Microsoft's analyst portal rather than shrugging.

## The sensor helper, and what needs administrator rights

Sensors come from `helper/`, a self-contained .NET process wrapping LibreHardwareMonitor,
fetched on demand rather than shipped: the installer stays small and the 68 MB is downloaded
once by people who want temperatures. One line out, one line back - the parent writes `read`
and gets one JSON object - so nothing samples hardware unless a reading was asked for, and
the helper is idle the rest of the time.

**Fetching it is not built.** Today `build.ps1` stages `barometer-sensors.exe` and the
installer packages it beside `barometer.exe`, which is where `main.rs::helper_path` looks for
it; nothing anywhere downloads the helper. What *is* fetched on demand is
LibreHardwareMonitor itself (`lhm_install.rs`), which is the part that must never be shipped.
Whoever revisits the installer's size should decide which of the two this paragraph meant.

It is confined to a job object with `KILL_ON_JOB_CLOSE`. A Barometer that crashes must not
leave a .NET process holding a driver handle, because the user will never find it.

Measured on the author's machine, **not elevated**: 7 devices, 144 sensors, 92 readable,
38 temperatures. Every NVIDIA and Intel Arc reading came through. **The Intel CPU core
temperatures did not** - they need administrator rights for the MSR reads, and they report
as unavailable rather than as zero.

That split matters for the Sensors pane: without elevation the user still gets GPU
temperatures, which is most of what they came for, and the CPU rows are honestly blank.
Do not demand elevation up front for readings that mostly work without it.

## NativeAOT does not work with LibreHardwareMonitor 0.9.6

Tested, not assumed. The AOT shim builds and exports correctly, `bs_open` returns 0, and
`bs_read` returns **zero hardware**. The identical code compiled for the JIT, in the same
non-elevated session, returns 7 devices and 144 sensors including 38 temperatures.

So the blocker is not administrator rights, which was the obvious suspect and is wrong.
It is the trimming that native compilation forces and will not let you disable. The ILC
warnings say it plainly:

```
Failed to load type 'Byte[]' from assembly '?'
  ... NativeToManaged__CSMI_SAS_STP_PASSTHRU_BUFFER
  ... ManagedToNative__MEGARAID_PASS_THROUGH_IOCTL
```

Those are P/Invoke marshalling stubs. They are not confined to the exotic storage paths the
names suggest: enough of LibreHardwareMonitor's driver interop is broken that every hardware
group fails to initialize, and `Computer.Open()` then reports success having found nothing.

Fixing it means fixing the marshalling upstream, in someone else's library. `TrimmerRoot-
Assembly` and `IlcGenerateCompleteTypeMetadata` are not enough; they preserve metadata, and
the failure is in generated interop stubs.

The shim probe that told these two causes apart is not in the tree any more; the experiment
is thirty lines and is easier to write again than to keep current.

## Weather icons

The fourteen marks are ported from `WeatherBadgeRenderer.swift` and drawn in code. **Not SF
Symbols, and not an icon font**: there is no icon and no container. The temperature is drawn
as plain text at full size and the condition mark is drawn beside it, so the weather costs
the strip a number's width plus the mark's and no more.

The set has to differ in **shape**, never only in color, and there is a test that holds it
to that. A user on a monochrome strip, or a taskbar tinted an unhelpful accent color, still
gets the forecast.

The split was deliberate: `barometer-core/src/weather/badge.rs` produces **geometry** in the
design's own units, with no drawing API anywhere near it, so all fourteen marks are unit
tested. Nothing draws that geometry today - the icon font below won - so `shapes`, `Shape`
and `Palette` are kept for their tests and for the day the marks are drawn as vectors again;
`Condition`, from the same file, is what everything uses. The renderer that consumed them,
`barometer/src/badge.rs`, has been deleted.

GDI+ is used wherever a two or three pixel feature has to be antialiased: the flyout's cards
and graphs (`flyout/paint.rs`), the settings window's marks (`settings_ui/gdi.rs`), and the
weather's sky. GDI has no antialiasing to give them; drawn aliased they read as damage.

Run `barometer.exe --marks <file.bmp>` to lay the whole set out on the taskbar's own dark, at
15 px and 60 px. It draws the **font** glyphs, which is what ships; look at it after any
change to the mapping, since the day and night pairs are easy to get subtly wrong and they
are next to each other on the sheet.


Do **not** use SF Symbols or any system icon set. The macOS app's weather marks were drawn
for it specifically, and they are the ones to port:
`Sources/MenuBarStatsUI/Rendering/WeatherBadgeRenderer.swift`, fourteen of them, drawn in
code rather than shipped as assets. There are tests beside it in
`Tests/MenuBarStatsUITests/WeatherBadgeRendererTests.swift`.

Drawn-in-code ports cleanly to Direct2D and stays sharp at every DPI, which an exported
asset would not.

### An icon font was weighed against them, and won

At roughly fourteen device pixels on a Windows 11 taskbar the ported marks read badly and sat
oddly beside the shell's own tray icons, so **Segoe Fluent Icons** replaced them. It is
Windows' own font, referenced by family name as Segoe UI is, so nothing is redistributed and
no license is owed. Everything that draws a mark - the strip, the preview, the weather panel,
the `--marks` sheet - goes through `barometer-core/src/weather/glyph.rs`.

The finding that nearly stopped it: **the font has no weather set.** Sweeping its whole
Private Use Area turns up a sun, a moon, a cloud, a cloud shedding drops, a raindrop, a
snowflake, a bolt, three drifting lines and a question mark - nine marks for fourteen
conditions, with no sun behind a cloud and nothing that says storm rather than lightning.
That would have cost the shape rule five pairs, and `SHARED_GLYPHS` records which. What
saved it is `glyph::composed`, which draws the nine in combination - a sun *behind* a cloud,
a bolt over one - and gets all fourteen back as distinct shapes. The single-glyph mapping is
kept beside it for the record and for its tests.

## Conventions

**American English, everywhere.** Comments, documentation, commit messages, identifiers and
UI strings: color, behavior, center, recognize, gray, license. The exception is a name that
belongs to somebody else - Open-Meteo really does spell its parameter
`vapour_pressure_deficit`, and a Win32 constant keeps whatever Microsoft called it. Match
the source and say so in a comment if it looks like a mistake.



Taken from Yamato and Clicker, which are the same author's Rust on Windows:

- **`windows-sys`, not `windows`.** Raw FFI, no COM wrappers, far less compile time and
  binary. Every feature in `Cargo.toml` carries a comment saying what calls into it.
- **SPDX header on every source file**, then the project line and copyright.
- **Comments explain why, not what**, and cite evidence where there is any. A comment that
  restates the code earns nothing.
- **Release profile is tuned for size**: `opt-level = "z"`, full LTO, one codegen unit,
  stripped. Nothing here is throughput-bound; the resident binary is the number that matters.
- `unsafe` is confined to the files that exist to call Win32: `sys.rs` and `sys/*` in core,
  plus `pdh.rs`, `net.rs`, `netinfo.rs`, `volumes.rs` and the D3DKMT block in `modules/gpu.rs`;
  in the app, the window, tray, spacer, settings and flyout files that own HWNDs and DCs.
  Everything that is *not* a Win32 call site takes plain Rust types and has none. Every
  `unsafe` block carries a `// SAFETY:` line saying what makes it sound.
- Readings that stop arriving render as unavailable, never as a stale number. This is the
  Mac app's rule and it is why `Readout` has an `unavailable` flag instead of an `Option`.

## Environment

- `target/` is marked `com.dropbox.ignored`. This checkout lives inside Dropbox, and
  without it every build syncs the whole build directory — which showed up as a 10.7 MB/s
  upload spike in Barometer's own network readout while testing.

## Verify

```sh
cargo build
cargo test
cargo run -- 6 --console   # six samples against real hardware, printed, then exit
```
