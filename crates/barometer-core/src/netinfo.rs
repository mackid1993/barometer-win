// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// What the network panel says about the connection: which interface carries
// the default route, its addresses, router and DNS, the radio behind it when
// it is one, and which processes are moving the bytes.
//
// Three sources, all Win32. The adapter table (GetAdaptersAddresses) for the
// interface and its addresses; the WLAN API for the radio; and, for
// per-process traffic, the TCP table with its owning process ids joined to
// per-connection statistics. Windows keeps no per-process byte counters
// outside ETW, so the counters are read per connection and summed by owner
// - which is what Resource Monitor did before it moved to ETW - and that
// needs the statistics switched on per connection, which needs the elevation
// this program already runs with.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Instant;

use windows_sys::Win32::Foundation::{BOOLEAN, HANDLE};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetAdaptersAddresses, GetBestInterfaceEx, GetExtendedTcpTable, GetIfTable2,
    GetPerTcp6ConnectionEStats, GetPerTcpConnectionEStats, SetPerTcp6ConnectionEStats,
    SetPerTcpConnectionEStats, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_MULTICAST, IF_TYPE_IEEE80211, IF_TYPE_PPP, IF_TYPE_PROP_VIRTUAL,
    IF_TYPE_SOFTWARE_LOOPBACK, IF_TYPE_TUNNEL, IP_ADAPTER_ADDRESSES_LH, MIB_IF_TABLE2,
    MIB_TCP6ROW, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_LH, MIB_TCPROW_LH_0, MIB_TCPTABLE_OWNER_PID,
    MIB_TCP_STATE_ESTAB, TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TCP_TABLE_OWNER_PID_ALL,
    TcpConnectionEstatsData,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::NetworkManagement::WiFi::{
    wlan_interface_state_connected, wlan_intf_opcode_channel_number,
    wlan_intf_opcode_current_connection, wlan_intf_opcode_rssi, WlanCloseHandle,
    WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
    DOT11_AUTH_ALGO_80211_OPEN, DOT11_AUTH_ALGO_OWE, DOT11_AUTH_ALGO_RSNA, DOT11_AUTH_ALGO_RSNA_PSK,
    DOT11_AUTH_ALGO_WPA, DOT11_AUTH_ALGO_WPA3, DOT11_AUTH_ALGO_WPA3_ENT, DOT11_AUTH_ALGO_WPA3_SAE,
    DOT11_AUTH_ALGO_WPA_PSK, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST,
};
use windows_sys::Win32::Networking::WinSock::{SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6};

const AF_UNSPEC: u32 = 0;
const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;

/// The interface carrying the default route, and what is known about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connection {
    /// The friendly name, "Wi-Fi" or "Ethernet".
    pub name: String,
    pub is_wifi: bool,
    /// A tunnel, PPP or virtual adapter, or one whose description names a
    /// VPN product. A heuristic, since Windows has no flag for it.
    pub is_vpn: bool,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub gateways: Vec<String>,
    pub dns: Vec<String>,
    pub errors_in: u64,
    pub errors_out: u64,
}

/// The radio behind a Wi-Fi connection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub ssid: Option<String>,
    /// dBm, when the driver answers; else derived from signal quality.
    pub rssi: Option<i32>,
    pub channel: Option<u32>,
    pub band: Option<String>,
    pub transmit_mbps: Option<f64>,
    pub receive_mbps: Option<f64>,
    pub security: Option<String>,
}

/// A process's share of the traffic over the last interval, bytes per second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessRate {
    pub pid: u32,
    pub down: f64,
    pub up: f64,
}

/// Whether an adapter's type or description says it is a VPN.
///
/// Tunnels and PPP links are VPNs nearly always; the description catches the
/// products that register as an ordinary Ethernet adapter instead.
pub fn looks_like_vpn(if_type: u32, description: &str) -> bool {
    if matches!(if_type, IF_TYPE_TUNNEL | IF_TYPE_PPP | IF_TYPE_PROP_VIRTUAL) {
        return true;
    }
    let lower = description.to_ascii_lowercase();
    ["vpn", "wireguard", "tap-windows", "tailscale", "openvpn", "zerotier", "nordlynx"]
        .iter()
        .any(|word| lower.contains(word))
}

