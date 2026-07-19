use crate::device_discovery::DiscoveredDevice;
use crate::settings::Settings;

mod eframe_host;
pub mod resume;
pub mod startup;

#[cfg(target_os = "linux")]
mod wayland;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowFrame {
    Dormant,
    FullScreen {
        monitor_size: egui::Vec2,
    },
    Content {
        position: egui::Pos2,
        size: egui::Vec2,
    },
}

// eframe (winit) can't do always-on-top/click-through on native Wayland, so on
// Linux Wayland sessions we drive a wlr-layer-shell surface directly instead.
pub trait OverlayHost {
    fn set_passthrough(&mut self, enabled: bool);
    fn request_close(&mut self);

    fn monitor_size(&self) -> Option<egui::Vec2> {
        None
    }

    fn set_window_frame(&mut self, _frame: WindowFrame) {}
}

pub fn run(
    settings: Settings,
    devices: Vec<DiscoveredDevice>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    {
        // `WAYLAND_DISPLAY` is unset under XWayland, so X11 falls through to eframe below.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            match wayland::run(settings.clone(), devices.clone()) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // No wlr-layer-shell (e.g. GNOME/Mutter): fall back to eframe on
                    // XWayland, since native Wayland ignores always-on-top.
                    eprintln!(
                        "KeyPeek: Wayland layer-shell host unavailable ({e}); \
                         falling back to eframe on XWayland for always-on-top."
                    );
                    return Ok(eframe_host::run(settings, devices, true)?);
                }
            }
        }
    }

    eframe_host::run(settings, devices, false)?;
    Ok(())
}
