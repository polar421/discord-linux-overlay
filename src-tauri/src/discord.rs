//! Bridges the Discord client thread to the web views.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use discord_overlay_core::client::{ClientEvent, ClientOptions, DiscordClient, Status};
use discord_overlay_core::state::OverlayState;
use discord_overlay_core::token::HttpTokenExchange;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{lock, AppState};
use crate::windows;

pub fn restart(app: &AppHandle) {
    app.state::<AppState>().stop_client();
    start(app.clone());
}

pub fn start(app: AppHandle) {
    let state = app.state::<AppState>();
    let credentials = lock(&state.credentials).clone();
    let token_store = state.token_store.clone();

    if !credentials.is_configured() {
        let status = Status::NeedsSetup {
            message: "Add your Discord application credentials in Settings → Discord.".to_string(),
        };
        *lock(&state.status) = status.clone();
        let overlay = OverlayState::default();
        *lock(&state.overlay) = overlay.clone();
        let _ = app.emit("connection:status", &status);
        let _ = app.emit("overlay:state", &overlay);
        return;
    }

    let exchange = Arc::new(HttpTokenExchange::new(
        credentials.client_id(),
        credentials.client_secret(),
        credentials.redirect_uri().map(str::to_string),
    ));
    let store = Arc::new(token_store);
    let (sender, receiver) = mpsc::channel::<ClientEvent>();

    let client = DiscordClient::start(
        ClientOptions::new(credentials.client_id()),
        exchange,
        store,
        sender,
    );
    *lock(&state.client) = Some(client);

    let spawn_result = thread::Builder::new()
        .name("discord-events".to_string())
        .spawn(move || {
            for event in receiver {
                let state = app.state::<AppState>();

                match event {
                    ClientEvent::Status(status) => {
                        *lock(&state.status) = status.clone();
                        let _ = app.emit("connection:status", &status);
                    }
                    ClientEvent::State(overlay) => {
                        let previous_channel = lock(&state.overlay).channel.clone();
                        *lock(&state.overlay) = (*overlay).clone();
                        let _ = app.emit("overlay:state", &*overlay);

                        let config = state.config();
                        let joined = previous_channel.is_none() && overlay.channel.is_some();
                        let left = previous_channel.is_some() && overlay.channel.is_none();

                        if joined && config.general.show_overlay_on_join {
                            windows::show_overlay(&app);
                        }
                        if left && config.general.hide_overlay_on_leave {
                            windows::hide_overlay(&app);
                        }
                    }
                    ClientEvent::AuthRequired => {
                        state.push_log(
                            "Discord is asking for authorization — approve the prompt in your Discord client.",
                        );
                        let _ = app.emit("discord:auth-required", ());
                    }
                    ClientEvent::Log(line) => {
                        state.push_log(line.clone());
                        let _ = app.emit("app:log", line);
                    }
                }
            }
        });

    if let Err(error) = spawn_result {
        log::error!("could not start the Discord event bridge: {error}");
    }
}