/// The band a Wi-Fi channel number sits in.
///
/// 6 GHz channels reuse numbers from the 5 GHz plan, and the WLAN API's
/// channel opcode does not say which; the 6 GHz numbers that are unique to
/// that band are called out, the rest are read as 5 GHz.
pub fn band_of(channel: u32) -> &'static str {
    match channel {
        1..=14 => "2.4 GHz",
        32..=177 => "5 GHz",
        _ => "6 GHz",
    }
}

/// The security name for an 802.11 authentication algorithm.
pub fn security_name(algorithm: i32) -> &'static str {
    match algorithm {
        DOT11_AUTH_ALGO_80211_OPEN => "Open",
        DOT11_AUTH_ALGO_WPA | DOT11_AUTH_ALGO_WPA_PSK => "WPA",
        DOT11_AUTH_ALGO_RSNA => "WPA2 Enterprise",
        DOT11_AUTH_ALGO_RSNA_PSK => "WPA2",
        DOT11_AUTH_ALGO_WPA3 | DOT11_AUTH_ALGO_WPA3_ENT => "WPA3 Enterprise",
        DOT11_AUTH_ALGO_WPA3_SAE => "WPA3",
        DOT11_AUTH_ALGO_OWE => "Enhanced Open",
        _ => "Unknown",
    }
}

/// A socket address as text, or None for a family this does not read.
///
/// Link-local IPv6 is left out: every interface has one, it says nothing
/// about the connection, and it doubles the length of the list.
unsafe fn address_text(address: *const SOCKADDR) -> Option<String> {
    if address.is_null() {
        return None;
    }
    match (*address).sa_family as u32 {
        AF_INET => {
            let v4 = &*(address as *const SOCKADDR_IN);
            let bytes = v4.sin_addr.S_un.S_addr.to_ne_bytes();
            Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string())
        }
        AF_INET6 => {
            let v6 = &*(address as *const SOCKADDR_IN6);
            let ip = Ipv6Addr::from(v6.sin6_addr.u.Byte);
            // fe80::/10 is link-local; fec0::/10 is the site-local range the
            // stack fills DNS with as a placeholder when nothing is set.
            let head = ip.segments()[0];
            if head & 0xffc0 == 0xfe80 || head & 0xffc0 == 0xfec0 {
                return None;
            }
            Some(ip.to_string())
        }
        _ => None,
    }
}

unsafe fn wide_text(text: *const u16) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut length = 0;
    while *text.add(length) != 0 {
        length += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(text, length))
}

/// The interface the default route goes out of, with its addresses.
///
/// The route decides, not the byte counts: the busiest interface is not
/// always the one the user thinks of as "the connection", and the Mac's
/// panel is about the primary interface too.
pub fn connection() -> Option<Connection> {
    // SAFETY: every pointer walked is inside the buffer GetAdaptersAddresses
    // filled, whose size it reported; the linked lists end in null.
    unsafe {
        let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
        let mut size: u32 = 16 * 1024;
        let mut buffer: Vec<u8> = vec![0; size as usize];
        let mut status = GetAdaptersAddresses(
            AF_UNSPEC,
            flags,
            std::ptr::null(),
            buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
            &mut size,
        );
        // ERROR_BUFFER_OVERFLOW: the size came back as what is needed.
        if status == 111 {
            buffer = vec![0; size as usize];
            status = GetAdaptersAddresses(
                AF_UNSPEC,
                flags,
                std::ptr::null(),
                buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            );
        }
        if status != 0 {
            return None;
        }

        // The interface the default route leaves by, asked about a public
        // address that is never actually contacted.
        let mut probe: SOCKADDR_IN = std::mem::zeroed();
        probe.sin_family = AF_INET as u16;
        probe.sin_addr.S_un.S_addr = u32::from_ne_bytes([8, 8, 8, 8]);
        let mut best_index = 0u32;
        let has_best =
            GetBestInterfaceEx(&probe as *const SOCKADDR_IN as *const SOCKADDR, &mut best_index) == 0;

        let mut fallback: Option<Connection> = None;
        let mut adapter = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let a = &*adapter;
            adapter = a.Next;
            if a.OperStatus != IfOperStatusUp || a.IfType == IF_TYPE_SOFTWARE_LOOPBACK {
                continue;
            }
            let mut found = Connection {
                name: wide_text(a.FriendlyName),
                is_wifi: a.IfType == IF_TYPE_IEEE80211,
                is_vpn: looks_like_vpn(a.IfType, &wide_text(a.Description)),
                ..Default::default()
            };
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                let u = &*unicast;
                unicast = u.Next;
                if let Some(text) = address_text(u.Address.lpSockaddr) {
                    if text.contains(':') {
                        found.ipv6.push(text);
                    } else {
                        found.ipv4.push(text);
                    }
                }
            }
            let mut gateway = a.FirstGatewayAddress;
            while !gateway.is_null() {
                let g = &*gateway;
                gateway = g.Next;
                if let Some(text) = address_text(g.Address.lpSockaddr) {
                    found.gateways.push(text);
                }
            }
            let mut dns = a.FirstDnsServerAddress;
            while !dns.is_null() {
                let d = &*dns;
                dns = d.Next;
                if let Some(text) = address_text(d.Address.lpSockaddr) {
                    found.dns.push(text);
                }
            }
            let (errors_in, errors_out) = interface_errors(a.Luid.Value);
            found.errors_in = errors_in;
            found.errors_out = errors_out;

            if has_best && a.Anonymous1.Anonymous.IfIndex == best_index {
                return Some(found);
            }
            // Without a route answer, the first interface that has a gateway
            // is the best guess there is.
            if fallback.is_none() && !found.gateways.is_empty() {
                fallback = Some(found);
            }
        }
        fallback
    }
}

