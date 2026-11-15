//! The `--diagnose` report.
//!
//! Two forms exist:
//!
//! * the running instance produces [`runtime_report`], which includes live
//!   connection, subscription and window state;
//! * `--diagnose` with no instance running falls back to [`offline_report`],
//!   which still describes the machine, the graphics stack, Discord socket
//!   discovery and the saved configuration.
//!
//! The report deliberately contains no secrets: no OAuth tokens, no client
//! secret, no authorization codes. Credentials are reported as configured or
//! not, and the token cache as present or absent.

use std::fmt::Write as _;
use std::os::unix::net::UnixStream;
use std::path::Path;

use discord_overlay_core::client::{Status, TokenStore};
use discord_overlay_core::config::{config_dir, Config, ConfigStore};
use discord_overlay_core::ipc;
use tauri::{AppHandle, Manager};

use crate::commands::status_text;
use crate::platform;
use crate::state::{lock, AppState};
use crate::windows;

/// Runtime-only information, filled from the live application.
struct Runtime {
    status: Status,
    credentials_configured: bool,
    cached_token: bool,
    participant_count: usize,
    channel: Option<String>,
    settings_window: Option<bool>,
    overlay: Vec<String>,
}

pub fn runtime_report(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let status = lock(&state.status).clone();
    let overlay_state = lock(&state.overlay).clone();
    let credentials_configured = lock(&state.credentials).is_configured();
    let cached_token = state.token_store.load().is_some();

    let mut overlay_lines = windows::overlay_diagnostics(app);

    let config = state.config();
    overlay_lines.push(format!(
        "configured mode: {:?}, anchor: {:?}",
        config.placement.mode, config.placement.anchor
    ));
    overlay_lines.push(format!(
        "configured click-through: {}, always-on-top: {}, locked: {}",
        config.behavior.click_through, config.behavior.always_on_top, config.behavior.lock_position
    ));

    let runtime = Runtime {
        status,
        credentials_configured,
        cached_token,
        participant_count: overlay_state.participants.len(),
        channel: overlay_state.channel.as_ref().map(|c| c.name.clone()),
        settings_window: windows::settings_window(app).and_then(|window| window.is_visible().ok()),
        overlay: overlay_lines,
    };

    render(&config, Some(&runtime))
}

pub fn offline_report() -> String {
    let config = ConfigStore::new(config_dir().join("config.json")).load();
    render(&config, None)
}

