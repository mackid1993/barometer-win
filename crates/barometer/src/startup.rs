// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Starting with Windows, through the same logon task the installer registers.
//!
//! Not the `Run` key, which is the obvious choice and the wrong one here:
//! Barometer's manifest asks for administrator - it has to, to reach the
//! sensor driver - and Windows never elevates a `Run` entry. One there would
//! sit in Task Manager's Startup tab looking correct and never start the
//! program.
//!
//! The installer already creates the task, named for the program, with
//! settings that matter and are not obvious: `AllowStartIfOnBatteries` and
//! `DontStopIfGoingOnBatteries`, or a laptop away from the wall does not start
//! it; `ExecutionTimeLimit 0`, or the scheduler stops it after three days;
//! `MultipleInstances IgnoreNew`, so a second sign-in does not race the first.
//! Switching off preserves the disabled task; switching on registers the one
//! canonical definition below with create-or-update. Updating matters because
//! an older task may point at a relocatable pre-release path or carry stale
//! settings. Task Scheduler COM keeps this atomic without `schtasks` quoting.
//!
//! Registering from scratch is shared by installed and development copies. The
//! installer calls this module through `--enable-startup`, so there is one task
//! definition and no second script to drift from it.
//!
//! Task Scheduler's own COM interfaces, not PowerShell. The version before
//! this one ran `Get-ScheduledTask` and `Enable-ScheduledTask` through
//! `powershell.exe`, on the settings window's own thread, three launches per
//! click. Each launch stands up .NET and compiles the ScheduledTasks module's
//! cmdlets: two seconds apiece once warm, measured on the author's machine,
//! and minutes for the first launches after a boot - during which the window
//! could not paint and Windows called it not responding. It also drew the
//! switch from whatever a child process printed, and a checkbox that reads a
//! program's stdout is a checkbox with more ways to be wrong than right. The
//! calls below answer in milliseconds and return the scheduler's own answer.
//!
//! `windows-sys` carries no bindings for these interfaces, so the three
//! vtables the switch uses are laid out here by hand, the way
//! `taskbar_buttons.rs` does for UI Automation. Slot order is `taskschd.h`'s,
//! and everything ahead of a slot this file calls is a plain count of the
//! slots it does not.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;
use std::thread;

use windows_sys::core::{BSTR, GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    CloseHandle, SysAllocString, SysFreeString, HANDLE, VARIANT_BOOL, VARIANT_FALSE,
    VARIANT_TRUE,
};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows_sys::Win32::System::Variant::{VARIANT, VT_BSTR};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};

/// The task's name, which is what Task Scheduler shows and what the installer
/// registers. **Must match `AppName` in `installer\barometer.iss`.**
const TASK: &str = "Barometer";

/// `CLSID_TaskScheduler` and `IID_ITaskService`, from `taskschd.h`.
const CLSID_TASK_SCHEDULER: GUID = GUID::from_u128(0x0f87369f_a4e5_4cfc_bd3e_73e6154572dd);
const IID_ITASK_SERVICE: GUID = GUID::from_u128(0x2faba4c7_4da9_4013_9697_20cc3fd40f85);

/// `TASK_LOGON_INTERACTIVE_TOKEN`: run as the signed-in user, with that
/// sign-in's own token, which is what lets a highest-run-level task start
/// elevated without a prompt.
const LOGON_INTERACTIVE_TOKEN: i32 = 3;
/// `TASK_CREATE_OR_UPDATE`.
const CREATE_OR_UPDATE: i32 = 6;

/// Whether Barometer is set to start at sign-in.
///
/// A task that exists but is disabled is off - which is the state this switch
/// leaves behind, and also what somebody gets by disabling it in Task
/// Scheduler themselves. No task at all is off too, and so is a scheduler
/// that cannot be reached: a switch that cannot find out says no rather than
/// guessing yes.
pub fn is_on() -> bool {
    with_root(|root| root.task().is_some_and(|task| task.enabled())).unwrap_or(false)
}

/// Turns starting at sign-in on or off.
///
/// Returns whether the task now reads the way it was asked to. The caller
/// reads `is_on` back afterwards rather than trusting this, so a machine that
/// refuses - a policy, an account that cannot touch the scheduler, a copy
/// running without administrator - leaves the switch showing what is actually
/// so instead of what was pressed.
pub fn set(on: bool) -> bool {
    set_for(on)
}

