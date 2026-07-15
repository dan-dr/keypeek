use super::state::{
    AppConnectionState, AutoConnectState, ConnectionDraft, ConnectionOrigin, UiNotice,
    ZmkTransportDraft,
};
use super::OverlayApp;
use crate::connection::{ConnectedState, ConnectionRequest, ConnectionTask};
use crate::device_discovery::{DeviceKind, DiscoveredDevice};
use crate::protocols::{ConnectionIdentity, ConnectionSpec, ZmkTransportConfig};
use crate::settings::{ConnectionPriority, SavedConnection};
use std::cmp::Reverse;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const AUTO_CONNECT_INTERVAL: Duration = Duration::from_secs(3);
const AUTO_CONNECT_ROUNDS: usize = 5;
const SUCCESS_NOTICE_DURATION: Duration = Duration::from_secs(2);
const FAILURE_NOTICE_DURATION: Duration = Duration::from_secs(6);

enum AutoConnectStep {
    Attempt(SavedConnection),
    Wait(Duration),
    Exhausted,
}

pub(super) fn connected_device_matches(
    identity: &ConnectionIdentity,
    spec: &ConnectionSpec,
    device: &DiscoveredDevice,
) -> bool {
    match identity {
        ConnectionIdentity::Via { vid, pid, .. } => {
            device.kind == DeviceKind::Qmk && device.vid == *vid && device.pid == *pid
        }
        ConnectionIdentity::Vial { .. } => match spec {
            ConnectionSpec::Vial { vid, pid, .. } => {
                device.kind == DeviceKind::Vial && device.vid == *vid && device.pid == *pid
            }
            _ => false,
        },
        ConnectionIdentity::ZmkBle {
            vid,
            pid,
            device_id,
        } => {
            device.kind == DeviceKind::Zmk
                && device.vid == *vid
                && device.pid == *pid
                && device.ble_device_id.as_ref() == Some(device_id)
        }
        ConnectionIdentity::ZmkSerial {
            vid,
            pid,
            device: id,
        } => {
            let transport_matches = match id {
                crate::protocols::ZmkSerialIdentity::SerialNumber(serial) => {
                    device.serial_number.as_ref() == Some(serial)
                }
                crate::protocols::ZmkSerialIdentity::PortName(port) => {
                    device.serial_port.as_ref() == Some(port)
                }
            };
            device.kind == DeviceKind::Zmk
                && device.vid == *vid
                && device.pid == *pid
                && transport_matches
        }
    }
}

