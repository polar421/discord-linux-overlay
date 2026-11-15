//! Discord local IPC transport.
//!
//! Discord exposes a local socket (behind the scenes a Unix domain socket on
//! Linux) that speaks a tiny binary framing protocol:
//!
//! ```text
//! [ opcode: u32 little endian ][ length: u32 little endian ][ json payload ]
//! ```
//!
//! Opcode `0` opens the connection with a handshake, `1` carries JSON
//! commands/events, `2` is a close frame and `3`/`4` are ping/pong.
//!
//! This module intentionally contains no Discord semantics: it only moves
//! frames. Everything above it lives in [`crate::protocol`] and
//! [`crate::client`].

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{Error, Result};

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

/// Discord frames are tiny. Anything larger than this is a corrupt stream.
const MAX_FRAME_LEN: usize = 8 * 1024 * 1024;

/// How long a blocking read waits before returning control to the client loop.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub op: u32,
    pub payload: Value,
}

/// Encode a single frame.
pub fn encode(op: u32, payload: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"null".to_vec());
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Reassembles frames from arbitrarily fragmented socket reads.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn extend(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Returns the next complete frame, or `None` when more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        if self.buffer.len() < 8 {
            return Ok(None);
        }

        let op = u32::from_le_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]);
        let len = u32::from_le_bytes([
            self.buffer[4],
            self.buffer[5],
            self.buffer[6],
            self.buffer[7],
        ]) as usize;

        if len > MAX_FRAME_LEN {
            return Err(Error::Protocol(format!(
                "frame of {len} bytes exceeds the {MAX_FRAME_LEN} byte limit"
            )));
        }

        if self.buffer.len() < 8 + len {
            return Ok(None);
        }

        let body: Vec<u8> = self.buffer[8..8 + len].to_vec();
        self.buffer.drain(..8 + len);

        let payload = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body)?
        };

        Ok(Some(Frame { op, payload }))
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

/// One connected Discord IPC socket.
#[derive(Debug)]
pub struct IpcConnection {
    stream: UnixStream,
    decoder: FrameDecoder,
    path: PathBuf,
}

