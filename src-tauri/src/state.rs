//! Application state shared by commands, tray, control socket and the Discord
//! worker thread.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use discord_overlay_core::client::{DiscordClient, FileTokenStore, Status};
use discord_overlay_core::config::{config_dir, Config, ConfigStore};
use discord_overlay_core::credentials::{CredentialsStore, DiscordCredentials};
use discord_overlay_core::state::OverlayState;

pub const MAX_LOG_LINES: usize = 250;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct AppState {
    pub config_store: ConfigStore,
    pub credentials_store: CredentialsStore,
    pub token_store: FileTokenStore,
    pub config: Mutex<Config>,
    pub credentials: Mutex<DiscordCredentials>,
    pub overlay: Mutex<OverlayState>,
    pub status: Mutex<Status>,
    pub logs: Mutex<VecDeque<String>>,
    pub client: Mutex<Option<DiscordClient>>,
    /// Rate limits how often window moves are written to disk.
    pub last_bounds_save: Mutex<Instant>,
}

impl AppState {
    pub fn new(start_hidden: bool) -> Self {
        let dir = config_dir();
        let config_store = ConfigStore::new(dir.join("config.json"));
        let mut config = config_store.load();
        if start_hidden {
            // `--hidden` (used by the autostart entry) applies to this run only.
            config.general.start_hidden = true;
        }

        Self {
            credentials_store: CredentialsStore::new(dir.join("credentials.json")),
            token_store: FileTokenStore::new(dir.join("discord-token.json")),
            config: Mutex::new(config),
            credentials: Mutex::new(DiscordCredentials::default()),
            overlay: Mutex::new(OverlayState::default()),
            status: Mutex::new(Status::Starting),
            logs: Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES)),
            client: Mutex::new(None),
            last_bounds_save: Mutex::new(Instant::now() - std::time::Duration::from_secs(5)),
            config_store,
        }
    }

    pub fn load_credentials(&self) {
        *lock(&self.credentials) = self.credentials_store.load();
    }

    pub fn config(&self) -> Config {
        lock(&self.config).clone()
    }

    pub fn directory(&self) -> PathBuf {
        config_dir()
    }

    pub fn push_log(&self, line: impl Into<String>) {
        let line = line.into();
        log::info!("{line}");
        let mut logs = lock(&self.logs);
        if logs.len() >= MAX_LOG_LINES {
            logs.pop_front();
        }
        logs.push_back(line);
    }

    pub fn stop_client(&self) {
        if let Some(mut client) = lock(&self.client).take() {
            client.stop();
        }
    }
}
