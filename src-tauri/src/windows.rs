//! Overlay window management: placement, click-through, always-on-top and
//! bounds persistence.

use discord_overlay_core::config::{Config, PlacementMode};
use discord_overlay_core::geometry::{
    clamp_into_monitor, compute_placement, is_offscreen, MonitorRect, WindowPlacement,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::platform;
use crate::state::{lock, AppState};

pub const OVERLAY_LABEL: &str = "overlay";
pub const SETTINGS_LABEL: &str = "settings";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub primary: bool,
}

pub fn overlay_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(OVERLAY_LABEL)
}

pub fn settings_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(SETTINGS_LABEL)
}

fn monitor_info(monitor: &tauri::Monitor, primary: Option<&tauri::Monitor>) -> MonitorInfo {
    MonitorInfo {
        name: monitor
            .name()
            .cloned()
            .unwrap_or_else(|| "monitor".to_string()),
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale_factor: monitor.scale_factor(),
        primary: primary
            .map(|primary| {
                primary.position() == monitor.position() && primary.size() == monitor.size()
            })
            .unwrap_or(false),
    }
}

pub fn monitor_list(window: &WebviewWindow) -> Result<Vec<MonitorInfo>, String> {
    let primary = window
        .primary_monitor()
        .map_err(|error| error.to_string())?;
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;

    Ok(monitors
        .iter()
        .map(|monitor| monitor_info(monitor, primary.as_ref()))
        .collect())
}

fn resolve_monitor(window: &WebviewWindow, name: Option<&str>) -> Result<tauri::Monitor, String> {
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;

    if let Some(name) = name {
        if let Some(monitor) = monitors
            .iter()
            .find(|monitor| monitor.name().map(String::as_str) == Some(name))
        {
            return Ok(monitor.clone());
        }
    }

    window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| monitors.first().cloned())
        .ok_or_else(|| "no monitor is available".to_string())
}

/// Apply the configured placement to the overlay window.
pub fn apply_placement(app: &AppHandle, config: &Config) -> Result<(), String> {
    let Some(window) = overlay_window(app) else {
        return Ok(());
    };

    let monitors = monitor_list(&window)?;
    let rects: Vec<MonitorRect> = monitors
        .iter()
        .map(|monitor| MonitorRect::new(monitor.x, monitor.y, monitor.width, monitor.height))
        .collect();

    if config.placement.mode == PlacementMode::Manual {
        if let (Some(x), Some(y)) = (config.placement.x, config.placement.y) {
            let scale = window.scale_factor().unwrap_or(1.0);
            let width = (config.placement.width * scale).round().max(1.0) as u32;
            let height = (config.placement.height * scale).round().max(1.0) as u32;
            let saved = WindowPlacement {
                x,
                y,
                width,
                height,
            };

            if !is_offscreen(saved, &rects) {
                window
                    .set_position(PhysicalPosition::new(saved.x, saved.y))
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }

            if let Some(primary) = rects.first().copied() {
                let clamped = clamp_into_monitor(saved, primary);
                window
                    .set_position(PhysicalPosition::new(clamped.x, clamped.y))
                    .map_err(|error| error.to_string())?;
            }
        }
        return Ok(());
    }

    let monitor = resolve_monitor(&window, config.placement.monitor.as_deref())?;
    let placement = compute_placement(
        MonitorRect::new(
            monitor.position().x,
            monitor.position().y,
            monitor.size().width,
            monitor.size().height,
        ),
        (config.placement.width, config.placement.height),
        monitor.scale_factor(),
        config.placement.anchor,
        (config.placement.offset_x, config.placement.offset_y),
    )
    .map_err(|error| error.to_string())?;

    // Some Wayland compositors reject set_position; resizing is still useful,
    // so ignore a positioning failure instead of aborting.
    if let Err(error) = window.set_position(PhysicalPosition::new(placement.x, placement.y)) {
        log::debug!("could not position overlay: {error}");
    }
    window
        .set_size(PhysicalSize::new(placement.width, placement.height))
        .map_err(|error| error.to_string())?;

    Ok(())
}

/// Apply behaviour settings that map directly onto window properties.
pub fn apply_behavior(app: &AppHandle, config: &Config) -> Result<(), String> {
    let Some(window) = overlay_window(app) else {
        return Ok(());
    };

    if let Err(error) = window.set_always_on_top(config.behavior.always_on_top) {
        log::debug!("always-on-top is not supported here: {error}");
    }
    if let Err(error) = window.set_ignore_cursor_events(config.behavior.click_through) {
        log::debug!("click-through is not supported here: {error}");
    }
    if let Err(error) = window.set_resizable(!config.behavior.lock_position) {
        log::debug!("could not change resizability: {error}");
    }

    Ok(())
}

pub fn show_overlay(app: &AppHandle) {
    if let Some(window) = overlay_window(app) {
        let _ = window.show();
        let config = app.state::<AppState>().config();
        let _ = apply_placement(app, &config);

        // Intentionally never call set_focus(): showing the overlay must not
        // pull keyboard focus away from a game.
        let _ = config.behavior.click_through;
    }
}

pub fn hide_overlay(app: &AppHandle) {
    if let Some(window) = overlay_window(app) {
        let _ = window.hide();
    }
}

pub fn is_visible(app: &AppHandle) -> bool {
    overlay_window(app)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Persist the current bounds of a manually dragged overlay.
pub fn remember_bounds(app: &AppHandle, window: &WebviewWindow) {
    let state = app.state::<AppState>();
    let mut config = state.config();
    if config.placement.mode != PlacementMode::Manual {
        return;
    }

    let Ok(position) = window.outer_position() else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);

    config.placement.x = Some(position.x);
    config.placement.y = Some(position.y);
    config.placement.width = (size.width as f64 / scale).max(64.0);
    config.placement.height = (size.height as f64 / scale).max(64.0);

    if let Err(error) = state.config_store.save(&config) {
        log::warn!("could not persist overlay bounds: {error}");
        return;
    }
    *lock(&state.config) = config.clone();
    let _ = app.emit("config:changed", &config);
}
