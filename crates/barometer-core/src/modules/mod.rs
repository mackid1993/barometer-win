// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein

mod cpu;
mod disk;
mod gpu;
mod memory;
mod network;
mod sensors;
mod weather;
pub use weather::Observation;
pub use cpu::LoadAverage;
pub use disk::DiskDevice;

pub use cpu::CpuModule;
pub use disk::DiskModule;
pub use gpu::GpuModule;
pub use memory::MemoryModule;
pub use network::NetworkModule;
pub use sensors::SensorsModule;
pub use weather::WeatherModule;