/// Receive and transmit error counts for an interface, by LUID.
unsafe fn interface_errors(luid: u64) -> (u64, u64) {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    if GetIfTable2(&mut table) != 0 || table.is_null() {
        return (0, 0);
    }
    let count = (*table).NumEntries as usize;
    let rows = (*table).Table.as_ptr();
    let mut errors = (0, 0);
    for index in 0..count {
        let row = &*rows.add(index);
        if row.InterfaceLuid.Value == luid {
            errors = (row.InErrors, row.OutErrors);
            break;
        }
    }
    FreeMibTable(table as *const _);
    errors
}

/// The connected radio, if the machine has one that is connected.
pub fn wifi() -> Option<Wifi> {
    // SAFETY: handles and buffers from the WLAN API, each freed with the
    // call the API pairs it with, and the handle closed on every path.
    unsafe {
        let mut version = 0u32;
        let mut handle: HANDLE = std::ptr::null_mut();
        if WlanOpenHandle(2, std::ptr::null(), &mut version, &mut handle) != 0 {
            return None;
        }
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let mut found = None;
        if WlanEnumInterfaces(handle, std::ptr::null(), &mut list) == 0 && !list.is_null() {
            let count = (*list).dwNumberOfItems as usize;
            let items = (*list).InterfaceInfo.as_ptr();
            for index in 0..count {
                let item = &*items.add(index);
                if item.isState != wlan_interface_state_connected {
                    continue;
                }
                let guid = &item.InterfaceGuid;
                let mut size = 0u32;
                let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
                let queried = WlanQueryInterface(
                    handle,
                    guid,
                    wlan_intf_opcode_current_connection,
                    std::ptr::null(),
                    &mut size,
                    &mut data,
                    std::ptr::null_mut(),
                );
                if queried != 0 || data.is_null() {
                    continue;
                }
                let attributes = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                let association = &attributes.wlanAssociationAttributes;
                let ssid_length = (association.dot11Ssid.uSSIDLength as usize).min(32);
                let ssid = String::from_utf8_lossy(&association.dot11Ssid.ucSSID[..ssid_length])
                    .trim()
                    .to_string();
                let mut radio = Wifi {
                    ssid: (!ssid.is_empty()).then_some(ssid),
                    // Rates come in kbps.
                    transmit_mbps: Some(association.ulTxRate as f64 / 1000.0),
                    receive_mbps: Some(association.ulRxRate as f64 / 1000.0),
                    security: Some(
                        security_name(attributes.wlanSecurityAttributes.dot11AuthAlgorithm)
                            .to_string(),
                    ),
                    // Signal quality is 0 to 100 mapped from -100 to -50 dBm,
                    // which is the fallback when the driver has no RSSI.
                    rssi: Some(association.wlanSignalQuality as i32 / 2 - 100),
                    ..Default::default()
                };
                WlanFreeMemory(data);

                let mut value: *mut std::ffi::c_void = std::ptr::null_mut();
                let mut value_size = 0u32;
                if WlanQueryInterface(
                    handle,
                    guid,
                    wlan_intf_opcode_rssi,
                    std::ptr::null(),
                    &mut value_size,
                    &mut value,
                    std::ptr::null_mut(),
                ) == 0
                    && !value.is_null()
                {
                    if value_size >= 4 {
                        radio.rssi = Some(*(value as *const i32));
                    }
                    WlanFreeMemory(value);
                }
                let mut channel: *mut std::ffi::c_void = std::ptr::null_mut();
                let mut channel_size = 0u32;
                if WlanQueryInterface(
                    handle,
                    guid,
                    wlan_intf_opcode_channel_number,
                    std::ptr::null(),
                    &mut channel_size,
                    &mut channel,
                    std::ptr::null_mut(),
                ) == 0
                    && !channel.is_null()
                {
                    if channel_size >= 4 {
                        let number = *(channel as *const u32);
                        radio.channel = Some(number);
                        radio.band = Some(band_of(number).to_string());
                    }
                    WlanFreeMemory(channel);
                }
                found = Some(radio);
                break;
            }
            WlanFreeMemory(list as *const _);
        }
        WlanCloseHandle(handle, std::ptr::null());
        found
    }
}

