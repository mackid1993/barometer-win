# Third-party notices

Barometer for Windows is free software under the GNU General Public License,
version 3. See [LICENSE.md](LICENSE.md).

This file lists what Barometer uses that other people wrote, what those parts
are licensed under, and - where it matters - what that license asks of anyone
who redistributes this.

## LibreHardwareMonitor

Temperatures, fan speeds and voltages come from
[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor),
under the **Mozilla Public License 2.0**. Barometer links
`LibreHardwareMonitorLib` into `barometer-sensors.exe`, a small helper program
whose whole job is to read sensors and print them.

MPL-2.0 is a file-level copyleft. `helper/Program.cs` carries an MPL-2.0 header
because it is written against that library's interfaces, and the library itself
is redistributed unmodified. Anyone receiving a build of the helper is entitled
to the source of the MPL-covered files, which is in this repository and at the
upstream project above. MPL-2.0 is compatible with GPL-3.0, which is what makes
the combination in this repository possible at all.

Windows publishes no API for temperature, fan or voltage sensors. There is no
version of this program that reads them without something like
LibreHardwareMonitor, and writing a replacement would mean writing kernel-level
hardware access, which is explicitly not what this project does.

## PawnIO

LibreHardwareMonitor reads some hardware through
[PawnIO](https://github.com/namazso/PawnIO), under the **GNU General Public
License, version 2**, with its hardware modules under the **GNU Lesser General
Public License, version 2.1**.

PawnIO is not bundled. It is a signed kernel driver that the user installs
themselves, and the reason that is the right arrangement is worth stating: a
program that ships its own kernel-level hardware access is a program antivirus
software is right to be suspicious of. Barometer stays entirely in user space
and talks to what the user chose to install.

## Weather data

Current conditions come from [Open-Meteo](https://open-meteo.com/), free for
non-commercial use and requiring no account or key. Weather data is licensed
CC BY 4.0 by its providers; Open-Meteo's own attribution requirements are
honored in the settings pane.

Approximate location, when the user has not chosen one, comes from
[ipapi.co](https://ipapi.co/) with [ipwho.is](https://ipwho.is/) as a fallback.
Neither is asked more than once per run.

## Rust crates

- [`windows-sys`](https://github.com/microsoft/windows-rs) - MIT or Apache-2.0.
  Raw bindings to the Win32 API.
- [`serde_json`](https://github.com/serde-rs/json) - MIT or Apache-2.0.
- [`winresource`](https://github.com/BenjaminRi/winresource) - MIT. Build time
  only; it compiles the icon and version block into the executable and is not
  part of what ships.

Full license texts for these are reproduced in `licenses/`.

## Barometer for macOS

This is a port. The design, the module set, the stack model, the settings
vocabulary and the fourteen weather marks come from
[Barometer](https://github.com/mackid1993/Barometer) for macOS, by the same
author, and the Swift is the reference the Rust is written against rather than
inspiration for it.

## Everything else

The taskbar reservation - the transparent placeholder icons that make Windows
repack its task buttons and leave the readout room - is the author's own work,
first written for a pull request against
[TrafficMonitor](https://github.com/zhongyang219/TrafficMonitor). It is carried
here as the author's code under this project's license. **No TrafficMonitor
code is used**; that project is licensed "Anti 996" v1.0, which is not
compatible with this one, and the two share nothing but a problem.
