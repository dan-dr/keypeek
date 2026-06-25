use crate::device_discovery::DiscoveredDevice;
use crate::settings::Settings;

mod eframe_host;

#[cfg(target_os = "linux")]
mod wayland;

// eframe (winit) can't do always-on-top/click-through on native Wayland, so on
// Linux Wayland sessions we drive a wlr-layer-shell surface directly instead.
pub trait OverlayHost {
    fn set_passthrough(&mut self, enabled: bool);
    fn request_close(&mut self);
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
                    eprintln!(
                        "KeyPeek: Wayland layer-shell host unavailable ({e}); \
                         falling back to eframe (XWayland)."
                    );
                }
            }
        }
    }

    eframe_host::run(settings, devices)?;
    Ok(())
}
