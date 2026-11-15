//! End-to-end test of the Discord client against a fake Discord IPC server.
//!
//! This exercises the real socket framing, the handshake, the authorization and
//! authentication dance, event subscriptions, speaking flags, participant
//! updates and reconnection - i.e. everything the overlay needs from Discord,
//! without requiring a Discord account in CI.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use discord_overlay_core::client::{
    ClientEvent, ClientOptions, DiscordClient, Status, Token, TokenExchange, TokenStore,
};
use discord_overlay_core::ipc::{encode, FrameDecoder, OP_HANDSHAKE, OP_PING, OP_PONG};
use discord_overlay_core::state::OverlayState;
use discord_overlay_core::Result;
use serde_json::{json, Value};

struct Exchange {
    calls: Arc<AtomicUsize>,
}

impl TokenExchange for Exchange {
    fn exchange(&self, code: &str) -> Result<Token> {
        assert_eq!(code, "fake-code");
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Token {
            access_token: "fake-access-token".to_string(),
            expires_at: None,
        })
    }
}

#[derive(Default)]
struct Store {
    token: Mutex<Option<Token>>,
}

impl TokenStore for Store {
    fn load(&self) -> Option<Token> {
        self.token.lock().unwrap().clone()
    }

    fn save(&self, token: &Token) {
        *self.token.lock().unwrap() = Some(token.clone());
    }

    fn clear(&self) {
        *self.token.lock().unwrap() = None;
    }
}

fn temp_socket(label: &str) -> PathBuf {
    let unique = format!(
        "dlo-e2e-{}-{label}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    );
    std::env::temp_dir().join(unique)
}

fn read_frame(stream: &mut UnixStream, decoder: &mut FrameDecoder) -> Option<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buffer = [0u8; 8192];

    while Instant::now() < deadline {
        if let Ok(Some(frame)) = decoder.next_frame() {
            return Some(frame.payload);
        }
        match std::io::Read::read(stream, &mut buffer) {
            Ok(0) => return None,
            Ok(read) => decoder.extend(&buffer[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return None,
        }
    }

    None
}

fn write_frame(stream: &mut UnixStream, payload: Value) {
    use std::io::Write;
    let bytes = encode(1, &payload);
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
}

/// Wait until a state matching `predicate` arrives.
fn wait_for_state<F>(events: &mpsc::Receiver<ClientEvent>, predicate: F) -> OverlayState
where
    F: Fn(&OverlayState) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last: Option<OverlayState> = None;

    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(ClientEvent::State(state)) => {
                if predicate(&state) {
                    return *state;
                }
                last = Some(*state);
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    panic!("timed out waiting for the expected overlay state; last seen: {last:?}");
}

fn wait_for_status<F>(events: &mpsc::Receiver<ClientEvent>, predicate: F, timeout: Duration) -> bool
where
    F: Fn(&Status) -> bool,
{
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(ClientEvent::Status(status)) if predicate(&status) => return true,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }

    false
}

fn snapshot(channel_id: &str, users: Value) -> Value {
    json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "data": {
            "id": channel_id,
            "name": "General",
            "type": 2,
            "guild_id": "42",
            "voice_states": users
        },
        "nonce": "reply"
    })
}

fn user(id: &str, name: &str, flags: Value) -> Value {
    json!({
        "nick": name,
        "user": { "id": id, "username": name.to_lowercase(), "global_name": name, "avatar": null },
        "voice_state": flags
    })
}

fn spawn_server(listener: UnixListener, behaviour: impl FnOnce(&mut UnixStream) + Send + 'static) {
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .ok();
            behaviour(&mut stream);
        }
    });
}

fn answer_until_selected(stream: &mut UnixStream, decoder: &mut FrameDecoder) -> Vec<Value> {
    let mut seen = Vec::new();

    for _ in 0..50 {
        let Some(payload) = read_frame(stream, decoder) else {
            break;
        };

        if payload.get("op").is_none() {
            match payload.get("cmd").and_then(Value::as_str) {
                Some("AUTHENTICATE") => {
                    write_frame(
                        stream,
                        json!({
                            "cmd": "AUTHENTICATE",
                            "data": {
                                "user": { "id": "1", "username": "me", "global_name": "Me", "avatar": null },
                                "expires": "2099-01-01T00:00:00.000000+00:00"
                            },
                            "nonce": payload.get("nonce").cloned().unwrap_or(Value::Null)
                        }),
                    );
                }
                Some("GET_SELECTED_VOICE_CHANNEL") => {
                    seen.push(payload.clone());
                    return seen;
                }
                _ => {}
            }
        }
    }

    seen
}

