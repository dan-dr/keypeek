use super::state::AppConnectionState;
use super::OverlayApp;
use crate::protocols::{ConnectionSpec, ZmkTransportConfig};
use crate::settings::{ConnectionPriority, SavedConnection, WindowPosition, ALL_LAYERS_MASK};
use egui::Window;

fn settings_window_size(viewport_size: egui::Vec2) -> egui::Vec2 {
    const WIDTH: f32 = 520.0;
    const MAX_HEIGHT: f32 = 1_000.0;
    const HORIZONTAL_MARGIN: f32 = 80.0;
    const VERTICAL_MARGIN: f32 = 96.0;

    egui::vec2(
        WIDTH.min((viewport_size.x - HORIZONTAL_MARGIN).max(1.0)),
        MAX_HEIGHT.min((viewport_size.y - VERTICAL_MARGIN).max(1.0)),
    )
}

impl OverlayApp {
    fn connection_details(connection: &SavedConnection) -> (String, Option<String>) {
        match &connection.spec {
            ConnectionSpec::Via { json_path } => (
                format!(
                    "QMK · {}",
                    connection
                        .layout_name
                        .as_deref()
                        .unwrap_or("default layout")
                ),
                Some(json_path.clone()),
            ),
            ConnectionSpec::Vial { .. } => ("Vial".to_string(), None),
            ConnectionSpec::Zmk { transport, .. } => (
                match transport {
                    ZmkTransportConfig::Ble(_) => "ZMK BLE",
                    ZmkTransportConfig::Serial { .. } => "ZMK Serial",
                }
                .to_string(),
                None,
            ),
        }
    }

    fn draw_saved_connections(&mut self, ui: &mut egui::Ui, connection_locked: bool) {
        if self.settings.draft.saved_connections.is_empty() {
            return;
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Saved connections").strong());
        ui.add_space(4.0);

        let manual_order = self.settings.draft.connection_priority == ConnectionPriority::Manual;
        let mut connect_identity = None;
        let mut remove_index = None;
        let mut drop_target = None;

        for index in 0..self.settings.draft.saved_connections.len() {
            let connection = self.settings.draft.saved_connections[index].clone();
            let identity = connection.identity.clone();
            let (details, secondary) = Self::connection_details(&connection);
            let discovered = self.connect.available_devices.iter().any(|device| {
                super::connection_flow::connected_device_matches(
                    &connection.identity,
                    &connection.spec,
                    device,
                )
            });
            let row = ui.horizontal(|ui| {
                let drag = ui.add_enabled(
                    manual_order,
                    egui::Label::new(egui::RichText::new("⠿").weak()).sense(egui::Sense::drag()),
                );
                if drag.drag_started() {
                    self.ui.dragged_connection = Some(identity.clone());
                }

                let mut enabled = connection.enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    self.settings.draft.saved_connections[index].enabled = enabled;
                }

                ui.vertical(|ui| {
                    ui.set_min_width(155.0);
                    let name = egui::RichText::new(&connection.display_name);
                    let details = egui::RichText::new(details).small();
                    if discovered {
                        ui.label(name);
                        ui.label(details);
                    } else {
                        ui.label(name.weak());
                        ui.label(details.weak());
                    }
                    if let Some(secondary) = secondary {
                        ui.label(egui::RichText::new(secondary).weak().small());
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!connection_locked, egui::Button::new("Remove"))
                        .clicked()
                    {
                        remove_index = Some(index);
                    }
                    if ui
                        .add_enabled(
                            !connection_locked && discovered,
                            egui::Button::new("Connect"),
                        )
                        .on_disabled_hover_text("Keyboard is not currently discovered")
                        .clicked()
                    {
                        connect_identity = Some(identity.clone());
                    }
                });
            });

            if manual_order
                && self.ui.dragged_connection.is_some()
                && row.response.hovered()
                && ui.input(|input| input.pointer.any_released())
            {
                drop_target = Some(index);
            }
            if index + 1 < self.settings.draft.saved_connections.len() {
                ui.separator();
            }
        }

        if let Some(target) = drop_target {
            if let Some(dragged) = self.ui.dragged_connection.as_ref() {
                if let Some(source) = self
                    .settings
                    .draft
                    .saved_connections
                    .iter()
                    .position(|connection| &connection.identity == dragged)
                {
                    let connection = self.settings.draft.saved_connections.remove(source);
                    let target = target.min(self.settings.draft.saved_connections.len());
                    self.settings
                        .draft
                        .saved_connections
                        .insert(target, connection);
                }
            }
        }
        if ui.input(|input| input.pointer.any_released()) {
            self.ui.dragged_connection = None;
        }
        if let Some(index) = remove_index {
            let removed = self.settings.draft.saved_connections.remove(index);
            if self.connect.selected_saved_identity.as_ref() == Some(&removed.identity) {
                self.connect.selected_saved_identity = None;
            }
        }
        if let Some(identity) = connect_identity {
            self.connect_saved_connection(&identity);
        }
    }

