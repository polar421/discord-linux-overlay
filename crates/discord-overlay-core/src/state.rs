//! The overlay's view of a Discord voice channel.
//!
//! Everything here is a pure data transformation so it can be tested without a
//! Discord client, a display server or a web view. The client feeds snapshots
//! and events in; the UI renders whatever [`OverlayState`] contains.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::avatar::avatar_url;
use crate::protocol::{Channel, VoiceStateUser};

/// Number of join/leave entries kept in memory.
pub const HISTORY_LIMIT: usize = 50;

/// A participant row as rendered by the overlay.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub id: String,
    pub display_name: String,
    pub username: String,
    pub avatar_url: String,
    /// Live speaking flag driven by `SPEAKING_START` / `SPEAKING_STOP`.
    pub speaking: bool,
    /// Muted by a moderator.
    pub muted: bool,
    /// Deafened by a moderator.
    pub deafened: bool,
    pub self_muted: bool,
    pub self_deafened: bool,
    /// Discord suppressed the user (e.g. audience in a stage channel).
    pub suppressed: bool,
    pub bot: bool,
    pub is_self: bool,
}

impl Participant {
    pub fn from_voice_state(
        voice: &VoiceStateUser,
        self_id: Option<&str>,
        avatar_size: u32,
    ) -> Self {
        let name = voice.display_name();
        Self {
            id: voice.user.id.clone(),
            display_name: name,
            username: voice.user.username.clone(),
            avatar_url: avatar_url(&voice.user.id, voice.user.avatar.as_deref(), avatar_size),
            speaking: false,
            muted: voice.voice_state.mute,
            deafened: voice.voice_state.deaf,
            self_muted: voice.voice_state.self_mute,
            self_deafened: voice.voice_state.self_deaf,
            suppressed: voice.voice_state.suppress,
            bot: voice.user.bot,
            is_self: self_id.is_some_and(|id| id == voice.user.id),
        }
    }

    /// True when the microphone is off for any reason.
    pub fn is_mic_off(&self) -> bool {
        self.self_muted || self.muted || self.suppressed
    }

    /// True when the user cannot hear the channel.
    pub fn is_deafened(&self) -> bool {
        self.self_deafened || self.deafened
    }
}

/// The current voice channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub guild_id: Option<String>,
    pub kind: Option<u8>,
}

impl ChannelInfo {
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            id: channel.id,
            name: if channel.name.trim().is_empty() {
                "Voice channel".to_string()
            } else {
                channel.name
            },
            guild_id: channel.guild_id,
            kind: channel.kind,
        }
    }
}

/// One join/leave notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub display_name: String,
    pub avatar_url: String,
    /// `"join"` or `"leave"`.
    pub action: String,
    pub timestamp: u64,
}

/// Everything the overlay renders.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OverlayState {
    pub me: Option<Participant>,
    pub channel: Option<ChannelInfo>,
    pub participants: Vec<Participant>,
    pub history: Vec<HistoryEntry>,
    /// Incremented on every meaningful change; the UI uses it to skip work.
    pub revision: u64,
}

impl OverlayState {
    pub fn set_self(&mut self, participant: Participant) {
        if self.me.as_ref() != Some(&participant) {
            self.me = Some(participant);
            self.bump();
        }
    }

    pub fn clear_self(&mut self) {
        if self.me.take().is_some() {
            self.bump();
        }
    }

