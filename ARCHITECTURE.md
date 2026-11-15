# Architecture

Discord Linux Overlay is a Tauri v2 application with a deliberately testable
core. Everything that can be reasoned about without a display server lives in a
separate Rust crate.

```text
Discord desktop client
   │  Unix socket: $XDG_RUNTIME_DIR/discord-ipc-0
   │  framing: [op:u32][len:u32][json]
   ▼
crates/discord-overlay-core
   ├── ipc.rs        frame codec, socket discovery, buffered reads
   ├── protocol.rs   RPC commands/events, voice-state types
   ├── client.rs     background thread: handshake → AUTHORIZE → AUTHENTICATE
   │                 → subscribe → resync → reconnect, token cache
   ├── state.rs      participants, speaking, mute/deafen, history, ordering
   ├── avatar.rs     Discord CDN URLs (default avatars, animated avatars)
   ├── config.rs     nested settings model, clamping, atomic 0600 writes
   ├── credentials.rs Discord application id/secret, never exposed to the UI
   ├── geometry.rs   monitor-aware anchoring math
   ├── control.rs    CLI/control-socket request protocol
   └── token.rs      OAuth2 code → access token (feature "http")
   ▲
   │  std::sync::mpsc: Status / State / AuthRequired / Log
   ▼
src-tauri (Tauri shell, GTK3 + WebKitGTK on Linux)
   ├── main.rs       CLI, single-instance handoff, setup, window events
   ├── commands.rs   invoke handlers + shared request dispatcher
   ├── discord.rs    client lifecycle, event bridge, auto show/hide
   ├── windows.rs    placement, click-through, always-on-top, bounds memory
   ├── tray.rs       StatusNotifierItem menu
   ├── control.rs    control socket server + CLI client
   └── platform.rs   session detection, NVIDIA/WebKit mitigations
   ▲
   │  Tauri events + invoke
   ▼
src/ (React 19 + TypeScript, two documents)
   ├── index.html → src/main.tsx → overlay/Overlay.tsx
   ├── settings.html → src/settings.tsx → settings/Settings.tsx
   ├── lib/bridge.ts  invoke/listen, or a browser-only mock harness
   └── components/    ParticipantRow, controls, icons
```

## Why a separate core crate

The Discord integration is the part most likely to break and the hardest to test
against a real client. Putting it in a GUI-free crate means:

- `cargo test -p discord-overlay-core` runs on any machine, including headless CI,
- the socket protocol is tested against a **fake Discord server** using a real
  Unix socket (`crates/discord-overlay-core/tests/discord_session.rs`),
- the Tauri layer is a thin adapter that only maps events onto windows and web
  views.

## Discord connection lifecycle

1. Discover the socket (`DISCORD_IPC_PATH`, `$XDG_RUNTIME_DIR`, Flatpak path,
   Snap path, `$TMPDIR`, `/tmp`, `discord-ipc-0..9`).
2. Send opcode 0 handshake `{ "v": 1, "client_id": … }`.
3. Discord replies `READY`.
4. If a cached token exists → `AUTHENTICATE`. Otherwise → `AUTHORIZE` with scopes
   `identify`, `rpc`; the user approves inside Discord; the returned code is
   exchanged locally for a token with the user's application secret; then
   `AUTHENTICATE`.
5. Subscribe to `VOICE_CHANNEL_SELECT`, ask for `GET_SELECTED_VOICE_CHANNEL`, and
   subscribe to the five per-channel events:
   `SPEAKING_START`, `SPEAKING_STOP`, `VOICE_STATE_CREATE`,
   `VOICE_STATE_UPDATE`, `VOICE_STATE_DELETE`.
6. While connected:
   - speaking events flip a boolean on an existing participant (no reordering),
   - `VOICE_STATE_CREATE` adds a row and records join history,
   - `VOICE_STATE_UPDATE` refreshes flags (mute/deafen/nickname) in place,
   - `VOICE_STATE_DELETE` removes the row and records leave history,
   - `VOICE_CHANNEL_SELECT` triggers a fresh snapshot,
   - every 20 s a nonce-tagged `GET_SELECTED_VOICE_CHANNEL` resync repairs lost
     events; two missed replies (5 s each) force a reconnect,
   - reconnects back off `1s, 2s, 5s, 10s, 20s, 30s`, and reset after a
     successful session.
