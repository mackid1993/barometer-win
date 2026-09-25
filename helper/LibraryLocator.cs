// SPDX-License-Identifier: MPL-2.0
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Finding LibreHardwareMonitor in an administrator-protected location.
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
using System.Security.AccessControl;
using System.Security.Principal;


internal static class LibraryLocator
{
    /// The assembly everything hinges on. Its dependencies are resolved from
    /// the same directory, which is why the resolver is not written to look
    /// for this one file alone.
    private const string Required = "LibreHardwareMonitorLib.dll";
    private const FileSystemRights WriteLike = FileSystemRights.Write |
        FileSystemRights.Delete | FileSystemRights.DeleteSubdirectoriesAndFiles |
        FileSystemRights.ChangePermissions | FileSystemRights.TakeOwnership;

    private static bool GrantsWriteLike(FileSystemRights rights) =>
        (rights & WriteLike) != 0;

    /// Build-time regression probe for the ACL mask. Composite rights such as
    /// FullControl include read bits; putting one in the mask makes Users RX
    /// look writable and rejects every correctly protected installation.
    internal static bool PermissionMaskSelfTest() =>
        !GrantsWriteLike(FileSystemRights.ReadAndExecute | FileSystemRights.Synchronize) &&
        GrantsWriteLike(FileSystemRights.Write) &&
        GrantsWriteLike(FileSystemRights.Modify) &&
        GrantsWriteLike(FileSystemRights.FullControl);

    /// Accepts only Barometer's managed directory beside the helper.
    public static string? Find(string[] args, List<string> searched)
    {
        for (int i = 0; i < args.Length - 1; i++)
        {
            if (args[i] == "--library")
            {
                return AcceptManaged(args[i + 1], searched);
            }
        }
        return null;
    }

    /// Records a directory as looked-at, and returns it only if the library is
    /// really there. Recording every candidate is what lets the settings pane
    /// answer "where did it look?" instead of shrugging.
    private static string? AcceptManaged(string? directory, List<string> searched)
    {
        if (string.IsNullOrWhiteSpace(directory)) return null;
        string full;
        try { full = Path.GetFullPath(directory); }
        catch { return null; }

        if (!searched.Contains(full)) searched.Add(full);
        string? executable = Path.GetDirectoryName(Environment.ProcessPath);
        if (string.IsNullOrWhiteSpace(executable) || !IsUnderProgramFiles(executable)) return null;
        string expected = Path.GetFullPath(Path.Combine(executable, "LibreHardwareMonitor"));
        if (!string.Equals(full, expected, StringComparison.OrdinalIgnoreCase)) return null;
        if (!Directory.Exists(full) || HasReparsePointBetween(full, executable) || !HasSafeAcl(full)) return null;
        return File.Exists(Path.Combine(full, Required)) ? full : null;
    }

    /// The helper runs elevated, so it may execute libraries only from
    /// Program Files. A path in AppData, Downloads or another user-writable
    /// location would turn the next scheduled launch into an elevation path
    /// for any medium-integrity process able to replace a DLL there.
    private static bool IsUnderProgramFiles(string directory)
    {
        foreach (Environment.SpecialFolder folder in new[]
        {
            Environment.SpecialFolder.ProgramFiles,
            Environment.SpecialFolder.ProgramFilesX86,
        })
        {
            string root = Environment.GetFolderPath(folder);
            if (string.IsNullOrWhiteSpace(root)) continue;
            root = Path.GetFullPath(root).TrimEnd(Path.DirectorySeparatorChar,
                                                 Path.AltDirectorySeparatorChar);
            string prefix = root + Path.DirectorySeparatorChar;
            if (directory.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)) return true;
        }
        return false;
    }

    /// Refuses junctions and symbolic links between the managed directory and
    /// the installed helper. A lexical Program Files name must not redirect DLL
    /// loading into a user-writable target.
    private static bool HasReparsePointBetween(string directory, string executable)
    {
        string stop = Path.GetFullPath(executable).TrimEnd(
            Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        DirectoryInfo? current = new(directory);
        while (current is not null)
        {
            try
            {
                if ((current.Attributes & FileAttributes.ReparsePoint) != 0) return true;
            }
            catch { return true; }
            string here = current.FullName.TrimEnd(
                Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            if (string.Equals(here, stop, StringComparison.OrdinalIgnoreCase)) return false;
            current = current.Parent;
        }
        return true;
    }

    /// Requires that no medium-integrity user principal holds a write-like
    /// allow rule on the directory, inherited or not.
    ///
    /// Effective rules and nothing more. The default ACL under Program Files
    /// - Users read and execute, Administrators and SYSTEM full control, all
    /// of it inherited - is exactly the guarantee wanted, and it is what an
    /// elevated installer leaves behind without anybody writing an ACL. An
    /// earlier version demanded a protected, non-inherited ACL with an
    /// Administrators owner to match one Barometer wrote with icacls; that
    /// rewrite is gone, and the demand with it, because a reset of the tree's
    /// ACLs - which the installer itself performed - turned a perfectly safe
    /// inherited ACL into "no library" until the user pressed Reinstall.
    /// CREATOR OWNER is not on the list: Program Files carries an inherit-only
    /// entry for it by default, and only an administrator can create anything
    /// there for it to apply to.
    private static bool HasSafeAcl(string directory)
    {
        try
        {
            DirectorySecurity security = new DirectoryInfo(directory).GetAccessControl(
                AccessControlSections.Access);

            SecurityIdentifier? current = WindowsIdentity.GetCurrent().User;
            HashSet<SecurityIdentifier> untrusted = new()
            {
                new SecurityIdentifier(WellKnownSidType.WorldSid, null),
                new SecurityIdentifier(WellKnownSidType.AuthenticatedUserSid, null),
                new SecurityIdentifier(WellKnownSidType.BuiltinUsersSid, null),
            };
            if (current is not null) untrusted.Add(current);
            foreach (FileSystemAccessRule rule in security.GetAccessRules(
                true, true, typeof(SecurityIdentifier)))
            {
                if (rule.AccessControlType == AccessControlType.Allow &&
                    untrusted.Contains((SecurityIdentifier)rule.IdentityReference) &&
                    GrantsWriteLike(rule.FileSystemRights)) return false;
            }
            return true;
        }
        catch { return false; }
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
