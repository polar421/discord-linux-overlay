//! Discord RPC vocabulary and payload types.
//!
//! These mirror the commands and dispatch events that the official Discord
//! desktop client exposes over its local IPC socket. Only the subset the
//! overlay needs is modelled: voice channel membership, voice state flags and
//! speaking activity.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// RPC commands sent to Discord.
pub mod command {
    pub const AUTHORIZE: &str = "AUTHORIZE";
    pub const AUTHENTICATE: &str = "AUTHENTICATE";
    pub const GET_CHANNEL: &str = "GET_CHANNEL";
    pub const GET_SELECTED_VOICE_CHANNEL: &str = "GET_SELECTED_VOICE_CHANNEL";
    pub const SUBSCRIBE: &str = "SUBSCRIBE";
    pub const UNSUBSCRIBE: &str = "UNSUBSCRIBE";
}

/// Dispatch events pushed by Discord.
pub mod event {
    /// Sent immediately after the handshake.
    pub const READY: &str = "READY";
    /// Sent for errors, including failed command responses.
    pub const ERROR: &str = "ERROR";
    pub const SPEAKING_START: &str = "SPEAKING_START";
    pub const SPEAKING_STOP: &str = "SPEAKING_STOP";
    pub const VOICE_STATE_CREATE: &str = "VOICE_STATE_CREATE";
    pub const VOICE_STATE_UPDATE: &str = "VOICE_STATE_UPDATE";
    pub const VOICE_STATE_DELETE: &str = "VOICE_STATE_DELETE";
    /// Sent when the local client joins or leaves a voice channel.
    pub const VOICE_CHANNEL_SELECT: &str = "VOICE_CHANNEL_SELECT";

    /// Every event that has to be subscribed per voice channel.
    pub const CHANNEL_EVENTS: [&str; 5] = [
        SPEAKING_START,
        SPEAKING_STOP,
        VOICE_STATE_CREATE,
        VOICE_STATE_UPDATE,
        VOICE_STATE_DELETE,
    ];
}

/// Discord RPC error codes relevant to authentication.
pub mod error_code {
    pub const INVALID_CLIENT_ID: u32 = 4007;
    pub const INVALID_TOKEN: u32 = 4009;
    pub const INVALID_USER: u32 = 4010;
    pub const OAUTH2_ERROR: u32 = 5000;

    pub fn requires_reauthorization(code: u32) -> bool {
        matches!(code, INVALID_TOKEN | INVALID_USER | OAUTH2_ERROR)
    }

    pub fn is_configuration_problem(code: u32) -> bool {
        matches!(code, INVALID_CLIENT_ID)
    }
}

/// A raw message in either direction.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RpcMessage {
    #[serde(default)]
    pub cmd: Option<String>,
    #[serde(default)]
    pub evt: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub args: Option<Value>,
}

impl RpcMessage {
    pub fn error_code(&self) -> Option<u32> {
        self.data
            .as_ref()?
            .get("code")
            .and_then(Value::as_u64)
            .map(|code| code as u32)
    }

    pub fn error_message(&self) -> String {
        self.data
            .as_ref()
            .and_then(|data| data.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Discord returned an unspecified error")
            .to_string()
    }
}

/// A Discord user, trimmed down to what the overlay shows.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct User {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

impl User {
    pub fn display_name(&self) -> &str {
        match self.global_name.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => self.username.as_str(),
        }
    }
}

/// Mute/deafen flags as reported by Discord.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct VoiceStateFlags {
    /// Server mute ("muted by a moderator").
    #[serde(default)]
    pub mute: bool,
    /// Server deafen.
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub self_mute: bool,
    #[serde(default)]
    pub self_deaf: bool,
    #[serde(default)]
    pub suppress: bool,
}

/// One entry of `voice_states` inside a voice channel.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct VoiceStateUser {
    #[serde(default)]
    pub nick: String,
    #[serde(default)]
    pub user: User,
    #[serde(default)]
    pub voice_state: VoiceStateFlags,
    #[serde(default)]
    pub volume: Option<f64>,
}

impl VoiceStateUser {
    /// Server-side nickname, then global display name, then username.
    pub fn display_name(&self) -> String {
        let nick = self.nick.trim();
        if !nick.is_empty() {
            return nick.to_string();
        }
        let global = self.user.display_name().trim();
        if !global.is_empty() {
            return global.to_string();
        }
        "Unknown".to_string()
    }
}