/// Every connection this process has switched extended accounting on for.
///
/// Kept so that it is switched on once rather than on every sample, and so
/// that it can be switched off again. `SetPerTcpConnectionEStats` makes
/// tcpip.sys keep extra per-connection statistics for the life of that
/// connection, and it was being issued for every established connection on
/// the machine - not just Barometer's - on every sample, and never undone.
/// Opening the network panel once therefore taxed every connection the
/// machine had open, a browser's several hundred included, for as long as
/// they lasted, whether or not the panel was still up.
static ENABLED: Mutex<Option<HashSet<ConnectionKey>>> = Mutex::new(None);

/// Switches extended accounting back off everywhere it was switched on.
///
/// Called when the sampler goes away, which is when the panel that wanted
/// the per-process figures has closed.
pub fn stop_collecting() {
    let Ok(mut guard) = ENABLED.lock() else { return };
    let Some(keys) = guard.take() else { return };
    for key in keys {
        // SAFETY: a row built from the key, and a value of the size stated.
        unsafe { set_collection(&key, false) };
    }
}

/// Switches extended accounting on or off for one connection.
///
/// # Safety
///
/// The key must describe a connection this process may ask about.
unsafe fn set_collection(key: &ConnectionKey, on: bool) -> bool {
    let value = TCP_ESTATS_DATA_RW_v0 { EnableCollection: u8::from(on) as BOOLEAN };
    if key.local_port == u32::MAX {
        return false;
    }
    let plain = MIB_TCPROW_LH {
        Anonymous: MIB_TCPROW_LH_0 { dwState: MIB_TCP_STATE_ESTAB as u32 },
        dwLocalAddr: u32::from_ne_bytes([key.local[0], key.local[1], key.local[2], key.local[3]]),
        dwLocalPort: key.local_port,
        dwRemoteAddr: u32::from_ne_bytes([
            key.remote[0],
            key.remote[1],
            key.remote[2],
            key.remote[3],
        ]),
        dwRemotePort: key.remote_port,
    };
    SetPerTcpConnectionEStats(
        &plain,
        TcpConnectionEstatsData,
        &value as *const TCP_ESTATS_DATA_RW_v0 as *const u8,
        0,
        std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
        0,
    ) == 0
}

/// A TCP connection, as the key its counters are remembered under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ConnectionKey {
    local: [u8; 16],
    local_port: u32,
    remote: [u8; 16],
    remote_port: u32,
}

/// Per-process traffic, from the byte counts of every TCP connection each
/// process owns, differenced between samples.
///
/// UDP is not counted: Windows keeps no per-socket counters for it. A
/// connection is first seen with its counts as a baseline and contributes
/// from the next sample on, so a connection that lives less than one
/// interval is missed - the cost of reading counters rather than events.
#[derive(Default)]
pub struct TrafficSampler {
    seen: HashMap<ConnectionKey, (u32, u64, u64)>,
    at: Option<Instant>,
}

