use image::load_from_memory;
use std::process;
use std::thread;
use tray_icon::{menu::Menu, menu::MenuEvent, menu::MenuItem, Icon, TrayIcon, TrayIconBuilder};

/// Keeps the tray icon alive for the lifetime of the app.
///
/// On Linux the icon lives on a dedicated GTK thread: libappindicator
/// registers the StatusNotifierItem over DBus via the glib main loop, so the
/// tray only works if `gtk::main()` runs on the thread that created it.
pub struct Tray {
    #[cfg(not(target_os = "linux"))]
    _icon: TrayIcon,
}

fn create_icon() -> Icon {
    const ICON_BYTES: &[u8] = include_bytes!("../resources/icon.ico");

    let icon = load_from_memory(ICON_BYTES)
        .expect("Failed to load icon.")
        .into_rgba8();

    let (width, height) = icon.dimensions();
    Icon::from_rgba(icon.into_raw(), width, height).expect("Failed to create icon.")
}

fn build_tray_icon() -> TrayIcon {
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append(&quit).expect("Failed to append menu item.");

    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(create_icon())
        .with_tooltip("KeyPeek")
        .build()
        .unwrap()
}

pub fn create_tray_icon() -> Tray {
    thread::spawn(move || {
        if MenuEvent::receiver().recv().is_ok() {
            process::exit(0);
        }
    });

    #[cfg(target_os = "linux")]
    {
        thread::spawn(|| {
            gtk::init().expect("Failed to initialize GTK. Is a display available?");
            let _icon = build_tray_icon();
            gtk::main();
        });
        Tray {}
    }

    #[cfg(not(target_os = "linux"))]
    Tray {
        _icon: build_tray_icon(),
    }
}