impl IpcConnection {
    /// Try each candidate socket path and return the first one that accepts.
    pub fn connect_any(paths: &[PathBuf]) -> Result<Self> {
        let mut last_error: Option<String> = None;

        for path in paths {
            match UnixStream::connect(path) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(DEFAULT_READ_TIMEOUT))?;
                    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                    return Ok(Self {
                        stream,
                        decoder: FrameDecoder::default(),
                        path: path.clone(),
                    });
                }
                Err(error) => {
                    last_error = Some(format!("{}: {error}", path.display()));
                }
            }
        }

        Err(Error::SocketUnavailable(
            last_error.unwrap_or_else(|| "no candidate paths".to_string()),
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn send(&mut self, op: u32, payload: &Value) -> Result<()> {
        let bytes = encode(op, payload);
        self.stream.write_all(&bytes)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Read the next frame. Returns `Ok(None)` on read timeout so the caller can
    /// service commands and timers; any real socket failure is an error.
    pub fn try_recv(&mut self) -> Result<Option<Frame>> {
        if let Some(frame) = self.decoder.next_frame()? {
            return Ok(Some(frame));
        }

        let mut buffer = [0u8; 16 * 1024];
        match self.stream.read(&mut buffer) {
            Ok(0) => Err(Error::Disconnected(
                "Discord closed the IPC connection".to_string(),
            )),
            Ok(read) => {
                self.decoder.extend(&buffer[..read]);
                self.decoder.next_frame()
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(Error::Io(error)),
        }
    }

    /// Opcode 0: identify ourselves to Discord. No other frame may be sent first.
    pub fn handshake(&mut self, client_id: &str) -> Result<()> {
        self.send(OP_HANDSHAKE, &json!({ "v": 1, "client_id": client_id }))
    }
}

/// Directory search order for the Discord IPC socket.
///
/// * `DISCORD_IPC_PATH` lets a user point at a non-standard client.
/// * `$XDG_RUNTIME_DIR` is where the native and Steam builds put it.
/// * The Flatpak build nests it under `app/com.discordapp.Discord`.
/// * The Snap build nests it under `snap.discord`.
/// * `/tmp` and `$TMPDIR` are the historical fallback locations.
pub fn socket_paths_from(
    explicit: Option<&Path>,
    runtime_dir: Option<&Path>,
    tmp_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();

    if let Some(explicit) = explicit {
        paths.push(explicit.to_path_buf());
        return paths;
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(runtime_dir) = runtime_dir {
        roots.push(runtime_dir.to_path_buf());
        roots.push(runtime_dir.join("app/com.discordapp.Discord"));
        roots.push(runtime_dir.join("snap.discord"));
    }
    if let Some(tmp_dir) = tmp_dir {
        roots.push(tmp_dir.to_path_buf());
    }
    roots.push(PathBuf::from("/tmp"));

    for root in roots {
        for index in 0..10 {
            paths.push(root.join(format!("discord-ipc-{index}")));
        }
    }

    paths
}

/// Discover socket paths using the current process environment.
pub fn discover_socket_paths() -> Vec<PathBuf> {
    let explicit = std::env::var_os("DISCORD_IPC_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let tmp_dir = std::env::var_os("TMPDIR").map(PathBuf::from);

    socket_paths_from(
        explicit.as_deref(),
        runtime_dir.as_deref(),
        tmp_dir.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let payload = json!({ "cmd": "AUTHENTICATE", "args": { "access_token": "abc" } });
        let bytes = encode(OP_FRAME, &payload);
        assert_eq!(bytes.len(), 8 + serde_json::to_vec(&payload).unwrap().len());

        let mut decoder = FrameDecoder::default();
        decoder.extend(&bytes);
        let frame = decoder.next_frame().unwrap().expect("complete frame");
        assert_eq!(frame.op, OP_FRAME);
        assert_eq!(frame.payload, payload);
        assert!(decoder.next_frame().unwrap().is_none());
    }

    #[test]
    fn frames_survive_fragmented_reads() {
        let payload = json!({ "evt": "SPEAKING_START", "data": { "user_id": "1" } });
        let bytes = encode(OP_FRAME, &payload);

        let mut decoder = FrameDecoder::default();
        for (index, byte) in bytes.iter().enumerate() {
            decoder.extend(&[*byte]);
            let decoded = decoder.next_frame().unwrap();
            if index + 1 == bytes.len() {
                assert_eq!(
                    decoded.expect("last byte completes the frame").payload,
                    payload
                );
            } else {
                assert!(decoded.is_none(), "frame decoded too early at byte {index}");
            }
        }
    }

    #[test]
    fn multiple_frames_in_one_buffer_are_drained_in_order() {
        let a = encode(OP_PING, &json!({ "n": 1 }));
        let b = encode(OP_PONG, &json!({ "n": 2 }));

        let mut decoder = FrameDecoder::default();
        decoder.extend(&a);
        decoder.extend(&b);

        assert_eq!(decoder.next_frame().unwrap().unwrap().op, OP_PING);
        assert_eq!(decoder.next_frame().unwrap().unwrap().op, OP_PONG);
        assert!(decoder.next_frame().unwrap().is_none());
    }

    #[test]
    fn oversized_frames_are_rejected_instead_of_allocating() {
        let mut decoder = FrameDecoder::default();
        decoder.extend(&OP_FRAME.to_le_bytes());
        decoder.extend(&(MAX_FRAME_LEN as u32 + 1).to_le_bytes());

        match decoder.next_frame() {
            Err(Error::Protocol(message)) => assert!(message.contains("exceeds")),
            other => panic!("expected protocol error, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_a_protocol_error() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&OP_FRAME.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(b"nope");

        let mut decoder = FrameDecoder::default();
        decoder.extend(&bytes);
        assert!(matches!(decoder.next_frame(), Err(Error::Json(_))));
    }

    #[test]
    fn socket_candidates_cover_native_flatpak_and_snap_layouts() {
        let paths = socket_paths_from(
            None,
            Some(Path::new("/run/user/1000")),
            Some(Path::new("/tmp/t")),
        );
        let rendered: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();

        assert!(rendered.contains(&"/run/user/1000/discord-ipc-0".to_string()));
        assert!(rendered
            .contains(&"/run/user/1000/app/com.discordapp.Discord/discord-ipc-0".to_string()));
        assert!(rendered.contains(&"/run/user/1000/snap.discord/discord-ipc-0".to_string()));
        assert!(rendered.contains(&"/tmp/t/discord-ipc-9".to_string()));
        assert!(rendered.contains(&"/tmp/discord-ipc-0".to_string()));

        // No duplicates: a repeated root must not double the candidate list.
        let mut sorted = rendered.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), rendered.len());
    }

    #[test]
    fn explicit_socket_path_wins() {
        let paths = socket_paths_from(
            Some(Path::new("/custom/discord-ipc-0")),
            Some(Path::new("/run/user/1000")),
            None,
        );
        assert_eq!(paths, vec![PathBuf::from("/custom/discord-ipc-0")]);
    }
}
