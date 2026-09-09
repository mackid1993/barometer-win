# Barometer for Windows: screen layouts

Companion to `ui-design.md`, which defines every token, control and rule referenced here. All dimensions
are DIPs at 100 %; diagrams are not to scale but every number in them is real. `x` and `y` are measured
from the top-left of the region named in the diagram's caption.

Contents

1. Window frame and navigation
2. Strip pane: the composer
3. Inspectors, module by module (CPU, GPU, Memory, Disks, Network, Sensors, Battery, Weather, Time, Combined)
4. Appearance pane
5. Taskbar pane
6. General pane
7. About pane
8. The strip itself
9. Flyout panels
10. Keyboard map

## 1. Window frame and navigation

```
880 × 640 (default), min 760 × 560. Origin: window client area.
┌────────────────────────────────────────────────────────────────────────────────────┐ y=0
│ [icon16]  Barometer                                                    ─      ✕    │ title bar 32 (Mica; DWM caption buttons)
├──────────────┬─────────────────────────────────────────────────────────────────────┤ y=32
│  nav 176     │ content layer (surface.layer), 8 radius on this corner only         │
│              │                                                                     │
│ ▌Strip       │  x=176; pane padding 24 → content x from 200 to 856 (656 wide)      │
│  Appearance  │                                                                     │
│  Taskbar     │                                                                     │
│  General     │                                                                     │
│              │                                                                     │
│              │                                                                     │
│              │                                                                     │
│  About       │  (About is pinned to the bottom of the nav, as in Settings apps)    │
└──────────────┴─────────────────────────────────────────────────────────────────────┘ y=640
```