/// The voice channel the local client is connected to.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Channel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
}

/// Build a command payload. A nonce is required by Discord for every command.
pub fn command_payload(cmd: &str, nonce: &str, args: Option<Value>) -> Value {
    let mut payload = serde_json::json!({ "cmd": cmd, "nonce": nonce });
    if let Some(args) = args {
        payload["args"] = args;
    }
    payload
}

/// Build an event subscription payload.
pub fn subscription_payload(
    cmd: &str,
    nonce: &str,
    event: &str,
    channel_id: Option<&str>,
) -> Value {
    let mut payload = serde_json::json!({ "cmd": cmd, "evt": event, "nonce": nonce });
    if let Some(channel_id) = channel_id {
        payload["args"] = serde_json::json!({ "channel_id": channel_id });
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_VOICE_STATE: &str = r#"{
        "nick": "Speedy",
        "mute": false,
        "volume": 100.0,
        "pan": { "left": 1.0, "right": 1.0 },
        "voice_state": {
            "mute": false,
            "deaf": false,
            "self_mute": true,
            "self_deaf": true,
            "suppress": false
        },
        "user": {
            "id": "123456789012345678",
            "username": "speedy",
            "global_name": "Speedy",
            "avatar": "a_abcdef",
            "bot": false,
            "discriminator": "0",
            "flags": 0,
            "premium_type": 0
        }
    }"#;

    #[test]
    fn voice_state_payloads_are_parsed_with_flags() {
        let state: VoiceStateUser = serde_json::from_str(REAL_VOICE_STATE).unwrap();
        assert_eq!(state.display_name(), "Speedy");
        assert_eq!(state.user.id, "123456789012345678");
        assert_eq!(state.user.avatar.as_deref(), Some("a_abcdef"));
        assert!(state.voice_state.self_mute);
        assert!(state.voice_state.self_deaf);
        assert!(!state.voice_state.mute);
        assert!(!state.voice_state.suppress);
        assert!(!state.user.bot);
    }

    #[test]
    fn display_name_falls_back_to_username_without_global_name() {
        let mut state = VoiceStateUser::default();
        state.user.username = "speedy".into();
        assert_eq!(state.display_name(), "speedy");

        state.nick = "Local Nick".into();
        assert_eq!(state.display_name(), "Local Nick");

        state.nick = "   ".into();
        assert_eq!(state.display_name(), "speedy");
    }

    #[test]
    fn channel_payload_parses_type_field() {
        let channel: Channel = serde_json::from_str(
            r#"{ "id": "1", "name": "General", "type": 2, "guild_id": "42", "voice_states": [] }"#,
        )
        .unwrap();

        assert_eq!(channel.id, "1");
        assert_eq!(channel.name, "General");
        assert_eq!(channel.kind, Some(2));
        assert_eq!(channel.guild_id.as_deref(), Some("42"));
    }

    #[test]
    fn rpc_messages_tolerate_missing_fields() {
        let message: RpcMessage = serde_json::from_str(
            r#"{ "cmd": "AUTHENTICATE", "evt": null, "data": { "user": { "id": "1" } } }"#,
        )
        .unwrap();
        assert_eq!(message.cmd.as_deref(), Some("AUTHENTICATE"));
        assert!(message.evt.is_none());
        assert_eq!(message.data.unwrap()["user"]["id"], "1");
    }

    #[test]
    fn error_codes_map_to_the_right_recovery_action() {
        assert!(error_code::requires_reauthorization(
            error_code::INVALID_TOKEN
        ));
        assert!(error_code::requires_reauthorization(
            error_code::OAUTH2_ERROR
        ));
        assert!(error_code::is_configuration_problem(
            error_code::INVALID_CLIENT_ID
        ));
        assert!(!error_code::requires_reauthorization(
            error_code::INVALID_CLIENT_ID
        ));
    }

    #[test]
    fn payload_builders_include_nonce_and_optional_args() {
        let payload = command_payload(command::GET_SELECTED_VOICE_CHANNEL, "n1", None);
        assert_eq!(payload["cmd"], "GET_SELECTED_VOICE_CHANNEL");
        assert_eq!(payload["nonce"], "n1");
        assert!(payload.get("args").is_none());

        let sub = subscription_payload(command::SUBSCRIBE, "n2", event::SPEAKING_START, Some("99"));
        assert_eq!(sub["evt"], "SPEAKING_START");
        assert_eq!(sub["args"]["channel_id"], "99");
    }
}
