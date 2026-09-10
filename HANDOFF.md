# Barometer for Windows — handoff

Written 2026-09-08 and kept current since, at version **1.0.0** - what
`Cargo.toml` carries and `.\version.ps1` prints. Everything below is true of
the working tree; see section 4 for what is committed and what is not.

Read `AGENTS.md` first — it holds the decisions, not just the conventions, and
several of them look arbitrary until you know why. This document is the state of
play and the work queue.

---

## 1. What this is

A taskbar system monitor for Windows 11, in Rust against Win32 directly. It is a
**port of the macOS app**, which lives in its own repository,
`mackid1993/Barometer`, and is read through `gh api` rather than checked out
beside this one (AGENTS.md gives the command). Same name, same design, no
shared code. Port means: read the Swift, then write the Rust. It does not mean
invent something similar.

Layout - this repository holds this app and nothing else:

```
barometer-win/
  README.md            the landing page for the Windows app
  .github/workflows/   build.yml, the only workflow: manual, builds an installer
  crates/barometer-core   modules, sensors, weather, settings, store. No UI.
  crates/barometer        lib `barometer_app`: window, settings_ui, flyout, tray
  crates/barometer-bin    bin `barometer`: main.rs and nothing else
  docs/                   ui-design.md and ui-layouts.md, the v1 design specs
  helper/                 .NET 10 sensor helper (BarometerSensorsHelper)
  installer/barometer.iss Inno Setup script
  scripts/build.ps1       local build + stage + package
  version.ps1             reads/sets the version
  scrub-check.ps1         refuses a binary carrying local paths
```

The lib/bin split exists because build-script link args reach every target
including test harnesses, and the manifest demands elevation — so `cargo test`
was prompting for UAC. `barometer-bin` sets `test = false`.

### Hard rules

- **`windows-sys` only, plus `serde_json`. Never add a third.** Not for time,
  not for HTTP: WinHTTP for HTTPS, BCrypt for SHA-256, `tar.exe` for zips.
  `serde_json` is the one exception and both crates carry the reason at the
  dependency - a settings file has to survive a hand-edit and a newer
  version's keys, which is exactly the shape a hand-written parser gets
  subtly wrong.
- **American English everywhere**, including identifiers. `color`, `gray`,
  `initialize`, `canceled`. The exception is a name somebody else owns —
  Open-Meteo really does spell it `vapour_pressure_deficit`.
- **Windows 11 only.** Windows 10 has desk bands and a dozen working options.
- **Never redistribute LibreHardwareMonitor.** Not in the binary, not in the
  installer. It is fetched on request from its own GitHub releases, pinned to
  a tag, size and SHA-256. See `lhm_install.rs`, and do not weaken the pinning.
- **Comments say why, not what.** Match the density of what is already there.
- Tests are full sentences describing behavior.

---

## 2. Build and release

Local:

```powershell
.\scripts\build.ps1              # builds, stages, packages, prints the SHA-256
.\scripts\build.ps1 -SkipInstaller -SkipHelper -Run
.\version.ps1                    # print the version
.\version.ps1 -Bump patch
```

`build.ps1` remaps the source paths out of the binary, builds the workspace in
release, publishes the .NET helper, stages the exe, `LICENSE.md`, `NOTICE.md`
and the icon into `<target>\dist\Barometer`, **refuses any staged executable
still carrying a local path**, then runs ISCC and prints the installer's
SHA-256.

CI is `.github/workflows/build.yml`, modeled on `mackid1993/Yamato`. Manual
dispatch only — no push or pull_request trigger, deliberately. Inputs:
`version`, `publish` (drafts a release), `notes`. It stamps the version with
`version.ps1`, **verifies the stamp landed** in both `Cargo.toml` and
`installer\barometer.iss`, runs `cargo test --workspace` in debug, makes sure
Inno Setup is on the image, and then runs `scripts\build.ps1` — which is what
keeps the local build and the release build from drifting.

**Tags are plain `v1.0.0`.** They used to carry a `windows-` prefix, back when
one repository held both apps and the updater had to avoid offering a Windows
user a DMG; the two have a repository each now, and `update.rs` matches `v`.
The asset name `Barometer-Setup-<version>.exe` is matched exactly by
`update.rs::installer()` — renaming it breaks in-app updates silently while the
download page looks fine.

