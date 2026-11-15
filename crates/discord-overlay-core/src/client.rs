//! Discord IPC client: connection lifecycle, authentication, subscriptions and
//! reconnection.
//!
//! One background thread owns the socket. It is entirely event driven: Discord
//! pushes `SPEAKING_START` / `SPEAKING_STOP` / `VOICE_STATE_*` frames, so the
//! overlay never polls. A 20 s `GET_SELECTED_VOICE_CHANNEL` resync exists only
//! to catch lost events, and a missing reply twice in a row is treated as a dead
//! socket (Discord does not always notice a suspended laptop).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::avatar;
use crate::config::write_private_json;
use crate::error::Result;
use crate::ipc::{self, Frame, IpcConnection, OP_CLOSE, OP_FRAME, OP_PING, OP_PONG};
use crate::protocol::{
    command, error_code, event, subscription_payload, Channel, RpcMessage, User, VoiceStateUser,
};
use crate::state::{ChannelInfo, OverlayState, Participant};

/// Milliseconds since the Unix epoch; used for join-history timestamps.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Connection status rendered by the settings window and the overlay badge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    Starting,
    #[serde(rename_all = "camelCase")]
    WaitingForDiscord {
        message: String,
    },
    Connecting,
    Authorizing,
    Authenticating,
    #[serde(rename_all = "camelCase")]
    Ready {
        user_name: String,
    },
    #[serde(rename_all = "camelCase")]
    NeedsSetup {
        message: String,
    },
    Stopped,
}

impl Status {
    pub fn is_ready(&self) -> bool {
        matches!(self, Status::Ready { .. })
    }
}

/// Everything the client thread reports to its owner.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    Status(Status),
    State(Box<OverlayState>),
    /// Discord wants the user to approve the application (the authorization
    /// prompt was opened in the Discord client).
    AuthRequired,
    Log(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientCommand {
    /// Ask for a fresh channel snapshot.
    Refresh,
    /// Forget the access token and run the authorization flow again.
    Reauthorize,
    SignOut,
    Stop,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Token {
    pub access_token: String,
    /// Unix milliseconds, when Discord reported an expiry.
    #[serde(default)]
    pub expires_at: Option<u64>,
}

/// Turns an OAuth2 authorization code into an access token.
pub trait TokenExchange: Send + Sync + 'static {
    fn exchange(&self, code: &str) -> Result<Token>;
}

/// Caches the access token between runs.
pub trait TokenStore: Send + Sync + 'static {
    fn load(&self) -> Option<Token>;
    fn save(&self, token: &Token);
    fn clear(&self);
}

/// Token cache backed by a `0600` JSON file.
#[derive(Debug, Clone)]
pub struct FileTokenStore {
    path: PathBuf,
}

impl FileTokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> Option<Token> {
        let contents = std::fs::read_to_string(&self.path).ok()?;
        let token: Token = serde_json::from_str(&contents).ok()?;
        if token.access_token.trim().is_empty() {
            None
        } else {
            Some(token)
        }
    }

    fn save(&self, token: &Token) {
        let value = match serde_json::to_value(token) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("could not serialise token: {error}");
                return;
            }
        };
        if let Err(error) = write_private_json(&self.path, &value) {
            log::warn!("could not persist token: {error}");
        }
    }

    fn clear(&self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not remove token file: {error}");
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub client_id: String,
    /// Overrides socket discovery (tests, custom clients).
    pub socket_paths: Option<Vec<PathBuf>>,
    pub initial_token: Option<Token>,
    pub resync_interval: Duration,
    pub resync_timeout: Duration,
    pub ping_interval: Duration,
    pub reconnect_delays: Vec<Duration>,
}

impl ClientOptions {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            socket_paths: None,
            initial_token: None,
            resync_interval: Duration::from_secs(20),
            resync_timeout: Duration::from_secs(5),
            ping_interval: Duration::from_secs(30),
            reconnect_delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(5),
                Duration::from_secs(10),
                Duration::from_secs(20),
                Duration::from_secs(30),
            ],
        }
    }
}