#[test]
fn full_discord_session_flow() {
    let socket = temp_socket("session");
    let listener = UnixListener::bind(&socket).expect("bind fake discord socket");
    let exchange_calls = Arc::new(AtomicUsize::new(0));
    let exchange = Exchange {
        calls: exchange_calls.clone(),
    };
    let store = Arc::new(Store::default());

    let (server_done_tx, server_done_rx) = mpsc::channel();

    spawn_server(listener, move |stream| {
        let mut decoder = FrameDecoder::default();

        // 1. Handshake, then READY.
        let handshake = read_frame(stream, &mut decoder).expect("handshake frame");
        assert_eq!(handshake["v"], 1);
        assert_eq!(handshake["client_id"], "1234");
        write_frame(stream, json!({ "evt": "READY", "data": { "v": 1 } }));

        // 2. AUTHORIZE -> code.
        let authorize = read_frame(stream, &mut decoder).expect("authorize frame");
        assert_eq!(authorize["cmd"], "AUTHORIZE");
        assert_eq!(authorize["args"]["client_id"], "1234");
        assert_eq!(authorize["args"]["scopes"][1], "rpc");
        write_frame(
            stream,
            json!({ "cmd": "AUTHORIZE", "data": { "code": "fake-code" } }),
        );

        // 3. AUTHENTICATE, subscribe and the channel snapshot.
        let selected = answer_until_selected(stream, &mut decoder);
        assert!(
            !selected.is_empty(),
            "client must ask for the selected channel"
        );

        write_frame(
            stream,
            snapshot(
                "10",
                json!([
                    user("1", "Me", json!({})),
                    user("2", "Bravo", json!({ "self_mute": true })),
                    user("3", "Charlie", json!({ "self_deaf": true, "deaf": true }))
                ]),
            ),
        );

        // 4. Live events.
        write_frame(
            stream,
            json!({ "evt": "SPEAKING_START", "data": { "user_id": "2", "channel_id": "10" } }),
        );
        thread::sleep(Duration::from_millis(150));
        write_frame(
            stream,
            json!({ "evt": "SPEAKING_STOP", "data": { "user_id": "2", "channel_id": "10" } }),
        );

        // 5. Somebody joins, then leaves.
        write_frame(
            stream,
            json!({ "evt": "VOICE_STATE_CREATE", "data": user("4", "Delta", json!({})) }),
        );
        thread::sleep(Duration::from_millis(150));
        write_frame(
            stream,
            json!({ "evt": "VOICE_STATE_DELETE", "data": { "user": { "id": "4" } } }),
        );

        // 6. Keep the socket open so the client stays connected until stopped.
        thread::sleep(Duration::from_millis(800));
        let _ = server_done_tx.send(());
    });

    let (events_tx, events_rx) = mpsc::channel();
    let mut client = DiscordClient::start(
        ClientOptions {
            client_id: "1234".to_string(),
            socket_paths: Some(vec![socket.clone()]),
            resync_interval: Duration::from_secs(60),
            ..ClientOptions::new("1234")
        },
        Arc::new(exchange),
        store.clone(),
        events_tx,
    );

    // The channel snapshot arrives with all three participants and correct flags.
    let state = wait_for_state(&events_rx, |state| {
        state.channel.is_some() && state.participants.len() == 3
    });

    assert_eq!(state.channel.as_ref().unwrap().name, "General");
    let names: Vec<&str> = state
        .participants
        .iter()
        .map(|participant| participant.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["Me", "Bravo", "Charlie"]);

    let me = state.participants.iter().find(|p| p.id == "1").unwrap();
    assert!(me.is_self);
    assert_eq!(
        me.avatar_url,
        "https://cdn.discordapp.com/embed/avatars/0.png"
    );

    let bravo = state.participants.iter().find(|p| p.id == "2").unwrap();
    assert!(bravo.self_muted);
    assert!(bravo.is_mic_off());

    let charlie = state.participants.iter().find(|p| p.id == "3").unwrap();
    assert!(charlie.self_deafened);
    assert!(charlie.deafened);
    assert!(charlie.is_deafened());

    // Speaking indicator turns on and off.
    let speaking = wait_for_state(&events_rx, |state| {
        state
            .participants
            .iter()
            .any(|participant| participant.id == "2" && participant.speaking)
    });
    assert!(speaking
        .participants
        .iter()
        .any(|p| p.id == "2" && p.speaking));

    let quiet = wait_for_state(&events_rx, |state| {
        state
            .participants
            .iter()
            .any(|participant| participant.id == "2" && !participant.speaking)
    });
    assert!(quiet
        .participants
        .iter()
        .any(|p| p.id == "2" && !p.speaking));

    // A join is visible and recorded in the join history.
    let joined = wait_for_state(&events_rx, |state| {
        state
            .participants
            .iter()
            .any(|participant| participant.id == "4")
    });
    assert!(joined
        .history
        .iter()
        .any(|entry| entry.id == "4" && entry.action == "join"));

    // ... and the leave removes the row again.
    let left = wait_for_state(&events_rx, |state| {
        state.participants.len() == 3
            && !state
                .participants
                .iter()
                .any(|participant| participant.id == "4")
    });
    assert!(left
        .history
        .iter()
        .any(|entry| entry.id == "4" && entry.action == "leave"));

    client.stop();

    // The OAuth2 code was exchanged exactly once and the token was cached.
    assert_eq!(exchange_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.load().map(|token| token.access_token),
        Some("fake-access-token".to_string())
    );

    server_done_rx.recv_timeout(Duration::from_secs(5)).ok();
    std::fs::remove_file(Path::new(&socket)).ok();
}