Version is **1.0.0**, and the two apps' numbers do not line up. They are
separate programs with separate features and separate release schedules, so
forcing one ordering on both only ever confused somebody.

### Never build a shippable binary carelessly

`scrub-check.ps1` exists because this repo lives under a directory named after
its author, inside a synced folder also named after him. It is run by hand
against one binary; `build.ps1` carries the same check inline over everything
it stages, so a release cannot be packaged without it. Its patterns are all
**path** forms on purpose: the version resource legitimately contains
`(c) 2026 David Brustein. GNU GPL v3.` and the first draft of the check failed a
correct build over it. A check that cries wolf is one somebody starts bypassing.

---

## 3. Architecture notes you will otherwise rediscover painfully

**The strip is a top-level layered window, not a child of the taskbar.** A
`WS_CHILD` under `Shell_TrayWnd` is composited under the XAML surface and
invisible. It is `WS_POPUP | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
| WS_EX_TOPMOST`, drawn with `UpdateLayeredWindow`, which only works on
top-level windows. Background alpha is 6, not 1 — at 1 the window is
hit-transparent and the tray icons underneath receive the mouse.

**Space is reserved by planting transparent tray icons.** `reserve.rs` +
`spacer.rs` + `tray.rs`. Widening the notification area is what makes Windows
repack the task buttons. Two traps:

- `NIF_GUID` binds a tray identity to the **executable's path**. Move the exe
  and every GUID it ever used is refused from the new location, permanently.
- **Deleting a `NotifyIconSettings` registry entry poisons that GUID forever.**
  The shell keeps recognizing it and never writes its entry again. This is why
  there is no purge at startup, and why `reserve_block.rs` persists which block
  of identities works and `--repair` advances it.

**The tray-click flash, which took three passes and is worth understanding
before you touch `reserve.rs`.** Clicking anything in the notification area makes
the shell re-lay the tray out, and for a tick or two some of our placeholders
report no rectangle at all. That produces a *partial measurement*, and a partial
measurement is dangerous in two different ways:

- Its span is still full width — the far placeholders report — while its count is
  short. `Obstruction::observe_span` calls a region intruded when it is wider
  than the slots it holds account for, so a partial measurement is bit-for-bit
  the shape of somebody dragging an icon in among the placeholders. Two in a row
  and the readout hid itself for as long as the flyout stayed open.
- Its span can also be briefly *wider*, when the re-layout spreads things out,
  which resized the strip.

The rule that covers both: **past the very first measurement, only a complete
one (`seen == held`) may change anything** — not the region, not the intrusion
verdict. `Steady` in `reserve.rs` holds the region; the `observe_span` call is
guarded by the same condition. On top of that, a complete-but-narrower
measurement still has to repeat before it is believed, because that is a real
intrusion or a real shrink and neither is urgent.

The first measurement is taken however partial it is, because startup adds
placeholders one per tick and the readout cannot appear until there is somewhere
to put it. Demanding a complete set there means a machine where one placeholder
never shows never gets a readout.

`Obstruction` (in `spacer.rs`) is still a separate thing with a separate job:
hiding the readout when an icon really is dragged into the region, so it does not
get covered and become unclickable.

**GDI text writes no alpha.** Draw white onto transparency, derive coverage from
`max(r,g,b)`, premultiply. See `colorize` in `window.rs`.

**Temperatures need elevation.** Measured on this machine: 0 readable sensors
unelevated against 43 elevated. (AGENTS.md quotes 92 readable and 38
temperatures from an earlier machine and an earlier LHM; both were true where
they were taken, and neither is a target to hit.) The manifest says `requireAdministrator`. The installer relaunches
itself elevated via `PrivilegesRequired=admin`.

**The scheduled task is registered through PowerShell**, not `schtasks`.
`schtasks` strips a quote level from `/TR`, so the path split at the space and
Task Scheduler looked for `C:\Program`. `Register-ScheduledTask` also lets us
turn off `DisallowStartIfOnBatteries`, which `schtasks` defaults on — a laptop
would never have started it on battery.

**The sensor helper is supervised, not spawned once.** `modules/sensors.rs`
re-reads where the library is on every pass and opens/closes/reopens the helper
around it. `sensors::library::hold()` closes it and waits, so
`lhm_install::{install,uninstall}` can replace files Windows would otherwise
refuse to delete. Do not go back to a single spawn at startup: installing the
library then does nothing until the app is restarted, which was a real bug.