fn set_for(on: bool) -> bool {
    // Never register a highest-privilege task for an executable whose directory
    // is writable by the medium-integrity user - a development checkout, a
    // relocatable pre-release copy. Under Program Files is the test; nothing
    // is written to get there, see `lhm_install::is_in_protected_location`.
    if on && !crate::lhm_install::is_in_protected_location() {
        return false;
    }
    // Resolve the actual process token and require it to be the user whose
    // Explorer owns this interactive desktop. Over-the-shoulder UAC runs this
    // process as another administrator; that account must not own this user's
    // startup task.
    let definition = on
        .then(|| interactive_user_sid().and_then(|user| definition(Some(&user))))
        .flatten();
    with_root(move |root| {
        let applied = if on {
            // Create-or-update even when a task already exists. Merely enabling
            // an old task preserves its old executable, account, principal and
            // settings, which can leave an elevated launch pointing at a
            // relocatable, user-writable pre-release path after an upgrade.
            match definition {
                Some((xml, user)) => root.register(&xml, &user),
                None => false,
            }
        } else {
            // Disabled, not deleted: the uninstaller owns removal. A missing
            // task already represents off.
            root.task().is_none_or(|task| task.set_enabled(false))
        };
        if !applied {
            return false;
        }
        // Read back rather than trusting the call: what the scheduler says
        // is the only thing the switch is allowed to show.
        root.task().is_some_and(|task| task.enabled()) == on
    })
    .unwrap_or(false)
}

/// The task definition and the account SID it runs as. Tests may provide a
/// representative identity directly; production derives it from matching
/// process and Explorer tokens.
fn definition(user: Option<&str>) -> Option<(String, String)> {
    let command = std::env::current_exe().ok()?;
    let user = user.map(str::to_string).or_else(interactive_user_sid)?;
    Some((xml(&command, &user), user))
}

/// The SID shared by this process token and the Explorer that owns this
/// desktop. A credential prompt may elevate as a different administrator; in
/// that case there is no prompt-free startup task Windows can safely register
/// for the person at the desktop, so return none.
fn interactive_user_sid() -> Option<String> {
    let current = token_sid(unsafe { GetCurrentProcess() })?;

    let class: Vec<u16> = "Shell_TrayWnd"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: a terminated class name and a null optional title.
    let shell = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
    if shell.is_null() {
        return None;
    }
    let mut process_id = 0;
    // SAFETY: the HWND came from FindWindowW and the output pointer is valid.
    unsafe { GetWindowThreadProcessId(shell, &mut process_id) };
    if process_id == 0 {
        return None;
    }
    // SAFETY: query-only access to the process that owns Explorer's taskbar.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    let shell_sid = token_sid(process);
    // SAFETY: a real handle returned by OpenProcess.
    unsafe { CloseHandle(process) };
    let shell_sid = shell_sid?;
    if current != shell_sid {
        return None;
    }
    sid_text(&current)
}

/// Copies the process token's SID out of the variable-length TOKEN_USER block.
fn token_sid(process: HANDLE) -> Option<Vec<u8>> {
    let mut token = ptr::null_mut();
    // SAFETY: valid process handle (or GetCurrentProcess pseudo-handle) and
    // writable output pointer.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }

    let mut bytes = 0;
    // The first call reports the required size.
    unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes) };
    if bytes == 0 {
        unsafe { CloseHandle(token) };
        return None;
    }
    // u64 storage supplies alignment for TOKEN_USER and its embedded SID.
    let mut storage = vec![0u64; (bytes as usize).div_ceil(8)];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            storage.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    };
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return None;
    }

    // SAFETY: GetTokenInformation filled a suitably aligned TOKEN_USER block.
    let sid = unsafe { (*(storage.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return None;
    }
    let length = unsafe { GetLengthSid(sid) } as usize;
    let base = storage.as_ptr() as usize;
    let address = sid as usize;
    let end = address.checked_add(length)?;
    if length < 8 || address < base || end > base + bytes as usize {
        return None;
    }
    // SAFETY: the SID lies in `storage` and GetLengthSid supplied its length.
    Some(unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), length) }.to_vec())
}

