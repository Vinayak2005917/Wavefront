use tauri::App;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::overlay;

pub fn install(app: &mut App) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Open Wavefront", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Exit Wavefront", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &quit])?;
    let icon = tauri::image::Image::new(include_bytes!("../../assets/tray.rgba"), 64, 64);

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Wavefront · Dictation shortcut in Settings")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "status" => {
                overlay::show_dashboard(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                overlay::show_dashboard(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