    fn draw_connection_group(&mut self, ui: &mut egui::Ui) {
        let auto_connecting = matches!(self.session.connection, AppConnectionState::AutoConnecting);
        let connected = matches!(
            self.session.connection,
            AppConnectionState::Connected { .. }
        );
        let connection_locked =
            !matches!(self.session.connection, AppConnectionState::Disconnected);
        let selected_device_text = self
            .connect
            .selected_device_index
            .and_then(|index| self.connect.available_devices.get(index))
            .map(|device| device.display_name())
            .unwrap_or_else(|| "Select device...".to_string());

        ui.heading("Connection");
        ui.add_space(8.0);
        let control_spacing = ui.spacing().item_spacing.x;
        const RIGHT_COLUMN_WIDTH: f32 = 100.0;

        egui::Grid::new("connection_grid")
            .num_columns(2)
            .striped(true)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                ui.label("Device");
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!connection_locked, |ui| {
                        let combo_width = (ui.available_width()
                            - RIGHT_COLUMN_WIDTH
                            - 28.0
                            - control_spacing * 2.0)
                            .max(100.0);
                        egui::ComboBox::from_id_salt("device_combo")
                            .width(combo_width)
                            .selected_text(selected_device_text)
                            .show_ui(ui, |ui| {
                                for index in 0..self.connect.available_devices.len() {
                                    let device = &self.connect.available_devices[index];
                                    let selected =
                                        self.connect.selected_device_index == Some(index);
                                    if ui
                                        .selectable_label(selected, device.display_name())
                                        .clicked()
                                    {
                                        self.select_device(index);
                                    }
                                }
                                if self.connect.available_devices.is_empty() {
                                    ui.weak("No devices found");
                                }
                            });
                    });

                    let refreshing = self.connect.discovery_task.is_some();
                    if ui
                        .add_enabled(!refreshing, egui::Button::new("↻"))
                        .on_hover_text("Refresh device discovery")
                        .clicked()
                    {
                        self.request_device_refresh();
                    }

