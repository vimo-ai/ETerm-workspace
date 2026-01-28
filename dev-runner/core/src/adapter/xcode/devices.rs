//! Device listing for Xcode adapter
//!
//! Handles parsing of `xcrun simctl list -j` and `xcrun devicectl list devices --json-output`

use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

use crate::adapter::{Device, DeviceState, DeviceType};

/// Platform type for filtering devices
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    IOS,
    TvOS,
    WatchOS,
    VisionOS,
}

// ============================================================================
// Simulator JSON structures (simctl list -j)
// ============================================================================

#[derive(Debug, Deserialize)]
pub(crate) struct SimctlOutput {
    pub devices: HashMap<String, Vec<SimctlDevice>>,
    pub runtimes: Vec<SimctlRuntime>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SimctlDevice {
    pub udid: String,
    pub name: String,
    pub state: String,
    #[serde(rename = "isAvailable")]
    pub is_available: bool,
    #[serde(rename = "deviceTypeIdentifier")]
    pub device_type_identifier: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SimctlRuntime {
    pub identifier: String,
    pub name: String,
    pub version: String,
    pub platform: String,
    #[serde(rename = "isAvailable")]
    pub is_available: bool,
}

// ============================================================================
// Physical device JSON structures (devicectl list devices --json-output)
// ============================================================================

#[derive(Debug, Deserialize)]
pub(crate) struct DevicectlOutput {
    pub result: DevicectlResult,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DevicectlResult {
    pub devices: Vec<DevicectlDevice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DevicectlDevice {
    pub identifier: String,
    #[serde(rename = "deviceProperties")]
    pub device_properties: DevicectlDeviceProperties,
    #[serde(rename = "hardwareProperties")]
    pub hardware_properties: DevicectlHardwareProperties,
    #[serde(rename = "connectionProperties")]
    pub connection_properties: DevicectlConnectionProperties,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DevicectlDeviceProperties {
    pub name: String,
    #[serde(rename = "osVersionNumber")]
    pub os_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DevicectlHardwareProperties {
    pub platform: String,
    #[serde(rename = "deviceType")]
    pub device_type: String,
    pub udid: String,
    #[serde(rename = "marketingName")]
    pub marketing_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DevicectlConnectionProperties {
    #[serde(rename = "tunnelState")]
    pub tunnel_state: String,
}

// ============================================================================
// Parsing functions
// ============================================================================

/// Parse simctl JSON output into Device list, filtered by platform
pub fn parse_simulators(json: &str, platform: Platform) -> Result<Vec<Device>, serde_json::Error> {
    let output: SimctlOutput = serde_json::from_str(json)?;

    let platform_str = match platform {
        Platform::IOS => "iOS",
        Platform::TvOS => "tvOS",
        Platform::WatchOS => "watchOS",
        Platform::VisionOS => "xrOS",
        Platform::MacOS => return Ok(Vec::new()), // No simulators for macOS
    };

    // Build runtime lookup: identifier -> (name, version)
    let runtime_map: HashMap<&str, (&str, &str)> = output
        .runtimes
        .iter()
        .filter(|r| r.is_available && r.platform == platform_str)
        .map(|r| (r.identifier.as_str(), (r.name.as_str(), r.version.as_str())))
        .collect();

    let mut devices = Vec::new();

    for (runtime_id, sims) in &output.devices {
        // Only include simulators for the target platform
        if !runtime_id.contains(platform_str) {
            continue;
        }

        let os_version = runtime_map
            .get(runtime_id.as_str())
            .map(|(_, v)| v.to_string());

        for sim in sims {
            if !sim.is_available {
                continue;
            }

            // Both Booted and Shutdown simulators are available for use
            // Shutdown ones will be auto-booted when needed
            let state = match sim.state.as_str() {
                "Booted" | "Shutdown" => DeviceState::Available,
                _ => DeviceState::Unknown,
            };

            devices.push(Device {
                id: sim.udid.clone(),
                name: sim.name.clone(),
                device_type: DeviceType::Simulator,
                os_version: os_version.clone(),
                state,
            });
        }
    }

    // Sort by OS version (descending) then name
    devices.sort_by(|a, b| {
        let va = a.os_version.as_deref().unwrap_or("");
        let vb = b.os_version.as_deref().unwrap_or("");
        match vb.cmp(va) {
            std::cmp::Ordering::Equal => a.name.cmp(&b.name),
            other => other,
        }
    });

    Ok(devices)
}

/// Parse devicectl JSON output into Device list, filtered by platform
pub fn parse_physical_devices(json: &str, platform: Platform) -> Result<Vec<Device>, serde_json::Error> {
    let output: DevicectlOutput = serde_json::from_str(json)?;

    let platform_str = match platform {
        Platform::IOS => "iOS",
        Platform::TvOS => "tvOS",
        Platform::WatchOS => "watchOS",
        Platform::VisionOS => "xrOS",
        Platform::MacOS => return Ok(Vec::new()), // No physical devices for macOS
    };

    let devices = output
        .result
        .devices
        .into_iter()
        .filter(|d| d.hardware_properties.platform == platform_str)
        .map(|d| {
            let state = match d.connection_properties.tunnel_state.as_str() {
                "connected" => DeviceState::Available,
                "disconnected" => DeviceState::Unavailable,
                _ => DeviceState::Unknown,
            };

            Device {
                id: d.identifier,
                name: d.device_properties.name,
                device_type: DeviceType::Physical,
                os_version: d.device_properties.os_version,
                state,
            }
        })
        .collect();

    Ok(devices)
}

// ============================================================================
// Command execution
// ============================================================================

/// Fetch simulator list via simctl, filtered by platform
pub fn fetch_simulators(platform: Platform) -> Vec<Device> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "-j"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let json = String::from_utf8_lossy(&out.stdout);
            parse_simulators(&json, platform).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Fetch physical device list via devicectl, filtered by platform
pub fn fetch_physical_devices(platform: Platform) -> Vec<Device> {
    // devicectl requires --json-output to a file, use temp file
    let temp_path = std::env::temp_dir().join("devrunner_devices.json");

    let status = Command::new("xcrun")
        .args([
            "devicectl",
            "list",
            "devices",
            "--json-output",
            temp_path.to_str().unwrap_or("/tmp/devrunner_devices.json"),
        ])
        .status();

    if status.map(|s| s.success()).unwrap_or(false) {
        if let Ok(json) = std::fs::read_to_string(&temp_path) {
            // Clean up temp file
            let _ = std::fs::remove_file(&temp_path);
            return parse_physical_devices(&json, platform).unwrap_or_default();
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simulators_basic() {
        let json = r#"{
            "devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
                    {
                        "udid": "ABC-123",
                        "name": "iPhone 15 Pro",
                        "state": "Shutdown",
                        "isAvailable": true
                    },
                    {
                        "udid": "DEF-456",
                        "name": "iPhone 15",
                        "state": "Booted",
                        "isAvailable": true
                    }
                ]
            },
            "runtimes": [
                {
                    "identifier": "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
                    "name": "iOS 17.0",
                    "version": "17.0",
                    "platform": "iOS",
                    "isAvailable": true
                }
            ]
        }"#;

        let devices = parse_simulators(json).unwrap();
        assert_eq!(devices.len(), 2);

        // Check first device
        let iphone15 = devices.iter().find(|d| d.name == "iPhone 15").unwrap();
        assert_eq!(iphone15.id, "DEF-456");
        assert_eq!(iphone15.device_type, DeviceType::Simulator);
        assert_eq!(iphone15.os_version, Some("17.0".to_string()));
        assert_eq!(iphone15.state, DeviceState::Available);

        // Check second device
        let iphone15pro = devices.iter().find(|d| d.name == "iPhone 15 Pro").unwrap();
        assert_eq!(iphone15pro.state, DeviceState::Unavailable);
    }

    #[test]
    fn test_parse_simulators_filters_unavailable() {
        let json = r#"{
            "devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
                    {
                        "udid": "ABC-123",
                        "name": "iPhone 15",
                        "state": "Shutdown",
                        "isAvailable": false
                    }
                ]
            },
            "runtimes": []
        }"#;

        let devices = parse_simulators(json).unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn test_parse_simulators_filters_non_ios() {
        let json = r#"{
            "devices": {
                "com.apple.CoreSimulator.SimRuntime.watchOS-10-0": [
                    {
                        "udid": "ABC-123",
                        "name": "Apple Watch Series 9",
                        "state": "Shutdown",
                        "isAvailable": true
                    }
                ]
            },
            "runtimes": []
        }"#;

        let devices = parse_simulators(json).unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn test_parse_simulators_multiple_runtimes() {
        let json = r#"{
            "devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
                    {
                        "udid": "A1",
                        "name": "iPhone 15",
                        "state": "Shutdown",
                        "isAvailable": true
                    }
                ],
                "com.apple.CoreSimulator.SimRuntime.iOS-16-4": [
                    {
                        "udid": "A2",
                        "name": "iPhone 14",
                        "state": "Shutdown",
                        "isAvailable": true
                    }
                ]
            },
            "runtimes": [
                {
                    "identifier": "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
                    "name": "iOS 17.0",
                    "version": "17.0",
                    "platform": "iOS",
                    "isAvailable": true
                },
                {
                    "identifier": "com.apple.CoreSimulator.SimRuntime.iOS-16-4",
                    "name": "iOS 16.4",
                    "version": "16.4",
                    "platform": "iOS",
                    "isAvailable": true
                }
            ]
        }"#;

        let devices = parse_simulators(json).unwrap();
        assert_eq!(devices.len(), 2);

        // Should be sorted by version descending
        assert_eq!(devices[0].os_version, Some("17.0".to_string()));
        assert_eq!(devices[1].os_version, Some("16.4".to_string()));
    }

    #[test]
    fn test_parse_physical_devices_basic() {
        let json = r#"{
            "info": {},
            "result": {
                "devices": [
                    {
                        "identifier": "D78B0B8B-E5DF-53A8-9190-0EF9B3957ED1",
                        "deviceProperties": {
                            "name": "My iPhone",
                            "osVersionNumber": "17.2"
                        },
                        "hardwareProperties": {
                            "platform": "iOS",
                            "deviceType": "iPhone",
                            "udid": "00008120-001A383914A0C01E",
                            "marketingName": "iPhone 14 Pro"
                        },
                        "connectionProperties": {
                            "tunnelState": "connected"
                        }
                    }
                ]
            }
        }"#;

        let devices = parse_physical_devices(json).unwrap();
        assert_eq!(devices.len(), 1);

        let device = &devices[0];
        assert_eq!(device.id, "D78B0B8B-E5DF-53A8-9190-0EF9B3957ED1");
        assert_eq!(device.name, "My iPhone");
        assert_eq!(device.device_type, DeviceType::Physical);
        assert_eq!(device.os_version, Some("17.2".to_string()));
        assert_eq!(device.state, DeviceState::Available);
    }