**Two publish channels to the settings window.** `sensors::health` says what the
sensor source is doing; `SettingsWindow::publish` pushes a `Snapshot` of live
readings from the sampling thread. The window never samples hardware itself.

---

## 4. Where things stand

Everything below was verified on screen on 2026-09-08 with the elevated
capture harness in `scripts/capture/` (section 8), not inferred from the code.

### Working and shipped

- **Strip**: draws, sizes to the taskbar, reserves space, follows the tray
  when it moves, right-click menu (owner-drawn, follows the taskbar theme).
  Tooltips are gone on purpose: the flyouts replaced them.
- **Flyouts, all seven, wired and fed**: Weather (48-hour scroller, 10 days,
  sun and moon, **air quality**, details, day pages), CPU (user/system/idle
  split, P/E cores, system card, top processes with app icons), GPU (engines,
  dedicated memory, clock/power/temperature from the sensor source), Memory
  (in use / modified / standby / free from the kernel's page lists, commit,
  pools, **page file**, top processes), CPU's System card carries the
  **load average**, Disks (volumes, physical disks **by model**), Network
  (activity graph, an **interface picker** - every one added up, or one on
  its own, tunnels marked - per-process traffic with app icons, connection
  with an optional **public address**, Wi-Fi), Sensors (per hardware,
  hottest first).
  Stacks open a tabbed panel whose tabs are the stack's readings: a reading
  of a module opens that module's whole panel, in that module's accent, and
  a sensor reading opens the sensors panel on that sensor's hardware with
  its row marked. Verified both ways - a CPU/GPU utilization stack and a
  CPU/GPU temperature stack.
- **Stacks**: every reading of every module, all ~160 LibreHardwareMonitor
  sensors by identifier with their kind shown, per-row freeform labels drawn
  as headings with one colon, per-metric figures (`Module::stack_value`).
- **GPU choice**: every adapter the kernel lists, keyed by name and ordinal;
  Automatic is the adapter with the most dedicated memory.
- Settings window: Strip / a page each for CPU, GPU, Memory, Disks, Network,
  Sensors and Weather / Stacks / Appearance / General / About - that is
  `Pane::ALL`'s order, which is what Ctrl+Tab and the arrow keys walk. The
  sensor source and the weather locations are sections on their own module's
  page; a flyout's gear opens the page for the module it belongs to.
- Settings drive the strip live; fonts resolve through `instance_family`
  (real Semibold face, cache invalidated on weight or family change);
  `ANTIALIASED_QUALITY` text.
- LibreHardwareMonitor: download, verify, install, reinstall, remove — all
  taking effect immediately, no restart.
- Updater, installer, sign-in task, CI. `store.rs` tolerates a UTF-8 BOM.

Tests: 386 in `barometer-app`, 201 in `barometer-core`, all passing, plus two
`#[ignore]`d. A few of the core tests touch the machine (disk 0's model, the
page lists, the interface table, adapter enumeration) and are written to pass
on a build agent with none of it.

### Not done

- **Committing is the user's call, not yours.** The session's work has since
  gone in as a series of commits on `master`, and there is usually a day's
  work in the working tree on top of them. Commit only when he says the word,
  and do not squash what is there into one commit without asking.
- The trace instrumentation is gated on `%LOCALAPPDATA%\Barometer\trace.on`
  or the `BAROMETER_TRACE` variable (`trace.rs`), and is harmless. The
  `click column N -> ...` line is written by the app's main loop where the
  click is acted on, not by the window procedure that received it.

---

## 5. The work queue, in the order I would do it

### 5.1 Mac parity still missing

- **Load average** is an analog too, in `modules/cpu.rs`: Windows has no
  `getloadavg`, but `\System\Processor Queue Length` is the same quantity
  Unix averages - threads runnable and waiting - so it is folded with the
  kernel's own exponential decay over one, five and fifteen minutes. It is
  a real load average, not the busy fraction under another name. One
  difference is written down at the type: Windows samples the queue where
  Unix accumulates it, so a burst between two samples is missed.
