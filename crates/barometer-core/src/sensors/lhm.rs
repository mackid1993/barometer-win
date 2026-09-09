// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Reading LibreHardwareMonitor.
//
// LHM is not shipped with Barometer. The user installs it, and this talks to
// the web server it already has (Utilities/HttpServer.cs upstream). Barometer
// only ever reads: the endpoint also accepts action=Set, which drives fans,
// and nothing here will call it.
//
// The HTTP is written by hand. The only host we speak to is 127.0.0.1, so
// there is no TLS to get right and no redirect to follow, and a request that
// fits in one write against a loopback socket does not justify an HTTP client
// crate and the async runtime that usually follows it.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::Value;

use super::{Sensor, SensorError, SensorKind, SensorProvider};

/// The port LHM listens on unless the user changed it.
pub const DEFAULT_PORT: u16 = 8085;

/// Loopback only. A sensor source on another machine is a different feature
/// with different consent, and this is not it.
const HOST: [u8; 4] = [127, 0, 0, 1];

/// Long enough for a busy machine, short enough that a sampling tick which
/// finds nothing listening does not stall the interval behind it.
const TIMEOUT: Duration = Duration::from_millis(1500);

/// Reads sensors from a running LibreHardwareMonitor.
pub struct LhmProvider {
    port: u16,
    display_name: String,
}

impl Default for LhmProvider {
    fn default() -> Self {
        LhmProvider::new(DEFAULT_PORT)
    }
}

