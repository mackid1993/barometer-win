// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Processes, per-processor load, and the system-wide counts, for the CPU and
// memory flyouts. The Mac's CPUMonitor and MemoryMonitor read these through
// libproc and host_processor_info; here one NtQuerySystemInformation call
// answers for every process at once, with its kernel and user time, its
// working set, and its thread and handle counts in the same record.
//
// None of it belongs on the strip's tick. A process list is a hundred
// kilobytes copied out of the kernel and walked, which at 1 Hz is more work
// than the rest of the readout put together, for a card that is only visible
// while a flyout is open. So the strip's own loop samples it on a slower
// cadence of its own, and only while a panel is open to read it - see
// `processes_at` in the app's main loop.
//
// The Win32 calls stay in this file, which is under sys.rs for that reason:
// the sampler hands out plain Rust types and the arithmetic that turns two
// snapshots into shares is a pure function with tests.

use std::collections::HashMap;
use std::mem;
use std::ptr;

use windows_sys::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SystemProcessInformation, SystemProcessorPerformanceInformation,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_INSUFFICIENT_BUFFER, STATUS_INFO_LENGTH_MISMATCH,
};
use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GetTickCount64, RelationProcessorCore,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use windows_sys::Win32::System::WindowsProgramming::{
    SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION, SYSTEM_PROCESS_INFORMATION,
};

/// What kind of core a logical processor is, from the Mac's CPUCoreKind.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CoreKind {
    Performance,
    Efficiency,
    /// A part with one kind of core, or an OS that does not say.
    #[default]
    Unknown,
}

/// One process as the sampler saw it, with its share of the interval.
#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u32,
    /// The image name without its extension: "chrome", not "chrome.exe",
    /// which is how the Mac names a process and how Task Manager's Details
    /// tab does not.
    pub name: String,
    /// Busy time as a fraction of every processor together, zero to one.
    /// None on the first sample, when there is nothing to diff against.
    pub cpu: Option<f32>,
    /// The working set, in bytes: Task Manager's Memory column.
    pub working_set: u64,
    pub threads: u32,
    pub handles: u32,
}

/// One pass over the machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    /// Every process the kernel listed, the idle process left out.
    pub processes: Vec<Process>,
    /// Each logical processor's busy fraction over the interval, in
    /// processor order. Empty on the first sample.
    pub cores: Vec<f32>,
}

impl Sample {
    /// The busiest processes first, `count` of them, from those that have
    /// a share yet.
    pub fn top_by_cpu(&self, count: usize) -> Vec<&Process> {
        let mut busy: Vec<&Process> = self.processes.iter().filter(|p| p.cpu.is_some()).collect();
        busy.sort_by(|a, b| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)));
        busy.truncate(count);
        busy
    }

    /// The largest working sets first, `count` of them.
    pub fn top_by_memory(&self, count: usize) -> Vec<&Process> {
        let mut large: Vec<&Process> = self.processes.iter().collect();
        large.sort_by(|a, b| b.working_set.cmp(&a.working_set));
        large.truncate(count);
        large
    }
}

/// The machine's totals, from GetPerformanceInfo, in bytes and counts.
///
/// The one call that answers the memory flyout's Commit card and the CPU
/// flyout's System card together: commit charge against its limit, the two
/// kernel pools, and how many processes, threads and handles there are.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemSummary {
    pub processes: u32,
    pub threads: u32,
    pub handles: u32,
    pub committed: u64,
    pub commit_limit: u64,
    pub paged_pool: u64,
    pub nonpaged_pool: u64,
    /// Seconds since boot.
    pub uptime_secs: u64,
}

// ---------------------------------------------------------------------------
// The raw reads
// ---------------------------------------------------------------------------

/// One record of SystemProcessInformation, as far as the sampler cares.
#[derive(Clone, Debug, PartialEq)]
pub struct RawProcess {
    pub pid: u32,
    pub name: String,
    /// Kernel plus user time since the process started, in 100ns ticks.
    pub cpu_ticks: u64,
    pub working_set: u64,
    pub threads: u32,
    pub handles: u32,
}

/// One processor's times since boot, in 100ns ticks. As with
/// GetSystemTimes, `kernel` includes `idle`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessorTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