    /// Apply a full channel snapshot (`GET_SELECTED_VOICE_CHANNEL`).
    ///
    /// Speaking flags are preserved for users that were already in the channel:
    /// a resync must never make a talking user flicker to silent.
    pub fn set_channel(&mut self, channel: Option<ChannelInfo>, users: Vec<VoiceStateUser>) {
        let self_id = self.me.as_ref().map(|me| me.id.clone());

        match channel {
            Some(channel) => {
                let speaking: HashMap<&str, bool> = self
                    .participants
                    .iter()
                    .map(|participant| (participant.id.as_str(), participant.speaking))
                    .collect();

                let mut participants: Vec<Participant> = Vec::with_capacity(users.len());
                for voice in &users {
                    if voice.user.id.is_empty() {
                        continue;
                    }
                    let mut participant =
                        Participant::from_voice_state(voice, self_id.as_deref(), 96);
                    if let Some(previous) = speaking.get(participant.id.as_str()) {
                        participant.speaking = *previous;
                    }
                    if self_id.as_deref() == Some(participant.id.as_str()) {
                        participant.is_self = true;
                    }
                    participants.push(participant);
                }

                dedupe_and_sort(&mut participants);
                self.channel = Some(channel);
                self.participants = participants;
                self.bump();
            }
            None => self.clear_channel(),
        }
    }

    /// Add or refresh a single participant (`VOICE_STATE_CREATE` / `_UPDATE`).
    pub fn upsert(&mut self, voice: &VoiceStateUser, timestamp: u64, record_join: bool) {
        if voice.user.id.is_empty() {
            return;
        }

        let self_id = self.me.as_ref().map(|me| me.id.clone());
        let mut participant = Participant::from_voice_state(voice, self_id.as_deref(), 96);

        if let Some(existing) = self
            .participants
            .iter_mut()
            .find(|existing| existing.id == participant.id)
        {
            // Keep the live speaking flag: Discord does not always re-send it.
            participant.speaking = existing.speaking;
            *existing = participant;
        } else {
            if record_join && !participant.is_self {
                self.push_history(&participant, "join", timestamp);
            }
            self.participants.push(participant);
        }

        sort_participants(&mut self.participants);
        self.bump();
    }

    /// Remove a participant (`VOICE_STATE_DELETE`).
    pub fn remove(&mut self, user_id: &str, timestamp: u64) {
        let Some(index) = self
            .participants
            .iter()
            .position(|participant| participant.id == user_id)
        else {
            return;
        };

        let participant = self.participants.remove(index);
        if !participant.is_self {
            self.push_history(&participant, "leave", timestamp);
        }
        self.bump();
    }

    /// Toggle a participant's speaking flag (`SPEAKING_START` / `_STOP`).
    pub fn set_speaking(&mut self, user_id: &str, speaking: bool) {
        if let Some(participant) = self
            .participants
            .iter_mut()
            .find(|participant| participant.id == user_id)
        {
            if participant.speaking != speaking {
                participant.speaking = speaking;
                self.bump();
            }
        }
    }

    /// Drop all channel data (left voice, disconnected).
    pub fn clear_channel(&mut self) {
        if self.channel.is_some() || !self.participants.is_empty() {
            self.channel = None;
            self.participants.clear();
            self.bump();
        }
    }

    /// Full reset used when the IPC connection is lost.
    pub fn on_disconnect(&mut self) {
        self.me = None;
        self.channel = None;
        self.participants.clear();
        self.bump();
    }

    pub fn clear_history(&mut self) {
        if !self.history.is_empty() {
            self.history.clear();
            self.bump();
        }
    }

    pub fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn push_history(&mut self, participant: &Participant, action: &str, timestamp: u64) {
        self.history.push(HistoryEntry {
            id: participant.id.clone(),
            display_name: participant.display_name.clone(),
            avatar_url: participant.avatar_url.clone(),
            action: action.to_string(),
            timestamp,
        });

        if self.history.len() > HISTORY_LIMIT {
            let excess = self.history.len() - HISTORY_LIMIT;
            self.history.drain(..excess);
        }
    }
}

fn dedupe_and_sort(participants: &mut Vec<Participant>) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut unique: Vec<Participant> = Vec::with_capacity(participants.len());

    for participant in participants.drain(..) {
        if let Some(index) = seen.get(&participant.id).copied() {
            unique[index] = participant;
        } else {
            seen.insert(participant.id.clone(), unique.len());
            unique.push(participant);
        }
    }

    // A stable order matters far more than perfect alphabetical accuracy: rows
    // must not jump around while people talk.
    sort_participants(&mut unique);
    *participants = unique;
}

