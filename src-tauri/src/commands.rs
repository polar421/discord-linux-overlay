//! Commands exposed to the web views, plus the dispatcher shared by the tray,
//! the control socket and the CLI.

use std::fs;
use std::path::PathBuf;

use discord_overlay_core::client::{Status, TokenStore};
use discord_overlay_core::config::Config;
use discord_overlay_core::control::{Request, Response};
use discord_overlay_core::credentials::{CredentialsView, DiscordCredentials};
use discord_overlay_core::state::OverlayState;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::platform::{self, PlatformInfo};
use crate::state::{lock, AppState};
use crate::windows;

pub const AUTOSTART_FILE: &str = "io.github.polar421.DiscordLinuxOverlay.desktop";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorsResponse {
    pub monitors: Vec<windows::MonitorInfo>,
    pub positioning_supported: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub status: Status,
    pub status_text: String,
    pub credentials: CredentialsView,
    pub configured: bool,
    pub channel: Option<discord_overlay_core::state::ChannelInfo>,
    pub participants: usize,
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config()
}

#[tauri::command]
pub fn update_config(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: Value,
) -> Result<Config, String> {
    let config = state
        .config_store
        .patch(&patch)
        .map_err(|error| error.to_string())?;

    *lock(&state.config) = config.clone();
    windows::apply_behavior(&app, &config)?;

    if patch.get("placement").is_some() {
        windows::apply_placement(&app, &config)?;
    }

    if let Some(value) = patch
        .get("general")
        .and_then(|general| general.get("launchAtLogin"))
        .and_then(Value::as_bool)
    {
        if let Err(error) = set_autostart(value) {
            state.push_log(format!("could not update autostart: {error}"));
        }
    }

    let _ = app.emit("config:changed", &config);
    Ok(config)
}

#[tauri::command]
pub fn get_overlay_state(state: State<'_, AppState>) -> OverlayState {
    lock(&state.overlay).clone()
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Status {
    lock(&state.status).clone()
}

#[tauri::command]
pub fn get_connection(state: State<'_, AppState>) -> ConnectionInfo {
    let status = lock(&state.status).clone();
    let credentials = lock(&state.credentials).public_view();
    let overlay = lock(&state.overlay).clone();

    ConnectionInfo {
        status_text: status_text(&status),
        configured: !credentials.client_id.is_empty() && credentials.has_secret,
        status,
        credentials,
        channel: overlay.channel,
        participants: overlay.participants.len(),
    }
}

#[tauri::command]
pub fn set_credentials(
    app: AppHandle,
    state: State<'_, AppState>,
    client_id: String,
    client_secret: String,
    redirect_uri: Option<String>,
) -> Result<CredentialsView, String> {
    let credentials = DiscordCredentials {
        client_id: client_id.trim().to_string(),
        client_secret: client_secret.trim().to_string(),
        redirect_uri: redirect_uri
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    };

    if credentials.client_id.is_empty() {
        return Err("A Discord application client id is required.".to_string());
    }
    if credentials.client_secret.is_empty() {
        return Err("A Discord application client secret is required.".to_string());
    }

    state
        .credentials_store
        .save(&credentials)
        .map_err(|error| error.to_string())?;
    *lock(&state.credentials) = credentials.clone();

    crate::discord::restart(&app);
    state.push_log("Discord application credentials saved");

    Ok(credentials.public_view())
}

#[tauri::command]
pub fn clear_credentials(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state
        .credentials_store
        .clear()
        .map_err(|error| error.to_string())?;
    *lock(&state.credentials) = DiscordCredentials::default();
    state.token_store.clear();
    crate::discord::restart(&app);
    Ok(())
}

#[tauri::command]
pub fn reconnect_discord(app: AppHandle) {
    crate::discord::restart(&app);
}

#[tauri::command]
pub fn reauthorize_discord(state: State<'_, AppState>) {
    if let Some(client) = lock(&state.client).as_ref() {
        client.reauthorize();
    }
}

#[tauri::command]
pub fn sign_out_discord(state: State<'_, AppState>) {
    state.token_store.clear();
    if let Some(client) = lock(&state.client).as_ref() {
        client.sign_out();
    }
}

fn set_click_through_inner(app: &AppHandle, value: bool) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let config = state
        .config_store
        .patch(&json!({ "behavior": { "clickThrough": value } }))
        .map_err(|error| error.to_string())?;

    *lock(&state.config) = config.clone();
    windows::apply_behavior(app, &config)?;
    if let Some(window) = windows::overlay_window(app) {
        let _ = window.set_skip_taskbar(value);
    }
    let _ = app.emit("config:changed", &config);
    Ok(value)
}

