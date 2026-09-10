; SPDX-License-Identifier: GPL-3.0-only
;
; Barometer - a system monitor for the Windows taskbar
; Copyright (c) 2026 David Brustein

; Barometer installer.
;
; Packages whatever build.ps1 staged into dist\Barometer. It deliberately does
; not know how that directory was assembled.
;
; Administrator, because the program is. Temperatures come from model-specific
; registers and reading one is a ring-0 operation, so Barometer's manifest
; requires elevation - see crates/barometer/barometer.manifest for the
; measurements behind that. An installer that ran unelevated could not create
; the scheduled task that makes sign-in silent, and would install a program the
; user then could not start without a prompt.

#define AppName        "Barometer"
#define AppExeName     "barometer.exe"
; Where build.ps1 staged the files. Passed in rather than assumed: the staging
; directory sits beside the build output, outside the repository, because the
; repository lives in a synced folder that holds directories open.
#ifndef StageDir
  #define StageDir     "..\dist\Barometer"
#endif
; Overridable from the command line: ISCC /DAppVersion=0.0.1
; build.ps1 passes whatever Cargo.toml says, so the installer, its filename and
; the executable's own version resource cannot drift apart.
#ifndef AppVersion
  #define AppVersion   "1.0.0"
#endif
#define AppPublisher   "David Brustein"