Nav items (x 8–168, 36 tall, first at y = 40 + 8, 4 gap): **Strip, Appearance, Taskbar, General** and
**About** pinned at the bottom (`y = window bottom − 8 − 36`). No icons: five words scan faster than five
invented glyphs, and the accent pill + hover fill already make it unmistakably a Win11 nav. Selected item:
`subtle.hover` fill + 3 × 16 `accent.pill` at x 8 (the item's left edge), vertically centered.

Order: Strip first because it is the pane people open the window for; General last because "Start with
Windows" is set once. The window reopens on the last pane, and opening Settings from a flyout's gear
selects that item in the Strip composer.

Behavior

- Ctrl+Tab / Ctrl+Shift+Tab move between panes from anywhere. Up/Down inside the nav move the selection and
  switch panes immediately (no Enter needed).
- The content pane scrolls vertically when its content is taller than the window; the nav never scrolls
  (five items fit at the minimum height). A 1 DIP `stroke.divider` appears at the top of the content pane
  while it is scrolled.
- Closing the window (Esc with no popup open, Alt+F4, the caption button) hides it; the app keeps running
  in the strip. Nothing needs saving because nothing is pending.
- Keyboard order: title bar (caption buttons) → nav → pane content in reading order.

## 2. Strip pane: the composer

```
Origin: content pane (x=0 at window x=176). Width 704; paddings 24. Shown at 880 × 640.
┌────────────────────────────────────────────────────────────────────────────────────┐ y=0
│  24                                                                                │
│  Strip                                                   Subtitle 20/28            │ y=24
│  7 items · 12 pt text · 90 % graphics  (Caption, text.secondary, live)             │ y=56
│                                                                                    │
│  ┌──────────────────────────────────────────────────────────────────────┐          │ y=80  preview, 656 × 48
│  │ PREVIEW  ▏CPU   MEM   ↓ 12.3 KB/s   ▂▃▅▇▅▃  ☀ 72°  ▏         [☾]      │          │  brand.ground or taskbar estimate
│  │          ▏24%   61%   ↑  1.2 KB/s                    ▏                 │          │  flip button 32×32 at the right
│  └──────────────────────────────────────────────────────────────────────┘          │ y=128
│                                                                                    │
│  Items                        Body Strong           CPU               Body Strong  │ y=152  (two column headers)
│  ┌─────────────────────────┐   ┌────────────────────────────────────────────────┐  │ y=180
│  │▌⋮ [▣] CPU         [●━] │   │ [tile32] CPU                                    │  │  list 256 wide; inspector 384 wide; gap 16
│  │   Label over value      │   │          Utilization, per-core load, top        │  │
│  │ ⋮ [▣] Memory      [●━] │   │          processes.                            │  │
│  │   Label over value      │   │                                                │  │
│  │ ⋮ [▣] Network     [●━] │   │ In the strip                                   │  │
│  │   Download over upload  │   │ ┌────────────────────────────────────────────┐ │  │
│  │ ⋮ [▣] GPU         [━○] │   │ │ Show in the strip                 On  [●━] │ │  │
│  │   Off                   │   │ ├────────────────────────────────────────────┤ │  │
│  │ ⋮ [▣] Disks       [●━] │   │ │ Readout            [Label over value    ▾] │ │  │
│  │   Activity graph        │   │ ├────────────────────────────────────────────┤ │  │
│  │ ⋮ [▣] Sensors     [━○] │   │ │ Graph window       [1 minute            ▾] │ │  │
│  │   Needs a sensor source │   │ ├────────────────────────────────────────────┤ │  │
│  │ ⋮ [▣] Weather     [●━] │   │ │ Graph width        ──────●─────  40 px     │ │  │
│  │   Icon and temperature  │   │ └────────────────────────────────────────────┘ │  │
│  │ ⋮ [▣] Time        [━○] │   │                                                │  │
│  │   Off                   │   │ Sampling                                       │  │
│  │ ⋮ [▣] Combined    [━○] │   │ ┌────────────────────────────────────────────┐ │  │
│  │   Off                   │   │ │ Interval           ──●────────  2 s        │ │  │
│  │                         │   │ │ Shorter intervals use more CPU.            │ │  │
│  │ [+ Add ▾]               │   │ └────────────────────────────────────────────┘ │  │
│  └─────────────────────────┘   │ Colors                    Using the Ocean palette │  │
│                                └────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────┘
```

### 2.1 Header

- Title "Strip" (Subtitle) at y 24. Caption at y 56: `"{n} items · {size} pt text · {scale} % graphics"`,
  `tnum`, updates as toggles change. This is the macOS sizing summary, made permanent instead of appearing
  in an apply bar. When the user's Text size ceiling is what is limiting the size, the caption says
  `"7 items · 11 pt text (your limit) · 90 % graphics"`.

### 2.2 Preview strip (y 80, 656 × 48, radius 8)

- Ground = the app's current taskbar estimate (`#202020`, `#EEEEEE`, or the accent), so the preview shows
  what the taskbar shows, including the backplate when it would be on. A 32 × 32 icon button at the right
  (sun / moon glyph, `text.secondary` on the ground's ink) flips the preview to the *other* theme so a
  color choice can be checked against both without changing Windows. The flip is preview-only and is
  forgotten when the window closes.
- The preview renders through the same code path as the taskbar strip, at the same size, with the same
  slack distribution for the current reservation. It is the real thing on a different canvas.
- Caption "PREVIEW" (Caption, 82 % ink, letter-spaced 0.8) at x 12; content centered in the remaining width;
  wider-than-available content scrolls horizontally inside the strip (no scrollbar; drag or wheel), never
  widens the window.

### 2.3 Item list (x 0–256, y 180 to pane bottom − 32; scrolls)

Rows 52 tall, one per strip item, in strip order.

```
Row, 256 × 52:
x=0   ▌ accent pill (3 × 16) when selected
x=8   ⋮ grip glyph E784, 16, text.tertiary; hover text.secondary
x=32  [▣] module tile 20 × 20 (12 glyph), module color; gray (control.disabled) when the row is off
x=60  Name       Body, text.primary  (Caption below: readout style, or the reason it cannot show)
x=204 [●━] toggle 40 × 20, vertically centered; no On/Off word here (the caption carries state)
```

Caption rules (Caption, `text.secondary`; `status.neutral` dot before a reason):

| Situation | Caption |
| --- | --- |
| On, has data | the readout style ("Label over value") |
| Off | "Off" |
| On, cannot show | "Needs a sensor source" / "Needs a location" / "No battery on this PC" |
| On, stale | "Last updated 47 min ago" (Weather) |

Selection and the inspector: clicking a row (not its toggle) selects it and the inspector shows it. Toggle
clicks do not change selection. Arrow keys move selection; Space toggles the selected row; Enter moves
focus to the inspector.

Reordering:

- **Mouse**: press anywhere on the row except the toggle, move 4 DIP → drag. The row lifts (`surface.card`
  fill, 1 DIP `stroke.strong`, drawn on top), follows the pointer vertically, and the other rows shift to
  open the gap (120 ms slide, or instantly under reduced motion). Release drops; Esc cancels. The strip
  reorders on drop.
- **Keyboard**: Alt+Up / Alt+Down move the selected row; the strip reorders immediately. Announced through
  UIA as "CPU, position 2 of 9".
- There is no separate "move up/down" button pair. The keyboard path is documented in the row's UIA help
  text and in a Caption under the list that appears only while the list has keyboard focus:
  "Alt+↑ / Alt+↓ to reorder".

Which rows exist:

- Always: CPU, GPU, Memory, Disks, Network, Sensors, Weather, Time, Combined (nine), in the default order
  CPU, Memory, Network, GPU, Disks, Sensors, Weather, Time, Combined. Defaults on: CPU, Memory, Network.
- Battery: present only when a battery has been detected on this PC at any time; absent otherwise (not
  grayed). A desktop user never sees a row they cannot use; a laptop docked to a desktop keeps its row.
- Additional instances: "Weather 2", "Sensors 2", further Combined groups, added through **Add**.

**Add** (standard button with `E710`, 32 tall, at the bottom of the list, 8 above the list's bottom
padding): a dropdown menu listing "Combined group", "Another weather location", "Another sensors widget".
New rows appear at the end, selected, with the inspector focused on the field that needs input (location
search; readings). Added instances have "Remove this item" (hyperlink button, `status.critical` text) at the
bottom of their inspector; the nine base rows do not.

Empty state (every row off): the list is unchanged; the preview strip shows Caption text "Nothing in the
strip. Turn on an item to start." in 82 % ink, centered; the header caption reads "0 items". The taskbar
reservation is released.

### 2.4 Inspector shell (x 272–656, y 180 to pane bottom; scrolls independently)

```
┌──────────────────────────────────────────────┐
│ [tile 32]  CPU                 Body Strong    │ y=0   header, 48 tall
│            Utilization, per-core load,        │       subtitle: Caption, text.secondary, 2 lines max
│            and top processes.                 │
│                                              │ 16
│ In the strip                  section header │
│ ┌──────────────────────────────────────────┐ │
│ │ Show in the strip                 On [●━]│ │ 48   the same switch as the list row; both move together
│ │ Readout                [dropdown 160  ▾] │ │ 48
│ │ …module-specific rows…                   │ │
│ └──────────────────────────────────────────┘ │
│ (module sections)                            │
│ Sampling                                     │
│ ┌──────────────────────────────────────────┐ │
│ │ Interval        ───●────────   2 s       │ │ 64   slider + value label; caption "Shorter intervals use more CPU."
│ └──────────────────────────────────────────┘ │
│ Colors                       Using Ocean ›   │ section header + trailing hyperlink to Appearance
│ ┌──────────────────────────────────────────┐ │ present only when Appearance › "Use one palette for every module" is off
│ │ Text        [■ light] [■ dark]           │ │ 48   swatch buttons 36 × 24, radius 4, 1 stroke; click opens the swatch menu
│ │ Graph line  [■] [■]      Fill  [■] [■]   │ │ 48
│ │ Warning     [■] [■]  Critical  [■] [■]   │ │ 48
│ │ 4.6:1 on a light taskbar · 7.7:1 on dark │ │ 32   Caption, live, for the Text role
│ └──────────────────────────────────────────┘ │
│ Remove this item                  (instances)│
└──────────────────────────────────────────────┘
```

Inspector width 384; rows are 48 unless they carry a description (64). Dropdowns are 160 wide, right-
aligned; sliders take the width between the label column (120) and the value label (48).

Swatch menu (popup, 5.9 style): "Theme color" (the current theme's value for this role), "Windows accent",
eight fixed swatches, "Custom…" (opens the Win32 `ChooseColor` dialog: native, ugly, and the only color
picker that needs no code). Each entry shows its ratio against the current taskbar estimate in Caption.

## 3. Inspectors, module by module

Only the rows that differ from the shell are listed. "Readout" options are the module's menu-bar modes from
the macOS app, renamed where Windows differs. Default in bold.

### 3.1 CPU

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | Percentage · **Label over value** · History graph · Per-core bars · Icon and value |
| Graph window | dropdown (graph modes only) | 30 s · **1 min** · 2 min · 5 min · 10 min |
| Graph width | slider 24–96, step 4 | **40** |
| Interval | slider 1–10 s | **2 s** |

Flyout: history graph (1 min–24 h), per-core bars with P/E labels when the OS reports hybrid cores, load
(1/5/15 min is Unix-only: show "Processes · Threads · Handles · Uptime" instead), top 5 processes with
icons and an end-task glyph on hover.

### 3.2 GPU

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Percentage** · History graph · With CPU (draws inside the CPU item as a second row/column; the GPU item itself disappears) |
| Adapter | dropdown | **Automatic (busiest)** · each adapter by name |
| Measure | dropdown | **Busiest engine** · 3D engine |
| Interval | slider 1–10 s | **2 s** |

"Adapter" is hidden when there is one adapter. If no adapter exposes engine counters (very old drivers), the
list row caption reads "No GPU counters available" and the item is not drawn.

### 3.3 Memory

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | Used percentage · **Label over value** · History graph · Usage bar |
| Measure | dropdown | **In use** · Committed |
| Interval | slider 1–10 s | **2 s** |

Flyout: breakdown bar in Task Manager terms (In use, Standby, Modified, Free), committed/limit, paged and
non-paged pool, top 5 processes by working set.

### 3.4 Disks

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Activity graph** · Free space (percent) · Free space (bytes) · Rates with arrows |
| Volume | dropdown | **C: (Windows)** · every mounted volume with a letter, "Label (Letter) · size" |
| Units | dropdown | **Decimal (GB)** · Binary (GiB) |
| Interval | slider 1–10 s | **2 s** |

Removable volumes appear in the dropdown while mounted; if the chosen volume goes away, the item shows the
system volume and the row caption says "D: was removed; showing C:" until the user picks again. Flyout:
every volume with a capacity bar and an eject glyph for removable ones, per-physical-disk read/write rates.

### 3.5 Network

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Download over upload** · Rate arrows on one line · NET label over total · Activity graph |
| Interface | dropdown | **Automatic (Ethernet)** · each interface by friendly name |
| Unit | dropdown | **Bytes per second** · Bits per second |
| Decimal places | dropdown | 0 · **1** · 2 |
| Order | dropdown | **Download first** · Upload first |
| Graph scale | dropdown + field | **Automatic** · Fixed → a 32-tall field "10 MB/s" appears beside it |
| Show public address in the panel | toggle | **Off** (fetches ipify only when on) |
| Interval | slider 1–10 s | **2 s** |

Flyout: throughput graph, interface, local addresses with copy glyphs, gateway and DNS, Wi-Fi SSID, signal,
band and rate (via the WLAN API), VPN interfaces flagged. On Windows 11 24H2 and later the WLAN queries
need the Location permission; when it is off, the Wi-Fi card shows the same calm info strip as Weather's
"Use current location" row ("Wi-Fi details need Location, which is off for desktop apps." + "Open Privacy
settings") and the rest of the panel is unaffected.

### 3.6 Sensors (the calm empty state)

```
┌──────────────────────────────────────────────┐
│ [tile]  Sensors                              │
│         Temperatures, fans and voltages from │
│         a monitoring app.                    │
│                                              │
│ Source                                       │
│ ┌──────────────────────────────────────────┐ │
│ │ ○ No sensor source found                 │ │  status.neutral dot; Body, text.primary
│ │                                          │ │
│ │ Windows doesn't provide temperatures,    │ │  Body, text.secondary, 3 lines, 16 padding
│ │ fans or voltages to apps, so Barometer   │ │
│ │ reads them from a monitoring app when    │ │
│ │ one is running. Install one, turn on its │ │
│ │ shared-memory option, and readings show  │ │
│ │ up here on their own.                    │ │
│ │                                          │ │
│ │ [Get HWiNFO ↗]  [Get LibreHardwareMonitor ↗] │  standard buttons (not accent), E8A7 suffix
│ │ Free, and Barometer only reads from them.│ │  Caption
│ └──────────────────────────────────────────┘ │
│                                              │
│ Readings in the strip                        │
│ ┌──────────────────────────────────────────┐ │
│ │ Available once a source is running.      │ │  Caption, text.secondary
│ │ [+ Add reading ▾]  (disabled)            │ │
│ └──────────────────────────────────────────┘ │
│ …Readout / Formatting / Sampling as below…   │
└──────────────────────────────────────────────┘
```

No warning glyph, no yellow, no "!" — the dot is gray because nothing is wrong. The paragraph explains the
mechanism once, in the second person, without apologising. Both buttons are standard, side by side, equal
weight; the app does not have an opinion about which monitor to install. The caption under them answers
the two questions people actually have (does it cost money; what does it do to my PC).

Source card states:

| State | Dot | Line 1 | Line 2 (Caption) | Action |
| --- | --- | --- | --- | --- |
| Providing | success | "HWiNFO 8.12 is providing 23 readings" | "Shared memory · updated 2 s ago" | none |
| Installed, not running | neutral | "HWiNFO is installed but not running" | "Readings appear when it starts. Its Shared Memory Support setting must be on." | "Start HWiNFO" (standard) |
| Running, sharing off | neutral | "HWiNFO is running but not sharing readings" | "Turn on Settings › Shared Memory Support in HWiNFO." | none |
| Sharing timed out (HWiNFO free edition, 12 h) | neutral | "HWiNFO stopped sharing readings" | "Its free edition shares for 12 hours at a time. Turn Shared Memory Support on again in HWiNFO." | none |
| Two sources running | success | "Using HWiNFO (LibreHardwareMonitor is also running)" | — | Source dropdown appears: **Automatic** · HWiNFO · LibreHardwareMonitor |
| None | neutral | as drawn above | | two Get buttons |

Detection runs every 10 s while this inspector is visible, every 60 s otherwise; the card updates in place.
The Sensors toggle in the list stays enabled in every state: turning it on records intent; the strip
simply does not draw the item until readings exist (design §9.7), and the list caption says why.

Remaining rows:

| Row | Control | Options |
| --- | --- | --- |
| Readings in the strip | chip list + "Add reading" dropdown | Detected readings grouped: Temperatures · Fans · Voltages · Power; each chip "CPU 51°" shows the live value; drag or Alt+←/→ to reorder; × removes |
| Readout | dropdown | **Compact two-row stack** · Labels and values · History graph · Fan RPM |
| Decimal places | dropdown | **0** · 1 |
| Hide equivalent readings | toggle | **On** ("Combines sensors that report the same value under different names.") |
| Show advanced sensors | toggle | **Off** ("Includes undocumented identifiers meant for diagnostics.") |
| Interval | slider 2–30 s | **5 s** |

Temperature unit lives in Appearance › Measurement units; the header of this inspector links to it
("Shown in °C · Change") so the user is never hunting.

Defaults on first source detection: the chip list is pre-filled with the hottest CPU die sensor and the
GPU temperature, which is what other monitors report and what the macOS app does.

### 3.7 Battery (laptops only)

| Row | Control | Options |
| --- | --- | --- |
| Readout | dropdown | **Glyph with percentage** · BAT label over percentage |
| Interval | slider 5–60 s | **10 s** |

Flyout: charge ring, state, health (full-charge / design capacity from the battery report), cycle count
where the firmware reports it, wattage in/out, adapter presence, charge history. No time estimates,
matching the macOS decision.

### 3.8 Weather

```
┌──────────────────────────────────────────────┐
│ [tile]  Weather                              │
│         Conditions and forecasts from        │
│         Open-Meteo.                          │
│                                              │
│ Locations                                    │
│ ┌──────────────────────────────────────────┐ │
│ │ ○ Austin                    Primary   ✕  │ │ 48  row: name Body; region Caption; "Primary" Caption tag; remove glyph
│ │   Texas, United States                   │ │
│ ├──────────────────────────────────────────┤ │
│ │ ○ Austin                    Set primary ✕│ │ 48  hyperlink "Set primary" on non-primary rows
│ │   Minnesota, United States               │ │
│ ├──────────────────────────────────────────┤ │
│ │ Use current location              Off[━○]│ │ 48  see states below
│ └──────────────────────────────────────────┘ │
│                                              │
│ Add a location                               │
│ ┌──────────────────────────────────────────┐ │
│ │ [🔍 Search cities                      ] │ │ 32-tall search field, full card width minus 32
│ └──────────────────────────────────────────┘ │
│   ┌────────────────────────────────────────┐ │ results popup, 8 × 48 max, opens under the field
│   │ Austin                          2.0 M  │ │ primary Body; secondary Caption; population right, text.tertiary
│   │ Texas, United States                   │ │
│   │ Austin                          38 K   │ │
│   │ Minnesota, United States               │ │
│   │ Austin                          5.6 K  │ │
│   │ Arkansas, United States                │ │
│   │ Austin                          2.1 K  │ │
│   │ Manitoba, Canada                       │ │
│   └────────────────────────────────────────┘ │
│                                              │
│ Units                                        │
│ ┌──────────────────────────────────────────┐ │
│ │ Temperature        [Fahrenheit (°F)   ▾] │ │
│ │ Wind               [Miles per hour    ▾] │ │  mph · km/h · m/s · knots
│ │ Pressure           [Inches of mercury ▾] │ │  inHg · hPa · mmHg
│ │ Precipitation      [Inches            ▾] │ │  in · mm
│ └──────────────────────────────────────────┘ │
│                                              │
│ Refresh                                      │
│ ┌──────────────────────────────────────────┐ │
│ │ Every            ───────●──── 15 min     │ │  slider 5–60, step 5
│ │ Updated 4 min ago · Refresh now          │ │  Caption + hyperlink button
│ └──────────────────────────────────────────┘ │
│                                              │
│ In the strip                                 │
│ ┌──────────────────────────────────────────┐ │
│ │ Show in the strip                 On[●━] │ │
│ │ Readout            [Icon and temperature▾]│ │
│ │ Color weather icons               Off[━○]│ │
│ │ In color the sun is amber, night is      │ │  Caption
│ │ lavender and rain is blue.               │ │
│ └──────────────────────────────────────────┘ │
│ Panel details                                │
│ ┌──────────────────────────────────────────┐ │
│ │ Show                [All details      ▾] │ │  All · Custom → checklist of sections appears
│ └──────────────────────────────────────────┘ │
│ Weather data by Open-Meteo.com               │  Caption, text.secondary, hyperlink
└──────────────────────────────────────────────┘
```

Search behavior

- Typing ≥ 2 characters starts a 300 ms debounce, then one geocoding request (`count=10`, language from the
  UI). While waiting, the field's right end shows the Caption "Searching…" in `text.secondary`; no spinner.
- Results popup: up to 8 rows; region = `admin1, country`; population right-aligned, abbreviated, in
  `text.tertiary`. Results are shown in the API's order (population-weighted), never re-sorted, so the big
  Austin is first.
- Down arrow moves from the field into the list; Enter adds the highlighted result; Esc closes the popup
  and keeps the text. Click adds. Adding clears the field, closes the popup, appends the row to Locations,
  and makes it primary if it is the first.
- States: no results → one non-interactive row "No matching places" (Body, `text.secondary`). Network
  failure → one row "Couldn't reach Open-Meteo. Check your connection and try again." Same text in the
  Refresh card's caption when a forecast fetch fails: "Couldn't update · showing the forecast from 09:12".
- Duplicate: adding a place already in the list highlights the existing row (`subtle.hover` for 1 s under
  motion, or a Caption "Already added" beside it) instead of adding twice.

Use current location

| State | Toggle | Caption |
| --- | --- | --- |
| Off | Off | "Uses Windows location to follow you." |
| On, allowed | On | "Near Austin, Texas" once resolved |
| On, location off for desktop apps | Off, disabled | Info strip under the row: "Location is off for desktop apps in Windows." + hyperlink "Open Privacy settings" (`ms-settings:privacy-location`) |
| On, no fix yet | On | "Finding your location…" (Caption), then the place |

Empty state (no locations, current location off): the Locations card holds one row "Add a city below to
start Weather." The list row caption says "Needs a location". The strip does not draw the item.

Readout options: Condition mark over temperature · **Icon and temperature** · Temperature only · Icon,
temperature and conditions · High and low · Rain chance · Custom template ("`{temp}` `{cond}` `{hi}` `{lo}`
`{pop}` `{wind}` `{aqi}`" listed in a Caption under a 32-tall field that appears when chosen).

Defaults from region: °F/mph/inHg/in when the Windows region is the United States, Liberia or Myanmar;
otherwise °C/km/h/hPa/mm. Refresh 15 min. Weather refreshes on resume from sleep and on network change as
on macOS. The last forecast is cached and shown with the stale rule (design §9.7).

### 3.9 Time

| Row | Control | Options |
| --- | --- | --- |
| Show in the strip | toggle | **Off** ("Windows shows its own clock. Time is for seconds, a second time zone or a different format.") |
| Format | text field, 200 | **`h:mm`**; Caption lists tokens: `h H mm ss a ddd d MMM yyyy z`; live sample to the right "9:41" |
| Show seconds | toggle | **Off** (interval becomes 1 s) |
| World clocks | chip list + "Add clock" dropdown (time zones grouped by region, searchable) | none |
| Week starts on | dropdown | **System default** · Sunday · Monday |
| Text size | dropdown | **Same as the strip** · 9 · 10 · 11 · 12 |

Flyout: month calendar (today highlighted with `accent.fill`), world clocks with offsets and day/night
marks, sunrise/sunset from the primary weather location. No notifications list and no calendar events:
Windows does not expose either to a desktop app, and the inspector says nothing about it because absent
features do not need apologies.

### 3.10 Combined

| Row | Control | Options |
| --- | --- | --- |
| Name | text field | **"Combined"** (shown in the list and flyout title; rename freely) |
| Members | checklist with grips, in order | CPU · GPU · Memory · Disks · Network · Sensors · Battery · Weather · Time; **CPU, Memory** on by default |
| Separators | toggle | **On** (1 DIP line between members) |
| Hide members' own items | toggle | **On** ("The items above stay off while they are in this group.") |

Members draw with their own readout style and colors. The group counts as *one* item for automatic sizing,
which is the reason to use it on Windows: four readings in one Combined item keep the strip at 12 pt where
four separate items would still, but seven would not. Flyout: tabs across the top (36 tall, Body,
selected tab underlined 2 DIP `accent.fill`), one member per tab, each tab the member's own panel.

## 4. Appearance pane

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│  Appearance                                                                        │ y=24
│  Colors, type and spacing for the strip and its panels.                           │ y=56
│                                                                                    │
│  ┌──────────────────────────────────────────────────────────────────────┐          │ y=80  preview strip, as in §2.2
│  └──────────────────────────────────────────────────────────────────────┘          │ y=128
│                                                                                    │
│  Theme                                                                             │ y=152
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐          │ y=180  tiles 100 × 72, 8 gap (640 total)
│  │▌CPU 24% │ │ CPU 24% │ │ CPU 24% │ │ CPU 24% │ │ CPU 24% │ │ CPU 24% │          │        each tile: mini strip sample 100 × 40 on brand.ground,
│  │ System  │ │ Ocean   │ │ Sunset  │ │ Forest  │ │ Neon    │ │ Custom  │          │        name Caption below; selected = accent pill + 2 DIP accent stroke
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘          │ y=252
│                                                                                    │
│  Text                                                                              │ y=276
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Weight                                          [Medium          ▾] │          │ 48  Regular · Medium · Semibold
│  │ Text size                              ─────────●──   12 pt          │          │ 64  slider 9–12; caption "The strip sizes itself by item count; this is the upper limit. Now: 7 items → 12 pt."
│  │ Graph opacity                          ────●────────   30 %          │          │ 48
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Strip                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Spacing between items                           [Normal          ▾] │          │ 48  Normal (12) · Snug (8) · Tight (4)
│  │ Dividers between items                                    Off [━○]  │          │ 48
│  │ Backplate                                       [Automatic       ▾] │          │ 64  Automatic · On · Off; caption: "A dark ground behind the strip. Automatic turns it on when the taskbar is light or colored."
│  │ Shrink items to fit the current reading                   Off [━○]  │          │ 64  caption: "Off keeps every item at a fixed width so nothing moves when a number changes."
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Colors                                                                           │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Use one palette for every module                           On [●━]  │          │ 64  caption: "Off lets each item set its own colors in the Strip pane."
│  │ Text          [■ light] [■ dark]     4.6:1 light · 7.7:1 dark        │          │ 48  swatches 36 × 24; live ratios Caption
│  │ Graph line    [■] [■]                                                │          │ 48
│  │ Graph fill    [■] [■]                                                │          │ 48
│  │ Warning       [■] [■]                                                │          │ 48
│  │ Critical      [■] [■]                                                │          │ 48
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Measurement units                                                                 │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Temperature                                     [Celsius (°C)    ▾] │          │ 64  caption: "Used by Sensors, GPU and Battery. Weather has its own units."
│  └──────────────────────────────────────────────────────────────────────┘          │
└────────────────────────────────────────────────────────────────────────────────────┘
```

Behavior

- Theme tiles are a radio group (one Tab stop, arrow keys move, Space selects). Choosing a preset writes all
  ten roles (the macOS `applyTheme`) and turns "Use one palette" on; System also sets monochrome. Editing
  any swatch afterwards switches the tile to Custom without asking.
- The Colors card is disabled with the caption "The System theme matches the taskbar's own text. Pick
  another theme to use colors." while System is selected. The swatch rows are hidden (not disabled) when
  "Use one palette" is off, replaced by a Caption "Each item sets its own colors in the Strip pane."
- Text size is a ceiling (design §9.4); its caption always shows what the strip is actually using.
- The preview strip reflects every change on this pane instantly, and so does the taskbar.

## 5. Taskbar pane

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│  Taskbar                                                                           │ y=24
│  How Barometer fits into the taskbar.                                              │ y=56
│                                                                                    │
│  Space in the notification area                                                    │ y=80
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │  Off                                                                 │          │ 20  Caption labels above each mini taskbar
│  │  ┌───────────────────────────────────────────────────────────────┐   │          │
│  │  │ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢▢▢ CPU 24% ⌃ ♪ ⚙ 9:41 │   │          │ 28  mini taskbar 600 × 28: buttons as 20 × 16 rounded rects,
│  │  └───────────────────────────────────────────────────────────────┘   │          │     the strip in brand.cyan, the tray as glyphs; overlap drawn where it happens
│  │  On                                                                  │          │
│  │  ┌───────────────────────────────────────────────────────────────┐   │          │
│  │  │ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢ »    CPU 24%  ⌃ ♪ ⚙ 9:41 │   │          │
│  │  └───────────────────────────────────────────────────────────────┘   │          │
│  ├──────────────────────────────────────────────────────────────────────┤          │
│  │ Reserve space for the strip                                On [●━]   │          │ 64
│  │ Windows lays the taskbar out as if Barometer weren't there, so when │          │ Body, text.secondary, wraps; 16 padding
│  │ many windows are open the task buttons run underneath the strip.    │          │
│  │ Reserving space tells Windows the strip is there, and the buttons    │          │
│  │ stop short of it. Windows may move a few buttons into its » overflow│          │
│  │ to make room.                                                        │          │
│  ├──────────────────────────────────────────────────────────────────────┤          │
│  │ ● Reserving 5 slots (210 px) for a 202 px strip                      │          │ 40  status line, live; success dot
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Tip                                                                               │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Windows frees space one whole task button at a time, so a small gap  │          │ 64  Body text.secondary
│  │ can remain beside the strip. It is smallest when Taskbar settings ›  │          │
│  │ Combine taskbar buttons is set to Always.       [Open Taskbar settings ↗] │     │ standard button, ms-settings:taskbar
│  └──────────────────────────────────────────────────────────────────────┘          │
└────────────────────────────────────────────────────────────────────────────────────┘
```

The diagram is two rows of Direct2D rectangles and a handful of glyphs, drawn once. It carries the whole
explanation: in the "Off" row the last three buttons are drawn under the strip; in the "On" row they stop,
one has become the `»` overflow, and the strip has room. It is not animated.

Toggle states

| Condition | Toggle | Status line (Caption, `text.secondary`, neutral dot) |
| --- | --- | --- |
| Available, on | On | "Reserving 5 slots (210 px) for a 202 px strip" (`tnum`, live) |
| Available, off | Off | "The strip may be overlapped when the taskbar is full." |
| Vertical taskbar | disabled, shows Off | "Not available on a vertical taskbar." |
| Windows 10 taskbar | disabled | "Not available on the Windows 10 taskbar; the strip is placed beside the clock instead." |
| Another shell owns the taskbar (ExplorerPatcher, StartAllBack, etc.) | disabled | "Not available while another app is managing the taskbar." |
| Registration failing (Explorer restarting, shell not ready) | on, dot neutral | "Waiting for the taskbar…" then retries with the existing backoff |

Disabled reasons are stated as facts about the environment, never as errors. Nothing here uses caution or
critical color. There is no "position" row (the strip lives left of the notification area; that is the
only place Windows 11 offers) and no "show on all taskbars" row (secondary taskbars have no notification
area); if either becomes possible the row goes here.

## 6. General pane

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│  General                                                                           │
│  Startup, sampling and updates.                                                    │
│                                                                                    │
│  Startup                                                                           │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Start with Windows                                          On [●━] │          │ 48 / 64 with a reason
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Sampling                                                                          │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Reduce sampling on battery                                  On [●━] │          │ 64  laptops only; caption "Doubles every interval while unplugged."
│  │ Sampling pauses while the screen is off.                             │          │ 40  Caption only, no control
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Updates                                                                           │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Barometer 1.0.0                            [Check for updates]       │          │ 64  version Body; caption "Checked today at 09:12 · Up to date"
│  │ Check automatically                                         On [●━] │          │ 64  caption "Once a week. Nothing is installed without you."
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Settings file                                                                     │
│  ┌──────────────────────────────────────────────────────────────────────┐          │
│  │ Export and import your settings as a file.   [Export…]  [Import…]   │          │ 64  caption "Contains display and sampling preferences, no system data."
│  └──────────────────────────────────────────────────────────────────────┘          │
│                                                                                    │
│  Reset all settings…                                                               │  hyperlink button, status.critical text; confirmation dialog
└────────────────────────────────────────────────────────────────────────────────────┘
```

Start with Windows states

| State | Toggle | Caption |
| --- | --- | --- |
| Registered (`Run` key present) | On | — |
| Not registered | Off | — |
| Registered but disabled in Windows Settings › Apps › Startup | disabled, shows Off | "Turned off in Windows Settings › Apps › Startup." + hyperlink "Open Startup settings" (`ms-settings:startupapps`) |
| Blocked by policy | disabled | "Managed by your organization." |

Updates states (the version row's caption): "Checking…" → "Up to date · checked just now" / "1.1.0 is
available" with the button becoming an accent **Download 1.1.0** (opens the release page; nothing is
installed silently) / "Couldn't check · try again later". Per-module intervals are in each inspector; there
is no global interval slider here because the macOS "global sampling rate" override confused the per-module
ones and the inspector already shows each.

Reset: a small centered dialog (320 wide, Body Strong title "Reset all settings?", Body "The strip goes back
to CPU, Memory and Network, and the System theme.", buttons [Reset] (accent) [Cancel]). Esc cancels.

## 7. About pane

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│  About                                                                             │
│                                                                                    │
│  ┌────┐  Barometer                                    Subtitle                     │  icon 64 (the dark square with the cyan arc)
│  │ ◠~ │  1.0.0 (2026-09-07)                           Caption, tnum                │
│  └────┘  A system monitor for the Windows taskbar.    Body                         │
│                                                                                    │
│  Source code ↗ · Report a problem ↗ · Weather data by Open-Meteo ↗ · Licenses       │  hyperlink buttons, · separators
│                                                                                    │
│  Free software under the GNU GPL 3.0. The macOS version lives in the same          │  Caption, text.secondary
│  repository. Temperatures come from HWiNFO or LibreHardwareMonitor when installed. │
└────────────────────────────────────────────────────────────────────────────────────┘
```

"Licenses" opens a plain scrolling text card in the same pane (GPL text, Open-Meteo CC BY 4.0, Segoe
fonts are the OS's). No Credits card beyond that: restraint.

## 8. The strip itself

### 8.1 Anatomy at 100 %

```
Taskbar, 48 tall. The strip is the reserved region; here 5 slots = 210 px for 202 px of content, slack 8.
x=0                                                                                   x=210
┌──┬──────┬────┬──────┬────┬────────────┬────┬───────────┬────┬───────┬──┐
│4 │ CPU  │ 12 │ MEM  │ 12 │ ↓ 12.3 KB/s│ 12 │ ▂▃▅▇▅▃▂▄  │ 12 │ ☀ 72° │4 │    ← gaps get +1 each from slack; 2 to each edge
│  │ 24%  │    │ 61%  │    │ ↑  1.2 KB/s│    │           │    │       │  │
└──┴──────┴────┴──────┴────┴────────────┴────┴───────────┴────┴───────┴──┘
     30          30           72             40 (graph)      50
```

Vertical placement (design §9.3): labels on row 1 (baseline y 20), values on row 2 (baseline y 36); single-
row items centered (baseline y 28.5); the graph spans y 8–40.

### 8.2 The readout styles, drawn

```
Percentage          Label over value      History graph        Per-core bars (8)      Icon and value
┌──────┐            ┌──────┐              ┌──────────┐         ┌───────────────┐      ┌──────────┐
│      │            │ CPU  │ 82 % ink     │ ▁▂▃▅▇▅▃▂ │ line 1  │ ▃▅▂▇▁▄▆▃      │ 3 w  │ [▣] 24%  │ mark 16 + 4 + value
│ 24%  │ centered    │ 24%  │ 100 % ink    │ ▁▂▃▅▇▅▃▂ │ fill    │               │ 1 gap│          │
└──────┘            └──────┘              └──────────┘         └───────────────┘      └──────────┘

Usage bar (Memory)  Download over upload  Rate arrows, one line   Condition over temperature   Battery glyph
┌──────┐            ┌────────────┐        ┌─────────────────┐     ┌──────┐                    ┌──────┐
│ MEM  │            │ ↓ 12.3 KB/s│        │ ↓12.3K  ↑1.2K   │     │  ☀   │ mark 16            │[▮▮▮ ]│ 26 × 12, percentage inside
│ ▇▇▇▃ │ 6 tall     │ ↑  1.2 KB/s│        │                 │     │ 72°  │                    │  84  │ in 9 pt when ≥ 3 digits fit
└──────┘            └────────────┘        └─────────────────┘     └──────┘                    └──────┘

Combined (CPU · MEM · NET), separators on:
┌──────┬─┬──────┬─┬────────────┐
│ CPU  │ │ MEM  │ │ ↓ 12.3 KB/s│   6 gap, 1 DIP separator at strip.divider, 20 tall
│ 24%  │ │ 61%  │ │ ↑  1.2 KB/s│
└──────┴─┴──────┴─┴────────────┘
```

Arrows are the U+2193/U+2191 glyphs of the text font at the item's size, not icons, so they sit on the
baseline and take the ink. Units are abbreviated at 9–10 pt ("K", "M") and full at 11–12 pt ("KB/s").

### 8.3 Widths and slots at four scales

The example strip above (CPU, MEM, NET, CPU graph, Weather) with Normal spacing:

| Scale | Content | Slot pitch | Slots | Reserved | Slack | Text px |
| --- | --- | --- | --- | --- | --- | --- |
| 100 % | 202 px | 42 | 5 | 210 | 8 | 12 |
| 125 % | 253 px | 52 | 5 | 260 | 7 | 15 |
| 150 % | 303 px | 63 | 5 | 315 | 12 | 18 |
| 200 % | 404 px | 84 | 5 | 420 | 16 | 24 |

Slot pitch is measured, not assumed; the table shows typical values. Slack never exceeds one slot and is
always absorbed inside the strip (design §9.2).

### 8.4 States

```
Rest                 Hover (pointer on MEM)           Open (MEM flyout showing)        Stale weather
CPU   MEM   ↓…       CPU  ╭────╮  ↓…                  CPU  ╭────╮  ↓…                  ☀ 72°   (value at 70 %)
24%   61%   ↑…       24%  │MEM │  ↑…                  24%  │MEM │  ↑…
                          │61% │                           │61% │
                          ╰────╯ strip.hover, r4           ╰────╯ strip.open
```

Sensors without a source, Weather without a location, Battery on a desktop: not drawn, no placeholder; the
reservation shrinks to what remains.

### 8.5 Legibility summary

| Taskbar | System theme | Color theme, Backplate Automatic | Color theme, Backplate Off |
| --- | --- | --- | --- |
| Dark | white ink, 16:1 | dark role values on the bare taskbar, 6.5–12:1 | same |
| Light | `#1B1B1B` ink, 15:1 | plate on; dark role values on `#2D2F32`, ≥ 4.85:1 | darkened light variants, ≥ 4.5:1 text |
| Accent-tinted | white or black, whichever is higher (5.7:1 on the default blue) | plate on; ≥ 6.9:1 | dark role values; the swatch caption warns when a role falls under 4.5:1 |
| High contrast | system window text on system window color, opaque | themes ignored | themes ignored |

## 9. Flyout panels

### 9.1 Anatomy (CPU shown), 320 wide

```
┌──────────────────────────────────────────┐  8 radius, acrylic, 1 stroke; 12 panel padding
│ ┌──────────────────────────────────────┐ │
│ │ [▣] CPU                        24 %  │ │  header card, tinted CPU color @ 12 %; Title 28 tnum right
│ │     8 cores · 2.4 GHz · up 3 d 4 h   │ │  Caption
│ └──────────────────────────────────────┘ │  10 gap
│ ┌──────────────────────────────────────┐ │
│ │ History                 [1 min   ▾]  │ │  card; dropdown 120 in the corner
│ │ ╭────────────────────────────────╮   │ │
│ │ │        ▁▂▃▅▇▆▄▃▂▁▂▃▅▇▅▃        │   │ │  graph 296 × 72; line 1 DIP, fill 30 %; hairline axis
│ │ ╰────────────────────────────────╯   │ │
│ │ 0 %                            100 % │ │  Caption ticks
│ └──────────────────────────────────────┘ │
│ ┌──────────────────────────────────────┐ │
│ │ Cores                                │ │
│ │ ▃▅▂▇▁▄▆▃  P0 P1 P2 P3  E0 E1 E2 E3   │ │  bars 8 × 24, labels Caption
│ └──────────────────────────────────────┘ │
│ ┌──────────────────────────────────────┐ │
│ │ Top processes                        │ │
│ │ [ic] Firefox                  18 %  ✕│ │  rows 36; end-task glyph on hover; value tnum
│ │ [ic] Code                      6 %   │ │
│ │ [ic] Explorer                  2 %   │ │
│ └──────────────────────────────────────┘ │
├──────────────────────────────────────────┤  hairline
│ [⚙]                              (empty)│  footer 40: gear at left; module actions at right
└──────────────────────────────────────────┘
        ▲ centered on the CPU item; bottom edge 8 above the taskbar
```

Panel height = content; scrolls past min(720, work area − 16). Weather's panel (conditions header, 48-hour
strip with the temperature curve and rain bars, 10-day rows, sun and moon, air quality, details grid,
location switcher, Refresh in the footer, attribution Caption) is the tallest and scrolls. Combined shows
a tab row (36) under the header.

### 9.2 Placement

```
Bottom taskbar (default)                  Item near the right edge                   Top taskbar
                                                                                     ┌──────── taskbar ────────┐
        ┌──────────┐                                        ┌──────────┐             │        CPU  MEM         │
        │  panel   │                                        │  panel   │             └──────────┬──────────────┘
        │          │                                        │          │  ← clamped:           │ 8
        └────┬─────┘                                        └──────┬───┘   right edge      ┌────┴─────┐
             │ 8                                                   │ 8     = work area     │  panel   │
┌────────────┴────────────┐                    ┌───────────────────┴───┐   − 8             └──────────┘
│        CPU  MEM   ⌃ 9:41│                    │            CPU  MEM ⌃ │
└─────────────────────────┘                    └───────────────────────┘
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
| | F6 | Cycle nav → list → inspector (Strip pane) or nav → content |
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
| Flyout | Tab, arrows, Esc | §9.4 |