impl ProcessorTimes {
    fn total(self) -> u64 {
        self.kernel.saturating_add(self.user)
    }
}

/// A buffer NtQuerySystemInformation is happy with: grown until the call
/// stops asking for more.
///
/// The kernel reports the size it wanted, but processes start between the
/// two calls, so the size is taken with slack and the loop is bounded rather
/// than trusted to converge.
fn query(class: i32) -> Option<Vec<u8>> {
    let mut buffer: Vec<u8> = Vec::with_capacity(256 * 1024);
    for _ in 0..8 {
        let mut needed = 0u32;
        // SAFETY: the buffer's capacity is what the call is told, and the
        // out parameter is a local.
        let status = unsafe {
            NtQuerySystemInformation(
                class,
                buffer.as_mut_ptr() as *mut _,
                buffer.capacity() as u32,
                &mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            let wanted = (needed as usize).max(buffer.capacity()) + 64 * 1024;
            buffer.reserve(wanted - buffer.len());
            continue;
        }
        if status < 0 {
            return None;
        }
        // SAFETY: the call wrote `needed` bytes of plain data.
        unsafe { buffer.set_len(needed as usize) };
        return Some(buffer);
    }
    None
}

/// The Win32 image name is "chrome.exe"; the row wants "chrome".
fn strip_extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && extension.eq_ignore_ascii_case("exe") => stem.to_string(),
        _ => name.to_string(),
    }
}

/// Every process the kernel lists, the idle process left out.
pub fn raw_processes() -> Option<Vec<RawProcess>> {
    let buffer = query(SystemProcessInformation)?;
    let mut processes = Vec::new();
    let mut offset = 0usize;
    let record = mem::size_of::<SYSTEM_PROCESS_INFORMATION>();
    while offset + record <= buffer.len() {
        // SAFETY: the kernel wrote a chain of records into the buffer, each
        // at least the documented size, and the bounds are checked above;
        // read unaligned because the entries are only as aligned as the
        // buffer's start.
        let entry: SYSTEM_PROCESS_INFORMATION =
            unsafe { ptr::read_unaligned(buffer.as_ptr().add(offset) as *const _) };
        // winternl.h publishes these seven fields as `BYTE Reserved1[48]`;
        // the layout underneath (ntexapi.h, phnt) is WorkingSetPrivateSize,
        // HardFaultCount, NumberOfThreadsHighWatermark, CycleTime,
        // CreateTime, UserTime, KernelTime - so the two times are the last
        // sixteen bytes of the reserved block. They have been at those
        // offsets since Windows XP, which is why Process Explorer and every
        // other monitor read them the same way.
        let user = i64::from_le_bytes(entry.Reserved1[32..40].try_into().unwrap_or([0; 8]));
        let kernel = i64::from_le_bytes(entry.Reserved1[40..48].try_into().unwrap_or([0; 8]));
        let pid = entry.UniqueProcessId as usize as u32;
        let name = if entry.ImageName.Buffer.is_null() || entry.ImageName.Length == 0 {
            String::new()
        } else {
            // SAFETY: the name's buffer points inside the same allocation,
            // and Length is in bytes.
            let units = unsafe {
                std::slice::from_raw_parts(entry.ImageName.Buffer, entry.ImageName.Length as usize / 2)
            };
            String::from_utf16_lossy(units)
        };
        // The idle process is pid 0 with no name. It is not a process
        // anybody means when they ask what is using the machine.
        if pid != 0 {
            processes.push(RawProcess {
                pid,
                name: strip_extension(&name),
                cpu_ticks: (user.max(0) as u64).saturating_add(kernel.max(0) as u64),
                working_set: entry.WorkingSetSize as u64,
                threads: entry.NumberOfThreads,
                handles: entry.HandleCount,
            });
        }
        if entry.NextEntryOffset == 0 {
            break;
        }
        offset += entry.NextEntryOffset as usize;
    }
    Some(processes)
}

