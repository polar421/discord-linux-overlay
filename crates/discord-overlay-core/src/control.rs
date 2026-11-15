//! The tiny command channel used by the `discord-linux-overlay` CLI.
//!
//! Wayland compositors do not let an application register global hotkeys (and
//! many tiling setups have no tray), so the running instance also listens on a
//! per-user Unix socket. KDE, Hyprland or a shell alias can then call:
//!
//! ```sh
//! discord-linux-overlay --toggle-click-through
//! ```
//!
//! This is deliberately a very small surface: parse a request, send JSON, read
//! a JSON reply.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Request {
    Show,
    Hide,
    ToggleVisibility,
    ToggleClickThrough,
    Pin {
        value: bool,
    },
    OpenSettings,
    CloseSettings,
    ResetPosition,
    ToggleLock,
    Status,
    /// Full diagnostic report, including runtime state of the running instance.
    Diagnose,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub ok: bool,
    pub message: String,
}

impl Response {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

pub const HELP: &str = "\
Discord Linux Overlay

Usage:
  discord-linux-overlay              Start the overlay (or focus the running instance)
  discord-linux-overlay --show       Show the overlay
  discord-linux-overlay --hide       Hide the overlay
  discord-linux-overlay --toggle     Toggle overlay visibility
  discord-linux-overlay --pin on|off Enable or disable click-through mode
  discord-linux-overlay --toggle-click-through
  discord-linux-overlay --settings   Open the settings window
  discord-linux-overlay --close-settings  Close (and release) the settings window
  discord-linux-overlay --reset-position
  discord-linux-overlay --toggle-lock
  discord-linux-overlay --status     Print the connection status of a running instance
  discord-linux-overlay --diagnose   Print a diagnostic report (works without a running instance)
  discord-linux-overlay --quit       Quit the running instance
  discord-linux-overlay --help       Show this help

Commands talk to the running instance over a Unix socket, which is what makes
them usable as KDE global shortcuts on Wayland.";

/// Parse CLI arguments (without the program name).
pub fn parse_args(args: &[String]) -> Option<Request> {
    let mut iterator = args.iter().map(String::as_str).peekable();

    match iterator.next()? {
        "--show" | "show" => Some(Request::Show),
        "--hide" | "hide" => Some(Request::Hide),
        "--toggle" | "--toggle-visibility" | "toggle" => Some(Request::ToggleVisibility),
        "--toggle-click-through" | "toggle-click-through" => Some(Request::ToggleClickThrough),
        "--settings" | "settings" => Some(Request::OpenSettings),
        "--close-settings" | "close-settings" => Some(Request::CloseSettings),
        "--reset-position" | "reset-position" => Some(Request::ResetPosition),
        "--toggle-lock" | "toggle-lock" => Some(Request::ToggleLock),
        "--status" | "status" => Some(Request::Status),
        "--diagnose" | "diagnose" => Some(Request::Diagnose),
        "--quit" | "quit" => Some(Request::Quit),
        "--pin" | "pin" => {
            let value = iterator.next()?;
            match value {
                "on" | "true" | "1" => Some(Request::Pin { value: true }),
                "off" | "false" | "0" => Some(Request::Pin { value: false }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Socket path for the current user, mirroring the config layout.
use std::path::PathBuf;

pub fn control_socket_path() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(runtime).join("discord-linux-overlay/control.sock");
    }

    let uid = unsafe_get_uid();
    std::env::temp_dir().join(format!("discord-linux-overlay-{uid}.sock"))
}

fn unsafe_get_uid() -> u32 {
    // `std` has no portable uid accessor; reading /proc is enough on Linux and
    // falls back to a stable placeholder if that fails.
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("Uid:") {
                if let Some(uid) = value.split_whitespace().next() {
                    if let Ok(uid) = uid.parse() {
                        return uid;
                    }
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_the_documented_switches() {
        assert_eq!(parse_args(&args(&["--show"])), Some(Request::Show));
        assert_eq!(parse_args(&args(&["--hide"])), Some(Request::Hide));
        assert_eq!(
            parse_args(&args(&["--toggle"])),
            Some(Request::ToggleVisibility)
        );
        assert_eq!(
            parse_args(&args(&["--toggle-click-through"])),
            Some(Request::ToggleClickThrough)
        );
        assert_eq!(
            parse_args(&args(&["--settings"])),
            Some(Request::OpenSettings)
        );
        assert_eq!(
            parse_args(&args(&["--close-settings"])),
            Some(Request::CloseSettings)
        );
        assert_eq!(
            parse_args(&args(&["--reset-position"])),
            Some(Request::ResetPosition)
        );
        assert_eq!(
            parse_args(&args(&["--toggle-lock"])),
            Some(Request::ToggleLock)
        );
        assert_eq!(parse_args(&args(&["--status"])), Some(Request::Status));
        assert_eq!(parse_args(&args(&["--diagnose"])), Some(Request::Diagnose));
        assert_eq!(parse_args(&args(&["--quit"])), Some(Request::Quit));
        assert_eq!(parse_args(&args(&["--help"])), None);
        assert_eq!(parse_args(&[]), None);
    }

    #[test]
    fn parses_pin_values() {
        assert_eq!(
            parse_args(&args(&["--pin", "on"])),
            Some(Request::Pin { value: true })
        );
        assert_eq!(
            parse_args(&args(&["--pin", "off"])),
            Some(Request::Pin { value: false })
        );
        assert_eq!(parse_args(&args(&["--pin"])), None);
        assert_eq!(parse_args(&args(&["--pin", "maybe"])), None);
    }

    #[test]
    fn requests_serialize_with_a_command_tag() {
        let json = serde_json::to_value(Request::ToggleClickThrough).unwrap();
        assert_eq!(json["command"], "toggle-click-through");

        let pin = serde_json::to_value(Request::Pin { value: true }).unwrap();
        assert_eq!(pin["command"], "pin");
        assert_eq!(pin["value"], true);

        let parsed: Request = serde_json::from_value(json!({ "command": "show" })).unwrap();
        assert_eq!(parsed, Request::Show);
    }

    #[test]
    fn control_socket_lives_under_the_runtime_dir() {
        // The function depends on process environment; assert the shape only.
        let path = control_socket_path();
        assert!(path.to_string_lossy().contains("discord-linux-overlay"));
    }
}
