pub mod kle_parser;
pub mod layout_geometry;
pub mod qmk_json_parser;
pub mod via;
pub mod vial;
pub mod zmk;
pub mod zmk_rpc;

use crate::layout_key::LayoutKey;
use hidapi::{HidApi, HidDevice};
use qmk_via_api::api::KeyboardApi;
use std::error::Error;
use std::ffi::CStr;
use std::sync::Arc;

use self::via::ViaProtocol;
use self::vial::VialProtocol;
use self::zmk::ZmkProtocol;

pub use self::zmk_rpc::DeviceLocked;

pub const KEYPEEK_SUBSCRIBE_MARKER: u8 = 0xC0;
pub const KEYPEEK_SUBSCRIBE_ACTIVE: u8 = 0xA1;
pub const KEYPEEK_SUBSCRIBE_INACTIVE: u8 = 0xA0;

pub type Row = usize;
pub type Column = usize;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Key {
    pub row: Row,
    pub col: Column,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Rotation angle in degrees, clockwise around the key's center.
    #[serde(default)]
    pub r: f32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct KeyboardLayout {
    pub name: String,
    pub keys: Vec<Key>,
}

impl KeyboardLayout {
    pub fn get_dimensions(&self) -> (f32, f32) {
        let max_x = self.keys.iter().map(|k| k.x + k.w).fold(0.0, f32::max);
        let max_y = self.keys.iter().map(|k| k.y + k.h).fold(0.0, f32::max);
        (max_x, max_y)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct KeyboardDefinition {
    pub vid: u16,
    pub pid: u16,
    pub rows: usize,
    pub cols: usize,
    pub layouts: Vec<KeyboardLayout>,
}

impl KeyboardDefinition {
    pub fn get_layout_names(&self) -> Vec<String> {
        self.layouts.iter().map(|l| l.name.clone()).collect()
    }

    pub fn get_layout(&self, layout_name: &str) -> Result<KeyboardLayout, String> {
        self.layouts
            .iter()
            .find(|l| l.name == layout_name)
            .cloned()
            .ok_or_else(|| format!("Layout '{}' not found.", layout_name))
    }
}

pub trait KeyboardProtocol: Send {
    fn get_layout_definition(&self) -> &KeyboardDefinition;

    fn get_layer_count(&self) -> Result<usize, Box<dyn Error>>;

    fn read_all_keys(
        &self,
        layers: usize,
        rows: usize,
        cols: usize,
    ) -> Vec<Vec<Vec<Option<LayoutKey>>>>;

    fn hid_read(&self) -> Result<Vec<u8>, Box<dyn Error>>;

    fn subscription_sender(&self) -> Option<Box<dyn SubscriptionSender>> {
        None
    }

    fn reopener(&self) -> Option<Arc<dyn Reopener>> {
        None
    }
}

pub trait Reopener: Send + Sync {
    fn reopen(&self) -> Result<Box<dyn KeyboardProtocol>, Box<dyn Error>>;
}

pub trait SubscriptionSender: Send {
    fn set_active(&self, active: bool) -> Result<(), Box<dyn Error>>;
}

pub struct RawHidSubscription {
    handle: RawHidSubscriptionHandle,
}

enum RawHidSubscriptionHandle {
    Api(KeyboardApi),
    Device(HidDevice),
}

impl RawHidSubscription {
    pub fn open(vid: u16, pid: u16) -> Option<Box<dyn SubscriptionSender>> {
        KeyboardApi::new(vid, pid, 0xff60, None).ok().map(|api| {
            Box::new(Self {
                handle: RawHidSubscriptionHandle::Api(api),
            }) as Box<dyn SubscriptionSender>
        })
    }

    pub fn open_path(path: &CStr) -> Option<Box<dyn SubscriptionSender>> {
        let api = HidApi::new().ok()?;
        let device = api.open_path(path).ok()?;
        Some(Box::new(Self {
            handle: RawHidSubscriptionHandle::Device(device),
        }))
    }
}

impl SubscriptionSender for RawHidSubscription {
    fn set_active(&self, active: bool) -> Result<(), Box<dyn Error>> {
        let value = if active {
            KEYPEEK_SUBSCRIBE_ACTIVE
        } else {
            KEYPEEK_SUBSCRIBE_INACTIVE
        };
        match &self.handle {
            RawHidSubscriptionHandle::Api(api) => api
                .hid_send(vec![KEYPEEK_SUBSCRIBE_MARKER, value])
                .map_err(|e| format!("Subscription keepalive write error: {e}").into()),
            RawHidSubscriptionHandle::Device(device) => {
                let mut report = vec![0; 33];
                report[1] = KEYPEEK_SUBSCRIBE_MARKER;
                report[2] = value;
                let written = device.write(&report)?;
                if written != report.len() {
                    return Err(format!(
                        "Subscription keepalive wrote {written} of {} bytes",
                        report.len()
                    )
                    .into());
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ZmkTransportConfig {
    Serial {
        port_name: String,
        serial_number: Option<String>,
    },
    Ble(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConnectionSpec {
    Via {
        json_path: String,
    },
    Vial {
        vid: u16,
        pid: u16,
        #[serde(default)]
        hid_path: Option<Vec<u8>>,
    },
    Zmk {
        vid: u16,
        pid: u16,
        transport: ZmkTransportConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ZmkSerialIdentity {
    SerialNumber(String),
    PortName(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ConnectionIdentity {
    Via {
        vid: u16,
        pid: u16,
        json_path: String,
    },
    Vial {
        keyboard_uid: [u8; 8],
    },
    ZmkBle {
        vid: u16,
        pid: u16,
        device_id: String,
    },
    ZmkSerial {
        vid: u16,
        pid: u16,
        device: ZmkSerialIdentity,
    },
}

impl ConnectionIdentity {
    pub fn has_stable_identity(&self) -> bool {
        !matches!(
            self,
            Self::ZmkSerial {
                device: ZmkSerialIdentity::PortName(_),
                ..
            }
        )
    }

    pub fn supports_cached_reopen(&self) -> bool {
        matches!(
            self,
            Self::ZmkSerial {
                device: ZmkSerialIdentity::SerialNumber(_),
                ..
            }
        )
    }
}

pub type ConnectedProtocol = (
    Box<dyn KeyboardProtocol>,
    ConnectionIdentity,
    ConnectionSpec,
);

pub fn connect_protocol(
    spec: &ConnectionSpec,
    expected_identity: Option<&ConnectionIdentity>,
) -> Result<ConnectedProtocol, Box<dyn Error>> {
    match spec {
        ConnectionSpec::Via { json_path } => {
            let protocol = ViaProtocol::connect(json_path)?;
            let definition = protocol.get_layout_definition();
            let canonical_path = std::fs::canonicalize(json_path)?;
            let canonical_path = canonical_path.to_string_lossy().into_owned();
            let identity = ConnectionIdentity::Via {
                vid: definition.vid,
                pid: definition.pid,
                json_path: canonical_path.clone(),
            };
            validate_expected_identity(expected_identity, &identity)?;
            Ok((
                Box::new(protocol),
                identity,
                ConnectionSpec::Via {
                    json_path: canonical_path,
                },
            ))
        }
        ConnectionSpec::Vial { vid, pid, hid_path } => {
            let expected_uid = match expected_identity {
                Some(ConnectionIdentity::Vial { keyboard_uid }) => Some(*keyboard_uid),
                _ => None,
            };
            let protocol =
                VialProtocol::connect_expected(*vid, *pid, expected_uid, hid_path.as_deref())?;
            let identity = ConnectionIdentity::Vial {
                keyboard_uid: protocol.keyboard_uid(),
            };
            validate_expected_identity(expected_identity, &identity)?;
            Ok((
                Box::new(protocol),
                identity,
                ConnectionSpec::Vial {
                    vid: *vid,
                    pid: *pid,
                    hid_path: None,
                },
            ))
        }
        ConnectionSpec::Zmk {
            vid,
            pid,
            transport,
        } => {
            let (zmk_transport, identity, normalized_spec) = match transport {
                ZmkTransportConfig::Serial {
                    port_name,
                    serial_number,
                } => {
                    if serial_number.is_none() && expected_identity.is_some() {
                        return Err(
                            "Cannot reconnect this saved ZMK serial connection because the keyboard has no stable serial number"
                                .into(),
                        );
                    }
                    let ports = zmk_rpc::scan_serial_ports();
                    let resolved_port = if let Some(serial_number) = serial_number {
                        ports
                            .iter()
                            .find(|port| {
                                port.vid == *vid
                                    && port.pid == *pid
                                    && port.serial_number.as_ref() == Some(serial_number)
                            })
                            .map(|port| port.port_name.clone())
                            .ok_or_else(|| {
                                format!(
                                    "Could not find saved ZMK keyboard with serial number {serial_number}"
                                )
                            })?
                    } else {
                        ports
                            .iter()
                            .find(|port| {
                                port.vid == *vid && port.pid == *pid && port.port_name == *port_name
                            })
                            .map(|port| port.port_name.clone())
                            .ok_or_else(|| {
                                format!(
                                    "Saved ZMK serial port {port_name} is not currently available"
                                )
                            })?
                    };
                    let device = serial_number
                        .clone()
                        .map(ZmkSerialIdentity::SerialNumber)
                        .unwrap_or_else(|| ZmkSerialIdentity::PortName(resolved_port.clone()));
                    (
                        zmk_rpc::ZmkTransport::SerialPort(resolved_port.clone()),
                        ConnectionIdentity::ZmkSerial {
                            vid: *vid,
                            pid: *pid,
                            device,
                        },
                        ConnectionSpec::Zmk {
                            vid: *vid,
                            pid: *pid,
                            transport: ZmkTransportConfig::Serial {
                                port_name: resolved_port,
                                serial_number: serial_number.clone(),
                            },
                        },
                    )
                }
                ZmkTransportConfig::Ble(device_id) => (
                    zmk_rpc::ZmkTransport::BleDevice(device_id.clone()),
                    ConnectionIdentity::ZmkBle {
                        vid: *vid,
                        pid: *pid,
                        device_id: device_id.clone(),
                    },
                    spec.clone(),
                ),
            };
            let hid_serial_number = match &identity {
                ConnectionIdentity::ZmkSerial {
                    device: ZmkSerialIdentity::SerialNumber(serial_number),
                    ..
                } => Some(serial_number.clone()),
                _ => None,
            };
            let protocol =
                ZmkProtocol::connect_live(*vid, *pid, &zmk_transport, hid_serial_number)?;
            validate_expected_identity(expected_identity, &identity)?;
            Ok((Box::new(protocol), identity, normalized_spec))
        }
    }
}

fn validate_expected_identity(
    expected: Option<&ConnectionIdentity>,
    observed: &ConnectionIdentity,
) -> Result<(), Box<dyn Error>> {
    if let Some(expected) = expected {
        if expected != observed {
            return Err(format!(
                "Connected keyboard identity does not match the saved connection: expected {expected:?}, observed {observed:?}"
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ConnectionIdentity, ZmkSerialIdentity};

    #[test]
    fn cached_reopen_requires_verified_zmk_serial_number() {
        let serial = ConnectionIdentity::ZmkSerial {
            vid: 1,
            pid: 2,
            device: ZmkSerialIdentity::SerialNumber("exact".to_string()),
        };
        let port = ConnectionIdentity::ZmkSerial {
            vid: 1,
            pid: 2,
            device: ZmkSerialIdentity::PortName("/dev/tty.test".to_string()),
        };
        let ble = ConnectionIdentity::ZmkBle {
            vid: 1,
            pid: 2,
            device_id: "exact".to_string(),
        };

        assert!(serial.supports_cached_reopen());
        assert!(!port.supports_cached_reopen());
        assert!(!ble.supports_cached_reopen());
        assert!(serial.has_stable_identity());
        assert!(!port.has_stable_identity());
        assert!(ble.has_stable_identity());
    }
}