#[test]
fn cached_token_skips_authorization() {
    let socket = temp_socket("cached");
    let listener = UnixListener::bind(&socket).expect("bind fake discord socket");
    let exchange_calls = Arc::new(AtomicUsize::new(0));
    let exchange = Exchange {
        calls: exchange_calls.clone(),
    };
    let store = Arc::new(Store::default());
    store.save(&Token {
        access_token: "cached-token".to_string(),
        expires_at: None,
    });

    let (done_tx, done_rx) = mpsc::channel();

    spawn_server(listener, move |stream| {
        let mut decoder = FrameDecoder::default();
        let handshake = read_frame(stream, &mut decoder).expect("handshake");
        assert_eq!(handshake["client_id"], "1234");
        write_frame(stream, json!({ "evt": "READY" }));

        // With a cached token the first command must be AUTHENTICATE.
        let first = read_frame(stream, &mut decoder).expect("authenticate");
        assert_eq!(first["cmd"], "AUTHENTICATE");
        assert_eq!(first["args"]["access_token"], "cached-token");

        write_frame(
            stream,
            json!({
                "cmd": "AUTHENTICATE",
                "data": { "user": { "id": "1", "username": "me", "global_name": "Me" } }
            }),
        );

        let _ = answer_until_selected(stream, &mut decoder);
        write_frame(stream, snapshot("10", json!([])));
        thread::sleep(Duration::from_millis(500));
        let _ = done_tx.send(());
    });

    let (events_tx, events_rx) = mpsc::channel();
    let mut client = DiscordClient::start(
        ClientOptions {
            socket_paths: Some(vec![socket.clone()]),
            ..ClientOptions::new("1234")
        },
        Arc::new(exchange),
        store.clone(),
        events_tx,
    );

    let state = wait_for_state(&events_rx, |state| state.channel.is_some());
    assert_eq!(state.channel.as_ref().unwrap().id, "10");
    assert_eq!(exchange_calls.load(Ordering::SeqCst), 0);

    client.stop();
    done_rx.recv_timeout(Duration::from_secs(5)).ok();
    std::fs::remove_file(Path::new(&socket)).ok();
}

