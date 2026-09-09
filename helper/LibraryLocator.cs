// SPDX-License-Identifier: MPL-2.0
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Finding LibreHardwareMonitor, and loading it from wherever the user put it.
//
// Barometer does not redistribute LibreHardwareMonitor. The user installs it
// and we read it: this helper is compiled against the library but ships
// without a byte of it, and resolves it at run time from an installation on
// the machine. That keeps a copyleft library out of our binary, and it means
// the user runs the build they chose and can update it without waiting for us.
//
// The library being absent is therefore an ordinary state, not a failure. The
// helper says so on stdout, the parent turns that into a prompt telling the
// user what to install and where to point us, and the rest of Barometer
// carries on without temperatures.

using System.Runtime.InteropServices;
using System.Runtime.Loader;
using Microsoft.Win32;

internal static class LibraryLocator
{
    /// The assembly everything hinges on. Its dependencies are resolved from
    /// the same directory, which is why the resolver is not written to look
    /// for this one file alone.
    private const string Required = "LibreHardwareMonitorLib.dll";

    /// Where to look, in the order somebody would want us to.
    ///
    /// An explicit path first, because a user who has told us where it is has
    /// answered the question and should not be second-guessed by a search that
    /// might find a different copy. Then the registry, which is the truth for
    /// an installed build. Then the handful of places a portable copy actually
    /// ends up, which is a convenience and never a substitute for asking.
    public static string? Find(string[] args, List<string> searched)
    {
        for (int i = 0; i < args.Length - 1; i++)
        {
            if (args[i] == "--library")
            {
                return Accept(args[i + 1], searched);
            }
        }

        string? fromEnvironment = Environment.GetEnvironmentVariable("BAROMETER_LHM_DIR");
        if (!string.IsNullOrWhiteSpace(fromEnvironment))
        {
            string? found = Accept(fromEnvironment, searched);
            if (found is not null) return found;
        }

        foreach (string candidate in FromRegistry())
        {
            string? found = Accept(candidate, searched);
            if (found is not null) return found;
        }

        foreach (string candidate in CommonPlaces())
        {
            string? found = Accept(candidate, searched);
            if (found is not null) return found;
        }

        return null;
    }

    /// Records a directory as looked-at, and returns it only if the library is
    /// really there. Recording every candidate is what lets the settings pane
    /// answer "where did it look?" instead of shrugging.
    private static string? Accept(string? directory, List<string> searched)
    {
        if (string.IsNullOrWhiteSpace(directory)) return null;
        string full;
        try { full = Path.GetFullPath(directory); }
        catch { return null; }

        if (!searched.Contains(full)) searched.Add(full);
        return File.Exists(Path.Combine(full, Required)) ? full : null;
    }

    /// Install locations recorded by an installer, per-machine and per-user.
    private static IEnumerable<string> FromRegistry()
    {
        (RegistryKey Root, string Path)[] places =
        {
            (Registry.LocalMachine, @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
            (Registry.LocalMachine, @"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
            (Registry.CurrentUser, @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        };

        foreach ((RegistryKey root, string path) in places)
        {
            RegistryKey? uninstall = null;
            try { uninstall = root.OpenSubKey(path); } catch { }
            if (uninstall is null) continue;

            string[] names;
            try { names = uninstall.GetSubKeyNames(); }
            catch { uninstall.Dispose(); continue; }

            foreach (string name in names)
            {
                string? location = null;
                try
                {
                    using RegistryKey? entry = uninstall.OpenSubKey(name);
                    if (entry?.GetValue("DisplayName") is string display &&
                        display.Replace(" ", string.Empty).Contains(
                            "librehardwaremonitor", StringComparison.OrdinalIgnoreCase))
                    {
                        // Matched with the spaces removed on purpose: the name
                        // has been written both as one word and as three at
                        // different times, and will be something else again.
                        location = entry.GetValue("InstallLocation") as string;
                    }
                }
                catch { }

                if (!string.IsNullOrWhiteSpace(location)) yield return location!;
            }

            uninstall.Dispose();
        }
    }

    /// Where a portable copy tends to end up.
    ///
    /// LibreHardwareMonitor is usually a zip somebody extracts rather than an
    /// installer, so for most people the registry knows nothing and this is
    /// the list that finds it.
    private static IEnumerable<string> CommonPlaces()
    {
        string?[] roots =
        {
            Environment.GetEnvironmentVariable("ProgramFiles"),
            Environment.GetEnvironmentVariable("ProgramFiles(x86)"),
            Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "Programs"),
        };

        foreach (string? root in roots)
        {
            if (!string.IsNullOrWhiteSpace(root))
            {
                yield return Path.Combine(root!, "LibreHardwareMonitor");
            }
        }

        // And beside the helper, which is where somebody who unzipped it into
        // Barometer's own folder would have put it.
        string? beside = Path.GetDirectoryName(Environment.ProcessPath);
        if (!string.IsNullOrWhiteSpace(beside))
        {
            yield return beside!;
            yield return Path.Combine(beside!, "LibreHardwareMonitor");
        }
    }

    /// Resolves LibreHardwareMonitor and everything it needs from one folder.
    ///
    /// Hooked onto the default load context rather than loaded into a context
    /// of its own: the helper has nothing to isolate the library from, and a
    /// separate context would only make its dependencies harder to satisfy for
    /// no benefit anybody could see.
    public static void ResolveFrom(string directory)
    {
        AssemblyLoadContext.Default.Resolving += (context, name) =>
        {
            string candidate = Path.Combine(directory, name.Name + ".dll");
            return File.Exists(candidate) ? context.LoadFromAssemblyPath(candidate) : null;
        };

        // Native libraries too: LibreHardwareMonitor carries a few, and they
        // sit beside the managed assemblies in the same installation.
        AssemblyLoadContext.Default.ResolvingUnmanagedDll += (assembly, name) =>
        {
            string candidate = Path.Combine(directory, name);
            if (!Path.HasExtension(candidate)) candidate += ".dll";
            return File.Exists(candidate) ? NativeLibrary.Load(candidate) : IntPtr.Zero;
        };
    }
}
