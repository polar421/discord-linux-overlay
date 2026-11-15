#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod control;
mod discord;
mod platform;
mod state;
mod tray;
mod windows;

use discord_overlay_core::control::{parse_args, Request, HELP};
use state::AppState;
use tauri::{Emitter, Manager, WindowEvent};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");
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

            if let Some(window) = windows::overlay_window(&handle) {
                if let Err(error) = window.set_always_on_top(config.behavior.always_on_top) {
                    log::debug!("always-on-top unavailable: {error}");
                }
                if let Err(error) = window.set_ignore_cursor_events(config.behavior.click_through) {
                    log::debug!("click-through unavailable: {error}");
                }
                if let Err(error) = window.set_resizable(!config.behavior.lock_position) {
                    log::debug!("resizability unavailable: {error}");
                }
                let _ = window.set_skip_taskbar(config.behavior.click_through);
            }

            if let Err(error) = windows::apply_placement(&handle, &config) {
                log::warn!("could not apply the saved overlay placement: {error}");
            }

            if config.general.start_hidden {
                windows::hide_overlay(&handle);
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
            let app = window.app_handle();

            match event {
                WindowEvent::CloseRequested { api, .. }
                    if window.label() == windows::SETTINGS_LABEL =>
                {
                    // The settings window is a singleton panel; hiding it keeps
                    // the app (and the overlay) alive.
                    api.prevent_close();
                    let _ = window.hide();
                }
                WindowEvent::Moved(_) | WindowEvent::Resized(_)
                    if window.label() == windows::OVERLAY_LABEL =>
                {
                    let state = app.state::<AppState>();
                    let mut last_save = state::lock(&state.last_bounds_save);
                    if last_save.elapsed() > std::time::Duration::from_millis(500) {
                        *last_save = std::time::Instant::now();
                        drop(last_save);
                        if let Some(overlay) = windows::overlay_window(&app) {
                            windows::remember_bounds(&app, &overlay);
                        }
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start Discord Linux Overlay")
        .run(|_app, _event| {});
}
