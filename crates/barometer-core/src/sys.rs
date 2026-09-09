// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The Win32 calls, and the only place in the crate that says `unsafe`.
// Everything here returns plain Rust types; nothing above knows what a
// FILETIME is.

pub mod processes;
pub mod storage;

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

/// A FILETIME is a count of 100ns ticks split across two words. Nothing here
/// cares about the epoch, only about differences, so it collapses to one u64.
fn ticks(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// Processor time since boot, in 100ns ticks.
///
/// `kernel` already includes `idle`, which is a Win32 wart worth stating once
/// rather than rediscovering: busy time is `kernel + user - idle`.
#[derive(Copy, Clone, Debug, Default)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

impl CpuTimes {
    /// Wall time multiplied by the number of logical processors.
    pub fn total(self) -> u64 {
        self.kernel.saturating_add(self.user)
    }

    pub fn busy(self) -> u64 {
        self.total().saturating_sub(self.idle)
    }
}

pub fn cpu_times() -> Option<CpuTimes> {
    let mut idle = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let mut kernel = idle;
    let mut user = idle;
    // SAFETY: three out parameters, all owned by this frame for the call.
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    if ok == 0 {
        return None;
    }
    Some(CpuTimes { idle: ticks(idle), kernel: ticks(kernel), user: ticks(user) })
}

/// Physical memory, in bytes.
#[derive(Copy, Clone, Debug, Default)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
}

impl Memory {
    pub fn used(self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// Used as a fraction of total, 0.0 to 1.0. Zero when total is unknown, so
    /// no caller has to guard a division.
    pub fn used_fraction(self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.used() as f32 / self.total as f32
    }
}

pub fn memory() -> Option<Memory> {
    // SAFETY: MEMORYSTATUSEX is plain data; the API's only requirement is that
    // dwLength is set before the call, which is done immediately below.
    let mut status: MEMORYSTATUSEX = unsafe { mem::zeroed() };
    status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: the struct outlives the call.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }
    Some(Memory { total: status.ullTotalPhys, available: status.ullAvailPhys })
}

/// Cumulative interface byte counters since boot.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetCounters {
    pub received: u64,
    pub sent: u64,
}

/// IANA ifType for software loopback. Traffic to 127.0.0.1 is not network
/// activity in any sense the user means by it.
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
/// IF_OPER_STATUS value for an interface that is up.
const IF_OPER_STATUS_UP: i32 = 1;

/// Bits of MIB_IF_ROW2's InterfaceAndOperStatusFlags, in the order
/// netioapi.h declares them.
///
/// The two that matter here are the ones that mark an entry as not being an
/// adapter anybody chose: a filter interface is an NDIS layer bound to a
/// real one - "Wi-Fi-QoS Packet Scheduler-0000" is the same Wi-Fi radio seen
/// through the scheduler - and an endpoint interface is a RAS or VPN
/// endpoint the system keeps ready, which is what all those "Local Area
/// Connection* 7" entries are. Windows hides both from its own adapter list
/// and so does this: their bytes are already counted on the interface
/// underneath.
const IF_FLAG_HARDWARE_INTERFACE: u8 = 0b0000_0001;
const IF_FLAG_FILTER_INTERFACE: u8 = 0b0000_0010;
const IF_FLAG_ENDPOINT_INTERFACE: u8 = 0b1000_0000;

/// Whether a row is an adapter a person would recognize and choose.
///
/// Measured against what Windows itself lists, which was two entries on the
/// machine this was written on where the table held fifteen:
///
/// - A **filter interface** is an NDIS layer bound to a real one, and the
///   table holds four of them for one Wi-Fi radio - the packet scheduler,
///   two MAC-layer filters, the Wi-Fi filter driver - each reporting the
///   radio's own byte counts. Counting them would multiply the machine's
///   traffic by five.
/// - An **endpoint interface** is a RAS or VPN endpoint.
/// - The **WAN miniports** ("Local Area Connection* 7") are neither, and
///   report as ordinary point-to-point Ethernet that is up. What they are
///   is idle: not hardware, and no byte has ever crossed them. A tunnel
///   that is actually carrying traffic - Tailscale, a VPN - is not
///   hardware either but has moved bytes, and belongs in the list; so the
///   rule is hardware or traffic, and a tunnel appears the moment it
///   carries anything.
fn is_selectable(row: &windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2) -> bool {
    let flags = row.InterfaceAndOperStatusFlags._bitfield;
    let used = row.InOctets > 0 || row.OutOctets > 0;
    row.Type != IF_TYPE_SOFTWARE_LOOPBACK
        && row.OperStatus == IF_OPER_STATUS_UP
        && flags & (IF_FLAG_FILTER_INTERFACE | IF_FLAG_ENDPOINT_INTERFACE) == 0
        && (flags & IF_FLAG_HARDWARE_INTERFACE != 0 || used)
}