fn layout_preference(name: &str) -> Option<String> {
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn next_auto_connect_step(state: &mut AutoConnectState, now: Instant) -> AutoConnectStep {
    if now < state.next_attempt_at {
        return AutoConnectStep::Wait(state.next_attempt_at - now);
    }

    if let Some(candidate) = state.candidates.get(state.next_index).cloned() {
        state.next_index += 1;
        return AutoConnectStep::Attempt(candidate);
    }

    state.round += 1;
    if state.round >= AUTO_CONNECT_ROUNDS {
        return AutoConnectStep::Exhausted;
    }

    state.next_index = 0;
    state.next_attempt_at = now + AUTO_CONNECT_INTERVAL;
    AutoConnectStep::Wait(AUTO_CONNECT_INTERVAL)
}

fn automatic_candidates(
    candidates: Vec<SavedConnection>,
    respect_enabled: bool,
    priority: ConnectionPriority,
) -> Vec<SavedConnection> {
    let mut candidates = if respect_enabled {
        candidates
            .into_iter()
            .filter(|connection| connection.enabled)
            .collect()
    } else {
        candidates
    };
    if priority == ConnectionPriority::LastConnected {
        candidates.sort_by_key(|connection| Reverse(connection.last_connected_at));
    }
    candidates
}

impl OverlayApp {
    fn sync_picker_to_connected(&mut self, identity: &ConnectionIdentity, spec: &ConnectionSpec) {
        self.connect.selected_device_index = self
            .connect
            .available_devices
            .iter()
            .position(|device| connected_device_matches(identity, spec, device));

        self.connect.draft = match spec {
            ConnectionSpec::Via { json_path } => ConnectionDraft::Via {
                json_path: json_path.clone(),
            },
            ConnectionSpec::Vial { .. } => ConnectionDraft::Vial,
            ConnectionSpec::Zmk { transport, .. } => ConnectionDraft::Zmk {
                transport: match transport {
                    ZmkTransportConfig::Serial {
                        port_name,
                        serial_number,
                    } => ZmkTransportDraft::Serial {
                        port_name: Some(port_name.clone()),
                        serial_number: serial_number.clone(),
                    },
                    ZmkTransportConfig::Ble(device_id) => ZmkTransportDraft::Ble {
                        device_id: Some(device_id.clone()),
                    },
                },
            },
        };
    }

    pub(super) fn select_device(&mut self, index: usize) {
        if let Some(device) = self.connect.available_devices.get(index) {
            self.connect.selected_device_index = Some(index);
            self.connect.selected_saved_identity = None;
            self.session.layout_names.clear();
            self.session.active_layout_name.clear();
            self.session.draft_layout_name.clear();

            match device.kind {
                DeviceKind::Zmk => {
                    let transport = if let Some(device_id) = &device.ble_device_id {
                        ZmkTransportDraft::Ble {
                            device_id: Some(device_id.clone()),
                        }
                    } else if let Some(port_name) = &device.serial_port {
                        ZmkTransportDraft::Serial {
                            port_name: Some(port_name.clone()),
                            serial_number: device.serial_number.clone(),
                        }
                    } else {
                        ZmkTransportDraft::Ble { device_id: None }
                    };
                    self.connect.draft = ConnectionDraft::Zmk { transport };
                }
                DeviceKind::Vial => {
                    self.connect.draft = ConnectionDraft::Vial;
                }
                DeviceKind::Qmk => {
                    self.connect.draft = ConnectionDraft::Via {
                        json_path: String::new(),
                    };
                }
            }
            self.ui.settings_error = None;
        }
    }

    fn build_connection_spec(&self) -> Result<ConnectionSpec, String> {
        let selected_device = self
            .connect
            .selected_device_index
            .and_then(|i| self.connect.available_devices.get(i))
            .ok_or_else(|| "No device selected".to_string())?;

        match &self.connect.draft {
            ConnectionDraft::Vial => Ok(ConnectionSpec::Vial {
                vid: selected_device.vid,
                pid: selected_device.pid,
                hid_path: selected_device.hid_path.clone(),
            }),
            ConnectionDraft::Via { json_path } => {
                let path = json_path.trim();
                if path.is_empty() {
                    Err("Please provide a JSON config file path".to_string())
                } else {
                    Ok(ConnectionSpec::Via {
                        json_path: path.to_string(),
                    })
                }
            }
            ConnectionDraft::Zmk { transport } => {
                let transport = match transport {
                    ZmkTransportDraft::Serial {
                        port_name,
                        serial_number,
                    } => {
                        let port = port_name
                            .as_ref()
                            .ok_or_else(|| "No serial port selected for ZMK".to_string())?;
                        ZmkTransportConfig::Serial {
                            port_name: port.clone(),
                            serial_number: serial_number.clone(),
                        }
                    }
                    ZmkTransportDraft::Ble { device_id } => {
                        let id = device_id
                            .as_ref()
                            .ok_or_else(|| "No BLE device selected for ZMK".to_string())?;
                        ZmkTransportConfig::Ble(id.clone())
                    }
                };

                Ok(ConnectionSpec::Zmk {
                    vid: selected_device.vid,
                    pid: selected_device.pid,
                    transport,
                })
            }
        }
    }

    fn selected_device_name(&self) -> Result<String, String> {
        self.connect
            .selected_device_index
            .and_then(|index| self.connect.available_devices.get(index))
            .map(|device| device.base_name.clone())
            .ok_or_else(|| "No device selected".to_string())
    }

    pub(super) fn apply_connected_state(
        &mut self,
        connected: ConnectedState,
        origin: ConnectionOrigin,
    ) {
        let saved_layout = if matches!(connected.spec, ConnectionSpec::Via { .. }) {
            Some(connected.selected_layout_name.clone())
        } else {
            None
        };
        let has_stable_identity = connected.identity.has_stable_identity();
        let saved = SavedConnection {
            enabled: true,
            display_name: connected.display_name.clone(),
            identity: connected.identity.clone(),
            spec: connected.spec.clone(),
            layout_name: saved_layout,
            last_connected_at: unix_timestamp(),
        };

        self.settings.active = self.settings.draft.clone();
        if has_stable_identity {
            self.settings.active.upsert_saved_connection(saved);
        }
        self.settings.draft.saved_connections = self.settings.active.saved_connections.clone();
        self.sync_picker_to_connected(&connected.identity, &connected.spec);
        self.connect.selected_saved_identity =
            has_stable_identity.then(|| connected.identity.clone());
        self.session.current_identity = Some(connected.identity);
        self.session.current_spec = Some(connected.spec);
        self.session.current_display_name = connected.display_name.clone();
        self.session.layout_names = connected.layout_names;
        self.session.active_layout_name = connected.selected_layout_name.clone();
        self.session.draft_layout_name = connected.selected_layout_name;
        self.session.connected_definition = Some(connected.definition);
        self.session.reopen = connected.reopen;
        self.session.connection = AppConnectionState::Connected {
            keyboard: connected.keyboard,
        };
        self.session.ever_connected = true;
        self.connect.auto_connect = None;
        self.ui.settings_error = None;
        self.ui.settings_warning = (!has_stable_identity).then(|| {
            "Connected, but this ZMK serial device has no stable serial number and was not saved."
                .to_string()
        });

        if origin == ConnectionOrigin::Automatic {
            self.ui.notice = Some(UiNotice {
                message: format!("Connected to {}", connected.display_name),
                success: true,
                expires_at: Instant::now() + SUCCESS_NOTICE_DURATION,
            });
        }

        self.persist_settings();
    }

    pub(super) fn persist_settings(&self) {
        if let Err(e) = self.settings.active.save() {
            eprintln!("Failed to save settings: {e}");
        }
    }

    pub(super) fn connect_from_ui(&mut self) {
        if !matches!(self.session.connection, AppConnectionState::Disconnected) {
            self.ui.settings_warning =
                Some("Switching device/protocol/layout requires disconnecting first.".to_string());
            return;
        }

        let saved_selected =
            self.connect
                .selected_saved_identity
                .as_ref()
                .is_some_and(|identity| {
                    self.settings
                        .active
                        .saved_connections
                        .iter()
                        .any(|saved| &saved.identity == identity)
                });

        if !saved_selected && self.connect.selected_device_index.is_none() {
            self.ui.settings_error = Some("No device selected".to_string());
            return;
        }

        if !saved_selected {
            if let ConnectionDraft::Via { json_path } = &self.connect.draft {
                if json_path.trim().is_empty() {
                    self.ui.file_dialog.pick_file();
                    return;
                }
            }
        }

        self.begin_connect_with_current_draft();
    }

    pub(super) fn disconnect_from_ui(&mut self) {
        let previous = std::mem::replace(
            &mut self.session.connection,
            AppConnectionState::Disconnected,
        );
        if let AppConnectionState::Connected { mut keyboard } = previous {
            keyboard.disconnect();
        }
        self.connect.auto_connect = None;
        self.session.current_identity = None;
        self.session.current_spec = None;
        self.session.current_display_name.clear();
        self.session.reopen = None;
        self.session.connected_definition = None;
        self.session.layout_names.clear();
        self.session.active_layout_name.clear();
        self.session.draft_layout_name.clear();
        self.ui.settings_warning = None;
    }

    fn begin_connect_with_current_draft(&mut self) {
        if self.connect.pending_connect.is_some() {
            return;
        }

        if let Some(saved) = self
            .connect
            .selected_saved_identity
            .as_ref()
            .and_then(|identity| {
                self.settings
                    .active
                    .saved_connections
                    .iter()
                    .find(|saved| &saved.identity == identity)
            })
            .cloned()
        {
            let expected_identity = saved.identity.clone();
            self.spawn_connection(
                saved.spec,
                saved.display_name,
                saved.layout_name,
                None,
                Some(expected_identity),
                ConnectionOrigin::Manual,
            );
            self.ui.settings_error = None;
            return;
        }

        let spec = match self.build_connection_spec() {
            Ok(spec) => spec,
            Err(error) => {
                self.ui.settings_error = Some(error);
                return;
            }
        };
        let display_name = match self.selected_device_name() {
            Ok(name) => name,
            Err(error) => {
                self.ui.settings_error = Some(error);
                return;
            }
        };

        let layout_name = layout_preference(&self.session.draft_layout_name);
        self.spawn_connection(
            spec,
            display_name,
            layout_name,
            None,
            None,
            ConnectionOrigin::Manual,
        );
        self.ui.settings_error = None;
    }

    pub(super) fn connect_saved_connection(&mut self, identity: &ConnectionIdentity) {
        if !matches!(self.session.connection, AppConnectionState::Disconnected)
            || self.connect.pending_connect.is_some()
        {
            return;
        }
        let Some(saved) = self
            .settings
            .active
            .saved_connections
            .iter()
            .find(|saved| &saved.identity == identity)
            .cloned()
        else {
            return;
        };

        let expected_identity = saved.identity.clone();
        self.spawn_connection(
            saved.spec,
            saved.display_name,
            saved.layout_name,
            None,
            Some(expected_identity),
            ConnectionOrigin::Manual,
        );
    }

    fn spawn_connection(
        &mut self,
        spec: ConnectionSpec,
        display_name: String,
        layout_name: Option<String>,
        reopen: Option<Arc<dyn crate::protocols::Reopener>>,
        expected_identity: Option<ConnectionIdentity>,
        origin: ConnectionOrigin,
    ) {
        let request = ConnectionRequest {
            spec,
            timeout: self.settings.active.timeout,
            layout_name,
            reopen,
            expected_identity,
            display_name,
        };
        self.connect.pending_connect = Some(ConnectionTask::start(request, self.ui_wake.clone()));
        self.connect.pending_origin = Some(origin);
    }

    fn begin_automatic_connect(
        &mut self,
        candidates: Vec<SavedConnection>,
        reopen_identity: Option<ConnectionIdentity>,
        reopen: Option<Arc<dyn crate::protocols::Reopener>>,
        respect_enabled: bool,
    ) {
        let candidates = automatic_candidates(
            candidates,
            respect_enabled,
            self.settings.active.connection_priority,
        );
        if candidates.is_empty() {
            self.session.connection = AppConnectionState::Disconnected;
            return;
        }

        self.connect.auto_connect = Some(AutoConnectState {
            candidates,
            round: 0,
            next_index: 0,
            next_attempt_at: Instant::now(),
            reopen_identity,
            reopen,
        });
        self.session.connection = AppConnectionState::AutoConnecting;
    }

    pub(super) fn begin_startup_auto_connect(&mut self) {
        if !self.settings.active.auto_connect {
            return;
        }
        self.begin_automatic_connect(
            self.settings.active.saved_connections.clone(),
            None,
            None,
            true,
        );
    }

    fn begin_disconnect_reconnect(&mut self) {
        let identity = self.session.current_identity.clone();
        let reopen = identity
            .as_ref()
            .filter(|identity| identity.supports_cached_reopen())
            .and(self.session.reopen.clone());
        let auto_connect_enabled = self.settings.active.auto_connect;
        let candidates = if auto_connect_enabled {
            self.settings.active.saved_connections.clone()
        } else {
            identity
                .as_ref()
                .and_then(|identity| {
                    self.settings
                        .active
                        .saved_connections
                        .iter()
                        .find(|saved| &saved.identity == identity)
                        .cloned()
                })
                .into_iter()
                .collect()
        };
        self.begin_automatic_connect(candidates, identity, reopen, auto_connect_enabled);
    }

    pub(super) fn maintain_connection(&mut self, ctx: &egui::Context) {
        let dropped = matches!(
            &self.session.connection,
            AppConnectionState::Connected { keyboard } if !keyboard.is_alive()
        );
        if dropped {
            self.begin_disconnect_reconnect();
        }

        if !matches!(self.session.connection, AppConnectionState::AutoConnecting)
            || self.connect.pending_connect.is_some()
        {
            return;
        }

        let now = Instant::now();
        let step = match self.connect.auto_connect.as_mut() {
            Some(state) => next_auto_connect_step(state, now),
            None => {
                self.session.connection = AppConnectionState::Disconnected;
                return;
            }
        };

        match step {
            AutoConnectStep::Attempt(saved) => {
                let reopen = self
                    .connect
                    .auto_connect
                    .as_ref()
                    .filter(|state| state.reopen_identity.as_ref() == Some(&saved.identity))
                    .and_then(|state| state.reopen.clone());
                let expected_identity = saved.identity.clone();
                self.spawn_connection(
                    saved.spec,
                    saved.display_name,
                    saved.layout_name,
                    reopen,
                    Some(expected_identity),
                    ConnectionOrigin::Automatic,
                );
            }
            AutoConnectStep::Wait(delay) => ctx.request_repaint_after(delay),
            AutoConnectStep::Exhausted => {
                self.connect.auto_connect = None;
                self.session.connection = AppConnectionState::Disconnected;
                self.ui.notice = Some(UiNotice {
                    message: "Could not connect to any saved keyboard. Open KeyPeek from the menu bar to connect."
                        .to_string(),
                    success: false,
                    expires_at: now + FAILURE_NOTICE_DURATION,
                });
            }
        }
    }

    pub(super) fn poll_connect_result(&mut self) {
        let Some(task) = self.connect.pending_connect.as_ref() else {
            return;
        };

        match task.try_finish() {
            Some(Ok(connected)) => {
                self.connect.pending_connect = None;
                let origin = self
                    .connect
                    .pending_origin
                    .take()
                    .unwrap_or(ConnectionOrigin::Manual);
                self.apply_connected_state(connected, origin);
            }
            Some(Err(error)) => {
                self.connect.pending_connect = None;
                let origin = self
                    .connect
                    .pending_origin
                    .take()
                    .unwrap_or(ConnectionOrigin::Manual);
                if origin == ConnectionOrigin::Automatic {
                    eprintln!("Automatic connection attempt failed: {error}");
                    if let Some(state) = self.connect.auto_connect.as_mut() {
                        state.next_attempt_at = Instant::now();
                    }
                } else {
                    self.session.connection = AppConnectionState::Disconnected;
                    self.ui.settings_error = Some(error);
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        automatic_candidates, connected_device_matches, next_auto_connect_step, AutoConnectStep,
        AUTO_CONNECT_ROUNDS,
    };
    use crate::device_discovery::{DeviceKind, DiscoveredDevice};
    use crate::overlay_window::state::AutoConnectState;
    use crate::protocols::{ConnectionIdentity, ConnectionSpec};
    use crate::settings::{ConnectionPriority, SavedConnection};
    use std::time::Instant;

    fn candidate(name: &str, path: &str) -> SavedConnection {
        SavedConnection {
            enabled: true,
            display_name: name.to_string(),
            identity: ConnectionIdentity::Via {
                vid: 1,
                pid: 2,
                json_path: path.to_string(),
            },
            spec: ConnectionSpec::Via {
                json_path: path.to_string(),
            },
            layout_name: None,
            last_connected_at: 0,
        }
    }

    #[test]
    fn round_robin_attempts_every_candidate_for_five_rounds() {
        let now = Instant::now();
        let mut state = AutoConnectState {
            candidates: vec![
                candidate("Home", "/home.json"),
                candidate("Office", "/office.json"),
            ],
            round: 0,
            next_index: 0,
            next_attempt_at: now,
            reopen_identity: None,
            reopen: None,
        };
        let mut attempted = Vec::new();

        loop {
            let attempt_at = state.next_attempt_at;
            match next_auto_connect_step(&mut state, attempt_at) {
                AutoConnectStep::Attempt(saved) => attempted.push(saved.display_name),
                AutoConnectStep::Wait(_) => {}
                AutoConnectStep::Exhausted => break,
            }
        }

        assert_eq!(attempted.len(), AUTO_CONNECT_ROUNDS * 2);
        assert_eq!(&attempted[..4], ["Home", "Office", "Home", "Office"]);
    }

    #[test]
    fn successful_saved_connection_matches_exact_discovered_device() {
        let identity = ConnectionIdentity::ZmkBle {
            vid: 0x1234,
            pid: 0x5678,
            device_id: "home-id".to_string(),
        };
        let spec = ConnectionSpec::Zmk {
            vid: 0x1234,
            pid: 0x5678,
            transport: crate::protocols::ZmkTransportConfig::Ble("home-id".to_string()),
        };
        let device = |device_id: &str| DiscoveredDevice {
            base_name: "Sofle".to_string(),
            vid: 0x1234,
            pid: 0x5678,
            serial_port: None,
            serial_number: None,
            ble_device_id: Some(device_id.to_string()),
            hid_path: None,
            kind: DeviceKind::Zmk,
        };

        assert!(connected_device_matches(
            &identity,
            &spec,
            &device("home-id")
        ));
        assert!(!connected_device_matches(
            &identity,
            &spec,
            &device("office-id")
        ));
    }

    #[test]
    fn active_disabled_connection_remains_a_reconnect_candidate() {
        let mut disabled = candidate("Home", "/home.json");
        disabled.enabled = false;

        assert!(
            automatic_candidates(vec![disabled.clone()], true, ConnectionPriority::Manual)
                .is_empty()
        );
        assert_eq!(
            automatic_candidates(vec![disabled], false, ConnectionPriority::Manual).len(),
            1
        );
    }

    #[test]
    fn last_connected_priority_sorts_manual_order_at_attempt_time() {
        let mut older = candidate("Office", "/office.json");
        older.last_connected_at = 10;
        let mut newer = candidate("Home", "/home.json");
        newer.last_connected_at = 20;

        let sorted =
            automatic_candidates(vec![older, newer], true, ConnectionPriority::LastConnected);

        assert_eq!(sorted[0].display_name, "Home");
    }
}
