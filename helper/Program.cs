// SPDX-License-Identifier: MPL-2.0
//
// This program wraps LibreHardwareMonitor, which is MPL-2.0, and so is MPL-2.0
// itself rather than GPL-3.0 like the rest of Barometer. MPL section 3.3 is
// what permits the combination. See NOTICE.md.
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The sensor helper.
//
// Windows exposes no temperature, fan or voltage API, and LibreHardwareMonitor
// already reads them from every chip family worth supporting. Running it in a
// separate process rather than inside Barometer is deliberate: it is .NET, and
// hosting a runtime inside a monitor that exists to be small would undo the
// whole point. Ahead-of-time compilation was tried first and does not work -
// see AGENTS.md - so this is the JIT, in its own process, where its runtime
// costs the user nothing while sensors are switched off.
//
// Protocol, one line at a time, so it can be read by anything:
//
//   stdin  "read"  ->  stdout  one JSON object of sensors
//   stdin  "quit"  ->  exits
//   stdin  closed  ->  exits
//
// Diagnostics go to stderr and never to stdout, which carries only protocol.

using System.Text.Json;
using LibreHardwareMonitor.Hardware;

/// Refreshes every device, tolerating the ones that cannot be read.
internal sealed class UpdateVisitor : IVisitor
{
    public void VisitComputer(IComputer computer) => computer.Traverse(this);

    public void VisitHardware(IHardware hardware)
    {
        // Per device. A controller that throws must not cost every other
        // reading on the machine.
        try { hardware.Update(); } catch { }
        foreach (IHardware sub in hardware.SubHardware)
        {
            try { sub.Accept(this); } catch { }
        }
    }

    public void VisitSensor(ISensor sensor) { }
    public void VisitParameter(IParameter parameter) { }
}

internal static class Program
{
    /// Stable codes shared with the Rust side. Ours rather than the library's
    /// enum values, so an upstream renumbering cannot silently relabel every
    /// reading.
    private static int KindCode(SensorType type) => type switch
    {
        SensorType.Voltage => 1,
        SensorType.Current => 2,
        SensorType.Power => 3,
        SensorType.Clock => 4,
        SensorType.Temperature => 5,
        SensorType.Load => 6,
        SensorType.Frequency => 7,
        SensorType.Fan => 8,
        SensorType.Flow => 9,
        SensorType.Control => 10,
        SensorType.Level => 11,
        SensorType.Factor => 12,
        SensorType.Data => 13,
        SensorType.SmallData => 14,
        SensorType.Throughput => 15,
        SensorType.TimeSpan => 16,
        SensorType.Energy => 17,
        SensorType.Noise => 18,
        SensorType.Humidity => 19,
        _ => 0,
    };

    /// Finds LibreHardwareMonitor, wires up the loader, and only then runs.
    ///
    /// The order is the whole point and it is fragile: the JIT resolves the
    /// types a method uses when that method is first compiled, so `Main` must
    /// not mention a single LibreHardwareMonitor type. If it did, resolution
    /// would happen before the resolver below is installed and the process
    /// would die with a FileNotFoundException nobody could act on. That is why
    /// the work lives in Run(), which is called rather than inlined.
    private static int Main(string[] args)
    {
        List<string> searched = new();
        string? directory = LibraryLocator.Find(args, searched);
        if (directory is null)
        {
            // Not an error to be logged and forgotten: the parent turns this
            // into the prompt that tells the user what to install and where to
            // point us.
            Console.Out.WriteLine(NoLibrary(searched));
            return 2;
        }

        LibraryLocator.ResolveFrom(directory);
        try
        {
            return Run();
        }
        catch (FileNotFoundException error)
        {
            // The directory existed but did not hold what we needed, or held a
            // version missing something. Same outcome for the user as not
            // finding it at all, so it is reported the same way.
            searched.Add($"{directory} ({error.FileName ?? "missing assembly"})");
            Console.Out.WriteLine(NoLibrary(searched));
            return 2;
        }
    }

    /// Tells the parent the library is not there, and where we looked.
    ///
    /// The search list is included because the first question anybody asks is
    /// "where did it look?", and answering it in the settings pane is the
    /// difference between a useful prompt and a shrug.
    private static string NoLibrary(List<string> searched)
    {
        using var buffer = new MemoryStream();
        using (var json = new Utf8JsonWriter(buffer))
        {
            json.WriteStartObject();
            json.WriteString("status", "no-library");
            json.WriteStartArray("searched");
            foreach (string place in searched)
            {
                json.WriteStringValue(place);
            }
            json.WriteEndArray();
            json.WriteEndObject();
        }
        return System.Text.Encoding.UTF8.GetString(buffer.ToArray());
    }