7. `INVALID_TOKEN` / `INVALID_USER` / `OAUTH2_ERROR` on `AUTHENTICATE` clears the
   token and re-runs authorization. `INVALID_CLIENT_ID` surfaces "setup required"
   instead of looping.

## State model

`OverlayState` is the complete UI input: `me`, `channel`, `participants`,
`history`, and a monotonic `revision`. The client emits a full snapshot on every
change; the UI compares nothing and simply re-renders. Snapshots are only sent
when the revision changes, so a resync that changes nothing does not touch the
web view.

Ordering is `self first, then case-insensitive display name, then id`. Speaking
never reorders rows. `set_channel()` preserves live speaking flags for users that
were already present so a resync cannot make a talking user flicker.

## Window model

Two windows, one process:

| Window | Properties |
| --- | --- |
| `overlay` | transparent, decorations off, shadow off, always-on-top requested, skip taskbar, resizable, minimum 64×64 |
| `settings` | normal decorated window, created hidden, close = hide (the app keeps running) |

- **Click-through** is `set_ignore_cursor_events(true)`; the whole window stops
  receiving input.
- **Interactive mode** re-enables input, shows the toolbar, and uses
  `startDragging()` from the web view (permission
  `core:window:allow-start-dragging`).
- **Anchoring** is pure arithmetic in `geometry.rs`: monitor rectangle, logical
  window size, scale factor, anchor and offsets → physical position/size with
  clamping. On Wayland the position request is ignored by the compositor; the
  resize still applies.
- **Manual mode** persists `outer_position()` and logical `inner_size()` on every
  move/resize, rate limited to one write per 500 ms. A stale position that no
  longer intersects any monitor is clamped back onto the primary monitor.
- **Never focused on show**: `show_overlay()` does not call `set_focus()`, so the
  overlay cannot steal keyboard focus from a game.

## Configuration and secrets

```
$XDG_CONFIG_HOME/discord-linux-overlay/
├── config.json        0644, settings (clamped on load)
├── credentials.json   0600, Discord client id + secret
└── discord-token.json 0600, cached RPC access token
```

`config.json` is merged with defaults on read, unknown keys are ignored, and all
numeric fields are clamped, so a corrupt or old file cannot prevent startup. The
settings UI sends **partial JSON patches** which are deep-merged and persisted
with a temp-file + rename write.

`credentials.json` is separate from `config.json` on purpose: the secret must
never travel to a web view. `CredentialsView` exposes only the client id, a
masked id and a `hasSecret` boolean.

## Threads

| Thread | Responsibility |
| --- | --- |
| main | Tauri/GTK event loop, window operations |
| `discord-ipc` | socket reads, auth, subscriptions, timers, reconnect |
| `discord-events` | forwards core events into Tauri state + `emit` |
| `control-socket` | accepts CLI requests, dispatches onto the main thread |

The Discord thread exits on `Stop` (set by `AppState::stop_client`), and the
`sleep_with_commands` helper keeps shutdown responsive even during a 30 s backoff.

## Front-end bridge and mock harness

`src/lib/bridge.ts` checks for `__TAURI_INTERNALS__`. Inside Tauri every call is
`invoke` and every subscription is `listen`. In a plain browser
(`npm run dev`) the same UI is served by `src/lib/mock.ts`, which simulates a
voice channel and speaking rotation. That harness exists so the layout can be
reviewed without Discord; it is not part of the shipped application logic and is
tree-shaken into the dev bundle only.

## File map

| Path | Contents |
| --- | --- |
| `crates/discord-overlay-core/src/` | Discord IPC, state, config, geometry, control |
| `crates/discord-overlay-core/tests/discord_session.rs` | fake-Discord end-to-end tests |
| `src-tauri/src/` | Tauri shell |
| `src-tauri/tauri.conf.json` | windows, CSP, bundle |
| `src-tauri/capabilities/default.json` | web view permissions |
| `src/` | React overlay + settings |
| `packaging/` | PKGBUILD, desktop entry, metainfo, Flatpak manifest, icons |
| `scripts/` | icon generation, dev/build/check helpers |
| `docs/` | research, Discord setup, milestones |