#[tauri::command]
pub fn set_click_through(app: AppHandle, value: bool) -> Result<bool, String> {
    set_click_through_inner(&app, value)
}

#[tauri::command]
pub fn toggle_click_through(app: AppHandle) -> Result<bool, String> {
    let current = app.state::<AppState>().config().behavior.click_through;
    set_click_through_inner(&app, !current)
}

#[tauri::command]
pub fn set_always_on_top(app: AppHandle, value: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let config = state
        .config_store
        .patch(&json!({ "behavior": { "alwaysOnTop": value } }))
        .map_err(|error| error.to_string())?;

    *lock(&state.config) = config.clone();
    windows::apply_behavior(&app, &config)?;
    let _ = app.emit("config:changed", &config);
    Ok(())
}

#[tauri::command]
pub fn set_overlay_visible(app: AppHandle, value: bool) -> bool {
    if value {
        windows::show_overlay(&app);
    } else {
        windows::hide_overlay(&app);
    }
    value
}

#[tauri::command]
pub fn toggle_overlay(app: AppHandle) -> bool {
    let visible = windows::is_visible(&app);
    set_overlay_visible(app, !visible)
}

#[tauri::command]
pub fn set_locked(app: AppHandle, state: State<'_, AppState>, value: bool) -> Result<(), String> {
    let config = state
        .config_store
        .patch(&json!({ "behavior": { "lockPosition": value } }))
        .map_err(|error| error.to_string())?;
    *lock(&state.config) = config.clone();
    windows::apply_behavior(&app, &config)?;
    let _ = app.emit("config:changed", &config);
    Ok(())
}

#[tauri::command]
pub fn reset_position(app: AppHandle, state: State<'_, AppState>) -> Result<Config, String> {
    let config = state
        .config_store
        .patch(&json!({
            "placement": {
                "mode": "anchored",
                "anchor": "bottom-right",
                "monitor": null,
                "offsetX": 24.0,
                "offsetY": 24.0,
                "x": null,
                "y": null
            }
        }))
        .map_err(|error| error.to_string())?;

    *lock(&state.config) = config.clone();
    windows::apply_placement(&app, &config)?;
    let _ = app.emit("config:changed", &config);
    Ok(config)
}

#[tauri::command]
pub fn apply_placement_now(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let config = state.config();
    windows::apply_placement(&app, &config)
}

