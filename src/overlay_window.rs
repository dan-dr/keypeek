use crate::device_discovery::{DiscoveredDevice, DiscoveryTask};
use crate::platform::{OverlayHost, WindowFrame};
use crate::settings::{Settings, WindowPosition};
use crate::ui_wake::UiWake;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

mod connection_flow;
mod lifecycle;
mod settings_sync;
mod state;
mod ui_overlay;
mod ui_settings;
use state::{
    AppConnectionState, ConnectDraftState, ConnectionDraft, SessionState, SettingsState, UiState,
};

fn overlay_content_frame(
    position: WindowPosition,
    margin: f32,
    monitor_size: egui::Vec2,
    content_size: egui::Vec2,
) -> WindowFrame {
    let left = margin;
    let right = (monitor_size.x - content_size.x - margin).max(0.0);
    let top = margin;
    let bottom = (monitor_size.y - content_size.y - margin).max(0.0);
    let center_x = ((monitor_size.x - content_size.x) * 0.5).max(0.0);

    let position = match position {
        WindowPosition::TopLeft => egui::pos2(left, top),
        WindowPosition::TopRight => egui::pos2(right, top),
        WindowPosition::BottomLeft => egui::pos2(left, bottom),
        WindowPosition::BottomRight => egui::pos2(right, bottom),
        WindowPosition::Bottom => egui::pos2(center_x, bottom),
        WindowPosition::Top => egui::pos2(center_x, top),
    };

    WindowFrame::Content {
        position,
        size: content_size,
    }
}

pub struct OverlayApp {
    _tray: crate::tray::Tray,
    settings_requested: Arc<AtomicBool>,
    ui_wake: UiWake,
    ui: UiState,
    settings: SettingsState,
    session: SessionState,
    connect: ConnectDraftState,
    resume_monitor: crate::platform::resume::ResumeMonitor,
    startup_status: crate::platform::startup::StartupStatus,
}

impl OverlayApp {
    pub fn new(
        tray: crate::tray::Tray,
        settings_requested: Arc<AtomicBool>,
        ui_wake: UiWake,
        resume_monitor: crate::platform::resume::ResumeMonitor,
        base_settings: Settings,
        available_devices: Vec<DiscoveredDevice>,
    ) -> Self {
        let auto_connect_at_start = base_settings.auto_connect
            && base_settings
                .saved_connections
                .iter()
                .any(|connection| connection.enabled);
        let mut app = Self {
            _tray: tray,
            settings_requested,
            ui_wake,
            ui: UiState {
                settings_visible: !auto_connect_at_start,
                settings_error: None,
                settings_warning: None,
                mouse_passthrough: None,
                file_dialog: egui_file_dialog::FileDialog::new(),
                notice: None,
                dragged_connection: None,
            },
            settings: SettingsState {
                active: base_settings.clone(),
                draft: base_settings,
            },
            session: SessionState {
                connection: AppConnectionState::Disconnected,
                ever_connected: false,
                disconnected_by_user: false,
                current_identity: None,
                current_spec: None,
                current_display_name: String::new(),
                reopen: None,
                connected_definition: None,
                layout_names: Vec::new(),
                active_layout_name: String::new(),
                draft_layout_name: String::new(),
            },
            connect: ConnectDraftState {
                available_devices,
                selected_device_index: None,
                selected_saved_identity: None,
                draft: ConnectionDraft::Via {
                    json_path: String::new(),
                },
                pending_connect: None,
                pending_origin: None,
                auto_connect: None,
                discovery_task: None,
            },
            resume_monitor,
            startup_status: crate::platform::startup::status(),
        };
        if auto_connect_at_start {
            app.begin_startup_auto_connect();
        }
        app
    }

    pub(super) fn request_device_refresh(&mut self) {
        if self.connect.discovery_task.is_none() {
            self.connect.discovery_task = Some(DiscoveryTask::start(self.ui_wake.clone()));
        }
    }

    fn poll_device_refresh(&mut self) {
        let Some(task) = self.connect.discovery_task.as_ref() else {
            return;
        };
        let Some(devices) = task.try_finish() else {
            return;
        };

        let selected = self
            .connect
            .selected_device_index
            .and_then(|index| self.connect.available_devices.get(index))
            .cloned();
        self.connect.available_devices = devices;
        self.connect.selected_device_index = selected.as_ref().and_then(|selected| {
            self.connect
                .available_devices
                .iter()
                .position(|device| device == selected)
        });
        self.connect.discovery_task = None;
    }

    fn sync_mouse_passthrough(&mut self, host: &mut dyn OverlayHost) {
        let mouse_passthrough = !self.ui.settings_visible;
        if self.ui.mouse_passthrough == Some(mouse_passthrough) {
            return;
        }

        host.set_passthrough(mouse_passthrough);
        self.ui.mouse_passthrough = Some(mouse_passthrough);
    }

