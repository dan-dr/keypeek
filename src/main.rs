#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod connection;
mod device_discovery;
mod key_matrix;
mod keyboard;
mod layout_key;
mod overlay_window;
mod platform;
mod protocols;
mod qmk_keycode_labels;
mod settings;
mod single_instance;
mod tray;
mod ui_wake;
mod zmk_keycode_labels;

use device_discovery::discover_devices;
use settings::Settings;
use single_instance::{temporary_lock_path, InstanceLock};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let lock_path = Settings::config_file_path()
        .and_then(|path| path.parent().map(|parent| parent.join("instance.lock")));
    let fallback_lock_path = temporary_lock_path(lock_path.as_deref());
    let _instance_lock =
        match InstanceLock::acquire_with_fallback(lock_path.as_deref(), &fallback_lock_path) {
            Ok(Some(lock)) => Some(lock),
            Ok(None) => return Ok(()),
            Err(error) => {
                eprintln!("KeyPeek: single-instance locking unavailable ({error}); continuing");
                None
            }
        };

    let settings = Settings::load().unwrap_or_default();
    let available_devices = discover_devices();
    platform::run(settings, available_devices)
}
