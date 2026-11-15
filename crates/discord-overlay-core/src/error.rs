use std::io;

/// Errors produced by the Discord IPC layer, the settings store and the
/// placement helpers. Everything that can go wrong in normal operation is
/// represented here so callers can decide whether to reconnect, re-authenticate
/// or simply surface a message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("discord ipc socket is not available ({0})")]
    SocketUnavailable(String),

    #[error("discord closed the ipc connection ({0})")]
    Disconnected(String),

    #[error("discord ipc protocol error: {0}")]
    Protocol(String),

    #[error("discord rpc error {code}: {message}")]
    Rpc { code: u32, message: String },

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("settings error: {0}")]
    Settings(String),

    #[error("token exchange failed: {0}")]
    TokenExchange(String),
}

pub type Result<T> = std::result::Result<T, Error>;