impl LhmProvider {
    pub fn new(port: u16) -> Self {
        LhmProvider { port, display_name: "LibreHardwareMonitor".to_string() }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// One GET, spoken as HTTP/1.0 on purpose.
    ///
    /// 1.0 means the server closes the connection when it is done and never
    /// chunks the body, so "read to end of stream" is the whole framing
    /// problem. Asking for 1.1 would buy keep-alive we do not want and a
    /// chunked decoder we would have to write.
    fn get(&self, path: &str) -> Result<String, SensorError> {
        let address = SocketAddr::from((HOST, self.port));
        let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(|e| {
            match e.kind() {
                // Nothing listening is the ordinary case of LHM not running,
                // not a fault worth showing as an error.
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut => {
                    SensorError::NotRunning
                }
                _ => SensorError::Transport(e.to_string()),
            }
        })?;
        stream.set_read_timeout(Some(TIMEOUT)).ok();
        stream.set_write_timeout(Some(TIMEOUT)).ok();

        let request = format!("GET /{path} HTTP/1.0\r\nHost: 127.0.0.1\r\nAccept: */*\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(|e| SensorError::Transport(e.to_string()))?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(|e| SensorError::Transport(e.to_string()))?;
        let text = String::from_utf8_lossy(&raw).into_owned();

        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| SensorError::Malformed("no header terminator".into()))?;
        let status = head.lines().next().unwrap_or_default();
        if status.contains(" 401") {
            // LHM's server can require basic auth. Carrying credentials is a
            // settings feature that does not exist yet, so say so precisely
            // rather than reporting a parse failure.
            return Err(SensorError::Unauthorized);
        }
        if !status.contains(" 200") {
            return Err(SensorError::Malformed(format!("status line: {status}")));
        }
        Ok(body.to_string())
    }
}

/// Walks the node tree, flattening it into readings.
///
/// The tree is Computer to hardware to category to sensor, and the hardware
/// name has to be carried down because a sensor node knows only its own label:
/// "Core (Tctl/Tdie)" is meaningless without the chip it belongs to.
fn flatten(node: &Value, hardware: &str, out: &mut Vec<Sensor>) {
    let text = node.get("Text").and_then(Value::as_str).unwrap_or_default();

    // A node carrying HardwareId names a device, and everything beneath it
    // belongs to that device.
    let hardware = if node.get("HardwareId").is_some() { text } else { hardware };

    if let Some(sensor_id) = node.get("SensorId").and_then(Value::as_str) {
        let kind = node
            .get("Type")
            .and_then(Value::as_str)
            .map(SensorKind::parse)
            .unwrap_or_else(|| SensorKind::Other(String::new()));
        // RawValue is documented upstream as the unformatted figure "for
        // external systems to have consistent readings", which is precisely
        // this. The formatted Value field is for their own UI and its units
        // change with magnitude.
        let value = node.get("RawValue").and_then(Value::as_f64).filter(|v| v.is_finite());
        out.push(Sensor {
            id: sensor_id.to_string(),
            name: text.to_string(),
            hardware: hardware.to_string(),
            kind,
            value,
        });
    }

    if let Some(children) = node.get("Children").and_then(Value::as_array) {
        for child in children {
            flatten(child, hardware, out);
        }
    }
}

impl SensorProvider for LhmProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    fn read(&mut self) -> Result<Vec<Sensor>, SensorError> {
        let body = self.get("data.json")?;
        let root: Value =
            serde_json::from_str(&body).map_err(|e| SensorError::Malformed(e.to_string()))?;
        let mut sensors = Vec::new();
        flatten(&root, "", &mut sensors);
        if sensors.is_empty() {
            // A tree with no sensors means LHM is up but has read nothing,
            // which is what happens when it is running without the rights its
            // driver needs.
            return Err(SensorError::Malformed(
                "no sensors in the response; LibreHardwareMonitor may lack administrator rights"
                    .into(),
            ));
        }
        Ok(sensors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::hottest;

    /// Shaped like a real /data.json: Computer, then hardware carrying
    /// HardwareId, then a category node, then the sensors.
    fn fixture() -> Value {
        serde_json::json!({
            "id": 0, "Text": "Sensor", "Min": "Min", "Value": "Value", "Max": "Max",
            "Children": [{
                "id": 1, "Text": "DESKTOP", "Children": [{
                    "id": 2, "Text": "AMD Ryzen 9 7950X", "HardwareId": "/amdcpu/0",
                    "Children": [{
                        "id": 3, "Text": "Temperatures", "Children": [
                            { "id": 4, "Text": "Core (Tctl/Tdie)",
                              "SensorId": "/amdcpu/0/temperature/2", "Type": "Temperature",
                              "Value": "58.6 \u{00b0}C", "RawValue": 58.625, "Children": [] },
                            { "id": 5, "Text": "Core (Tccd1)",
                              "SensorId": "/amdcpu/0/temperature/3", "Type": "Temperature",
                              "Value": "NaN", "RawValue": null, "Children": [] }
                        ]
                    }]
                }, {
                    "id": 6, "Text": "NVIDIA GeForce RTX 4080", "HardwareId": "/gpu-nvidia/0",
                    "Children": [{
                        "id": 7, "Text": "Temperatures", "Children": [
                            { "id": 8, "Text": "GPU Core",
                              "SensorId": "/gpu-nvidia/0/temperature/0", "Type": "Temperature",
                              "Value": "71.0 \u{00b0}C", "RawValue": 71.0, "Children": [] }
                        ]
                    }, {
                        "id": 9, "Text": "Fans", "Children": [
                            { "id": 10, "Text": "GPU Fan", "SensorId": "/gpu-nvidia/0/fan/0",
                              "Type": "Fan", "Value": "1400 RPM", "RawValue": 1400.0,
                              "Children": [] }
                        ]
                    }]
                }]
            }]
        })
    }

    fn flattened() -> Vec<Sensor> {
        let mut out = Vec::new();
        flatten(&fixture(), "", &mut out);
        out
    }

    #[test]
    fn every_sensor_is_found_and_category_nodes_are_not_sensors() {
        // Four sensor nodes; the Computer, hardware and category nodes carry
        // no SensorId and must not become readings.
        assert_eq!(flattened().len(), 4);
    }

    #[test]
    fn the_owning_device_is_carried_down_the_tree() {
        // "GPU Core" is meaningless without the card it belongs to, and the
        // sensor node itself does not know it.
        let sensors = flattened();
        let gpu = sensors.iter().find(|s| s.name == "GPU Core").unwrap();
        assert_eq!(gpu.hardware, "NVIDIA GeForce RTX 4080");
        let cpu = sensors.iter().find(|s| s.name == "Core (Tctl/Tdie)").unwrap();
        assert_eq!(cpu.hardware, "AMD Ryzen 9 7950X");
    }

    #[test]
    fn an_unreadable_sensor_is_none_rather_than_zero() {
        // A fan that cannot be read is not a stopped fan, and a core that
        // cannot be read is not a cold core.
        let sensors = flattened();
        let unreadable = sensors.iter().find(|s| s.name == "Core (Tccd1)").unwrap();
        assert_eq!(unreadable.value, None);
    }

    #[test]
    fn kinds_and_units_come_through() {
        let sensors = flattened();
        let fan = sensors.iter().find(|s| s.name == "GPU Fan").unwrap();
        assert_eq!(fan.kind, SensorKind::Fan);
        assert_eq!(fan.kind.unit(), "RPM");
        assert_eq!(fan.value, Some(1400.0));
    }

    #[test]
    fn identifiers_are_the_stable_ones_not_the_display_names() {
        // Display names repeat across devices; the identifier is what a pinned
        // sensor is remembered by.
        let sensors = flattened();
        assert!(sensors.iter().any(|s| s.id == "/gpu-nvidia/0/temperature/0"));
    }

    #[test]
    fn the_hottest_reading_ignores_fans_and_unreadable_sensors() {
        let sensors = flattened();
        let hot = hottest(&sensors).unwrap();
        assert_eq!(hot.name, "GPU Core");
        assert_eq!(hot.value, Some(71.0));
    }
}
