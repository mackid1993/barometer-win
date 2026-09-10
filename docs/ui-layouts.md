# Barometer for Windows: screen layouts

Companion to `ui-design.md`, which defines every token, control and rule referenced here. All dimensions
are DIPs at 100 %; diagrams are not to scale but every number in them is real. `x` and `y` are measured
from the top-left of the region named in the diagram's caption.

**Drawn before the app was built.** The settings window that shipped has a page per module rather than
inspectors inside the composer, and several cards below were never made; each section says so where it
diverges rather than being deleted. `crates/barometer/src/settings_ui/panes.rs` is what the window
actually builds, and it is the authority where the two disagree.

Contents

1. Window frame and navigation
2. Strip pane: the composer
3. Module pages (CPU, GPU, Memory, Disks, Network, Sensors, Weather; ~~Battery~~, ~~Time~~ dropped;
   Combined became Stacks)
4. Appearance pane
5. ~~Taskbar pane~~ - not built
6. General pane
7. About pane
8. The strip itself
9. Flyout panels
10. Keyboard map

## 1. Window frame and navigation

```
880 Ã 640 (default), min 760 Ã 560. Origin: window client area.
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ y=0
â [icon16]  Barometer                                                    â      â    â title bar 32 (Mica; DWM caption buttons)
ââââââââââââââââ¬ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ¤ y=32
â  nav 176     â content layer (surface.layer), 8 radius on this corner only         â
â              â                                                                     â
â âStrip       â  x=176; pane padding 24 â content x from 200 to 856 (656 wide)      â
â  CPU         â                                                                     â
â  GPU         â                                                                     â
â  Memory      â                                                                     â
â  Disks       â                                                                     â
â  Network     â                                                                     â
â  Sensors     â                                                                     â
â  Weather     â                                                                     â
â  Stacks      â                                                                     â
â  Appearance  â                                                                     â
â  General     â                                                                     â
â  About       â  (About is pinned to the bottom of the nav, as in Settings apps)    â
ââââââââââââââââ´ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ y=640
```