    #[test]
    fn test_parse_physical_devices_disconnected() {
        let json = r#"{
            "result": {
                "devices": [
                    {
                        "identifier": "ABC-123",
                        "deviceProperties": {
                            "name": "My iPhone"
                        },
                        "hardwareProperties": {
                            "platform": "iOS",
                            "deviceType": "iPhone",
                            "udid": "DEF-456"
                        },
                        "connectionProperties": {
                            "tunnelState": "disconnected"
                        }
                    }
                ]
            }
        }"#;

        let devices = parse_physical_devices(json).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].state, DeviceState::Unavailable);
    }

    #[test]
    fn test_parse_physical_devices_filters_non_ios() {
        let json = r#"{
            "result": {
                "devices": [
                    {
                        "identifier": "ABC-123",
                        "deviceProperties": {
                            "name": "My Watch"
                        },
                        "hardwareProperties": {
                            "platform": "watchOS",
                            "deviceType": "appleWatch",
                            "udid": "DEF-456"
                        },
                        "connectionProperties": {
                            "tunnelState": "connected"
                        }
                    },
                    {
                        "identifier": "GHI-789",
                        "deviceProperties": {
                            "name": "My iPhone"
                        },
                        "hardwareProperties": {
                            "platform": "iOS",
                            "deviceType": "iPhone",
                            "udid": "JKL-012"
                        },
                        "connectionProperties": {
                            "tunnelState": "connected"
                        }
                    }
                ]
            }
        }"#;

        let devices = parse_physical_devices(json).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "My iPhone");
    }

    #[test]
    fn test_parse_physical_devices_empty() {
        let json = r#"{"result": {"devices": []}}"#;
        let devices = parse_physical_devices(json).unwrap();
        assert!(devices.is_empty());
    }

    // Integration tests - run with: cargo test -- --ignored
    #[test]
    #[ignore]
    fn test_fetch_simulators_real() {
        let devices = super::fetch_simulators();
        println!("Found {} simulators", devices.len());
        for device in devices.iter().take(5) {
            println!(
                "  {} ({:?}) - {:?}",
                device.name, device.os_version, device.state
            );
        }
        assert!(!devices.is_empty(), "Should find at least one simulator");
    }

    #[test]
    #[ignore]
    fn test_fetch_physical_devices_real() {
        let devices = super::fetch_physical_devices();
        println!("Found {} physical devices", devices.len());
        for device in &devices {
            println!(
                "  {} ({:?}) - {:?}",
                device.name, device.os_version, device.state
            );
        }
        // Physical devices may or may not be connected, so we just verify parsing works
    }
}
