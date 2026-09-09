// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// What a physical disk is called: the model string the drive itself reports,
// "Samsung SSD 990 PRO 2TB", which is what the Mac's disk panel shows and
// what a person recognizes. The counters only number the disks.

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// The model of physical disk `number`, as the drive reports it, or None
/// where there is no such disk or it will not say.
///
/// Opened with no access rights at all: a query for the device's
/// properties needs none, and asking for read access to a raw disk is
/// what needs an administrator. The vendor comes first where the bus
/// reports one separately - SCSI and USB bridges do, NVMe and SATA fold it
/// into the product - and doubled spaces and padding are taken out.
pub fn disk_model(number: u32) -> Option<String> {
    let path: Vec<u16> = format!("\\\\.\\PhysicalDrive{number}\0").encode_utf16().collect();
    // SAFETY: the path is terminated; the handle is checked and closed
    // below; the query and the buffer outlive the call, and the buffer's
    // length is what the call is told.
    unsafe {
        let handle = CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let mut buffer = vec![0u8; 1024];
        let mut returned = 0u32;
        let ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&mut query as *mut STORAGE_PROPERTY_QUERY).cast(),
            mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut returned,
            ptr::null_mut(),
        );
        CloseHandle(handle);
        if ok == 0 || (returned as usize) < mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return None;
        }
        buffer.truncate(returned as usize);
        let descriptor = ptr::read_unaligned(buffer.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR);
        model_from(&buffer, descriptor.VendorIdOffset, descriptor.ProductIdOffset)
    }
}

/// The vendor and product strings at their offsets in a descriptor buffer,
/// joined as one name. An offset of zero means the field is absent.
fn model_from(buffer: &[u8], vendor: u32, product: u32) -> Option<String> {
    let string_at = |offset: u32| -> Option<String> {
        let start = offset as usize;
        if start == 0 || start >= buffer.len() {
            return None;
        }
        let end = buffer[start..].iter().position(|b| *b == 0).map(|n| start + n).unwrap_or(buffer.len());
        // NVMe drives report "SKHynix_HFS001TEJ9X162N": the underscores are
        // the spaces the drive could not put in a fixed-width field.
        let text = String::from_utf8_lossy(&buffer[start..end]).replace('_', " ");
        let words: Vec<&str> = text.split_whitespace().collect();
        (!words.is_empty()).then(|| words.join(" "))
    };
    let product = string_at(product);
    let vendor = string_at(vendor);
    match (vendor, product) {
        // A product that already starts with the vendor's name does not
        // want it twice; NVMe and SATA drives report it this way.
        (Some(v), Some(p)) if p.to_ascii_lowercase().starts_with(&v.to_ascii_lowercase()) => Some(p),
        (Some(v), Some(p)) => Some(format!("{v} {p}")),
        (None, Some(p)) => Some(p),
        (Some(v), None) => Some(v),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(vendor: &str, product: &str) -> (Vec<u8>, u32, u32) {
        // Fields laid out after a header of 40 bytes, as the real
        // descriptor does; the offsets are what matter here.
        let mut buffer = vec![0u8; 40];
        let vendor_at = if vendor.is_empty() { 0 } else { buffer.len() as u32 };
        buffer.extend_from_slice(vendor.as_bytes());
        buffer.push(0);
        let product_at = if product.is_empty() { 0 } else { buffer.len() as u32 };
        buffer.extend_from_slice(product.as_bytes());
        buffer.push(0);
        (buffer, vendor_at, product_at)
    }

    #[test]
    fn a_product_with_its_vendor_folded_in_is_not_named_twice() {
        let (buffer, v, p) = descriptor("Samsung ", "Samsung SSD 990 PRO 2TB   ");
        assert_eq!(model_from(&buffer, v, p).as_deref(), Some("Samsung SSD 990 PRO 2TB"));
    }

    #[test]
    fn underscores_are_the_spaces_an_nvme_drive_could_not_write() {
        let (buffer, v, p) = descriptor("", "SKHynix_HFS001TEJ9X162N");
        assert_eq!(model_from(&buffer, v, p).as_deref(), Some("SKHynix HFS001TEJ9X162N"));
    }

    #[test]
    fn a_vendor_reported_apart_goes_in_front() {
        let (buffer, v, p) = descriptor("WD      ", "My Passport 25E2");
        assert_eq!(model_from(&buffer, v, p).as_deref(), Some("WD My Passport 25E2"));
    }

    #[test]
    fn a_missing_field_is_an_offset_of_zero_and_padding_is_not_a_name() {
        let (buffer, v, p) = descriptor("", "NVMe KIOXIA 1TB");
        assert_eq!(model_from(&buffer, v, p).as_deref(), Some("NVMe KIOXIA 1TB"));
        let (buffer, v, p) = descriptor("   ", "");
        assert_eq!(model_from(&buffer, v, p), None);
        // An offset past the buffer is a descriptor that lied about its size.
        assert_eq!(model_from(&buffer, 4_000, 0), None);
    }

    #[test]
    fn the_first_physical_disk_has_a_model_a_person_would_recognize() {
        // Every machine that runs the tests boots from something.
        let model = disk_model(0).expect("disk 0 answers");
        assert!(model.len() > 3, "{model:?}");
        assert!(!model.contains("  "));
    }

    #[test]
    fn a_disk_that_is_not_there_is_none_rather_than_a_name() {
        assert_eq!(disk_model(250), None);
    }
}
