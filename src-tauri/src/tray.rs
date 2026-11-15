//! System tray integration.

use discord_overlay_core::control::Request;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::commands;

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / hide overlay", true, None::<&str>)?;
    let pin = MenuItem::with_id(app, "pin", "Toggle click-through", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset", "Reset overlay position", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&toggle, &pin, &settings, &reset, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Discord Linux Overlay")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let request = match event.id().as_ref() {
                "toggle" => Some(Request::ToggleVisibility),
                "pin" => Some(Request::ToggleClickThrough),
                "settings" => Some(Request::OpenSettings),
                "reset" => Some(Request::ResetPosition),
                "quit" => Some(Request::Quit),
                _ => None,
            };

            if let Some(request) = request {
                let _ = commands::apply_request(app, request);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let _ = commands::apply_request(app, Request::ToggleVisibility);
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}