    fn sync_window_frame(
        &self,
        host: &mut dyn OverlayHost,
        overlay_visible: bool,
    ) -> Option<WindowFrame> {
        let needs_fullscreen = self.ui.settings_visible
            || self.ui.settings_error.is_some()
            || self.ui.settings_warning.is_some()
            || self.ui.notice.is_some();

        let frame = if needs_fullscreen {
            host.monitor_size()
                .map(|monitor_size| WindowFrame::FullScreen { monitor_size })
        } else if overlay_visible {
            let AppConnectionState::Connected { keyboard } = &self.session.connection else {
                return None;
            };
            host.monitor_size().map(|monitor_size| {
                let dimensions = keyboard.layout.get_dimensions();
                let scale = self.settings.active.size as f32;
                overlay_content_frame(
                    self.settings.active.position,
                    self.settings.active.margin as f32,
                    monitor_size,
                    egui::vec2(dimensions.0 * scale, dimensions.1 * scale),
                )
            })
        } else {
            Some(WindowFrame::Dormant)
        };

        if let Some(frame) = frame {
            host.set_window_frame(frame);
        }
        frame
    }

    /// Draw a centered modal with `message` and an OK button that clears `slot`.
    fn message_window(ctx: &egui::Context, title: &str, slot: &mut Option<String>) {
        let Some(message) = slot.clone() else {
            return;
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(message);
                ui.add_space(10.0);
                if ui.button("OK").clicked() {
                    *slot = None;
                }
            });
    }

    fn schedule_overlay_hide_repaint(&self, ctx: &egui::Context) {
        if self.ui.settings_visible {
            return;
        }

        let AppConnectionState::Connected { keyboard } = &self.session.connection else {
            return;
        };

        let Some(time_to_hide) = keyboard
            .time_to_hide_overlay
            .lock()
            .unwrap()
            .as_ref()
            .copied()
        else {
            return;
        };

        if let Some(delay) = time_to_hide.checked_duration_since(Instant::now()) {
            ctx.request_repaint_after(delay);
        }
    }

    fn draw_notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.ui.notice.as_ref() else {
            return;
        };
        let now = Instant::now();
        if now >= notice.expires_at {
            self.ui.notice = None;
            return;
        }

        let color = if notice.success {
            egui::Color32::from_rgb(52, 120, 72)
        } else {
            egui::Color32::from_rgb(145, 65, 65)
        };
        egui::Area::new("connection_notice".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color)
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&notice.message).color(egui::Color32::WHITE));
                    });
            });
        ctx.request_repaint_after(notice.expires_at - now);
    }
}

impl OverlayApp {
    /// Backdrop color the host clears to before egui paints: dimmed while settings
    /// are open, otherwise transparent so only the overlay is visible.
    pub fn clear_color(&self) -> egui::Rgba {
        if self.ui.settings_visible {
            egui::Rgba::from_black_alpha(0.65)
        } else {
            egui::Rgba::TRANSPARENT
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut dyn OverlayHost) {
        self.maintain_lifecycle();

        if self.settings_requested.swap(false, Ordering::Relaxed) {
            if !self.ui.settings_visible {
                self.request_device_refresh();
            }
            self.ui.settings_visible = true;
        }

        self.poll_connect_result();
        self.poll_device_refresh();
        self.maintain_connection(ctx);
        self.apply_live_visual_settings();
        self.apply_live_layout_settings();
        self.ui.file_dialog.update(ctx);

        if let Some(path) = self.ui.file_dialog.take_picked() {
            if let ConnectionDraft::Via { json_path } = &mut self.connect.draft {
                *json_path = path.to_string_lossy().to_string();
            }
            self.connect_from_ui();
        }

        self.sync_mouse_passthrough(host);

        let visible_layers = self.current_visible_layers();
        let overlay_visible = self.overlay_visible(visible_layers);
        let window_frame = self.sync_window_frame(host, overlay_visible);
        if let AppConnectionState::Connected { keyboard } = &self.session.connection {
            self.draw_overlay_window(
                ctx,
                keyboard,
                overlay_visible,
                visible_layers,
                matches!(window_frame, Some(WindowFrame::Content { .. })),
            );
        }

        if self.ui.settings_visible {
            self.draw_settings_window(ctx, host);
        }

        Self::message_window(ctx, "Error", &mut self.ui.settings_error);
        Self::message_window(ctx, "Notice", &mut self.ui.settings_warning);
        self.draw_notice(ctx);

        self.schedule_overlay_hide_repaint(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::overlay_content_frame;
    use crate::platform::WindowFrame;
    use crate::settings::WindowPosition;

    #[test]
    fn positions_content_at_bottom_right_with_margin() {
        assert_eq!(
            overlay_content_frame(
                WindowPosition::BottomRight,
                10.0,
                egui::vec2(1920.0, 1080.0),
                egui::vec2(600.0, 240.0),
            ),
            WindowFrame::Content {
                position: egui::pos2(1310.0, 830.0),
                size: egui::vec2(600.0, 240.0),
            }
        );
    }

    #[test]
    fn centers_top_content_and_keeps_margin() {
        assert_eq!(
            overlay_content_frame(
                WindowPosition::Top,
                12.0,
                egui::vec2(1440.0, 900.0),
                egui::vec2(500.0, 200.0),
            ),
            WindowFrame::Content {
                position: egui::pos2(470.0, 12.0),
                size: egui::vec2(500.0, 200.0),
            }
        );
    }
}