impl TrafficSampler {
    pub fn new() -> TrafficSampler {
        TrafficSampler::default()
    }

    /// Bytes per second per process since the previous sample, or None on
    /// the first sample and when the tables cannot be read.
    pub fn sample(&mut self) -> Option<Vec<ProcessRate>> {
        let now = Instant::now();
        let current = read_connections()?;
        let elapsed = self.at.map(|at| now.duration_since(at).as_secs_f64());
        let mut rates: HashMap<u32, (f64, f64)> = HashMap::new();
        for (key, (pid, bytes_in, bytes_out)) in &current {
            if let (Some(elapsed), Some((_, was_in, was_out))) = (elapsed, self.seen.get(key)) {
                if elapsed > 0.0 {
                    let entry = rates.entry(*pid).or_insert((0.0, 0.0));
                    entry.0 += bytes_in.saturating_sub(*was_in) as f64 / elapsed;
                    entry.1 += bytes_out.saturating_sub(*was_out) as f64 / elapsed;
                }
            }
        }
        self.seen = current;
        let first = self.at.is_none();
        self.at = Some(now);
        if first {
            return None;
        }
        let mut list: Vec<ProcessRate> =
            rates.into_iter().map(|(pid, (down, up))| ProcessRate { pid, down, up }).collect();
        list.sort_by(|a, b| (b.down + b.up).total_cmp(&(a.down + a.up)));
        Some(list)
    }
}

