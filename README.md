<p align="center">
  <img src="assets/AppIcon.png" width="160" alt="Barometer app icon">
</p>

<h1 align="center">Barometer for Windows</h1>

<p align="center">
  A system monitor for the Windows 11 taskbar.
</p>

<p align="center">
  <a href="https://github.com/mackid1993/barometer-win/actions/workflows/build.yml">
    <img alt="Build"
         src="https://img.shields.io/github/actions/workflow/status/mackid1993/barometer-win/build.yml?style=for-the-badge">
  </a>
  <a href="LICENSE.md">
    <img alt="GPL-3.0 License" src="https://img.shields.io/github/license/mackid1993/barometer-win?style=for-the-badge">
  </a>
  <a href="https://github.com/mackid1993/barometer-win/releases/latest">
    <img alt="Latest release" src="https://img.shields.io/github/v/release/mackid1993/barometer-win?style=for-the-badge">
  </a>
  <a href="https://github.com/mackid1993/barometer-win/releases">
    <img alt="Downloads" src="https://img.shields.io/github/downloads/mackid1993/barometer-win/total?style=for-the-badge">
  </a>
</p>

<p align="center">
  <img alt="Windows 11 or later" src="https://img.shields.io/badge/Windows-11%2B-111827?style=flat-square&logo=windows">
  <img alt="x64 native" src="https://img.shields.io/badge/x64-native-111827?style=flat-square&logo=windows">
  <img alt="Free and open source" src="https://img.shields.io/badge/free-open%20source-06B6D4?style=flat-square">
</p>

Barometer is a free, open source system monitor that draws a strip of readings on the
Windows 11 taskbar: processor and memory load, graphics, disk and network throughput,
hardware temperatures, and the weather. Click a column and a panel opens under it with
the detail behind that number.