fn render(config: &Config, runtime: Option<&Runtime>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Discord Linux Overlay {} — diagnostic report",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        out,
        "mode: {}",
        if runtime.is_some() {
            "running instance"
        } else {
            "offline (no running instance; start the app for live connection state)"
        }
    );

    section(&mut out, "Application");
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let _ = writeln!(out, "binary: {exe}");
    let _ = writeln!(out, "config directory: {}", config_dir().display());
    let _ = writeln!(
        out,
        "control socket: {}",
        discord_overlay_core::control::control_socket_path().display()
    );

    section(&mut out, "System");
    let _ = writeln!(out, "os: {}", os_pretty_name());
    let _ = writeln!(out, "kernel: {}", kernel_release());
    let _ = writeln!(out, "architecture: {}", std::env::consts::ARCH);
    let info = platform::platform_info();
    let _ = writeln!(
        out,
        "session: {} (XDG_SESSION_TYPE={})",
        info.session_type,
        env_or_unset("XDG_SESSION_TYPE")
    );
    let _ = writeln!(out, "desktop: {}", info.desktop);
    let _ = writeln!(out, "compositor: {}", info.compositor);
    let _ = writeln!(
        out,
        "DISPLAY={} WAYLAND_DISPLAY={}",
        env_or_unset("DISPLAY"),
        env_or_unset("WAYLAND_DISPLAY")
    );
    let _ = writeln!(out, "toolkit backend: {}", toolkit_backend(&info));

    section(&mut out, "Graphics");
    let _ = writeln!(
        out,
        "nvidia: {}",
        if info.nvidia {
            "yes"
        } else {
            "no (or driver not loaded)"
        }
    );
    let _ = writeln!(
        out,
        "nvidia driver: {}",
        info.nvidia_driver
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    );
    let _ = writeln!(
        out,
        "always-on-top support: {:?}, click-through: {:?}, positioning: {:?}",
        info.always_on_top, info.click_through, info.window_positioning
    );
    let _ = writeln!(
        out,
        "graphics environment: __NV_DISABLE_EXPLICIT_SYNC={} WEBKIT_DISABLE_DMABUF_RENDERER={} DLO_NO_GRAPHICS_WORKAROUNDS={} GDK_BACKEND={}",
        env_or_unset("__NV_DISABLE_EXPLICIT_SYNC"),
        env_or_unset("WEBKIT_DISABLE_DMABUF_RENDERER"),
        env_or_unset("DLO_NO_GRAPHICS_WORKAROUNDS"),
        env_or_unset("GDK_BACKEND")
    );
    let _ = writeln!(
        out,
        "disableDmabufRenderer (config): {}",
        config.general.disable_dmabuf_renderer
    );
    if info.nvidia && info.session_type == "wayland" {
        let _ = writeln!(
            out,
            "workaround: NVIDIA + Wayland sets __NV_DISABLE_EXPLICIT_SYNC=1 at startup unless already set or DLO_NO_GRAPHICS_WORKAROUNDS=1"
        );
    }
    if config.general.disable_dmabuf_renderer {
        let _ = writeln!(
            out,
            "workaround: the DMA-BUF renderer is disabled in the configuration (software compositing)"
        );
    }

    section(&mut out, "Discord");
    let candidates = ipc::discover_socket_paths();
    if candidates.is_empty() {
        let _ = writeln!(
            out,
            "socket candidates: none found (is the official Discord client running?)"
        );
    } else {
        let _ = writeln!(out, "socket candidates:");
        for path in &candidates {
            let _ = writeln!(out, "  {} — {}", path.display(), socket_state(path));
        }
    }
    let connectable = candidates
        .iter()
        .find(|path| UnixStream::connect(path).is_ok())
        .map(|path| path.display().to_string());
    let _ = writeln!(
        out,
        "connectable socket: {}",
        connectable.unwrap_or_else(|| "none".to_string())
    );

    match runtime {
        Some(runtime) => {
            let _ = writeln!(out, "rpc connection: {}", status_text(&runtime.status));
            let _ = writeln!(
                out,
                "authentication: credentials {}, token cache {}",
                if runtime.credentials_configured {
                    "configured"
                } else {
                    "not configured"
                },
                if runtime.cached_token {
                    "present"
                } else {
                    "absent"
                }
            );
            let _ = writeln!(
                out,
                "voice event subscription: {}",
                if matches!(runtime.status, Status::Ready { .. }) {
                    "active"
                } else {
                    "not active (subscription starts after authentication)"
                }
            );
            let _ = writeln!(
                out,
                "voice channel: {}",
                runtime
                    .channel
                    .clone()
                    .unwrap_or_else(|| "none".to_string())
            );
            let _ = writeln!(out, "participants: {}", runtime.participant_count);
            let _ = writeln!(
                out,
                "settings window: {}",
                match runtime.settings_window {
                    Some(true) => "open",
                    Some(false) => "created, hidden",
                    None => "not created",
                }
            );

            section(&mut out, "Overlay window");
            for line in &runtime.overlay {
                let _ = writeln!(out, "{line}");
            }
        }
        None => {
            section(&mut out, "Overlay window");
            let _ = writeln!(
                out,
                "not running — start `discord-linux-overlay`, then run `--diagnose` again for live state"
            );
        }
    }

    section(&mut out, "Configuration");
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            let _ = writeln!(out, "{json}");
        }
        Err(error) => {
            let _ = writeln!(out, "could not render the configuration: {error}");
        }
    }

    section(&mut out, "KDE / Wayland notes");
    if info.notes.is_empty() {
        let _ = writeln!(out, "no platform caveats detected for this session");
    } else {
        for note in &info.notes {
            let _ = writeln!(out, "- {note}");
        }
    }

    out
}

fn section(out: &mut String, title: &str) {
    let _ = writeln!(out);
    let _ = writeln!(out, "[{title}]");
}

fn toolkit_backend(info: &crate::platform::PlatformInfo) -> String {
    match std::env::var("GDK_BACKEND").ok().as_deref() {
        Some("x11") if info.session_type == "wayland" => {
            "XWayland (GDK_BACKEND=x11 forces X11; unset it for native Wayland)".to_string()
        }
        Some("wayland") => "Wayland (GDK_BACKEND=wayland)".to_string(),
        Some("x11") => "X11 (GDK_BACKEND=x11)".to_string(),
        _ if info.session_type == "wayland" => "Wayland (native)".to_string(),
        _ if std::env::var_os("DISPLAY").is_some() => "X11".to_string(),
        _ => "unknown".to_string(),
    }
}

fn env_or_unset(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => "<unset>".to_string(),
    }
}

fn socket_state(path: &Path) -> &'static str {
    if !path.exists() {
        return "absent";
    }
    match UnixStream::connect(path) {
        Ok(_) => "present, accepts connections",
        Err(_) => "present, connection refused",
    }
}

fn os_pretty_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| format!("{} {}", std::env::consts::OS, std::env::consts::ARCH))
}

fn kernel_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}