/// Each logical processor's times since boot, in processor order.
///
/// Not through `query`: this class refuses a length that is not a whole
/// number of records, so the buffer is exactly one processor group of them,
/// which is the most the call answers for anyway, and the kernel says how
/// many it filled.
pub fn processor_times() -> Option<Vec<ProcessorTimes>> {
    const GROUP: usize = 64;
    let mut records = [SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION {
        IdleTime: 0,
        KernelTime: 0,
        UserTime: 0,
        Reserved1: [0; 2],
        Reserved2: 0,
    }; GROUP];
    let record = mem::size_of::<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>();
    let mut written = 0u32;
    // SAFETY: the array is the length the call is told, and the out
    // parameter is a local.
    let status = unsafe {
        NtQuerySystemInformation(
            SystemProcessorPerformanceInformation,
            records.as_mut_ptr() as *mut _,
            (record * GROUP) as u32,
            &mut written,
        )
    };
    if status < 0 {
        return None;
    }
    let count = (written as usize / record).min(GROUP);
    Some(
        records[..count]
            .iter()
            .map(|entry| ProcessorTimes {
                idle: entry.IdleTime.max(0) as u64,
                kernel: entry.KernelTime.max(0) as u64,
                user: entry.UserTime.max(0) as u64,
            })
            .collect(),
    )
}

/// What kind of core each logical processor is, in processor order.
///
/// From GetLogicalProcessorInformationEx's EfficiencyClass, which is zero on
/// a part with one kind of core and counts up with the performance of the
/// core on a hybrid part. The classes are told apart only when there are
/// two of them: the Swift labels anything not "performance" as E, which on
/// a plain eight-core part would call every core an efficiency core.
pub fn core_kinds() -> Vec<CoreKind> {
    let mut needed = 0u32;
    // SAFETY: a null buffer asks for the size; the out parameter is a local.
    let ok = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, ptr::null_mut(), &mut needed) };
    // SAFETY: reading the thread's last error has no side effects.
    if ok != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u8; needed as usize];
    // SAFETY: the buffer is the size the first call asked for.
    let ok = unsafe {
        GetLogicalProcessorInformationEx(RelationProcessorCore, buffer.as_mut_ptr() as *mut _, &mut needed)
    };
    if ok == 0 {
        return Vec::new();
    }
    buffer.truncate(needed as usize);
    classify(&core_classes(&buffer))
}

/// Each logical processor's efficiency class, by index, from the records
/// GetLogicalProcessorInformationEx wrote.
///
/// The records are variable length and each says how long it is. A core's
/// record is shorter than the struct that declares the union of every kind
/// of record, so the record is copied into a zeroed struct rather than read
/// as one: reading a whole struct at the last record would run off the end
/// of the buffer, and skipping any record that does not fit a whole struct
/// loses the last core - which on a part with two low-power cores at the
/// end of the list is one of them, left "unknown" among its classified
/// siblings.
fn core_classes(buffer: &[u8]) -> Vec<Option<u8>> {
    let mut classes: Vec<Option<u8>> = Vec::new();
    let mut offset = 0usize;
    let whole = mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    // Relationship and Size, the fields every record starts with.
    const HEADER: usize = 8;
    while offset + HEADER <= buffer.len() {
        let size = u32::from_ne_bytes([buffer[offset + 4], buffer[offset + 5], buffer[offset + 6], buffer[offset + 7]])
            as usize;
        if size < HEADER || offset + size > buffer.len() {
            break;
        }
        // SAFETY: the struct is plain data, zeroed, and only as many bytes
        // as the record has (and the struct holds) are copied into it.
        let entry: SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX = unsafe {
            let mut entry: SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX = mem::zeroed();
            ptr::copy_nonoverlapping(
                buffer.as_ptr().add(offset),
                &mut entry as *mut _ as *mut u8,
                size.min(whole),
            );
            entry
        };
        if entry.Relationship == RelationProcessorCore {
            // SAFETY: the relationship says which arm of the union is live.
            let core = unsafe { entry.Anonymous.Processor };
            let affinity = core.GroupMask[0];
            // Only the first processor group is indexed here: a machine with
            // more than sixty-four logical processors has the rest in other
            // groups, and their kind is left unknown rather than misfiled.
            if affinity.Group == 0 {
                for bit in 0..(usize::BITS as usize) {
                    if affinity.Mask & (1usize << bit) != 0 {
                        if classes.len() <= bit {
                            classes.resize(bit + 1, None);
                        }
                        classes[bit] = Some(core.EfficiencyClass);
                    }
                }
            }
        }
        offset += size;
    }
    classes
}