It is a port of [Barometer for macOS](https://github.com/mackid1993/Barometer), which
lives in the menu bar, by the same author. Same name, same design, no shared code - that
one is Swift against AppKit, this one is Rust against Win32 - so the two do not have all
the same features and their version numbers are their own. Each has its own releases here
on GitHub and each updates itself from its own.

There is no account, no subscription and no telemetry.

## What it shows

Seven modules. Each can be on the strip or off it, in whatever order you drag them into,
and each opens a panel when clicked. A fresh install shows **Processor and Memory** and
nothing else; the rest are switched on as you want them.

| Module | On the strip | In its panel |
| --- | --- | --- |
| **Processor** | `CPU` over the total load | A history graph from 1 minute to 24 hours, the user/system/idle split, every core as a bar with performance and efficiency cores labeled, load averages, uptime, process, thread and handle counts, and the busiest processes with an end-task button |
| **Graphics** | `GPU` over the busiest engine's load | A history graph, every engine type's share, memory in use against dedicated, and frequency, power and temperature when a sensor source reports them |
| **Memory** | `MEM` over the percentage in use | A breakdown of in use, modified, standby and free, commit charge with its own graph and a Normal/High/Critical state, page file, pools, and the largest processes by working set |
| **Disks** | The read rate over the write rate | A mirrored read/write graph, every volume with a capacity bar and free space, and every physical disk with its model, rates and operations per second |
| **Network** | The download rate over the upload rate, with arrows | Download and upload graphed together, per-process throughput, every local address with a copy button, router, DNS, an optional public address, and Wi-Fi signal, noise, channel, rate and security |
| **Sensors** | `TEMP` over one temperature | Every reading the source gives, grouped into temperatures, fans, power, voltage and current, each with a one-minute sparkline |
| **Weather** | The temperature with its condition drawn beside it | Current conditions over a painted sky, the next 48 hours as a scrolling chart, a 10-day forecast, sun and moon, air quality, and a page of detail per day |

Any of those readings can also go into a **stack**: several values in one column, which is
how you fit more on without making the strip longer.

## Requirements

| | |
| --- | --- |
| **Windows 11** | x64. Windows 10 is refused by the installer - see below |
| **Taskbar on top or bottom** | A side-docked taskbar is refused with a message |
| **Administrator** | Required. The installer registers a sign-in task, so only a manual launch prompts |
| **A .NET runtime** | Not needed. The sensor helper carries its own |
| **[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)** | Optional, for temperatures, fans and voltages. Barometer can fetch it for you, into your own AppData, and ships none of it |
| **[PawnIO](https://pawnio.eu)** | Optional, for the processor's temperatures and the motherboard's fans, voltages and board temperatures. Graphics and drive temperatures arrive without it. Somebody else's signed driver, installed by you |
| **Internet** | Only for what asks: the weather, your approximate location if you want it, and update checks |

Everything optional above is genuinely optional: anything unavailable reads as unavailable
rather than as a wrong number.

**Windows 11.** Not a preference. Windows 11 removed deskband support and took every
taskbar monitor with it, so there is no supported way to put anything on that taskbar;
Barometer claims its room by a trick that is specific to how the Windows 11 taskbar lays
itself out. On Windows 10 it would install and draw nothing, and the installer refuses to
run there rather than letting that happen. Windows 10 does not need it in any case - it
still supports deskbands, so it has every system monitor under the sun already.

**Top or bottom.** The strip gets horizontal room by widening the notification area, which
makes Windows repack the task buttons aside. On a taskbar docked to the left or the right
the tray is a vertical grid: widening it frees no horizontal space and the readout has
nowhere to go. That is geometry, not a rendering problem, so Barometer says so and exits
rather than running invisibly - at startup, and again if the taskbar is moved to a side
edge while it is running. The small taskbar that 26H2 brought back is fine: the strip
measures the bar and sizes itself to whatever it finds.

**Barometer runs as administrator.** Most hardware sensors are behind instructions and
ports that user mode may not touch - the processor's temperatures in model-specific
registers, the motherboard's fans, voltages and board temperatures behind the SuperIO
chip, the embedded controller or the SMBus - and reading any of them is a ring-0
operation. PawnIO, which does that reading, gives its device object an ACL that admits
only SYSTEM and the Administrators group, so an unelevated Barometer is refused outright.
The installer registers a scheduled task that starts Barometer with highest privileges at
sign-in, which is why sign-in is silent; the ordinary Windows startup list cannot start an
elevated program at all.

Nothing else about the app needs elevation, and the sensors that have a path of their own -
the graphics card, which vendor libraries report, and the drives, which answer SMART and
NVMe queries directly - arrive either way.

## Installing

Download `Barometer-Setup-<version>.exe` from
[the releases page](https://github.com/mackid1993/barometer-win/releases) and run it. It is
an [Inno Setup](https://jrsoftware.org/isinfo.php) installer and asks for administrator,
because the program does.

Three tick boxes on the tasks page:

- **Start Barometer when I sign in** - on by default. It registers a scheduled task named
  `Barometer` that runs with highest privileges, is allowed to start on battery, is not
  stopped when the machine goes onto battery, has no execution time limit, and ignores a
  second instance. The same switch is on the General settings pane afterwards, where it
  enables and disables that task rather than making a new one.
- **Create a desktop shortcut** - off by default.
- **Open the PawnIO download page** - off by default, and deliberately: Barometer installs
  no driver and must not look like it is nudging one onto the machine. It opens
  [pawnio.eu](https://pawnio.eu) in your browser, as yourself and not as administrator, and
  nothing is downloaded or run by the installer.

**Settings live in `%APPDATA%\Barometer\settings.json`.** Roaming, because the font, the
strip order and the units are your choices and should follow you between machines. The file
is written whole through a temporary beside it and renamed over, after being parsed back
and compared, so a crash or a full disk cannot leave half a file where your settings were;
a file it cannot read is moved aside rather than written over. Keys a newer version wrote
are preserved when an older version saves, so the two can share one file.

**The sensor library is never shipped.** LibreHardwareMonitor is not in the binary and not
in the installer. The Sensors settings pane offers to download it - about 9 MB, from
LibreHardwareMonitor's own GitHub releases, pinned to a tag, an asset name, a size and a
SHA-256 - into `%LOCALAPPDATA%\Barometer\LibreHardwareMonitor`. Nothing that arrives over
the network chooses what is fetched, where it goes, or whether it is kept. You can also
point Barometer at an installation of your own, including a portable copy.

**PawnIO you install yourself**, from its own site. Barometer contains no kernel driver and
never will. It probes for the driver's device object and tells two answers apart: absent,
where it offers the download page; and present but closed to an unelevated process, where
offering a download to somebody who already installed it would be worse than silence. If it
is genuinely missing, Barometer says so **once** - what it costs, that it is optional, and
that it is somebody else's driver - and does not ask again.

Uninstalling removes the sign-in task and leaves `%APPDATA%\Barometer` alone.

## The strip

The Windows taskbar has no multi-item API, so where macOS gives Barometer one draggable
menu bar item per module, here every enabled item is laid out inside **one strip**.

### Columns, modules and stacks

A **column** is one reading, two rows tall. A module's own column is either a label over a
value (`CPU` over `42%`, and likewise `GPU`, `MEM` and `TEMP`), two values stacked (Disks
puts its read rate over its write rate; Network puts `↓` over `↑`, or the other way round if
you prefer), or - for Weather - the temperature at full size with its condition mark drawn
beside it, which costs the strip a number's width plus the mark's and no more.

A **stack** is a second kind of item. It composes *readings* from several modules into one
column - CPU, GPU and memory together, say - each drawn as `LABEL: value` with a label you
can override, either paired two to a row or all on one row. Twenty-two fixed readings are
offered, plus every individual sensor the source reports. A stack can also **hide the
columns of the modules it draws from**, which is the space saving; that is a tick box, not
the architecture. Modules and stacks are independent, so a valid strip is any mix: Memory on
its own, and a stack carrying GPU, disk and network readings with its sources hidden.

Modules and stacks live in **one order**, set by dragging rows in the Strip settings pane
(or Alt+Up / Alt+Down on a focused row). A module that is enabled but has nothing to say -
Sensors with no source, Weather with no location - is not drawn at all rather than shown as
a placeholder, and the composer row says why.

### How it claims room

Windows 11 lays the taskbar out in two columns, and the task-button strip's right edge *is*
the notification area's left edge. Nothing can be inserted between them, and anything drawn
over the buttons is drawn over somebody's window button. So Barometer does not ask for room:
it registers **transparent placeholder icons** in the notification area, Windows repacks the
task buttons aside on its own, and the readout sits on the space that opened up. A slot is
about 42 physical pixels at 100%, but the real pitch is measured once the icons are on
screen rather than assumed.

What that means in practice: **the space comes out of the task buttons, a whole button at a
time.** On a full taskbar, turning on another column can tip a task button into the overflow
menu. It is also why the column gap defaults to three pixels rather than something
comfortable-looking.

Two behaviors follow from sharing the tray with everyone else's icons. If another program's
icon is dragged in among the placeholders, the readout **hides itself** rather than covering
it, and comes back on its own when the icon leaves. And when Explorer restarts, the shell
takes the placeholders and the strip with it; Barometer rebuilds both on the next tick.

### Layout, type and color

**Two rows, always.** There is no one-row layout and there must never be one: a label beside
its number turns the whole strip sideways and makes it wider than anything anybody asked for.

**Text size is 6 to 24 points**, one figure for the whole strip, 9 by default. A size that
two rows of will not fit the taskbar you have is **held down** to the largest that will,
rather than dropping a row, and the caption under the slider says when that is happening. The
default fits two rows in every taskbar height Windows 11 offers, the 32 DIP small bar
included. Size is worked out in DIPs and not in pixels, because the small 26H2 taskbar at
150% is exactly as many physical pixels as the large one at 100% and they are not the same
bar.

**The gap between columns is 0 to 24 pixels**, three by default.

**Fonts**: any family Windows reports, with an optional second family for the labels - on
Windows a weight is often a family of its own, and Segoe UI Semibold over Segoe UI Variable
Text is an ordinary piece of typography a weight control cannot express. Labels and values
carry separate weights, and only the weights a family actually has faces for are offered.
The default is Segoe UI Variable Text, regular values under semibold labels.

**Light and dark follow the taskbar**, not the app theme. Those are separate switches in
Windows, and the readout lives on the taskbar, so it reads `SystemUsesLightTheme`. Its
background is per-pixel alpha rather than a color key, so the numbers sit directly on
whatever the taskbar is showing - there is no plate and no container.

**Right-clicking the strip** gives three items: Settings..., Check for updates, and Exit
Barometer. Quitting lives there; the settings window has no quit control.

## The modules

### Processor

The column shows `CPU` over the total busy percentage. The panel's header carries the load
to one decimal and the user, system and idle split. A **History** card graphs busy time over
a span you pick from `1m`, `5m`, `30m`, `3h` and `24h` - five minutes by default - with each
sample placed by its timestamp, so a sleep gap reads as a gap rather than a seam; up to
24 hours are kept, and they accumulate whether or not the panel is open. A **Cores** card
draws every logical processor as a labeled bar, with performance cores as `P1`, `P2` and
efficiency cores as `E1`, `E2` where Windows reports the distinction. A **System** card gives
the one, five and fifteen minute load averages (computed from the ready queue, the way the
Unix kernel computes them, since Windows has no `getloadavg`), uptime, the process and thread
counts, and the handle count. **Top processes** lists the five busiest with their real icons,
a bar for each one's share of the whole machine, and an end-task glyph at the right of the
row that terminates it with no confirmation - keyed by process id, so a list that re-sorted
under your finger cannot kill the wrong thing.

### Graphics

The column shows `GPU` over the busiest engine's utilization, which is the figure Task
Manager puts in its headline; summing every engine would report 200% for a machine decoding
video while it renders. The panel names the adapter, graphs the headline over the same five
spans, breaks utilization down by engine type - 3D, Copy, Video Decode, Video Encode and
whatever else the driver publishes - and shows dedicated memory in use against the adapter's
total. A **Hardware** card carries frequency, power and temperature; Windows publishes none
of those, so they come from the sensor source and read as dashes without one. Machines with
more than one adapter get a picker in settings, defaulting to the busiest.

### Memory

The column shows `MEM` over the percentage in use, which is total minus available - Task
Manager's figure. The panel's **Breakdown** card draws one segmented bar of in use, modified,
standby and free, from the kernel's own page lists, with chips naming each. Windows has no
memory-pressure figure, so the **Commit** card stands in for the Mac's: commit charge against
its limit, graphed over the whole history held, with a state chip reading Normal, High at
70%, or Critical at 90%, and rows for committed bytes, the page file, and the paged and
non-paged pools. **Top processes** lists the five largest working sets with a bar for each
one's share of installed memory. There is no end-task button here.

### Disks

The column shows the read rate over the write rate, for every disk added together or for one
you choose. Disks are always in bytes; there is no unit setting. The panel opens with two
rate tiles and a mirrored graph - reads above a centerline, writes hanging below it, both
scaled to one shared ceiling so a quiet disk is a flat line rather than noise - over the last
300 samples. A **Volumes** card gives every mounted volume its own plate: name, mount point,
a capacity bar, the used percentage and the free space, turning red at 90% full. A **Physical
disks** card, present when Windows publishes per-disk counters, names each device by its
model, tags it with the system's own name for it, and gives read and write rates and
operations per second. Which disk the rates describe and which volume the space describes are
separate settings: a PC routinely has three or four drives, and "how busy is the machine" is
a different question from "is the scratch drive full".

### Network

The column shows the download rate over the upload rate with `↓` and `↑` arrows, or the
other way round. Rates can be counted in bytes or in bits. By default every active interface
is summed **except loopback and VPN tunnels** - a tunnel's bytes also cross the physical
adapter beneath it and would be counted twice - while the identity shown stays the primary
route's; you can also follow one adapter.

The panel graphs download and upload over one another across the last 300 samples, tags the
interface with chips reading `VPN` and `Primary` where they apply, and gives the bytes
received and sent. **Top network activity** lists five processes with a download and an upload
rate each, summed from the TCP table. A **Connection** card lists every IPv4 and IPv6 address,
the router, every DNS server, and - only if you switch it on - the public address the internet
sees you from, looked up through ipify while the panel is open. **Clicking any of those rows
copies the address**, and the copy glyph turns into a check for a second. A **Wi-Fi** card
appears when there is a radio, with the signal in dBm colored by strength, the SSID, noise,
channel and band, transmit rate and security.

### Sensors

Windows publishes no API for temperature, fan or voltage sensors, so this module reads them
through LibreHardwareMonitor - see below. The column shows `TEMP` over one temperature: the
hottest processor reading by default, or any temperature sensor you pin. The panel groups
everything the source reports into cards for temperatures, fans, power, voltage and current,
each row carrying a 64-pixel sparkline of its last minute normalized to its own range, so a
flat reading sits mid-height instead of on the floor. Temperatures are sorted hottest first
with unreadable ones last; a name that repeats across devices is qualified with the hardware
it came from. Hardware temperatures have their own Celsius/Fahrenheit setting, separate from
the weather's, because nobody knows what 85 degrees means for a processor in the units they
use for the sky. With no source the panel is calm about it and says what to install; a fresh
machine with no LibreHardwareMonitor is the normal first-run state, not an error.

Individual sensors are not limited to the strip's one column: a stack can carry any of them
by name.

### Weather

The column is the temperature with its condition drawn beside it - fourteen marks, composed
from Segoe Fluent Icons glyphs, chosen to differ in **shape** and not only in color so a
monochrome or accent-tinted taskbar still gives you the forecast.

The panel opens on a painted sky card - a gradient chosen by condition and by whether it is
day there - carrying the place, the conditions in words, the feels-like temperature, the
headline temperature, and when it was last updated in the location's own time zone. Below it:

- **Next 48 hours**, a chart that **scrolls sideways**, plotting the temperature curve over a
  wash with rain-probability bars along the bottom, labeled every third hour with the hour,
  its condition mark, its temperature and its chance of rain. It starts at the next whole
  hour, not the one in progress.
- **10-day forecast**, one row per day: weekday, condition mark, chance of rain, the low, a
  temperature range bar colored on an absolute scale, and the high. Rows open a day page.
- **Sun & moon**: the day drawn as an arc with the sun placed along it by how far through
  daylight you are, sunrise and sunset either side of the horizon, the length of the day, then
  a drawn moon with its phase name, percentage illuminated, moonrise and moonset.
- **Air quality**, when the service answers: the US AQI as a number and a band - Good,
  Moderate, Unhealthy for sensitive groups, Unhealthy, Very unhealthy, Hazardous - on a scale,
  with PM2.5, PM10 and ozone in µg/m³.
- **Details**: humidity, wind with a drawn compass arrow, pressure with a drawn gauge, UV
  index, cloud cover, gusts, precipitation and visibility. A tile with no value is dropped
  rather than shown as a dash.
- **Location**: a chip per saved place. Pressing one shows that city.

A **day page** - reached by clicking a day, left by the back button or Backspace - carries
that day's summary, a glance card of the high and low feels-like, precipitation chance and
total, maximum UV, wind and gusts, then rain, showers, snowfall, precipitation hours and
sunshine; averages for temperature, humidity, dew point, cloud cover, wind, visibility and
pressure; that day's 24 hours in the same sideways-scrolling chart, where hovering or clicking
an hour opens every reading for it - down to soil temperature at four depths, soil moisture at
five, CAPE, vapor pressure deficit, freezing level and the radiation components; its own sun
and moon card; and the atmospheric and growing figures for the day. Rows and whole sections
with no data are omitted rather than filled with dashes.

## Flyouts

Click a column and its panel opens anchored under it. **One at a time**: clicking another
column swaps to that panel, clicking the open column closes it. It is dismissed by Esc, by
pressing anywhere outside it, or by activation going elsewhere - another window, Start, the
lock screen. Pressing the column it belongs to does not dismiss it, because that click is what
toggles it.

Panels are a fixed width and as tall as their content, capped; taller content scrolls, with
the wheel or with Up, Down, Page Up, Page Down, Home and End. The page has a soft resistance
at each end rather than a hard stop. The weather's hourly charts are the only things that
scroll sideways, on a horizontal wheel, on the wheel with Shift held, or by dragging.

Every panel has a **footer**: a gear at the left, which opens the settings window at that
module's own pane, and the panel's own actions at the right. Only Weather has one - `Refresh`,
disabled while a fetch is in flight - which refetches the panel's forecast and the strip's
reading together. Panels update live from the same samples the strip is drawing.

A **stack's** panel is not a picture of its own: it shows the whole panel of whichever reading
you pick, with a row of chips at the top when there is more than one, captioned exactly as
the strip captions them. Picking a sensor reading opens the Sensors panel focused on that
sensor - that device's readings only, with the row marked - so a CPU-temperature chip and a
GPU-temperature chip of the same stack are two different panels. The gear, the accent and the
footer buttons follow whichever reading is showing.

## Settings

Reached from the strip's right-click menu, from any panel's gear, or with
`barometer.exe --settings`. Twelve panes down the left, and **every change applies as it is
made** - there is no OK or Apply, and closing the window only hides it.

| Pane | What is in it |
| --- | --- |
| **Strip** | A live preview of the strip drawn at your taskbar's height and colors, with a button to flip it between light and dark, and the composer: one row per module and stack, each with its own on/off switch and a caption saying how it is drawn or why it is not. Drag to reorder, click to open that item's page, **New stack** to make one |
| **CPU**, **GPU**, **Memory**, **Disks**, **Network**, **Sensors**, **Weather** | One page each: whether the module draws its own column, which stacks carry its readings, and a control to put it into a stack or start a new one with it. Then whatever that module has to choose - the graphics adapter; which disk the rates are for and which volume the space is for; the network's row order, bytes or bits, interface, and whether to look up the public address; the sensors' pinned reading, unit, decimal places and read interval; the weather's locations, units and refresh |
| **Stacks** | The list of stacks and an editor for the selected one: its name, whether it is on, one row or two, its readings in order with a label field and up/down/remove buttons each, a control to add any of the twenty-two fixed readings or any individual sensor, and **Hide these modules' own columns** |
| **Appearance** | The same preview, then the value font, the heading font, the two weights, text size (6-24) and the gap between columns (0-24). There is no theme setting: the window follows the shell, and the strip follows the taskbar |
| **General** | Start Barometer when I sign in; the running version with a **Check now** button, automatic weekly checking, and a row for a skipped release with a way to stop skipping; and **Export…** / **Import…** for the settings file |
| **About** | The version, the license, links to the source and to the GPL, and credits for LibreHardwareMonitor, PawnIO and Open-Meteo |

**Export and import** write and read the same JSON document the store keeps, so a file that
survives an import is a file the app would have loaded at startup. Import goes through the
parser rather than the loader, so a file you merely pointed at is never moved aside if it
turns out to be unreadable.

The window can be driven entirely from the keyboard. Tab and Shift+Tab move through every
control in reading order; Ctrl+Tab moves between panes; Space and Enter activate what is
focused; arrows move the selection in a list, a slider by one step, or scroll the pane;
Alt+Up and Alt+Down move a strip item in the order; Delete removes a focused stack; Esc
closes an open dropdown, cancels an edit in a text field, or hides the window. Focus rings
appear once the keyboard has been used and not before.

## Where temperatures come from

Windows exposes no temperature, fan or voltage API. The real readings live behind SuperIO
over LPC, CPU model-specific registers, the SMBus and vendor GPU libraries - all ring-0 or
vendor SDK, and reimplementing that is most of what
[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) *is*.

So Barometer orchestrates it rather than reimplementing or redistributing it:

- `barometer-sensors.exe`, a small self-contained .NET helper installed beside Barometer,
  loads `LibreHardwareMonitorLib.dll` in process and answers one line at a time. Nothing
  samples hardware unless a reading was asked for, and the helper is confined to a job object
  so a crashed Barometer cannot leave a process holding a driver handle.
- The library itself is **fetched on request** from LibreHardwareMonitor's own GitHub
  releases, pinned to the tag, asset name, byte count and SHA-256 that were tested against
  this helper, and unpacked into your local application data. It is never shipped, so no MPL
  redistribution obligation is taken on and the installer stays small.
- If you already have a copy - installed, or portable from a zip - the helper finds it in the
  registry and in the usual places, or you can name the folder yourself.
- [PawnIO](https://pawnio.eu) is the signed kernel driver LibreHardwareMonitor runs its
  hardware modules inside: `IntelMSR`, `AMDFamily17` and `RyzenSMU` for the processor, `LpcIO`
  for the SuperIO chip's fans and voltages, `LpcACPIEC` and `IsaBridgeEC` for embedded
  controllers, `SmbusI801` and friends for memory module temperatures. You install it;
  Barometer never loads it and never talks to it.

**Without any of it**, six modules work exactly as described and Sensors reads unavailable
with a sentence saying what to install. **With LibreHardwareMonitor but without PawnIO**, you
get what has a path of its own - the graphics card and the drives - which for most people is
most of what they came for. **With both, unelevated**, PawnIO refuses the handle and the
processor and motherboard rows are honestly blank rather than zero. **With both, elevated**,
everything the hardware reports arrives.

## Where the weather comes from

Weather comes from [Open-Meteo](https://open-meteo.com/), free for non-commercial use, with
no account and no key. Four endpoints are used: the forecast for the strip's observation and
again for the panel's ten days, the air-quality service, and the geocoder behind the city
search.

**Finding you.** By default Barometer guesses your location from your internet address,
through [ipapi.co](https://ipapi.co/) with [ipwho.is](https://ipwho.is/) as a fallback, asked
no more than once per run. That is on by default here where it is off on macOS, because an
address lookup raises no consent prompt and asks nothing of the machine. Turn it off and
nothing is asked of anyone. Either way you can search for cities by name and keep as many as
you like, one of them primary, switching between them from the panel's location chips.

**Units** are four independent choices: temperature (Celsius or Fahrenheit, and it also
decides whether visibility is in miles or kilometers), wind speed (km/h, mph, m/s or knots),
pressure (hPa, inHg or mmHg) and precipitation (millimeters or inches). Changing one makes
the forecast stale and refetches rather than converting a stale number.

**Refresh** is every 5 to 60 minutes, 15 by default. A failed fetch keeps the last good
reading, says so, and retries in a minute. The address lookup has a ladder of its own -
1, 2, 4, 8, 16 and then 32 minutes - because the first-run state with no saved location is
exactly the state a laptop is in behind a captive portal, and a flat one-minute retry there
is about 1,440 requests a day against a free tier that allows 1,000.

## Updates

Barometer checks its own [GitHub releases](https://github.com/mackid1993/barometer-win/releases)
**once a week**, and you can check whenever you like from the General pane. The check is quiet
unless it finds something newer; the last check time is remembered in the settings file, so
restarting the machine is not a way of asking again. A release you skip stays hidden from the
automatic check and is still reported by a check you asked for.

**Nothing is installed without you.** When there is a newer release you are shown its version
and its notes and choose. If you say yes, the installer is downloaded from the project's own
releases over HTTPS and **verified against the SHA-256 GitHub itself published** before it is
written anywhere it could be run from - a mismatch never reaches the disk. The URL is built
from constants rather than taken from a response body, the hash comes from Windows' own
BCrypt rather than from a dependency, and the comparison runs without an early exit.

## Diagnostics

Barometer is a GUI-subsystem binary, so it never flashes a console. The diagnostic modes
attach to the console they were launched from, if there is one, and print there.

| Command | What it does |
| --- | --- |
| `barometer.exe` | Draws on the taskbar. This is the program |
| `barometer.exe 6 --console` | Six samples of every module, printed and then exit. `0` runs until interrupted; the count defaults to 10 |
| `barometer.exe 6 path\to\barometer-sensors.exe --console` | The same, after dumping every device and sensor the helper reports |
| `barometer.exe --settings` | The settings window on its own, with no strip behind it |
| `barometer.exe --menu` | The right-click menu on its own, held open |
| `barometer.exe --marks sheet.bmp` | All fourteen weather marks on the taskbar's own dark, at 15 pixels (life size) and again at 60 |
| `barometer.exe --weather` | Locates by IP address and fetches a forecast, printing each step |
| `barometer.exe --reserve 4` | Adds four placeholder icons and reports what the tray did |
| `barometer.exe --install-lhm` | Fetches LibreHardwareMonitor, printing every step the settings pane draws as a bar |
| `barometer.exe --repair` | Abandons the current block of tray-icon identities for a fresh one, for the case where the shell has stopped drawing them. Nothing is deleted - deleting one of those registry entries is what makes an identity permanently unusable |

**Tracing.** Create an empty file named `trace.on` in `%LOCALAPPDATA%\Barometer` and Barometer
appends what the strip is doing to `trace.log` beside it: reservations, measurements, why the
readout moved or hid. It is switched on by a file rather than an environment variable because
the copy that misbehaves is usually the one the scheduled task started at sign-in, elevated,
with no terminal anywhere near it - and a file is something you can make in Explorer and
delete afterwards. `BAROMETER_TRACE` in the environment still works. Delete the file to stop;
the switch is read once at startup.

## Building from source

```
cargo build --release
```

That is the whole story for the readout: a Rust toolchain, nothing else.

Three crates:

| Crate | What is in it |
| --- | --- |
| `barometer-core` | The sampling engine: modules, sensors, weather, settings and the store. No UI |
| `barometer-app` (`crates/barometer`) | The strip window, the tray reservation, the flyouts and the settings window |
| `barometer` (`crates/barometer-bin`) | `main.rs` and nothing else |

The bin/lib split is not cosmetic: a build script's link arguments reach every target in its
package including test harnesses, and the manifest requires administrator, so a package with
tests in it asked for elevation before it could run one. `barometer-bin` therefore sets
`test = false` and every test lives in the other two.

**The dependency rule is `windows-sys` and `serde_json`, and nothing else.** Not for time, not
for HTTP, not for hashing: WinHTTP carries the one HTTPS request, BCrypt does SHA-256, and
`tar.exe` unpacks a zip. `serde_json` is the single exception, and it is there because the
settings file has to survive a hand-edit and a newer version's keys, and because the sensor
source and the GitHub release feed are both somebody else's JSON. Every `windows-sys` feature
in every `Cargo.toml` carries a comment saying what calls into it. `winresource` is a build
dependency only; it compiles the icon and the version block into the executable.

```
cargo test --workspace
```

Around 620 tests, written as full sentences describing behavior. They run unelevated.

The sensor helper needs a **.NET 10 SDK** and is a separate program on purpose, so this one
does not inherit that. .NET 10 is not a preference either: LibreHardwareMonitor publishes a
.NET Framework 4.7.2 build and a .NET 10 build and nothing between them, and only the matching
pair loads.

To build everything and package an installer:

```
powershell -File scripts\build.ps1
```

It rebuilds the icon if the artwork is newer, builds the Rust binary and the .NET helper,
stages both into `dist\Barometer`, runs [Inno Setup](https://jrsoftware.org/isinfo.php) over
the result and prints the installer's SHA-256. `-SkipHelper`, `-SkipInstaller` and `-Run` skip
or add steps. It also refuses to ship a binary carrying the path it was built on;
`scrub-check.ps1` is the same check against one binary at a time.

`version.ps1` reads or sets the version in the one place it is defined, and `scripts\build.ps1`
and the release workflow both go through it, so the installer, its filename and the
executable's own version resource cannot drift apart - which matters, because the updater
compares the running version against a release tag.

## License

GPL-3.0-only. See [LICENSE.md](LICENSE.md).

[NOTICE.md](NOTICE.md) has the full detail; in short:

- **[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)**,
  MPL-2.0. The helper is compiled against it and ships without a byte of it; the library is
  loaded at run time from an installation on your own machine. `helper/Program.cs` and
  `helper/LibraryLocator.cs` carry MPL-2.0 headers, and their source is in this repository.
- **[PawnIO](https://github.com/namazso/PawnIO)**, GPL-2.0, with its hardware modules under
  LGPL-2.1. Not bundled. It is a signed kernel driver you install yourself.
- **[Open-Meteo](https://open-meteo.com/)** for the weather, free for non-commercial use;
  the data is CC BY 4.0 by its providers, and the attribution is in the app.
- **[.NET](https://github.com/dotnet/runtime)**, MIT. `barometer-sensors.exe` is published
  self-contained, so the parts of the runtime it needs are inside it, redistributed
  unmodified. Nothing else in Barometer is .NET, and a machine with no .NET installed runs
  all of this.
- **[Barometer for macOS](https://github.com/mackid1993/Barometer)**, by the same author, is
  what this is a port of: the design, the module set, the stack model, the settings vocabulary
  and the fourteen weather marks come from it.