/// Every established TCP connection with its owner and byte counts.
fn read_connections() -> Option<HashMap<ConnectionKey, (u32, u64, u64)>> {
    let mut all = HashMap::new();
    // SAFETY: tables sized by the API's own report, rows read within that
    // count; the statistics calls take rows this function builds.
    unsafe {
        // The table is asked for its size and then read, and asked again if
        // it grew in between - which it does, because connections open while
        // this is running. The margin that used to stand in for the retry was
        // a guessed 1024 bytes, so about forty new connections between the
        // two calls dropped the whole per-process list for that tick.
        //
        // The buffer is a vector of u64 rather than of u8 so that it is
        // aligned for the structure it is read back as. A `Vec<u8>` is
        // aligned to one, and forming a reference at an address that does not
        // suit the type is undefined however the address happens to come out.
        let mut buffer: Vec<u64> = Vec::new();
        let mut size = 0u32;
        GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0);
        let mut attempts = 0;
        loop {
            buffer.clear();
            buffer.resize((size as usize).div_ceil(8).max(1), 0);
            let mut asked = (buffer.len() * 8) as u32;
            let status = GetExtendedTcpTable(
                buffer.as_mut_ptr().cast(),
                &mut asked,
                0,
                AF_INET,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
            if status == 0 {
                break;
            }
            attempts += 1;
            // ERROR_INSUFFICIENT_BUFFER, with `asked` now holding what the
            // table has grown to. Bounded, because a machine opening
            // connections faster than this loop can allocate has no answer to
            // give and would otherwise spin.
            if status != 122 || attempts > 4 {
                return None;
            }
            size = asked;
        }
        let table = &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let rows = table.table.as_ptr();
        for index in 0..table.dwNumEntries as usize {
            let row = &*rows.add(index);
            if row.dwState != MIB_TCP_STATE_ESTAB as u32 {
                continue;
            }
            let plain = MIB_TCPROW_LH {
                Anonymous: MIB_TCPROW_LH_0 { dwState: row.dwState },
                dwLocalAddr: row.dwLocalAddr,
                dwLocalPort: row.dwLocalPort,
                dwRemoteAddr: row.dwRemoteAddr,
                dwRemotePort: row.dwRemotePort,
            };
            let mut local = [0u8; 16];
            local[..4].copy_from_slice(&row.dwLocalAddr.to_ne_bytes());
            let mut remote = [0u8; 16];
            remote[..4].copy_from_slice(&row.dwRemoteAddr.to_ne_bytes());
            let key = ConnectionKey {
                local,
                local_port: row.dwLocalPort,
                remote,
                remote_port: row.dwRemotePort,
            };
            // Switched on once per connection rather than on every sample.
            // Switching collection on needs elevation; without it the read
            // below answers with whatever was in the buffer, so a refusal
            // means this connection is not counted at all.
            let already = ENABLED
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().map(|set| set.contains(&key)))
                .unwrap_or(false);
            if !already {
                let enable = TCP_ESTATS_DATA_RW_v0 { EnableCollection: 1 as BOOLEAN };
                if SetPerTcpConnectionEStats(
                    &plain,
                    TcpConnectionEstatsData,
                    &enable as *const TCP_ESTATS_DATA_RW_v0 as *const u8,
                    0,
                    std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                    0,
                ) != 0
                {
                    continue;
                }
                if let Ok(mut guard) = ENABLED.lock() {
                    guard.get_or_insert_with(HashSet::new).insert(key.clone());
                }
            }
            let mut data: TCP_ESTATS_DATA_ROD_v0 = std::mem::zeroed();
            if GetPerTcpConnectionEStats(
                &plain,
                TcpConnectionEstatsData,
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                0,
                0,
                &mut data as *mut TCP_ESTATS_DATA_ROD_v0 as *mut u8,
                0,
                std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
            ) != 0
            {
                continue;
            }
            all.insert(key, (row.dwOwningPid, data.DataBytesIn, data.DataBytesOut));
        }

        let mut size6 = 0u32;
        GetExtendedTcpTable(std::ptr::null_mut(), &mut size6, 0, AF_INET6, TCP_TABLE_OWNER_PID_ALL, 0);
        let mut buffer6: Vec<u8> = vec![0; size6 as usize + 1024];
        let mut size6 = buffer6.len() as u32;
        if GetExtendedTcpTable(
            buffer6.as_mut_ptr().cast(),
            &mut size6,
            0,
            AF_INET6,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        ) == 0
        {
            let table = &*(buffer6.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
            let rows = table.table.as_ptr();
            for index in 0..table.dwNumEntries as usize {
                let row = &*rows.add(index);
                if row.dwState != MIB_TCP_STATE_ESTAB as u32 {
                    continue;
                }
                let mut plain: MIB_TCP6ROW = std::mem::zeroed();
                plain.State = row.dwState as i32;
                plain.LocalAddr.u.Byte = row.ucLocalAddr;
                plain.dwLocalScopeId = row.dwLocalScopeId;
                plain.dwLocalPort = row.dwLocalPort;
                plain.RemoteAddr.u.Byte = row.ucRemoteAddr;
                plain.dwRemoteScopeId = row.dwRemoteScopeId;
                plain.dwRemotePort = row.dwRemotePort;
                let enable = TCP_ESTATS_DATA_RW_v0 { EnableCollection: 1 as BOOLEAN };
                // Switching collection on needs elevation; without it the read
                // below answers with whatever was in the buffer, so a refusal
                // means this connection is not counted at all.
                if SetPerTcp6ConnectionEStats(
                    &plain,
                    TcpConnectionEstatsData,
                    &enable as *const TCP_ESTATS_DATA_RW_v0 as *const u8,
                    0,
                    std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                    0,
                ) != 0
                {
                    continue;
                }
                let mut data: TCP_ESTATS_DATA_ROD_v0 = std::mem::zeroed();
                if GetPerTcp6ConnectionEStats(
                    &plain,
                    TcpConnectionEstatsData,
                    std::ptr::null_mut(),
                    0,
                    0,
                    std::ptr::null_mut(),
                    0,
                    0,
                    &mut data as *mut TCP_ESTATS_DATA_ROD_v0 as *mut u8,
                    0,
                    std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
                ) != 0
                {
                    continue;
                }
                let key = ConnectionKey {
                    local: row.ucLocalAddr,
                    local_port: row.dwLocalPort,
                    remote: row.ucRemoteAddr,
                    remote_port: row.dwRemotePort,
                };
                all.insert(key, (row.dwOwningPid, data.DataBytesIn, data.DataBytesOut));
            }
        }
    }
    Some(all)
}

/// The machine's addresses as the internet sees them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicIp {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