/// Performance and efficiency by class, or unknown throughout when there is
/// only one class to be had.
pub fn classify(classes: &[Option<u8>]) -> Vec<CoreKind> {
    let known: Vec<u8> = classes.iter().flatten().copied().collect();
    let highest = known.iter().copied().max();
    let lowest = known.iter().copied().min();
    let hybrid = matches!((highest, lowest), (Some(high), Some(low)) if high != low);
    classes
        .iter()
        .map(|class| match class {
            Some(class) if hybrid && Some(*class) == highest => CoreKind::Performance,
            Some(_) if hybrid => CoreKind::Efficiency,
            _ => CoreKind::Unknown,
        })
        .collect()
}

/// The kernel's page lists, in bytes: what Resource Monitor calls Modified,
/// Standby and Free.
///
/// The module's "used" is installed memory less what is available, and
/// available is standby plus free; so modified pages are inside "used" and
/// the bar takes them back out to draw them in their own color.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryLists {
    pub modified: u64,
    pub standby: u64,
    pub free: u64,
}

/// SystemMemoryListInformation, which windows-sys does not declare. The
/// layout is winternl's SYSTEM_MEMORY_LIST_INFORMATION: counts of pages,
/// with the standby list split by its eight priorities.
const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;

#[repr(C)]
#[derive(Copy, Clone)]
struct SystemMemoryListInformation {
    zero_page_count: usize,
    free_page_count: usize,
    modified_page_count: usize,
    modified_no_write_page_count: usize,
    bad_page_count: usize,
    page_count_by_priority: [usize; 8],
    repurposed_pages_by_priority: [usize; 8],
    modified_page_count_page_file: usize,
}

/// The page lists right now, or None where the kernel will not say: the
/// class is readable by a standard user on the Windows this app runs on, but
/// a policy can close it, and the panel then draws in use against available
/// instead.
pub fn memory_lists() -> Option<MemoryLists> {
    // SAFETY: a plain-data struct the call fills; the length passed is its
    // size and the out parameter is a local.
    let mut info: SystemMemoryListInformation = unsafe { mem::zeroed() };
    let mut written = 0u32;
    let status = unsafe {
        NtQuerySystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            &mut info as *mut _ as *mut _,
            mem::size_of::<SystemMemoryListInformation>() as u32,
            &mut written,
        )
    };
    if status < 0 {
        return None;
    }
    let page = page_size()?;
    let bytes = |pages: usize| (pages as u64).saturating_mul(page);
    Some(MemoryLists {
        modified: bytes(info.modified_page_count.saturating_add(info.modified_no_write_page_count)),
        standby: bytes(info.page_count_by_priority.iter().fold(0usize, |sum, n| sum.saturating_add(*n))),
        free: bytes(info.zero_page_count.saturating_add(info.free_page_count)),
    })
}

/// The page files together: bytes in use and bytes altogether, from
/// SystemPageFileInformation, which windows-sys does not declare. A machine
/// with no page file answers with nothing, which is (0, 0): there is no swap
/// and none of it is used.
///
/// This is the Mac's "swap used". Windows' page file is not quite swap - it
/// backs the commit charge rather than holding evicted pages alone - but it
/// is the figure a person asking about swap on Windows means.
pub fn page_file() -> Option<(u64, u64)> {
    const SYSTEM_PAGEFILE_INFORMATION: i32 = 18;
    /// The fixed head of each record; the file's name follows it and is not
    /// wanted here. Counts are in pages.
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct PageFileRecord {
        next_entry_offset: u32,
        total_size: u32,
        total_in_use: u32,
        peak_usage: u32,
    }
    let buffer = query(SYSTEM_PAGEFILE_INFORMATION)?;
    let page = page_size()?;
    let (mut used, mut total) = (0u64, 0u64);
    let mut offset = 0usize;
    while offset + mem::size_of::<PageFileRecord>() <= buffer.len() {
        // SAFETY: the record's head lies within the bytes the call wrote.
        let record: PageFileRecord = unsafe { ptr::read_unaligned(buffer.as_ptr().add(offset) as *const _) };
        used = used.saturating_add((record.total_in_use as u64).saturating_mul(page));
        total = total.saturating_add((record.total_size as u64).saturating_mul(page));
        if record.next_entry_offset == 0 {
            break;
        }
        offset += record.next_entry_offset as usize;
    }
    Some((used, total))
}

