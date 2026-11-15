//! Core logic for Discord Linux Overlay.
//!
//! This crate is GUI free on purpose: the Discord IPC client, the overlay state
//! machine, settings persistence and placement maths are all testable without a
//! display server. The Tauri shell in `src-tauri` is a thin adapter on top.
//!
//! ```text
//!  Discord desktop client
//!         │  Unix socket ($XDG_RUNTIME_DIR/discord-ipc-0)
//!         ▼
//!   ipc  ──►  client (auth, subscriptions, resync, reconnect)
//!                 │  events: Status / State / AuthRequired / Log
//!                 ▼
//!             state (participants, mute/deafen, speaking, history)
//!                 │
//!                 ▼
//!          Tauri app ──► overlay + settings web views
//! ```

pub mod avatar;
pub mod client;
pub mod config;
pub mod control;
pub mod credentials;
pub mod error;
pub mod geometry;
pub mod ipc;
pub mod protocol;
pub mod state;
#[cfg(feature = "http")]
pub mod token;

pub use error::{Error, Result};
