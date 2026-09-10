# Barometer for Windows: UI design language

Status: v1 design, 2026-09-07. Companion: `ui-layouts.md` (every pane and the strip, drawn with dimensions).
Sibling: the macOS app's `docs/DESIGN.md` and `Sources/MenuBarStatsUI/Design/BarometerDesign.swift` in
`mackid1993/Barometer`. This document is the Windows counterpart. Where it departs from the macOS app, the
reason is in section 13; where it departs from the original brief, section 14.

**This was written before the app existed.** Where the shipped app went another way, the section says so
in a line rather than being deleted - the reasoning is worth keeping even where the outcome changed. Two
divergences run through the whole document and are not repeated at every mention: everything is drawn with
**GDI, with GDI+ where a two or three pixel feature has to be antialiased**, not Direct2D/DirectWrite; and
the settings window's navigation is the list in section 13, a page per module, not the five panes drawn
here. The tokens, metrics, states and rules below are otherwise what the code implements.

Every dimension in this document is in DIPs (device-independent pixels, 96 per inch). Multiply by the
monitor's scale factor (`GetDpiForWindow / 96`) and round to whole physical pixels at draw time. Nothing is
specified in physical pixels except where the taskbar forces it (section 9.2).

Contrast ratios quoted here were computed against the resolved hex values below (WCAG 2.x relative
luminance). Where a WinUI token is defined as an alpha over a surface, the composited value is given so a
Direct2D implementation can create one opaque `SolidColorBrush` per token at theme-change time and never
blend per frame.

## 0. The decisions, in one screen

1. **One strip, one composer.** Windows gives an app one place in the taskbar, not N movable items, so the
   settings window's center of gravity is a *strip composer*: a reorderable list of enabled modules with an
   inspector for the selected one. Every module's options live where the module is.
2. **The settings window is a Windows citizen; the strip and its flyouts are Barometer.** Chrome, controls,
   type and the accent color in Settings are Windows 11's. Barometer's own visual identity (dark ground,
   module colors, the themes) lives in the strip, the flyout panels and the preview strip, where it belongs.
3. **Monochrome by default, color on a dark ground.** The default theme matches the taskbar's own text
   color and is indistinguishable in weight from the clock. Color themes are legible everywhere because,
   on a light or accent-tinted taskbar, they sit on Barometer's dark backplate rather than on the wallpaper.
4. **Nothing animates at rest.** The strip redraws when a sample changes. Settings has four short
   transitions, all one-shot, all zero under reduced motion. Live graphs exist only inside an open flyout.
5. **Apply on change.** No OK/Cancel/Apply. The macOS app's "Apply Changes" bar is a macOS 27 workaround for
   status-item geometry; the Windows strip is one window the app fully owns, so every change is live.
6. **Fixed widths, tabular digits, shared baselines.** The macOS readout typography rules carry over
   verbatim, because they are what makes a live readout restful.

## 1. Window chrome and surfaces

### 1.1 Settings window

| Property | Value | Why |
| --- | --- | --- |
| Default size | 880 × 640 | Composer needs three columns (nav, list, inspector). |
| Minimum size | 760 × 560 | Below this the inspector wraps controls. |
| Resizable | Yes; size and pane remembered | Windows text scaling up to 225 % must have somewhere to go. |
| Caption buttons | Minimize, Close | Maximize is pointless for a control panel; omitting it is a Win11 dialog convention. |
| Corner radius | 8, via `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND` | System-drawn, free. |
| Backdrop | Mica: `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW` | Cheapest Win11 material; DWM samples the wallpaper once, no per-frame cost. |
| Title bar | 32 tall, client-area extended (`DwmExtendFrameIntoClientArea` with `-1` margins); `DWMWA_CAPTION_COLOR = DWMWA_COLOR_NONE` so DWM draws its own caption buttons over the Mica | Keeps the caption buttons, their hover states, snap layouts and accessibility for free; only the title text is ours. |
| Title text | Caption 12/16, `text.primary`, 16 from the left edge, vertically centered; app icon 16 × 16 at 12, gap 8 | Matches Settings and Terminal. |
| Dark mode | `DWMWA_USE_IMMERSIVE_DARK_MODE = TRUE` when the app theme is dark | Caption buttons pick up the right color. |
| Fallback | If Mica is unavailable (Windows 10, transparency effects off, remote session): paint `surface.base` solid | Same layout, no material. |

The window is one top-level HWND. Everything inside is custom-drawn with Direct2D/DirectWrite into a
swap chain (or a `WM_PAINT` `ID2D1HwndRenderTarget`, which is enough here), with one child HWND per native
control we borrow: the text input caret and IME handling come from an `EDIT` control positioned under our
drawn field; everything else (buttons, toggles, sliders, dropdown popups) is drawn and hit-tested by us.
Reason: consistent Win11 appearance without WinUI 3's runtime, and per-monitor DPI handling in one place.

### 1.2 Surface stack

Three planes, from back to front. Each is a single filled rectangle.

```
 Mica (surface.base fallback)                 nav pane sits directly on this
 └─ surface.layer, 8 DIP top-left radius      content pane, starts at x = 176
    └─ surface.card, 4 radius, 1 stroke       group cards; rows inside separated by stroke.divider
```