/// Handle to the background client thread.
pub struct DiscordClient {
    commands: Sender<ClientCommand>,
    handle: Option<JoinHandle<()>>,
}

impl DiscordClient {
    pub fn start(
        options: ClientOptions,
        exchange: Arc<dyn TokenExchange>,
        store: Arc<dyn TokenStore>,
        events: Sender<ClientEvent>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("discord-ipc".to_string())
            .spawn(move || run_client(options, exchange, store, events, receiver))
            .ok();

        Self {
            commands: sender,
            handle,
        }
    }

    pub fn send(&self, command: ClientCommand) {
        let _ = self.commands.send(command);
    }

    pub fn refresh(&self) {
        self.send(ClientCommand::Refresh);
    }

    pub fn reauthorize(&self) {
        self.send(ClientCommand::Reauthorize);
    }

    pub fn sign_out(&self) {
        self.send(ClientCommand::SignOut);
    }

    pub fn stop(&mut self) {
        let _ = self.commands.send(ClientCommand::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DiscordClient {
    fn drop(&mut self) {
        self.stop();
    }
}

enum SessionEnd {
    Stop,
    Reconnect { reason: String, was_ready: bool },
    SetupRequired(String),
}

fn delay_for(delays: &[Duration], index: usize) -> Duration {
    if delays.is_empty() {
        return Duration::from_secs(5);
    }
    delays[index.min(delays.len() - 1)]
}

/// Sleep in small slices so a `Stop` command is honoured promptly.
fn sleep_with_commands(duration: Duration, commands: &Receiver<ClientCommand>) -> bool {
    let deadline = Instant::now() + duration;

    loop {
        match commands.try_recv() {
            Ok(ClientCommand::Stop) | Err(TryRecvError::Disconnected) => return false,
            Ok(_) => {}
            Err(TryRecvError::Empty) => {}
        }

        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        thread::sleep((deadline - now).min(Duration::from_millis(100)));
    }
}

fn run_client(
    options: ClientOptions,
    exchange: Arc<dyn TokenExchange>,
    store: Arc<dyn TokenStore>,
    events: Sender<ClientEvent>,
    commands: Receiver<ClientCommand>,
) {
    let mut state = OverlayState::default();
    let mut token = options.initial_token.clone().or_else(|| store.load());
    let mut delay_index = 0usize;
    let mut waiting_announced = false;

    let _ = events.send(ClientEvent::Status(Status::Starting));

    if options.client_id.trim().is_empty() {
        let _ = events.send(ClientEvent::Status(Status::NeedsSetup {
            message: "No Discord application configured. Add your client id and client secret in Settings → Discord."
                .to_string(),
        }));
        return;
    }

    loop {
        let paths = options
            .socket_paths
            .clone()
            .unwrap_or_else(ipc::discover_socket_paths);

        match IpcConnection::connect_any(&paths) {
            Err(error) => {
                state.on_disconnect();
                let _ = events.send(ClientEvent::State(Box::new(state.clone())));

                if !waiting_announced {
                    let _ = events.send(ClientEvent::Status(Status::WaitingForDiscord {
                        message: error.to_string(),
                    }));
                    waiting_announced = true;
                }

                let delay = delay_for(&options.reconnect_delays, delay_index);
                delay_index += 1;
                if !sleep_with_commands(delay, &commands) {
                    break;
                }
            }
            Ok(connection) => {
                waiting_announced = false;

                let mut session = Session {
                    options: &options,
                    exchange: &exchange,
                    store: &store,
                    events: &events,
                    state: &mut state,
                    token: &mut token,
                    connection,
                    authenticated: false,
                    subscribed_channel: None,
                    resync_nonce: None,
                    resync_deadline: None,
                    missed_resyncs: 0,
                    last_resync: Instant::now(),
                    last_ping: Instant::now(),
                    nonce: 0,
                    last_sent_revision: None,
                    was_ready: false,
                };

                match session.run(&commands) {
                    SessionEnd::Stop => break,
                    SessionEnd::Reconnect { reason, was_ready } => {
                        let _ = events.send(ClientEvent::Log(format!(
                            "Discord IPC connection lost: {reason}"
                        )));
                        state.on_disconnect();
                        let _ = events.send(ClientEvent::State(Box::new(state.clone())));

                        if was_ready {
                            delay_index = 0;
                        }
                        let delay = delay_for(&options.reconnect_delays, delay_index);
                        delay_index += 1;
                        if !sleep_with_commands(delay, &commands) {
                            break;
                        }
                    }
                    SessionEnd::SetupRequired(message) => {
                        let _ = events.send(ClientEvent::Status(Status::NeedsSetup {
                            message: message.clone(),
                        }));
                        state.on_disconnect();
                        let _ = events.send(ClientEvent::State(Box::new(state.clone())));
                        // The owner restarts the client when credentials change;
                        // poll slowly so a fixed configuration is picked up even
                        // if no restart happened.
                        if !sleep_with_commands(Duration::from_secs(60), &commands) {
                            break;
                        }
                    }
                }
            }
        }
    }

    let _ = events.send(ClientEvent::Status(Status::Stopped));
}

struct Session<'a> {
    options: &'a ClientOptions,
    exchange: &'a Arc<dyn TokenExchange>,
    store: &'a Arc<dyn TokenStore>,
    events: &'a Sender<ClientEvent>,
    state: &'a mut OverlayState,
    token: &'a mut Option<Token>,
    connection: IpcConnection,
    authenticated: bool,
    subscribed_channel: Option<String>,
    resync_nonce: Option<String>,
    resync_deadline: Option<Instant>,
    missed_resyncs: u8,
    last_resync: Instant,
    last_ping: Instant,
    nonce: u64,
    last_sent_revision: Option<u64>,
    was_ready: bool,
}

impl<'a> Session<'a> {
    fn run(&mut self, commands: &Receiver<ClientCommand>) -> SessionEnd {
        if let Err(error) = self.connection.handshake(self.options.client_id.trim()) {
            return SessionEnd::Reconnect {
                reason: format!("handshake failed: {error}"),
                was_ready: false,
            };
        }

        self.emit_status(Status::Connecting);

        loop {
            if let Some(end) = self.drain_commands(commands) {
                return end;
            }

            match self.connection.try_recv() {
                Ok(Some(frame)) => {
                    let end = self.handle_frame(frame);
                    self.emit_state();
                    if let Some(end) = end {
                        return end;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.emit_state();
                    return SessionEnd::Reconnect {
                        reason: error.to_string(),
                        was_ready: self.was_ready,
                    };
                }
            }

            if let Some(end) = self.tick() {
                return end;
            }
        }
    }

    fn next_nonce(&mut self, prefix: &str) -> String {
        self.nonce += 1;
        format!("{prefix}:{}", self.nonce)
    }

    fn log(&self, message: impl Into<String>) {
        let message = message.into();
        log::debug!("{message}");
        let _ = self.events.send(ClientEvent::Log(message));
    }

    fn emit_status(&self, status: Status) {
        let _ = self.events.send(ClientEvent::Status(status));
    }

    fn emit_state(&mut self) {
        if self.last_sent_revision == Some(self.state.revision) {
            return;
        }
        self.last_sent_revision = Some(self.state.revision);
        let _ = self
            .events
            .send(ClientEvent::State(Box::new(self.state.clone())));
    }

    fn send(&mut self, payload: Value) -> Result<()> {
        self.connection.send(OP_FRAME, &payload)
    }

    fn drain_commands(&mut self, commands: &Receiver<ClientCommand>) -> Option<SessionEnd> {
        loop {
            match commands.try_recv() {
                Ok(ClientCommand::Stop) | Err(TryRecvError::Disconnected) => {
                    return Some(SessionEnd::Stop)
                }
                Ok(ClientCommand::Refresh) => {
                    self.request_selected(false);
                }
                Ok(ClientCommand::Reauthorize) | Ok(ClientCommand::SignOut) => {
                    *self.token = None;
                    self.store.clear();
                    self.authenticated = false;
                    if let Some(channel) = self.subscribed_channel.take() {
                        self.unsubscribe_channel(&channel);
                    }
                    self.state.clear_channel();
                    self.emit_state();
                    self.emit_status(Status::Authorizing);
                    let _ = self.events.send(ClientEvent::AuthRequired);
                    if let Err(error) = self.authorize() {
                        return Some(SessionEnd::Reconnect {
                            reason: error.to_string(),
                            was_ready: self.was_ready,
                        });
                    }
                }
                Err(TryRecvError::Empty) => return None,
            }
        }
    }

    fn handle_frame(&mut self, frame: Frame) -> Option<SessionEnd> {
        match frame.op {
            OP_PING => {
                let _ = self.connection.send(OP_PONG, &frame.payload);
                None
            }
            OP_PONG => None,
            OP_CLOSE => Some(SessionEnd::Reconnect {
                reason: format!("Discord closed the connection ({})", frame.payload),
                was_ready: self.was_ready,
            }),
            OP_FRAME => self.handle_payload(frame.payload),
            other => {
                self.log(format!("ignoring unknown Discord IPC opcode {other}"));
                None
            }
        }
    }

    fn handle_payload(&mut self, payload: Value) -> Option<SessionEnd> {
        let message: RpcMessage = match serde_json::from_value(payload) {
            Ok(message) => message,
            Err(error) => {
                self.log(format!("ignoring malformed Discord payload: {error}"));
                return None;
            }
        };

        if message.evt.as_deref() == Some(event::ERROR) {
            return self.handle_error(&message);
        }

        if message.evt.as_deref() == Some(event::READY) {
            self.begin_auth();
            return None;
        }

        match message.cmd.as_deref() {
            Some(command::AUTHORIZE) => return self.finish_authorization(&message),
            Some(command::AUTHENTICATE) => {
                self.finish_authentication(&message);
                return None;
            }
            Some(command::GET_SELECTED_VOICE_CHANNEL) => {
                self.apply_selected_channel(&message);
                return None;
            }
            _ => {}
        }

        match message.evt.as_deref() {
            Some(event::SPEAKING_START) => {
                if let Some(user_id) = speaking_user_id(&message) {
                    self.state.set_speaking(&user_id, true);
                }
            }
            Some(event::SPEAKING_STOP) => {
                if let Some(user_id) = speaking_user_id(&message) {
                    self.state.set_speaking(&user_id, false);
                }
            }
            Some(event::VOICE_STATE_CREATE) => self.on_voice_state(&message, true),
            Some(event::VOICE_STATE_UPDATE) => self.on_voice_state(&message, false),
            Some(event::VOICE_STATE_DELETE) => self.on_voice_state_delete(&message),
            Some(event::VOICE_CHANNEL_SELECT) => self.on_voice_channel_select(&message),
            _ => {}
        }

        None
    }

    fn handle_error(&mut self, message: &RpcMessage) -> Option<SessionEnd> {
        let code = message.error_code().unwrap_or(0);
        let text = message.error_message();

        match message.cmd.as_deref() {
            Some(command::AUTHENTICATE) => {
                if error_code::requires_reauthorization(code) {
                    self.log(format!(
                        "Discord rejected the cached token ({code}: {text}); authorizing again"
                    ));
                    *self.token = None;
                    self.store.clear();
                    self.authenticated = false;
                    self.emit_status(Status::Authorizing);
                    let _ = self.events.send(ClientEvent::AuthRequired);
                    if let Err(error) = self.authorize() {
                        return Some(SessionEnd::Reconnect {
                            reason: error.to_string(),
                            was_ready: self.was_ready,
                        });
                    }
                    return None;
                }

                Some(SessionEnd::SetupRequired(format!(
                    "Discord refused the sign-in ({code}): {text}"
                )))
            }
            Some(command::AUTHORIZE) => Some(SessionEnd::SetupRequired(format!(
                "Discord refused the application authorization ({code}): {text}. \
                 Check the client id, and make sure your Discord application has RPC enabled."
            ))),
            Some(command::GET_SELECTED_VOICE_CHANNEL) => {
                if message.nonce.as_deref() == self.resync_nonce.as_deref() {
                    self.resync_nonce = None;
                    self.resync_deadline = None;
                    self.missed_resyncs += 1;
                    if self.missed_resyncs >= 2 {
                        return Some(SessionEnd::Reconnect {
                            reason: "Discord stopped answering channel resyncs".to_string(),
                            was_ready: self.was_ready,
                        });
                    }
                } else {
                    self.log(format!("Discord error {code}: {text}"));
                }
                None
            }
            _ => {
                self.log(format!("Discord error {code}: {text}"));
                None
            }
        }
    }

    fn begin_auth(&mut self) {
        match self.token.clone() {
            Some(token) if !token.access_token.trim().is_empty() => {
                self.emit_status(Status::Authenticating);
                self.authenticate(&token.access_token);
            }
            _ => {
                self.emit_status(Status::Authorizing);
                let _ = self.events.send(ClientEvent::AuthRequired);
                if let Err(error) = self.authorize() {
                    self.log(format!("could not send AUTHORIZE: {error}"));
                }
            }
        }
    }

    fn authorize(&mut self) -> Result<()> {
        let nonce = self.next_nonce("authorize");
        let payload = crate::protocol::command_payload(
            command::AUTHORIZE,
            &nonce,
            Some(json!({
                "client_id": self.options.client_id.trim(),
                "scopes": ["identify", "rpc"],
            })),
        );
        self.send(payload)
    }

    fn authenticate(&mut self, access_token: &str) {
        let nonce = self.next_nonce("authenticate");
        let payload = crate::protocol::command_payload(
            command::AUTHENTICATE,
            &nonce,
            Some(json!({ "access_token": access_token })),
        );
        if let Err(error) = self.send(payload) {
            self.log(format!("could not send AUTHENTICATE: {error}"));
        }
    }

    fn finish_authorization(&mut self, message: &RpcMessage) -> Option<SessionEnd> {
        let code = message
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string);

        let Some(code) = code else {
            self.log("Discord returned an authorization response without a code");
            return None;
        };

        match self.exchange.exchange(&code) {
            Ok(token) => {
                self.store.save(&token);
                *self.token = Some(token.clone());
                self.emit_status(Status::Authenticating);
                self.authenticate(&token.access_token);
                None
            }
            Err(error) => Some(SessionEnd::SetupRequired(format!(
                "could not finish Discord authorization: {error}"
            ))),
        }
    }

    fn finish_authentication(&mut self, message: &RpcMessage) {
        let Some(user_value) = message
            .data
            .as_ref()
            .and_then(|data| data.get("user"))
            .cloned()
        else {
            self.log("Discord authenticated without returning a user object");
            return;
        };

        let user: User = serde_json::from_value(user_value).unwrap_or_default();
        if user.id.is_empty() {
            self.log("Discord authenticated with an empty user id");
            return;
        }

        if let Some(token) = self.token.as_mut() {
            let expires = message
                .data
                .as_ref()
                .and_then(|data| data.get("expires"))
                .and_then(Value::as_str);
            if expires.is_some() && token.expires_at.is_none() {
                // Discord sent an expiry string; keep the token either way and
                // rely on INVALID_TOKEN if it goes stale.
                self.store.save(token);
            }
        }

        self.authenticated = true;
        self.missed_resyncs = 0;
        self.was_ready = true;

        let name = user.display_name().to_string();
        let participant = Participant {
            id: user.id.clone(),
            display_name: name.clone(),
            username: user.username.clone(),
            avatar_url: avatar::avatar_url(&user.id, user.avatar.as_deref(), 96),
            speaking: false,
            muted: false,
            deafened: false,
            self_muted: false,
            self_deafened: false,
            suppressed: false,
            bot: user.bot,
            is_self: true,
        };
        self.state.set_self(participant);

        self.emit_status(Status::Ready {
            user_name: name.clone(),
        });

        // Ask for VOICE_CHANNEL_SELECT so joining/leaving is instant, then read
        // the channel the user is already in (if any).
        let nonce = self.next_nonce("subscribe");
        let payload = subscription_payload(
            command::SUBSCRIBE,
            &nonce,
            event::VOICE_CHANNEL_SELECT,
            None,
        );
        if let Err(error) = self.send(payload) {
            self.log(format!("could not subscribe to channel selects: {error}"));
        }

        self.request_selected(false);
        self.emit_state();
    }

    fn request_selected(&mut self, resync: bool) {
        let nonce = if resync {
            let nonce = self.next_nonce("resync");
            self.resync_nonce = Some(nonce.clone());
            self.resync_deadline = Some(Instant::now() + self.options.resync_timeout);
            self.last_resync = Instant::now();
            nonce
        } else {
            let nonce = self.next_nonce("selected");
            self.last_resync = Instant::now();
            nonce
        };

        let payload =
            crate::protocol::command_payload(command::GET_SELECTED_VOICE_CHANNEL, &nonce, None);
        if let Err(error) = self.send(payload) {
            self.log(format!("could not request the current channel: {error}"));
        }
    }

    fn apply_selected_channel(&mut self, message: &RpcMessage) {
        let nonce = message.nonce.clone().unwrap_or_default();

        if nonce.starts_with("resync:") {
            if self.resync_nonce.as_deref() != Some(nonce.as_str()) {
                // A late reply to an earlier probe; ignore it.
                return;
            }
            self.resync_nonce = None;
            self.resync_deadline = None;
            self.missed_resyncs = 0;
        }

        let data = message.data.clone().unwrap_or(Value::Null);

        if data.is_null() || data.get("id").is_none() {
            if let Some(channel_id) = self.subscribed_channel.take() {
                self.unsubscribe_channel(&channel_id);
            }
            self.state.clear_channel();
            return;
        }

        let channel: Channel = match serde_json::from_value(data.clone()) {
            Ok(channel) => channel,
            Err(error) => {
                self.log(format!("could not parse the channel snapshot: {error}"));
                return;
            }
        };

        if channel.id.is_empty() {
            self.state.clear_channel();
            return;
        }

        let users: Vec<VoiceStateUser> = data
            .get("voice_states")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default();

        if self.subscribed_channel.as_deref() != Some(channel.id.as_str()) {
            if let Some(previous) = self.subscribed_channel.clone() {
                self.unsubscribe_channel(&previous);
            }
            self.subscribe_channel(&channel.id);
        }

        self.state
            .set_channel(Some(ChannelInfo::from_channel(channel)), users);
    }

    fn on_voice_channel_select(&mut self, message: &RpcMessage) {
        let channel_id = message
            .data
            .as_ref()
            .and_then(|data| data.get("channel_id"))
            .and_then(Value::as_str)
            .map(str::to_string);

        match channel_id {
            Some(channel_id) if !channel_id.is_empty() => self.request_selected(false),
            _ => {
                if let Some(channel_id) = self.subscribed_channel.take() {
                    self.unsubscribe_channel(&channel_id);
                }
                self.state.clear_channel();
            }
        }
    }

    fn on_voice_state(&mut self, message: &RpcMessage, created: bool) {
        let Some(data) = message.data.clone() else {
            return;
        };

        let voice: VoiceStateUser = match serde_json::from_value(data) {
            Ok(voice) => voice,
            Err(error) => {
                self.log(format!("could not parse a voice state: {error}"));
                return;
            }
        };

        let self_id = self.state.me.as_ref().map(|me| me.id.clone());
        if self_id.is_some() && voice.user.id == self_id.clone().unwrap_or_default() {
            // Keep our own row's mute/deafen flags in sync too.
            if let Some(me) = self.state.me.as_mut() {
                me.self_muted = voice.voice_state.self_mute;
                me.self_deafened = voice.voice_state.self_deaf;
                me.muted = voice.voice_state.mute;
                me.deafened = voice.voice_state.deaf;
            }
        }

        self.state.upsert(&voice, now_millis(), created);
    }

    fn on_voice_state_delete(&mut self, message: &RpcMessage) {
        let Some(user_id) = message
            .data
            .as_ref()
            .and_then(|data| data.get("user"))
            .and_then(|user| user.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };

        if self.state.me.as_ref().map(|me| me.id.as_str()) == Some(user_id.as_str()) {
            if let Some(channel_id) = self.subscribed_channel.take() {
                self.unsubscribe_channel(&channel_id);
            }
            self.state.clear_channel();
        }

        self.state.remove(&user_id, now_millis());
    }

    fn subscribe_channel(&mut self, channel_id: &str) {
        self.subscribed_channel = Some(channel_id.to_string());
        for event_name in event::CHANNEL_EVENTS {
            let nonce = self.next_nonce("subscribe-channel");
            let payload =
                subscription_payload(command::SUBSCRIBE, &nonce, event_name, Some(channel_id));
            if let Err(error) = self.send(payload) {
                self.log(format!("could not subscribe to {event_name}: {error}"));
            }
        }
    }

    fn unsubscribe_channel(&mut self, channel_id: &str) {
        if self.subscribed_channel.as_deref() == Some(channel_id) {
            self.subscribed_channel = None;
        }
        for event_name in event::CHANNEL_EVENTS {
            let nonce = self.next_nonce("unsubscribe-channel");
            let payload =
                subscription_payload(command::UNSUBSCRIBE, &nonce, event_name, Some(channel_id));
            if let Err(error) = self.send(payload) {
                self.log(format!("could not unsubscribe from {event_name}: {error}"));
            }
        }
    }

    fn tick(&mut self) -> Option<SessionEnd> {
        if self.authenticated {
            if let Some(deadline) = self.resync_deadline {
                if Instant::now() >= deadline {
                    self.resync_deadline = None;
                    self.resync_nonce = None;
                    self.missed_resyncs += 1;

                    if self.missed_resyncs >= 2 {
                        return Some(SessionEnd::Reconnect {
                            reason: "Discord stopped answering channel resyncs".to_string(),
                            was_ready: self.was_ready,
                        });
                    }

                    self.request_selected(true);
                }
            } else if self.last_resync.elapsed() >= self.options.resync_interval {
                self.request_selected(true);
            }
        }

        if self.last_ping.elapsed() >= self.options.ping_interval {
            self.last_ping = Instant::now();
            let nonce = self.next_nonce("ping");
            let _ = self.connection.send(OP_PING, &json!({ "nonce": nonce }));
        }

        None
    }
}

fn speaking_user_id(message: &RpcMessage) -> Option<String> {
    message
        .data
        .as_ref()
        .and_then(|data| {
            data.get("user_id").and_then(Value::as_str).or_else(|| {
                data.get("user")
                    .and_then(|user| user.get("id"))
                    .and_then(Value::as_str)
            })
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_are_capped_at_the_last_value() {
        let delays = vec![Duration::from_secs(1), Duration::from_secs(5)];
        assert_eq!(delay_for(&delays, 0), Duration::from_secs(1));
        assert_eq!(delay_for(&delays, 1), Duration::from_secs(5));
        assert_eq!(delay_for(&delays, 99), Duration::from_secs(5));
        assert_eq!(delay_for(&[], 0), Duration::from_secs(5));
    }

    #[test]
    fn status_serializes_with_a_state_tag() {
        let json = serde_json::to_value(Status::Ready {
            user_name: "Me".into(),
        })
        .unwrap();
        assert_eq!(json["state"], "ready");
        assert_eq!(json["userName"], "Me");

        let json = serde_json::to_value(Status::WaitingForDiscord {
            message: "no socket".into(),
        })
        .unwrap();
        assert_eq!(json["state"], "waitingForDiscord");
    }

    #[test]
    fn file_token_store_round_trips_privately() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("dlo-token-{}-{}", std::process::id(), now_millis()));
        let path = dir.join("token.json");
        let store = FileTokenStore::new(&path);

        assert!(store.load().is_none());

        store.save(&Token {
            access_token: "abc".into(),
            expires_at: Some(42),
        });

        let loaded = store.load().expect("token on disk");
        assert_eq!(loaded.access_token, "abc");
        assert_eq!(loaded.expires_at, Some(42));

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        store.clear();
        assert!(store.load().is_none());
        store.clear();

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn empty_tokens_are_treated_as_missing() {
        let dir = std::env::temp_dir().join(format!(
            "dlo-token-empty-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let path = dir.join("token.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"access_token":"   "}"#).unwrap();

        assert!(FileTokenStore::new(&path).load().is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
