use super::state::AppConnectionState;
use super::OverlayApp;
use crate::settings::{ProtocolType, WindowPosition, ALL_LAYERS_MASK};
use egui::Align2;
use std::time::Instant;

fn layer_selection_allows_overlay(
    active_layers: u32,
    last_deactivated_layers: u32,
    visible_layers: u32,
    base_timeout_active: Option<bool>,
) -> bool {
    let active_non_base_layers = active_layers & !1;
    if active_non_base_layers != 0 {
        return active_non_base_layers & visible_layers != 0;
    }
    if visible_layers & 1 == 0 {
        return false;
    }

    match base_timeout_active {
        Some(timeout_active) => {
            (last_deactivated_layers == 0 || last_deactivated_layers & visible_layers != 0)
                && timeout_active
        }
        None => true,
    }
}

impl OverlayApp {
    pub(super) fn apply_live_visual_settings(&mut self) {
        if self.settings.active == self.settings.draft {
            return;
        }

        let old_timeout = self.settings.active.timeout;
        self.settings.active = self.settings.draft.clone();

        if let AppConnectionState::Connected { keyboard } = &self.session.connection {
            if old_timeout != self.settings.active.timeout {
                keyboard.set_timeout(self.settings.active.timeout);
            }
        }

        self.persist_settings();
    }

    pub(super) fn apply_live_layout_settings(&mut self) {
        if self.session.active_layout_name == self.session.draft_layout_name {
            return;
        }

        if !matches!(
            self.connect.draft.protocol_type(),
            ProtocolType::Via | ProtocolType::Vial
        ) {
            self.session.draft_layout_name = self.session.active_layout_name.clone();
            return;
        }

        let Some(definition) = self.session.connected_definition.as_ref() else {
            self.ui.settings_error =
                Some("Missing keyboard definition for live layout switch".to_string());
            self.session.draft_layout_name = self.session.active_layout_name.clone();
            return;
        };

        let selected_layout = self.session.draft_layout_name.clone();
        let next_layout = match definition.get_layout(&selected_layout) {
            Ok(layout) => layout,
            Err(e) => {
                self.ui.settings_error = Some(format!("Failed to switch layout: {e}"));
                self.session.draft_layout_name = self.session.active_layout_name.clone();
                return;
            }
        };

        let AppConnectionState::Connected { keyboard } = &mut self.session.connection else {
            return;
        };

        keyboard.set_layout(next_layout);
        self.session.active_layout_name = selected_layout;
        if let Some(identity) = self.session.current_identity.as_ref() {
            if let Some(saved) = self
                .settings
                .active
                .saved_connections
                .iter_mut()
                .find(|saved| &saved.identity == identity)
            {
                if matches!(saved.spec, crate::protocols::ConnectionSpec::Via { .. }) {
                    saved.layout_name = Some(self.session.active_layout_name.clone());
                    if let Some(draft_saved) = self
                        .settings
                        .draft
                        .saved_connections
                        .iter_mut()
                        .find(|draft| &draft.identity == identity)
                    {
                        draft_saved.layout_name = saved.layout_name.clone();
                    }
                    self.persist_settings();
                }
            }
        }
    }

    pub(super) fn get_anchor_params(&self) -> (Align2, egui::Vec2) {
        match self.settings.active.position {
            WindowPosition::TopLeft => (
                Align2::LEFT_TOP,
                egui::vec2(
                    self.settings.active.margin as f32,
                    self.settings.active.margin as f32,
                ),
            ),
            WindowPosition::TopRight => (
                Align2::RIGHT_TOP,
                egui::vec2(
                    -(self.settings.active.margin as f32),
                    self.settings.active.margin as f32,
                ),
            ),
            WindowPosition::BottomLeft => (
                Align2::LEFT_BOTTOM,
                egui::vec2(
                    self.settings.active.margin as f32,
                    -(self.settings.active.margin as f32),
                ),
            ),
            WindowPosition::BottomRight => (
                Align2::RIGHT_BOTTOM,
                egui::vec2(
                    -(self.settings.active.margin as f32),
                    -(self.settings.active.margin as f32),
                ),
            ),
            WindowPosition::Bottom => (
                Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -(self.settings.active.margin as f32)),
            ),
            WindowPosition::Top => (
                Align2::CENTER_TOP,
                egui::vec2(0.0, self.settings.active.margin as f32),
            ),
        }
    }

    pub(super) fn current_visible_layers(&self) -> u32 {
        let Some(identity) = self.session.current_identity.as_ref() else {
            return ALL_LAYERS_MASK;
        };

        self.settings
            .active
            .saved_connections
            .iter()
            .find(|connection| &connection.identity == identity)
            .map_or(ALL_LAYERS_MASK, |connection| connection.visible_layers)
    }

    pub(super) fn overlay_visible(&self, visible_layers: u32) -> bool {
        match &self.session.connection {
            AppConnectionState::Disconnected | AppConnectionState::AutoConnecting => false,
            AppConnectionState::Connected { keyboard } => {
                if self.ui.settings_visible {
                    true
                } else {
                    let time_to_hide = *keyboard.time_to_hide_overlay.lock().unwrap();
                    layer_selection_allows_overlay(
                        keyboard.active_layers(),
                        keyboard.last_deactivated_layers(),
                        visible_layers,
                        time_to_hide.map(|time_to_hide| Instant::now() < time_to_hide),
                    )
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::layer_selection_allows_overlay;

    #[test]
    fn selected_active_layer_shows_and_hidden_active_layer_stays_hidden() {
        let visible_layers = 0b0011;

        assert!(layer_selection_allows_overlay(
            0b0011,
            0,
            visible_layers,
            Some(false)
        ));
        assert!(!layer_selection_allows_overlay(
            0b0101,
            0,
            visible_layers,
            Some(true)
        ));
    }

    #[test]
    fn base_preview_only_follows_a_selected_layer() {
        let visible_layers = 0b0011;

        assert!(layer_selection_allows_overlay(
            0b0001,
            0b0010,
            visible_layers,
            Some(true)
        ));
        assert!(!layer_selection_allows_overlay(
            0b0001,
            0b0100,
            visible_layers,
            Some(true)
        ));
        assert!(!layer_selection_allows_overlay(
            0b0001,
            0b0010,
            0b0010,
            Some(true)
        ));
    }
}
