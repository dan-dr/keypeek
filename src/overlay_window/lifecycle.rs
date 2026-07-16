use super::state::AppConnectionState;
use super::OverlayApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResumeAction {
    None,
    ReconnectCurrent,
    RetrySaved,
}

fn resume_action(
    disconnected_by_user: bool,
    has_current_connection: bool,
    auto_connect: bool,
) -> ResumeAction {
    if disconnected_by_user {
        ResumeAction::None
    } else if has_current_connection {
        ResumeAction::ReconnectCurrent
    } else if auto_connect {
        ResumeAction::RetrySaved
    } else {
        ResumeAction::None
    }
}

impl OverlayApp {
    pub(super) fn maintain_lifecycle(&mut self) {
        if !self.resume_monitor.is_requested() {
            return;
        }

        // Let an explicit user connection finish. Its result wakes the UI. A
        // success clears this event; a failure leaves it pending for the next frame.
        if self.connect.pending_connect.is_some()
            && self.connect.pending_origin == Some(super::state::ConnectionOrigin::Manual)
        {
            return;
        }

        if !self.resume_monitor.take_requested() {
            return;
        }

        self.request_device_refresh();

        match resume_action(
            self.session.disconnected_by_user,
            matches!(
                self.session.connection,
                AppConnectionState::Connected { .. } | AppConnectionState::AutoConnecting
            ) || (self.session.current_identity.is_some() && self.session.current_spec.is_some()),
            self.settings.active.auto_connect,
        ) {
            ResumeAction::None => {}
            ResumeAction::ReconnectCurrent => self.begin_disconnect_reconnect(),
            ResumeAction::RetrySaved => self.begin_startup_auto_connect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{resume_action, ResumeAction};

    #[test]
    fn reconnects_current_or_saved_connections_but_respects_manual_disconnect() {
        assert_eq!(
            resume_action(false, true, false),
            ResumeAction::ReconnectCurrent
        );
        assert_eq!(resume_action(false, false, true), ResumeAction::RetrySaved);
        assert_eq!(resume_action(true, true, true), ResumeAction::None);
        assert_eq!(resume_action(false, false, false), ResumeAction::None);
    }
}
