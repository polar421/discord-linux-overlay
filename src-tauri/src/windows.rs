//! Window management.
//!
//! The overlay is a native GTK window (see `native_overlay.rs`). The settings
//! panel is the only WebKit web view, and it is created on demand and destroyed
//! on close so idle memory stays low.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use discord_overlay_core::config::Config;

pub const SETTINGS_LABEL: &str = "settings";

/// Incremented every time settings is opened. A cleanup task only terminates
/// WebKit children if no newer settings window was opened in the meantime.
static SETTINGS_GENERATION: AtomicU64 = AtomicU64::new(0);

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

pub fn settings_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(SETTINGS_LABEL)
}

/// Open the settings panel, creating its webview only on first use.
///
/// Keeping this window out of `tauri.conf.json` avoids a WebKit web process and
/// network process for users who never open Settings. Closing the window
/// destroys the webview again; the configuration itself lives on disk, so no
/// state is lost.
pub fn open_settings_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    SETTINGS_GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Some(window) = settings_window(app) {
        window.show().map_err(|error| error.to_string())?;
        let _ = window.set_focus();
        return Ok(window);
    }

    WebviewWindowBuilder::new(app, SETTINGS_LABEL, WebviewUrl::App("settings.html".into()))
        .title("Discord Linux Overlay — Settings")
        .inner_size(780.0, 700.0)
        .min_inner_size(640.0, 520.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|error| error.to_string())
}

pub fn monitor_list(app: &AppHandle) -> Result<Vec<MonitorInfo>, String> {
    crate::native_overlay::monitor_list(app)
}

pub fn apply_placement(app: &AppHandle, config: &Config) -> Result<(), String> {
    crate::native_overlay::apply_placement(app, config)
}

pub fn apply_behavior(app: &AppHandle, config: &Config) {
    crate::native_overlay::apply_behavior(app, config);
}

pub fn show_overlay(app: &AppHandle) {
    crate::native_overlay::show(app);
}

pub fn hide_overlay(app: &AppHandle) {
    crate::native_overlay::hide(app);
}

pub fn is_visible(app: &AppHandle) -> bool {
    crate::native_overlay::is_visible(app)
}

pub fn overlay_diagnostics(app: &AppHandle) -> Vec<String> {
    crate::native_overlay::describe(app)
}

/// WebKitGTK keeps a suspended web process (and its network process) in a pool
/// after the last web view is destroyed. That is harmless for a browser but
/// costs ~180 MiB here, so after closing Settings we terminate the WebKit
/// children of this process. The next Settings window starts fresh ones.
///
/// Only descendants whose executable is WebKit's own helper are signalled, and
/// only if no Settings window was opened again while waiting.
pub fn release_webkit_children(app: &AppHandle) {
    if !app.webview_windows().is_empty() {
        return;
    }
    let generation = SETTINGS_GENERATION.load(Ordering::SeqCst);
    let root = std::process::id();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(900));
        if SETTINGS_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let pids = webkit_descendants(root);
        if pids.is_empty() {
            return;
        }
        for pid in &pids {
            // SIGTERM lets WebKit shut down its IPC and child processes cleanly.
            unsafe {
                libc::kill(*pid as i32, libc::SIGTERM);
            }
        }
        std::thread::sleep(Duration::from_millis(400));
        for pid in pids {
            if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
                if exe.to_string_lossy().contains("WebKit") {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
            }
        }
        // WebKit leaves large freed heap arenas behind in the UI process.
        // Returning them to the kernel is safe here: Settings just closed and
        // the next paint is a simple native overlay frame.
        unsafe {
            libc::malloc_trim(0);
        }
    });
}

fn webkit_descendants(root: u32) -> Vec<u32> {
    use std::collections::HashMap;

    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // The command name is parenthesised and may contain spaces; the parent
        // PID is the field after the final `)`.
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let mut fields = stat[close + 1..].split_whitespace();
        let _state = fields.next();
        let Some(ppid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        children.entry(ppid).or_default().push(pid);
    }

    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(parent) = stack.pop() {
        if let Some(kids) = children.get(&parent) {
            for kid in kids {
                if let Ok(exe) = std::fs::read_link(format!("/proc/{kid}/exe")) {
                    if exe.to_string_lossy().contains("WebKit") {
                        result.push(*kid);
                    }
                }
                stack.push(*kid);
            }
        }
    }
    result
}