/// The page size the counts above are in.
fn page_size() -> Option<u64> {
    // SAFETY: plain data; cb is set before the call.
    let mut info: PERFORMANCE_INFORMATION = unsafe { mem::zeroed() };
    info.cb = mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    // SAFETY: the struct outlives the call.
    if unsafe { GetPerformanceInfo(&mut info, info.cb) } == 0 {
        return None;
    }
    Some(info.PageSize as u64)
}

/// The machine's totals right now.
pub fn summary() -> Option<SystemSummary> {
    // SAFETY: plain data; cb is set before the call, which is all the API
    // asks.
    let mut info: PERFORMANCE_INFORMATION = unsafe { mem::zeroed() };
    info.cb = mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    // SAFETY: the struct outlives the call.
    if unsafe { GetPerformanceInfo(&mut info, info.cb) } == 0 {
        return None;
    }
    let page = info.PageSize as u64;
    let bytes = |pages: usize| (pages as u64).saturating_mul(page);
    // SAFETY: a plain read of the tick counter.
    let uptime_ms = unsafe { GetTickCount64() };
    Some(SystemSummary {
        processes: info.ProcessCount,
        threads: info.ThreadCount,
        handles: info.HandleCount,
        committed: bytes(info.CommitTotal),
        commit_limit: bytes(info.CommitLimit),
        paged_pool: bytes(info.KernelPaged),
        nonpaged_pool: bytes(info.KernelNonpaged),
        uptime_secs: uptime_ms / 1000,
    })
}