/// NDIS media that mean a tunnel: an adapter carrying IP with no link layer
/// of its own (WireGuard, Tailscale, an OpenVPN TUN) or one declaring itself
/// a tunnel outright.
const NDIS_MEDIUM_IP: i32 = 19;
const NDIS_MEDIUM_TUNNEL: i32 = 15;

/// One interface as the counters see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetInterface {
    /// The name Windows shows: "Wi-Fi", "Ethernet 2". The same string the
    /// connection card names, which comes from the adapter table's friendly
    /// name, so the picker and the card agree.
    pub name: String,
    pub counters: NetCounters,
    /// Whether this is a tunnel, whose bytes also cross the adapter beneath
    /// it. Left out of the default sum; still selectable on its own.
    pub is_tunnel: bool,
}

/// Every operational, non-loopback interface with its counters.
///
/// Loopback is left out: traffic to 127.0.0.1 is not network activity in any
/// sense the user means by it.
pub fn net_interfaces() -> Vec<NetInterface> {
    let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
    // SAFETY: GetIfTable2 allocates and writes the pointer. On success we own
    // that allocation and owe it to FreeMibTable, which the guard below pays
    // on every path out of this function, panics included.
    let err = unsafe { GetIfTable2(&mut table) };
    if err != 0 || table.is_null() {
        return Vec::new();
    }

    struct Guard(*mut MIB_IF_TABLE2);
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: only ever built from a pointer GetIfTable2 returned.
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }
    let guard = Guard(table);

    // SAFETY: Table is a flexible array member holding NumEntries rows; the
    // allocation is live for as long as the guard is.
    let rows = unsafe {
        let count = (*guard.0).NumEntries as usize;
        std::slice::from_raw_parts((*guard.0).Table.as_ptr(), count)
    };
    let mut found: Vec<NetInterface> = Vec::new();
    for row in rows {
        if !is_selectable(row) {
            continue;
        }
        let end = row.Alias.iter().position(|c| *c == 0).unwrap_or(row.Alias.len());
        let name = String::from_utf16_lossy(&row.Alias[..end]).trim().to_string();
        if name.is_empty() {
            continue;
        }
        let counters = NetCounters { received: row.InOctets, sent: row.OutOctets };
        let is_tunnel = row.MediaType == NDIS_MEDIUM_IP
            || row.MediaType == NDIS_MEDIUM_TUNNEL
            || row.TunnelType != 0;
        // Two rows with one name is a driver reporting the same adapter
        // twice; their counters belong together rather than as two entries a
        // person is asked to choose between.
        match found.iter_mut().find(|found| found.name == name) {
            Some(found) => {
                found.counters.received = found.counters.received.saturating_add(counters.received);
                found.counters.sent = found.counters.sent.saturating_add(counters.sent);
            }
            None => found.push(NetInterface { name, counters, is_tunnel }),
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Bytes in and out on one interface, by the name `net_interfaces` gave it.
///
/// None where that interface is gone - unplugged, or a dock left behind -
/// which the module shows as unavailable rather than as a stalled zero.
pub fn net_counters_of(name: &str) -> Option<NetCounters> {
    net_interfaces().into_iter().find(|found| found.name == name).map(|found| found.counters)
}

/// Bytes in and out summed across every interface that is not a tunnel.
///
/// Summing rather than following one chosen adapter is the default because
/// the interface carrying traffic changes when a laptop leaves a dock, and a
/// monitor that keeps faithfully reporting zero from an adapter nobody is
/// using is worse than one that adds them up. Somebody who wants one adapter
/// can name it in the settings.
///
/// Tunnels are left out for the reason the Swift's NetworkMonitor leaves out
/// `utun` devices: a tunnel's bytes also cross the physical device beneath
/// it, so counting both reports twice the traffic that actually moved. The
/// tunnel is still in the picker, for somebody who wants to watch that and
/// nothing else.
pub fn net_counters() -> Option<NetCounters> {
    let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
    // SAFETY: GetIfTable2 allocates and writes the pointer. On success we own
    // that allocation and owe it to FreeMibTable, which the guard below pays
    // on every path out of this function, panics included.
    let err = unsafe { GetIfTable2(&mut table) };
    if err != 0 || table.is_null() {
        return None;
    }

    struct Guard(*mut MIB_IF_TABLE2);
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: only ever built from a pointer GetIfTable2 returned.
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }
    let guard = Guard(table);

    let mut totals = NetCounters::default();
    // SAFETY: Table is a flexible array member holding NumEntries rows; the
    // allocation is live for as long as the guard is.
    let rows = unsafe {
        let count = (*guard.0).NumEntries as usize;
        std::slice::from_raw_parts((*guard.0).Table.as_ptr(), count)
    };
    for row in rows {
        // The rows the picker offers, less the tunnels among them.
        if !is_selectable(row) || row.MediaType == NDIS_MEDIUM_IP || row.MediaType == NDIS_MEDIUM_TUNNEL || row.TunnelType != 0 {
            continue;
        }
        totals.received = totals.received.saturating_add(row.InOctets);
        totals.sent = totals.sent.saturating_add(row.OutOctets);
    }
    Some(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interfaces_are_the_ones_windows_itself_lists() {
        let found = net_interfaces();
        for interface in &found {
            eprintln!("interface {:?} in {} out {}", interface.name, interface.counters.received, interface.counters.sent);
            // Not a filter layer or an endpoint: those are the entries whose
            // names carry a bound layer's name after the adapter's.
            assert!(!interface.name.contains("Packet Scheduler"), "{interface:?}");
            assert!(!interface.name.contains("LightWeight Filter"), "{interface:?}");
            assert!(!interface.name.is_empty());
            // Nor an idle WAN miniport, which is what those are called.
            assert!(!interface.name.starts_with("Local Area Connection*"), "{interface:?}");
        }
        // A machine running these tests has a network of some kind; a build
        // agent with none is a machine that could not have fetched the code.
        assert!(!found.is_empty());
        // Every interface named is one this returns counters for.
        let name = found[0].name.clone();
        assert!(net_counters_of(&name).is_some());
        assert_eq!(net_counters_of("no such interface"), None);
    }

    /// What the old rule counted, kept only so the test below can say how
    /// much it over-counted by.
    fn every_row_summed() -> NetCounters {
        let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
        let mut totals = NetCounters::default();
        // SAFETY: the table is freed before returning; rows are read within
        // the count the table reports.
        unsafe {
            if GetIfTable2(&mut table) != 0 || table.is_null() {
                return totals;
            }
            let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
            for row in rows {
                if row.Type != IF_TYPE_SOFTWARE_LOOPBACK && row.OperStatus == IF_OPER_STATUS_UP {
                    totals.received = totals.received.saturating_add(row.InOctets);
                    totals.sent = totals.sent.saturating_add(row.OutOctets);
                }
            }
            FreeMibTable(table.cast());
        }
        totals
    }

    #[test]
    fn the_filter_layers_are_not_counted_a_second_time() {
        // Every NDIS filter bound to an adapter reports that adapter's own
        // byte counts, so summing the table whole multiplies a machine's
        // traffic by however many filters it happens to have - four on the
        // Wi-Fi radio of the machine this was written on. This is that bug,
        // held down by a test: the honest total is the smaller one.
        let honest = net_counters().expect("the table answers");
        let doubled = every_row_summed();
        eprintln!("honest {} received, counting every row {}", honest.received, doubled.received);
        assert!(honest.received <= doubled.received);
    }

    #[test]
    fn every_interface_together_is_what_the_sum_counts() {
        let (mut received, mut sent) = (0u64, 0u64);
        for interface in net_interfaces().into_iter().filter(|found| !found.is_tunnel) {
            received += interface.counters.received;
            sent += interface.counters.sent;
        }
        let total = net_counters().expect("the table answers");
        // Not equality: the two calls are a moment apart and the counters
        // move. Within a megabyte of each other says they count the same
        // set of interfaces rather than two different sets.
        assert!(total.received.abs_diff(received) < 1 << 20, "{} vs {received}", total.received);
        assert!(total.sent.abs_diff(sent) < 1 << 20, "{} vs {sent}", total.sent);
    }
}