#[tauri::command]
pub fn get_monitors(app: AppHandle) -> Result<MonitorsResponse, String> {
    let window =
        windows::overlay_window(&app).ok_or_else(|| "overlay window is missing".to_string())?;
    Ok(MonitorsResponse {
        monitors: windows::monitor_list(&window)?,
        positioning_supported: !platform::is_wayland(),
    })
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    if let Some(window) = windows::settings_window(&app) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn close_settings(app: AppHandle) {
    if let Some(window) = windows::settings_window(&app) {
        let _ = window.hide();
    }
}

#[tauri::command]
pub fn get_platform() -> PlatformInfo {
    platform::platform_info()
}

#[tauri::command]
pub fn get_logs(state: State<'_, AppState>) -> Vec<String> {
    lock(&state.logs).iter().cloned().collect()
}

#[tauri::command]
pub fn clear_join_history(state: State<'_, AppState>) {
    lock(&state.overlay).clear_history();
}

#[tauri::command]
pub fn open_config_dir(state: State<'_, AppState>) -> Result<(), String> {
    let dir = state.directory();
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    std::process::Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open {}: {error}", dir.display()))
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.state::<AppState>().stop_client();
    app.exit(0);
}

/// Dispatch a control request. Used by the tray, the control socket and the CLI.
pub fn apply_request(app: &AppHandle, request: Request) -> Response {
    match request {
        Request::Show => {
            windows::show_overlay(app);
            Response::ok("overlay shown")
        }
        Request::Hide => {
            windows::hide_overlay(app);
            Response::ok("overlay hidden")
        }
        Request::ToggleVisibility => {
            let visible = windows::is_visible(app);
            set_overlay_visible(app.clone(), !visible);
            Response::ok(if visible {
                "overlay hidden"
            } else {
                "overlay shown"
            })
        }
        Request::ToggleClickThrough => match toggle_click_through(app.clone()) {
            Ok(true) => Response::ok("click-through enabled"),
            Ok(false) => Response::ok("click-through disabled"),
            Err(error) => Response::error(error),
        },
        Request::Pin { value } => match set_click_through_inner(app, value) {
            Ok(_) => Response::ok(if value {
                "click-through enabled"
            } else {
                "click-through disabled"
            }),
            Err(error) => Response::error(error),
        },
        Request::OpenSettings => {
            open_settings(app.clone());
            Response::ok("settings opened")
        }
        Request::ResetPosition => {
            let state = app.state::<AppState>();
            match state
                .config_store
                .patch(&json!({
                    "placement": {
                        "mode": "anchored",
                        "anchor": "bottom-right",
                        "monitor": null,
                        "offsetX": 24.0,
                        "offsetY": 24.0,
                        "x": null,
                        "y": null
                    }
                }))
                .and_then(|config| {
                    *lock(&state.config) = config.clone();
                    windows::apply_placement(app, &config)
                        .map_err(discord_overlay_core::Error::Settings)?;
                    let _ = app.emit("config:changed", &config);
                    Ok(config)
                }) {
                Ok(_) => Response::ok("overlay position reset"),
                Err(error) => Response::error(error.to_string()),
            }
        }
        Request::ToggleLock => {
            let state = app.state::<AppState>();
            let current = state.config().behavior.lock_position;
            match state
                .config_store
                .patch(&json!({ "behavior": { "lockPosition": !current } }))
            {
                Ok(config) => {
                    *lock(&state.config) = config.clone();
                    let _ = windows::apply_behavior(app, &config);
                    let _ = app.emit("config:changed", &config);
                    Response::ok(if current {
                        "position unlocked"
                    } else {
                        "position locked"
                    })
                }
                Err(error) => Response::error(error.to_string()),
            }
        }
        Request::Status => {
            let state = app.state::<AppState>();
            let status = lock(&state.status).clone();
            let overlay = lock(&state.overlay).clone();
            let channel = overlay
                .channel
                .as_ref()
                .map(|channel| channel.name.clone())
                .unwrap_or_else(|| "not in a voice channel".to_string());
            Response::ok(format!(
                "Discord: {} Channel: {}. Participants: {}.",
                status_text(&status),
                channel,
                overlay.participants.len()
            ))
        }
        Request::Diagnose => Response::ok(crate::diagnose::runtime_report(app)),
        Request::Quit => {
            app.state::<AppState>().stop_client();
            app.exit(0);
            Response::ok("quitting")
        }
    }
}

pub fn status_text(status: &Status) -> String {
    match status {
        Status::Starting => "starting".to_string(),
        Status::WaitingForDiscord { .. } => "waiting for the Discord desktop client".to_string(),
        Status::Connecting => "connecting to Discord".to_string(),
        Status::Authorizing => "waiting for authorization in Discord".to_string(),
        Status::Authenticating => "signing in".to_string(),
        Status::Ready { user_name } => format!("connected as {user_name}"),
        Status::NeedsSetup { message } => format!("setup required — {message}"),
        Status::Stopped => "stopped".to_string(),
    }
}

fn autostart_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;

    Some(base.join("autostart").join(AUTOSTART_FILE))
}

/// Create or remove the XDG autostart entry.
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(path) = autostart_path() else {
        return Err("could not determine the autostart directory".to_string());
    };

    if !enabled {
        match fs::remove_file(&path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Discord Linux Overlay\n\
         Comment=Discord voice overlay for Linux\n\
         Exec={} --hidden\n\
         Icon=io.github.polar421.DiscordLinuxOverlay\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        executable.display()
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, contents).map_err(|error| error.to_string())
}