#[test]
fn reconnects_after_the_socket_drops() {
    let socket = temp_socket("reconnect");
    let listener = UnixListener::bind(&socket).expect("bind fake discord socket");
    let store = Arc::new(Store::default());
    let exchange = Exchange {
        calls: Arc::new(AtomicUsize::new(0)),
    };

    // First connection: handshake + READY, then vanish.
    let first_listener = listener;
    let (ready_tx, ready_rx) = mpsc::channel();

    thread::spawn(move || {
        for attempt in 0..3 {
            let Ok((mut stream, _)) = first_listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .ok();
            let mut decoder = FrameDecoder::default();
            let _ = read_frame(&mut stream, &mut decoder);
            write_frame(&mut stream, json!({ "evt": "READY" }));
            let _ = read_frame(&mut stream, &mut decoder);

            write_frame(
                &mut stream,
                json!({
                    "cmd": "AUTHENTICATE",
                    "data": { "user": { "id": "1", "username": "me", "global_name": "Me" } }
                }),
            );
            write_frame(
                &mut stream,
                snapshot("10", json!([user("1", "Me", json!({}))])),
            );
            let _ = ready_tx.send(attempt);
            thread::sleep(Duration::from_millis(250));
            // Dropping the stream simulates Discord restarting.
        }
    });

    let (events_tx, events_rx) = mpsc::channel();
    let mut client = DiscordClient::start(
        ClientOptions {
            socket_paths: Some(vec![socket.clone()]),
            reconnect_delays: vec![Duration::from_millis(50)],
            ..ClientOptions::new("1234")
        },
        Arc::new(exchange),
        store,
        events_tx,
    );

    assert!(
        wait_for_status(
            &events_rx,
            |status| status.is_ready(),
            Duration::from_secs(10)
        ),
        "client should reach the ready state"
    );
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first connection");

    // After the drop the client must connect again and become ready once more.
    let mut ready_count = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && ready_count < 2 {
        if let Ok(ClientEvent::Status(Status::Ready { .. })) =
            events_rx.recv_timeout(Duration::from_millis(200))
        {
            ready_count += 1;
        }
    }
    assert_eq!(
        ready_count, 2,
        "client should recover from a dropped socket"
    );

    client.stop();
    std::fs::remove_file(Path::new(&socket)).ok();
}

#[test]
fn missing_socket_reports_waiting_for_discord() {
    let socket = temp_socket("missing");
    let (events_tx, events_rx) = mpsc::channel();

    let mut client = DiscordClient::start(
        ClientOptions {
            socket_paths: Some(vec![socket]),
            reconnect_delays: vec![Duration::from_millis(50)],
            ..ClientOptions::new("1234")
        },
        Arc::new(Exchange {
            calls: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(Store::default()),
        events_tx,
    );

    assert!(wait_for_status(
        &events_rx,
        |status| matches!(status, Status::WaitingForDiscord { .. }),
        Duration::from_secs(5)
    ));

    client.stop();
}

#[test]
fn ping_frames_are_answered_with_pong() {
    let socket = temp_socket("ping");
    let listener = UnixListener::bind(&socket).expect("bind fake discord socket");
    let (pong_tx, pong_rx) = mpsc::channel();

    spawn_server(listener, move |stream| {
        let mut decoder = FrameDecoder::default();
        let _ = read_frame(stream, &mut decoder);
        write_frame(stream, json!({ "evt": "READY" }));

        // Raw PING frame (opcode 3).
        use std::io::Write;
        let bytes = encode(OP_PING, &json!({ "nonce": "abc" }));
        stream.write_all(&bytes).ok();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(frame)) = decoder.next_frame() {
                if frame.op == OP_PONG {
                    let _ = pong_tx.send(frame.payload);
                    return;
                }
                continue;
            }
            let mut buffer = [0u8; 1024];
            match std::io::Read::read(stream, &mut buffer) {
                Ok(0) => return,
                Ok(read) => decoder.extend(&buffer[..read]),
                Err(_) => {}
            }
        }
        let _ = pong_tx.send(Value::Null);
    });

    let (events_tx, _events_rx) = mpsc::channel();
    let mut client = DiscordClient::start(
        ClientOptions {
            socket_paths: Some(vec![socket.clone()]),
            ..ClientOptions::new("1234")
        },
        Arc::new(Exchange {
            calls: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(Store::default()),
        events_tx,
    );

    let payload = pong_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("server should receive a pong");
    assert_eq!(payload["nonce"], "abc");
    assert_eq!(OP_HANDSHAKE, 0);

    client.stop();
    std::fs::remove_file(Path::new(&socket)).ok();
}
