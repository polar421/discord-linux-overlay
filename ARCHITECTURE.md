# Architecture

Discord Linux Overlay is a Tauri v2 application with a deliberately testable
core and a **native GTK3 overlay**. Everything that can be reasoned about
without a display server lives in a separate Rust crate.

The overlay used to be a transparent WebKit web view. WebKitGTK's UI/web/network
process model put a ~265 MiB floor on the idle application, so the overlay is
now drawn directly with GTK/cairo (`native_overlay.rs`). Tauri remains for the
tray, global shortcuts, control socket, Discord worker and the on-demand
settings web view. See [docs/PERFORMANCE.md](docs/PERFORMANCE.md) and
[docs/MEMORY_ANALYSIS.md](docs/MEMORY_ANALYSIS.md).

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
src-tauri (Tauri shell; native GTK overlay; WebKit only for Settings)
   ├── main.rs         CLI, single-instance handoff, setup, window events
   ├── native_overlay.rs native GTK3/cairo overlay, avatars, input, placement
   ├── commands.rs     invoke handlers + shared request dispatcher
   ├── discord.rs      client lifecycle, event bridge, auto show/hide
   ├── windows.rs      on-demand Settings window, WebKit child cleanup
   ├── tray.rs         StatusNotifierItem menu
   ├── control.rs      control socket server + CLI client
   └── platform.rs     session detection, NVIDIA/WebKit mitigations
   ▲
   │  Tauri events (config/state/status) + invoke (Settings only)
   ▼
src/ (React 19 + TypeScript)
   ├── settings.html → src/settings.tsx → settings/Settings.tsx  (runtime, on demand)
   ├── index.html → overlay/Overlay.tsx   (browser preview only; not loaded by the app)
   ├── lib/bridge.ts  invoke/listen, or a browser-only mock harness
   └── components/    ParticipantRow, controls, icons
```

## Why a separate core crate

The Discord integration is the part most likely to break and the hardest to test
against a real client. Putting it in a GUI-free crate means:

- `cargo test -p discord-overlay-core` runs on any machine, including headless CI,
- the socket protocol is tested against a **fake Discord server** using a real
  Unix socket (`crates/discord-overlay-core/tests/discord_session.rs`),
- the Tauri layer is a thin adapter that maps events onto the native overlay,
  the tray and the on-demand settings web view.

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
when the revision changes, so a resync that changes nothing does not repaint the
native overlay.

Ordering is `self first, then case-insensitive display name, then id`. Speaking
never reorders rows. `set_channel()` preserves live speaking flags for users that
were already present so a resync cannot make a talking user flicker.

## Window model

One native overlay window; the settings web view only exists while it is open:

| Window | Properties |
| --- | --- |
| native overlay (GTK) | one transparent `GtkWindow` + `DrawingArea`, decorations off (empty CSD titlebar on Wayland), always-on-top, skip taskbar/pager, accepts no keyboard focus, resizable, minimum 64×64 |
| `settings` (WebKit) | normal decorated window, created on first use, destroyed on close; WebKit helpers are terminated afterwards |

- **Rendering** is cairo/Pango: rounded panels, avatars (gdk-pixbuf, circular
  clip, bounded LRU cache), speaking ring, mute/deafen/bot icons, compact badges,
  empty/status row, three layouts. Repaints happen only on state/config/pointer
  events; there is no animation loop or polling.
- **Click-through** installs an empty GTK input region; interactive mode
  restores the default region, shows the toolbar and drags with
  `begin_move_drag()` (no JavaScript involved).
- **Anchoring** reuses the same arithmetic in `geometry.rs`: monitor rectangle,
  logical window size, scale factor, anchor and offsets → physical
  position/size with clamping. On Wayland the position request is ignored by
  the compositor; the resize still applies.
- **Manual mode** persists logical window size and, on X11, position on every
  move/resize, rate limited to one write per 500 ms. A stale position that no
  longer intersects any monitor is clamped back onto the primary monitor.
- **Never focused on show**: `show()` does not present/focus the window, so the
  overlay cannot steal keyboard focus from a game.
- **Settings teardown**: closing the last web view makes the app terminate only
  its own WebKit descendants (generation-guarded) and call `malloc_trim(0)`;
  reopening Settings spawns fresh helpers.

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
| main | GTK/Tauri event loop; native overlay painting and input |
| `discord-ipc` | socket reads, auth, subscriptions, timers, reconnect |
| `discord-events` | forwards core events into Tauri state + `emit` |
| `control-socket` | accepts CLI requests, dispatches onto the main thread |

The Discord thread exits on `Stop` (set by `AppState::stop_client`), and the
`sleep_with_commands` helper keeps shutdown responsive even during a 30 s backoff.

## Front-end bridge and mock harness

The runtime overlay is native; it does not load React or any JavaScript. The
settings document still uses `src/lib/bridge.ts` (`invoke` + `listen`). In a
plain browser (`npm run dev`) both documents are served by `src/lib/mock.ts`,
which simulates a voice channel and speaking rotation. That harness exists so
the layout can be reviewed without Discord; it is not part of the shipped
application logic.

## File map

| Path | Contents |
| --- | --- |
| `crates/discord-overlay-core/src/` | Discord IPC, state, config, geometry, control |
| `crates/discord-overlay-core/tests/discord_session.rs` | fake-Discord end-to-end tests |
| `src-tauri/src/` | Tauri shell + native overlay renderer |
| `src-tauri/tauri.conf.json` | CSP, bundle (no startup windows) |
| `src-tauri/capabilities/default.json` | settings web view permissions |
| `src/` | React settings + browser overlay preview |
| `packaging/` | PKGBUILD, desktop entry, metainfo, Flatpak manifest, icons |
| `scripts/` | icon generation, dev/build/check helpers |
| `docs/` | research, Discord setup, milestones |