                    let connect_in_progress = self.connect.pending_connect.is_some();
                    let (label, action_enabled) = if connected {
                        ("Disconnect", true)
                    } else if auto_connecting || connect_in_progress {
                        ("Connecting...", false)
                    } else {
                        (
                            "Connect",
                            self.connect.selected_device_index.is_some()
                                || self.connect.selected_saved_identity.is_some(),
                        )
                    };
                    if ui
                        .add_enabled(
                            action_enabled,
                            egui::Button::new(label).min_size([RIGHT_COLUMN_WIDTH, 20.0].into()),
                        )
                        .clicked()
                    {
                        if connected {
                            self.disconnect_from_ui();
                        } else {
                            self.connect_from_ui();
                        }
                    }
                });
                ui.end_row();

                ui.label("Layout");
                ui.horizontal(|ui| {
                    let layout_enabled = !self.session.layout_names.is_empty();
                    let selected_text = if layout_enabled {
                        self.session.draft_layout_name.clone()
                    } else {
                        "Connect to device first".to_string()
                    };
                    ui.add_enabled_ui(layout_enabled, |ui| {
                        egui::ComboBox::from_id_salt("layout_combo")
                            .width(
                                (ui.available_width() - RIGHT_COLUMN_WIDTH - control_spacing)
                                    .max(120.0),
                            )
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for name in &self.session.layout_names {
                                    ui.selectable_value(
                                        &mut self.session.draft_layout_name,
                                        name.clone(),
                                        name,
                                    );
                                }
                            });
                    });
                    ui.allocate_space(egui::vec2(RIGHT_COLUMN_WIDTH, 20.0));
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.checkbox(&mut self.settings.draft.auto_connect, "Auto-connect");
        ui.add_enabled_ui(self.settings.draft.auto_connect, |ui| {
            ui.horizontal(|ui| {
                ui.label("Priority:");
                ui.radio_value(
                    &mut self.settings.draft.connection_priority,
                    ConnectionPriority::LastConnected,
                    "Last connected",
                );
                ui.radio_value(
                    &mut self.settings.draft.connection_priority,
                    ConnectionPriority::Manual,
                    "Manual",
                );
            });
        });

        self.draw_saved_connections(ui, connection_locked);
        if self.settings.draft.auto_connect && !self.settings.draft.saved_connections.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "Tries each enabled connection in priority order, waits 3 seconds, and repeats for 5 rounds.",
                )
                .weak()
                .small(),
            );
        }
    }

    fn draw_start_on_login(&mut self, ui: &mut egui::Ui) {
        if !self.startup_status.is_available() {
            return;
        }
        let mut enabled = self.startup_status.is_enabled();
        if ui
            .checkbox(&mut enabled, "Start KeyPeek on login")
            .changed()
        {
            match crate::platform::startup::set_enabled(enabled) {
                Ok(status) => self.startup_status = status,
                Err(error) => self.ui.settings_error = Some(error),
            }
        }
        if self.startup_status == crate::platform::startup::StartupStatus::RequiresApproval {
            ui.label(
                egui::RichText::new("Approval required in the system Login Items settings.")
                    .weak()
                    .small(),
            );
        }
    }

    fn draw_layer_visibility(&mut self, ui: &mut egui::Ui) {
        ui.heading("Layer Visibility");
        ui.add_space(8.0);

        let layer_count = match &self.session.connection {
            AppConnectionState::Connected { keyboard } => keyboard.get_num_layers(),
            AppConnectionState::Disconnected | AppConnectionState::AutoConnecting => {
                ui.label(
                    egui::RichText::new("Connect a keyboard to choose its visible layers.").weak(),
                );
                return;
            }
        };
        let Some(identity) = self.session.current_identity.as_ref() else {
            ui.label(egui::RichText::new("This connection cannot save layer choices.").weak());
            return;
        };
        let Some(connection) = self
            .settings
            .draft
            .saved_connections
            .iter_mut()
            .find(|connection| &connection.identity == identity)
        else {
            ui.label(egui::RichText::new("This connection cannot save layer choices.").weak());
            return;
        };

        ui.horizontal(|ui| {
            ui.label("Show:");
            if ui.small_button("All").clicked() {
                connection.visible_layers = ALL_LAYERS_MASK;
            }
            if ui.small_button("None").clicked() {
                connection.visible_layers = 0;
            }
        });
        ui.add_space(4.0);

        egui::Grid::new("visible_layers_grid")
            .num_columns(4)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                for layer in 0..layer_count {
                    let layer_mask = 1u32 << layer;
                    let mut visible = connection.visible_layers & layer_mask != 0;
                    if ui
                        .checkbox(&mut visible, format!("Layer {layer}"))
                        .changed()
                    {
                        if visible {
                            connection.visible_layers |= layer_mask;
                        } else {
                            connection.visible_layers &= !layer_mask;
                        }
                    }
                    if layer % 4 == 3 {
                        ui.end_row();
                    }
                }
                if layer_count % 4 != 0 {
                    ui.end_row();
                }
            });

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Unchecked layers do not open the overlay or override a selected layer.",
            )
            .weak()
            .small(),
        );
    }

    fn timeout_to_ui_value(timeout: i64) -> u32 {
        if timeout < 0 {
            15_000
        } else {
            (timeout as u32).min(14_999)
        }
    }

    fn ui_value_to_timeout(value: u32) -> i64 {
        if value >= 15_000 {
            -1
        } else {
            value as i64
        }
    }

    pub(super) fn draw_settings_window(
        &mut self,
        ctx: &egui::Context,
        host: &mut dyn crate::platform::OverlayHost,
    ) {
        let mut open = self.ui.settings_visible;
        let settings_window_size = settings_window_size(ctx.viewport_rect().size());

        Window::new("KeyPeek Settings")
            .open(&mut open)
            .fixed_size(settings_window_size)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.group(|ui| {
                            self.draw_connection_group(ui);
                        });

                        ui.add_space(10.0);

                        ui.group(|ui| {
                            self.draw_layer_visibility(ui);
                        });

                        ui.add_space(10.0);

                        ui.group(|ui| {
                            ui.heading("Overlay Appearance");
                            ui.add_space(8.0);

                            egui::Grid::new("appearance_grid")
                                .num_columns(2)
                                .striped(true)
                                .spacing([20.0, 10.0])
                                .show(ui, |ui| {
                                    ui.label("Alignment");
                                    egui::ComboBox::from_id_salt("position_combo")
                                        .width(ui.available_width())
                                        .selected_text(self.settings.draft.position.to_string())
                                        .show_ui(ui, |ui| {
                                            for pos in [
                                                WindowPosition::TopLeft,
                                                WindowPosition::TopRight,
                                                WindowPosition::BottomLeft,
                                                WindowPosition::BottomRight,
                                                WindowPosition::Top,
                                                WindowPosition::Bottom,
                                            ] {
                                                ui.selectable_value(
                                                    &mut self.settings.draft.position,
                                                    pos,
                                                    pos.to_string(),
                                                );
                                            }
                                        });
                                    ui.end_row();

                                    ui.label("Display duration");
                                    let mut timeout_ui =
                                        Self::timeout_to_ui_value(self.settings.draft.timeout);
                                    ui.add_sized(
                                        ui.available_size(),
                                        egui::DragValue::new(&mut timeout_ui)
                                            .speed(50)
                                            .range(0..=15_000)
                                            .custom_formatter(|value, _range| {
                                                if value >= 15_000.0 {
                                                    "∞".to_string()
                                                } else {
                                                    format!("{} ms", value as i64)
                                                }
                                            }),
                                    );
                                    self.settings.draft.timeout =
                                        Self::ui_value_to_timeout(timeout_ui);
                                    ui.end_row();

                                    ui.label("Distance from screen edge");
                                    ui.add_sized(
                                        ui.available_size(),
                                        egui::DragValue::new(&mut self.settings.draft.margin)
                                            .speed(1)
                                            .suffix(" px"),
                                    );
                                    ui.end_row();

                                    ui.label("Key unit size");
                                    ui.add_sized(
                                        ui.available_size(),
                                        egui::DragValue::new(&mut self.settings.draft.size)
                                            .speed(1)
                                            .range(20..=1000)
                                            .suffix(" px"),
                                    );
                                    ui.end_row();

                                    ui.label("Key label font scale");
                                    ui.add_sized(
                                        ui.available_size(),
                                        egui::DragValue::new(
                                            &mut self.settings.draft.font_size_multiplier,
                                        )
                                        .speed(0.01)
                                        .range(0.5..=1.5)
                                        .suffix(" x"),
                                    );
                                    ui.end_row();

                                    ui.label("Auto-fit long labels");
                                    ui.checkbox(
                                        &mut self.settings.draft.auto_fit_before_ellipsis,
                                        "Fit long labels to available space",
                                    );
                                    ui.end_row();
                                });
                        });

                        ui.add_space(10.0);

                        ui.group(|ui| {
                            ui.heading("Theme");
                            ui.add_space(8.0);

                            ui.columns(2, |columns| {
                                columns[0].vertical(|ui| {
                                    Self::theme_color_entry(
                                        ui,
                                        "Font color",
                                        &mut self.settings.draft.theme.font_color,
                                    );
                                    Self::theme_color_entry(
                                        ui,
                                        "Layer 0 color",
                                        &mut self.settings.draft.theme.layer_colors[0],
                                    );
                                    Self::theme_color_entry(
                                        ui,
                                        "Layer 1 color",
                                        &mut self.settings.draft.theme.layer_colors[1],
                                    );
                                    Self::theme_color_entry(
                                        ui,
                                        "Layer 2 color",
                                        &mut self.settings.draft.theme.layer_colors[2],
                                    );
                                });

                                columns[1].vertical(|ui| {
                                    Self::theme_color_entry(
                                        ui,
                                        "Layer 3 color",
                                        &mut self.settings.draft.theme.layer_colors[3],
                                    );
                                    Self::theme_color_entry(
                                        ui,
                                        "Layer 4 color",
                                        &mut self.settings.draft.theme.layer_colors[4],
                                    );
                                    Self::theme_color_entry(
                                        ui,
                                        "Layer 5 color",
                                        &mut self.settings.draft.theme.layer_colors[5],
                                    );
                                    Self::theme_color_entry(
                                        ui,
                                        "Other layers color",
                                        &mut self.settings.draft.theme.layer_colors[6],
                                    );
                                });
                            });
                        });

                        ui.add_space(8.0);
                        self.draw_start_on_login(ui);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::Hyperlink::from_label_and_url(
                                egui::RichText::new("github.com/srwi/keypeek").weak(),
                                "https://github.com/srwi/keypeek",
                            ));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.add(egui::Hyperlink::from_label_and_url(
                                        egui::RichText::new(format!(
                                            "Version {}",
                                            env!("CARGO_PKG_VERSION")
                                        ))
                                        .weak(),
                                        "https://github.com/srwi/keypeek/releases",
                                    ));
                                },
                            );
                        });
                    });
            });

        if self.ui.settings_visible && !open {
            self.ui.settings_visible = false;
            if !self.session.ever_connected {
                host.request_close();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::settings_window_size;

    #[test]
    fn settings_window_uses_available_desktop_height() {
        assert_eq!(
            settings_window_size(egui::vec2(1_440.0, 900.0)),
            egui::vec2(520.0, 804.0)
        );
    }

    #[test]
    fn settings_window_caps_height_on_large_displays() {
        assert_eq!(
            settings_window_size(egui::vec2(3_840.0, 2_160.0)),
            egui::vec2(520.0, 1_000.0)
        );
    }
}