/// Canonical `S-revision-authority-subauthority...` text without another FFI
/// allocation. The SID has already passed IsValidSid.
fn sid_text(sid: &[u8]) -> Option<String> {
    if sid.len() < 8 {
        return None;
    }
    let count = sid[1] as usize;
    if sid.len() < 8 + count * 4 {
        return None;
    }
    let authority = sid[2..8]
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    let mut text = format!("S-{}-{authority}", sid[0]);
    for index in 0..count {
        let at = 8 + index * 4;
        let part = u32::from_le_bytes(sid[at..at + 4].try_into().ok()?);
        text.push_str(&format!("-{part}"));
    }
    Some(text)
}

/// The task as Task Scheduler's XML, with the installer's settings.
///
/// Every element here is the installer's definition too: the installer calls
/// this module rather than carrying a second copy. `ExecutionTimeLimit` of
/// `PT0S` is what the scheduler records for no limit; its default is three
/// days, after which it stops the program.
fn xml(command: &Path, user: &str) -> String {
    let command = escape(&command.display().to_string());
    let user = escape(user);
    format!(
        "<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\
         <RegistrationInfo><Description>Starts Barometer at sign-in.</Description></RegistrationInfo>\
         <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>\
         <Principals><Principal id=\"Author\"><UserId>{user}</UserId>\
         <LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>\
         <Settings>\
         <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\
         <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\
         <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\
         <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\
         <Enabled>true</Enabled>\
         </Settings>\
         <Actions Context=\"Author\"><Exec><Command>{command}</Command></Exec></Actions>\
         </Task>"
    )
}

/// XML's five reserved characters, for a path or an account name that has one.
///
/// The PowerShell version refused a path with a quote in it rather than let
/// it end a string half way through a script. Text in XML has no such edge:
/// escaped, any path registers.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

// ---- the scheduler, through COM ---------------------------------------------

/// Runs `work` against the scheduler's root folder, on a thread of its own.
///
/// A thread rather than the caller's, because the caller is the settings
/// window's thread, which has never initialized COM and should not come to
/// own an apartment for the sake of one click. Spawning one, initializing it
/// and joining it costs well under a millisecond; the scheduler call itself
/// is the only time that shows, and it is tens of milliseconds. `None` means
/// COM or the scheduler could not be reached at all.
fn with_root<T: Send + 'static>(work: impl FnOnce(&Folder) -> T + Send + 'static) -> Option<T> {
    let worker = thread::Builder::new()
        .name("barometer-startup".to_string())
        .spawn(move || {
            // SAFETY: a fresh thread that has not initialized COM. Every
            // object made below is released before CoUninitialize: the
            // folder is dropped when the closure it is lent to returns.
            unsafe {
                if CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) < 0 {
                    return None;
                }
                let result = root().map(|folder| work(&folder));
                CoUninitialize();
                result
            }
        })
        .ok()?;
    worker.join().ok()?
}

/// Connects to the local scheduler and opens its root folder.
///
/// # Safety
/// COM must be initialized on the calling thread.
unsafe fn root() -> Option<Folder> {
    let mut raw = ptr::null_mut();
    if CoCreateInstance(
        &CLSID_TASK_SCHEDULER,
        ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_ITASK_SERVICE,
        &mut raw,
    ) < 0
    {
        return None;
    }
    let service = Com::from_raw(raw)?;
    // Four empty variants: this machine, this user, this domain, no password.
    let connect = vtable::<ServiceVtable>(service.0).connect;
    if connect(service.0, empty(), empty(), empty(), empty()) < 0 {
        return None;
    }
    let path = Bstr::new("\\");
    let mut raw = ptr::null_mut();
    if (vtable::<ServiceVtable>(service.0).get_folder)(service.0, path.0, &mut raw) < 0 {
        return None;
    }
    Com::from_raw(raw).map(Folder)
}

/// The scheduler's root folder, where the installer puts the task.
struct Folder(Com);