Nav items (x 8â168, 36 tall, first at y = 40 + 8, 4 gap): **Strip**, then a page each for **CPU, GPU,
Memory, Disks, Network, Sensors, Weather**, then **Stacks**, **Appearance**, **General**, and **About**
pinned at the bottom (`y = window bottom â 8 â 36`) - `Pane::ALL`'s order, which is what Ctrl+Tab and the
arrow keys walk. That is twelve rather than the five drawn above: the module inspectors moved out of the
composer, for the reasons in `ui-design.md` Â§13. The module and Stacks rows carry their own mark in their
own color, which is what tells eight of the twelve apart at a glance; Strip, Appearance, General and About
are words alone. Selected item: `subtle.hover` fill +
3 Ã 16 `accent.pill` at x 8 (the item's left edge), vertically centered.

Order: Strip first because it is the pane people open the window for; General last before About because
"Start with Windows" is set once. The window reopens on the last pane, and opening Settings from a
flyout's gear goes to that module's own page.

Behavior

- Ctrl+Tab / Ctrl+Shift+Tab move between panes from anywhere. Up/Down inside the nav move the selection and
  switch panes immediately (no Enter needed).
- The content pane scrolls vertically when its content is taller than the window; the nav never scrolls
  (twelve items at 36 + 4 still fit at the minimum height, with About pinned under them). A 1 DIP
  `stroke.divider` appears at the top of the content pane while it is scrolled.
- Closing the window (Esc with no popup open, Alt+F4, the caption button) hides it; the app keeps running
  in the strip. Nothing needs saving because nothing is pending.
- Keyboard order: title bar (caption buttons) â nav â pane content in reading order.

## 2. Strip pane: the composer

```
Origin: content pane (x=0 at window x=176). Width 704; paddings 24. Shown at 880 Ã 640.
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ y=0
â  24                                                                                â
â  Strip                                                   Subtitle 20/28            â y=24
â  7 items Â· 9 pt text Â· two rows        (Caption, text.secondary, live)             â y=56
â                                                                                    â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â y=80  preview, 656 Ã 48
â  â PREVIEW  âCPU   MEM   â 12.3 KB/s   ââââââ  â 72Â°  â         [â¾]      â          â  brand.ground or taskbar estimate
â  â          â24%   61%   â  1.2 KB/s                    â                 â          â  flip button 32Ã32 at the right
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â y=128
â                                                                                    â
â  Items                        Body Strong           CPU               Body Strong  â y=152  (two column headers)
â  âââââââââââââââââââââââââââ   ââââââââââââââââââââââââââââââââââââââââââââââââââ  â y=180
â  âââ® [â£] CPU         [ââ] â   â [tile32] CPU                                    â  â  list 256 wide; inspector 384 wide; gap 16
â  â   Label over value      â   â          Utilization, per-core load, top        â  â
â  â â® [â£] Memory      [ââ] â   â          processes.                            â  â
â  â   Label over value      â   â                                                â  â
â  â â® [â£] Network     [ââ] â   â In the strip                                   â  â
â  â   Download over upload  â   â ââââââââââââââââââââââââââââââââââââââââââââââ â  â
â  â â® [â£] GPU         [ââ] â   â â Show in the strip                 On  [ââ] â â  â
â  â   Off                   â   â ââââââââââââââââââââââââââââââââââââââââââââââ¤ â  â
â  â â® [â£] Disks       [ââ] â   â â Readout            [Label over value    â¾] â â  â
â  â   Activity graph        â   â ââââââââââââââââââââââââââââââââââââââââââââââ¤ â  â
â  â â® [â£] Sensors     [ââ] â   â â Graph window       [1 minute            â¾] â â  â
â  â   Needs a sensor source â   â ââââââââââââââââââââââââââââââââââââââââââââââ¤ â  â
â  â â® [â£] Weather     [ââ] â   â â Graph width        ââââââââââââ  40 px     â â  â
â  â   Icon and temperature  â   â ââââââââââââââââââââââââââââââââââââââââââââââ â  â
â  â â® [â] Heat        [ââ] â   â                                                â  â
â  â   Stack: CPU Â· GPU      â   â Sampling                                       â  â
â  â â® [â] Traffic     [ââ] â   â ââââââââââââââââââââââââââââââââââââââââââââââ â  â
â  â   Off                   â   â â Interval           âââââââââââ  2 s        â â  â
â  â                         â   â â Shorter intervals use more CPU.            â â  â
â  â  (new stacks: Stacks)   â   â ââââââââââââââââââââââââââââââââââââââââââââââ â  â
â  âââââââââââââââââââââââââââ   â (no Colors card: ui-design.md 2.6 was not built)  â  â
â                                ââââââââââââââââââââââââââââââââââââââââââââââââââ  â
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
```

### 2.1 Header

- Title "Strip" (Subtitle) at y 24. Caption at y 56: `"{n} items Â· {size} pt text Â· {rows}"` - the size is
  always 9 and the rows are "two rows" or "one row", whichever the bar's height admits (`preview::summary`).
  `tnum`, updates as toggles change. This is the macOS sizing summary, made permanent instead of appearing
  in an apply bar; the graphics percentage it used to carry went with the size ladder.

### 2.2 Preview strip (y 80, 656 Ã 48, radius 8)

- Ground = the app's current taskbar estimate (`#202020`, `#EEEEEE`, or the accent), so the preview shows
  what the taskbar shows, including the backplate when it would be on. A 32 Ã 32 icon button at the right
  (sun / moon glyph, `text.secondary` on the ground's ink) flips the preview to the *other* theme so a
  color choice can be checked against both without changing Windows. The flip is preview-only and is
  forgotten when the window closes.
- The preview renders through the same code path as the taskbar strip, at the same size, with the same
  slack distribution for the current reservation. It is the real thing on a different canvas.
- Caption "PREVIEW" (Caption, 82 % ink, letter-spaced 0.8) at x 12; content centered in the remaining width;
  wider-than-available content scrolls horizontally inside the strip (no scrollbar; drag or wheel), never
  widens the window.

### 2.3 Item list (x 0â256, y 180 to pane bottom â 32; scrolls)

Rows 52 tall, one per strip item, in strip order.

```
Row, 256 Ã 52:
x=0   â accent pill (3 Ã 16) when selected
x=8   â® grip glyph E784, 16, text.tertiary; hover text.secondary
x=32  [â£] module tile 20 Ã 20 (12 glyph), module color; gray (control.disabled) when the row is off
x=60  Name       Body, text.primary  (Caption below: readout style, or the reason it cannot show)
x=204 [ââ] toggle 40 Ã 20, vertically centered; no On/Off word here (the caption carries state)
```

Caption rules (Caption, `text.secondary`; `status.neutral` dot before a reason):

| Situation | Caption |
| --- | --- |
| On, has data | the readout style ("Label over value") |
| Off | "Off" |
| On, cannot show | "Needs a sensor source" / "Needs a location" |
| On, stale | "Last updated 47 min ago" (Weather) |

Selection and the inspector: clicking a row (not its toggle) selects it and the inspector shows it. Toggle
clicks do not change selection. Arrow keys move selection; Space toggles the selected row; Enter moves
focus to the inspector.

Reordering:

- **Mouse**: press anywhere on the row except the toggle, move 4 DIP â drag. The row lifts (`surface.card`
  fill, 1 DIP `stroke.strong`, drawn on top), follows the pointer vertically, and the other rows shift to
  open the gap (120 ms slide, or instantly under reduced motion). Release drops; Esc cancels. The strip
  reorders on drop.
- **Keyboard**: Alt+Up / Alt+Down move the selected row; the strip reorders immediately. Announced through
  UIA as "CPU, position 2 of 9".
- There is no separate "move up/down" button pair. The keyboard path is documented in the row's UIA help
  text and in a Caption under the list that appears only while the list has keyboard focus:
  "Alt+â / Alt+â to reorder".

Which rows exist:

- Always the seven modules, in `ModuleId::ALL`'s order: CPU, GPU, Memory, Disks, Network, Sensors,
  Weather. **On for a fresh install: CPU and Memory** (`shown_on_a_fresh_install`).
- Any stacks the user has made, interleaved among them in strip order - `Settings::order` is what places
  one, and `modules` is kept in agreement with it.
- Battery and Time were dropped before either was built; Windows shows both already.

Stacks are made on the Stacks page rather than from an **Add** button here, and a module has one row and
one item however much it knows: Weather keeps several saved locations and puts the primary one on the
strip, and a second reading of anything - another sensor, another metric - is a reading in a stack.

Empty state (every row off): the list is unchanged; the preview strip shows Caption text "Nothing in the
strip. Turn on an item to start." in 82 % ink, centered; the header caption reads "0 items". The taskbar
reservation is released.

### 2.4 Inspector shell - moved out to a page per module

**Not in the Strip pane any more.** What is drawn here is what a module's own page carries (Â§3), minus the
Colors card, which was not built. The Strip pane's right-hand column is the Order header and its one
caption: "Drag an item to move it. The switch takes it off the strip without forgetting how it was set up."

```
ââââââââââââââââââââââââââââââââââââââââââââââââ
â [tile 32]  CPU                 Body Strong    â y=0   header, 48 tall
â            Utilization, per-core load,        â       subtitle: Caption, text.secondary, 2 lines max
â            and top processes.                 â
â                                              â 16
â In the strip                  section header â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Show in the strip                 On [ââ]â â 48   the same switch as the list row; both move together
â â Readout                [dropdown 160  â¾] â â 48
â â â¦module-specific rowsâ¦                   â â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â (module sections)                            â
â Sampling                                     â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Interval        ââââââââââââ   2 s       â â 64   slider + value label; caption "Shorter intervals use more CPU."
â ââââââââââââââââââââââââââââââââââââââââââââ â
â Colors                       Using Ocean âº   â section header + trailing hyperlink to Appearance
â ââââââââââââââââââââââââââââââââââââââââââââ â present only when Appearance âº "Use one palette for every module" is off
â â Text        [â  light] [â  dark]           â â 48   swatch buttons 36 Ã 24, radius 4, 1 stroke; click opens the swatch menu
â â Graph line  [â ] [â ]      Fill  [â ] [â ]   â â 48
â â Warning     [â ] [â ]  Critical  [â ] [â ]   â â 48
â â 4.6:1 on a light taskbar Â· 7.7:1 on dark â â 32   Caption, live, for the Text role
â ââââââââââââââââââââââââââââââââââââââââââââ â
â Remove this item                  (instances)â
ââââââââââââââââââââââââââââââââââââââââââââââââ
```

Inspector width 384; rows are 48 unless they carry a description (64). Dropdowns are 160 wide, right-
aligned; sliders take the width between the label column (120) and the value label (48).

Swatch menu (popup, 5.9 style): "Theme color" (the current theme's value for this role), "Windows accent",
eight fixed swatches, "Customâ¦" (opens the Win32 `ChooseColor` dialog: native, ugly, and the only color
picker that needs no code). Each entry shows its ratio against the current taskbar estimate in Caption.

## 3. Module pages

**These were drawn as inspectors inside the composer; each is a page of its own now** (Â§1), which is why
the widths quoted below are a 384 column and the real ones are the content pane's. What each page offers
is otherwise as listed, with the exceptions marked per section.

Only the rows that differ from the shell are listed. "Readout" options are the module's menu-bar modes from
the macOS app, renamed where Windows differs. Default in bold.

### 3.1 CPU

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | Percentage Â· **Label over value** Â· History graph Â· Per-core bars Â· Icon and value |
| Graph window | dropdown (graph modes only) | 30 s Â· **1 min** Â· 2 min Â· 5 min Â· 10 min |
| Graph width | slider 24â96, step 4 | **40** |
| Interval | slider 1â10 s | **2 s** |

Flyout: history graph (1 minâ24 h), per-core bars with P/E labels when the OS reports hybrid cores, load
(1/5/15 min is Unix-only: show "Processes Â· Threads Â· Handles Â· Uptime" instead), top 5 processes with
icons and an end-task glyph on hover.

### 3.2 GPU

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Percentage** Â· History graph Â· With CPU (draws inside the CPU item as a second row/column; the GPU item itself disappears) |
| Adapter | dropdown | **Automatic (busiest)** Â· each adapter by name |
| Measure | dropdown | **Busiest engine** Â· 3D engine |
| Interval | slider 1â10 s | **2 s** |

"Adapter" is hidden when there is one adapter. If no adapter exposes engine counters (very old drivers), the
list row caption reads "No GPU counters available" and the item is not drawn.

### 3.3 Memory

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | Used percentage Â· **Label over value** Â· History graph Â· Usage bar |
| Measure | dropdown | **In use** Â· Committed |
| Interval | slider 1â10 s | **2 s** |

Flyout: breakdown bar in Task Manager terms (In use, Standby, Modified, Free), committed/limit, paged and
non-paged pool, top 5 processes by working set.

### 3.4 Disks

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Activity graph** Â· Free space (percent) Â· Free space (bytes) Â· Rates with arrows |
| Volume | dropdown | **C: (Windows)** Â· every mounted volume with a letter, "Label (Letter) Â· size" |
| Units | dropdown | **Decimal (GB)** Â· Binary (GiB) |
| Interval | slider 1â10 s | **2 s** |

Removable volumes appear in the dropdown while mounted; if the chosen volume goes away, the item shows the
system volume and the row caption says "D: was removed; showing C:" until the user picks again. Flyout:
every volume with a capacity bar and an eject glyph for removable ones, per-physical-disk read/write rates.

### 3.5 Network

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Download over upload** Â· Rate arrows on one line Â· NET label over total Â· Activity graph |
| Interface | dropdown | **Automatic (Ethernet)** Â· each interface by friendly name |
| Unit | dropdown | **Bytes per second** Â· Bits per second |
| Decimal places | dropdown | 0 Â· **1** Â· 2 |
| Order | dropdown | **Download first** Â· Upload first |
| Graph scale | dropdown + field | **Automatic** Â· Fixed â a 32-tall field "10 MB/s" appears beside it |
| Show public address in the panel | toggle | **Off** (fetches ipify only when on) |
| Interval | slider 1â10 s | **2 s** |

Flyout: throughput graph, interface, local addresses with copy glyphs, gateway and DNS, Wi-Fi SSID, signal,
band and rate (via the WLAN API), VPN interfaces flagged. On Windows 11 24H2 and later the WLAN queries
need the Location permission; when it is off, the Wi-Fi card shows the same calm info strip as Weather's
"Use current location" row ("Wi-Fi details need Location, which is off for desktop apps." + "Open Privacy
settings") and the rest of the panel is unaffected.

### 3.6 Sensors (the calm empty state)

**The calm empty state is what shipped; the source model is not.** There is one source,
LibreHardwareMonitor, and nothing reads anybody's shared memory: the page offers to download the pinned
release into the user's own application data, says whether PawnIO is there and whether this process can
open it, and can remove the library again - all taking effect without a restart. HWiNFO is not supported,
so the two-button "we have no opinion" state below is one button and a sentence. Everything about the
tone, the gray dot and the wording is unchanged and is the point of this section.

```
ââââââââââââââââââââââââââââââââââââââââââââââââ
â [tile]  Sensors                              â
â         Temperatures, fans and voltages from â
â         a monitoring app.                    â
â                                              â
â Source                                       â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â â No sensor source found                 â â  status.neutral dot; Body, text.primary
â â                                          â â
â â Windows doesn't provide temperatures,    â â  Body, text.secondary, 3 lines, 16 padding
â â fans or voltages to apps, so Barometer   â â
â â reads them from a monitoring app when    â â
â â one is running. Install one, turn on its â â
â â shared-memory option, and readings show  â â
â â up here on their own.                    â â
â â                                          â â
â â [Get HWiNFO â]  [Get LibreHardwareMonitor â] â  standard buttons (not accent), E8A7 suffix
â â Free, and Barometer only reads from them.â â  Caption
â ââââââââââââââââââââââââââââââââââââââââââââ â
â                                              â
â Readings in the strip                        â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Available once a source is running.      â â  Caption, text.secondary
â â [+ Add reading â¾]  (disabled)            â â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â¦Readout / Formatting / Sampling as belowâ¦   â
ââââââââââââââââââââââââââââââââââââââââââââââââ
```

No warning glyph, no yellow, no "!" â the dot is gray because nothing is wrong. The paragraph explains the
mechanism once, in the second person, without apologizing. Both buttons are standard, side by side, equal
weight; the app does not have an opinion about which monitor to install. The caption under them answers
the two questions people actually have (does it cost money; what does it do to my PC).

Source card states:

| State | Dot | Line 1 | Line 2 (Caption) | Action |
| --- | --- | --- | --- | --- |
| Providing | success | "HWiNFO 8.12 is providing 23 readings" | "Shared memory Â· updated 2 s ago" | none |
| Installed, not running | neutral | "HWiNFO is installed but not running" | "Readings appear when it starts. Its Shared Memory Support setting must be on." | "Start HWiNFO" (standard) |
| Running, sharing off | neutral | "HWiNFO is running but not sharing readings" | "Turn on Settings âº Shared Memory Support in HWiNFO." | none |
| Sharing timed out (HWiNFO free edition, 12 h) | neutral | "HWiNFO stopped sharing readings" | "Its free edition shares for 12 hours at a time. Turn Shared Memory Support on again in HWiNFO." | none |
| Two sources running | success | "Using HWiNFO (LibreHardwareMonitor is also running)" | â | Source dropdown appears: **Automatic** Â· HWiNFO Â· LibreHardwareMonitor |
| None | neutral | as drawn above | | two Get buttons |

Detection runs every 10 s while this inspector is visible, every 60 s otherwise; the card updates in place.
The Sensors toggle in the list stays enabled in every state: turning it on records intent; the strip
simply does not draw the item until readings exist (design Â§9.7), and the list caption says why.

Remaining rows:

| Row | Control | Options |
| --- | --- | --- |
| Readings in the strip | chip list + "Add reading" dropdown | Detected readings grouped: Temperatures Â· Fans Â· Voltages Â· Power; each chip "CPU 51Â°" shows the live value; drag or Alt+â/â to reorder; Ã removes |
| Readout | dropdown | **Compact two-row stack** Â· Labels and values Â· History graph Â· Fan RPM |
| Decimal places | dropdown | **0** Â· 1 |
| Hide equivalent readings | toggle | **On** ("Combines sensors that report the same value under different names.") |
| Show advanced sensors | toggle | **Off** ("Includes undocumented identifiers meant for diagnostics.") |
| Interval | slider 2â30 s | **5 s** |

Temperature unit lives in Appearance âº Measurement units; the header of this inspector links to it
("Shown in Â°C Â· Change") so the user is never hunting.

Defaults on first source detection: the chip list is pre-filled with the hottest CPU die sensor and the
GPU temperature, which is what other monitors report and what the macOS app does.

### 3.7 ~~Battery (laptops only)~~ - dropped

**Dropped before it was built.** Windows already shows battery in the tray, with time remaining, and
battery *sensors* arrive through Sensors. Kept for the record.

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Glyph with percentage** Â· BAT label over percentage |
| Interval | slider 5â60 s | **10 s** |

Flyout: charge ring, state, health (full-charge / design capacity from the battery report), cycle count
where the firmware reports it, wattage in/out, adapter presence, charge history. No time estimates,
matching the macOS decision.

### 3.8 Weather

```
ââââââââââââââââââââââââââââââââââââââââââââââââ
â [tile]  Weather                              â
â         Conditions and forecasts from        â
â         Open-Meteo.                          â
â                                              â
â Locations                                    â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â â Austin                    Primary   â  â â 48  row: name Body; region Caption; "Primary" Caption tag; remove glyph
â â   Texas, United States                   â â
â ââââââââââââââââââââââââââââââââââââââââââââ¤ â
â â â Austin                    Set primary ââ â 48  hyperlink "Set primary" on non-primary rows
â â   Minnesota, United States               â â
â ââââââââââââââââââââââââââââââââââââââââââââ¤ â
â â Use current location              Off[ââ]â â 48  see states below
â ââââââââââââââââââââââââââââââââââââââââââââ â
â                                              â
â Add a location                               â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â [ð Search cities                      ] â â 32-tall search field, full card width minus 32
â ââââââââââââââââââââââââââââââââââââââââââââ â
â   ââââââââââââââââââââââââââââââââââââââââââ â results popup, 8 Ã 48 max, opens under the field
â   â Austin                          2.0 M  â â primary Body; secondary Caption; population right, text.tertiary
â   â Texas, United States                   â â
â   â Austin                          38 K   â â
â   â Minnesota, United States               â â
â   â Austin                          5.6 K  â â
â   â Arkansas, United States                â â
â   â Austin                          2.1 K  â â
â   â Manitoba, Canada                       â â
â   ââââââââââââââââââââââââââââââââââââââââââ â
â                                              â
â Units                                        â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Temperature        [Fahrenheit (Â°F)   â¾] â â
â â Wind               [Miles per hour    â¾] â â  mph Â· km/h Â· m/s Â· knots
â â Pressure           [Inches of mercury â¾] â â  inHg Â· hPa Â· mmHg
â â Precipitation      [Inches            â¾] â â  in Â· mm
â ââââââââââââââââââââââââââââââââââââââââââââ â
â                                              â
â Refresh                                      â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Every            ââââââââââââ 15 min     â â  slider 5â60, step 5
â â Updated 4 min ago Â· Refresh now          â â  Caption + hyperlink button
â ââââââââââââââââââââââââââââââââââââââââââââ â
â                                              â
â In the strip                                 â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Show in the strip                 On[ââ] â â
â â Readout            [Icon and temperatureâ¾]â â
â â Color weather icons               Off[ââ]â â
â â In color the sun is amber, night is      â â  Caption
â â lavender and rain is blue.               â â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â Panel details                                â
â ââââââââââââââââââââââââââââââââââââââââââââ â
â â Show                [All details      â¾] â â  All Â· Custom â checklist of sections appears
â ââââââââââââââââââââââââââââââââââââââââââââ â
â Weather data by Open-Meteo.com               â  Caption, text.secondary, hyperlink
ââââââââââââââââââââââââââââââââââââââââââââââââ
```

Search behavior

- Typing â¥ 2 characters starts a 300 ms debounce, then one geocoding request (`count=10`, language from the
  UI). While waiting, the field's right end shows the Caption "Searchingâ¦" in `text.secondary`; no spinner.
- Results popup: up to 8 rows; region = `admin1, country`; population right-aligned, abbreviated, in
  `text.tertiary`. Results are shown in the API's order (population-weighted), never re-sorted, so the big
  Austin is first.
- Down arrow moves from the field into the list; Enter adds the highlighted result; Esc closes the popup
  and keeps the text. Click adds. Adding clears the field, closes the popup, appends the row to Locations,
  and makes it primary if it is the first.
- States: no results â one non-interactive row "No matching places" (Body, `text.secondary`). Network
  failure â one row "Couldn't reach Open-Meteo. Check your connection and try again." Same text in the
  Refresh card's caption when a forecast fetch fails: "Couldn't update Â· showing the forecast from 09:12".
- Duplicate: adding a place already in the list highlights the existing row (`subtle.hover` for 1 s under
  motion, or a Caption "Already added" beside it) instead of adding twice.

Use current location

**On by default**, which is the one place the two apps differ on purpose: macOS resolves this through
CoreLocation and would prompt somebody who has not asked for weather yet, while Windows resolves it from
the IP address - no prompt, no precision worth worrying about - and defaulting it off would mean a fresh
install shows an empty weather column that looks broken. There is no Windows Location permission in this
path at all.

| State | Toggle | Caption |
| --- | --- | --- |
| Off | Off | "Uses your internet address to follow you." |
| On, allowed | On | "Near Austin, Texas" once resolved |
| On, location off for desktop apps | Off, disabled | Info strip under the row: "Location is off for desktop apps in Windows." + hyperlink "Open Privacy settings" (`ms-settings:privacy-location`) |
| On, no fix yet | On | "Finding your locationâ¦" (Caption), then the place |

Empty state (no locations, current location off): the Locations card holds one row "Add a city below to
start Weather." The list row caption says "Needs a location". The strip does not draw the item.

Readout options: Condition mark over temperature Â· **Icon and temperature** Â· Temperature only Â· Icon,
temperature and conditions Â· High and low Â· Rain chance Â· Custom template ("`{temp}` `{cond}` `{hi}` `{lo}`
`{pop}` `{wind}` `{aqi}`" listed in a Caption under a 32-tall field that appears when chosen).

Units default to Â°F/mph/inHg/in (`WeatherUnits::IMPERIAL`) wherever the machine is; deriving them from
the Windows region is not built, and `METRIC` is beside it for the day it is. Refresh 15 min. Weather refreshes on resume from sleep and on network change as
on macOS. The last forecast is cached and shown with the stale rule (design Â§9.7).

### 3.9 ~~Time~~ - dropped

**Dropped before it was built.** Windows draws its own clock on the same taskbar, and a second one spends
strip width duplicating the shell. Kept for the record.

| Row | Control | Options |
| --- | --- | --- |
| Show in the strip | toggle | **Off** ("Windows shows its own clock. Time is for seconds, a second time zone or a different format.") |
| Format | text field, 200 | **`h:mm`**; Caption lists tokens: `h H mm ss a ddd d MMM yyyy z`; live sample to the right "9:41" |
| Show seconds | toggle | **Off** (interval becomes 1 s) |
| World clocks | chip list + "Add clock" dropdown (time zones grouped by region, searchable) | none |
| Week starts on | dropdown | **System default** Â· Sunday Â· Monday |
| Text size | dropdown | **Same as the strip** Â· 9 Â· 10 Â· 11 Â· 12 |

Flyout: month calendar (today highlighted with `accent.fill`), world clocks with offsets and day/night
marks, sunrise/sunset from the primary weather location. No notifications list and no calendar events:
Windows does not expose either to a desktop app, and the inspector says nothing about it because absent
features do not need apologies.

### 3.10 Combined - became Stacks, on a page of its own

**Renamed and reshaped.** A stack is not a group of modules, it is a column of *readings* (`StackMetric`),
which is what lets it carry one CPU figure, one GPU temperature and one LibreHardwareMonitor sensor
together. The Stacks page is a list of stacks (256 wide) with **New stack** under it and an editor beside
it; its rows are:

| Row | Control | Options |
| --- | --- | --- |
| Show this stack | toggle | on |
| Name | text field | free text; empty is allowed and the stack is named by its readings |
| Layout | dropdown | one row or two; two pairs the readings, which is what makes a stack cheaper than separate columns |
| Readings | a row per reading, with a freeform label each, plus an "Add a reading" dropdown | every reading of every module, and every sensor by identifier |
| Hide these modules' own columns | toggle | off; enabled once the stack has readings |
| Remove this stack | hyperlink | |

The rows below are what the Combined item was going to be, kept for the record.

| Row | Control | Options |
| --- | --- | --- |
| Name | text field | **"Combined"** (shown in the list and flyout title; rename freely) |
| Members | checklist with grips, in order | CPU Â· GPU Â· Memory Â· Disks Â· Network Â· Sensors Â· Battery Â· Weather Â· Time; **CPU, Memory** on by default |
| Separators | toggle | **On** (1 DIP line between members) |
| Hide members' own items | toggle | **On** ("The items above stay off while they are in this group.") |

Members draw with their own readout style and colors. The group counts as *one* item for automatic sizing,
which is the reason to use it on Windows: four readings in one Combined item keep the strip at 12 pt where
four separate items would still, but seven would not. Flyout: tabs across the top (36 tall, Body,
selected tab underlined 2 DIP `accent.fill`), one member per tab, each tab the member's own panel.

## 4. Appearance pane

**Two cards shipped, not five.** Appearance is "Type and spacing for the strip. Every change shows in the
preview and on the taskbar", and holds:

| Card | Row | What it is |
| --- | --- | --- |
| Text | Font | The family the strip draws in |
| | Heading font | A face of its own for the names on the strip, or the same family. On Windows a weight is often a family - Segoe UI Semibold is its own - so a heading can be a different face rather than only a heavier one |
| | Weight | Headings and values side by side, one dropdown each, offering only the weights that family has faces for |
| Spacing | Between columns | Slider 0â24, **default 3** |

The Theme tiles, the Colors card and the graph-opacity row were not built (`ui-design.md` Â§2.6), and
Measurement units did not land here: the sensors' temperature unit and decimals are on the Sensors page,
deliberately separate from the weather's, which are on Weather's. The diagram below is the plan.

```
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
â  Appearance                                                                        â y=24
â  Colors, type and spacing for the strip and its panels.                           â y=56
â                                                                                    â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â y=80  preview strip, as in Â§2.2
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â y=128
â                                                                                    â
â  Theme                                                                             â y=152
â  âââââââââââ âââââââââââ âââââââââââ âââââââââââ âââââââââââ âââââââââââ          â y=180  tiles 100 Ã 72, 8 gap (640 total)
â  ââCPU 24% â â CPU 24% â â CPU 24% â â CPU 24% â â CPU 24% â â CPU 24% â          â        each tile: mini strip sample 100 Ã 40 on brand.ground,
â  â System  â â Ocean   â â Sunset  â â Forest  â â Neon    â â Custom  â          â        name Caption below; selected = accent pill + 2 DIP accent stroke
â  âââââââââââ âââââââââââ âââââââââââ âââââââââââ âââââââââââ âââââââââââ          â y=252
â                                                                                    â
â  Text                                                                              â y=276
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Weight                                          [Medium          â¾] â          â 48  Regular Â· Medium Â· Semibold
â  â Graph opacity                          âââââââââââââ   30 %          â          â 48
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Strip                                                                             â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Spacing between items                           [Normal          â¾] â          â 48  Normal (12) Â· Snug (8) Â· Tight (4)
â  â Dividers between items                                    Off [ââ]  â          â 48
â  â Backplate                                       [Automatic       â¾] â          â 64  Automatic Â· On Â· Off; caption: "A dark ground behind the strip. Automatic turns it on when the taskbar is light or colored."
â  â Shrink items to fit the current reading                   Off [ââ]  â          â 64  caption: "Off keeps every item at a fixed width so nothing moves when a number changes."
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Colors                                                                           â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Use one palette for every module                           On [ââ]  â          â 64  caption: "Off lets each item set its own colors in the Strip pane."
â  â Text          [â  light] [â  dark]     4.6:1 light Â· 7.7:1 dark        â          â 48  swatches 36 Ã 24; live ratios Caption
â  â Graph line    [â ] [â ]                                                â          â 48
â  â Graph fill    [â ] [â ]                                                â          â 48
â  â Warning       [â ] [â ]                                                â          â 48
â  â Critical      [â ] [â ]                                                â          â 48
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Measurement units                                                                 â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Temperature                                     [Celsius (Â°C)    â¾] â          â 64  caption: "Used by Sensors, GPU and Battery. Weather has its own units."
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
```

Behavior

- Theme tiles are a radio group (one Tab stop, arrow keys move, Space selects). Choosing a preset writes all
  ten roles (the macOS `applyTheme`) and turns "Use one palette" on; System also sets monochrome. Editing
  any swatch afterwards switches the tile to Custom without asking.
- The Colors card is disabled with the caption "The System theme matches the taskbar's own text. Pick
  another theme to use colors." while System is selected. The swatch rows are hidden (not disabled) when
  "Use one palette" is off, replaced by a Caption "Each item sets its own colors in the Strip pane."
- There is no text size control; the strip draws at one size (design Â§9.4).
- The preview strip reflects every change on this pane instantly, and so does the taskbar.

## 5. ~~Taskbar pane~~ - not built

**There is no Taskbar pane.** Reserving space is not a setting anybody chooses: without the reservation
there is no readout at all, so a toggle for it would be a toggle for the program. A side-docked taskbar is
refused at startup with a message rather than reported in a pane, and the "Combine taskbar buttons" tip
below never found a home. The whole section is kept because the diagram explains the mechanism better than
the prose anywhere else does.

```
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
â  Taskbar                                                                           â y=24
â  How Barometer fits into the taskbar.                                              â y=56
â                                                                                    â
â  Space in the notification area                                                    â y=80
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â  Off                                                                 â          â 20  Caption labels above each mini taskbar
â  â  âââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ   â          â
â  â  â â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢â¢â¢ CPU 24% â âª â 9:41 â   â          â 28  mini taskbar 600 Ã 28: buttons as 20 Ã 16 rounded rects,
â  â  âââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ   â          â     the strip in brand.cyan, the tray as glyphs; overlap drawn where it happens
â  â  On                                                                  â          â
â  â  âââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ   â          â
â  â  â â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ â¢ Â»    CPU 24%  â âª â 9:41 â   â          â
â  â  âââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ   â          â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ¤          â
â  â Reserve space for the strip                                On [ââ]   â          â 64
â  â Windows lays the taskbar out as if Barometer weren't there, so when â          â Body, text.secondary, wraps; 16 padding
â  â many windows are open the task buttons run underneath the strip.    â          â
â  â Reserving space tells Windows the strip is there, and the buttons    â          â
â  â stop short of it. Windows may move a few buttons into its Â» overflowâ          â
â  â to make room.                                                        â          â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ¤          â
â  â â Reserving 5 slots (210 px) for a 202 px strip                      â          â 40  status line, live; success dot
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Tip                                                                               â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Windows frees space one whole task button at a time, so a small gap  â          â 64  Body text.secondary
â  â can remain beside the strip. It is smallest when Taskbar settings âº  â          â
â  â Combine taskbar buttons is set to Always.       [Open Taskbar settings â] â     â standard button, ms-settings:taskbar
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
```

The diagram is two rows of Direct2D rectangles and a handful of glyphs, drawn once. It carries the whole
explanation: in the "Off" row the last three buttons are drawn under the strip; in the "On" row they stop,
one has become the `Â»` overflow, and the strip has room. It is not animated.

Toggle states

| Condition | Toggle | Status line (Caption, `text.secondary`, neutral dot) |
| --- | --- | --- |
| Available, on | On | "Reserving 5 slots (210 px) for a 202 px strip" (`tnum`, live) |
| Available, off | Off | "The strip may be overlapped when the taskbar is full." |
| Vertical taskbar | disabled, shows Off | "Not available on a vertical taskbar." |
| Windows 10 taskbar | disabled | "Not available on the Windows 10 taskbar; the strip is placed beside the clock instead." |
| Another shell owns the taskbar (ExplorerPatcher, StartAllBack, etc.) | disabled | "Not available while another app is managing the taskbar." |
| Registration failing (Explorer restarting, shell not ready) | on, dot neutral | "Waiting for the taskbarâ¦" then retries with the existing backoff |

Disabled reasons are stated as facts about the environment, never as errors. Nothing here uses caution or
critical color. There is no "position" row (the strip lives left of the notification area; that is the
only place Windows 11 offers) and no "show on all taskbars" row (secondary taskbars have no notification
area); if either becomes possible the row goes here.

## 6. General pane

**Startup, Updates and Settings file shipped; Sampling did not.** The caption is "When Barometer starts,
and how it keeps itself current." Startup is one toggle, "Start Barometer when I sign in", which registers
a **logon task** rather than a `Run` key - Barometer runs elevated and Windows will not start an elevated
program from the ordinary startup list. Updates is the version row with **Check now**, the automatic-check
toggle, and a "Skipping 1.2.3" row with **Stop skipping** when a release has been skipped. Settings file is
two buttons, **Export…** and **Import…**, which write and read the same JSON document the store keeps,
through the common file dialogs; a caption under them reports where the file went or why it was refused.
There is no per-battery sampling row and no reset. The diagram below is the plan.

```
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
â  General                                                                           â
â  Startup, sampling and updates.                                                    â
â                                                                                    â
â  Startup                                                                           â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Start with Windows                                          On [ââ] â          â 48 / 64 with a reason
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Sampling                                                                          â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Reduce sampling on battery                                  On [ââ] â          â 64  laptops only; caption "Doubles every interval while unplugged."
â  â Sampling pauses while the screen is off.                             â          â 40  Caption only, no control
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Updates                                                                           â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Barometer 1.0.0                            [Check for updates]       â          â 64  version Body; caption "Checked today at 09:12 Â· Up to date"
â  â Check automatically                                         On [ââ] â          â 64  caption "Once a week. Nothing is installed without you."
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Settings file                                                                     â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â Export and import your settings as a file.   [Exportâ¦]  [Importâ¦]   â          â 64  caption "Contains display and sampling preferences, no system data."
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â                                                                                    â
â  Reset all settingsâ¦                                                               â  hyperlink button, status.critical text; confirmation dialog
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
```

Start with Windows states

| State | Toggle | Caption |
| --- | --- | --- |
| Registered (a logon task named Barometer exists) | On | â |
| Not registered | Off | â |
| Registered but disabled in Windows Settings âº Apps âº Startup | disabled, shows Off | "Turned off in Windows Settings âº Apps âº Startup." + hyperlink "Open Startup settings" (`ms-settings:startupapps`) |
| Blocked by policy | disabled | "Managed by your organization." |

Updates states (the version row's caption): "Checkingâ¦" â "Up to date Â· checked just now" / "1.1.0 is
available" with the button becoming an accent **Download 1.1.0** (opens the release page; nothing is
installed silently) / "Couldn't check Â· try again later". Per-module intervals are in each inspector; there
is no global interval slider here because the macOS "global sampling rate" override confused the per-module
ones and the inspector already shows each.

Reset: a small centered dialog (320 wide, Body Strong title "Reset all settings?", Body "The strip goes back
to CPU, Memory and Network, and the System theme.", buttons [Reset] (accent) [Cancel]). Esc cancels.

## 7. About pane

```
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
â  About                                                                             â
â                                                                                    â
â  ââââââ  Barometer                                    Subtitle                     â  icon 64 (the dark square with the cyan arc)
â  â â ~ â  1.0.0 (2026-09-07)                           Caption, tnum                â
â  ââââââ  A system monitor for the Windows taskbar.    Body                         â
â                                                                                    â
â  [Source code â]  [License â]                                                      â  standard buttons in a card
â                                                                                    â
â  Credits                                                                           â
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
â  â LibreHardwareMonitor                                  Project page â â          â  MPL 2.0. Not included; downloaded on request
â  â PawnIO                                                Project page â â          â  GPL 2.0, LGPL 2.1 modules. Installed by you
â  â Open-Meteo                                          open-meteo.com â â          â  CC BY 4.0
â  ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ          â
ââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââââ
```

The masthead reads "Version {v} Â· Free software under the GNU GPL 3.0" over "A system monitor for the
Windows taskbar, ported from Barometer for macOS by the same author." The macOS app is a separate
repository, not a folder of this one, and the license button opens the GPL rather than a text card in the
pane. HWiNFO is not a source; temperatures come from LibreHardwareMonitor, which Barometer will fetch and
never ships.

## 8. The strip itself

### 8.1 Anatomy at 100 %

Drawn at the 12 DIP gap this document originally specified. **The gap is now 3 by default and the end
padding 0**, and the slack is not spread into the gaps: the content is centered in the reserved region, so
the slack falls at the two ends as margin (`ui-design.md` Â§9.2, Â§9.6). The shape is right; halve the
horizontal numbers twice and it is today's strip.

```
Taskbar, 48 tall. The strip is the reserved region; here 5 slots = 210 px for 202 px of content, slack 8.
x=0                                                                                   x=210
ââââ¬âââââââ¬âââââ¬âââââââ¬âââââ¬âââââââââââââ¬âââââ¬ââââââââââââ¬âââââ¬ââââââââ¬âââ
â4 â CPU  â 12 â MEM  â 12 â â 12.3 KB/sâ 12 â ââââââââ  â 12 â â 72Â° â4 â    â gaps get +1 each from slack; 2 to each edge
â  â 24%  â    â 61%  â    â â  1.2 KB/sâ    â           â    â       â  â
ââââ´âââââââ´âââââ´âââââââ´âââââ´âââââââââââââ´âââââ´ââââââââââââ´âââââ´ââââââââ´âââ
     30          30           72             40 (graph)      50
```

Vertical placement (design Â§9.3): labels on row 1 (baseline y 20), values on row 2 (baseline y 36); single-
row items centered (baseline y 28.5); the graph spans y 8â40.

### 8.2 The readout styles, drawn

```
Percentage          Label over value      History graph        Per-core bars (8)      Icon and value
ââââââââ            ââââââââ              ââââââââââââ         âââââââââââââââââ      ââââââââââââ
â      â            â CPU  â 82 % ink     â ââââââââ â line 1  â ââââââââ      â 3 w  â [â£] 24%  â mark 16 + 4 + value
â 24%  â centered    â 24%  â 100 % ink    â ââââââââ â fill    â               â 1 gapâ          â
ââââââââ            ââââââââ              ââââââââââââ         âââââââââââââââââ      ââââââââââââ

Usage bar (Memory)  Download over upload  Rate arrows, one line   Condition over temperature   ~~Battery~~ - dropped
ââââââââ            ââââââââââââââ        âââââââââââââââââââ     ââââââââ                    ââââââââ
â MEM  â            â â 12.3 KB/sâ        â â12.3K  â1.2K   â     â  â   â mark 16            â[â®â®â® ]â dropped with the module
â ââââ â 6 tall     â â  1.2 KB/sâ        â                 â     â 72Â°  â                    â  84  â
ââââââââ            ââââââââââââââ        âââââââââââââââââââ     ââââââââ                    ââââââââ

A stack of three readings (CPU Â· MEM Â· NET):
ââââââââ¬ââ¬âââââââ¬ââ¬âââââââââââââ
â CPU  â â MEM  â â â 12.3 KB/sâ   one column of the strip, the readings laid out inside it
â 24%  â â 61%  â â â  1.2 KB/sâ
ââââââââ´ââ´âââââââ´ââ´âââââââââââââ
```

Arrows are the U+2193/U+2191 glyphs of the text font at the item's size, not icons, so they sit on the
baseline and take the ink. Units are abbreviated ("K", "M"), the strip being at the bottom of the ladder.

### 8.3 Widths and slots at four scales

The example strip above (CPU, MEM, NET, CPU graph, Weather), worked at the 12 DIP gap this document
originally specified. Today's default gap of 3 makes the content narrower and can drop a slot; the
reasoning is what this table is for.

| Scale | Content | Slot pitch | Slots | Reserved | Slack | Text px |
| --- | --- | --- | --- | --- | --- | --- |
| 100 % | 202 px | 42 | 5 | 210 | 8 | 9 |
| 125 % | 253 px | 52 | 5 | 260 | 7 | 11 |
| 150 % | 303 px | 63 | 5 | 315 | 12 | 14 |
| 200 % | 404 px | 84 | 5 | 420 | 16 | 18 |

Slot pitch is seeded at 42 and then measured from where the placeholders actually landed, never assumed;
the table shows typical values. Slack never exceeds one slot and is always absorbed inside the strip,
split between its two ends (design Â§9.2). Text px is `TEXT_DIP` at the scale factor.

### 8.4 States

```
Rest                 Hover (pointer on MEM)           Open (MEM flyout showing)        Stale weather
CPU   MEM   ââ¦       CPU  â­âââââ®  ââ¦                  CPU  â­âââââ®  ââ¦                  â 72Â°   (value at 70 %)
24%   61%   ââ¦       24%  âMEM â  ââ¦                  24%  âMEM â  ââ¦
                          â61% â                           â61% â
                          â°âââââ¯ strip.hover, r4           â°âââââ¯ strip.open
```

Sensors without a source, Weather without a location: not drawn, no placeholder; the
reservation shrinks to what remains.

### 8.5 Legibility summary

| Taskbar | System theme | Color theme, Backplate Automatic | Color theme, Backplate Off |
| --- | --- | --- | --- |
| Dark | white ink, 16:1 | dark role values on the bare taskbar, 6.5â12:1 | same |
| Light | `#1B1B1B` ink, 15:1 | plate on; dark role values on `#2D2F32`, â¥ 4.85:1 | darkened light variants, â¥ 4.5:1 text |
| Accent-tinted | white or black, whichever is higher (5.7:1 on the default blue) | plate on; â¥ 6.9:1 | dark role values; the swatch caption warns when a role falls under 4.5:1 |
| High contrast | system window text on system window color, opaque | themes ignored | themes ignored |

## 9. Flyout panels

### 9.1 Anatomy (CPU shown), 320 wide

```
ââââââââââââââââââââââââââââââââââââââââââââ  8 radius, acrylic, 1 stroke; 12 panel padding
â ââââââââââââââââââââââââââââââââââââââââ â
â â [â£] CPU                        24 %  â â  header card, tinted CPU color @ 12 %; Title 28 tnum right
â â     8 cores Â· 2.4 GHz Â· up 3 d 4 h   â â  Caption
â ââââââââââââââââââââââââââââââââââââââââ â  10 gap
â ââââââââââââââââââââââââââââââââââââââââ â
â â History                 [1 min   â¾]  â â  card; dropdown 120 in the corner
â â â­âââââââââââââââââââââââââââââââââ®   â â
â â â        ââââââââââââââââ        â   â â  graph 296 Ã 72; line 1 DIP, fill 30 %; hairline axis
â â â°âââââââââââââââââââââââââââââââââ¯   â â
â â 0 %                            100 % â â  Caption ticks
â ââââââââââââââââââââââââââââââââââââââââ â
â ââââââââââââââââââââââââââââââââââââââââ â
â â Cores                                â â
â â ââââââââ  P0 P1 P2 P3  E0 E1 E2 E3   â â  bars 8 Ã 24, labels Caption
â ââââââââââââââââââââââââââââââââââââââââ â
â ââââââââââââââââââââââââââââââââââââââââ â
â â Top processes                        â â
â â [ic] Firefox                  18 %  ââ â  rows 36; end-task glyph on hover; value tnum
â â [ic] Code                      6 %   â â
â â [ic] Explorer                  2 %   â â
â ââââââââââââââââââââââââââââââââââââââââ â
ââââââââââââââââââââââââââââââââââââââââââââ¤  hairline
â [â]                              (empty)â  footer 40: gear at left; module actions at right
ââââââââââââââââââââââââââââââââââââââââââââ
        â² centered on the CPU item; bottom edge 8 above the taskbar
```

Panel height = content; scrolls past min(720, work area â 16). Weather's panel (conditions header, 48-hour
strip with the temperature curve and rain bars, 10-day rows, sun and moon, air quality, details grid,
location switcher, Refresh in the footer, attribution Caption) is the tallest and scrolls. Combined shows
a tab row (36) under the header.

### 9.2 Placement

```
Bottom taskbar (default)                  Item near the right edge                   Top taskbar
                                                                                     âââââââââ taskbar âââââââââ
        ââââââââââââ                                        ââââââââââââ             â        CPU  MEM         â
        â  panel   â                                        â  panel   â             ââââââââââââ¬âââââââââââââââ
        â          â                                        â          â  â clamped:           â 8
        ââââââ¬ââââââ                                        ââââââââ¬ââââ   right edge      ââââââ´ââââââ
             â 8                                                   â 8     = work area     â  panel   â
ââââââââââââââ´âââââââââââââ                    âââââââââââââââââââââ´ââââ   â 8             ââââââââââââ
â        CPU  MEM   â 9:41â                    â            CPU  MEM â â
âââââââââââââââââââââââââââ                    âââââââââââââââââââââââââ
```

- Anchor rect = the clicked item's rect in screen pixels (from the strip's own layout).
- Preferred: horizontally centered on the anchor; vertically 8 DIP away from the taskbar edge on the
  desktop side.
- Clamp to the monitor work area inset 8; the panel never covers the taskbar. The anchor keeps its open
  backplate, which is the visual tether when clamping has moved the panel.
- Left/right taskbars: the panel sits 8 beside the taskbar, vertically centered on the anchor, clamped.
- Multiple monitors: the panel opens on the monitor that owns the taskbar the strip is on (the primary).
- DPI: the panel HWND is per-monitor DPI aware and is created at the strip's monitor DPI.

### 9.3 Dismissal

| Trigger | Result |
| --- | --- |
| Pointer down outside the panel (anywhere, including the taskbar) | closes; if the pointer is on another strip item, that item's panel opens in its place |
| Pointer down on the same item | closes |
| Esc | closes, focus returns to nothing (the taskbar keeps its state) |
| Panel loses activation (`WA_INACTIVE`) for any reason: Alt+Tab, Start menu, another window | closes |
| The item is turned off, removed, or the strip re-lays out because of a DPI or taskbar change | closes |
| Screen lock, sleep | closes |

The panel is a foreground window while open (so keyboard works) and is `WS_EX_TOOLWINDOW` so it never
appears in Alt+Tab or the taskbar. Opening is 150 ms fade + 8 DIP slide from the taskbar; closing 100 ms
fade; both 0 under reduced motion. Nothing inside the panel animates except graph redraws on new samples.

### 9.4 Keyboard and accessibility

Tab moves through the time-range dropdown, process rows (Enter = end task after a one-line inline confirm
"End Firefox?" [End] [Keep] replacing the row), copy glyphs, tabs (Combined) and the footer. Arrow keys move
within a list; Left/Right switch Combined tabs. The panel is a UIA `Pane` named "CPU details"; graphs expose
their current value and range as text.

## 10. Keyboard map

| Where | Keys | Action |
| --- | --- | --- |
| Anywhere in Settings | Ctrl+Tab / Ctrl+Shift+Tab | Next / previous pane |
| | F6 | Cycle nav â list â inspector (Strip pane) or nav â content |
| | Esc | Close popup; else close the window |
| | Alt+F4 | Close the window |
| Nav | Up / Down, Home / End | Move and switch pane |
| Composer list | Up / Down | Select item (inspector follows) |
| | Space | Toggle the selected item |
| | Alt+Up / Alt+Down | Move the selected item |
| | Enter | Focus the inspector |
| | Delete | Remove an added instance (asks) |
| Chips | Left / Right | Move between chips; Alt+Left / Alt+Right reorders; Delete removes |
| Dropdown | Alt+Down, Space | Open; Up/Down move; Enter commit; Esc revert; typing jumps |
| Slider | Left / Right (1), PageUp / PageDown (5), Home / End | Adjust; the value label updates live |
| Text field | Enter commit, Esc revert | |
| Search field | Down | Into the results; Enter adds; Esc closes results |
| Theme tiles | Left / Right, Space | Radio group |
| Strip (UIA / Narrator) | Enter or Space on an item | Open its panel |
| | Shift+F10 / Menu key | Context menu for the item |
| Flyout | Tab, arrows, Esc | Â§9.4 |