/// Asks ipify what address this machine appears from, one family at a time,
/// as the macOS app's PublicIPSource does. Blocking, and for a worker
/// thread: two requests to the internet.
///
/// Never called unless the user switched it on: a monitor that phones out
/// by default is a monitor telling somebody where its user is.
pub fn public_ip() -> Option<PublicIp> {
    let ipv4 = crate::net::get("api.ipify.org", "/?format=json")
        .ok()
        .and_then(|body| address_in(&body))
        .filter(|a| a.parse::<Ipv4Addr>().is_ok());
    let ipv6 = crate::net::get("api64.ipify.org", "/?format=json")
        .ok()
        .and_then(|body| address_in(&body))
        .filter(|a| a.parse::<Ipv6Addr>().is_ok());
    (ipv4.is_some() || ipv6.is_some()).then_some(PublicIp { ipv4, ipv6 })
}

/// The address in an ipify answer, `{"ip":"203.0.113.9"}`.
fn address_in(body: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(body).ok()?;
    root.get("ip")?.as_str().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ipify_answer_is_the_address_in_it_and_nothing_else_is() {
        assert_eq!(address_in(r#"{"ip":"203.0.113.9"}"#).as_deref(), Some("203.0.113.9"));
        assert_eq!(address_in(r#"{"ip":" 2001:db8::1 "}"#).as_deref(), Some("2001:db8::1"));
        assert_eq!(address_in("<html>not json</html>"), None);
        assert_eq!(address_in(r#"{"address":"203.0.113.9"}"#), None);
    }

    #[test]
    fn tunnels_and_named_vpn_products_read_as_vpns_and_ethernet_does_not() {
        assert!(looks_like_vpn(IF_TYPE_TUNNEL, "Anything"));
        assert!(looks_like_vpn(6, "WireGuard Tunnel"));
        assert!(looks_like_vpn(6, "TAP-Windows Adapter V9"));
        assert!(!looks_like_vpn(6, "Intel(R) Ethernet Connection"));
        assert!(!looks_like_vpn(IF_TYPE_IEEE80211, "Intel(R) Wi-Fi 6E AX211"));
    }

    #[test]
    fn channel_numbers_map_to_their_bands() {
        assert_eq!(band_of(6), "2.4 GHz");
        assert_eq!(band_of(36), "5 GHz");
        assert_eq!(band_of(149), "5 GHz");
        assert_eq!(band_of(233), "6 GHz");
    }

    #[test]
    fn security_names_read_as_a_user_would_expect() {
        assert_eq!(security_name(DOT11_AUTH_ALGO_RSNA_PSK), "WPA2");
        assert_eq!(security_name(DOT11_AUTH_ALGO_WPA3_SAE), "WPA3");
        assert_eq!(security_name(DOT11_AUTH_ALGO_80211_OPEN), "Open");
        assert_eq!(security_name(999), "Unknown");
    }

    #[test]
    fn the_first_traffic_sample_is_a_baseline_and_yields_nothing() {
        // Live against the machine: the tables read, the first call returns
        // None, and a second call returns a list that may well be empty.
        let mut sampler = TrafficSampler::new();
        assert!(sampler.sample().is_none());
        assert!(sampler.sample().is_some());
    }

    /// `cargo test -p barometer-core netinfo -- --ignored --nocapture` shows
    /// what this machine answers, for eyeballing the addresses and the radio.
    #[test]
    #[ignore]
    fn what_this_machine_answers() {
        eprintln!("connection: {:#?}", connection());
        eprintln!("wifi: {:#?}", wifi());
        let mut sampler = TrafficSampler::new();
        sampler.sample();
        std::thread::sleep(std::time::Duration::from_secs(2));
        let rates = sampler.sample().unwrap_or_default();
        eprintln!("traffic: {} processes with connections; busiest: {:?}", rates.len(), rates.first());
        let first = read_connections().unwrap_or_default();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let second = read_connections().unwrap_or_default();
        for (key, (pid, i1, o1)) in first.iter().take(8) {
            let later = second.get(key);
            eprintln!("pid {pid} port {} -> {}: in {i1} out {o1}  then {:?}", key.local_port, key.remote_port, later);
        }
    }

    #[test]
    fn the_connection_and_the_radio_can_be_asked_for_without_failing() {
        // Live: whether either is Some depends on the machine, but asking
        // must not panic and an answer must be self-consistent.
        if let Some(found) = connection() {
            assert!(!found.name.is_empty());
        }
        if let Some(radio) = wifi() {
            if let Some(channel) = radio.channel {
                assert!(channel > 0);
            }
        }
    }
}
