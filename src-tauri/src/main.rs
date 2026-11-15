#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod control;
mod diagnose;
mod discord;
mod native_overlay;
mod platform;
mod state;
mod tray;
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};

use discord_overlay_core::control::{parse_args, Request, HELP};
use state::AppState;
use tauri::{Emitter, Listener, Manager, WindowEvent};

/// Set right before an intentional shutdown, so the windowless app is allowed
/// to leave its event loop.
pub static QUITTING: AtomicBool = AtomicBool::new(false);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");
        return;
    }

    // Diagnostics are also useful before anything is running, so they are
    // handled before the usual single-instance hand-off.
    if args
        .iter()
        .any(|arg| arg == "--diagnose" || arg == "diagnose")
    {
        match control::send_request(Request::Diagnose) {
            Ok(response) => println!("{}", response.message),
            Err(_) => println!("{}", diagnose::offline_report()),
        }
        return;
    }

    // CLI / KDE shortcut mode: talk to the running instance and exit.
    if let Some(request) = parse_args(&args) {
        match control::send_request(request) {
            Ok(response) => {
                println!("{}", response.message);
                if !response.ok {
                    std::process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("discord-linux-overlay: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let start_hidden = args.iter().any(|arg| arg == "--hidden");

    // Second launch: open the settings window of the running instance instead of
    // starting a second overlay.
    if control::send_request(Request::OpenSettings).is_ok() {
        return;
    }

    let state = AppState::new(start_hidden);
    platform::apply_graphics_workarounds(&state.config());
    state.load_credentials();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::update_config,
            commands::get_overlay_state,
            commands::get_status,
            commands::get_connection,
            commands::set_credentials,
            commands::clear_credentials,
            commands::reconnect_discord,
            commands::reauthorize_discord,
            commands::sign_out_discord,
            commands::set_click_through,
            commands::toggle_click_through,
            commands::set_always_on_top,
            commands::set_overlay_visible,
            commands::toggle_overlay,
            commands::set_locked,
            commands::reset_position,
            commands::apply_placement_now,
            commands::get_monitors,
            commands::open_settings,
            commands::close_settings,
            commands::get_platform,
            commands::get_logs,
            commands::clear_join_history,
            commands::open_config_dir,
            commands::quit_app,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let config = app.state::<AppState>().config();
            let overlay_state = state::lock(&app.state::<AppState>().overlay).clone();
            let status = state::lock(&app.state::<AppState>().status).clone();

            native_overlay::init(&handle, &config, &overlay_state, &status)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;

            if let Err(error) = windows::apply_placement(&handle, &config) {
                log::warn!("could not apply the saved overlay placement: {error}");
            }

            if config.general.start_hidden {
                windows::hide_overlay(&handle);
            }

            for event in ["config:changed", "overlay:state", "connection:status"] {
                let listener = handle.clone();
                handle.listen(event, move |_| native_overlay::refresh(&listener));
            }

            if let Err(error) = tray::create(&handle) {
                log::warn!("could not create the tray icon: {error}");
            }

            control::start_server(handle.clone());
            discord::start(handle.clone());

            let _ = handle.emit("config:changed", &config);
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) && window.label() == windows::SETTINGS_LABEL
            {
                windows::release_webkit_children(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start Discord Linux Overlay")
        .run(|_app, event| {
            // The overlay is a native GTK window owned by this process, and the
            // settings webview only exists on demand. Tauri sees zero webview
            // windows, so keep the event loop alive until quit is requested.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !QUITTING.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
