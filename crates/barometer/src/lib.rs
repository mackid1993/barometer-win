// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Everything the taskbar readout is made of, as a library.
//
// The executable is a separate package with nothing in it but `main`, and that
// separation is not tidiness. The icon, the version block and above all the
// manifest are linked by a build script, and a build script's link arguments
// apply to every target in its package - test harnesses included. With the
// manifest requiring administrator, that meant `cargo test` could not run
// without elevation and CI could not run at all. Keeping the build script in a
// package that has no tests is what fixes it.

pub mod flyout;
pub mod trace;
pub mod lhm_install;
pub mod pawnio;
pub mod registry;
pub mod reserve;
pub mod settings_ui;
pub mod startup;
mod taskbar_buttons;
pub mod reserve_block;
pub mod spacer;
pub mod tray;
pub mod update;
pub mod window;
