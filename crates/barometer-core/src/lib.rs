// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The sampling engine and the readout modules. This crate creates no window
// and paints nothing: it turns syscalls into short strings, and owns the
// settings and the taskbar measurements those strings are laid out from.

pub mod format;
pub mod module;
pub mod net;
pub mod netinfo;
pub mod volumes;
pub mod pdh;
pub mod sensors;
pub mod settings;
pub mod stack;
pub mod store;
pub mod modules;
pub mod sys;
pub mod weather;
pub mod taskbar;

pub use module::{Module, ModuleId, Readout};
