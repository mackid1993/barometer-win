<div align="center">

<img src="assets/AppIcon.png" width="144" alt="Barometer">

# Barometer for Windows

**Weather and system statistics, on the taskbar.**

![Platform](https://img.shields.io/badge/windows%2011-4fd8f5?style=flat-square)
![License](https://img.shields.io/badge/license-GPL--3.0-4fd8f5?style=flat-square)
![Built with](https://img.shields.io/badge/rust-4fd8f5?style=flat-square)

</div>

A companion to [Barometer for macOS](https://github.com/mackid1993/Barometer),
which lives in the menu bar. This one lives in the Windows 11 taskbar and shows
the same things: network throughput, processor and memory load, graphics load,
disk throughput, temperatures, and the weather.

The two are separate programs rather than one program built twice - that one is
Swift against AppKit, this one is Rust against Win32 - so they do not have all
the same features and their version numbers do not line up. Each has its own
releases here on GitHub, and each updates itself from its own.

## What it looks like

The readout is transparent. There is no plate and no container: the numbers sit
directly on the taskbar, and the weather is the temperature with its condition
drawn in the space above and below it.

It claims room by registering transparent placeholder icons in the notification
area, which makes Windows repack the task buttons aside. That is the only way
to get space on a Windows 11 taskbar without drawing over somebody's window
button. If another program's icon is dragged into that space the readout hides
itself rather than covering it, and comes back when the icon leaves.

## Requirements

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

**Barometer runs as administrator.** Package and core temperatures live in
model-specific registers, and reading one is a ring-0 operation. Measured on
the machine this was written on, same binary, same driver, elevation the only
difference: unelevated, 144 sensors and every one of the 38 temperatures
unreadable; elevated, 196 sensors and all 43 readable, including 35 CPU cores.
There is no unprivileged path to the number. The installer registers a
scheduled task so sign-in is silent; only a manual launch prompts.

**Temperatures need LibreHardwareMonitor, which Barometer does not ship.**
Windows publishes no API for temperature, fan or voltage sensors.
[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
does the reading; you install it and Barometer loads it from wherever you put
it. Nothing of it is in our binary, and you update it on your own schedule.

**Some sensors additionally need PawnIO.** [PawnIO](https://pawnio.eu) is a
small third-party signed kernel driver. You install it yourself, from its own
site. Barometer contains no kernel driver of its own and never will; see
[NOTICE.md](NOTICE.md).

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

That is the whole story for the readout: a Rust toolchain, nothing else. The
sensor helper needs a .NET 8 SDK and is a separate program on purpose, so this
one does not inherit that.

To build everything and package an installer:

```
powershell -File scripts\build.ps1
```

It builds both, stages them, runs [Inno Setup](https://jrsoftware.org/isinfo.php)
over the result, and prints the SHA-256 of the installer. It also refuses to
ship a binary carrying the path it was built on.

Two things worth knowing while working on it:

- `barometer.exe --strip` draws on the taskbar. Without it you get a console
  readout of the same numbers, which is easier to watch.
- `barometer.exe --marks sheet.bmp` lays out all fourteen weather marks at three
  times life size. They are too small to judge one at a time on a live taskbar,
  at whatever the weather happens to be doing.

## License

GPL-3.0-only. See [LICENSE.md](LICENSE.md), and [NOTICE.md](NOTICE.md) for what
other people wrote.