[Setup]
; Barometer's identity, and it must never change: this GUID is what Windows
; matches an upgrade against, and a new one turns every future version into a
; second entry in Add or Remove Programs beside the first.
AppId={{B4A70E62-1D5C-4F38-9C21-7E0A5D3B8F14}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
; This app's own repository. mackid1993/Barometer is the macOS program, which
; shares the name and nothing else; sending a Windows user there for support is
; sending them to somebody else's issue tracker.
AppSupportURL=https://github.com/mackid1993/barometer-win
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputBaseFilename={#AppName}-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

; Admin on purpose. The sensor helper reads temperatures through a kernel
; driver, and Barometer runs elevated so that it can start the helper. The
; per-user-area warning Inno prints about the desktop shortcut and the
; logon task is known and accepted.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; Windows 11 and up. Not a preference: the whole strip stands on claiming space
; in the Windows 11 notification area and being composited over the taskbar's
; XAML surface. Windows 10's taskbar is a different thing entirely and
; Barometer would install and then draw nothing.
MinVersion=10.0.22000

LicenseFile={#StageDir}\LICENSE.md
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
SetupIconFile=..\assets\barometer.ico

; The uninstaller has to be able to stop a running Barometer, or it leaves the
; executable locked and its placeholder icons sitting in the tray.
CloseApplications=yes
CloseApplicationsFilter=*.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start {#AppName} when I sign in"; GroupDescription: "Startup:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
; Most hardware sensors are behind instructions and ports no ordinary program
; may use: the processor's temperatures in model-specific registers, and the
; motherboard's fans, voltages and board temperatures behind the SuperIO chip,
; the embedded controller or the SMBus. PawnIO is the third-party signed driver
; that reads all of them, and it is somebody else's software with its own
; installer and license.
;
; unchecked, deliberately, and it is the one task here that would be wrong to
; tick by default. Barometer installs no driver and must not look like it is
; nudging one onto the machine; anything driver-shaped is opt-in or it is not
; offered at all. The usual cost of an unchecked box - most people never see
; the option - is paid for on the other side rather than by ticking it:
; Barometer checks for PawnIO itself at startup and explains, once, when it is
; genuinely missing. So this is a shortcut for people who already know they
; want it, not the only chance they get to hear about it.
;
; The wording says what it is *for*. "Optional download" on its own tells the
; user nothing they can decide with - optional in aid of what? Naming what they
; lose by skipping it is the entire content of the sentence, and "optional"
; belongs on the group heading where it describes the whole section rather than
; standing in for a reason.
;
; What they lose is not only the processor's temperature, which is what this
; line used to say. It is nearly every hardware sensor except the graphics card
; and the drives, both of which have a path of their own.
Name: "pawnio"; Description: "Open the PawnIO download page - needed for processor, fan and voltage sensors"; GroupDescription: "Hardware sensors (optional):"; Flags: unchecked

[Files]
Source: "{#StageDir}\{#AppExeName}";  DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE.md";     DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\NOTICE.md";      DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\barometer.ico";  DestDir: "{app}"; Flags: ignoreversion

; The sensor helper. Self-contained, so nobody needs a .NET runtime installed,
; and genuinely a single file - but only when it is taken from the `publish`
; directory. The build output sitting next to that one is not self-contained,
; and a helper staged from there starts and exits immediately with nothing to
; say why; the only symptom on the Barometer side is an unreadable greeting.
;
; skipifsourcedoesntexist because temperatures are optional: somebody who never
; runs the helper build still gets a working install, with the Sensors module
; reading unavailable and the settings pane explaining what to install.
Source: "{#StageDir}\barometer-sensors.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; The two native libraries a single-file publish cannot bundle, when a publish
; produces them. They are absent while LibreHardwareMonitor is referenced with
; ExcludeAssets="runtime", which is why this is skipifsourcedoesntexist and not
; a hard requirement - but a build that does stage them must package them, or
; the helper starts and exits with nothing to say why. See AGENTS.md.
Source: "{#StageDir}\MonoPosixHelper.dll";    DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#StageDir}\libMonoPosixHelper.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
; IconFilename is given explicitly rather than left to inherit from the
; executable. The exe carries the icon as a Win32 resource, so inheriting would
; work - but a shortcut that names its icon keeps the right one even if the
; resource is ever stripped, and costs nothing.
Name: "{group}\{#AppName}";       Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\barometer.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\barometer.ico"; Tasks: desktopicon

[Run]
; Start with Windows, as a scheduled task rather than a Run key.
;
; A Run entry cannot start an elevated program. Windows either refuses it or
; prompts, and neither is a way to bring up a taskbar readout at sign-in. A
; task registered to run with highest privileges starts elevated with no prompt
; at all, which is the entire reason this installer asks for administrator once
; so that nobody is asked again.
;
; The quoting is exact and was wrong once already. Inno collapses a doubled
; quote to a single one, so ""{app}\{#AppExeName}"" is what puts one pair of
; quotes around the path for schtasks. Six and seven quotes produced
; /TR """C:\...exe""", schtasks refused it, no task was created, and the
; "Start Barometer" tick box then had nothing to run - which is exactly how
; this was found.
;
; /RL HIGHEST is the part that matters. /F overwrites a task left behind by an
; earlier version instead of failing, and runhidden keeps schtasks from
; flashing a console during the install - which is precisely the thing this
; program has already been accused of doing.
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""$a = New-ScheduledTaskAction -Execute '{app}\{#AppExeName}'; $t = New-ScheduledTaskTrigger -AtLogOn -User '{username}'; $s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0 -MultipleInstances IgnoreNew; $p = New-ScheduledTaskPrincipal -UserId '{username}' -LogonType Interactive -RunLevel Highest; Register-ScheduledTask -TaskName '{#AppName}' -Action $a -Trigger $t -Settings $s -Principal $p -Force | Out-Null"""; Flags: runhidden; Tasks: startup; StatusMsg: "Registering the sign-in task..."

; Started through the task rather than directly, when there is a task.
;
; A postinstall entry runs as the user who started Setup, which after a UAC
; prompt is the *unelevated* user - and Barometer requires elevation, so
; launching it that way fails with "CreateProcess failed; code 740. The
; requested operation requires elevation." Running the task we just registered
; starts it exactly the way sign-in will, which also proves the task works
; before anybody reboots.
Filename: "{sys}\schtasks.exe"; Parameters: "/Run /TN ""{#AppName}""";     Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent runhidden;     Tasks: startup

; And directly for somebody who declined the sign-in task. runascurrentuser is
; what keeps this from hitting the same 740: it runs as the elevated account
; Setup itself is running as, rather than dropping back to the original user.
Filename: "{app}\{#AppExeName}"; Description: "Start {#AppName}";     Flags: nowait postinstall skipifsilent runascurrentuser; Tasks: not startup

; Opens PawnIO's own site, and only for somebody who asked for it on the tasks
; page. Nothing is downloaded or run by the installer: the whole action is
; handing a URL to the browser, which is as far as Barometer goes towards a
; kernel driver.
;
; No postinstall flag, so this does not also appear as a second tick box on the
; finished page. The person already answered this question one page earlier and
; asking twice reads as pressure.
;
; runasoriginaluser matters here, and more than it used to: this installer now
; always runs elevated, because it has a scheduled task to register. A browser
; launched from it would inherit that - a whole browsing session running as
; administrator because somebody wanted a temperature reading.
;
; skipifsilent belongs on anything that opens a window: an unattended install
; that pops a browser is a broken unattended install, and /TASKS="pawnio" on a
; silent command line must not defeat that.
Filename: "https://pawnio.eu/"; Flags: shellexec nowait skipifsilent runasoriginaluser; Tasks: pawnio

[UninstallRun]
; Taking the task with us. A logon task pointing at a deleted executable fails
; silently at every sign-in and sits in Task Scheduler forever.
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /F /TN ""{#AppName}""";     Flags: runhidden; RunOnceId: "RemoveStartupTask"

[UninstallDelete]
; The placeholder identities Barometer registers in the notification area live
; under NotifyIconSettings and are keyed by a hash Windows chooses, so they
; cannot be named here. Barometer hands its icons back on exit and deliberately
; never deletes those registry entries - deleting one makes that identity
; permanently unusable, which is a worse thing to leave behind than a few bytes
; of settings. See crates/barometer/src/reserve.rs.
;
; Settings are written under the user's application data rather than into the
; program directory, so uninstalling leaves the machine as it was.
Type: dirifempty; Name: "{app}"

[Code]
// Getting the running copy out of the way before its files are overwritten.
//
// CloseApplications=yes hands this to Restart Manager, which asks a top-level
// window to close and waits. That is the polite path and it now works - the
// strip answers WM_CLOSE by quitting through the same route as the menu item,
// so the tray placeholders are handed back rather than left for the shell to
// notice.
//
// It is not enough on its own, for two reasons that both apply to every update
// from an existing installation.
//
// The polite path depends on the copy that is *running*, and that is the old
// one. Every version installed before the WM_CLOSE handler existed lets
// DefWindowProcW take it, which destroys the window and leaves the process
// alive holding barometer.exe open - so the installer is told the application
// closed when it did not. No future version can fix that retroactively.
//
// And barometer-sensors.exe has no window at all. Restart Manager has nothing
// to ask, so it can only report it or terminate it. Its job object takes it
// down with its parent, but only once the parent has actually gone.
//
// So the task is stopped and the tree is killed first, and Restart Manager is
// left to handle whatever is still standing. Both commands are allowed to fail:
// on a first install there is no task and no process, and neither absence is a
// reason to refuse to install.
procedure StopRunningCopy();
var
  ResultCode: Integer;
begin
  // The task first. Killing the process while its task is still running invites
  // Task Scheduler to start it again underneath the install.
  Exec(ExpandConstant('{sys}\schtasks.exe'),
       '/End /TN "{#AppName}"',
       '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  // /T for the tree, so the sensor helper goes with it rather than waiting on
  // its job object. /F because a GUI-subsystem process with no message pump
  // running cannot be asked nicely, and by this point it has already been asked.
  Exec(ExpandConstant('{sys}\taskkill.exe'),
       '/F /T /IM {#AppExeName}',
       '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'),
       '/F /IM barometer-sensors.exe',
       '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  // The handles close asynchronously. Without this the file replacement can
  // still lose a race with a process that has been told to die and has not
  // finished dying, which reports as "cannot access the file" on a machine
  // where nothing is running any more.
  Sleep(400);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningCopy();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopRunningCopy();
  Result := True;
end;
