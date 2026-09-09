// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

//! Puts the icon and version details into the executable.
//!
//! Nothing native is compiled and nothing is linked here. `cargo build` needs
//! a Rust toolchain and nothing else, which is the point: the sensor helper is
//! the only part of Barometer with a build dependency worth the name, and it
//! is a separate program precisely so this one does not inherit it.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    embed_icon(&root);

    // The version goes into a Win32 resource, so a version change has to
    // re-run this script. Once any rerun-if line is printed those become the
    // *only* triggers, and without this one a version bump would relink the
    // previous version's resource: the installer would carry a binary that
    // disagrees with its own filename, and the updater would then offer the
    // release the user had just installed.
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
}

/// Puts the application icon and version details into the executable itself.
///
/// Naming the .ico in the Inno Setup script only gives the *installer* an
/// icon. Explorer, the taskbar and Alt+Tab read a Win32 resource compiled into
/// the binary, and without one they show the generic default however good the
/// artwork is.
///
/// Skipped rather than fatal when the icon or a resource compiler is missing:
/// an ugly icon should never be the reason a build fails.
fn embed_icon(root: &std::path::Path) {
    let icon = root.join("..").join("..").join("assets").join("barometer.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if !icon.exists() {
        println!(r"cargo:warning=assets/barometer.ico not found; run scripts\make-icon.ps1");
        return;
    }

    // The manifest carries the administrator requirement, the DPI awareness and
    // the version 6 common controls. It is a file rather than a string so it
    // can be read and reviewed on its own; every line in it is load-bearing and
    // explained there.
    let manifest = root.join("barometer.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_manifest_file(&manifest.to_string_lossy())
        .set_icon(&icon.to_string_lossy())
        .set("ProductName", "Barometer")
        .set("FileDescription", "Weather and system statistics on the taskbar")
        .set("CompanyName", "Barometer")
        .set("LegalCopyright", "\u{00A9} 2026 David Brustein. GNU GPL v3.");

    if let Err(why) = resource.compile() {
        println!("cargo:warning=could not embed the icon: {why}");
    }
}
