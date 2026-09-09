<div align="center">

<img src="assets/AppIcon.png" width="144" alt="Barometer">

# Barometer for Windows

**Weather and system statistics, in the system tray.**

![Platform](https://img.shields.io/badge/windows%2011-4fd8f5?style=flat-square)
![License](https://img.shields.io/badge/license-GPL--3.0-4fd8f5?style=flat-square)
![Built with](https://img.shields.io/badge/rust-4fd8f5?style=flat-square)

</div>

A companion to [Barometer for macOS](https://github.com/mackid1993/Barometer),
which lives in the menu bar. This one lives in the Windows 11 system tray and
shows the same things: network throughput, processor and memory load, graphics load,
disk throughput, temperatures, and the weather.

The two are separate programs rather than one program built twice - that one is
Swift against AppKit, this one is Rust against Win32 - so they do not have all
the same features and their version numbers do not line up. Each has its own
releases here on GitHub, and each updates itself from its own.

## What it looks like

The readout is transparent. There is no plate and no container: the numbers sit
directly in the notification area, and the weather is the temperature with its
condition drawn in the space above and below it.

It claims room by registering transparent placeholder icons in the notification
area, which makes Windows repack the task buttons aside. That is the only way
to get space in a Windows 11 system tray without drawing over somebody's
window button. If another program's icon is dragged into that space the readout hides
itself rather than covering it, and comes back when the icon leaves.

## What it shows

Seven modules. Each can be on the strip or off it, in whatever order you drag
them into, and each opens a panel when clicked.

| Module | On the strip | In its panel |
| --- | --- | --- |
| **Processor** | Total load, or the user and system split | Per-core bars, a timeline, load average, uptime, and the busiest processes with an end-task button |
| **Graphics** | Utilization of the busiest engine | Per-engine load, memory in use, clock, power and temperature, with an adapter picker for machines with more than one |
| **Memory** | Used, free, or the percentage | A breakdown of what the memory is doing, commit charge, page file, and the hungriest processes |
| **Disks** | Read and write rates, for every disk together or one you choose | Every physical disk with its model and rates, and the space used on a volume you choose |
| **Network** | Upload and download rates, either order | Per-interface and per-process throughput, addresses, and your public address if you want it |
| **Sensors** | Any one temperature, fan or voltage you pick | Everything the source reports, grouped by device, with a timeline each |
| **Weather** | Temperature with its condition drawn around it | Now, the next forty-eight hours, ten days, air quality, sunrise and moon |

Any of those readings can also go into a **stack**: several values in one
column, which is how you fit more on without making the strip longer.

## Requirements

| | |
| --- | --- |
| **Windows 11** | x64. Windows 10 is refused by the installer - see below |
| **Taskbar on top or bottom** | A side-docked taskbar is refused with a message |
| **Administrator** | Required. The installer registers a sign-in task so this is silent |
| **A .NET runtime** | Not needed. The sensor helper carries its own |
| **[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)** | Optional, for temperatures, fans and voltages. Barometer can fetch it for you, into your own AppData, and ships none of it |
| **[PawnIO](https://pawnio.eu)** | Optional, for the processor's temperatures and the motherboard's fans, voltages and board temperatures. Graphics and drive temperatures arrive without it. Somebody else's signed driver, installed by you |
| **Internet** | Only for what asks: the weather, your approximate location if you want it, and update checks |

Everything optional above is genuinely optional: anything unavailable reads as
unavailable rather than as a wrong number.

**Windows 11.** Not a preference. The mechanism above is specific to the
Windows 11 taskbar, and on Windows 10 Barometer would install and draw nothing;
the installer refuses to run there rather than letting that happen.

Windows 10 does not need it in any case. It still supports deskbands, so it has
every system monitor under the sun already. Windows 11 removed deskband support
and took all of them with it, which is the hole this fills.

**Top or bottom.** A taskbar docked to the left or right is refused with a
message rather than run: widening the notification area frees no horizontal
space there, so the readout has nowhere to go. The small 26H2 taskbar is fine -
the strip measures the bar and sizes itself to whatever it finds.

**Barometer runs as administrator.** Most hardware sensors are behind
instructions and ports that user mode may not touch - the processor's
temperatures in model-specific registers, the motherboard's fans, voltages and
board temperatures behind the SuperIO chip, the embedded controller or the
SMBus - and reading any of them is a ring-0 operation. Measured on the machine
this was written on, same binary, same driver, elevation the only difference:
unelevated, 144 sensors and every one of the 38 temperatures unreadable;
elevated, 196 sensors and all 43 readable, including 35 CPU cores.
There is no unprivileged path to the number. The installer registers a
scheduled task so sign-in is silent; only a manual launch prompts.

**Temperatures need LibreHardwareMonitor, which Barometer does not ship.**
Windows publishes no API for temperature, fan or voltage sensors.
[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
does the reading; you install it and Barometer loads it from wherever you put
it. Nothing of it is in our binary, and you update it on your own schedule.

**Most sensors additionally need PawnIO.** [PawnIO](https://pawnio.eu) is a
small third-party signed kernel driver, and it is what actually does the
privileged reading: LibreHardwareMonitor carries a PawnIO module for each way
into the hardware - `IntelMSR`, `AMDFamily17` and `RyzenSMU` for the
processor, `LpcIO` for the SuperIO chip's fans and voltages, `LpcACPIEC` and
`IsaBridgeEC` for embedded controllers, `SmbusI801` and friends for memory
module temperatures - and runs them inside the driver.

So this is not only about the processor. What arrives without PawnIO is what
has a path of its own: the graphics card, which vendor libraries report, and
the drives, which answer SMART and NVMe queries directly.

You install it yourself, from its own site. Barometer contains no kernel
driver of its own and never will; see [NOTICE.md](NOTICE.md).

Barometer tells those apart rather than reporting one vague "sensors do not
work": PawnIO absent, or PawnIO present but closed to an unelevated process.
They have different answers, and a machine that already has PawnIO is never
told to go and install PawnIO.

If it is missing, Barometer says so **once** - what it costs, that it is
optional, and that it is somebody else's driver - and offers to open the
download page. Whichever you answer, it does not ask again.

PawnIO is optional. Everything else works without it, and anything unavailable
reads as unavailable rather than as a wrong number.

## Building

```
cargo build --release
```

That is the whole story for the readout: a Rust toolchain, nothing else. Two
crates and `serde_json`, which is there because the sensor source and the
GitHub release feed are both somebody else's JSON; everything that talks to
Windows is `windows-sys`.

The sensor helper needs a .NET 10 SDK, and is a separate program on purpose so
this one does not inherit that. .NET 10 is not a preference either:
LibreHardwareMonitor publishes a .NET Framework 4.7.2 build and a .NET 10 build
and nothing between them, and only the matching pair loads.

To build everything and package an installer:

```
powershell -File scripts\build.ps1
```

It builds both, stages them, runs [Inno Setup](https://jrsoftware.org/isinfo.php)
over the result, and prints the SHA-256 of the installer. It also refuses to
ship a binary carrying the path it was built on.

Two things worth knowing while working on it:

- `barometer.exe --strip` draws in the system tray. Without it you get a console
  readout of the same numbers, which is easier to watch.
- `barometer.exe --marks sheet.bmp` lays out all fourteen weather marks at three
  times life size. They are too small to judge one at a time in a live tray,
  at whatever the weather happens to be doing.

## License

GPL-3.0-only. See [LICENSE.md](LICENSE.md), and [NOTICE.md](NOTICE.md) for what
other people wrote.
