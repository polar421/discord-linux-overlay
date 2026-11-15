//! Linux session detection and honest capability reporting.
//!
//! Wayland deliberately gives applications no way to raise themselves above
//! other windows, no way to position a toplevel window and (without a portal)
//! no global hotkeys. The settings UI therefore shows what is actually
//! supported on the current session instead of pretending a toggle worked.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Support {
    /// Works through the normal toolkit path.
    Supported,
    /// Attempted, but the compositor may ignore it.
    BestEffort,
    /// Cannot work; the UI suggests a workaround.
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub os: String,
    pub session_type: String,
    pub desktop: String,
    pub compositor: String,
    pub nvidia: bool,
    pub nvidia_driver: Option<String>,
    pub always_on_top: Support,
    pub click_through: Support,
    pub global_shortcuts: Support,
    pub window_positioning: Support,
    pub notes: Vec<String>,
}

pub fn is_wayland() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
    let forced_x11 = std::env::var("GDK_BACKEND")
        .map(|value| value.trim().eq_ignore_ascii_case("x11"))
        .unwrap_or(false);
    wayland && !forced_x11
}

pub fn nvidia_driver_version() -> Option<String> {
    let contents = std::fs::read_to_string(Path::new("/proc/driver/nvidia/version")).ok()?;
    let first_line = contents.lines().next()?;
    // "NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.107.02  ..."
    first_line
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()) && part.contains('.'))
        .map(str::to_string)
}

pub fn has_nvidia_gpu() -> bool {
    Path::new("/proc/driver/nvidia/version").exists()
        || Path::new("/dev/nvidia0").exists()
        || Path::new("/dev/nvidiactl").exists()
}

pub fn desktop_name() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn compositor_name() -> String {
    if is_wayland() {
        match std::env::var("XDG_CURRENT_DESKTOP") {
            Ok(desktop) if desktop.to_ascii_uppercase().contains("KDE") => {
                "KWin (Wayland)".to_string()
            }
            Ok(desktop) if desktop.to_ascii_uppercase().contains("GNOME") => {
                "Mutter (Wayland)".to_string()
            }
            Ok(desktop) if desktop.to_ascii_uppercase().contains("HYPRLAND") => {
                "Hyprland".to_string()
            }
            _ => "Wayland compositor".to_string(),
        }
    } else if std::env::var_os("DISPLAY").is_some() {
        "X11 / XWayland".to_string()
    } else {
        "unknown".to_string()
    }
}

pub fn platform_info() -> PlatformInfo {
    let wayland = is_wayland();
    let nvidia = has_nvidia_gpu();
    let dmabuf_disabled = std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER")
        .map(|value| value == "1")
        .unwrap_or(false);

    let mut notes: Vec<String> = Vec::new();

    if wayland {
        notes.push(
            "Wayland has no client-side \"always on top\". KWin honours a window rule instead — see LINUX.md."
                .to_string(),
        );
        notes.push(
            "Global hotkeys are unavailable on Wayland; bind `discord-linux-overlay --toggle-click-through` to a KDE shortcut."
                .to_string(),
        );
    }

    if nvidia {
        notes.push(
            "NVIDIA detected: WebKitGTK's DMA-BUF renderer is known to fail on some driver/compositor combinations. This build sets __NV_DISABLE_EXPLICIT_SYNC=1 on Wayland to avoid the KWin explicit-sync protocol error."
                .to_string(),
        );
    }

    if dmabuf_disabled {
        notes.push(
            "WEBKIT_DISABLE_DMABUF_RENDERER is set: rendering is software-composited (reliable, but no GPU acceleration)."
                .to_string(),
        );
    }

    PlatformInfo {
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        session_type: if wayland {
            "wayland".to_string()
        } else {
            "x11".to_string()
        },
        desktop: desktop_name(),
        compositor: compositor_name(),
        nvidia,
        nvidia_driver: nvidia_driver_version(),
        always_on_top: if wayland {
            Support::BestEffort
        } else {
            Support::Supported
        },
        click_through: if wayland {
            Support::BestEffort
        } else {
            Support::Supported
        },
        global_shortcuts: if wayland {
            Support::Unsupported
        } else {
            Support::Supported
        },
        window_positioning: if wayland {
            Support::Unsupported
        } else {
            Support::Supported
        },
        notes,
    }
}

/// Apply the graphics mitigations that are safe to decide automatically.
///
/// This runs *before* GTK/WebKit initialisation, because both variables are read
/// once by the graphics stack. Everything is logged, and
/// `DLO_NO_GRAPHICS_WORKAROUNDS=1` disables it.
pub fn apply_graphics_workarounds(config: &discord_overlay_core::config::Config) {
    if std::env::var("DLO_NO_GRAPHICS_WORKAROUNDS")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return;
    }

    let nvidia = has_nvidia_gpu();
    let wayland = is_wayland();

    // WebKit bug 324551 / Tauri #10702: on KWin Wayland the DMA-BUF renderer
    // violates the linux-drm-syncobj protocol ("explicit sync is used, but no
    // acquire point is set") and the compositor kills the client. Disabling
    // explicit sync keeps the fast renderer and fixes the crash.
    if nvidia && wayland && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        log::info!("NVIDIA + Wayland detected: setting __NV_DISABLE_EXPLICIT_SYNC=1");
    }

    // Opt-in fallback for the "Failed to create GBM buffer: Invalid argument"
    // path (X11 / XWayland on NVIDIA), where WebKitGTK cannot map the buffer.
    if config.general.disable_dmabuf_renderer
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        log::info!("disable_dmabuf_renderer enabled: setting WEBKIT_DISABLE_DMABUF_RENDERER=1");
    }
}