/// Ends a process, as the Mac's row sends SIGTERM. Err carries the Win32
/// error, which is ERROR_ACCESS_DENIED for a process that is not ours to end.
pub fn terminate(pid: u32) -> Result<(), u32> {
    // SAFETY: the handle is checked and closed below.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return Err(GetLastError());
        }
        let ok = TerminateProcess(handle, 1);
        let error = if ok == 0 { GetLastError() } else { 0 };
        CloseHandle(handle);
        if ok == 0 {
            Err(error)
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// The sampler
// ---------------------------------------------------------------------------

/// Turns successive raw reads into shares of the interval between them.
#[derive(Debug, Default)]
pub struct Sampler {
    /// Each process's ticks at the last sample, by pid.
    previous: HashMap<u32, u64>,
    /// Every processor's times at the last sample, which is also where the
    /// interval's total capacity comes from.
    previous_cores: Vec<ProcessorTimes>,
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler::default()
    }

    /// Reads the machine and diffs it against the last read.
    pub fn sample(&mut self) -> Option<Sample> {
        let processes = raw_processes()?;
        let cores = processor_times().unwrap_or_default();
        Some(self.observe(processes, cores))
    }

    /// The arithmetic, apart from the reads so that a test can hand in two
    /// snapshots and check the shares.
    ///
    /// The capacity of the interval is the sum of every processor's own
    /// elapsed time, which is wall time times the processor count without
    /// having to ask for either; a process's share of the machine is its
    /// ticks over that. A pid seen for the first time has no share yet
    /// rather than a share of zero, and a pid whose count went backwards was
    /// recycled by a new process and starts over.
    pub fn observe(&mut self, processes: Vec<RawProcess>, cores: Vec<ProcessorTimes>) -> Sample {
        let capacity: u64 = if self.previous_cores.len() == cores.len() {
            cores
                .iter()
                .zip(&self.previous_cores)
                .map(|(now, before)| now.total().saturating_sub(before.total()))
                .sum()
        } else {
            0
        };
        let core_loads: Vec<f32> = if capacity > 0 {
            cores
                .iter()
                .zip(&self.previous_cores)
                .map(|(now, before)| {
                    let elapsed = now.total().saturating_sub(before.total());
                    let idle = now.idle.saturating_sub(before.idle);
                    if elapsed == 0 {
                        0.0
                    } else {
                        (1.0 - idle as f64 / elapsed as f64).clamp(0.0, 1.0) as f32
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut next = HashMap::with_capacity(processes.len());
        let listed: Vec<Process> = processes
            .into_iter()
            .map(|raw| {
                let cpu = match self.previous.get(&raw.pid) {
                    Some(before) if capacity > 0 && raw.cpu_ticks >= *before => {
                        Some(((raw.cpu_ticks - before) as f64 / capacity as f64).clamp(0.0, 1.0) as f32)
                    }
                    _ => None,
                };
                next.insert(raw.pid, raw.cpu_ticks);
                Process {
                    pid: raw.pid,
                    name: raw.name,
                    cpu,
                    working_set: raw.working_set,
                    threads: raw.threads,
                    handles: raw.handles,
                }
            })
            .collect();
        self.previous = next;
        self.previous_cores = cores;
        Sample { processes: listed, cores: core_loads }
    }
}

/// The full path of a process's executable, for its icon.
///
/// Limited query rights are enough for the path and are granted for nearly
/// every process, including elevated and protected ones; a process that
/// refuses even that has no icon worth chasing.
pub fn image_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: a handle opened here and closed on every path; the buffer's
    // length is handed to the call and read back as what was written.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) != 0;
        CloseHandle(process);
        if !ok || length == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(pid: u32, name: &str, ticks: u64, working_set: u64) -> RawProcess {
        RawProcess { pid, name: name.into(), cpu_ticks: ticks, working_set, threads: 4, handles: 40 }
    }

    fn cores(idle: u64, kernel: u64, user: u64) -> Vec<ProcessorTimes> {
        vec![ProcessorTimes { idle, kernel, user }; 4]
    }

    #[test]
    fn the_page_file_answers_with_a_use_that_fits_its_size() {
        let (used, total) = page_file().expect("the page file query answers, even with no page file");
        assert!(used <= total, "{used} of {total}");
    }

    #[test]
    fn the_page_lists_are_read_and_fit_inside_the_installed_memory() {
        let lists = memory_lists().expect("a standard user can read the page lists");
        let installed = crate::sys::memory().expect("installed memory").total;
        assert!(lists.standby.saturating_add(lists.free).saturating_add(lists.modified) <= installed);
        // A running machine always has something on the standby list.
        assert!(lists.standby > 0);
    }

    #[test]
    fn the_first_sample_has_no_shares_and_the_second_has_them_against_the_whole_machine() {
        let mut sampler = Sampler::new();
        let first = sampler.observe(vec![raw(10, "a", 1_000, 1), raw(20, "b", 500, 2)], cores(1_000, 2_000, 1_000));
        assert!(first.processes.iter().all(|p| p.cpu.is_none()));
        assert!(first.cores.is_empty());
        // Four processors each advance 1000 ticks: a capacity of 4000. Process
        // a used 1000 of them, a quarter of the machine; b used none.
        let second =
            sampler.observe(vec![raw(10, "a", 2_000, 1), raw(20, "b", 500, 2)], cores(1_500, 2_500, 1_500));
        let a = second.processes.iter().find(|p| p.pid == 10).unwrap();
        let b = second.processes.iter().find(|p| p.pid == 20).unwrap();
        assert!((a.cpu.unwrap() - 0.25).abs() < 1e-6);
        assert_eq!(b.cpu, Some(0.0));
        // Each processor idled 500 of its 1000: half busy.
        assert_eq!(second.cores.len(), 4);
        assert!(second.cores.iter().all(|load| (load - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_recycled_pid_and_a_newcomer_start_over_rather_than_showing_a_wild_share() {
        let mut sampler = Sampler::new();
        sampler.observe(vec![raw(10, "a", 9_000, 1)], cores(0, 1_000, 0));
        let next = sampler.observe(vec![raw(10, "fresh", 100, 1), raw(30, "new", 50, 1)], cores(0, 2_000, 0));
        assert!(next.processes.iter().all(|p| p.cpu.is_none()));
        // And a processor count that changed between samples is no interval
        // at all.
        let mut sampler = Sampler::new();
        sampler.observe(vec![raw(10, "a", 0, 1)], cores(0, 1_000, 0));
        let next = sampler.observe(vec![raw(10, "a", 500, 1)], vec![ProcessorTimes::default(); 2]);
        assert_eq!(next.processes[0].cpu, None);
        assert!(next.cores.is_empty());
    }

    #[test]
    fn the_top_lists_order_by_share_and_by_working_set_and_stop_at_the_count() {
        let sample = Sample {
            processes: vec![
                Process { pid: 1, name: "a".into(), cpu: Some(0.1), working_set: 300, threads: 1, handles: 1 },
                Process { pid: 2, name: "b".into(), cpu: None, working_set: 900, threads: 1, handles: 1 },
                Process { pid: 3, name: "c".into(), cpu: Some(0.4), working_set: 100, threads: 1, handles: 1 },
            ],
            cores: Vec::new(),
        };
        let cpu: Vec<u32> = sample.top_by_cpu(5).iter().map(|p| p.pid).collect();
        assert_eq!(cpu, [3, 1], "a process with no share yet is not listed");
        let memory: Vec<u32> = sample.top_by_memory(2).iter().map(|p| p.pid).collect();
        assert_eq!(memory, [2, 1]);
    }

    #[test]
    fn image_names_lose_their_exe_and_nothing_else() {
        assert_eq!(strip_extension("chrome.exe"), "chrome");
        assert_eq!(strip_extension("Code.EXE"), "Code");
        assert_eq!(strip_extension("svchost"), "svchost");
        assert_eq!(strip_extension("archive.tar.gz"), "archive.tar.gz");
        assert_eq!(strip_extension(".exe"), ".exe");
    }

    /// One RelationProcessorCore record, the length the kernel writes: the
    /// header, then PROCESSOR_RELATIONSHIP with its one group mask.
    fn core_record(class: u8, mask: u64) -> Vec<u8> {
        let mut record = Vec::new();
        record.extend_from_slice(&(RelationProcessorCore as u32).to_ne_bytes());
        record.extend_from_slice(&48u32.to_ne_bytes());
        record.push(0); // Flags
        record.push(class);
        record.extend_from_slice(&[0u8; 20]); // Reserved
        record.extend_from_slice(&1u16.to_ne_bytes()); // GroupCount
        record.extend_from_slice(&mask.to_ne_bytes()); // GROUP_AFFINITY.Mask
        record.extend_from_slice(&0u16.to_ne_bytes()); // Group
        record.extend_from_slice(&[0u8; 6]); // Reserved
        assert_eq!(record.len(), 48);
        record
    }

    #[test]
    fn the_last_core_record_is_read_even_though_it_is_shorter_than_the_union() {
        // Two performance cores with two threads each, then two efficiency
        // cores, the second of which is the last record in the buffer.
        let mut buffer = Vec::new();
        buffer.extend(core_record(1, 0b0000_0011));
        buffer.extend(core_record(1, 0b0000_1100));
        buffer.extend(core_record(0, 0b0001_0000));
        buffer.extend(core_record(0, 0b0010_0000));
        assert!(buffer.len() < mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>() * 4);
        let classes = core_classes(&buffer);
        assert_eq!(classes, vec![Some(1), Some(1), Some(1), Some(1), Some(0), Some(0)]);
        assert_eq!(classify(&classes)[5], CoreKind::Efficiency);
        // A record that claims to run past the buffer ends the walk.
        let mut torn = buffer.clone();
        torn.truncate(buffer.len() - 4);
        assert_eq!(core_classes(&torn).len(), 5);
    }

    #[test]
    fn cores_are_told_apart_only_on_a_part_with_two_classes() {
        assert_eq!(classify(&[Some(0), Some(0), Some(0)]), vec![CoreKind::Unknown; 3]);
        assert_eq!(
            classify(&[Some(1), Some(1), Some(0), None]),
            [CoreKind::Performance, CoreKind::Performance, CoreKind::Efficiency, CoreKind::Unknown]
        );
        assert!(classify(&[]).is_empty());
    }

    #[test]
    fn the_machine_answers_with_its_own_processes_and_processors() {
        // A smoke test against the real kernel: this process is in the list,
        // named without its extension, and there is a processor.
        let processes = raw_processes().expect("the process list");
        let me = std::process::id();
        assert!(processes.iter().any(|p| p.pid == me));
        assert!(processes.iter().all(|p| p.pid != 0));
        assert!(!processes.iter().any(|p| p.name.to_ascii_lowercase().ends_with(".exe")));
        assert!(!processor_times().expect("processor times").is_empty());
        let summary = summary().expect("the summary");
        assert!(summary.processes > 0 && summary.commit_limit > 0);
        let kinds = core_kinds();
        assert!(kinds.is_empty() || kinds.len() <= 64);
    }
}