impl Folder {
    /// The task, if there is one. A missing task comes back as a file-not-
    /// found failure from the scheduler, which is the `None` here.
    fn task(&self) -> Option<Task> {
        let name = Bstr::new(TASK);
        let mut raw = ptr::null_mut();
        // SAFETY: a live folder; the out-pointer is written only on success,
        // and the reference it carries is owned by the returned `Com`.
        unsafe {
            if (vtable::<FolderVtable>(self.0 .0).get_task)(self.0 .0, name.0, &mut raw) < 0 {
                return None;
            }
            Com::from_raw(raw).map(Task)
        }
    }

    /// Registers the task from its XML, replacing one of the same name, to
    /// run as `user` with that user's interactive token.
    fn register(&self, xml: &str, user: &str) -> bool {
        let name = Bstr::new(TASK);
        let text = Bstr::new(xml);
        let account = Bstr::new(user);
        // SAFETY: a live folder. The variant borrows `account`, which outlives
        // the call, and the scheduler copies what it needs; the task handed
        // back is released at once, since nothing here reads it.
        unsafe {
            let mut who = empty();
            who.Anonymous.Anonymous.vt = VT_BSTR;
            who.Anonymous.Anonymous.Anonymous.bstrVal = account.0;
            let mut raw = ptr::null_mut();
            let registered = (vtable::<FolderVtable>(self.0 .0).register_task)(
                self.0 .0,
                name.0,
                text.0,
                CREATE_OR_UPDATE,
                who,
                empty(),
                LOGON_INTERACTIVE_TOKEN,
                empty(),
                &mut raw,
            ) >= 0;
            drop(Com::from_raw(raw));
            registered
        }
    }
}

/// A registered task.
struct Task(Com);

impl Task {
    fn enabled(&self) -> bool {
        let mut value: VARIANT_BOOL = VARIANT_FALSE;
        // SAFETY: a live task; the value is written only on success.
        let read = unsafe { (vtable::<TaskVtable>(self.0 .0).get_enabled)(self.0 .0, &mut value) };
        read >= 0 && value != VARIANT_FALSE
    }

    fn set_enabled(&self, on: bool) -> bool {
        let value = if on { VARIANT_TRUE } else { VARIANT_FALSE };
        // SAFETY: a live task.
        unsafe { (vtable::<TaskVtable>(self.0 .0).put_enabled)(self.0 .0, value) >= 0 }
    }
}

/// One owned COM reference, released on drop.
struct Com(*mut c_void);

impl Com {
    unsafe fn from_raw(value: *mut c_void) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: every Com owns exactly one reference.
        unsafe { (vtable::<UnknownVtable>(self.0).release)(self.0) };
    }
}

/// A `BSTR` allocated from Rust text and freed on drop.
struct Bstr(BSTR);

impl Bstr {
    fn new(text: &str) -> Bstr {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated UTF-16 string that outlives the call; the
        // allocation is the scheduler's to read and ours to free.
        Bstr(unsafe { SysAllocString(wide.as_ptr()) })
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        // SAFETY: allocated by SysAllocString above; a null from a failed
        // allocation is accepted by SysFreeString.
        unsafe { SysFreeString(self.0) };
    }
}

/// An empty variant, `VT_EMPTY`, which every optional argument here takes.
fn empty() -> VARIANT {
    // SAFETY: all-zero is VT_EMPTY, a valid variant.
    unsafe { std::mem::zeroed() }
}

unsafe fn vtable<'a, T>(object: *mut c_void) -> &'a T {
    &**(object as *const *const T)
}

#[repr(C)]
struct UnknownVtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// `ITaskService`. `IDispatch` sits between `IUnknown` and the first method
/// of every scheduler interface, four slots of it.
#[repr(C)]
struct ServiceVtable {
    base: UnknownVtable,
    dispatch: [usize; 4],
    get_folder: unsafe extern "system" fn(*mut c_void, BSTR, *mut *mut c_void) -> HRESULT,
    get_running_tasks: usize,
    new_task: usize,
    connect: unsafe extern "system" fn(*mut c_void, VARIANT, VARIANT, VARIANT, VARIANT) -> HRESULT,
}