- **Memory pressure and swap** are analogs, and say so in
  `modules/memory.rs`: pressure is commit charge against its limit (what the
  panel's Commit chip grades), swap is the page file in use
  (`SystemPageFileInformation`, hand-declared in `sys/processes.rs`).
- `docs/ui-design.md` and `docs/ui-layouts.md` were written before the app
  existed. They have been reconciled with it - the nav, the module pages, the
  strip's one text size, the 3 DIP gap, the slot pitch - and every section
  describing something that was never built now says so in a line rather than
  having been deleted: themes and the backplate (design §2.6, §2.7), the
  Taskbar pane (layouts §5), the Colors card, export/import and reset
  (layouts §4, §6). Those notes are the standing list of what the design asks
  for and the app does not do yet.

### 5.2 One that was wrong for a long time, in case it recurs

The network readout was reporting several times the traffic that actually
moved. `GetIfTable2` returns an entry for every NDIS **filter** bound to an
adapter - the packet scheduler, two MAC-layer filters, the Wi-Fi filter
driver - and each one reports the adapter's own byte counters. Summing every
row that was up and not loopback therefore counted one Wi-Fi radio five
times: 37.4 GB where the radio had moved 8.5. `sys::is_selectable` is the
rule that fixes it, and `the_filter_layers_are_not_counted_a_second_time`
holds it down. If a rate ever looks like a suspiciously round multiple of
the truth again, that is where to look.

### 5.3 Rough edges seen on screen

- A panel taller than `MAX_PANEL_H` (720 DIP) scrolls; a process dying
  mid-paint leaves such a window with a black band where the unpainted
  content was. That band is a symptom of the crash, not of sizing:
  `relayout` already refits the frame.
- A stack of many readings may overflow the stack flyout's tab strip. Nobody
  has made one that wide yet; it is untested.
- `Idle 9...` was a chip whose label measured a pixel narrower than it drew;
  `paint.rs` now lets chip labels draw into the right padding. Watch for the
  same in any new fixed-width text box: `DrawTextW` with `DT_END_ELLIPSIS`
  cuts at one pixel over.

### 5.4 Smaller things

- The build is warning-free; keep it so. Note that `cargo test --workspace`
  does **not** build the bin - `barometer-bin` sets `test = false` - so its
  warnings hide from the usual loop. `cargo check -p barometer --bin barometer`
  is what catches them, and two sat there unnoticed until an audit found them.
- The scratch `stage*.py` patch scripts are not in the repo and should not be;
  they were the session's way of editing CRLF files safely.

---

## 6. Things that will bite you

- **An empty string into `DrawTextW` is an access violation.** An empty
  `Vec<u16>` has a dangling pointer of `0x2`, and `DrawTextExWorker` reads it
  even when told the length is zero. It killed the flyout thread inside a
  window procedure (`c000041d`), which WER reports as a crash in `USER32.dll`
  with no Rust frame near the top. `paint.rs::text`, `measure` and
  `text_alpha` now return early on empty text; keep that if you add a fourth
  text path. The trigger was a GPU engine the counters named `engtype_` with
  nothing after it, which `engine_type` now drops.
- **`GetLogicalProcessorInformationEx` records are shorter than the struct.**
  The struct declares the union of every record kind; a core's record is 48
  bytes. Walking with `offset + size_of::<STRUCT>() <= len` skipped the last
  record, which on a 185H was one of the low-power E-cores (shown as `C22`).
  `core_classes` copies each record into a zeroed struct by its own size.
- **Windows PowerShell's `Set-Content -Encoding utf8` writes a BOM**, and
  `ConvertTo-Json` round-trips lose nothing but that. The store now strips a
  BOM, but the harness writes settings through
  `[IO.File]::WriteAllText(..., UTF8Encoding($false))` anyway.
- **Do not write Rust source through PowerShell text round-trips.** UTF-8 with
  non-ASCII gets double-encoded. Write escapes (`\u{2193}`) and edit at the
  byte level with explicit encoding.
- **Bash heredocs mangle `\u` and quotes** in Python that patches Rust. Every
  patch this session went through a `.py` written with the file-write tool,
  using a `swap` helper that matches `\r?\n` so the CRLF tree is untouched.
- **Line endings are mixed, file by file, and always have been.** About a
  quarter of the sources are CRLF and the rest LF, while git's index is LF
  for every one of them (`core.autocrlf=true`, no `.gitattributes`), so a
  commit normalizes whatever you wrote. What matters is not rewriting a
  whole file's endings and burying a two-line change under a whole-file
  diff: match patterns with `\r?\n`, write back what the file already had,
  and check with `git ls-files --eol <path>` rather than assuming.
- **The app runs elevated, so the harness must too.** UIPI drops injected
  clicks and `WM_*` from an unelevated process; `Get-Process` cannot read the
  elevated process's path; `tasklist //FI` under Git Bash mangles the filter
  and says nothing is running. Launch captures with `Start-Process -Verb
  RunAs`, and check for the process with `Get-Process -Name barometer`.
- **A single-instance mutex** means a second launch exits silently, and
  `Stop-Process` from an unelevated shell fails silently against the
  elevated app. Together those two cost an hour: a capture that changed
  settings, "stopped" the app, and relaunched was reading the *old*
  instance with the *old* settings, and looked exactly like the stack tabs
  ignoring which reading was chosen. Kill it from an elevated shell and
  confirm `Get-Process -Name barometer` is empty before relaunching.
  When a capture finds no strip at all, look for a modal message box
  instead: the store's "could not read its settings" box parks the main
  thread in `WaitMessage` with a "Barome…" button on the taskbar.
- **`cargo build --workspace` builds the bin too**; `cargo check -p
  barometer-app` is the fast loop for lib work. The bin cannot be rebuilt
  while the dev instance runs (`failed to remove file ... barometer.exe`).
- `build.ps1` finds the target directory through `cargo metadata` rather than
  assuming `./target`, so a machine that redirects it with a `.cargo/config.toml`
  still packages the right binary. This machine does not redirect it today.
- Dropbox was force-stopped on this machine during development. It may want
  restarting.

---

## 7. Verifying you have not broken the hard-won behaviors

All invisible in a screenshot and all took a long time to get right.

1. **Click the tray overflow chevron, then the tray icon beside the strip.**
   The strip must not flash, resize or disappear, even for a frame. Then
   dictate something so the microphone icon appears and goes away: the strip
   must follow the tray both ways with no gap left behind.
2. **With Barometer running, remove LibreHardwareMonitor from the Sensors pane,
   then install it again.** Temperatures must go away and come back on their
   own, with no restart and no instruction to turn the Sensors module off first.
3. **Open every flyout** (`scripts/capture/capture-scroll.ps1` does it) and
   scroll each to the bottom. A panel that opens and later goes blank, or a
   process that vanishes while a panel is open, is the empty-string crash or
   a relative of it: run under `cdb -G` and look for `DrawTextExWorker`.

---

## 8. The capture harness

`scripts/capture/` holds the PowerShell that verified this session's work.
Each script is run **elevated** (`Start-Process -Verb RunAs -FilePath
powershell -ArgumentList "-NoProfile","-ExecutionPolicy","Bypass","-File",...`)
with the app already running, and writes a PNG beside its `-Out` file:

- `capture-strip.ps1 -Out strip.txt -WaitMs 10000` — waits, then snaps the
  strip. `0,0-0,0` means there is no strip window: the app is still holding
  for its tray reservation, or it is parked in a message box.
- `capture-scroll.ps1 -Out x.txt -Fraction 0.5 -Name cpu -Wheel 10` — clicks
  the strip at a fraction of its width, waits, scrolls the panel by `-Wheel`
  notches, snaps it, presses Escape. One panel per run: the fraction is the
  column's center over the strip's width, so read the strip snap first.
- `capture-tabs.ps1 -Out x.txt -Fraction 0.64 -ChipX 129 -ChipY 33` — opens a
  stack's panel and presses one of its tabs, snapping before and after.
- `capture-settings.ps1 -Out x.txt [-ClickX -ClickY ...]` — brings up the
  settings window, opening it from the strip's menu if it is not up, and
  clicks up to three points in it before snapping. Two traps are baked in:
  `FindWindowW` does not match `BarometerSettings` across processes, so it
  enumerates instead, and a menu ignores an injected click on an item the
  real pointer never entered, so the menu is driven from the keyboard.
- `capture-menu.ps1 -Out menu.txt` — right-clicks the strip and snaps the menu
  with the second row hovered.

Switch tracing on with an empty `%LOCALAPPDATA%\Barometer\trace.on`; the
`click column N -> ...` lines in `trace.log` say which column a click landed
on. Back up the user's `%APPDATA%\Barometer\settings.json` before enabling a
module just for a capture, stop the app, and copy the backup back.