    [System.Runtime.CompilerServices.MethodImpl(
        System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    private static int Run()
    {
        Computer computer;
        try
        {
            computer = new Computer
            {
                IsCpuEnabled = true,
                IsGpuEnabled = true,
                IsMotherboardEnabled = true,
                IsMemoryEnabled = true,
                IsStorageEnabled = true,
                IsBatteryEnabled = true,
                // Network throughput and controller state already come from
                // Windows APIs Barometer reads natively. No reason to pay for
                // the same numbers twice, through a heavier path.
                IsNetworkEnabled = false,
                IsControllerEnabled = false,
                IsPsuEnabled = false,
            };
            computer.Open();
        }
        catch (Exception error)
        {
            // Type and inner exception, not just the message. The message is
            // routinely empty here - a TypeInitializationException from deep
            // inside the library says nothing on its own - and an empty line
            // of diagnostics is worse than none, because it looks like the
            // problem was reported when it was not.
            Console.Error.WriteLine($"open failed: {Describe(error)}");
            return 1;
        }

        var visitor = new UpdateVisitor();
        // Line-buffered by hand: the parent blocks on a reply, so anything left
        // sitting in a buffer is a hang rather than a delay.
        using var output = new StreamWriter(Console.OpenStandardOutput()) { AutoFlush = true };

        output.WriteLine(Ready(computer));

        try
        {
            string? line;
            while ((line = Console.In.ReadLine()) is not null)
            {
                switch (line.Trim())
                {
                    case "read":
                        output.WriteLine(Read(computer, visitor));
                        break;
                    case "quit":
                        return Close(computer);
                    default:
                        Console.Error.WriteLine($"unknown command: {line}");
                        break;
                }
            }
        }
        catch (IOException)
        {
            // The parent went away mid-line. Nothing to report to.
        }

        // stdin closed: the parent is gone, so we are done.
        return Close(computer);
    }

    /// Unwraps an exception into something a person could act on.
    private static string Describe(Exception error)
    {
        var text = new System.Text.StringBuilder();
        for (Exception? current = error; current is not null; current = current.InnerException)
        {
            if (text.Length > 0) text.Append(" <- ");
            text.Append(current.GetType().Name);
            if (!string.IsNullOrWhiteSpace(current.Message))
            {
                text.Append(": ").Append(current.Message);
            }
        }
        return text.ToString();
    }

    private static int Close(Computer computer)
    {
        try { computer.Close(); } catch { }
        return 0;
    }

    /// Announces the helper and what it found, so the parent can distinguish
    /// "started but sees no hardware" from "did not start".
    private static string Ready(Computer computer)
    {
        int devices = 0;
        try { devices = computer.Hardware.Count; } catch { }
        using var buffer = new MemoryStream();
        using (var json = new Utf8JsonWriter(buffer))
        {
            json.WriteStartObject();
            json.WriteString("status", "ready");
            json.WriteNumber("devices", devices);
            json.WriteEndObject();
        }
        return System.Text.Encoding.UTF8.GetString(buffer.ToArray());
    }

    private static string Read(Computer computer, UpdateVisitor visitor)
    {
        try { computer.Accept(visitor); } catch { }

        using var buffer = new MemoryStream();
        using (var json = new Utf8JsonWriter(buffer))
        {
            json.WriteStartObject();
            json.WriteStartArray("sensors");
            foreach (IHardware hardware in computer.Hardware)
            {
                try { WriteHardware(json, hardware); } catch { }
            }
            json.WriteEndArray();
            json.WriteEndObject();
        }
        return System.Text.Encoding.UTF8.GetString(buffer.ToArray());
    }

    private static void WriteHardware(Utf8JsonWriter json, IHardware hardware)
    {
        foreach (ISensor sensor in hardware.Sensors)
        {
            json.WriteStartObject();
            json.WriteString("id", sensor.Identifier.ToString());
            json.WriteString("name", sensor.Name);
            json.WriteString("hardware", hardware.Name);
            json.WriteNumber("kind", KindCode(sensor.SensorType));
            // null, not zero: a fan that cannot be read is not a stopped fan.
            float? value = sensor.Value;
            if (value.HasValue && !float.IsNaN(value.Value) && !float.IsInfinity(value.Value))
                json.WriteNumber("value", value.Value);
            else
                json.WriteNull("value");
            json.WriteEndObject();
        }

        foreach (IHardware sub in hardware.SubHardware)
        {
            try { WriteHardware(json, sub); } catch { }
        }
    }
}