/// `ITaskFolder`.
#[repr(C)]
struct FolderVtable {
    base: UnknownVtable,
    dispatch: [usize; 4],
    /// `get_Name`, `get_Path`, `GetFolder`, `GetFolders`, `CreateFolder`,
    /// `DeleteFolder`.
    before_get_task: [usize; 6],
    get_task: unsafe extern "system" fn(*mut c_void, BSTR, *mut *mut c_void) -> HRESULT,
    get_tasks: usize,
    delete_task: usize,
    /// Path, XML, creation flags, user, password, logon type, security
    /// descriptor, and the task registered.
    register_task: unsafe extern "system" fn(
        *mut c_void,
        BSTR,
        BSTR,
        i32,
        VARIANT,
        VARIANT,
        i32,
        VARIANT,
        *mut *mut c_void,
    ) -> HRESULT,
}

/// `IRegisteredTask`.
#[repr(C)]
struct TaskVtable {
    base: UnknownVtable,
    dispatch: [usize; 4],
    /// `get_Name`, `get_Path`, `get_State`.
    before_get_enabled: [usize; 3],
    get_enabled: unsafe extern "system" fn(*mut c_void, *mut VARIANT_BOOL) -> HRESULT,
    put_enabled: unsafe extern "system" fn(*mut c_void, VARIANT_BOOL) -> HRESULT,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// The installer script, read at compile time so these tests fail on the
    /// build rather than on somebody's machine.
    const INSTALLER: &str = include_str!("../../../installer/barometer.iss");

    #[test]
    fn the_task_is_named_what_the_installer_names_it() {
        // The installer registers this task and this switch enables and
        // disables it. If the two names part company the switch registers a
        // second task, and Barometer starts twice at sign-in.
        let defines_it = INSTALLER.lines().any(|line| {
            let line = line.trim();
            line.starts_with("#define AppName") && line.contains(&format!("\"{TASK}\""))
        });
        assert!(defines_it, "installer\\barometer.iss no longer defines AppName as {TASK}");
        assert!(INSTALLER.contains("Parameters: \"--enable-startup\""));
        assert!(!INSTALLER.contains("Parameters: \"--enable-startup \""));
    }

    #[test]
    fn a_binary_sid_is_written_the_way_task_scheduler_accepts_it() {
        let sid = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0];
        assert_eq!(sid_text(&sid).as_deref(), Some("S-1-5-32-544"));
    }


    #[test]
    fn the_settings_are_the_ones_the_installer_registers() {
        // The installer delegates to this module, so one XML definition owns
        // the settings. Losing the battery allowance means a laptop away from
        // the wall never starts Barometer; losing the execution limit means
        // the scheduler stops it after three days. Both failures are silent.
        assert!(INSTALLER.contains("--enable-startup"));
        let definition = xml(Path::new(r"C:\Program Files\Barometer\barometer.exe"), "David");
        for element in [
            "<LogonTrigger>",
            "<DisallowStartIfOnBatteries>false<",
            "<StopIfGoingOnBatteries>false<",
            "<ExecutionTimeLimit>PT0S<",
            "<MultipleInstancesPolicy>IgnoreNew<",
            "<LogonType>InteractiveToken<",
            "<RunLevel>HighestAvailable<",
        ] {
            assert!(definition.contains(element), "the switch's task no longer carries {element}");
        }
    }

    #[test]
    fn a_path_that_would_break_the_xml_is_escaped_rather_than_refused() {
        let definition = xml(Path::new(r"C:\Tools & Odds\<Barometer>\barometer.exe"), "O'Brien");
        assert!(definition.contains(
            "<Command>C:\\Tools &amp; Odds\\&lt;Barometer&gt;\\barometer.exe</Command>"
        ));
        assert!(definition.contains("<UserId>O&apos;Brien</UserId>"));
    }

    #[test]
    fn reading_the_switch_does_not_keep_the_window_waiting() {
        // The reason this module talks COM. The PowerShell it replaced took
        // two seconds warm and minutes cold, on the settings window's
        // thread; the scheduler answers this in tens of milliseconds, and
        // the bound leaves room for a slow machine without letting a process
        // launch back in.
        let started = Instant::now();
        let _ = is_on();
        let took = started.elapsed();
        assert!(took < Duration::from_secs(1), "is_on took {took:?}");
    }

}
