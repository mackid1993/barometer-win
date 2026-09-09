// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// HTTPS, through WinHTTP.
//
// Open-Meteo is HTTPS only, so unlike the loopback client in sensors/lhm.rs
// this cannot be a socket and a hand-written request. The alternative to
// WinHTTP is a TLS stack in the binary - rustls or openssl, a certificate
// store, and an async runtime under whichever HTTP client sits on top - for a
// program that makes one request every fifteen minutes.
//
// WinHTTP is already in Windows. It costs nothing at rest, validates against
// the machine's own certificate store, and picks up the system proxy and its
// credentials, which is the difference between working and not working on a
// corporate network. It is also the transport Defender expects a small native
// program to use; a private TLS implementation reaching the internet is the
// shape of thing that gets looked at.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// How long any one stage may take.
///
/// Generous enough for a slow hotel connection, short enough that a refresh
/// which is never going to arrive stops rather than pinning a thread until the
/// program exits. The worker treats a timeout the same as any other failure:
/// keep the last reading, try again on the next interval.
const TIMEOUT_MS: i32 = 8_000;

/// The user agent every request carries.
///
/// Named and versioned deliberately. Open-Meteo asks for identification on its
/// free tier, and an anonymous request is also the first thing an API blocks
/// when it decides to start blocking things - which is exactly how the weather
/// plugin's geolocation lookup started failing with a 403.
const USER_AGENT: &str = concat!("Barometer/", env!("CARGO_PKG_VERSION"), " (Windows)");

#[derive(Debug)]
pub enum HttpError {
    /// WinHTTP could not be started, or the host could not be reached.
    Transport(String),
    /// The server answered with something other than 2xx.
    Status(u32),
    /// The body was not valid UTF-8.
    Encoding,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Transport(why) => write!(f, "could not reach the service: {why}"),
            HttpError::Status(code) => write!(f, "the service answered {code}"),
            HttpError::Encoding => f.write_str("the service sent something that was not text"),
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// A WinHTTP handle that closes itself.
///
/// The four handles in a request nest, and every early return has to close all
/// of the ones opened so far. Written by hand that is four cleanup paths per
/// error and a leaked session the first time one is missed.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle this type owns, closed exactly once.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// Fetches a URL and returns the body as text.
///
/// Blocking, and meant to be: it is called from a worker thread that has
/// nothing else to do. Never call it from the thread that owns the strip.
pub fn get(host: &str, path_and_query: &str) -> Result<String, HttpError> {
    // No ceiling worth the name: a JSON reply that will not fit in memory is
    // already a fault, and the callers here are asking small questions.
    let bytes = get_bytes(host, path_and_query, 32 * 1024 * 1024)?;
    String::from_utf8(bytes).map_err(|_| HttpError::Encoding)
}

/// Fetches a URL and returns the body as bytes, refusing to grow past `limit`.
///
/// The limit is not decoration. This is what an installer is downloaded
/// through, and a wrong URL - or a right one serving something else - must not
/// be able to fill the disk before anybody notices.
pub fn get_bytes(host: &str, path_and_query: &str, limit: usize) -> Result<Vec<u8>, HttpError> {
    let fail = |what: &str| HttpError::Transport(what.to_string());

    // SAFETY: every handle below is checked for null before use and owned by a
    // Handle that closes it. The buffers passed out are sized by the same call
    // that fills them.
    unsafe {
        let session = Handle(WinHttpOpen(
            wide(USER_AGENT).as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(fail("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);

        let connection = Handle(WinHttpConnect(
            session.0,
            wide(host).as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT as u16,
            0,
        ));
        if connection.0.is_null() {
            return Err(fail("WinHttpConnect"));
        }

        let request = Handle(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(path_and_query).as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(fail("WinHttpOpenRequest"));
        }

        if WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0 {
            return Err(fail("WinHttpSendRequest"));
        }
        if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err(fail("WinHttpReceiveResponse"));
        }

        // The status is asked for as a number rather than parsed out of the
        // status line, which spells it differently in different HTTP versions.
        let mut status: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status as *mut u32 as *mut _,
            &mut status_size,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(fail("WinHttpQueryHeaders"));
        }
        if !(200..300).contains(&status) {
            return Err(HttpError::Status(status));
        }

        // Read until the server says there is nothing more. Availability is
        // asked for each time rather than assumed, because a chunked response
        // arrives in pieces of whatever size it likes.
        let mut body: Vec<u8> = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(fail("WinHttpQueryDataAvailable"));
            }
            if available == 0 {
                break;
            }
            let start = body.len();
            if start + available as usize > limit {
                return Err(HttpError::Transport(format!(
                    "the download is larger than the {limit} byte limit"
                )));
            }
            body.resize(start + available as usize, 0);
            let mut read: u32 = 0;
            if WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr() as *mut _,
                available,
                &mut read,
            ) == 0
            {
                return Err(fail("WinHttpReadData"));
            }
            // A short read is normal, so the buffer is trimmed to what
            // actually arrived rather than left with zeroes in the middle of
            // the JSON.
            body.truncate(start + read as usize);
            if read == 0 {
                break;
            }
        }

        Ok(body)
    }
}

/// Percent-encodes a query parameter value.
///
/// Place names carry spaces, commas and accents, and any of them left
/// unencoded produces a request WinHTTP either mangles or refuses.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_leaves_the_unreserved_set_alone() {
        assert_eq!(encode("Austin"), "Austin");
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn encoding_escapes_what_a_query_string_cannot_carry() {
        assert_eq!(encode("New York"), "New%20York");
        assert_eq!(encode("Bogota, D.C."), "Bogota%2C%20D.C.");
        assert_eq!(encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn encoding_handles_non_ascii_a_byte_at_a_time() {
        // Zurich with an umlaut is two bytes in UTF-8 and has to stay two.
        assert_eq!(encode("Z\u{00FC}rich"), "Z%C3%BCrich");
    }

    #[test]
    fn the_user_agent_names_the_program_and_its_version() {
        assert!(USER_AGENT.starts_with("Barometer/"));
        assert!(USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
    }
}