There is no per-row card (the Windows Settings app's pattern). One card per section with hairline
dividers halves the vertical spend, draws one stroke instead of eight, and reads calmer. It is still
recognizably Win11: same radius, same stroke color, same 16 inner padding.

### 1.3 Popups (dropdown lists, search results, color swatch menus)

Separate `WS_POPUP` HWNDs (they must escape the window bounds), `DWMWCP_ROUNDSMALL` corners (4), 1 DIP
`stroke.control` inside the edge, fill `surface.card` opaque, and a DWM shadow (which Win11 supplies for
rounded top-level windows; if absent the stroke alone is acceptable). No acrylic on popups: they are
short-lived and the blur is not worth the composition.

### 1.4 Flyout panels (the module detail panels under the strip)

`WS_POPUP`, `DWMSBT_TRANSIENTWINDOW` (acrylic, the same backdrop Windows uses for its own tray flyouts),
`DWMWCP_ROUND` (8), 1 DIP `stroke.control` inside the edge. Fallback: `surface.flyout` solid. Detailed in
section 10.

## 2. Color

### 2.1 Neutral tokens (settings window, popups, flyouts)

Resolved values. Light column is against the light surface stack, dark against the dark one.

| Token | Light | Dark | Use | Contrast (light / dark, vs `surface.card`) |
| --- | --- | --- | --- | --- |
| `surface.base` | `#F3F3F3` | `#202020` | Mica fallback; window ground; nav pane | — |
| `surface.layer` | `#F9F9F9` | `#282828` | Content pane | — |
| `surface.card` | `#FDFDFD` | `#333333` | Group cards, popup fill, flyout cards (fallback) | — |
| `surface.flyout` | `#F2F2F2` | `#2C2C2C` | Acrylic fallback for flyouts | — |
| `control.rest` | `#FBFBFB` | `#3F3F3F` | Button, dropdown, text field | — |
| `control.hover` | `#F6F6F6` | `#444444` | | — |
| `control.pressed` | `#F1F1F1` | `#3A3A3A` | | — |
| `control.disabled` | `#F5F5F5` | `#3C3C3C` | | — |
| `subtle.hover` | `#EDEDED` | `#353535` | Nav items, list rows, icon buttons: no stroke | text.secondary on it 5.5 / 8.0 |
| `subtle.pressed` | `#F1F1F1` | `#313131` | Lighter than hover, as WinUI does | — |
| `stroke.control` | `#E5E5E5` | `#4C4C4C` | Control outline | decorative |
| `stroke.control.edge` | `#D4D4D4` | `#5A5A5A` | Bottom edge in light, top edge in dark (WinUI's 1 DIP "lift") | decorative |
| `stroke.strong` | `#8A8A8A` | `#9D9D9D` | Toggle-off ring, text field bottom edge, slider rail, checkbox box | 3.4 / 4.7 (≥ 3:1, WCAG 1.4.11) |
| `stroke.divider` | `#E9E9E9` | `#444444` | Row hairlines inside cards | decorative |
| `stroke.card` | `#E5E5E5` | `#3D3D3D` | Card outline | decorative |
| `text.primary` | `#1B1B1B` | `#FFFFFF` | Body, labels, values | 16.9 / 12.6 |
| `text.secondary` | `#5E5E5E` | `#D0D0D0` | Captions, descriptions, placeholders, On/Off words | 6.4 / 8.2 |
| `text.tertiary` | `#8E8E8E` | `#9D9D9D` | Glyphs only: chevrons, grips, clear buttons. **Never words.** | 3.2 / 4.7 |
| `text.disabled` | `#A0A0A0` | `#767676` | Disabled control text | 2.6 / 2.8 (exempt: disabled) |
| `text.on-accent` | `#FFFFFF` | `#000000` | Text on accent fills | see 2.2 |
| `focus.outer` | `#000000` | `#FFFFFF` | 2 DIP focus ring | 20 / 15 vs layer |
| `focus.inner` | `#FFFFFF` | `#000000` | 1 DIP inner ring | — |

`text.tertiary` fails 4.5:1 in light. That is deliberate and safe only because the rule above is absolute:
it colors symbols that are never the sole carrier of information. Placeholder text is `text.secondary`.

### 2.2 Accent (the user's Windows accent color)

Settings uses the user's accent, read from `UISettings.GetColorValue` (WinRT, works from Win32) or the
`AccentPalette` registry blob, and refreshed on `WM_SETTINGCHANGE`/`ColorValuesChanged`.

| Token | Light | Dark | Default value (Windows default blue) | Contrast |
| --- | --- | --- | --- | --- |
| `accent.fill` | Accent | AccentLight2 | `#0067C0` / `#4CC2FF` | white on it 5.7 / black on it 10.5 |
| `accent.hover` | fill at 90 % over card | fill at 90 % over card | `#1976C6` / `#4AB4EB` | 4.7 / 9.0 |
| `accent.pressed` | AccentDark1 | AccentLight1 | `#005FB8` / `#0091F8` | 6.3 / 6.4 |
| `accent.text` | AccentDark1 | AccentLight3 | `#005FB8` / `#99EBFF` | 6.2 / 9.4 |
| `accent.pill` | = `accent.fill` | | | 3 × 16 selection indicator |

Pressed goes one palette step *darker* in light (WinUI fades it to 80 %, where white text drops to 3.9:1);
this keeps white text above 4.5:1 in every state. Dark keeps black text, which never gets close to the line.

Derivation rule for arbitrary accents: for `accent.text`, start at AccentDark1 (light) / AccentLight3 (dark)
and step further away from the surface through the palette (Dark2, Dark3 / Light2, Light1) until the ratio
against `surface.card` is ≥ 4.5:1; if none passes, use `text.primary`. For `accent.fill`, choose
`text.on-accent` by testing white and black against the fill and taking the higher ratio. Both checks run
once per accent change.

### 2.3 Status

| Token | Light | Dark | Badge fill (light / dark) | Contrast on card | Contrast on badge |
| --- | --- | --- | --- | --- | --- |
| `status.success` | `#0F7B0F` | `#6CCB5F` | `#DFF6DD` / `#393D1B` | 5.4 / 6.2 | 4.8 / 5.6 |
| `status.caution` | `#9D5D00` | `#FCE100` | `#FFF4CE` / `#433519` | 5.2 / 9.6 | 4.8 / 9.0 |
| `status.critical` | `#C42B1C` | `#FF99A4` | `#FDE7E9` / `#442726` | 5.6 / 6.2 | 4.8 / 6.6 |
| `status.neutral` | `stroke.strong` | `stroke.strong` | none | 3.4 / 4.7 (dot only) | — |

Status is shown as an 8 DIP dot plus words; never as color alone. The Sensors "no source" state uses
`status.neutral`, not caution: it is the normal state of a fresh install (section 12 of `ui-layouts.md`).

### 2.4 Brand

| Token | Value | Where it appears |
| --- | --- | --- |
| `brand.ground` | `#1C1E22` | App icon ground; the strip backplate (2.7); preview strip in Settings; About mark |
| `brand.cyan` | `#06B6D4` | App icon arc; the gauge in About; the "Automatic" swatch in color pickers. **Never text on a light surface**: 2.4:1 on `surface.card` light. On `surface.layer` dark it is 6.1:1. |

The settings accent is the user's, not brand cyan. Forcing cyan would fail contrast in light mode and break
the "belongs on Windows 11" requirement; the brand is carried by the icon and the dark grounds instead.

### 2.5 Module signature colors

From `BarometerDesign.swift`, unchanged, so the two apps' flyouts match. Used for icon tiles, flyout header
tints and graph strokes when the theme is System (monochrome); color themes flow their own graph and fill
roles through instead.

| Module | Primary | Secondary | Icon-tile glyph ink | Why |
| --- | --- | --- | --- | --- |
| CPU | `#3B82F6` | `#22D3EE` | white (3.7:1) | |
| GPU | `#A855F7` | `#EC4899` | white (4.0) | |
| Memory | `#6366F1` | `#A78BFA` | white (4.5) | |
| Disks | `#14B8A6` | `#4ADE80` | `brand.ground` (6.7) | white would be 2.5 |
| Network | `#0EA5E9` | `#34D399` | `brand.ground` (6.0) | white 2.8 |
| Sensors | `#F97316` | `#EF4444` | `brand.ground` (6.0) | white 2.8 |
| ~~Battery~~ | `#22C55E` | `#A3E635` | `brand.ground` (7.3) | **dropped** - Windows shows its own battery |
| Weather | `#38BDF8` | `#FBBF24` | `brand.ground` (7.8) | white 2.1 |
| ~~Time~~ | `#8B5CF6` | `#38BDF8` | white (4.2) | **dropped** - Windows shows its own clock |
| Stacks (was Combined) | `#2F7CF6` | `#6BA4FF` | white (3.9) | |

Tiles are flat (primary color only, 4 radius). The macOS app uses a two-color gradient; the Windows brand
rule is "two colors on dark, no gradients", and a flat tile with a correctly chosen glyph ink is also the
only version that passes 3:1 for every module. None of these colors is used as text on a light surface
(they range 2.1–4.4:1 there).

### 2.6 Themes (the strip and flyout graphs)

**Not built.** The strip draws in the taskbar's own ink and the flyouts in the module signature colors of
2.5; there is no theme control in Appearance and no backplate. `AppearancePreset` in
`barometer-core/src/settings.rs` carries these five names and nothing reads it yet. The palettes below are
the specification for the day it does, and the contrast work under them is the part worth keeping.

The five presets and their ten roles are the macOS values, verbatim, so a user with both apps sees one
palette. Each role has a *light* value (for a light taskbar) and a *dark* value (for a dark taskbar or the
backplate).

| Theme | Text L / D | Graph L / D | Fill L / D | Warning L / D | Critical L / D |
| --- | --- | --- | --- | --- | --- |
| System (monochrome) | taskbar ink (2.7) | same | same at graph opacity | same | same |
| Ocean | `#1677FF` / `#70B7FF` | `#00A7C7` / `#5EE5FF` | `#68D5E8` / `#147EA3` | `#F59E0B` / `#FBBF24` | `#DC2626` / `#F87171` |
| Sunset | `#C241A7` / `#FF8BD8` | `#F97316` / `#FDBA74` | `#FB7185` / `#BE185D` | `#F59E0B` / `#FBBF24` | `#B91C1C` / `#FB7185` |
| Forest | `#16803A` / `#72E49A` | `#0F9F6E` / `#5EE6B8` | `#6CCF8D` / `#137A54` | `#D97706` / `#FBBF24` | `#B91C1C` / `#F87171` |
| Neon | `#16A34A` / `#39FF14` | `#0D9488` / `#00FFC6` | `#65A30D` / `#7CFF4D` | `#CA8A04` / `#FACC15` | `#DC2626` / `#FF3B5C` |
| Custom | user's, per role, per module | | | | |

(The macOS "System" preset also stores a blue `#2F7CF6` / `#6BA4FF` set for its flyouts; on Windows the
System theme's flyouts use the module signature colors in 2.5, as macOS does when monochrome.)

**Contrast reality.** The dark values pass on a dark taskbar (`#202020`): text 6.5–12.0:1, warning
9.8–10.6:1, critical 4.7–6.1:1. The light values do **not** pass on a light Windows taskbar (`#EEEEEE`):
text 2.8–4.3:1, warning 1.9–2.8:1. macOS accepted that; this design does not, because the Windows taskbar is
lighter and the text is often 9 DIP. Two mechanisms fix it, and the user never has to know:

1. **The backplate (2.7) is the default on light and accent-tinted taskbars for color themes.** On it, the
   dark values are used and pass: text 5.1–8.0:1, warning ≥ 6.5:1, critical ≥ 4.85:1, with one exception
   (Neon critical `#FF3B5C`, 3.9:1) where the app substitutes `#FF99A4` for that role only.
2. **When the user turns the backplate off on a light taskbar**, the strip uses these darkened variants,
   derived by scaling the light value until it reaches 4.5:1 on `#EEEEEE` (hue preserved). Graph strokes
   need only 3:1 (non-text).

| Theme | Text (light bar) | Graph | Warning | Critical |
| --- | --- | --- | --- | --- |
| Ocean | `#1366DB` (4.6) | `#0096B3` (3.0) | `#956007` (4.6) | `#D12424` (4.5) |
| Sunset | `#B23C9A` (4.5) | `#DE6614` (3.0) | `#956007` (4.6) | `#B91C1C` (5.6) |
| Forest | `#157C38` (4.6) | `#0F9C6C` (3.0) | `#A35904` (4.5) | `#B91C1C` (5.6) |
| Neon | `#117C38` (4.6) | `#0D9488` (3.2) | `#916303` (4.5) | `#D12424` (4.5) |

Warnings in light lose their amber and become ochre. That is the honest cost of 4.5:1 on a near-white bar; it
is also exactly what WinUI's own light caution color (`#9D5D00`) looks like, so it reads as native.

Custom colors: the swatch editor shows the live ratio against the current taskbar estimate and marks
anything under 4.5:1 with a caption ("3.1:1 on the current taskbar: hard to read"). It does not block.

### 2.7 The strip's ink and backplate

The ink is built and follows `SystemUsesLightTheme` - the taskbar's switch, not `AppsUseLightTheme`,
because the readout lives on the taskbar. **The backplate is not built**: nothing draws a plate behind the
strip, so `strip.plate` and the Backplate setting below belong with 2.6.

| Token | Dark taskbar | Light taskbar | Accent-tinted taskbar |
| --- | --- | --- | --- |
| `strip.ink` (System theme value text) | `#FFFFFF` (16.3 on `#202020`) | `#1B1B1B` (15.0 on `#EEEEEE`) | `#FFFFFF` (5.7 on `#0067C0`; re-tested per accent, black if higher) |
| `strip.label` | ink at 82 % alpha (11.0) | ink at 82 % (8.8) | ink at 82 % |
| `strip.stale` | ink at 70 % | ink at 70 % | ink at 70 % |
| `strip.hover` | white 8 % | black 6 % | white 8 % |
| `strip.open` | white 12 % | black 9 % | white 12 % |
| `strip.divider` | ink at 24 % | ink at 24 % | ink at 24 % |
| `strip.plate` | `brand.ground` at 92 % | same | same |

Taskbar theme comes from `HKCU\...\Themes\Personalize\SystemUsesLightTheme` and accent tinting from
`ColorPrevalence` in the same key; both change with `WM_SETTINGCHANGE`. The taskbar-ground *estimate* used
for contrast captions is `#202020` / `#EEEEEE` / the accent color; when Backplate is Automatic the app
additionally samples its own 1 × 48 DIP column of the taskbar once a minute (`BitBlt` from the screen DC,
one narrow strip: microseconds) so a transparent taskbar over a bright wallpaper is caught.

The plate: a rounded rectangle (radius 6) behind the whole strip, inset 6 DIP top and bottom, `brand.ground`
at 92 %. Composited it is `#1C1E22` on a dark bar, `#2D2F32` on a light one, `#1A242F` on the default
accent. White ink on it is ≥ 13:1 everywhere. It is the same ground as the app icon, so a color theme on a
plate is Barometer's signature look rather than a compromise.

Backplate setting: **Automatic** (default: on when the theme is not System and the taskbar is light or
accent-tinted, or the sampled ground contrast falls under 4.5:1; off otherwise), **On**, **Off**.

### 2.8 High contrast

When `SystemParametersInfo(SPI_GETHIGHCONTRAST)` reports on (and on `WM_THEMECHANGED`):

- Every token above is replaced by the system colors: `COLOR_WINDOW`, `COLOR_WINDOWTEXT`, `COLOR_HIGHLIGHT`,
  `COLOR_HIGHLIGHTTEXT`, `COLOR_BTNFACE`, `COLOR_BTNTEXT`, `COLOR_GRAYTEXT`, `COLOR_HOTLIGHT` for links.
- Mica and acrylic are disabled; every surface is opaque `COLOR_WINDOW`; every control gets a 1 DIP
  `COLOR_WINDOWTEXT` outline; focus ring is 2 DIP `COLOR_WINDOWTEXT` with no inner ring.
- The strip draws opaque `COLOR_WINDOW` behind `COLOR_WINDOWTEXT` ink; themes and the plate are ignored.

## 3. Type

Family: **Segoe UI Variable**, via DirectWrite, using the named optical instances Windows ships. Fallback on
systems without it (Windows 10): Segoe UI at the same sizes; the Medium weight falls back to Regular.

| Style | Size / line | Weight | Instance | Use |
| --- | --- | --- | --- | --- |
| Caption | 12 / 16 | Regular 400 | Segoe UI Variable Small | Descriptions, status lines, On/Off words, nav group headers |
| Body | 14 / 20 | Regular 400 | Segoe UI Variable Text | Everything that is not otherwise listed |
| Body Strong | 14 / 20 | Semibold 600 | Text | Section headers, inspector item name, dialog titles |
| Subtitle | 20 / 28 | Semibold 600 | Segoe UI Variable Display | Pane titles |
| Title | 28 / 36 | Semibold 600 | Display, `tnum` | Flyout hero value ("24 %") only |
| Strip | 9 / see 9.3 | user's, per family, headings and values chosen separately | Text, `tnum` | The taskbar readout |
| Mono | 13 / 18 | Regular | Cascadia Mono, fallback Consolas | Addresses, sensor identifiers in flyouts |

Rules:

- Tabular figures (`DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES`) on every live number: the strip, flyout
  values, the sampling readout in Settings. Proportional figures elsewhere.
- Semibold, not Bold, is the only emphasis. No italics anywhere.
- No all-caps except the strip's module labels (`CPU`, `MEM`, `NET`), which follow the macOS app.
- Text scaling: the app honors Windows' Accessibility text size (`SPI_GETLOGICALDPIOVERRIDE` is not it; read
  `UISettings.TextScaleFactor`) for the settings window and flyouts, up to 225 %. It does not apply it to
  the strip, which draws at the size chosen in 9.4; the strip has to fit the bar it is given.
- Antialiasing: ClearType in the settings window (opaque surfaces); grayscale
  (`DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE`) on the strip and in flyouts, because both sit on translucent
  composition where ClearType fringes.
- Rendering mode `DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC`, vertical pixel snapping on, so baselines land on
  whole pixels at every scale factor.

## 4. Spacing and grid

Base unit 4. Scale: **4, 8, 12, 16, 20, 24, 32, 40, 48**. Nothing else, except 3 for the accent pill width and
1 for hairlines.

| Measure | Value |
| --- | --- |
| Title bar | 32 |
| Nav pane width | 176 (items inset 8 each side, so 160 wide) |
| Nav item | 36 tall, 4 gap, 12 left text padding, radius 4 |
| Pane padding | 24 left/right, 24 top, 32 bottom (room under the last card) |
| Pane title to caption | 4; caption to first content | 20 |
| Section header (Body Strong) to its card | 8 |
| Between sections | 24 |
| Card inner padding | 16 horizontal; rows are full-bleed with dividers |
| Settings row | 48 (label + control) / 64 (label + description + control); content vertically centered |
| Row label column | starts at 16; control column right-aligned ending at 16 |
| Composer list row | 52 |
| Dropdown item | 36; search result row 48 |
| Control height | 32 (button, dropdown, text field); toggle 20; checkbox 20 |
| Minimum control widths | button 96, dropdown 160 in a row, text field 200 |
| Gap between adjacent controls | 8; between a control and its On/Off word | 12 |

The grid is "everything sits on a multiple of 4 from the pane origin". A row's text baseline is not forced
to the grid; the row box is, and text is centered within it.

## 5. Controls

Every control has five states: rest, hover, pressed, focus (keyboard only), disabled. Focus is drawn
*in addition to* the current state, never instead of it. Hover states change instantly (no tween).

Common: radius 4; stroke 1 DIP inside the bounds; text Body; disabled means `control.disabled` fill,
`stroke.control` stroke, `text.disabled` text, and the pointer does not change. Disabled controls always
carry a Caption line saying why (never a bare gray control).

### 5.1 Button (standard)

| State | Fill | Stroke | Text |
| --- | --- | --- | --- |
| Rest | `control.rest` | `stroke.control`, bottom edge `stroke.control.edge` (top edge in dark) | `text.primary` |
| Hover | `control.hover` | same | same |
| Pressed | `control.pressed` | `stroke.control` all round (edge lift removed) | `text.secondary` |
| Focus | as state | as state | + focus ring |
| Disabled | `control.disabled` | `stroke.control` | `text.disabled` |

32 tall, padding 12 horizontal, minimum width 96, text centered. An optional 16 DIP glyph sits left of the
text with an 8 gap. Buttons that open something outside the app (a browser, a Windows settings page) end
their label with the "open in new" glyph (`E8A7`), so the user knows before clicking.

### 5.2 Button (accent)

Rest `accent.fill` / `text.on-accent`, no stroke; hover `accent.hover`; pressed `accent.pressed`; disabled
`control.disabled` with `text.disabled`. One accent button per pane at most, and only for the primary
positive action ("Download update", "Add" in the search results). Never for destructive actions.

### 5.3 Hyperlink button

Text-only, `accent.text`, no fill; hover underline; pressed `accent.text` at 80 % alpha; focus ring at
the text bounds + 2. Used for in-app navigation ("Open Sensors") and small secondary actions ("Refresh now",
"Reset all settings…"). Adjacent hyperlink buttons are separated by a `·` in `text.tertiary` with 8 each side.

### 5.4 Toggle switch

Track 40 × 20, radius 10; knob circle.

| State | Track (off) | Knob (off) | Track (on) | Knob (on) |
| --- | --- | --- | --- | --- |
| Rest | transparent, 1 DIP `stroke.strong` | 12 dia, `text.secondary`, inset 4 | `accent.fill` | 12 dia, `text.on-accent`, inset 4 |
| Hover | `subtle.hover` fill | 14 dia | `accent.hover` | 14 dia |
| Pressed | `subtle.pressed` | 14 dia, stretched to 17 wide toward travel | `accent.pressed` | same |
| Disabled | `stroke.control` ring | `text.disabled` | `control.disabled` | `text.disabled` |

The state word ("On" / "Off", Caption, `text.secondary`) sits 12 to the *left* of the switch when the
switch is right-aligned in a row (the Windows Settings convention), so color is never the only signal. Knob
travel animates 150 ms (section 7); under reduced motion it jumps.

### 5.5 Dropdown (ComboBox)

32 tall, minimum 160, padding 12 left, chevron `E70D` 12 DIP at 12 from the right in `text.tertiary`.
Same fills and strokes as the standard button. Open state: `control.pressed` fill; the popup (1.3) lists
items 36 tall with 4 inset, 12 left padding; the selected item shows the 3 × 16 `accent.pill` at its left
edge and `subtle.hover` fill; hover on other items `subtle.hover`. Popup opens below with 4 gap, flips above
when there is not room. Keyboard: Alt+Down or Space opens; Up/Down move; Enter commits; Esc restores.
Typing letters jumps. Width of the popup = max(control width, widest item + 24).

### 5.6 Text field

32 tall, `control.rest` fill, `stroke.control` sides and top, **`stroke.strong` bottom edge** (rest) →
**2 DIP `accent.fill` bottom edge** (focus). Placeholder `text.secondary`. Clear button (`E711`, 16, in
`text.tertiary`, hover `text.secondary`) appears at the right when the field is non-empty and focused. Caret
and selection come from the underlying `EDIT` control, restyled: selection fill `accent.fill` at 40 %.
Search variant: `E721` glyph at the left (12 from edge), placeholder "Search cities", and results appear in a
popup (5.9) rather than inline. Commit on Enter or blur; Esc reverts to the last committed value.

### 5.7 Slider

Rail 4 tall, radius 2, `stroke.strong`; filled portion `accent.fill`; thumb: 20 dia outer circle
`control.rest` with 1 DIP `stroke.control`, inner dot `accent.fill` 12 dia (rest), 14 (hover), 10 (pressed).
No value tooltip: the value is always visible as a Caption label to the right of the rail (48 wide, right-
aligned, `tnum`) with the unit ("12 pt", "5 s", "40 %"). A tooltip needs a popup and a timer; a label needs
neither and is always readable. Keyboard: Left/Right step 1, PageUp/Down step 5, Home/End. Snap points are
drawn as 1 × 4 ticks in `stroke.control` under the rail only when there are ≤ 12 of them.

### 5.8 Checkbox and radio

20 × 20, radius 4 (checkbox) / 10 (radio). Rest: `control.rest` fill, 1 DIP `stroke.strong`. Checked:
`accent.fill` fill, `E73E` check at 12 DIP in `text.on-accent` (radio: 8 dia dot). Hover: `control.hover` /
`accent.hover`. Label Body 8 to the right; the whole label is the hit target. Radios are used only inside a
card where the options need descriptions; otherwise a dropdown.

### 5.9 List rows, search results, chips

- **Settings row**: 48/64 tall, label Body at x 16, description Caption `text.secondary` under it (64 rows),
  control right-aligned. Rows are not interactive themselves; the control is.
- **Composer row** (the strip list): 52 tall, described in `ui-layouts.md` §2. Rest transparent; hover
  `subtle.hover`; selected `subtle.hover` + `accent.pill` at x 0 (3 × 16, vertically centered); pressed
  `subtle.pressed`; dragging: `surface.card` fill + 1 DIP `stroke.strong`, drawn last.
- **Search result row**: 48 tall, primary Body `text.primary` ("Austin"), secondary Caption `text.secondary`
  ("Texas, United States"), optional right-aligned Caption `text.tertiary` ("2.0 M" population, which is how
  the three Austins are told apart at a glance in addition to the region). Hover `subtle.hover`; keyboard
  highlight the same plus the pill; Enter or click adds it.
- **Chip** (chosen sensors, world clocks): 28 tall, radius 14, `control.rest` fill + `stroke.control`, text
  Body at 12 padding, remove glyph `E711` 12 DIP at the right with 8 gap; hover `control.hover`; focus ring
  around the chip; Delete removes the focused chip; Alt+Left/Right reorders.

### 5.10 Section header

Body Strong, `text.primary`, 8 above its card, 24 below the previous card. Optional trailing hyperlink
button at the right edge (e.g. "Open Taskbar settings"). Never underlined, never colored.

### 5.11 Icon tile

Flat square, radius 4 (32 tile) or 8 (46 tile), module primary color, glyph 16 (or 24) in the ink from
2.5. In the composer list the tile is 20 with a 12 glyph. No shadow, no gradient.

### 5.12 Status line

8 DIP dot + 8 gap + Body text, optional Caption second line. Dot color from 2.3. The dot is drawn with a
1 DIP inner highlight only in high contrast (where it becomes an outlined circle).

### 5.13 Info strip (in-pane notice)

Card variant: `status.*` badge fill, 1 DIP stroke of the status color at 40 %, 16 padding, an `E946` info
glyph (or `E7BA` warning) at 16 DIP, Body text, optional hyperlink button at the right. Used sparingly: the
"Location is off for desktop apps" notice, the update-available notice. Never for the Sensors empty state,
which is a plain status line (2.3).

## 6. Iconography

Segoe Fluent Icons, from the system (present on Windows 11; on Windows 10 fall back to Segoe MDL2 Assets,
same code points for everything used here). Drawn as text through DirectWrite at 16 DIP (12 in dense
places), color `text.primary` / `text.secondary` / `text.tertiary` by role.

The rule from Clicker carries over unchanged: **one table maps names to code points, and no inline
`\u{E7xx}` literal appears anywhere else.** Verified entries from that table and the WinUI symbol set:

| Name | Code point | Use |
| --- | --- | --- |
| ChevronDown / Up / Right | `E70D` / `E70E` / `E76C` | Dropdowns, expanders, nav |
| Search | `E721` | Search field |
| Cancel | `E711` | Clear, remove chip |
| CheckMark | `E73E` | Checkbox, "up to date" |
| Add | `E710` | Add item / reading / clock |
| More | `E712` | Overflow |
| Settings | `E713` | Flyout footer |
| Refresh | `E72C` | Weather refresh, check for updates |
| Copy | `E8C8` | Addresses in flyouts |
| Info | `E946` | Info strip |
| Warning | `E7BA` | Info strip (caution) |
| OpenInNewWindow | `E8A7` | Buttons that leave the app |
| GripperBarVertical | `E784` | Drag handle |
| Globe | `E774` | Location search |
| Wifi | `E701` | Network flyout |
| Cloud | `E753` | Weather (settings only) |

Module glyphs for nav tiles and the strip's icon modes (CPU, GPU, memory, disk, thermometer, battery, clock)
are **Barometer's own 16 DIP marks drawn as Direct2D geometry**, not font glyphs: Segoe Fluent has no
thermometer or GPU and its battery and clock glyphs are the wrong weight next to 12 DIP Medium text. The
marks are single-color, 1.5 DIP stroke, optical size tuned at 16, and the same marks render in the strip,
so an icon in Settings and the icon beside a value in the taskbar are identical. Weather condition marks
(sun, cloud, rain, snow, fog, bolt, moon) are the macOS app's "Barometer's own" set redrawn as geometry,
with the optional color variant (amber sun, lavender night, blue rain) behind the same "Color weather
icons" toggle.

## 7. Motion

Four transitions. Everything else is instant.

| What | Duration | Easing | Notes |
| --- | --- | --- | --- |
| Toggle knob travel | 150 ms | decelerate `cubic-bezier(0, 0, 0, 1)` | Timer runs only during travel |
| Nav accent pill moving to the new item | 150 ms | decelerate | Pane content itself switches instantly |
| Flyout open | 150 ms fade 0→1 + 8 DIP slide away from the taskbar | decelerate | Close: 100 ms fade, no slide |
| Composer rows shifting during a drag | 120 ms | decelerate | The dragged row follows the pointer with no easing |

Reduced motion (`SPI_GETCLIENTAREAANIMATION` off): all four durations become 0. Hover, pressed and focus
changes are never animated. Values in flyouts do not roll or cross-fade (the macOS `numericText`
transition is dropped: it is a timer for nothing). Graph lines in an open flyout redraw when a sample
arrives, which is the only "animation" in the product, and it stops the instant the flyout closes.

## 8. Accessibility

- **Keyboard**: every control reachable by Tab in reading order; nav pane is one Tab stop with Up/Down
  inside; composer list is one Tab stop with Up/Down inside and Alt+Up/Down to reorder; Ctrl+Tab /
  Ctrl+Shift+Tab switch panes from anywhere; Esc closes popups, then the window. F6 cycles nav → list →
  inspector.
- **Focus visual**: shown only after keyboard input (`WM_UPDATEUISTATE` / `UISF_HIDEFOCUS` convention).
  2 DIP `focus.outer` ring at 1 DIP outside the control, radius = control radius + 2, plus 1 DIP
  `focus.inner` inside it. In lists, the ring wraps the row.
- **Contrast**: all words ≥ 4.5:1 (section 2 tables); UI component boundaries ≥ 3:1 (`stroke.strong`);
  disabled text is exempt and always accompanied by a reason line in `text.secondary`.
- **UI Automation**: the settings window exposes a provider tree (`IRawElementProviderSimple`) with control
  types for every drawn control; names are the visible labels; toggles expose `TogglePattern`, sliders
  `RangeValuePattern`, lists `SelectionPattern`, the composer rows `Invoke` + `Toggle`. The strip exposes one
  `Group` per module with `Invoke` (opens the flyout) and the live reading as `Value`; names are static
  ("CPU"), values change. Flyouts are dialogs with `Escape` documented.
- **High contrast**: section 2.8.
- **Reduced motion**: section 7.
- **Text scaling**: section 3.
- **Right-click** and **Shift+F10** open the same context menus.

## 9. The strip (the taskbar readout)

### 9.1 What it is

One layered `WS_POPUP` **owned by** the taskbar - never a `WS_CHILD` of it, which the shell's own XAML
surface composites away; see AGENTS.md - `taskbar height` tall (48 at 100 %), as wide as the tray space
reserved for it (section 9.2), drawn with `UpdateLayeredWindow` over a per-pixel-alpha ground so the
taskbar's own material shows through. It holds every enabled item in order, laid out left to right. It is Barometer's
single point of presence on Windows; there is no separate tray icon.

### 9.2 Width, slots and slack

The reservation is not asked for, it is taken, by planting transparent placeholder icons in the
notification area (AGENTS.md, "How the strip gets space of its own"), so it is quantized to tray-icon
slots: 42 px at 100 % on Windows 11 as a seed, and then measured from where the placeholders actually
landed with `Shell_NotifyIconGetRect`, because the pitch changes with DPI and OS builds. The strip's
content width is the sum of item widths and gaps; the reserved width is `ceil(content / slot) × slot`.
The difference, **slack**, is 0 to one slot minus 1 px.

Slack stays *inside the strip*, never left as a hole beside it: the content is centered in the reserved
region, so the slack is split evenly between the two ends and reads as margin. Left-aligning would pile
all of it against the right-hand end, against the tray, where it reads as a gap somebody forgot to close.
There is no end-padding setting: asking a few pixels past a slot boundary buys a whole further slot of
taskbar, and the margin is already there.

Windows may still leave up to one task-button width of blank between the last task button and the tray;
that is Explorer's quantization and not ours. There is no pane to say so in: the Taskbar pane that was
to carry that sentence was never built (`ui-layouts.md` §5).

### 9.3 Band, rows, baselines

The taskbar is 48 tall but its content band is 32 (the clock is two rows of 12 DIP text; tray icons are 16
centered). The strip uses the same band: 8 above, 8 below.

```
y=0   ┌─────────────────────────────────────────┐
      │                 8 (top margin)           │
y=8   │  row 1: line box 16   baseline at y=20   │  ← labels of stacked items; single-row items are centered on the band
y=24  │  row 2: line box 16   baseline at y=36   │  ← values of stacked items
y=40  │                 8 (bottom margin)        │
y=48  └─────────────────────────────────────────┘
```

- Every stacked item's label baseline is `y = 20` and its value baseline `y = 36`, strip-wide. Two-row
  items therefore **share one baseline**, as on macOS.
- Single-row items (percentage, icon + value, arrows with rates on one line) center on the band: baseline
  at `y = 28.5`, snapped to a pixel.
- Graphs occupy the full 32 band; bars (per-core) the same; a usage bar is 6 tall centered on row 2 with
  its label on row 1.
- The font is one size, the user's (9.4), so the line box is one size too; the band stays 32 whatever the
  bar's height, and the two rows float toward the center rather than toward the edges. A size two rows of
  which the bar cannot hold is held down to the largest it can; there is no one-row layout
  (`Density::choose`).

### 9.4 Type in the strip

The user's family and weight (Appearance), tabular figures, grayscale antialiasing, color from 2.7 or the
theme. Headings and values are chosen separately, and only the weights the chosen family actually has faces
for are offered - GDI never refuses a weight, it smears the nearest face, so offering five on a
single-weight family was offering four lies.

**One size, the user's, 9 DIP by default** (`taskbar::TEXT_DIP`, `StripFont::size_dip`). macOS steps its
type from 12 down to 9 as items are added; here the strip starts at the bottom of that ladder and stays
wherever the user puts it, because its width is paid for by the task buttons beside it and the shell gives
those up a whole button at a time - a readout that grows whenever it has fewer items spends that room on
nothing anyone asked for. The Text size slider (6-24) sets the size itself; an earlier version made it a
ceiling on an automatic size, which confused more than it helped. Two rows always; a size two rows of
which do not fit the bar is held to the largest that does, which the slider's caption reports. The strip
never lays itself out as one row. Nothing else steps with the item count either; the marks and
graphs are sized from the band and the text, not from a ladder.

Labels (`CPU`, `MEM`, `NET`, sensor names) are drawn at 82 % alpha of the ink; values at 100 %. That single
difference is what stops a strip of eight items from being a run-on sentence: the eye lands on values.

### 9.5 Widths: fixed by design

Each item reserves the width of its widest plausible reading and never changes width while its numbers
change ("100 %", "999.9 KB/s", "-99.9°"). Reserved strings are measured on the window's own DC in the font
that will draw them, and cached. There is no setting for this: a taskbar that breathes every second is the
single most common complaint about this genre of app, and live widths were tried and reverted.

Approximate widths, worked at 12 (measure at runtime; the strip draws at 9, and these are for layout
planning only):

| Item | Reserved string | Width (DIP) |
| --- | --- | --- |
| Percentage | `100%` | 30 |
| Stacked label over percentage | `MEM` / `100%` | 30 |
| Stacked rates (Network two-line) | `↓ 999.9 KB/s` / `↑ 999.9 KB/s` | 72 |
| Rate arrows on one line | `↓999.9K ↑999.9K` | 96 |
| Temperature | `-99.9°` | 38 |
| Icon + temperature (Weather) | mark 16 + 4 + `-99°` | 50 |
| Condition mark over temperature | max(16, `-99°`) | 30 |
| History graph | user width 24–96 | default 40 |
| Per-core bars | cores × 3 + (cores − 1) × 1 | 16 cores → 63 |
| ~~Battery glyph with percentage~~ | dropped with the module | — |
| ~~Time~~ | dropped with the module | — |

### 9.6 Separation between items

Two cooperating mechanisms, in order of strength:

1. **Gap**: one slider, 0 to 24, **3 by default** (`DEFAULT_COLUMN_GAP_DIP`). There is no end-padding
   setting; the margin comes from the slot slack (9.2). Compact from the first run and
   deliberately so: every pixel spent between columns is a pixel that can tip another task button into the
   overflow, and three is enough that two columns of digits still read as two numbers. Fourteen, the first
   figure, was far too much. The named steps this section used to specify - Normal 12, Snug 8, Tight 4 -
   were never built; the slider replaced them.
2. **Shape contrast**: a stacked item next to a graph next to an icon + value already reads as three
   things. The default composition (CPU stacked, MEM stacked, NET two-line rates) is deliberately not three
   identical shapes.

Dividers between items were specified here and are not built. At a 3 DIP gap they would have nowhere to go.

### 9.7 Interaction states on the strip

| State | Drawing |
| --- | --- |
| Rest | Nothing behind the item |
| Hover | `strip.hover` rounded rect, radius 4, inset 4 top/bottom, spanning the item's width + 8 (4 each side, into the gap) |
| Pressed | `strip.open` fill |
| Open (its flyout is showing) | `strip.open` fill for as long as the flyout is open |
| Keyboard focus (UIA / Narrator) | focus ring per section 8 around the item |
| Stale (weather older than two refresh intervals; sensor source lost) | value ink at 70 %, mark unchanged |
| Unavailable (module enabled but has no data: Sensors without a source, Weather without a location) | item is **not drawn at all**; the reservation shrinks. The composer row carries the reason. An empty placeholder in the taskbar is worse than nothing. |
| Warning / critical reading (threshold per module, e.g. CPU temp ≥ 85 °C) | value ink switches to the theme's warning / critical role; the label does not |

This is the same hover backplate the taskbar gives its own buttons, so the module boundaries teach
themselves the first time the pointer crosses the strip.

### 9.8 DPI

| Scale | Taskbar | Band | 9 DIP text (the default) | Slot pitch (measured; typical) | Notes |
| --- | --- | --- | --- | --- | --- |
| 100 % | 48 px | 32 px | 9 px | 42 px | |
| 125 % | 60 px | 40 px | 11 px | 52 px | |
| 150 % | 72 px | 48 px | 14 px | 63 px | |
| 200 % | 96 px | 64 px | 18 px | 84 px | |

The strip's HWND is `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`; on `WM_DPICHANGED` it re-measures the slot
pitch, re-creates text formats, re-measures reserved strings, and re-requests its reservation. Fonts are
requested in DIPs and DirectWrite is given the true scale factor; nothing is pre-rounded to pixels except
1 DIP strokes, which are snapped to one physical pixel at 100–150 % and two at 200 %.

### 9.9 Rendering budget

Redraw only when a sample changes (per module, ≥ 1 s) or on hover transitions. One `BeginDraw/EndDraw` per
redraw, ≤ 1 ms. Text layouts are cached per item and rebuilt only when the string changes. Brushes are
created once per theme. No timers run when nothing changes; hover does not start a timer.

## 10. Flyout panels

- **Size**: 320 wide (matching macOS). Height = content, capped at min(720, work-area height − 16); taller
  content scrolls inside.
- **Anchor**: horizontally centered on the clicked item's rect. Bottom taskbar: panel bottom = taskbar top − 8.
  Top taskbar: panel top = taskbar bottom + 8. Left/right taskbar: panel beside it with 8 gap, vertically
  centered on the item. Then clamp to the monitor's work area inset by 8 on every side (the macOS
  `PopoverPlacement.containedFrame`, same 8). Near a screen edge the panel slides along the taskbar and the
  item keeps its "open" backplate, so the association survives without a beak.
- **Material**: section 1.4. Cards inside: `surface.card` at 70 % (light) / white at 6 % (dark) over the
  acrylic, radius 8, 12 padding, 10 between cards, 12 panel padding (the macOS spacing constants; the radius
  is Windows').
- **Header card**: tinted with the module color at 12 % over the card; module tile 32; Title value (28,
  `tnum`) right-aligned; Body caption beneath.
- **Rows**: 36 tall, hover `subtle.hover`, radius 4; process rows carry an icon 16, name, value `tnum`, and
  an action glyph that appears on hover (end task `E711`, copy `E8C8`).
- **Graphs**: 1 DIP line in the graph role, area fill in the fill role at the graph-opacity setting
  (default 30 %), 1 DIP hairline axis in `stroke.divider`, Caption tick labels in `text.secondary`, time-range
  dropdown (1 min … 24 h) in the card's top-right.
- **Footer**: 40 tall, hairline above, `E713` Settings icon button at the left (opens Settings with this
  item selected in the composer), module actions at the right ("Refresh" for Weather, "Copy address" for
  Network).
- **Open/close**: click the item (or Enter on it via UIA). One flyout at a time; clicking another item swaps
  instantly. Clicking the open item closes it.
- **Dismiss**: pointer down outside the panel (the panel is a foreground window that closes on
  `WM_ACTIVATE`/`WA_INACTIVE`, the way Windows' own tray flyouts do), Esc, focus leaving to another window,
  Start/Win key, the item being disabled, or the taskbar moving.
- **Keyboard inside**: Tab cycles interactive rows and the footer; Esc closes; the time-range dropdown is a
  normal dropdown.
- **Live updates**: the panel observes the module store and redraws on new samples while open. Nothing
  tweens.

## 11. Context menu (right-click on the strip)

A `TrackPopupMenuEx` popup, owner-drawn rather than left to the OS so it follows the taskbar's theme
instead of the app one: "Settings...", "Check for updates", separator, "Exit Barometer". Three rows, not
five - there is no "Hide *CPU*", because the item under the pointer is a column of a strip rather than a
window of its own, and no "Pause updates", which nobody asked for. This is where quitting lives; the
settings window has no quit control, matching Windows conventions.

## 12. Resource rules that shape the visuals

- No effect requires a per-frame composition pass: Mica and acrylic are DWM's, not ours.
- The only periodic work outside sampling is the optional once-a-minute 1 × 48 DIP ground sample for
  Backplate = Automatic.
- Fonts, brushes, geometries (module marks), and text layouts are cached; the strip never allocates per
  frame.
- An open flyout is the only time a graph is drawn more than once per settings change.

## 13. Deviations from the macOS sibling, and why

| macOS | Windows | Why |
| --- | --- | --- |
| N independent menu bar items, ordered by Cmd-drag in the bar | One strip; order and visibility in the Strip composer | Windows has no multi-item tray API. |
| One settings pane per module in the sidebar; General holds appearance, spacing, colors, units | Sidebar: Strip, then a page each for CPU, GPU, Memory, Disks, Network, Sensors and Weather, then Stacks, Appearance, General, About | The composer needed a pane of its own, and the module inspectors that lived inside it were reachable only by selecting a row, invisible to anybody who had switched that module off, and capped at a 240 DIP column - so the macOS shape won after all. |
| "Apply Changes" staging bar for visibility changes | Everything applies live | The staging exists for macOS 27 status-item geometry; the strip is one window we own. |
| Focus and Now Playing modules | Not in v1 | Outside the module set specified for the Windows app. |
| Time module | **Dropped.** Windows draws its own clock on the same taskbar | A second clock spends strip width duplicating the shell. |
| Sensors read from IOHID/SMC directly | Sensors read through a .NET helper that loads LibreHardwareMonitor's library from the user's own machine; "no source" is a first-class calm state | No Windows API exposes temperatures, and the library is never redistributed. |
| Battery module always present | **Dropped.** Windows already shows battery in the tray, with time remaining | Battery *sensors* still arrive through Sensors. |
| Theme light values used as-is on a light menu bar | Backplate by default on light/accent taskbars; darkened variants when the plate is off | The light values are 2.8–4.3:1 on a Windows light taskbar. |
| Gradient icon tiles, 14 / 10 radii, glass cards | Flat tiles, 8 / 4 radii, acrylic panel with layer cards | Windows 11 idiom and the brand's "no gradients". |
| Memory pressure | In use / committed | Windows' terms (Task Manager). |
| Compact two-row point size = thickness/2 − 2 | Two rows at the full item size | The Windows band is 32 DIP, the macOS bar 24; two rows of 12 pt fit. |

## 14. Where this departs from the brief

- **"Text color with an option to follow the system accent."** Provided as the *Custom* theme's swatch
  menu entry "Windows accent", but it is not the default and it auto-disables when the taskbar is
  accent-tinted: accent text on an accent taskbar is 2.8:1 (default blue on itself). The default is the
  taskbar's own ink.
- **"One-line vs two-line layout."** Replaced by per-item readout styles (stacked items are two rows; the
  strip itself is always one row). A global two-line switch fights the module model and, on a 48 DIP taskbar,
  every item already gets two rows.
- **"Font size"** sets the size itself, 6-24, rather than a ceiling on an automatic one (§9.4).
- **"Item spacing"** was to be three named steps rather than a slider. It ended up a slider after all, 0 to
  24, because the useful range turned out to be at the bottom of that scale (default 3, §9.6) and named
  steps could not say the difference between 2 and 4.
- **Weather and Sensors** were to be inspectors in the composer rather than panes. Every module has a page
  of its own now (§13), and the sensor source and the weather locations are sections on theirs.
- **Taskbar space** was to be a top-level pane with a before/after diagram. There is no such pane: the
  reservation is not a setting anybody chooses - without it there is no readout - so there was nothing to
  put on the page but a picture.