fn sort_participants(participants: &mut [Participant]) {
    participants.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn voice(id: &str, name: &str, flags: serde_json::Value) -> VoiceStateUser {
        serde_json::from_value(json!({
            "nick": name,
            "user": { "id": id, "username": name.to_lowercase(), "global_name": name },
            "voice_state": flags
        }))
        .unwrap()
    }

    fn channel(id: &str, name: &str) -> ChannelInfo {
        ChannelInfo {
            id: id.into(),
            name: name.into(),
            guild_id: Some("42".into()),
            kind: Some(2),
        }
    }

    #[test]
    fn snapshot_sorts_self_first_then_case_insensitive_names() {
        let mut state = OverlayState::default();
        state.set_self(Participant {
            id: "2".into(),
            display_name: "zed".into(),
            ..Default::default()
        });

        state.set_channel(
            Some(channel("10", "General")),
            vec![
                voice("3", "charlie", json!({})),
                voice("2", "Zed", json!({})),
                voice("1", "Bravo", json!({})),
            ],
        );

        let order: Vec<&str> = state
            .participants
            .iter()
            .map(|p| p.display_name.as_str())
            .collect();
        assert_eq!(order, vec!["Zed", "Bravo", "charlie"]);
        assert_eq!(state.channel.as_ref().unwrap().name, "General");
    }

    #[test]
    fn flags_are_mapped_from_the_voice_state() {
        let mut state = OverlayState::default();
        state.set_channel(
            Some(channel("10", "General")),
            vec![voice(
                "1",
                "Bravo",
                json!({ "self_mute": true, "self_deaf": false, "mute": true, "suppress": false }),
            )],
        );

        let participant = &state.participants[0];
        assert!(participant.self_muted);
        assert!(participant.muted);
        assert!(participant.is_mic_off());
        assert!(!participant.is_deafened());
    }

    #[test]
    fn speaking_flags_are_event_driven_and_do_not_reorder() {
        let mut state = OverlayState::default();
        state.set_channel(
            Some(channel("10", "General")),
            vec![
                voice("1", "Bravo", json!({})),
                voice("2", "Alpha", json!({})),
            ],
        );
        let before: Vec<String> = state.participants.iter().map(|p| p.id.clone()).collect();

        let revision = state.revision;
        state.set_speaking("1", true);
        assert!(
            state
                .participants
                .iter()
                .find(|p| p.id == "1")
                .unwrap()
                .speaking
        );
        assert!(state.revision > revision);

        // Repeating the same state must not bump the revision, otherwise the UI
        // would re-render on every duplicate event.
        let revision = state.revision;
        state.set_speaking("1", true);
        assert_eq!(state.revision, revision);

        let after: Vec<String> = state.participants.iter().map(|p| p.id.clone()).collect();
        assert_eq!(before, after);

        state.set_speaking("1", false);
        assert!(
            !state
                .participants
                .iter()
                .find(|p| p.id == "1")
                .unwrap()
                .speaking
        );
    }

    #[test]
    fn resync_preserves_live_speaking_flags() {
        let mut state = OverlayState::default();
        state.set_channel(
            Some(channel("10", "General")),
            vec![voice("1", "Bravo", json!({}))],
        );
        state.set_speaking("1", true);

        state.set_channel(
            Some(channel("10", "General")),
            vec![
                voice("1", "Bravo", json!({})),
                voice("2", "Alpha", json!({})),
            ],
        );

        let bravo = state.participants.iter().find(|p| p.id == "1").unwrap();
        assert!(bravo.speaking);
        let alpha = state.participants.iter().find(|p| p.id == "2").unwrap();
        assert!(!alpha.speaking);
    }

    #[test]
    fn users_missing_from_a_resync_are_dropped() {
        let mut state = OverlayState::default();
        state.set_channel(
            Some(channel("10", "General")),
            vec![
                voice("1", "Bravo", json!({})),
                voice("2", "Alpha", json!({})),
            ],
        );
        state.set_channel(
            Some(channel("10", "General")),
            vec![voice("1", "Bravo", json!({}))],
        );

        assert_eq!(state.participants.len(), 1);
        assert_eq!(state.participants[0].id, "1");
    }

    #[test]
    fn join_and_leave_events_update_membership_and_history() {
        let mut state = OverlayState::default();
        state.set_channel(Some(channel("10", "General")), vec![]);

        state.upsert(&voice("7", "Delta", json!({})), 1_700_000_000_000, true);
        assert_eq!(state.participants.len(), 1);
        assert_eq!(state.history.last().unwrap().action, "join");

        state.upsert(
            &voice("7", "Delta", json!({ "self_mute": true })),
            1_700_000_000_100,
            false,
        );
        assert_eq!(state.participants.len(), 1);
        assert!(state.participants[0].self_muted);
        assert_eq!(
            state.history.len(),
            1,
            "updates must not create history entries"
        );

        state.remove("7", 1_700_000_000_200);
        assert!(state.participants.is_empty());
        assert_eq!(state.history.last().unwrap().action, "leave");
    }

    #[test]
    fn own_join_and_leave_are_not_logged() {
        let mut state = OverlayState::default();
        state.set_self(Participant {
            id: "1".into(),
            display_name: "Me".into(),
            ..Default::default()
        });
        state.upsert(&voice("1", "Me", json!({})), 1, true);
        state.remove("1", 2);
        assert!(state.history.is_empty());
    }

    #[test]
    fn history_is_bounded() {
        let mut state = OverlayState::default();
        for index in 0..(HISTORY_LIMIT + 10) {
            state.upsert(
                &voice(&format!("{index}"), "User", json!({})),
                index as u64,
                true,
            );
        }
        assert_eq!(state.history.len(), HISTORY_LIMIT);
        assert_eq!(
            state.history.last().unwrap().id,
            format!("{}", HISTORY_LIMIT + 9)
        );
    }

    #[test]
    fn switching_channels_clears_participants_and_speaking() {
        let mut state = OverlayState::default();
        state.set_channel(
            Some(channel("10", "General")),
            vec![voice("1", "Bravo", json!({}))],
        );
        state.set_speaking("1", true);

        state.set_channel(
            Some(channel("11", "Gaming")),
            vec![voice("2", "Alpha", json!({}))],
        );

        assert_eq!(state.channel.as_ref().unwrap().id, "11");
        assert_eq!(state.participants.len(), 1);
        assert!(state.participants.iter().all(|p| !p.speaking));
    }

    #[test]
    fn leaving_voice_clears_channel_but_keeps_identity() {
        let mut state = OverlayState::default();
        state.set_self(Participant {
            id: "1".into(),
            display_name: "Me".into(),
            ..Default::default()
        });
        state.set_channel(
            Some(channel("10", "General")),
            vec![voice("1", "Me", json!({}))],
        );

        state.set_channel(None, vec![]);
        assert!(state.channel.is_none());
        assert!(state.participants.is_empty());
        assert!(state.me.is_some());
    }

    #[test]
    fn duplicate_snapshot_entries_are_deduplicated() {
        let mut state = OverlayState::default();
        state.set_channel(
            Some(channel("10", "General")),
            vec![
                voice("1", "Bravo", json!({})),
                voice("1", "Bravo Renamed", json!({})),
            ],
        );
        assert_eq!(state.participants.len(), 1);
        assert_eq!(state.participants[0].display_name, "Bravo Renamed");
    }

    #[test]
    fn disconnect_resets_everything() {
        let mut state = OverlayState::default();
        state.set_self(Participant {
            id: "1".into(),
            display_name: "Me".into(),
            ..Default::default()
        });
        state.set_channel(
            Some(channel("10", "General")),
            vec![voice("1", "Me", json!({}))],
        );

        state.on_disconnect();
        assert!(state.me.is_none());
        assert!(state.channel.is_none());
        assert!(state.participants.is_empty());
    }
}
